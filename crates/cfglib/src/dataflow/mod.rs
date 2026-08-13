//! Data flow analysis framework.
//!
//! Provides generic infrastructure for computing data flow properties
//! over a [`Cfg`](crate::Cfg):
//!
//! - **Reaching definitions** — which writes can reach a given point
//! - **Liveness** — which variables are live at each point
//! - **Def-use / use-def chains** — linking writers to readers
//!
//! # Usage
//!
//! Implement [`InstrInfo`] for your instruction type to declare which
//! IR variables each instruction reads from and writes to, then run any
//! of the provided analyses. Variables are supplied by the IR adapter;
//! the framework does not impose a register numbering scheme.

pub mod abs_int;
pub mod constprop;
pub mod copyprop;
pub mod defuse;
pub mod fixpoint;
pub mod liveness;
pub mod memssa;
pub mod node_fixpoint;
pub mod phi_web;
pub mod reaching;
pub mod sccp;
pub mod ssa;
pub mod ssa_destruct;

use crate::block::BlockId;

/// An IR-defined identity that participates in data-flow analysis.
///
/// This blanket trait accepts any ordered, cloneable type. An adapter can
/// therefore use a compact integer, an x86 register/flag enum, a shader
/// register-component structure, or another native identity directly.
///
/// The identity must describe the atomic unit tracked by the analyses. For
/// architectures with overlapping resources, such as x86 subregisters, the
/// adapter is responsible for exposing canonical units or all affected units.
pub trait VariableId: Clone + Ord {}

impl<T: Clone + Ord> VariableId for T {}

/// Trait that an instruction type implements to expose its data
/// dependencies (which variables it reads and writes).
///
/// This is the data-flow counterpart of [`FlowControl`](crate::FlowControl),
/// which classifies control-flow effects.
///
/// Returns slices rather than `Vec`s to avoid heap allocations in
/// hot-path fixpoint iteration. The instruction is expected to own (or
/// borrow from its own storage) the def/use sets it reports; adapters over
/// external structures typically materialise them once at construction.
pub trait InstrInfo {
    /// IR variable identity used by this instruction representation.
    type Variable: VariableId;

    /// Variables that this instruction **reads** (uses).
    fn uses(&self) -> &[Self::Variable];

    /// Variables that this instruction **writes** (defines).
    fn defs(&self) -> &[Self::Variable];
}

/// Opt-in: instructions whose execution is predicated on a condition
/// variable (ARM IT blocks, GPU wave predication, x86 CMOV, guarded
/// statements).
///
/// The predicate is expressed in the consumer's own variable domain.
/// [`lift_predicated`](crate::lift_predicated) regionises maximal runs of
/// same-predicate instructions into
/// [`AstNode::Guarded`](crate::ast::node::AstNode::Guarded) nodes.
pub trait Predicated: InstrInfo {
    /// The guarding variable and the polarity under which the instruction
    /// executes. `None` for unconditionally executed instructions.
    fn predicate(&self) -> Option<(Self::Variable, bool)>;
}

/// Opt-in: instructions that declare side effects beyond their def/use sets.
///
/// The effect vocabulary is consumer-typed — machine memory and I/O for a
/// binary adapter, GC allocation / panics / channel sends for a source
/// language — the library imposes none. Purity analysis and dead-code
/// elimination consume this trait; requiring it there means a consumer must
/// state an effect vocabulary before code with possible side effects can be
/// classified or deleted.
pub trait EffectInfo: InstrInfo {
    /// Consumer effect vocabulary.
    type Effect: Clone + Ord;

    /// Side effects of this instruction. An empty slice means the
    /// instruction is pure.
    fn effects(&self) -> &[Self::Effect];
}

/// A point in the program identified by a (block, instruction-index) pair.
///
/// Used to refer to both definition sites and use sites in data flow
/// analyses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProgramPoint {
    /// The block containing the instruction.
    pub block: BlockId,
    /// Index of the instruction within the block.
    pub inst_idx: usize,
}

impl core::fmt::Display for ProgramPoint {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}:{}", self.block, self.inst_idx)
    }
}

/// Alias for [`ProgramPoint`] used in definition contexts.
pub type DefSite = ProgramPoint;
/// Alias for [`ProgramPoint`] used in use-site contexts.
pub type UseSite = ProgramPoint;
