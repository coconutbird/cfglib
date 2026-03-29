//! Higher-level analysis passes built on top of the CFG.

pub mod alias;
pub mod expr;
pub mod metrics;
pub mod pattern;
pub mod purity;
pub mod switch_table;
pub mod tailcall;
pub mod valuenumber;
