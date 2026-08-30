//! Verification vocabulary shared by every IR level.
//!
//! MLIL and HLIL run different invariant suites but report them identically:
//! a flat list of independently actionable [`VerificationIssue`]s inside one
//! deterministic [`VerificationReport`] that names its level only when
//! rendered. Level-specific `Error` enums wrap the report; the vocabulary
//! itself is level-free so pipeline stages can consume any level's report
//! uniformly.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// One independently actionable verification failure.
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

/// Complete deterministic result of one IR level's verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationReport {
    level: &'static str,
    /// All structural, semantic, typing, and provenance failures.
    pub issues: Vec<VerificationIssue>,
}

impl VerificationReport {
    /// Creates a report for the named IR level's verification run.
    #[must_use]
    pub const fn new(level: &'static str, issues: Vec<VerificationIssue>) -> Self {
        Self { level, issues }
    }

    /// IR level whose invariants were checked, as rendered in failures.
    #[must_use]
    pub const fn level(&self) -> &'static str {
        self.level
    }

    /// Returns whether the function satisfies every checked invariant.
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
        write!(formatter, "{} verification failed", self.level)?;
        for issue in &self.issues {
            write!(formatter, "; {issue}")?;
        }
        Ok(())
    }
}

impl core::error::Error for VerificationReport {}
