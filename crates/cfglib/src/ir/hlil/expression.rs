//! Typed HLIL expression-tree nodes.

extern crate alloc;

use alloc::vec::Vec;

use super::{Dialect, ExpressionId, VariableId};

/// One typed expression-tree node.
///
/// Expressions form strict trees: every node is referenced by exactly one
/// parent (a statement or one enclosing expression), so each node is one
/// program-point occurrence with its own point-specific type — mirroring
/// MLIL's typed occurrences. A value read twice is two nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expression<D: Dialect> {
    pub(super) id: ExpressionId,
    pub(super) kind: ExpressionKind<D>,
    pub(super) value_type: D::ValueType,
}

impl<D: Dialect> Expression<D> {
    /// Returns the stable expression identity.
    #[must_use]
    pub const fn id(&self) -> ExpressionId {
        self.id
    }

    /// Returns the expression shape.
    #[must_use]
    pub const fn kind(&self) -> &ExpressionKind<D> {
        &self.kind
    }

    /// Returns the type of the value this occurrence produces.
    #[must_use]
    pub const fn value_type(&self) -> &D::ValueType {
        &self.value_type
    }
}

/// The shape of one expression node.
///
/// The library owns only the three universal shapes; everything else —
/// calls, memory access, arithmetic, conversions, allocation — is a dialect
/// [`Operation`](Dialect::Operation) over operand expressions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpressionKind<D: Dialect> {
    /// One read occurrence of a variable.
    Variable(VariableId),
    /// One dialect constant literal.
    Constant(D::Constant),
    /// One dialect operation over ordered operand expressions.
    Operation {
        /// The consumer-defined semantic operation.
        operation: D::Operation,
        /// Ordered operand expressions.
        operands: Vec<ExpressionId>,
    },
}
