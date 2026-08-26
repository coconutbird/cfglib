//! Paired enter/exit region recovery: `enter(x); try { …; exit(x) }
//! catch-any { exit(x); rethrow }` becomes one
//! [`Region`](StatementKind::Region) statement — the lowered shape of
//! `synchronized`, `lock`, and similar language constructs, whose source
//! form regenerates the release-and-rethrow cleanup exactly.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use super::{
    ActiveRegion, ExpressionId, ExpressionKind, HandlerKind, Rebuilder, RecoverDialect, Result,
    StatementId, StatementKind, VariableId, VerifyDialect,
};

impl<D: RecoverDialect + VerifyDialect> Rebuilder<'_, D> {
    /// Recovers `ids[index]; ids[index + 1]` as one region statement.
    pub(super) fn try_region(
        &mut self,
        ids: &[StatementId],
        index: usize,
    ) -> Result<Option<StatementId>> {
        let enter_id = ids[index];
        let guarded_id = ids[index + 1];
        let StatementKind::Expression(enter_expression) = self.statement_kind(enter_id)? else {
            return Ok(None);
        };
        let ExpressionKind::Operation {
            operation: enter,
            operands,
        } = self.expression(*enter_expression)?.kind()
        else {
            return Ok(None);
        };
        let Some(region_operation) = D::region_enter(enter) else {
            return Ok(None);
        };
        let [subject] = operands.as_slice() else {
            return Ok(None);
        };
        let ExpressionKind::Variable(entered) = self.expression(*subject)?.kind() else {
            return Ok(None);
        };
        let entered = *entered;
        let StatementKind::Try {
            body,
            handlers,
            finally_body,
        } = self.statement_kind(guarded_id)?
        else {
            return Ok(None);
        };
        let [handler] = handlers.as_slice() else {
            return Ok(None);
        };
        if !finally_body.is_empty() || !matches!(handler.kind, HandlerKind::CatchAll) {
            return Ok(None);
        }
        let Some(released) = self.cleanup_release(enter, &handler.body)? else {
            return Ok(None);
        };
        // The released view must provably hold the entered object: the
        // same variable, or one copied from the other beforehand.
        if released != entered && !self.copies_link(&ids[..index], entered, released)? {
            return Ok(None);
        }
        // Neither view may be reassigned inside the guarded body.
        if self.assigns_variable(body, entered)?
            || (released != entered && self.assigns_variable(body, released)?)
        {
            return Ok(None);
        }

        let new_subject = self.rebuild_expression(*subject)?;
        let aliases = [entered, released].into_iter().collect();
        self.active_regions.push(ActiveRegion {
            enter: enter.clone(),
            aliases,
            exits: Vec::new(),
        });
        let new_body = self.rebuild_body(body);
        let active = self
            .active_regions
            .pop()
            .expect("the region pushed above is still active");
        let new_body = new_body?;
        let region = self.builder.add_statement(
            StatementKind::Region {
                operation: region_operation,
                operands: vec![new_subject],
                body: new_body,
            },
            None,
        )?;
        // The construct re-owns the enter, the cleanup, and every
        // suppressed exit.
        self.statements.insert(enter_id, region);
        self.statements.insert(guarded_id, region);
        for &cleanup in &handler.body {
            self.statements.insert(cleanup, region);
        }
        for exit in active.exits {
            self.statements.insert(exit, region);
        }
        self.regions += 1;
        Ok(Some(region))
    }

    /// The variable the cleanup handler releases before rethrowing the
    /// delivered exception, when the handler is exactly that cleanup.
    fn cleanup_release(
        &self,
        enter: &D::Operation,
        body: &[StatementId],
    ) -> Result<Option<VariableId>> {
        let (exit, throw, bound) = match body {
            [exit, throw] => (*exit, *throw, None),
            [assign, exit, throw] => {
                let StatementKind::Assign { target, value } = self.statement_kind(*assign)? else {
                    return Ok(None);
                };
                let ExpressionKind::Variable(bound) = self.expression(*target)?.kind() else {
                    return Ok(None);
                };
                if !self.contains_exception_materialization(*value)? {
                    return Ok(None);
                }
                (*exit, *throw, Some(*bound))
            }
            _ => return Ok(None),
        };
        let StatementKind::Expression(exit) = self.statement_kind(exit)? else {
            return Ok(None);
        };
        let ExpressionKind::Operation {
            operation,
            operands,
        } = self.expression(*exit)?.kind()
        else {
            return Ok(None);
        };
        if !D::releases(enter, operation) {
            return Ok(None);
        }
        let [released] = operands.as_slice() else {
            return Ok(None);
        };
        let ExpressionKind::Variable(released) = self.expression(*released)?.kind() else {
            return Ok(None);
        };
        let StatementKind::Expression(throw) = self.statement_kind(throw)? else {
            return Ok(None);
        };
        let ExpressionKind::Operation {
            operation: thrown_operation,
            operands: thrown,
        } = self.expression(*throw)?.kind()
        else {
            return Ok(None);
        };
        if !D::is_throw(thrown_operation) {
            return Ok(None);
        }
        let [thrown] = thrown.as_slice() else {
            return Ok(None);
        };
        let rethrows = match self.expression(*thrown)?.kind() {
            ExpressionKind::Variable(variable) => Some(*variable) == bound,
            _ => self.contains_exception_materialization(*thrown)?,
        };
        Ok(rethrows.then_some(*released))
    }

    /// Whether one expression tree materializes the delivered exception.
    fn contains_exception_materialization(&self, expression: ExpressionId) -> Result<bool> {
        let mut stack = vec![expression];
        while let Some(id) = stack.pop() {
            if let ExpressionKind::Operation {
                operation,
                operands,
            } = self.expression(id)?.kind()
            {
                if D::is_exception_materialization(operation) {
                    return Ok(true);
                }
                stack.extend(operands.iter().copied());
            }
        }
        Ok(false)
    }

    /// Whether the preceding statements copy one variable into the other.
    fn copies_link(
        &self,
        preceding: &[StatementId],
        first: VariableId,
        second: VariableId,
    ) -> Result<bool> {
        for &id in preceding {
            let StatementKind::Assign { target, value } = self.statement_kind(id)? else {
                continue;
            };
            let ExpressionKind::Variable(target) = self.expression(*target)?.kind() else {
                continue;
            };
            let ExpressionKind::Variable(value) = self.expression(*value)?.kind() else {
                continue;
            };
            let pair = (*target, *value);
            if pair == (first, second) || pair == (second, first) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether any statement of the source subtree assigns the variable.
    fn assigns_variable(&self, ids: &[StatementId], variable: VariableId) -> Result<bool> {
        let mut stack: Vec<StatementId> = ids.to_vec();
        while let Some(id) = stack.pop() {
            let kind = self.statement_kind(id)?;
            if let StatementKind::Assign { target, .. } = kind
                && let ExpressionKind::Variable(assigned) = self.expression(*target)?.kind()
                && *assigned == variable
            {
                return Ok(true);
            }
            for body in super::super::statement::child_bodies(kind) {
                stack.extend(body.iter().copied());
            }
        }
        Ok(false)
    }
}
