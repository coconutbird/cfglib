//! Stable identities for HLIL entities.

use crate::identity::define_dense_id;

define_dense_id! {
    /// Stable identity of one HLIL variable.
    pub struct VariableId(u32);
    display = "v";
    raw;
}

define_dense_id! {
    /// Stable identity of one HLIL expression within a function.
    pub struct ExpressionId(u32);
    display = "e";
    raw;
}

define_dense_id! {
    /// Stable identity of one HLIL statement within a function.
    pub struct StatementId(u32);
    display = "s";
    raw;
}

/// Stable identity of an HLIL entity that can originate from source code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntityId {
    /// Structured statement.
    Statement(StatementId),
    /// Expression-tree node.
    Expression(ExpressionId),
    /// Declared variable.
    Variable(VariableId),
}
