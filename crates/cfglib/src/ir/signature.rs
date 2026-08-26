//! Ordered function signatures over level-specific vocabularies.

extern crate alloc;

use alloc::vec::Vec;

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

impl<Variable, ValueType> Default for Signature<Variable, ValueType> {
    fn default() -> Self {
        Self {
            parameters: Vec::new(),
            returns: Vec::new(),
        }
    }
}
