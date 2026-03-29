//! Generic control-flow graph library for binary analysis.
//!
//! This crate provides an ISA-agnostic [`Cfg<I>`] data structure and a
//! builder that converts any flat instruction stream into a structured
//! control-flow graph. The only requirement is that the instruction type
//! implements the [`FlowControl`] trait.
//!
//! # Quick start
//!
//! ```rust,no_run
//! use cfglib::{Cfg, CfgBuilder, FlowControl, FlowEffect};
//!
//! // 1. Implement FlowControl for your instruction type.
//! // 2. Build the CFG:
//! // let cfg = CfgBuilder::build(instructions)?;
//! // 3. Traverse, compute dominators, or export to DOT:
//! // println!("{}", cfg.to_dot());
//! ```

#![no_std]
#![warn(missing_docs)]
extern crate alloc;

// Core types.
pub mod block;
pub mod builder;
pub mod cfg;
pub mod edge;
pub mod flow;
pub mod purity;
pub mod region;

// Submodules.
pub mod analysis;
pub mod ast;
pub mod dataflow;
pub mod graph;
pub mod linearize;
pub mod ssa;
pub mod transform;

// Shared test utilities.
#[cfg(test)]
pub(crate) mod test_util;

// Re-exports — core types for building and traversing CFGs.
pub use block::{BasicBlock, BlockId, Guard};
pub use builder::{BuildError, CfgBuilder};
pub use cfg::Cfg;
pub use edge::{CallSite, Edge, EdgeId, EdgeKind};
pub use flow::{FlowControl, FlowEffect};

// Re-exports — data flow and analysis.
pub use dataflow::fixpoint::{Direction, FixpointResult, Problem};
pub use dataflow::{InstrInfo, Location, ProgramPoint};
pub use purity::Effect;

// Re-exports — graph analysis.
pub use graph::dominator::DominatorTree;
pub use graph::structure::{BackEdge, NaturalLoop, detect_loops, find_back_edges};

// Re-exports — transforms and linearization.
pub use linearize::{BlockOrder, Emitter, LinearInst, linearize};
pub use region::{Handler, HandlerKind, Region, RegionId};
pub use transform::{
    dead_code_elimination, merge_blocks, remove_empty_blocks, remove_unreachable, simplify,
    split_critical_edges,
};

// Re-exports — SSA.
pub use ssa::{DominanceFrontiers, PhiMap, PhiNode, insert_phis};

// Re-exports — interval analysis.
pub use graph::interval::{Interval, IntervalAnalysis, interval_analysis};

// Re-exports — loop canonicalization.
pub use graph::structure::{CanonicalLoop, canonicalize_loops, insert_preheader, loop_exit_blocks};

// Re-exports — SCC and CDG.
pub use graph::cdg::ControlDependenceGraph;
pub use graph::scc::{Scc, SccResult, tarjan_scc};

// Re-exports — constant propagation.
pub use dataflow::constprop::{ConstPropProblem, ConstValue, ConstantFolder, constant_propagation};
