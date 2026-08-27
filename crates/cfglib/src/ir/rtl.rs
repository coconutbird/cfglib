//! Generic register-transfer storage and its typed lifting into MLIL.
//!
//! RTL sits between a language-specific low-level IR and MLIL. Languages
//! whose native form is machine-shaped — raw storage locations written
//! lane-by-lane, untyped bit patterns, parallel multi-destination
//! instructions — express each native instruction here as one
//! [`Statement`]: a group of *parallel* typed transfers whose reads all
//! observe the pre-statement state. Languages that are already
//! variable-shaped skip RTL and build MLIL directly.
//!
//! cfglib deliberately does not define a generic LLIL: low-level IRs are
//! language-owned (a decoded shader program, JVM bytecode, a machine
//! instruction stream), and the existing minimal traits —
//! [`FlowControl`](crate::FlowControl), [`InstrInfo`](crate::InstrInfo),
//! [`Cfg`](crate::Cfg), [`SsaForm`](crate::SsaForm) — are their shared
//! surface. RTL is where those language forms converge.
//!
//! [`lift`] converts an RTL function into an MLIL builder by computing
//! per-lane SSA, uniting versions into def-use *webs* (φ operands with
//! results, co-written lanes with each other), inferring one scalar type
//! per web, and emitting one MLIL instruction per serialized transfer.
//! Webs make storage reuse harmless: one register reused for a float and
//! then a counter becomes two typed variables, and a parallel transfer's
//! reads never see its own writes because they reference prior versions.

mod dialect;
mod error;
mod expr;
mod function;
mod lift;
mod statement;
mod types;

pub use dialect::{Dialect, Lift};
pub use error::{Error, Result};
pub use expr::{Expr, Place};
pub use function::{Function, FunctionBuilder};
pub use lift::{LiftedStatement, Lifting, VarExpr, WebInfo, lift};
pub use statement::{Lane, Statement, StatementNode};
pub use types::{ScalarType, ValueShape};

#[cfg(test)]
mod tests;
