//! Edges connecting basic blocks in a control-flow graph.

use crate::block::BlockId;

/// Opaque identifier for an edge within a [`Cfg`](crate::Cfg).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EdgeId(pub(crate) u32);

impl EdgeId {
    /// Returns the raw index.
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// The kind of a control-flow edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    /// Sequential fallthrough to the next block.
    Fallthrough,
    /// Taken branch of a conditional (the "true" path).
    ConditionalTrue,
    /// Not-taken branch of a conditional (the "false" / merge path).
    ConditionalFalse,
    /// Unconditional jump.
    Unconditional,
    /// Back-edge to a loop header.
    Back,
    /// Edge to a call target.
    Call,
    /// Return edge from a call site.
    CallReturn,
    /// Edge for a switch/case arm.
    SwitchCase,
}

/// A directed edge between two basic blocks.
#[derive(Debug, Clone)]
pub struct Edge {
    /// Edge identity.
    pub id: EdgeId,
    /// Source block.
    pub source: BlockId,
    /// Target block.
    pub target: BlockId,
    /// Classification.
    pub kind: EdgeKind,
}
