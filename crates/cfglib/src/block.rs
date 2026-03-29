//! Basic block — a contiguous sequence of instructions with a single
//! entry point and a single exit point.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

/// Opaque identifier for a basic block within a [`Cfg`](crate::Cfg).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub(crate) u32);

impl BlockId {
    /// Create a `BlockId` from a raw `u32` index.
    ///
    /// This is intended for ISA frontends that discover blocks by
    /// decoding and need to construct IDs directly.
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

impl core::fmt::Display for BlockId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "bb{}", self.0)
    }
}

/// A predication guard on a basic block.
///
/// Represents blocks whose execution is predicated on a condition
/// register/flag rather than a branch (ARM IT blocks, GPU wave
/// predication, x86 CMOV sequences, etc.).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guard {
    /// Predicate register or condition name (ISA-specific).
    pub predicate: String,
    /// Whether the block executes when the predicate is *true*
    /// (`false` means the block executes when the predicate is false).
    pub when_true: bool,
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
    /// Optional predication guard — the block only executes when this
    /// condition is satisfied. `None` for unconditionally executed blocks.
    pub(crate) guard: Option<Guard>,
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

    /// The predication guard, if any.
    #[inline]
    pub fn guard(&self) -> Option<&Guard> {
        self.guard.as_ref()
    }

    /// Set a predication guard on this block.
    #[inline]
    pub fn set_guard(&mut self, guard: Option<Guard>) {
        self.guard = guard;
    }

    /// Returns `true` if this block is predicated (guarded).
    #[inline]
    pub fn is_guarded(&self) -> bool {
        self.guard.is_some()
    }
}
