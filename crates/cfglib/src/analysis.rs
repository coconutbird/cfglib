//! Higher-level analysis passes built on top of the CFG.

pub mod alias;
pub mod dead_code;
pub mod expr;
pub mod metrics;
pub mod pattern;
pub mod profile;
pub mod purity;
pub mod switch_table;
pub mod tail_call;
pub mod value_numbering;
