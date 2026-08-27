//! RTL statements: parallel transfers, effects, and control flow.

extern crate alloc;

use alloc::vec::Vec;

use crate::InstrInfo;
use crate::ir::dialect::Vocabulary;

use super::dialect::Dialect;
use super::expr::{Expr, Place};
use super::types::ScalarType;

/// One storage lane: a native location and a lane index within it.
pub type Lane<D> = (<D as Vocabulary>::NativeVariable, u8);

/// One native instruction expressed at the RTL level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement<D: Dialect> {
    /// Parallel transfers: every read observes pre-statement storage,
    /// no matter which assignment consumes it.
    Transfer {
        /// Destination places with their values, in native order.
        assignments: Vec<(Place<D>, Expr<D>)>,
        /// Observable effects beyond the storage writes.
        effects: Vec<<D as Vocabulary>::Effect>,
        /// Whether the instruction can transfer exceptionally.
        may_throw: bool,
    },
    /// An effect-bearing operation with pure operand values.
    Effect {
        /// The dialect effect operation.
        operation: D::EffectOp,
        /// Operand values in operation order.
        operands: Vec<Expr<D>>,
        /// Observable effects of the operation.
        effects: Vec<<D as Vocabulary>::Effect>,
        /// Whether the operation can transfer exceptionally.
        may_throw: bool,
    },
    /// A conditional transfer on one scalar condition; outcomes are the
    /// block's outgoing edges.
    Branch {
        /// The scalar branch condition.
        condition: Expr<D>,
    },
    /// A function return carrying result values.
    Return {
        /// Returned values in signature order.
        values: Vec<Expr<D>>,
    },
}

impl<D: Dialect> Statement<D> {
    /// Visits every read in deterministic statement order.
    ///
    /// The traversal order — assignments (or operands, or values) in
    /// order, each expression in pre-order — is the contract that keeps
    /// SSA use positions aligned with read nodes during lowering.
    pub fn for_each_read(
        &self,
        visit: &mut impl FnMut(&<D as Vocabulary>::NativeVariable, &[u8], ScalarType),
    ) {
        match self {
            Self::Transfer { assignments, .. } => {
                for (_, value) in assignments {
                    value.for_each_read(visit);
                }
            }
            Self::Effect { operands, .. } => {
                for operand in operands {
                    operand.for_each_read(visit);
                }
            }
            Self::Branch { condition } => condition.for_each_read(visit),
            Self::Return { values } => {
                for value in values {
                    value.for_each_read(visit);
                }
            }
        }
    }
}

/// One stored statement with its cached lane-level data dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementNode<D: Dialect> {
    pub(super) statement: Statement<D>,
    pub(super) span: Option<<D as Vocabulary>::SourceSpan>,
    pub(super) uses: Vec<Lane<D>>,
    pub(super) defs: Vec<Lane<D>>,
}

impl<D: Dialect> StatementNode<D> {
    pub(super) fn new(
        statement: Statement<D>,
        span: Option<<D as Vocabulary>::SourceSpan>,
    ) -> Self {
        let mut uses = Vec::new();
        statement.for_each_read(&mut |storage, lanes, _| {
            for &lane in lanes {
                uses.push((storage.clone(), lane));
            }
        });
        let mut defs = Vec::new();
        if let Statement::Transfer { assignments, .. } = &statement {
            for (place, _) in assignments {
                for &lane in &place.lanes {
                    defs.push((place.storage.clone(), lane));
                }
            }
        }
        Self {
            statement,
            span,
            uses,
            defs,
        }
    }

    /// The stored statement.
    #[must_use]
    pub const fn statement(&self) -> &Statement<D> {
        &self.statement
    }

    /// The source span the statement lowers from.
    #[must_use]
    pub const fn span(&self) -> Option<&<D as Vocabulary>::SourceSpan> {
        self.span.as_ref()
    }
}

impl<D: Dialect> InstrInfo for StatementNode<D> {
    type Variable = Lane<D>;

    fn uses(&self) -> &[Self::Variable] {
        &self.uses
    }

    fn defs(&self) -> &[Self::Variable] {
        &self.defs
    }
}
