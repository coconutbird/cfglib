//! The [`Cfg`] data structure — a control-flow graph parameterised over
//! an instruction type `I` and optional consumer edge payload `E`.

extern crate alloc;
use alloc::vec::Vec;
use smallvec::SmallVec;

use crate::block::{BasicBlock, BlockId};
use crate::edge::{Edge, EdgeId};
use crate::region::{Cleanup, Region};

mod cleanup;
mod mutation;
mod regions;
mod subgraph;
mod view;

pub use view::{Predecessors, Successors};

/// Why a requested instruction split-point sequence is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitPointError {
    /// A point lies beyond the original block's instruction count.
    OutOfBounds {
        /// Invalid instruction boundary.
        point: usize,
        /// Number of instructions in the block before any split.
        instruction_count: usize,
    },
    /// Points must be strictly increasing, so duplicates are invalid too.
    NotStrictlyIncreasing {
        /// Earlier point in the supplied sequence.
        previous: usize,
        /// Point that did not follow `previous`.
        point: usize,
    },
}

impl core::fmt::Display for SplitPointError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::OutOfBounds {
                point,
                instruction_count,
            } => write!(
                formatter,
                "split point {point} exceeds instruction count {instruction_count}"
            ),
            Self::NotStrictlyIncreasing { previous, point } => write!(
                formatter,
                "split point {point} does not strictly follow {previous}"
            ),
        }
    }
}

/// A control-flow graph over instruction type `I` and edge payload `E`.
///
/// `E = ()` retains the compact unannotated form. A frontend can instead use
/// `Cfg<I, E>` to keep format-specific edge provenance without teaching
/// cfglib about that format.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Cfg<I, E = ()> {
    pub(crate) blocks: Vec<BasicBlock<I>>,
    /// Edge arena — slots become `None` when removed via [`remove_edge`].
    pub(crate) edges: Vec<Option<Edge<E>>>,
    /// Successor edge ids per block (indexed by `BlockId`).
    pub(crate) succs: Vec<SmallVec<[EdgeId; 2]>>,
    /// Predecessor edge ids per block (indexed by `BlockId`).
    pub(crate) preds: Vec<SmallVec<[EdgeId; 4]>>,
    /// Entry block.
    pub(crate) entry: BlockId,
    /// Exception-handler regions (optional; empty for simple ISAs).
    pub(crate) regions: Vec<Region>,
    /// Cleanup records for handlers that continue somewhere once their body
    /// ends (optional; empty unless a frontend records them).
    #[cfg_attr(feature = "serde", serde(default))]
    pub(crate) cleanups: Vec<Cleanup>,
}

impl<I, E> Cfg<I, E> {
    /// Create an empty CFG with a single entry block.
    ///
    /// This is the primary constructor for ISA frontends that build
    /// the graph manually (as opposed to [`crate::CfgBuilder::build`] which
    /// processes a structured instruction stream).
    ///
    /// # Examples
    ///
    /// ```
    /// use cfglib::{Cfg, EdgeKind};
    ///
    /// let mut cfg = Cfg::<u32>::new();
    /// let entry = cfg.entry();
    /// let b1 = cfg.new_block();
    /// cfg.add_edge(entry, b1, EdgeKind::Fallthrough);
    /// assert_eq!(cfg.num_blocks(), 2);
    /// assert_eq!(cfg.num_edges(), 1);
    /// ```
    #[must_use]
    pub fn new_with_edge_payload() -> Self {
        let entry = BlockId(0);
        Self {
            blocks: alloc::vec![BasicBlock {
                id: entry,
                instructions: Vec::new(),
                label: None,
            }],
            edges: Vec::new(),
            succs: alloc::vec![SmallVec::new()],
            preds: alloc::vec![SmallVec::new()],
            entry,
            regions: Vec::new(),
            cleanups: Vec::new(),
        }
    }

    /// The entry block of the graph.
    #[inline]
    #[must_use]
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
    #[must_use]
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
    #[must_use]
    pub fn blocks(&self) -> &[BasicBlock<I>] {
        &self.blocks
    }

    /// Look up an edge by id.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not refer to a live edge in this CFG.
    #[inline]
    #[must_use]
    pub fn edge(&self, id: EdgeId) -> &Edge<E> {
        self.edges[id.index()]
            .as_ref()
            .expect("edge has been removed")
    }

    /// All live edges (skips tombstones left by [`Self::remove_edge`]).
    pub fn edges(&self) -> impl Iterator<Item = &Edge<E>> {
        self.edges.iter().filter_map(|slot| slot.as_ref())
    }

    /// Number of edge slots (including tombstones).
    ///
    /// This is the raw arena length, **not** the count of live edges.
    /// Use `edges().count()` for the live edge count.
    #[inline]
    pub(crate) fn edge_slots(&self) -> usize {
        self.edges.len()
    }

    /// Successor edges for a block.
    ///
    /// # Panics
    ///
    /// Panics if `id` does not refer to a block in this CFG.
    #[inline]
    #[must_use]
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
    #[must_use]
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
    ///
    /// # Examples
    ///
    /// ```
    /// use cfglib::{Cfg, EdgeKind};
    ///
    /// let mut cfg = Cfg::<u32>::new();
    /// let b0 = cfg.entry();
    /// let b1 = cfg.new_block();
    /// let b2 = cfg.new_block();
    /// cfg.add_edge(b0, b1, EdgeKind::ConditionalTrue);
    /// cfg.add_edge(b0, b2, EdgeKind::ConditionalFalse);
    ///
    /// let succs: Vec<_> = cfg.successors(b0).collect();
    /// assert_eq!(succs.len(), 2);
    /// ```
    #[must_use]
    pub fn successors(&self, id: BlockId) -> Successors<'_, I, E> {
        Successors {
            cfg: self,
            iter: self.succs[id.index()].iter(),
        }
    }

    /// Predecessor block ids (allocation-free).
    #[must_use]
    pub fn predecessors(&self, id: BlockId) -> Predecessors<'_, I, E> {
        Predecessors {
            cfg: self,
            iter: self.preds[id.index()].iter(),
        }
    }

    /// Number of basic blocks.
    #[inline]
    #[must_use]
    pub fn num_blocks(&self) -> usize {
        self.blocks.len()
    }

    /// Number of live edges (excludes tombstones).
    #[inline]
    #[must_use]
    pub fn num_edges(&self) -> usize {
        self.edges.iter().filter(|e| e.is_some()).count()
    }

    /// Returns an iterator over exit blocks — blocks with no outgoing edges.
    ///
    /// These are the natural exit points of the control-flow graph
    /// (return blocks, terminators, etc.).
    ///
    /// # Examples
    ///
    /// ```
    /// use cfglib::{Cfg, EdgeKind};
    ///
    /// let mut cfg = Cfg::<u32>::new();
    /// let b1 = cfg.new_block();
    /// cfg.add_edge(cfg.entry(), b1, EdgeKind::Fallthrough);
    /// // b1 has no outgoing edges — it's the only exit block.
    /// let exits: Vec<_> = cfg.exit_blocks().collect();
    /// assert_eq!(exits, vec![b1]);
    /// ```
    pub fn exit_blocks(&self) -> impl Iterator<Item = BlockId> + '_ {
        self.blocks
            .iter()
            .filter(|b| self.succs[b.id().index()].is_empty())
            .map(BasicBlock::id)
    }
}

impl<I, E> Default for Cfg<I, E> {
    fn default() -> Self {
        Self::new_with_edge_payload()
    }
}

impl<I> Cfg<I> {
    /// Create an empty CFG with a single entry block and unit edge payloads.
    ///
    /// Use [`Cfg::new_with_edge_payload`] when edge metadata has a
    /// consumer-defined type.
    #[must_use]
    pub fn new() -> Self {
        Self::new_with_edge_payload()
    }
}

impl<I> Cfg<I> {
    /// Run a fixpoint dataflow analysis on this CFG.
    ///
    /// This is a thin convenience wrapper around
    /// [`dataflow::fixpoint::solve`](crate::dataflow::fixpoint::solve).
    pub fn solve_dataflow<P: crate::dataflow::fixpoint::Problem<I>>(
        &self,
        problem: &P,
    ) -> crate::dataflow::fixpoint::FixpointResult<P::Fact> {
        crate::dataflow::fixpoint::solve(self, problem)
    }
}

#[cfg(test)]
mod tests;
