//! Shared core of the per-level "toy dialect" IR test fixtures.
//!
//! Each IR level's test module defines its own dialect marker,
//! operations, and level-specific trait impls — those differ by design.
//! What they share is the source-coordinate vocabulary and, for RTL
//! dialects raised into MLIL, the metadata classification of a
//! [`LiftedStatement`].

extern crate alloc;

use alloc::vec::Vec;

use crate::ir::mlil::InstructionMetadata;
use crate::ir::rtl::{Dialect as RtlDialect, LiftedStatement};

/// The common toy source span: a half-open `[start, end)` range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Span {
    /// Inclusive start point.
    pub start: u32,
    /// Exclusive end point.
    pub end: u32,
}

/// The shared `Vocabulary::span_is_empty` body for [`Span`].
pub fn span_is_empty(span: Span) -> bool {
    span.start >= span.end
}

/// The shared `Vocabulary::span_contains` body for [`Span`].
pub fn span_contains(span: Span, point: u32) -> bool {
    span.start <= point && point < span.end
}

/// The shared `mlil::Dialect::instruction_metadata` body for MLIL
/// dialects whose operation is an RTL [`LiftedStatement`]: statements
/// carry their own effects and flow classification.
pub fn lifted_statement_metadata<D: RtlDialect>(
    operation: &LiftedStatement<D>,
    may_throw: bool,
) -> InstructionMetadata<D::Effect> {
    let effects = match operation {
        LiftedStatement::Assign { effects, .. }
        | LiftedStatement::Effect { effects, .. }
        | LiftedStatement::Raise { effects, .. } => effects.clone(),
        LiftedStatement::Branch { .. }
        | LiftedStatement::Dispatch { .. }
        | LiftedStatement::Return { .. } => Vec::new(),
    };
    InstructionMetadata::new(effects, operation.flow_effect(), may_throw)
}
