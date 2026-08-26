//! Consumer contracts that specialize generic MLIL storage.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::fmt::Debug;

use crate::ir::dialect::Vocabulary;
use crate::{EdgeKind, FlowEffect};

use super::{Function, Instruction, VariableId, VerificationIssue};

/// Cached control-flow and side-effect facts for one semantic operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstructionMetadata<E> {
    /// Observable effects beyond variable definitions.
    pub effects: Vec<E>,
    /// Intraprocedural control-flow behavior.
    pub flow: FlowEffect,
    /// Whether the instruction can transfer through an exceptional edge.
    pub may_throw: bool,
}

impl<E> InstructionMetadata<E> {
    /// Creates explicit metadata for one instruction.
    #[must_use]
    pub const fn new(effects: Vec<E>, flow: FlowEffect, may_throw: bool) -> Self {
        Self {
            effects,
            flow,
            may_throw,
        }
    }
}

/// Complete storage contract for one MLIL semantic dialect.
///
/// A dialect is a semantic vocabulary, not a source ISA. Multiple frontends
/// can therefore converge on the same dialect while retaining different
/// source identities and native-variable provenance. The level-independent
/// types — value types, effects, source coordinates, variable roles — come
/// from the [`Vocabulary`] supertrait shared with
/// [`ir::hlil::Dialect`](crate::ir::hlil::Dialect); this trait adds the
/// flat-control-flow contract MLIL storage needs.
pub trait Dialect: Vocabulary {
    /// Semantic operation stored by an instruction.
    type Operation: Clone + Debug + Eq;
    /// Exact caller-owned metadata stored on control-flow edges.
    type Edge: Clone + Debug + Eq;

    /// Classifies and caches the effects of one operation.
    fn instruction_metadata(
        operation: &Self::Operation,
        may_throw: bool,
    ) -> InstructionMetadata<Self::Effect>;

    /// Returns a compact stable mnemonic for an operation.
    fn mnemonic(operation: &Self::Operation) -> &str;

    /// Maps caller edge metadata to cfglib's structural edge kind.
    fn edge_kind(edge: &Self::Edge) -> EdgeKind;

    /// Returns whether an edge is the synthetic root's unique entry edge.
    fn is_entry_edge(edge: &Self::Edge) -> bool;
}

/// Optional semantic hooks used by reusable MLIL analyses.
pub trait AnalysisDialect: Dialect {
    /// Constant domain used by propagation and recovered expressions.
    type Constant: Clone + Debug + Eq;
    /// Pure operator identity retained in recovered expression trees.
    type ExpressionOperator: Clone + Debug + Eq;
    /// Statically known call-target identity.
    type Callee: Clone + Debug + Ord;

    /// Returns whether an operation is a pure one-use, one-definition copy.
    fn is_copy(operation: &Self::Operation) -> bool;

    /// Returns the pure expression operator represented by an operation.
    fn expression_operator(operation: &Self::Operation) -> Option<Self::ExpressionOperator>;

    /// Returns a constant directly materialized by an operation.
    fn constant(operation: &Self::Operation) -> Option<Self::Constant>;

    /// Folds an instruction using constants known at its input.
    fn fold_constant(
        instruction: &Instruction<Self>,
        known: &BTreeMap<VariableId, Self::Constant>,
    ) -> Option<(VariableId, Self::Constant)>;

    /// Returns the statically known target of a call operation.
    fn callee(operation: &Self::Operation) -> Option<Self::Callee>;
}

/// Dialect-specific semantic and typing verification.
pub trait VerifyDialect: Dialect {
    /// Appends every dialect invariant violation in deterministic order.
    fn verify(function: &Function<Self>, issues: &mut Vec<VerificationIssue>);
}
