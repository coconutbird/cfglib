//! Level-independent semantic vocabulary shared by IR dialects.

use core::fmt::Debug;
use core::hash::Hash;

/// Level-independent semantic vocabulary shared by every IR level.
///
/// A dialect describes one consumer-owned semantic world. The vocabulary
/// carried here — value types, observable effects, source coordinates, and
/// variable identities — is meaningful at every representation level, so
/// [`ir::mlil::Dialect`](crate::ir::mlil::Dialect) and
/// [`ir::hlil::Dialect`](crate::ir::hlil::Dialect) both require it. A
/// consumer that implements several level dialects on one type therefore
/// states these types once, and cross-level passes (such as MLIL-to-HLIL
/// lifting) share value types, effects, variables, and source coordinates
/// without any conversion.
pub trait Vocabulary: Clone + Debug + Eq + 'static {
    /// Point-specific value type attached to typed occurrences.
    type ValueType: Clone + Debug + Eq + Hash + Ord;
    /// Observable effect vocabulary used by purity and dead-code analysis.
    type Effect: Clone + Debug + Eq + Ord;
    /// Identity and coordinate system of the source function.
    type Source: Clone + Debug + Eq;
    /// One ordered source span used by many-to-many provenance.
    type SourceSpan: Clone + Debug + Eq + Ord;
    /// One source point queried against provenance spans.
    type SourcePoint: Clone + Debug + Eq;
    /// Semantic role assigned to a mutable variable.
    type VariableRole: Clone + Debug + Eq + Hash;
    /// Optional source-native storage identity retained for a variable.
    type NativeVariable: Clone + Debug + Eq + Hash + Ord;

    /// Returns whether a source span is empty or reversed.
    fn span_is_empty(span: &Self::SourceSpan) -> bool;

    /// Returns whether a source span contains a point.
    fn span_contains(span: &Self::SourceSpan, point: &Self::SourcePoint) -> bool;
}
