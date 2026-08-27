//! Scalar type classes and value shapes carried by RTL expressions.
//!
//! The scalar vocabulary is deliberately a closed, library-owned set: RTL
//! describes machine storage, and machine lane interpretations are a
//! finite alphabet shared by every ISA. Consumer-owned typing enters one
//! level up, where [`Lift::value_type`](super::Lift::value_type) maps each
//! web's [`ValueShape`] into the dialect's own MLIL type domain.

/// The interpretation of one storage lane.
///
/// `Bits` is the unknown interpretation: raw storage whose meaning no
/// operation has constrained yet. Reads that impose `Bits` (transports,
/// bitwise moves) leave type inference untouched; a web whose reads
/// genuinely conflict (float against integer) also resolves to `Bits`,
/// and the consumer then renders explicit reinterpretations at each
/// access. Inference itself runs on [`ScalarInference`], which keeps
/// "unconstrained" and "conflicted" distinct while observations fold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ScalarType {
    /// 16-bit IEEE-754 float.
    F16,
    /// 32-bit IEEE-754 float.
    F32,
    /// 64-bit IEEE-754 float.
    F64,
    /// 80-bit x87 extended-precision float.
    F80,
    /// 128-bit IEEE-754 float.
    F128,
    /// 8-bit signed integer.
    I8,
    /// 16-bit signed integer.
    I16,
    /// 32-bit signed integer.
    I32,
    /// 64-bit signed integer.
    I64,
    /// 128-bit signed integer.
    I128,
    /// 256-bit signed integer.
    I256,
    /// 512-bit signed integer.
    I512,
    /// 8-bit unsigned integer.
    U8,
    /// 16-bit unsigned integer.
    U16,
    /// 32-bit unsigned integer.
    U32,
    /// 64-bit unsigned integer.
    U64,
    /// 128-bit unsigned integer.
    U128,
    /// 256-bit unsigned integer.
    U256,
    /// 512-bit unsigned integer.
    U512,
    /// Boolean truth value of dialect-defined width.
    Bool,
    /// Uninterpreted bits.
    Bits,
}

impl ScalarType {
    /// The lane width in bits, when the interpretation fixes one.
    ///
    /// `Bool` and `Bits` leave the width to the storage and return
    /// `None`; reinterpretation validation treats an unknown width as
    /// compatible with anything.
    #[must_use]
    pub const fn width(self) -> Option<u32> {
        match self {
            Self::I8 | Self::U8 => Some(8),
            Self::F16 | Self::I16 | Self::U16 => Some(16),
            Self::F32 | Self::I32 | Self::U32 => Some(32),
            Self::F64 | Self::I64 | Self::U64 => Some(64),
            Self::F80 => Some(80),
            Self::F128 | Self::I128 | Self::U128 => Some(128),
            Self::I256 | Self::U256 => Some(256),
            Self::I512 | Self::U512 => Some(512),
            Self::Bool | Self::Bits => None,
        }
    }

    /// The number of 64-bit words one lane's bit pattern occupies in a
    /// constant — `width` rounded up to whole words, one word when the
    /// interpretation fixes no width.
    #[must_use]
    pub const fn words(self) -> usize {
        match self.width() {
            Some(width) => width.div_ceil(64) as usize,
            None => 1,
        }
    }

    /// The signed form of a same-width signed/unsigned integer pair.
    ///
    /// The conversion between the two is value-representation-exact, so
    /// inference merges them instead of conflicting; the consumer can
    /// spell the difference as an ordinary conversion.
    const fn integer_merge(self, other: Self) -> Option<Self> {
        match (self, other) {
            (Self::I8 | Self::U8, Self::U8 | Self::I8) => Some(Self::I8),
            (Self::I16 | Self::U16, Self::U16 | Self::I16) => Some(Self::I16),
            (Self::I32 | Self::U32, Self::U32 | Self::I32) => Some(Self::I32),
            (Self::I64 | Self::U64, Self::U64 | Self::I64) => Some(Self::I64),
            (Self::I128 | Self::U128, Self::U128 | Self::I128) => Some(Self::I128),
            (Self::I256 | Self::U256, Self::U256 | Self::I256) => Some(Self::I256),
            (Self::I512 | Self::U512, Self::U512 | Self::I512) => Some(Self::I512),
            _ => None,
        }
    }
}

/// Folding scalar-type inference over one web's observations.
///
/// A three-point lattice — unconstrained, one known interpretation, and
/// conflict — so a genuine conflict is never forgotten: unlike a pairwise
/// merge whose "unknown" and "conflict" share one value, `F32` then `U32`
/// then `F32` resolves to `Bits` here no matter the observation order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ScalarInference {
    /// No observation has constrained the interpretation yet.
    #[default]
    Unseen,
    /// Every observation so far agrees on one interpretation.
    Known(ScalarType),
    /// Observations imposed incompatible interpretations.
    Conflict,
}

impl ScalarInference {
    /// Folds one observed interpretation into the inference.
    ///
    /// `Bits` observations impose nothing and leave the state untouched.
    /// Same-width signed/unsigned integers merge to the signed form; any
    /// other disagreement is a conflict, and conflicts are permanent.
    pub fn observe(&mut self, scalar: ScalarType) {
        if scalar == ScalarType::Bits {
            return;
        }
        *self = match *self {
            Self::Unseen => Self::Known(scalar),
            Self::Known(known) if known == scalar => Self::Known(known),
            Self::Known(known) => match known.integer_merge(scalar) {
                Some(merged) => Self::Known(merged),
                None => Self::Conflict,
            },
            Self::Conflict => Self::Conflict,
        };
    }

    /// The inferred interpretation: unconstrained and conflicted webs
    /// both resolve to raw [`Bits`](ScalarType::Bits).
    #[must_use]
    pub const fn resolve(self) -> ScalarType {
        match self {
            Self::Known(scalar) => scalar,
            Self::Unseen | Self::Conflict => ScalarType::Bits,
        }
    }
}

/// The shape of one value: a scalar interpretation across `lanes` lanes.
///
/// A lane's bit pattern travels as [`ScalarType::words`] little-endian
/// 64-bit words in constants, so interpretations up to 512 bits stay one
/// scalar lane.
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
