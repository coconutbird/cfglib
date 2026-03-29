//! Generic control-flow graph library for binary analysis.
//!
//! This crate provides an ISA-agnostic [`Cfg<I>`] data structure and a
//! builder that converts any flat instruction stream into a structured
//! control-flow graph. The only requirement is that the instruction type
//! implements the [`FlowControl`] trait.
//!
//! # Quick start
//!
//! ```ignore
//! use cfglib::{Cfg, CfgBuilder, FlowControl, FlowEffect};
//!
//! // 1. Implement FlowControl for your instruction type.
//! // 2. Build the CFG:
//! let cfg = CfgBuilder::build(instructions);
//! // 3. Traverse, compute dominators, or export to DOT:
//! println!("{}", cfg.to_dot());
//! ```

#![no_std]
extern crate alloc;

// Core types.
pub mod block;
pub mod builder;
pub mod cfg;
pub mod edge;
pub mod flow;
pub mod purity;

// Submodules.
pub mod ast;
pub mod dataflow;
pub mod graph;

// Re-exports for convenience.
pub use ast::{AstNode, lift};
pub use block::{BasicBlock, BlockId};
pub use builder::CfgBuilder;
pub use cfg::Cfg;
pub use dataflow::{DataDeps, DefSite, Location, UseSite};
pub use dataflow::defuse::DefUseChains;
pub use dataflow::fixpoint::{Direction, FixpointResult, Problem};
pub use dataflow::liveness::Liveness;
pub use dataflow::reaching::ReachingDefs;
pub use edge::{Edge, EdgeId, EdgeKind};
pub use flow::{FlowControl, FlowEffect};
pub use graph::dominator::DominatorTree;
pub use graph::structure::NaturalLoop;
pub use purity::{Effect, Purity, SideEffects};
