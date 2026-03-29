//! Basic block — a contiguous sequence of instructions with a single
//! entry point and a single exit point.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

/// Opaque identifier for a basic block within a [`Cfg`](crate::Cfg).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub(crate) u32);

impl BlockId {
    /// Returns the raw index.
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl core::fmt::Display for BlockId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "bb{}", self.0)
    }
}

/// A basic block containing a linear sequence of instructions.
#[derive(Debug, Clone)]
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
    pub fn id(&self) -> BlockId {
        self.id
    }

    /// The instructions inside this block.
    #[inline]
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
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    /// Returns `true` if the block contains no instructions.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.instructions.is_empty()
    }
}
