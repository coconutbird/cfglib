//! Data flow analysis framework.
//!
//! Provides generic infrastructure for computing data flow properties
//! over a [`Cfg`]:
//!
//! - **Reaching definitions** — which writes can reach a given point
//! - **Liveness** — which variables are live at each point
//! - **Def-use / use-def chains** — linking writers to readers
//!
//! # Usage
//!
//! Implement [`DataDeps`] for your instruction type to declare which
//! locations each instruction reads from and writes to, then run any
//! of the provided analyses.

pub mod defuse;
pub mod fixpoint;
pub mod liveness;
pub mod reaching;

extern crate alloc;
use alloc::vec::Vec;

use crate::block::BlockId;

/// An abstract storage location that an instruction can read or write.
///
/// Locations are identified by a `u16` index, which the ISA adapter
/// maps to concrete resources (registers, temporaries, memory slots, etc.).
///
/// For SM4/SM5 shaders this might be:
/// - `Location(0..=15)` → `r0`–`r15` (temporaries)
/// - `Location(16..=31)` → `v0`–`v15` (inputs)
/// - `Location(32..=39)` → `o0`–`o7` (outputs)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Location(pub u16);

impl Location {
    /// Create a new location with the given index.
    #[inline]
    pub fn new(index: u16) -> Self {
        Self(index)
    }

    /// Returns the raw index.
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl core::fmt::Display for Location {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "loc{}", self.0)
    }
}

/// Trait that an instruction type implements to expose its data
/// dependencies (which locations it reads and writes).
///
/// This is the data-flow counterpart of [`FlowControl`](crate::FlowControl),
/// which classifies control-flow effects.
pub trait DataDeps {
    /// Locations that this instruction **reads** (uses).
    fn uses(&self) -> Vec<Location>;

    /// Locations that this instruction **writes** (defines).
    fn defs(&self) -> Vec<Location>;
}

/// A single definition site: a (block, instruction-index) pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DefSite {
    /// The block containing the defining instruction.
    pub block: BlockId,
    /// Index of the instruction within the block.
    pub inst_idx: usize,
}

impl core::fmt::Display for DefSite {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}:{}", self.block, self.inst_idx)
    }
}

/// A single use site: a (block, instruction-index) pair.
pub type UseSite = DefSite;
