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
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConstruction(message) => {
                write!(formatter, "invalid RTL construction: {message}")
            }
            Self::Lifting(message) => write!(formatter, "RTL lift: {message}"),
        }
    }
}

impl core::error::Error for Error {}

/// RTL result alias.
pub type Result<T> = core::result::Result<T, Error>;
