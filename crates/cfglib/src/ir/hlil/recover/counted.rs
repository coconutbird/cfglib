//! Counted-loop recovery: `init; while (c) { …; update }` becomes a
//! [`For`](StatementKind::For) statement when the initializer writes the
//! variable the condition reads and the body's trailing statement updates
//! it.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use super::{
    ExpressionId, ExpressionKind, Rebuilder, RecoverDialect, Result, StatementId, StatementKind,
    VariableId, VerifyDialect,
};

impl<D: RecoverDialect + VerifyDialect> Rebuilder<'_, D> {
    /// Recovers `ids[index]; ids[index + 1]` as one `for` statement.
    ///
    /// The transformation is exact only when no `continue` reaches the
    /// loop: [`For`](StatementKind::For) runs its update on `continue`,
    /// while the `while` form skips the trailing statement.
    pub(super) fn try_for(
        &mut self,
        ids: &[StatementId],
        index: usize,
    ) -> Result<Option<StatementId>> {
        let init = ids[index];
        let loop_id = ids[index + 1];
        let StatementKind::Assign { target, .. } = self.statement_kind(init)? else {
            return Ok(None);
        };
        let target_node = self.expression(*target)?;
        let ExpressionKind::Variable(inductive) = target_node.kind() else {
            return Ok(None);
        };
        let inductive = *inductive;
        if !D::single_expression_assignment(target_node.value_type()) {
            return Ok(None);
        }
        let StatementKind::While { condition, body } = self.statement_kind(loop_id)? else {
            return Ok(None);
        };
        if !self.expression_reads(*condition, inductive)? {
            return Ok(None);
        }
        let Some((&update, rest)) = body.split_last() else {
            return Ok(None);
        };
        let StatementKind::Assign {
            target: update_target,
            ..
        } = self.statement_kind(update)?
        else {
            return Ok(None);
        };
        let update_node = self.expression(*update_target)?;
        let ExpressionKind::Variable(updated) = update_node.kind() else {
            return Ok(None);
        };
        if *updated != inductive || !D::single_expression_assignment(update_node.value_type()) {
            return Ok(None);
        }
        if self.contains_continue(rest)? {
            return Ok(None);
        }

        let new_init = self.rebuild_statement(init)?;
        let new_condition = self.rebuild_expression(*condition)?;
        let new_body = self.rebuild_body(rest)?;
        let new_update = self.rebuild_statement(update)?;
        let counted = self.builder.add_statement(
            StatementKind::For {
                initializer: vec![new_init],
                condition: Some(new_condition),
                update: vec![new_update],
                body: new_body,
            },
            None,
        )?;
        self.statements.insert(loop_id, counted);
        self.for_loops += 1;
        Ok(Some(counted))
    }

    /// Whether any node of the source expression tree reads the variable.
    pub(super) fn expression_reads(
        &self,
        expression: ExpressionId,
        variable: VariableId,
    ) -> Result<bool> {
        let mut stack = vec![expression];
        while let Some(id) = stack.pop() {
            match self.expression(id)?.kind() {
                ExpressionKind::Variable(read) if *read == variable => return Ok(true),
                ExpressionKind::Operation { operands, .. } => {
                    stack.extend(operands.iter().copied());
                }
                ExpressionKind::Variable(_) | ExpressionKind::Constant(_) => {}
            }
        }
        Ok(false)
    }

    /// Whether any statement of the source subtree is a `continue`; nested
    /// loops are included, keeping the check conservative.
    pub(super) fn contains_continue(&self, ids: &[StatementId]) -> Result<bool> {
        let mut stack: Vec<StatementId> = ids.to_vec();
        while let Some(id) = stack.pop() {
            let kind = self.statement_kind(id)?;
            if matches!(kind, StatementKind::Continue { .. }) {
                return Ok(true);
            }
            for body in super::super::statement::child_bodies(kind) {
                stack.extend(body.iter().copied());
            }
        }
        Ok(false)
    }
}
