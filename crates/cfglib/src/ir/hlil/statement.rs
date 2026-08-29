//! Structured HLIL statements.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use super::{Dialect, ExpressionId, StatementId, VariableId};

/// One structured statement.
///
/// Statements form strict trees rooted at the function body: every
/// statement is referenced by exactly one parent body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement<D: Dialect> {
    pub(super) id: StatementId,
    pub(super) kind: StatementKind<D>,
}

impl<D: Dialect> Statement<D> {
    /// Returns the stable statement identity.
    #[must_use]
    pub const fn id(&self) -> StatementId {
        self.id
    }

    /// Returns the statement shape.
    #[must_use]
    pub const fn kind(&self) -> &StatementKind<D> {
        &self.kind
    }
}

/// The universal structured-statement vocabulary.
///
/// Control shapes are library-owned; the open semantic vocabulary lives in
/// expressions, and [`Region`](Self::Region) is the escape hatch for
/// dialect statements that carry a body (`synchronized`, `lock`, `using`,
/// `with`, `unsafe`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatementKind<D: Dialect> {
    /// Evaluate an expression for its effects (calls, stores modeled as
    /// operations, throws).
    Expression(ExpressionId),

    /// Store `value` into the place named by `target` — a
    /// [`Variable`](super::ExpressionKind::Variable) occurrence or a
    /// dialect lvalue operation (dereference, field, element), never a
    /// constant.
    Assign {
        /// The place written.
        target: ExpressionId,
        /// The value stored.
        value: ExpressionId,
    },

    /// A conditional branch.
    If {
        /// The decided condition.
        condition: ExpressionId,
        /// Statements executed when the condition is true.
        then_body: Vec<StatementId>,
        /// Statements executed when the condition is false; empty for no
        /// `else`.
        else_body: Vec<StatementId>,
    },

    /// A pre-tested loop.
    While {
        /// The condition evaluated before every iteration.
        condition: ExpressionId,
        /// The loop body.
        body: Vec<StatementId>,
    },

    /// A post-tested loop.
    DoWhile {
        /// The loop body.
        body: Vec<StatementId>,
        /// The condition evaluated after every iteration.
        condition: ExpressionId,
    },

    /// An endless loop; iteration ends only through an inner transfer.
    Loop {
        /// The loop body.
        body: Vec<StatementId>,
    },

    /// A counted-style loop with explicit initializer and update sections.
    For {
        /// Statements executed once before the first test.
        initializer: Vec<StatementId>,
        /// The condition evaluated before every iteration; `None` iterates
        /// unconditionally.
        condition: Option<ExpressionId>,
        /// Statements executed after every iteration, before the next test.
        update: Vec<StatementId>,
        /// The loop body.
        body: Vec<StatementId>,
    },

    /// A multi-way branch over constant case values.
    ///
    /// Arms do not fall through: a body that ends transfers to the switch
    /// continuation. Source fallthrough lowers to explicit gotos.
    Switch {
        /// The dispatched value.
        scrutinee: ExpressionId,
        /// The labeled arms.
        cases: Vec<SwitchArm<D>>,
        /// The arm taken when no case matches; empty when the default
        /// transfers straight to the continuation.
        default_body: Vec<StatementId>,
    },

    /// Break out of the innermost enclosing loop or switch, or out of the
    /// enclosing [`Labeled`](Self::Labeled) statement named by `label`.
    Break {
        /// The named enclosing statement; `None` breaks the innermost
        /// loop or switch.
        label: Option<String>,
    },

    /// Continue the innermost enclosing loop, or the enclosing labeled
    /// loop named by `label`.
    Continue {
        /// The named enclosing loop; `None` continues the innermost loop.
        label: Option<String>,
    },

    /// Return from the function with zero or more values.
    Return {
        /// Returned values in result order.
        values: Vec<ExpressionId>,
    },

    /// A labeled statement group: the target of [`Goto`](Self::Goto) and of
    /// labeled [`Break`](Self::Break) / [`Continue`](Self::Continue).
    Labeled {
        /// The unique label name.
        label: String,
        /// The labeled body.
        body: Vec<StatementId>,
    },

    /// An unconditional transfer to the [`Labeled`](Self::Labeled)
    /// statement with the matching name — the structured residue for
    /// irreducible control flow.
    Goto {
        /// The target label name.
        label: String,
    },

    /// A protected region with handlers.
    Try {
        /// The protected body.
        body: Vec<StatementId>,
        /// Handler arms in dispatch order.
        handlers: Vec<Handler<D>>,
        /// Statements executed on every completion path; empty for no
        /// `finally`.
        finally_body: Vec<StatementId>,
    },

    /// A dialect-defined statement wrapping a body — `synchronized`,
    /// `lock`, `using`, `with`, and similar language regions.
    Region {
        /// The consumer-defined region operation.
        operation: D::Operation,
        /// Ordered operand expressions (e.g. the monitor or resource).
        operands: Vec<ExpressionId>,
        /// The wrapped body.
        body: Vec<StatementId>,
    },
}

/// One labeled arm of a [`StatementKind::Switch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchArm<D: Dialect> {
    /// The constant values selecting this arm; never empty (the default
    /// arm is stored separately).
    pub values: Vec<D::Constant>,
    /// The arm body.
    pub body: Vec<StatementId>,
}

/// One handler arm of a [`StatementKind::Try`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handler<D: Dialect> {
    /// The handler classification.
    pub kind: HandlerKind,
    /// The variable bound to the delivered exception at handler entry.
    pub binding: Option<VariableId>,
    /// The exception types this handler accepts (e.g. a multi-catch);
    /// empty for catch-all, fault, and filter handlers.
    pub caught_types: Vec<D::ValueType>,
    /// The handler body.
    pub body: Vec<StatementId>,
}

/// Classification of one [`Handler`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandlerKind {
    /// Catches the types named by
    /// [`caught_types`](Handler::caught_types).
    Catch,
    /// Catches any exception.
    CatchAll,
    /// Runs on exceptional entry only.
    Fault,
    /// A predicate decides whether this handler accepts the exception.
    Filter {
        /// The statements evaluating the predicate.
        filter_body: Vec<StatementId>,
    },
}

/// Whether a child statement sequence introduces a lexical body or belongs
/// to the containing statement's expression-like syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChildBodyKind {
    Lexical,
    Clause,
}

/// One child statement sequence and its declaration-scope role.
#[derive(Debug, Clone, Copy)]
pub(super) struct ChildBody<'a> {
    pub(super) statements: &'a [StatementId],
    pub(super) kind: ChildBodyKind,
}

impl<'a> ChildBody<'a> {
    const fn lexical(statements: &'a [StatementId]) -> Self {
        Self {
            statements,
            kind: ChildBodyKind::Lexical,
        }
    }

    const fn clause(statements: &'a [StatementId]) -> Self {
        Self {
            statements,
            kind: ChildBodyKind::Clause,
        }
    }
}

/// Every child statement body of one statement, in execution order.
pub(super) fn child_body_entries<D: Dialect>(kind: &StatementKind<D>) -> Vec<ChildBody<'_>> {
    match kind {
        StatementKind::Expression(_)
        | StatementKind::Assign { .. }
        | StatementKind::Break { .. }
        | StatementKind::Continue { .. }
        | StatementKind::Return { .. }
        | StatementKind::Goto { .. } => Vec::new(),
        StatementKind::If {
            then_body,
            else_body,
            ..
        } => alloc::vec![ChildBody::lexical(then_body), ChildBody::lexical(else_body)],
        StatementKind::While { body, .. }
        | StatementKind::DoWhile { body, .. }
        | StatementKind::Loop { body }
        | StatementKind::Labeled { body, .. }
        | StatementKind::Region { body, .. } => alloc::vec![ChildBody::lexical(body)],
        StatementKind::For {
            initializer,
            update,
            body,
            ..
        } => alloc::vec![
            ChildBody::clause(initializer),
            ChildBody::lexical(body),
            ChildBody::clause(update)
        ],
        StatementKind::Switch {
            cases,
            default_body,
            ..
        } => cases
            .iter()
            .map(|case| ChildBody::lexical(&case.body))
            .chain(core::iter::once(ChildBody::lexical(default_body)))
            .collect(),
        StatementKind::Try {
            body,
            handlers,
            finally_body,
        } => {
            let mut bodies = alloc::vec![ChildBody::lexical(body)];
            for handler in handlers {
                if let HandlerKind::Filter { filter_body } = &handler.kind {
                    bodies.push(ChildBody::lexical(filter_body));
                }
                bodies.push(ChildBody::lexical(&handler.body));
            }
            bodies.push(ChildBody::lexical(finally_body));
            bodies
        }
    }
}

/// Every child statement sequence without its declaration-scope role.
pub(super) fn child_bodies<D: Dialect>(kind: &StatementKind<D>) -> Vec<&[StatementId]> {
    child_body_entries(kind)
        .into_iter()
        .map(|body| body.statements)
        .collect()
}

/// Visits every expression directly referenced by one statement.
pub(super) fn expression_references<D: Dialect>(
    kind: &StatementKind<D>,
    visit: &mut impl FnMut(ExpressionId),
) {
    match kind {
        StatementKind::Expression(expression) => visit(*expression),
        StatementKind::Assign { target, value } => {
            visit(*target);
            visit(*value);
        }
        StatementKind::If { condition, .. }
        | StatementKind::While { condition, .. }
        | StatementKind::DoWhile { condition, .. }
        | StatementKind::Switch {
            scrutinee: condition,
            ..
        } => visit(*condition),
        StatementKind::For { condition, .. } => {
            if let Some(condition) = condition {
                visit(*condition);
            }
        }
        StatementKind::Return { values } => {
            for &value in values {
                visit(value);
            }
        }
        StatementKind::Region { operands, .. } => {
            for &operand in operands {
                visit(operand);
            }
        }
        StatementKind::Loop { .. }
        | StatementKind::Break { .. }
        | StatementKind::Continue { .. }
        | StatementKind::Labeled { .. }
        | StatementKind::Goto { .. }
        | StatementKind::Try { .. } => {}
    }
}
