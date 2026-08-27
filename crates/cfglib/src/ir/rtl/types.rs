//! Scalar type classes and value shapes carried by RTL expressions.

/// The interpretation of one 32/64-bit lane.
///
/// `Bits` is the unknown interpretation: raw storage whose meaning no
/// operation has constrained yet. Type inference treats it as the
/// identity of unification, and a genuine conflict (float against
/// integer) collapses back to it — the consumer then renders explicit
/// reinterpretations at each access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScalarType {
    /// 32-bit IEEE-754 float.
    F32,
    /// 64-bit IEEE-754 float.
    F64,
    /// 32-bit signed integer.
    I32,
    /// 64-bit signed integer.
    I64,
    /// 32-bit unsigned integer.
    U32,
    /// 64-bit unsigned integer.
    U64,
    /// Boolean truth value.
    Bool,
    /// Uninterpreted bits.
    Bits,
}

impl ScalarType {
    /// Unifies two lane interpretations.
    ///
    /// Equal types unify to themselves and `Bits` yields to anything.
    /// Same-width signed/unsigned integers unify to the signed form —
    /// the conversion between them is value-representation-exact, so the
    /// consumer can spell it as an ordinary conversion. Any other pair
    /// is a genuine representation conflict and collapses to `Bits`.
    #[must_use]
    pub fn unify(self, other: Self) -> Self {
        match (self, other) {
            (a, b) if a == b => a,
            (Self::Bits, other) => other,
            (this, Self::Bits) => this,
            (Self::I32 | Self::U32, Self::U32 | Self::I32) => Self::I32,
            (Self::I64 | Self::U64, Self::U64 | Self::I64) => Self::I64,
            _ => Self::Bits,
        }
    }
}

/// The shape of one value: a scalar interpretation across `lanes` lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ValueShape {
    /// Lane interpretation.
    pub scalar: ScalarType,
    /// Number of lanes (1 for scalars).
    pub lanes: u8,
}

impl ValueShape {
    /// A one-lane shape.
    #[must_use]
    pub const fn scalar(scalar: ScalarType) -> Self {
        Self { scalar, lanes: 1 }
    }

    /// A multi-lane shape.
    #[must_use]
    pub const fn vector(scalar: ScalarType, lanes: u8) -> Self {
        Self { scalar, lanes }
    }
}
