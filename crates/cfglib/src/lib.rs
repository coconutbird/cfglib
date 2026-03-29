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

// Shared test utilities.
#[cfg(test)]
pub(crate) mod test_util;

// Re-exports — core types for building and traversing CFGs.
pub use block::{BasicBlock, BlockId};
pub use builder::{BuildError, CfgBuilder};
pub use cfg::Cfg;
pub use edge::{Edge, EdgeId, EdgeKind};
pub use flow::{FlowControl, FlowEffect};

// Re-exports — data flow and analysis.
pub use dataflow::{InstrInfo, Location, ProgramPoint};
pub use purity::Effect;
