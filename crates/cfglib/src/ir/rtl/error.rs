//! RTL construction and lowering errors.

extern crate alloc;

use alloc::string::String;
use core::fmt;

/// A construction or lift failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The function under construction violated a structural rule.
    InvalidConstruction(String),
    /// The lift into MLIL could not proceed.
    Lifting(String),
    /// A read/operand alignment broke while resolving a lifted
    /// expression against its HLIL operands.
    Resolution(String),
    /// The lowering from MLIL could not represent the semantics — a
    /// typed refusal, never a silent approximation.
    Lowering(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConstruction(message) => {
                write!(formatter, "invalid RTL construction: {message}")
            }
            Self::Lifting(message) => write!(formatter, "RTL lift: {message}"),
            Self::Resolution(message) => write!(formatter, "RTL read resolution: {message}"),
            Self::Lowering(message) => write!(formatter, "RTL lowering: {message}"),
        }
    }
}

impl core::error::Error for Error {}

/// RTL result alias.
pub type Result<T> = core::result::Result<T, Error>;
