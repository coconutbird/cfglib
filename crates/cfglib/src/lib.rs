//! Generic control-flow graph library for binary analysis.
//!
//! This crate provides an ISA-agnostic [`Cfg<I>`] data structure and a
//! suite of analyses, transforms, and visualization tools for working
//! with control-flow graphs. The only requirement is that the
//! instruction type `I` implements the [`FlowControl`] trait.
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
//!
//! # Trait hierarchy
//!
//! Instruction types implement progressively richer traits depending on
//! which analyses they want to use:
//!
//! ```text
//! FlowControl              (required — control-flow classification)
//!   └─ SwitchCandidate     (optional — switch table recovery)
//!
//! InstrInfo                (optional — defs/uses/effects for dataflow)
//!   ├─ CopySource          (optional — copy propagation)
//!   ├─ ConstantFolder      (optional — constant propagation)
//!   └─ ExprInstr           (optional — expression tree recovery)
//! ```
//!
//! Additionally, [`Problem`] is the trait for pluggable dataflow
//! analyses, and [`Emitter`] is the trait for linearization output.

#![no_std]
#![warn(missing_docs)]
extern crate alloc;

// ── Modules ─────────────────────────────────────────────────────────

// Core types.
pub mod block;
pub mod builder;
pub mod cfg;
pub mod edge;
pub mod flow;
pub mod purity;
pub mod region;

// Graph algorithms.
pub mod graph;

// Dataflow framework and analyses.
pub mod dataflow;

// Higher-level analyses.
pub mod analysis;

// AST lifting / structural recovery.
pub mod ast;

// SSA construction.
pub mod ssa;

// Transforms and linearization.
pub mod linearize;
pub mod transform;

// Shared test utilities (crate-internal).
#[cfg(test)]
pub(crate) mod test_util;

// ── Re-exports: Core ────────────────────────────────────────────────

pub use block::{BasicBlock, BlockId, Guard};
pub use builder::{BuildError, CfgBuilder};
pub use cfg::Cfg;
pub use edge::{CallSite, Edge, EdgeId, EdgeKind};
pub use flow::{FlowControl, FlowEffect};
pub use purity::Effect;
pub use region::{Handler, HandlerKind, Region, RegionId};

// ── Re-exports: Dataflow framework ──────────────────────────────────

pub use dataflow::fixpoint::{Direction, FixpointResult, Problem};
pub use dataflow::{InstrInfo, Location, ProgramPoint};

// ── Re-exports: Graph algorithms ────────────────────────────────────

pub use graph::cdg::ControlDependenceGraph;
pub use graph::dominator::DominatorTree;
pub use graph::interval::{Interval, IntervalAnalysis, interval_analysis};
pub use graph::scc::{Scc, SccResult, tarjan_scc};
pub use graph::structure::{
    BackEdge, CanonicalLoop, NaturalLoop, canonicalize_loops, detect_loops, find_back_edges,
    insert_preheader, loop_exit_blocks,
};

// ── Re-exports: SSA ─────────────────────────────────────────────────

pub use ssa::{DominanceFrontiers, PhiMap, PhiNode, insert_phis};

// ── Re-exports: Analyses ────────────────────────────────────────────

pub use analysis::expr::{
    BlockExprTrees, ExprInstr, ExprNode, recover_block_expressions, recover_expressions,
};
pub use dataflow::constprop::{ConstPropProblem, ConstValue, ConstantFolder, constant_propagation};
pub use dataflow::copyprop::{CopyPropResult, CopySource, copy_propagation};

// ── Re-exports: Transforms & linearization ──────────────────────────

pub use linearize::{BlockOrder, Emitter, LinearInst, linearize};
pub use transform::{
    dead_code_elimination, merge_blocks, remove_empty_blocks, remove_unreachable, simplify,
    split_critical_edges,
};
