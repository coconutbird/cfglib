//! Generic mutable variables and point-specific typed occurrences.

use super::{Dialect, VariableId};

/// One declared MLIL variable before SSA renaming.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Variable<D: Dialect> {
    /// Stable dense identity.
    pub id: VariableId,
    /// Semantic role used by analyses and presentation.
    pub role: D::VariableRole,
    /// Optional source-native storage provenance.
    pub native: Option<D::NativeVariable>,
}

/// One variable occurrence paired with its type at that program point.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypedVariable<D: Dialect> {
    /// Mutable variable identity.
    pub variable: VariableId,
    /// Value type required or produced at this occurrence.
    pub value_type: D::ValueType,
}

impl<D: Dialect> TypedVariable<D> {
    /// Creates a typed variable occurrence.
    #[must_use]
    pub const fn new(variable: VariableId, value_type: D::ValueType) -> Self {
        Self {
            variable,
            value_type,
        }
    }
}
