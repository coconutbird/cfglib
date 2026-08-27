//! Consumer contracts that specialize generic RTL storage.

use core::fmt::Debug;

use crate::EdgeKind;
use crate::ir::dialect::Vocabulary;

use super::lift::LiftedStatement;
use super::types::ValueShape;

/// The canonical edge vocabulary for dialects with plain two-way
/// branching.
///
/// A dialect whose control flow is fallthrough plus conditional
/// true/false — no fused dispatch, no exceptional edges — can use this
/// as its [`Dialect::Edge`] (and its
/// [`mlil::Dialect::Edge`](crate::ir::mlil::Dialect::Edge)) and delegate
/// the trait's edge hooks to [`kind`](Self::kind) and
/// [`is_entry`](Self::is_entry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Edge {
    /// The synthetic root's unique entry edge.
    Entry,
    /// Sequential flow.
    Fall,
    /// Taken branch of a conditional.
    True,
    /// Not-taken branch of a conditional.
    False,
}

impl Edge {
    /// The structural classification of the edge.
    #[must_use]
    pub const fn kind(self) -> EdgeKind {
        match self {
            Self::Entry | Self::Fall => EdgeKind::Fallthrough,
            Self::True => EdgeKind::ConditionalTrue,
            Self::False => EdgeKind::ConditionalFalse,
        }
    }

    /// Whether the edge is the synthetic root's entry edge.
    #[must_use]
    pub const fn is_entry(self) -> bool {
        matches!(self, Self::Entry)
    }
}

/// Storage contract for one RTL semantic dialect.
///
/// The level-independent types — value types, effects, source
/// coordinates, variable roles, native storage — come from the
/// [`Vocabulary`] supertrait shared with the MLIL and HLIL dialects.
/// Storage locations are the vocabulary's `NativeVariable`: RTL operates
/// on raw language storage, before any variable recovery.
pub trait Dialect: Vocabulary {
    /// Pure typed operator applied by expressions.
    type Operator: Clone + Debug + Eq;
    /// Effect-bearing operation retained as a statement.
    type EffectOp: Clone + Debug + Eq;
    /// Exact caller-owned metadata stored on control-flow edges.
    type Edge: Clone + Debug + Eq;

    /// Returns a compact stable mnemonic for an operator.
    fn mnemonic(operator: &Self::Operator) -> &str;

    /// Returns a compact stable mnemonic for an effect operation.
    fn effect_mnemonic(operation: &Self::EffectOp) -> &str;

    /// Maps caller edge metadata to cfglib's structural edge kind.
    fn edge_kind(edge: &Self::Edge) -> EdgeKind;

    /// Returns whether an edge is the synthetic root's unique entry edge.
    fn is_entry_edge(edge: &Self::Edge) -> bool;
}

/// The lifting contract from RTL into a dialect's MLIL.
///
/// A consumer implements this on the same type that implements
/// [`crate::ir::mlil::Dialect`], so the two levels share one vocabulary
/// and one edge type by construction.
pub trait Lift: Dialect + crate::ir::mlil::Dialect<Edge = <Self as Dialect>::Edge> {
    /// The MLIL value type of one lifted web shape.
    fn value_type(shape: ValueShape) -> <Self as Vocabulary>::ValueType;

    /// The variable role of one lifted web.
    ///
    /// `storage` is the native location the web lives in, or `None` for
    /// a synthetic serialization temporary.
    fn web_role(
        storage: Option<&<Self as Vocabulary>::NativeVariable>,
    ) -> <Self as Vocabulary>::VariableRole;

    /// Builds the MLIL operation of one lifted statement.
    fn operation(statement: LiftedStatement<Self>)
    -> <Self as crate::ir::mlil::Dialect>::Operation;
}
