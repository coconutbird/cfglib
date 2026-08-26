//! Generic high-level intermediate-language storage.
//!
//! HLIL is the structured, expression-oriented level of the IR stack:
//! statements form trees (conditionals, loops, switches, exception
//! regions) and values nest as typed expression trees, while a
//! caller-defined [`Dialect`] supplies the operation, constant, type,
//! effect, source, and variable vocabularies — the same neutrality
//! contract as [`ir::mlil`](crate::ir::mlil).
//!
//! Two front doors build HLIL:
//! - [`FunctionBuilder`] — checked bottom-up construction, the path for
//!   source-language frontends lowering syntax trees;
//! - [`lift_function`] — structuring and expression recovery over an MLIL
//!   function, the path for binary and bytecode frontends lifting upward.

mod builder;
mod dialect;
mod display;
mod error;
mod expression;
mod function;
mod identity;
mod lift;
mod statement;
mod variable;
mod verify;

pub use builder::FunctionBuilder;
pub use dialect::{Dialect, VerifyDialect};
pub use error::{Error, Result, VerificationIssue, VerificationReport};
pub use expression::{Expression, ExpressionKind};
pub use function::Function;
pub use identity::{EntityId, ExpressionId, StatementId, VariableId};
pub use lift::{LiftDialect, Lifted, LiftedFunction, lift_function};
pub use statement::{Handler, HandlerKind, Statement, StatementKind, SwitchArm};
pub use variable::Variable;

/// One source span mapped to one HLIL entity.
pub type ProvenanceEntry<D> = crate::ir::provenance::ProvenanceEntry<D, EntityId>;

/// Deterministic many-to-many source-to-HLIL provenance.
pub type ProvenanceMap<D> = crate::ir::provenance::ProvenanceMap<D, EntityId>;

/// Ordered parameter and return signature of one HLIL function.
pub type Signature<D> =
    crate::ir::signature::Signature<VariableId, <D as crate::ir::dialect::Vocabulary>::ValueType>;

#[cfg(test)]
mod tests;
