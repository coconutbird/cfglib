//! HLIL construction and verification failures.

extern crate alloc;

use alloc::string::String;

use core::fmt;

pub use crate::ir::verification::{VerificationIssue, VerificationReport};

/// Rendered level name for HLIL verification reports.
pub(super) const LEVEL: &str = "HLIL";

/// Error returned while constructing or validating HLIL.
#[derive(Debug)]
pub enum Error {
    /// Builder input names an invalid statement, expression, or variable.
    InvalidConstruction(String),
    /// A provenance span is empty, reversed, or outside the source model.
    InvalidProvenance(String),
    /// A lifted function's source representation cannot be translated.
    UnsupportedLift(String),
    /// A lowered function's statement shape cannot be translated.
    UnsupportedLower(String),
    /// Lowering produced an invalid MLIL function.
    Lowering(crate::ir::mlil::Error),
    /// The completed function violates one or more HLIL invariants.
    Verification(VerificationReport),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConstruction(message) => {
                write!(formatter, "invalid HLIL construction: {message}")
            }
            Self::InvalidProvenance(message) => {
                write!(formatter, "invalid HLIL provenance: {message}")
            }
            Self::UnsupportedLift(message) => {
                write!(formatter, "unsupported HLIL lift: {message}")
            }
            Self::UnsupportedLower(message) => {
                write!(formatter, "unsupported HLIL lowering: {message}")
            }
            Self::Lowering(error) => write!(formatter, "HLIL lowering failed: {error}"),
            Self::Verification(report) => report.fmt(formatter),
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Verification(report) => Some(report),
            Self::Lowering(error) => Some(error),
            Self::InvalidConstruction(_)
            | Self::InvalidProvenance(_)
            | Self::UnsupportedLift(_)
            | Self::UnsupportedLower(_) => None,
        }
    }
}

impl From<crate::ir::mlil::Error> for Error {
    fn from(error: crate::ir::mlil::Error) -> Self {
        Self::Lowering(error)
    }
}

impl From<VerificationReport> for Error {
    fn from(report: VerificationReport) -> Self {
        Self::Verification(report)
    }
}

impl From<crate::ir::provenance::ProvenanceError> for Error {
    fn from(error: crate::ir::provenance::ProvenanceError) -> Self {
        Self::InvalidProvenance(error.message().into())
    }
}

/// Result type returned by HLIL APIs.
pub type Result<T> = core::result::Result<T, Error>;
