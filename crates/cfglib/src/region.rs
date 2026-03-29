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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockId;
    use crate::cfg::Cfg;
    use crate::test_util::MockInst;
    use alloc::collections::BTreeSet;

    fn block_set(ids: &[u32]) -> BTreeSet<BlockId> {
        ids.iter().map(|&i| BlockId::from_raw(i)).collect()
    }

    #[test]
    fn add_region_assigns_sequential_ids() {
        let mut cfg: Cfg<MockInst> = Cfg::new();
        let r0 = cfg.add_region(Region {
            id: RegionId(999), // should be overwritten
            protected_blocks: block_set(&[0]),
            handlers: alloc::vec![],
            parent: None,
        });
        let r1 = cfg.add_region(Region {
            id: RegionId(999),
            protected_blocks: block_set(&[0]),
            handlers: alloc::vec![],
            parent: Some(r0),
        });
        assert_eq!(r0.index(), 0);
        assert_eq!(r1.index(), 1);
        assert_eq!(cfg.regions().len(), 2);
        assert_eq!(cfg.regions()[1].parent, Some(r0));
    }

    #[test]
    fn protecting_region_finds_innermost() {
        let mut cfg: Cfg<MockInst> = Cfg::new();
        let _b1 = cfg.new_block();
        let _b2 = cfg.new_block();
        // Outer region protects blocks 0,1,2.
        let outer = cfg.add_region(Region {
            id: RegionId(0),
            protected_blocks: block_set(&[0, 1, 2]),
            handlers: alloc::vec![],
            parent: None,
        });
        // Inner region protects block 1 only.
        let inner = cfg.add_region(Region {
            id: RegionId(0),
            protected_blocks: block_set(&[1]),
            handlers: alloc::vec![],
            parent: Some(outer),
        });

        // Block 1 should find the inner (last-added) region.
        let r = cfg.protecting_region(BlockId::from_raw(1)).unwrap();
        assert_eq!(r.id, inner);

        // Block 0 should find the outer region.
        let r = cfg.protecting_region(BlockId::from_raw(0)).unwrap();
        assert_eq!(r.id, outer);

        // Block that's not in any region.
        let b3 = cfg.new_block();
        assert!(cfg.protecting_region(b3).is_none());
    }

    #[test]
    fn handler_kind_variants() {
        let h = Handler {
            entry: BlockId::from_raw(1),
            body: block_set(&[1, 2]),
            kind: HandlerKind::Finally,
        };
        assert_eq!(h.kind, HandlerKind::Finally);

        let h2 = Handler {
            entry: BlockId::from_raw(3),
            body: block_set(&[3]),
            kind: HandlerKind::Filter {
                filter_block: BlockId::from_raw(4),
            },
        };
        assert!(matches!(h2.kind, HandlerKind::Filter { .. }));
    }
}
