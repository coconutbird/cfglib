//! Exception-handler region model.
//!
//! Regions represent protected areas of code (try blocks) and their
//! associated handlers (catch, finally, fault, filter). This model
//! is inspired by the CLR / JVM exception metadata and Echo's
//! `ExceptionHandlerRegion`.
//!
//! Regions are **optional metadata** on a [`Cfg`](crate::Cfg) — GPU
//! shaders and simple ISAs simply leave the region list empty.

extern crate alloc;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::block::BlockId;

/// Opaque identifier for a region within a [`Cfg`](crate::Cfg).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RegionId(pub(crate) u32);

impl RegionId {
    /// Create a `RegionId` from a raw index.
    #[inline]
    pub fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// Returns the raw index.
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl core::fmt::Display for RegionId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "region{}", self.0)
    }
}

/// A protected region (try block) and its handlers.
#[derive(Debug, Clone)]
pub struct Region {
    /// Region identity.
    pub id: RegionId,
    /// Blocks covered by the protected (try) region.
    pub protected_blocks: BTreeSet<BlockId>,
    /// Exception handlers attached to this region.
    pub handlers: Vec<Handler>,
    /// Parent region (for nested try/catch).
    pub parent: Option<RegionId>,
}

/// An exception handler attached to a [`Region`].
#[derive(Debug, Clone)]
pub struct Handler {
    /// Entry block of the handler.
    pub entry: BlockId,
    /// All blocks in the handler body.
    pub body: BTreeSet<BlockId>,
    /// The handler classification.
    pub kind: HandlerKind,
}

/// Classification of an exception handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandlerKind {
    /// Catch handler — catches a specific exception type.
    Catch,
    /// Catch-all handler — catches any exception.
    CatchAll,
    /// Finally handler — always executed.
    Finally,
    /// Fault handler — executed on exception only (CLR).
    Fault,
    /// Filter handler — a user-defined predicate determines whether
    /// this handler catches the exception.
    Filter {
        /// Block containing the filter predicate.
        filter_block: BlockId,
    },
}
