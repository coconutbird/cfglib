//! The [`Cfg`] data structure — a control-flow graph parameterised over
//! an instruction type `I`.

extern crate alloc;
use alloc::vec::Vec;

use crate::block::{BasicBlock, BlockId};
use crate::edge::{Edge, EdgeId, EdgeKind};

/// A control-flow graph over instruction type `I`.
#[derive(Debug, Clone)]
pub struct Cfg<I> {
    pub(crate) blocks: Vec<BasicBlock<I>>,
    pub(crate) edges: Vec<Edge>,
    /// Successor edge ids per block (indexed by `BlockId`).
    pub(crate) succs: Vec<Vec<EdgeId>>,
    /// Predecessor edge ids per block (indexed by `BlockId`).
    pub(crate) preds: Vec<Vec<EdgeId>>,
    /// Entry block.
    pub(crate) entry: BlockId,
}

impl<I> Cfg<I> {
    /// The entry block of the graph.
    #[inline]
    pub fn entry(&self) -> BlockId {
        self.entry
    }

    /// Look up a block by id.
    #[inline]
    pub fn block(&self, id: BlockId) -> &BasicBlock<I> {
        &self.blocks[id.index()]
    }

    /// Mutable access to a block.
    #[inline]
    pub fn block_mut(&mut self, id: BlockId) -> &mut BasicBlock<I> {
        &mut self.blocks[id.index()]
    }

    /// All blocks in allocation order.
    #[inline]
    pub fn blocks(&self) -> &[BasicBlock<I>] {
        &self.blocks
    }

    /// Look up an edge by id.
    #[inline]
    pub fn edge(&self, id: EdgeId) -> &Edge {
        &self.edges[id.index()]
    }

    /// All edges.
    #[inline]
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// Successor edges for a block.
    #[inline]
    pub fn successor_edges(&self, id: BlockId) -> &[EdgeId] {
        &self.succs[id.index()]
    }

    /// Predecessor edges for a block.
    #[inline]
    pub fn predecessor_edges(&self, id: BlockId) -> &[EdgeId] {
        &self.preds[id.index()]
    }

    /// Successor block ids.
    pub fn successors(&self, id: BlockId) -> Vec<BlockId> {
        self.succs[id.index()]
            .iter()
            .map(|&eid| self.edges[eid.index()].target)
            .collect()
    }

    /// Predecessor block ids.
    pub fn predecessors(&self, id: BlockId) -> Vec<BlockId> {
        self.preds[id.index()]
            .iter()
            .map(|&eid| self.edges[eid.index()].source)
            .collect()
    }

    /// Number of basic blocks.
    #[inline]
    pub fn num_blocks(&self) -> usize {
        self.blocks.len()
    }

    /// Number of edges.
    #[inline]
    pub fn num_edges(&self) -> usize {
        self.edges.len()
    }

    /// Allocate a new empty block and return its id.
    pub(crate) fn new_block(&mut self) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        self.blocks.push(BasicBlock {
            id,
            instructions: Vec::new(),
            label: None,
        });

        self.succs.push(Vec::new());
        self.preds.push(Vec::new());

        id
    }

    /// Add a directed edge and return its id.
    pub(crate) fn add_edge(&mut self, source: BlockId, target: BlockId, kind: EdgeKind) -> EdgeId {
        let id = EdgeId(self.edges.len() as u32);
        self.edges.push(Edge {
            id,
            source,
            target,
            kind,
        });

        self.succs[source.index()].push(id);
        self.preds[target.index()].push(id);

        id
    }
}
