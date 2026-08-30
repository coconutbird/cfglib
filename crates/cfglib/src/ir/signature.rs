//! Ordered function signatures over level-specific vocabularies.

extern crate alloc;

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// One function signature: ordered parameters and ordered return types.
///
/// The variable identity and value type are level-specific —
/// [`ir::mlil`](crate::ir::mlil) and [`ir::hlil`](crate::ir::hlil) each
/// instantiate this with their own variable identity and the shared
/// [`Vocabulary::ValueType`](crate::ir::dialect::Vocabulary::ValueType).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature<Variable, ValueType> {
    /// Parameter variables in declaration order.
    pub parameters: Vec<Variable>,
    /// Return value types in result order; empty for no return value.
    pub returns: Vec<ValueType>,
}

impl<Variable, ValueType> Signature<Variable, ValueType> {
    /// Creates a signature from ordered parameters and return types.
    #[must_use]
    pub const fn new(parameters: Vec<Variable>, returns: Vec<ValueType>) -> Self {
        Self {
            parameters,
            returns,
        }
    }
}

impl<Variable, ValueType> Signature<Variable, ValueType>
where
    Variable: Clone + Ord + fmt::Display,
{
    /// Reports every violation of the shared parameter rules.
    ///
    /// Each parameter must satisfy `declared` and may appear only once, in
    /// that checking order per parameter. Builders fail on the first message
    /// while verifiers record them all, so both stay in exact agreement.
    #[must_use]
    pub fn parameter_issues(&self, mut declared: impl FnMut(&Variable) -> bool) -> Vec<String> {
        let mut seen = BTreeSet::new();
        let mut issues = Vec::new();
        for parameter in &self.parameters {
            if !declared(parameter) {
                issues.push(format!("signature names undeclared parameter {parameter}"));
            }
            if !seen.insert(parameter.clone()) {
                issues.push(format!("signature repeats parameter {parameter}"));
            }
        }
        issues
    }
}

impl<Variable, ValueType> Default for Signature<Variable, ValueType> {
    fn default() -> Self {
        Self {
            parameters: Vec::new(),
            returns: Vec::new(),
        }
    }
}
