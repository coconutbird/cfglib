//! Stable identities for MLIL entities.

use core::fmt;

use crate::{BlockId, EdgeId};

/// Stable identity of one mutable MLIL variable before SSA renaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VariableId(u32);

impl VariableId {
    /// Creates an identity from its dense raw index.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// Returns the dense zero-based index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Returns the compact raw identity.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl fmt::Display for VariableId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "v{}", self.0)
    }
}

/// Stable identity of one MLIL instruction within a function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstructionId(u32);

impl InstructionId {
    /// Creates an identity from its dense raw index.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// Returns the dense zero-based index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Returns the compact raw identity.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl fmt::Display for InstructionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "i{}", self.0)
    }
}

/// Stable identity of an MLIL entity that can originate from source code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntityId {
    /// Control-flow block.
    Block(BlockId),
    /// Stable control-flow edge.
    Edge(EdgeId),
    /// Semantic instruction.
    Instruction(InstructionId),
    /// Mutable pre-SSA variable.
    Variable(VariableId),
}
