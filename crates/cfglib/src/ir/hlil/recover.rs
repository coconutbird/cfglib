//! Structural recovery over lifted HLIL.
//!
//! Binary and bytecode frontends encode source constructs as flatter
//! shapes: a counted loop arrives as `init; while (c) { …; update }`, a
//! conditional expression as an assigning diamond, and a language region
//! protocol (`synchronized`, `lock`) as paired enter/exit operations with
//! an exceptional cleanup handler. [`recover_structure`] rebuilds a lifted
//! function with those source-level shapes restored — [`For`], value
//! selection through the dialect's [`select`](RecoverDialect::select)
//! operation, and [`Region`] statements — leaving everything the dialect
//! does not claim untouched.
//!
//! Every transformation is a pure re-expression: a recovered `for` runs
//! the same statements in the same order, a selection still evaluates its
//! condition once and exactly one arm, and a recovered region re-owns the
//! release-and-rethrow cleanup its construct regenerates.
//!
//! [`For`]: StatementKind::For
//! [`Region`]: StatementKind::Region

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::{
    Dialect, Error, Expression, ExpressionId, ExpressionKind, Function, FunctionBuilder, Handler,
    HandlerKind, Result, Signature, Statement, StatementId, StatementKind, VariableId,
    VerifyDialect,
};

/// Consumer hooks steering structural recovery. Every hook defaults to
/// "not claimed", so a dialect opts into each recovery independently.
pub trait RecoverDialect: Dialect {
    /// The operation selecting between two values by a condition, applied
    /// as `select(condition, when_true, when_false)` with exactly one arm
    /// evaluated. `None` disables conditional-expression recovery.
    #[must_use]
    fn select() -> Option<Self::Operation> {
        None
    }

    /// Whether an assignment producing a value of this type is one plain
    /// expression in the dialect's source language — eligible as a `for`
    /// initializer or update, and as a selection arm. Types that expand to
    /// multiple statements (dual-view zero patterns) return false.
    #[must_use]
    fn single_expression_assignment(value_type: &Self::ValueType) -> bool {
        let _ = value_type;
        true
    }

    /// Whether this operation has one expression spelling. Return `false` for
    /// operations a renderer must expand into multiple statements. Recovery
    /// checks every operation in an assignment's value tree, then leaves the
    /// assignment out of `for` clauses and conditional selections if any
    /// operation is not expression-like.
    #[must_use]
    fn single_expression_operation(operation: &Self::Operation) -> bool {
        let _ = operation;
        true
    }

    /// The [`Region`](StatementKind::Region) operation recovering one
    /// paired enter/exit protocol, given its enter operation. `None`
    /// leaves the enter operation as an ordinary statement.
    #[must_use]
    fn region_enter(operation: &Self::Operation) -> Option<Self::Operation> {
        let _ = operation;
        None
    }

    /// Whether `exit` releases what `enter` acquired.
    #[must_use]
    fn releases(enter: &Self::Operation, exit: &Self::Operation) -> bool {
        let _ = (enter, exit);
        false
    }

    /// Whether the operation materializes the delivered exception inside a
    /// handler.
    #[must_use]
    fn is_exception_materialization(operation: &Self::Operation) -> bool {
        let _ = operation;
        false
    }

    /// Whether the operation throws its operand.
    #[must_use]
    fn is_throw(operation: &Self::Operation) -> bool {
        let _ = operation;
        false
    }
}

/// The result of one structural recovery pass.
#[derive(Debug, Clone)]
pub struct Recovery<D: Dialect> {
    /// The rebuilt function.
    pub function: Function<D>,
    /// Source statement → the statement carrying it now: its copy, or the
    /// recovered construct that consumed it.
    pub statements: BTreeMap<StatementId, StatementId>,
    /// Source expression → its copy. Expressions of consumed cleanup
    /// statements have no copy; their statements map to the construct.
    pub expressions: BTreeMap<ExpressionId, ExpressionId>,
    /// Counted loops recovered.
    pub for_loops: usize,
    /// Conditional value selections recovered.
    pub selects: usize,
    /// Paired enter/exit regions recovered.
    pub regions: usize,
}

/// Rebuilds one function with source-level structure recovered.
///
/// # Errors
///
/// Returns an error when the rebuilt function fails verification; the
/// source function is never mutated.
pub fn recover_structure<D>(source: &Function<D>) -> Result<Recovery<D>>
where
    D: RecoverDialect + VerifyDialect,
{
    let mut rebuilder = Rebuilder {
        source,
        builder: FunctionBuilder::new(source.source().clone()),
        statements: BTreeMap::new(),
        expressions: BTreeMap::new(),
        active_regions: Vec::new(),
        for_loops: 0,
        selects: 0,
        regions: 0,
    };
    for variable in source.variables() {
        rebuilder.builder.declare_variable(
            variable.role.clone(),
            variable.native.clone(),
            variable.declared_type.clone(),
        )?;
    }
    rebuilder.builder.set_signature(Signature::<D>::new(
        source.signature().parameters.clone(),
        source.signature().returns.clone(),
    ))?;
    let body = rebuilder.rebuild_body(source.body())?;
    rebuilder.builder.set_body(body)?;
    for entry in source.provenance().entries() {
        let entity = match entry.entity {
            super::EntityId::Variable(variable) => Some(super::EntityId::Variable(variable)),
            super::EntityId::Statement(statement) => rebuilder
                .statements
                .get(&statement)
                .map(|&new| super::EntityId::Statement(new)),
            super::EntityId::Expression(expression) => rebuilder
                .expressions
                .get(&expression)
                .map(|&new| super::EntityId::Expression(new)),
        };
        if let Some(entity) = entity {
            rebuilder.builder.map_entity(entry.source.clone(), entity)?;
        }
    }
    Ok(Recovery {
        function: rebuilder.builder.finish()?,
        statements: rebuilder.statements,
        expressions: rebuilder.expressions,
        for_loops: rebuilder.for_loops,
        selects: rebuilder.selects,
        regions: rebuilder.regions,
    })
}

/// One region under construction: its enter operation, the variables
/// currently holding the entered object (grown by copies, shrunk by
/// reassignment, in program order), and the exit statements its construct
/// re-owns.
struct ActiveRegion<D: Dialect> {
    enter: D::Operation,
    aliases: alloc::collections::BTreeSet<VariableId>,
    exits: Vec<StatementId>,
}

struct Rebuilder<'a, D: RecoverDialect + VerifyDialect> {
    source: &'a Function<D>,
    builder: FunctionBuilder<D>,
    statements: BTreeMap<StatementId, StatementId>,
    expressions: BTreeMap<ExpressionId, ExpressionId>,
    active_regions: Vec<ActiveRegion<D>>,
    for_loops: usize,
    selects: usize,
    regions: usize,
}

impl<'a, D: RecoverDialect + VerifyDialect> Rebuilder<'a, D> {
    fn statement_kind(&self, id: StatementId) -> Result<&'a StatementKind<D>> {
        self.source
            .statement(id)
            .map(Statement::kind)
            .ok_or_else(|| Error::InvalidConstruction("unresolvable statement identity".into()))
    }

    fn expression(&self, id: ExpressionId) -> Result<&'a Expression<D>> {
        self.source
            .expression(id)
            .ok_or_else(|| Error::InvalidConstruction("unresolvable expression identity".into()))
    }

    /// Whether one assignment can appear in a source-language expression
    /// clause rather than expanding into a statement sequence.
    fn assignment_is_single_expression(
        &self,
        target: ExpressionId,
        value: ExpressionId,
    ) -> Result<bool> {
        let target = self.expression(target)?;
        if !D::single_expression_assignment(target.value_type()) {
            return Ok(false);
        }
        let mut pending = Vec::new();
        pending.push(value);
        while let Some(expression) = pending.pop() {
            if let ExpressionKind::Operation {
                operation,
                operands,
            } = self.expression(expression)?.kind()
            {
                if !D::single_expression_operation(operation) {
                    return Ok(false);
                }
                pending.extend(operands.iter().copied());
            }
        }
        Ok(true)
    }

    fn rebuild_body(&mut self, ids: &[StatementId]) -> Result<Vec<StatementId>> {
        let mut out = Vec::new();
        let mut index = 0;
        while index < ids.len() {
            if self.suppress_region_exit(ids[index])? {
                index += 1;
                continue;
            }
            self.track_region_aliases(ids[index])?;
            if index + 1 < ids.len() {
                if let Some(region) = self.try_region(ids, index)? {
                    out.push(region);
                    index += 2;
                    continue;
                }
                if let Some(counted) = self.try_for(ids, index)? {
                    out.push(counted);
                    index += 2;
                    continue;
                }
            }
            out.push(self.rebuild_statement(ids[index])?);
            index += 1;
        }
        Ok(out)
    }

    /// Whether the statement is an exit of a region under construction;
    /// the recovered construct re-owns it.
    fn suppress_region_exit(&mut self, id: StatementId) -> Result<bool> {
        let StatementKind::Expression(expression) = self.statement_kind(id)? else {
            return Ok(false);
        };
        let ExpressionKind::Operation {
            operation,
            operands,
        } = self.expression(*expression)?.kind()
        else {
            return Ok(false);
        };
        let [released] = operands.as_slice() else {
            return Ok(false);
        };
        let ExpressionKind::Variable(released) = self.expression(*released)?.kind() else {
            return Ok(false);
        };
        let position = self.active_regions.iter().rposition(|region| {
            region.aliases.contains(released) && D::releases(&region.enter, operation)
        });
        if let Some(position) = position {
            self.active_regions[position].exits.push(id);
            return Ok(true);
        }
        Ok(false)
    }

    /// Follows the entered object through copies: an assignment from an
    /// alias adds its target to the region's alias set, and any other
    /// assignment of a tracked variable removes it.
    fn track_region_aliases(&mut self, id: StatementId) -> Result<()> {
        if self.active_regions.is_empty() {
            return Ok(());
        }
        let StatementKind::Assign { target, value } = self.statement_kind(id)? else {
            return Ok(());
        };
        let ExpressionKind::Variable(target) = self.expression(*target)?.kind() else {
            return Ok(());
        };
        let source = match self.expression(*value)?.kind() {
            ExpressionKind::Variable(source) => Some(*source),
            _ => None,
        };
        for region in &mut self.active_regions {
            match source {
                Some(source) if region.aliases.contains(&source) => {
                    region.aliases.insert(*target);
                }
                _ => {
                    region.aliases.remove(target);
                }
            }
        }
        Ok(())
    }

    fn rebuild_statement(&mut self, id: StatementId) -> Result<StatementId> {
        let kind = self.statement_kind(id)?;
        if let StatementKind::If {
            condition,
            then_body,
            else_body,
        } = kind
            && let Some(select) = self.try_select(id, *condition, then_body, else_body)?
        {
            return Ok(select);
        }
        let new_kind = self.rebuild_kind(kind)?;
        let new_id = self.builder.add_statement(new_kind, None)?;
        self.statements.insert(id, new_id);
        Ok(new_id)
    }

    fn rebuild_kind(&mut self, kind: &'a StatementKind<D>) -> Result<StatementKind<D>> {
        Ok(match kind {
            StatementKind::Expression(expression) => {
                StatementKind::Expression(self.rebuild_expression(*expression)?)
            }
            StatementKind::Assign { target, value } => StatementKind::Assign {
                target: self.rebuild_expression(*target)?,
                value: self.rebuild_expression(*value)?,
            },
            StatementKind::If {
                condition,
                then_body,
                else_body,
            } => StatementKind::If {
                condition: self.rebuild_expression(*condition)?,
                then_body: self.rebuild_body(then_body)?,
                else_body: self.rebuild_body(else_body)?,
            },
            StatementKind::While { condition, body } => StatementKind::While {
                condition: self.rebuild_expression(*condition)?,
                body: self.rebuild_body(body)?,
            },
            StatementKind::DoWhile { body, condition } => StatementKind::DoWhile {
                body: self.rebuild_body(body)?,
                condition: self.rebuild_expression(*condition)?,
            },
            StatementKind::Loop { body } => StatementKind::Loop {
                body: self.rebuild_body(body)?,
            },
            StatementKind::For {
                initializer,
                condition,
                update,
                body,
            } => StatementKind::For {
                initializer: self.rebuild_body(initializer)?,
                condition: condition
                    .map(|condition| self.rebuild_expression(condition))
                    .transpose()?,
                update: self.rebuild_body(update)?,
                body: self.rebuild_body(body)?,
            },
            StatementKind::Switch {
                scrutinee,
                cases,
                default_body,
            } => self.rebuild_switch(*scrutinee, cases, default_body)?,
            StatementKind::Break { label } => StatementKind::Break {
                label: label.clone(),
            },
            StatementKind::Continue { label } => StatementKind::Continue {
                label: label.clone(),
            },
            StatementKind::Return { values } => StatementKind::Return {
                values: values
                    .iter()
                    .map(|&value| self.rebuild_expression(value))
                    .collect::<Result<Vec<_>>>()?,
            },
            StatementKind::Labeled { label, body } => StatementKind::Labeled {
                label: label.clone(),
                body: self.rebuild_body(body)?,
            },
            StatementKind::Goto { label } => StatementKind::Goto {
                label: label.clone(),
            },
            StatementKind::Try {
                body,
                handlers,
                finally_body,
            } => StatementKind::Try {
                body: self.rebuild_body(body)?,
                handlers: handlers
                    .iter()
                    .map(|handler| self.rebuild_handler(handler))
                    .collect::<Result<Vec<_>>>()?,
                finally_body: self.rebuild_body(finally_body)?,
            },
            StatementKind::Region {
                operation,
                operands,
                body,
            } => StatementKind::Region {
                operation: operation.clone(),
                operands: operands
                    .iter()
                    .map(|&operand| self.rebuild_expression(operand))
                    .collect::<Result<Vec<_>>>()?,
                body: self.rebuild_body(body)?,
            },
        })
    }

    fn rebuild_switch(
        &mut self,
        scrutinee: ExpressionId,
        cases: &'a [super::SwitchArm<D>],
        default_body: &'a [StatementId],
    ) -> Result<StatementKind<D>> {
        Ok(StatementKind::Switch {
            scrutinee: self.rebuild_expression(scrutinee)?,
            cases: cases
                .iter()
                .map(|case| {
                    Ok(super::SwitchArm {
                        values: case.values.clone(),
                        body: self.rebuild_body(&case.body)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            default_body: self.rebuild_body(default_body)?,
        })
    }

    fn rebuild_handler(&mut self, handler: &'a Handler<D>) -> Result<Handler<D>> {
        Ok(Handler {
            kind: match &handler.kind {
                HandlerKind::Filter { filter_body } => HandlerKind::Filter {
                    filter_body: self.rebuild_body(filter_body)?,
                },
                other => other.clone(),
            },
            binding: handler.binding,
            caught_types: handler.caught_types.clone(),
            body: self.rebuild_body(&handler.body)?,
        })
    }

    fn rebuild_expression(&mut self, id: ExpressionId) -> Result<ExpressionId> {
        let node = self.expression(id)?;
        let kind = match node.kind() {
            ExpressionKind::Variable(variable) => ExpressionKind::Variable(*variable),
            ExpressionKind::Constant(constant) => ExpressionKind::Constant(constant.clone()),
            ExpressionKind::Operation {
                operation,
                operands,
            } => ExpressionKind::Operation {
                operation: operation.clone(),
                operands: operands
                    .iter()
                    .map(|&operand| self.rebuild_expression(operand))
                    .collect::<Result<Vec<_>>>()?,
            },
        };
        let new_id = self
            .builder
            .add_expression(kind, node.value_type().clone())?;
        self.expressions.insert(id, new_id);
        Ok(new_id)
    }
}

mod counted;
mod region;
mod select;
