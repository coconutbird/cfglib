//! Level-agnostic AST reconstruction from flat control-flow graphs.
//!
//! Lifting changes the control representation without changing the semantic
//! level of the instruction payload. [`AstNode::map_instructions`] is the
//! re-leveling hook once structure is recovered. [`lift_borrowed`] is the
//! zero-copy path for consumers that keep the source CFG alive; [`lift`]
//! produces an independently owned tree.

mod display;
mod lift;
mod node;
mod report;
mod visit;

pub use lift::{lift, lift_borrowed, lift_borrowed_with_report, lift_predicated, lift_with_report};
pub use node::{AstNode, CatchHandler, LoopKind, SwitchCase};
pub use report::{GotoDiagnostic, GotoReason, LiftReport};
