//! MLIL construction and verification failures.

extern crate alloc;

use alloc::string::String;

use core::fmt;

pub use crate::ir::verification::{VerificationIssue, VerificationReport};

/// Rendered level name for MLIL verification reports.
pub(super) const LEVEL: &str = "MLIL";

/// Error returned while constructing or validating MLIL.
#[derive(Debug)]
pub enum Error {
    /// Builder input names an invalid block, variable, or instruction.
    InvalidConstruction(String),
    /// A provenance span is empty, reversed, or outside the source model.
    InvalidProvenance(String),
    /// The completed function violates one or more MLIL invariants.
    Verification(VerificationReport),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConstruction(message) => {
                write!(formatter, "invalid MLIL construction: {message}")
            }
            Self::InvalidProvenance(message) => {
                write!(formatter, "invalid MLIL provenance: {message}")
            }
            Self::Verification(report) => report.fmt(formatter),
        }
    }
}

impl core::error::Error for Error {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Verification(report) => Some(report),
            Self::InvalidConstruction(_) | Self::InvalidProvenance(_) => None,
        }
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

/// Result type returned by MLIL APIs.
pub type Result<T> = core::result::Result<T, Error>;
