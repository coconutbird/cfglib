//! The [`Cfg`] data structure — a control-flow graph parameterised over
//! an instruction type `I`.

extern crate alloc;
use alloc::vec::Vec;
use core::ops::Index;
use core::slice;

use crate::block::{BasicBlock, BlockId};
use crate::edge::{Edge, EdgeId, EdgeKind};
use crate::region::{Region, RegionId};

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
    /// Exception-handler regions (optional; empty for simple ISAs).
    pub(crate) regions: Vec<Region>,
}

impl<I> Cfg<I> {
    /// Create an empty CFG with a single entry block.
    ///
    /// This is the primary constructor for ISA frontends that build
    /// the graph manually (as opposed to [`CfgBuilder::build`] which
    /// processes a structured instruction stream).
    pub fn new() -> Self {
        let entry = BlockId(0);
        Self {
            blocks: alloc::vec![BasicBlock {
                id: entry,
                instructions: Vec::new(),
                label: None,
            }],
            edges: Vec::new(),
            succs: alloc::vec![Vec::new()],
            preds: alloc::vec![Vec::new()],
            entry,
            regions: Vec::new(),
        }
    }

    /// The entry block of the graph.
    #[inline]
    pub fn entry(&self) -> BlockId {
        self.entry
    }

    /// Change the entry block of the graph.
    ///
    /// # Panics
    ///
    /// Panics (debug) if `id` does not refer to a block in this CFG.
    #[inline]
    pub fn set_entry(&mut self, id: BlockId) {
        debug_assert!(
            id.index() < self.blocks.len(),
            "BlockId {} out of range (num_blocks = {})",
            id,
            self.blocks.len(),
        );
        self.entry = id;
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

    // ── Region methods ─────────────────────────────────────────────

    /// All exception-handler regions.
    #[inline]
    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    /// Add a region and return its id.
    pub fn add_region(&mut self, mut region: Region) -> RegionId {
        let id = RegionId(self.regions.len() as u32);
        region.id = id;
        self.regions.push(region);
        id
    }

    /// Returns the innermost region that protects `block`, if any.
    pub fn protecting_region(&self, block: BlockId) -> Option<&Region> {
        // Return the deepest (last-added) region whose protected set
        // contains this block.
        self.regions
            .iter()
            .rev()
            .find(|r| r.protected_blocks.contains(&block))
    }

    // ── Block / edge mutation ─────────────────────────────────────

    /// Allocate a new empty block and return its id.
    pub fn new_block(&mut self) -> BlockId {
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
    pub fn add_edge(&mut self, source: BlockId, target: BlockId, kind: EdgeKind) -> EdgeId {
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

    /// Remove an edge by id.
    ///
    /// Returns the removed [`Edge`], or `None` if the id is out of
    /// range. The edge slot is **not** compacted — removed edges
    /// leave a tombstone so that existing [`EdgeId`]s remain valid.
    ///
    /// The successor and predecessor lists of the affected blocks are
    /// updated.
    pub fn remove_edge(&mut self, id: EdgeId) -> Option<Edge> {
        if id.index() >= self.edges.len() {
            return None;
        }
        let edge = self.edges[id.index()].clone();
        // Remove from succs/preds.
        self.succs[edge.source.index()].retain(|&e| e != id);
        self.preds[edge.target.index()].retain(|&e| e != id);
        Some(edge)
    }

    /// Split a block at instruction index `at`.
    ///
    /// Instructions `[at..]` are moved into a new block. A
    /// [`Fallthrough`](EdgeKind::Fallthrough) edge is inserted from
    /// the original block to the new one, and all outgoing edges of
    /// the original block are transferred to the new block.
    ///
    /// Returns the id of the newly created block.
    ///
    /// # Panics
    ///
    /// Panics if `id` is out of range or `at > instructions.len()`.
    pub fn split_block(&mut self, id: BlockId, at: usize) -> BlockId {
        let tail_insts: Vec<I> = self.blocks[id.index()].instructions.split_off(at);
        let new_id = self.new_block();
        self.blocks[new_id.index()].instructions = tail_insts;

        // Move outgoing edges from `id` to `new_id`.
        let outgoing: Vec<EdgeId> = self.succs[id.index()].drain(..).collect();
        for &eid in &outgoing {
            self.edges[eid.index()].source = new_id;
            self.succs[new_id.index()].push(eid);
        }

        // Insert fallthrough edge from original to new block.
        self.add_edge(id, new_id, EdgeKind::Fallthrough);

        new_id
    }

    /// Redirect all edges that target `old` to target `new_target` instead.
    ///
    /// This is useful for bypassing a block before removal.
    pub fn redirect_edges_to(&mut self, old: BlockId, new_target: BlockId) {
        let incoming: Vec<EdgeId> = self.preds[old.index()].clone();
        for eid in incoming {
            self.edges[eid.index()].target = new_target;
            self.preds[old.index()].retain(|&e| e != eid);
            self.preds[new_target.index()].push(eid);
        }
    }

    /// Mutable access to an edge's kind.
    ///
    /// # Panics
    ///
    /// Panics (debug) if `id` is out of range.
    #[inline]
    pub fn edge_mut(&mut self, id: EdgeId) -> &mut Edge {
        debug_assert!(
            id.index() < self.edges.len(),
            "EdgeId {} out of range (num_edges = {})",
            id,
            self.edges.len(),
        );
        &mut self.edges[id.index()]
    }
}

// ── Default impl ──────────────────────────────────────────────────

impl<I> Default for Cfg<I> {
    fn default() -> Self {
        Self::new()
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
