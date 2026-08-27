//! Consumer contracts that specialize generic RTL storage.

use core::fmt::Debug;

use crate::EdgeKind;
use crate::ir::dialect::Vocabulary;

use super::emission::{EdgeContext, Emission};
use super::error::Result;
use super::template::LiftedStatement;
use super::types::{Constraint, Shape};

/// The canonical edge vocabulary for dialects with plain two-way
/// branching.
///
/// A dialect whose control flow is fallthrough plus conditional
/// true/false — no fused dispatch, no exceptional edges — can use this
/// as its [`Dialect::Edge`] (and its
/// [`mlil::Dialect::Edge`](crate::ir::mlil::Dialect::Edge)) and delegate
/// the trait's edge hooks to [`kind`](Self::kind) and
/// [`is_entry`](Self::is_entry). Dialects with switches, exceptions, or
/// legacy continuations define their own edge type instead.
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
/// on raw language storage — shader registers, JVM locals and stack
/// slots, machine registers and flags, wasm value slots — before any
/// variable recovery.
pub trait Dialect: Vocabulary {
    /// The lane constraint domain web typing folds over.
    ///
    /// Numeric machine dialects use the provided
    /// [`ScalarType`](super::ScalarType); managed dialects supply their
    /// own domain covering references, null, uninitialized objects, and
    /// hierarchy-dependent merges.
    type Constraint: Constraint;
    /// Pure typed operator applied by expressions.
    type Operator: Clone + Debug + Eq;
    /// Effect-bearing operation retained as a statement.
    type EffectOp: Clone + Debug + Eq;
    /// Exact caller-owned metadata stored on control-flow edges — branch
    /// polarity, switch case values, handler catch types and order,
    /// legacy continuations.
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
/// by construction. The edge types stay independent: each level keeps
/// self-contained metadata — an RTL exceptional edge can name a
/// [`StatementId`](super::StatementId) while its MLIL counterpart names
/// an emitted instruction — and [`lift_edge`](Self::lift_edge)
/// translates between them. A dialect using one edge type for both
/// levels translates by cloning.
pub trait Lift: Dialect + crate::ir::mlil::Dialect {
    /// The MLIL value type of one lifted web shape.
    fn value_type(shape: Shape<Self::Constraint>) -> <Self as Vocabulary>::ValueType;

    /// The variable role of one lifted web.
    ///
    /// `storage` is the native location the web lives in, or `None` for
    /// a synthetic serialization temporary.
    fn web_role(
        storage: Option<&<Self as Vocabulary>::NativeVariable>,
    ) -> <Self as Vocabulary>::VariableRole;

    /// Translates one lifted statement into MLIL instructions.
    ///
    /// The simple, storage-flavored form is one line —
    /// [`Emission::single`] appends one instruction whose uses,
    /// definitions, throw site, and span all derive from the statement.
    /// Semantic dialects instead call [`Emission::append`] one or more
    /// times, staging through [`Emission::temporary`] where an expansion
    /// needs intermediate values, and the context validates read
    /// alignment and exceptional placement.
    ///
    /// # Errors
    ///
    /// Returns an error when the statement has no legal translation.
    fn emit(context: &mut Emission<'_, '_, Self>, statement: LiftedStatement<Self>) -> Result<()>;

    /// Translates one RTL edge into the lifted function's MLIL edge
    /// metadata.
    ///
    /// Runs after every instruction is emitted, so the context resolves
    /// statements to emitted MLIL instruction identities — an
    /// exceptional edge's payload can carry its exact throw site in the
    /// MLIL identity domain. A dialect sharing one edge type across
    /// both levels clones the metadata.
    #[must_use]
    fn lift_edge(
        edge: &<Self as Dialect>::Edge,
        context: &EdgeContext<'_>,
    ) -> <Self as crate::ir::mlil::Dialect>::Edge;
}
