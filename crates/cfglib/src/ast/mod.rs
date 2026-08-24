//! AST reconstruction — lifting a flat CFG into structured control flow.

pub mod lift;
pub mod node;

pub use lift::{lift, lift_predicated};
pub use node::{AstNode, CatchHandler, SwitchCase};
