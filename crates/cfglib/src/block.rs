//! Basic block — a contiguous sequence of instructions with a single
//! entry point and a single exit point.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::identity::define_dense_id;

define_dense_id! {
    /// Opaque identifier for a basic block within a [`Cfg`](crate::Cfg).
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct BlockId(pub(crate) u32);
    display = "bb";
    /// Create a `BlockId` from a dense zero-based index.
    ///
    /// # Panics
    ///
    /// Panics when `index` exceeds `u32::MAX`.
    from_index = "block index exceeds u32::MAX";
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

    /// Mutable access to the instruction vector.
    ///
    /// Blocks impose no invariants on their instruction list, so full `Vec`
    /// control (insert, remove, drain) is available directly.
    #[inline]
    pub fn instructions_mut(&mut self) -> &mut Vec<I> {
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
}
