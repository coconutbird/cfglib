//! The [`Cfg`] data structure — a control-flow graph parameterised over
//! an instruction type `I`.

extern crate alloc;
use alloc::vec::Vec;
use core::ops::Index;
use core::slice;

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
    ///
    /// # Panics
    ///
    /// Panics if `id` does not refer to a block in this CFG.
    #[inline]
    pub fn block(&self, id: BlockId) -> &BasicBlock<I> {
        debug_assert!(
            id.index() < self.blocks.len(),
            "BlockId {} out of range (num_blocks = {})",
            id,
            self.blocks.len(),
        );
        &self.blocks[id.index()]
    }

    /// Mutable access to a block.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not refer to a block in this CFG.
    #[inline]
    pub fn block_mut(&mut self, id: BlockId) -> &mut BasicBlock<I> {
        debug_assert!(
            id.index() < self.blocks.len(),
            "BlockId {} out of range (num_blocks = {})",
            id,
            self.blocks.len(),
        );
        &mut self.blocks[id.index()]
    }

    /// All blocks in allocation order.
    #[inline]
    pub fn blocks(&self) -> &[BasicBlock<I>] {
        &self.blocks
    }

    /// Look up an edge by id.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not refer to an edge in this CFG.
    #[inline]
    pub fn edge(&self, id: EdgeId) -> &Edge {
        debug_assert!(
            id.index() < self.edges.len(),
            "EdgeId {} out of range (num_edges = {})",
            id,
            self.edges.len(),
        );
        &self.edges[id.index()]
    }

    /// All edges.
    #[inline]
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// Successor edges for a block.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not refer to a block in this CFG.
    #[inline]
    pub fn successor_edges(&self, id: BlockId) -> &[EdgeId] {
        debug_assert!(
            id.index() < self.succs.len(),
            "BlockId {} out of range for successor lookup (num_blocks = {})",
            id,
            self.succs.len(),
        );
        &self.succs[id.index()]
    }

    /// Predecessor edges for a block.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not refer to a block in this CFG.
    #[inline]
    pub fn predecessor_edges(&self, id: BlockId) -> &[EdgeId] {
        debug_assert!(
            id.index() < self.preds.len(),
            "BlockId {} out of range for predecessor lookup (num_blocks = {})",
            id,
            self.preds.len(),
        );
        &self.preds[id.index()]
    }

    /// Successor block ids (allocation-free).
    pub fn successors(&self, id: BlockId) -> Successors<'_> {
        Successors {
            edges: &self.edges,
            iter: self.succs[id.index()].iter(),
        }
    }

    /// Predecessor block ids (allocation-free).
    pub fn predecessors(&self, id: BlockId) -> Predecessors<'_> {
        Predecessors {
            edges: &self.edges,
            iter: self.preds[id.index()].iter(),
        }
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

    /// Returns the exit blocks — blocks with no outgoing edges.
    ///
    /// These are the natural exit points of the control-flow graph
    /// (return blocks, terminators, etc.).
    pub fn exit_blocks(&self) -> Vec<BlockId> {
        self.blocks
            .iter()
            .filter(|b| self.succs[b.id().index()].is_empty())
            .map(|b| b.id())
            .collect()
    }

    /// Allocate a new empty block and return its id.
    pub(crate) fn new_block(&mut self) -> BlockId {
        debug_assert!(
            self.blocks.len() < u32::MAX as usize,
            "too many blocks: would overflow u32 BlockId",
        );

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
        debug_assert!(
            self.edges.len() < u32::MAX as usize,
            "too many edges: would overflow u32 EdgeId",
        );
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

// ── Index impls ────────────────────────────────────────────────────

impl<I> Index<BlockId> for Cfg<I> {
    type Output = BasicBlock<I>;

    /// Index into the CFG by [`BlockId`].
    ///
    /// Equivalent to [`Cfg::block`] but usable with `cfg[id]` syntax.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not refer to a block in this CFG.
    #[inline]
    fn index(&self, id: BlockId) -> &BasicBlock<I> {
        &self.blocks[id.index()]
    }
}

impl<I> Index<EdgeId> for Cfg<I> {
    type Output = Edge;

    /// Index into the CFG by [`EdgeId`].
    ///
    /// Equivalent to [`Cfg::edge`] but usable with `cfg[id]` syntax.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not refer to an edge in this CFG.
    #[inline]
    fn index(&self, id: EdgeId) -> &Edge {
        &self.edges[id.index()]
    }
}

/// Iterator over successor block ids (zero-allocation).
pub struct Successors<'a> {
    edges: &'a [Edge],
    iter: slice::Iter<'a, EdgeId>,
}

impl<'a> Iterator for Successors<'a> {
    type Item = BlockId;
    #[inline]
    fn next(&mut self) -> Option<BlockId> {
        self.iter.next().map(|&eid| self.edges[eid.index()].target)
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<'a> ExactSizeIterator for Successors<'a> {}

/// Iterator over predecessor block ids (zero-allocation).
pub struct Predecessors<'a> {
    edges: &'a [Edge],
    iter: slice::Iter<'a, EdgeId>,
}

impl<'a> Iterator for Predecessors<'a> {
    type Item = BlockId;
    #[inline]
    fn next(&mut self) -> Option<BlockId> {
        self.iter.next().map(|&eid| self.edges[eid.index()].source)
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<'a> ExactSizeIterator for Predecessors<'a> {}
