//! HLIL variable declarations.

use super::{Dialect, VariableId};

/// One declared HLIL variable.
///
/// Occurrence types live on expressions ([`Expression::value_type`](super::Expression::value_type)),
/// so a variable legitimately holds differently typed values at different
/// program points; the declared type is the optional presentation-level
/// declaration when one is known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable<D: Dialect> {
    /// Stable dense identity.
    pub id: VariableId,
    /// Semantic role used by analyses and presentation.
    pub role: D::VariableRole,
    /// Optional source-native storage provenance.
    pub native: Option<D::NativeVariable>,
    /// Optional declared type; occurrence types remain authoritative.
    pub declared_type: Option<D::ValueType>,
}
