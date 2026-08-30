//! MLIL construction and verification failures.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// One independently actionable MLIL verification failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationIssue {
    /// Human-readable description of the violated invariant.
    pub message: String,
}

impl VerificationIssue {
    /// Creates one verification issue.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for VerificationIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

/// Complete deterministic result of MLIL verification.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerificationReport {
    /// All structural, semantic, typing, and provenance failures.
    pub issues: Vec<VerificationIssue>,
}

impl VerificationReport {
    /// Returns whether the function satisfies every MLIL invariant.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.issues.is_empty()
    }

    /// Returns the number of independent failures.
    #[must_use]
    pub fn issue_count(&self) -> usize {
        self.issues.len()
    }
}

impl fmt::Display for VerificationReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MLIL verification failed")?;
        for issue in &self.issues {
            write!(formatter, "; {issue}")?;
        }
        Ok(())
    }
}

impl core::error::Error for VerificationReport {}

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
