//! Consumer contracts that specialize generic HLIL storage.

extern crate alloc;

use alloc::vec::Vec;
use core::fmt::Debug;

use crate::ir::dialect::Vocabulary;

use super::{Function, VerificationIssue};

/// Complete storage contract for one HLIL semantic dialect.
///
/// The structured statement shapes (assignments, conditionals, loops,
/// switches, exception regions) are universal and library-owned; everything
/// semantic — the expression operation vocabulary, constants, value types,
/// effects, source coordinates, and variable identities — stays
/// consumer-defined. The level-independent types come from the
/// [`Vocabulary`] supertrait shared with
/// [`ir::mlil::Dialect`](crate::ir::mlil::Dialect), so a consumer
/// implementing both level dialects on one type states them once.
pub trait Dialect: Vocabulary {
    /// Semantic operation applied to ordered operand expressions: calls,
    /// loads, stores-as-places, arithmetic, field and element access —
    /// the whole open vocabulary of the consumer's language or machine.
    type Operation: Clone + Debug + Eq;
    /// Constant domain materialized by literal expressions and named by
    /// switch-case values.
    type Constant: Clone + Debug + Eq;

    /// Returns a compact stable mnemonic for an operation.
    fn mnemonic(operation: &Self::Operation) -> &str;

    /// Writes one constant for pseudocode rendering.
    ///
    /// The default renders the constant's `Debug` form; dialects override
    /// this for presentation-quality literals.
    ///
    /// # Errors
    ///
    /// Propagates formatter errors.
    fn fmt_constant(
        formatter: &mut core::fmt::Formatter<'_>,
        constant: &Self::Constant,
    ) -> core::fmt::Result {
        write!(formatter, "{constant:?}")
    }
}

/// Dialect-specific semantic and typing verification.
pub trait VerifyDialect: Dialect {
    /// Appends every dialect invariant violation in deterministic order.
    fn verify(function: &Function<Self>, issues: &mut Vec<VerificationIssue>);
}
