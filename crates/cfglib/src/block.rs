//! Basic block — a contiguous sequence of instructions with a single
//! entry point and a single exit point.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

/// Opaque identifier for a basic block within a [`Cfg`](crate::Cfg).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BlockId(pub(crate) u32);

impl BlockId {
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u32::try_from(index).expect("block index exceeds u32::MAX"))
    }

    /// Create a `BlockId` from a raw `u32` index.
    ///
    /// This is intended for ISA frontends that discover blocks by
    /// decoding and need to construct IDs directly.
    #[inline]
    #[must_use]
    pub fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// Returns the raw index.
    #[inline]
    #[must_use]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl core::fmt::Display for BlockId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "bb{}", self.0)
    }
}

impl crate::graph::view::DenseNodeId for BlockId {
    fn from_index(index: usize) -> Self {
        Self::from_index(index)
    }

    fn index(self) -> usize {
        self.index()
    }
}

/// A basic block containing a linear sequence of instructions.
///
/// Predication (ARM IT blocks, GPU wave predication, CMOV sequences) is not
/// block state: instructions declare their guards through
/// [`Predicated`](crate::Predicated), and
/// [`lift_predicated`](crate::lift_predicated) regionizes them.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BasicBlock<I> {
    /// Block identity.
    pub(crate) id: BlockId,
    /// Instructions in program order.
    pub(crate) instructions: Vec<I>,
    /// Optional human-readable label (e.g. from a `label` instruction).
    pub(crate) label: Option<String>,
}

impl<I> BasicBlock<I> {
    /// The block's unique identifier.
    #[inline]
    #[must_use]
    pub fn id(&self) -> BlockId {
        self.id
    }

    /// The instructions inside this block.
    #[inline]
    #[must_use]
    pub fn instructions(&self) -> &[I] {
        &self.instructions
    }

    /// Mutable access to the instructions (as a slice).
    #[inline]
    pub fn instructions_mut(&mut self) -> &mut [I] {
        &mut self.instructions
    }

    /// Optional label for this block.
    #[inline]
    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// Returns `true` if the block contains no instructions.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.instructions.is_empty()
    }

    /// Append an instruction to the end of the block.
    #[inline]
    pub fn push(&mut self, inst: I) {
        self.instructions.push(inst);
    }

    /// Set or replace the block's human-readable label.
    #[inline]
    pub fn set_label(&mut self, label: impl Into<String>) {
        self.label = Some(label.into());
    }

    /// Mutable access to the instruction vector.
    ///
    /// This gives full `Vec` control (insert, remove, drain, etc.)
    /// unlike [`instructions_mut`](Self::instructions_mut) which
    /// returns only a mutable slice.
    #[inline]
    pub fn instructions_vec_mut(&mut self) -> &mut Vec<I> {
        &mut self.instructions
    }
}
