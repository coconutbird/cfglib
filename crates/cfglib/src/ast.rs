//! AST reconstruction — lifting a flat CFG into structured control flow.

mod lift;
mod node;

pub use lift::{lift, lift_predicated};
pub use node::{AstNode, CatchHandler, SwitchCase};
