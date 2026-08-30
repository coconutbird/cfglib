//! Conditional value selection: an assigning or returning diamond becomes
//! one dialect `select` expression evaluating its condition once and
//! exactly one arm — the source shape of `?:` and if-expressions.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use super::{
    ExpressionId, ExpressionKind, Rebuilder, RecoverDialect, Result, StatementId, StatementKind,
    VariableId, VerifyDialect,
};

impl<D: RecoverDialect + VerifyDialect> Rebuilder<'_, D> {
    /// Recovers `if (c) x = a; else x = b;` as `x = select(c, a, b);` and
    /// the all-arms-return diamond as `return select(c, a, b);`, recursing
    /// through nested one-statement diamonds.
    pub(super) fn try_select(
        &mut self,
        if_id: StatementId,
        condition: ExpressionId,
        then_body: &'_ [StatementId],
        else_body: &'_ [StatementId],
    ) -> Result<Option<StatementId>> {
        if D::select().is_none() {
            return Ok(None);
        }
        if let Some(target) = self.assigned_target(then_body)?
            && self.assigns_shape(then_body, target)?
            && self.assigns_shape(else_body, target)?
        {
            // Validated: build the selection tree and the single assign.
            let (variable, value_type) = {
                let (variable, value_type) = self.selection_target_node(then_body)?;
                (variable, value_type.clone())
            };
            let value = self.build_select(condition, then_body, else_body, target)?;
            let new_target = self
                .builder
                .add_expression(ExpressionKind::Variable(variable), value_type)?;
            let assign = self.builder.add_statement(
                StatementKind::Assign {
                    target: new_target,
                    value,
                },
                None,
            )?;
            self.map_selection(if_id, then_body, else_body, assign)?;
            self.selects += 1;
            return Ok(Some(assign));
        }
        if self.returns_shape(then_body)? && self.returns_shape(else_body)? {
            let value = self.build_return_select(condition, then_body, else_body)?;
            let statement = self.builder.add_statement(
                StatementKind::Return {
                    values: vec![value],
                },
                None,
            )?;
            self.map_selection(if_id, then_body, else_body, statement)?;
            self.selects += 1;
            return Ok(Some(statement));
        }
        Ok(None)
    }

    /// The variable the then-arm chain assigns, when there is one.
    fn assigned_target(&self, ids: &[StatementId]) -> Result<Option<VariableId>> {
        let [only] = ids else {
            return Ok(None);
        };
        match self.statement_kind(*only)? {
            StatementKind::Assign { target, .. } => Ok(match self.expression(*target)?.kind() {
                ExpressionKind::Variable(variable) => Some(*variable),
                _ => None,
            }),
            StatementKind::If { then_body, .. } => self.assigned_target(then_body),
            _ => Ok(None),
        }
    }

    /// The variable and occurrence type of the leftmost assignment.
    fn selection_target_node(&self, ids: &[StatementId]) -> Result<(VariableId, &'_ D::ValueType)> {
        let [only] = ids else {
            return Err(super::Error::InvalidConstruction(
                "a validated selection arm is a single statement".into(),
            ));
        };
        match self.statement_kind(*only)? {
            StatementKind::Assign { target, .. } => {
                let node = self.expression(*target)?;
                let ExpressionKind::Variable(variable) = node.kind() else {
                    return Err(super::Error::InvalidConstruction(
                        "a validated selection arm assigns a variable".into(),
                    ));
                };
                Ok((*variable, node.value_type()))
            }
            StatementKind::If { then_body, .. } => self.selection_target_node(then_body),
            _ => Err(super::Error::InvalidConstruction(
                "a validated selection arm assigns or nests".into(),
            )),
        }
    }

    /// Whether one arm is a single-expression assignment of `target`, or a
    /// nested diamond of such arms.
    fn assigns_shape(&self, ids: &[StatementId], target: VariableId) -> Result<bool> {
        let [only] = ids else {
            return Ok(false);
        };
        match self.statement_kind(*only)? {
            StatementKind::Assign {
                target: assigned,
                value,
            } => {
                let node = self.expression(*assigned)?;
                let ExpressionKind::Variable(variable) = node.kind() else {
                    return Ok(false);
                };
                Ok(*variable == target
                    && self.assignment_is_single_expression(*assigned, *value)?)
            }
            StatementKind::If {
                then_body,
                else_body,
                ..
            } => {
                Ok(self.assigns_shape(then_body, target)?
                    && self.assigns_shape(else_body, target)?)
            }
            _ => Ok(false),
        }
    }

    /// Whether one arm returns a single value, or nests such diamonds.
    fn returns_shape(&self, ids: &[StatementId]) -> Result<bool> {
        let [only] = ids else {
            return Ok(false);
        };
        match self.statement_kind(*only)? {
            StatementKind::Return { values } => Ok(values.len() == 1),
            StatementKind::If {
                then_body,
                else_body,
                ..
            } => Ok(self.returns_shape(then_body)? && self.returns_shape(else_body)?),
            _ => Ok(false),
        }
    }

    /// Builds the selection expression for one validated assigning arm.
    fn arm_value(&mut self, ids: &[StatementId], target: VariableId) -> Result<ExpressionId> {
        let [only] = ids else {
            unreachable!("a validated selection arm is a single statement");
        };
        match self.statement_kind(*only)? {
            StatementKind::Assign { value, .. } => self.rebuild_expression(*value),
            StatementKind::If {
                condition,
                then_body,
                else_body,
            } => self.build_select(*condition, then_body, else_body, target),
            _ => unreachable!("a validated selection arm assigns or nests"),
        }
    }

    fn build_select(
        &mut self,
        condition: ExpressionId,
        then_body: &[StatementId],
        else_body: &[StatementId],
        target: VariableId,
    ) -> Result<ExpressionId> {
        let operation = D::select().expect("selection recovery requires the select operation");
        let value_type = self.selection_target_node(then_body)?.1.clone();
        let condition = self.rebuild_expression(condition)?;
        let when_true = self.arm_value(then_body, target)?;
        let when_false = self.arm_value(else_body, target)?;
        self.builder.add_expression(
            ExpressionKind::Operation {
                operation,
                operands: vec![condition, when_true, when_false],
            },
            value_type,
        )
    }

    /// Builds the selection expression for one validated returning arm.
    fn return_value(&mut self, ids: &[StatementId]) -> Result<ExpressionId> {
        let [only] = ids else {
            unreachable!("a validated selection arm is a single statement");
        };
        match self.statement_kind(*only)? {
            StatementKind::Return { values } => self.rebuild_expression(values[0]),
            StatementKind::If {
                condition,
                then_body,
                else_body,
            } => self.build_return_select(*condition, then_body, else_body),
            _ => unreachable!("a validated selection arm returns or nests"),
        }
    }

    fn build_return_select(
        &mut self,
        condition: ExpressionId,
        then_body: &[StatementId],
        else_body: &[StatementId],
    ) -> Result<ExpressionId> {
        let operation = D::select().expect("selection recovery requires the select operation");
        let condition_expression = self.rebuild_expression(condition)?;
        let when_true = self.return_value(then_body)?;
        let when_false = self.return_value(else_body)?;
        let value_type = self
            .builder
            .expression(when_true)
            .map(|expression| expression.value_type().clone())
            .expect("the arm expression was just built");
        self.builder.add_expression(
            ExpressionKind::Operation {
                operation,
                operands: vec![condition_expression, when_true, when_false],
            },
            value_type,
        )
    }

    /// Maps every statement the selection consumed onto the new statement.
    fn map_selection(
        &mut self,
        if_id: StatementId,
        then_body: &[StatementId],
        else_body: &[StatementId],
        statement: StatementId,
    ) -> Result<()> {
        let mut stack: Vec<StatementId> = vec![if_id];
        stack.extend(then_body.iter().copied());
        stack.extend(else_body.iter().copied());
        while let Some(id) = stack.pop() {
            self.statements.insert(id, statement);
            if let StatementKind::If {
                then_body,
                else_body,
                ..
            } = self.statement_kind(id)?
            {
                stack.extend(then_body.iter().copied());
                stack.extend(else_body.iter().copied());
            }
        }
        Ok(())
    }
}
