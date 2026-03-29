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

pub mod block;
pub mod builder;
pub mod cfg;
pub mod dominator;
pub mod dot;
pub mod edge;
pub mod flow;
pub mod traverse;

// Re-exports for convenience.
pub use block::{BasicBlock, BlockId};
pub use builder::CfgBuilder;
pub use cfg::Cfg;
pub use dominator::DominatorTree;
pub use edge::{Edge, EdgeId, EdgeKind};
pub use flow::{FlowControl, FlowEffect};
