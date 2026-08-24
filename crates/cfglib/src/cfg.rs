//! The [`Cfg`] data structure — a control-flow graph parameterised over
//! an instruction type `I` and optional consumer edge payload `E`.

extern crate alloc;
use alloc::vec::Vec;
use core::ops::Index;
use core::slice;
use smallvec::SmallVec;

use crate::block::{BasicBlock, BlockId};
use crate::edge::{Edge, EdgeId, EdgeKind};
use crate::region::{Cleanup, Continuation, HandlerRef, Region, RegionId};
use crate::rewrite::RewriteMap;

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
            .map(super::block::BasicBlock::id)
    }

    // ── Region methods ─────────────────────────────────────────────

    /// All exception-handler regions.
    #[inline]
    #[must_use]
    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    /// Add a region and return its id.
    pub fn add_region(&mut self, mut region: Region) -> RegionId {
        let id = RegionId::from_index(self.regions.len());
        region.id = id;
        self.regions.push(region);
        id
    }

    /// Returns the innermost region that protects `block`, if any.
    #[must_use]
    pub fn protecting_region(&self, block: BlockId) -> Option<&Region> {
        // Return the deepest (last-added) region whose protected set
        // contains this block.
        self.regions
            .iter()
            .rev()
            .find(|r| r.protected_blocks.contains(&block))
    }

    // ── Cleanup continuations ─────────────────────────────────────

    /// Every recorded [`Cleanup`], in the order the handlers first recorded
    /// one.
    #[inline]
    #[must_use]
    pub fn cleanups(&self) -> &[Cleanup] {
        &self.cleanups
    }

    /// The cleanup record of `handler`, if it has one.
    #[must_use]
    pub fn cleanup(&self, handler: HandlerRef) -> Option<&Cleanup> {
        self.cleanups
            .iter()
            .find(|cleanup| cleanup.handler == handler)
    }

    /// Record one route out of a cleanup handler: where control resumes once
    /// the cleanup body ends, and the reason that entered it.
    ///
    /// The first call for a handler creates its record. Recording the same
    /// `(reason, resume)` pair twice is a no-op, so a lowering that walks
    /// several transfers into one cleanup keeps one route per distinct
    /// destination, in first-recorded order.
    ///
    /// # Examples
    ///
    /// A `try { ... } finally { ... }` whose body both falls out normally and
    /// `return`s: one cleanup block, two routes, told apart by reason.
    ///
    /// ```
    /// use cfglib::{
    ///     Cfg, CompletionReason, Continuation, Handler, HandlerKind, HandlerRef, Region,
    ///     RegionId, build_eh_model,
    /// };
    ///
    /// let mut cfg = Cfg::<&'static str>::new();
    /// let cleanup_block = cfg.new_block();
    /// let after = cfg.new_block();
    /// let exit = cfg.new_block();
    ///
    /// let region = cfg.add_region(Region {
    ///     id: RegionId::from_raw(0), // overwritten by `add_region`
    ///     protected_blocks: [cfg.entry()].into_iter().collect(),
    ///     handlers: vec![Handler {
    ///         entry: cleanup_block,
    ///         body: [cleanup_block].into_iter().collect(),
    ///         kind: HandlerKind::Finally,
    ///     }],
    ///     parent: None,
    /// });
    ///
    /// let handler = HandlerRef::new(region, 0);
    /// cfg.set_cleanup_resume(handler, cleanup_block);
    /// cfg.add_continuation(handler, Continuation {
    ///     reason: CompletionReason::Normal,
    ///     resume: after,
    /// });
    /// cfg.add_continuation(handler, Continuation {
    ///     reason: CompletionReason::Return,
    ///     resume: exit,
    /// });
    ///
    /// // Both routes leave the same block, and each one is identifiable.
    /// let model = build_eh_model(&cfg);
    /// let recorded = &model.cleanups[&cleanup_block];
    /// assert_eq!(recorded.resume_from, Some(cleanup_block));
    /// assert_eq!(
    ///     recorded.resumes_for(CompletionReason::Return).collect::<Vec<_>>(),
    ///     vec![exit]
    /// );
    /// ```
    ///
    /// # Panics
    ///
    /// Panics (debug) if `handler` does not refer to a handler of a region in
    /// this CFG — register the region first, then attach its routes.
    pub fn add_continuation(&mut self, handler: HandlerRef, continuation: Continuation) {
        debug_assert!(
            self.handler_exists(handler),
            "handler does not exist in this CFG"
        );
        let cleanup = self.cleanup_entry(handler);
        if !cleanup.continuations.contains(&continuation) {
            cleanup.continuations.push(continuation);
        }
    }

    /// Record the block a cleanup handler's body ends in — the block every
    /// continuation edge leaves from.
    ///
    /// A cleanup that diverges never reaches one, so leaving it unset is the
    /// honest description of a `finally` that returns.
    ///
    /// # Panics
    ///
    /// Panics (debug) if `handler` does not refer to a handler of a region in
    /// this CFG, or if `resume_from` does not refer to a block in it.
    pub fn set_cleanup_resume(&mut self, handler: HandlerRef, resume_from: BlockId) {
        debug_assert!(
            self.handler_exists(handler),
            "handler does not exist in this CFG"
        );
        debug_assert!(
            resume_from.index() < self.blocks.len(),
            "resume block does not exist in this CFG"
        );
        self.cleanup_entry(handler).resume_from = Some(resume_from);
    }

    /// The cleanup record of `handler`, created empty when it is the first
    /// route recorded for it.
    fn cleanup_entry(&mut self, handler: HandlerRef) -> &mut Cleanup {
        let existing = self
            .cleanups
            .iter()
            .position(|cleanup| cleanup.handler == handler);
        let at = existing.unwrap_or_else(|| {
            self.cleanups.push(Cleanup {
                handler,
                resume_from: None,
                continuations: Vec::new(),
            });
            self.cleanups.len() - 1
        });
        &mut self.cleanups[at]
    }

    /// Whether `handler` refers to a handler of a region in this CFG.
    fn handler_exists(&self, handler: HandlerRef) -> bool {
        self.regions
            .get(handler.region().index())
            .is_some_and(|region| handler.index() < region.handlers.len())
    }

    // ── Block / edge mutation ─────────────────────────────────────

    /// Allocate a new empty block and return its id.
    pub fn new_block(&mut self) -> BlockId {
        let id = BlockId::from_index(self.blocks.len());
        self.blocks.push(BasicBlock {
            id,
            instructions: Vec::new(),
            label: None,
        });

        self.succs.push(SmallVec::new());
        self.preds.push(SmallVec::new());

        id
    }

    /// Add a directed edge with the default consumer payload.
    ///
    /// This is the compatibility front door for `Cfg<I>` and is also useful
    /// when a custom payload has a meaningful [`Default`] value.
    pub fn add_edge(&mut self, source: BlockId, target: BlockId, kind: EdgeKind) -> EdgeId
    where
        E: Default,
    {
        self.add_edge_with_payload(source, target, kind, E::default())
    }

    /// Add a weighted directed edge with the default consumer payload.
    pub fn add_weighted_edge(
        &mut self,
        source: BlockId,
        target: BlockId,
        kind: EdgeKind,
        weight: f64,
    ) -> EdgeId
    where
        E: Default,
    {
        self.add_weighted_edge_with_payload(source, target, kind, weight, E::default())
    }

    /// Add a directed edge with consumer metadata and return its id.
    pub fn add_edge_with_payload(
        &mut self,
        source: BlockId,
        target: BlockId,
        kind: EdgeKind,
        payload: E,
    ) -> EdgeId {
        self.add_edge_inner(source, target, kind, None, payload)
    }

    /// Add a directed edge with a branch weight and consumer metadata.
    pub fn add_weighted_edge_with_payload(
        &mut self,
        source: BlockId,
        target: BlockId,
        kind: EdgeKind,
        weight: f64,
        payload: E,
    ) -> EdgeId {
        self.add_edge_inner(source, target, kind, Some(weight), payload)
    }

    fn add_edge_inner(
        &mut self,
        source: BlockId,
        target: BlockId,
        kind: EdgeKind,
        weight: Option<f64>,
        payload: E,
    ) -> EdgeId {
        let id = EdgeId::from_index(self.edges.len());
        self.edges.push(Some(Edge {
            id,
            source,
            target,
            kind,
            weight,
            payload,
        }));

        self.succs[source.index()].push(id);
        self.preds[target.index()].push(id);

        id
    }

    /// Remove an edge by id.
    ///
    /// Returns the removed [`Edge`], or `None` if the id is out of
    /// range or already removed. The edge slot is replaced with a
    /// tombstone (`None`) so that existing [`EdgeId`]s remain valid.
    ///
    /// The successor and predecessor lists of the affected blocks are
    /// updated.
    ///
    /// # Examples
    ///
    /// ```
    /// use cfglib::{Cfg, EdgeKind};
    ///
    /// let mut cfg = Cfg::<u32>::new();
    /// let b0 = cfg.entry();
    /// let b1 = cfg.new_block();
    /// let eid = cfg.add_edge(b0, b1, EdgeKind::Fallthrough);
    ///
    /// assert_eq!(cfg.num_edges(), 1);
    /// let removed = cfg.remove_edge(eid).unwrap();
    /// assert_eq!(removed.kind(), EdgeKind::Fallthrough);
    /// assert_eq!(cfg.num_edges(), 0);
    /// // Double-remove returns None.
    /// assert!(cfg.remove_edge(eid).is_none());
    /// ```
    pub fn remove_edge(&mut self, id: EdgeId) -> Option<Edge<E>> {
        let slot = self.edges.get_mut(id.index())?;
        let edge = slot.take()?;
        self.succs[edge.source.index()].retain(|e| *e != id);
        self.preds[edge.target.index()].retain(|e| *e != id);
        Some(edge)
    }

    /// Remove an edge and return both its value and explicit identity mapping.
    pub fn remove_edge_mapped(&mut self, id: EdgeId) -> (Option<Edge<E>>, RewriteMap) {
        let removed = self.remove_edge(id);
        let mut mapping = RewriteMap::new();
        if removed.is_some() {
            mapping.record_edge(id, []);
        }
        (removed, mapping)
    }

    /// Split a block at instruction index `at` using the default payload for
    /// the new fallthrough edge.
    pub fn split_block(&mut self, id: BlockId, at: usize) -> BlockId
    where
        E: Default,
    {
        self.split_block_with_payload(id, at, E::default())
    }

    /// Split a block at instruction index `at` with an explicit payload for
    /// the new fallthrough edge.
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
    pub fn split_block_with_payload(
        &mut self,
        id: BlockId,
        at: usize,
        fallthrough_payload: E,
    ) -> BlockId {
        self.split_block_with_payload_mapped(id, at, fallthrough_payload)
            .0
    }

    /// Split a block and return its new tail plus every affected identity.
    ///
    /// # Panics
    ///
    /// Panics if `id` is invalid, `at` exceeds the instruction count, or the
    /// CFG's internal adjacency refers to a removed edge.
    pub fn split_block_with_payload_mapped(
        &mut self,
        id: BlockId,
        at: usize,
        fallthrough_payload: E,
    ) -> (BlockId, RewriteMap) {
        let tail_insts: Vec<I> = self.blocks[id.index()].instructions.split_off(at);
        let new_id = self.new_block();
        self.blocks[new_id.index()].instructions = tail_insts;

        // Move outgoing edges from `id` to `new_id`.
        let outgoing: SmallVec<[EdgeId; 2]> = self.succs[id.index()].drain(..).collect();
        for &eid in &outgoing {
            self.edges[eid.index()].as_mut().unwrap().source = new_id;
            self.succs[new_id.index()].push(eid);
        }

        // Insert fallthrough edge from original to new block.
        let fallthrough =
            self.add_edge_with_payload(id, new_id, EdgeKind::Fallthrough, fallthrough_payload);

        let mut mapping = RewriteMap::new();
        mapping.record_block(id, [id, new_id]);
        mapping.record_created_block(new_id);
        for edge in outgoing {
            mapping.record_edge(edge, [edge]);
        }
        mapping.record_created_edge(fallthrough);
        (new_id, mapping)
    }

    /// Split one block at ordered instruction boundaries with explicit edge
    /// payloads and return every resulting block plus an identity mapping.
    ///
    /// Points are offsets in the original block, not in successively shorter
    /// tails. They must be strictly increasing and may include `0` or the
    /// original instruction count. Validation happens before mutation, so an
    /// error leaves the CFG unchanged. The returned blocks are in execution
    /// order and include `id` as the first element.
    ///
    /// # Errors
    ///
    /// Returns [`SplitPointError`] when a point is out of bounds or the points
    /// are not strictly increasing.
    pub fn split_block_at_points_with_payloads(
        &mut self,
        id: BlockId,
        points: impl IntoIterator<Item = (usize, E)>,
    ) -> Result<(Vec<BlockId>, RewriteMap), SplitPointError> {
        let points: Vec<_> = points.into_iter().collect();
        let instruction_count = self.block(id).instructions().len();
        let mut previous = None;
        for &(point, _) in &points {
            if point > instruction_count {
                return Err(SplitPointError::OutOfBounds {
                    point,
                    instruction_count,
                });
            }
            if let Some(previous) = previous
                && point <= previous
            {
                return Err(SplitPointError::NotStrictlyIncreasing { previous, point });
            }
            previous = Some(point);
        }

        let mut blocks = alloc::vec![id];
        let mut mapping = RewriteMap::new();
        let mut current = id;
        let mut base = 0;
        for (point, payload) in points {
            let (tail, split) =
                self.split_block_with_payload_mapped(current, point - base, payload);
            mapping.compose(split);
            blocks.push(tail);
            current = tail;
            base = point;
        }
        Ok((blocks, mapping))
    }

    /// Split one block at ordered instruction boundaries using default edge
    /// payloads.
    ///
    /// # Errors
    ///
    /// Returns [`SplitPointError`] when a point is out of bounds or the points
    /// are not strictly increasing.
    pub fn split_block_at_points(
        &mut self,
        id: BlockId,
        points: &[usize],
    ) -> Result<(Vec<BlockId>, RewriteMap), SplitPointError>
    where
        E: Default,
    {
        self.split_block_at_points_with_payloads(
            id,
            points.iter().copied().map(|point| (point, E::default())),
        )
    }

    /// Redirect all edges that target `old` to target `new_target` instead.
    ///
    /// This is useful for bypassing a block before removal.
    ///
    /// # Panics
    ///
    /// Panics if either block is out of range or an incoming edge was removed.
    pub fn redirect_edges_to(&mut self, old: BlockId, new_target: BlockId) {
        let _ = self.redirect_edges_to_mapped(old, new_target);
    }

    /// Redirect one edge's source while retaining its identity and payload.
    ///
    /// Returns the previous source.
    pub fn redirect_edge_source(&mut self, id: EdgeId, new_source: BlockId) -> BlockId {
        self.redirect_edge_source_mapped(id, new_source).0
    }

    /// Redirect one edge's source and return its stable identity mapping.
    ///
    /// # Panics
    ///
    /// Panics if the edge is not live or either endpoint is out of range.
    pub fn redirect_edge_source_mapped(
        &mut self,
        id: EdgeId,
        new_source: BlockId,
    ) -> (BlockId, RewriteMap) {
        let old_source = self.edge(id).source();
        if old_source == new_source {
            return (old_source, RewriteMap::new());
        }
        self.succs[old_source.index()].retain(|edge| *edge != id);
        self.succs[new_source.index()].push(id);
        self.edges[id.index()]
            .as_mut()
            .expect("edge has been removed")
            .source = new_source;
        let mut mapping = RewriteMap::new();
        mapping.record_edge(id, [id]);
        (old_source, mapping)
    }

    /// Redirect one edge's target while retaining its identity and payload.
    ///
    /// Returns the previous target.
    pub fn redirect_edge_target(&mut self, id: EdgeId, new_target: BlockId) -> BlockId {
        self.redirect_edge_target_mapped(id, new_target).0
    }

    /// Redirect one edge's target and return its stable identity mapping.
    ///
    /// # Panics
    ///
    /// Panics if the edge is not live or either endpoint is out of range.
    pub fn redirect_edge_target_mapped(
        &mut self,
        id: EdgeId,
        new_target: BlockId,
    ) -> (BlockId, RewriteMap) {
        let old_target = self.edge(id).target();
        if old_target == new_target {
            return (old_target, RewriteMap::new());
        }
        self.preds[old_target.index()].retain(|edge| *edge != id);
        self.preds[new_target.index()].push(id);
        self.edges[id.index()]
            .as_mut()
            .expect("edge has been removed")
            .target = new_target;
        let mut mapping = RewriteMap::new();
        mapping.record_edge(id, [id]);
        (old_target, mapping)
    }

    /// Redirect every edge targeting `old` and return their stable mapping.
    pub fn redirect_edges_to_mapped(&mut self, old: BlockId, new_target: BlockId) -> RewriteMap {
        let incoming: SmallVec<[EdgeId; 4]> = self.preds[old.index()].clone();
        let mut mapping = RewriteMap::new();
        for eid in incoming {
            self.redirect_edge_target(eid, new_target);
            mapping.record_edge(eid, [eid]);
        }
        mapping
    }

    /// Mutable access to an edge.
    ///
    /// # Panics
    ///
    /// Panics if `id` is out of range or has been removed.
    #[inline]
    pub fn edge_mut(&mut self, id: EdgeId) -> &mut Edge<E> {
        self.edges[id.index()]
            .as_mut()
            .expect("edge has been removed")
    }
}

// ── Default impl ──────────────────────────────────────────────────

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

// ── Graph view impls ───────────────────────────────────────────────

impl<I, E> crate::graph::view::DirectedGraphView for Cfg<I, E> {
    type NodeId = BlockId;

    fn node_count(&self) -> usize {
        self.num_blocks()
    }

    fn successors(&self, node: Self::NodeId) -> impl Iterator<Item = Self::NodeId> + '_ {
        Cfg::successors(self, node)
    }

    fn predecessors(&self, node: Self::NodeId) -> impl Iterator<Item = Self::NodeId> + '_ {
        Cfg::predecessors(self, node)
    }
}

impl<I, E> crate::graph::view::RootedGraphView for Cfg<I, E> {
    fn root(&self) -> Self::NodeId {
        self.entry()
    }
}

// ── Index impls ────────────────────────────────────────────────────

impl<I, E> Index<BlockId> for Cfg<I, E> {
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

impl<I, E> Index<EdgeId> for Cfg<I, E> {
    type Output = Edge<E>;

    /// Index into the CFG by [`EdgeId`].
    ///
    /// # Panics
    ///
    /// Panics if `id` does not refer to a live edge in this CFG.
    #[inline]
    fn index(&self, id: EdgeId) -> &Edge<E> {
        self.edges[id.index()]
            .as_ref()
            .expect("edge has been removed")
    }
}

/// Iterator over successor block ids (zero-allocation).
pub struct Successors<'a, I, E = ()> {
    cfg: &'a Cfg<I, E>,
    iter: slice::Iter<'a, EdgeId>,
}

impl<I, E> Iterator for Successors<'_, I, E> {
    type Item = BlockId;
    #[inline]
    fn next(&mut self) -> Option<BlockId> {
        self.iter.next().map(|&eid| self.cfg.edge(eid).target)
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<I, E> ExactSizeIterator for Successors<'_, I, E> {}

/// Iterator over predecessor block ids (zero-allocation).
pub struct Predecessors<'a, I, E = ()> {
    cfg: &'a Cfg<I, E>,
    iter: slice::Iter<'a, EdgeId>,
}

impl<I, E> Iterator for Predecessors<'_, I, E> {
    type Item = BlockId;
    #[inline]
    fn next(&mut self) -> Option<BlockId> {
        self.iter.next().map(|&eid| self.cfg.edge(eid).source)
    }
    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<I, E> ExactSizeIterator for Predecessors<'_, I, E> {}

// ── Convenience dataflow method ────────────────────────────────────
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

// ── Subgraph extraction ───────────────────────────────────────────
impl<I: Clone, E: Clone> Cfg<I, E> {
    /// Extract a sub-CFG containing only the specified blocks.
    ///
    /// The resulting CFG preserves edges between the selected blocks
    /// and remaps block IDs to be contiguous starting from 0.
    /// The first block in `blocks` becomes the entry.
    ///
    /// Edges that cross the boundary (one endpoint outside the set)
    /// are dropped.
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
    /// cfg.add_edge(b0, b1, EdgeKind::Fallthrough);
    /// cfg.add_edge(b1, b2, EdgeKind::Fallthrough);
    ///
    /// let sub = cfg.subgraph(&[b0, b1]);
    /// assert_eq!(sub.num_blocks(), 2);
    /// assert_eq!(sub.num_edges(), 1); // b1→b2 dropped
    /// ```
    #[must_use]
    pub fn subgraph(&self, blocks: &[BlockId]) -> Self {
        self.subgraph_mapped(blocks).0
    }

    /// Extract a sub-CFG and return a complete old-to-new identity mapping.
    #[must_use]
    pub fn subgraph_mapped(&self, blocks: &[BlockId]) -> (Self, RewriteMap) {
        let mut mapping = RewriteMap::new();
        if blocks.is_empty() {
            for block in self.blocks() {
                mapping.record_block(block.id(), []);
            }
            for edge in self.edges() {
                mapping.record_edge(edge.id(), []);
            }
            let empty = Self::new_with_edge_payload();
            mapping.record_created_block(empty.entry());
            return (empty, mapping);
        }

        let mut new_cfg = Self::new_with_edge_payload();

        // Map old BlockId → new BlockId via dense Vec (O(1) lookup).
        let mut id_map: Vec<Option<BlockId>> = alloc::vec![None; self.num_blocks()];
        id_map[blocks[0].index()] = Some(new_cfg.entry());
        mapping.record_block(blocks[0], [new_cfg.entry()]);
        mapping.record_created_block(new_cfg.entry());

        // Copy instructions into the entry block.
        let src = &self.blocks[blocks[0].index()];
        for inst in src.instructions() {
            new_cfg.block_mut(new_cfg.entry()).push(inst.clone());
        }
        if let Some(lbl) = src.label() {
            new_cfg.block_mut(new_cfg.entry()).set_label(lbl);
        }

        // Create remaining blocks.
        for &bid in &blocks[1..] {
            let new_id = new_cfg.new_block();
            id_map[bid.index()] = Some(new_id);
            mapping.record_block(bid, [new_id]);
            mapping.record_created_block(new_id);
            let old_block = &self.blocks[bid.index()];
            for inst in old_block.instructions() {
                new_cfg.block_mut(new_id).push(inst.clone());
            }
            if let Some(lbl) = old_block.label() {
                new_cfg.block_mut(new_id).set_label(lbl);
            }
        }

        // Copy live edges that stay within the subgraph.
        for edge in self.edges() {
            let new_src = id_map.get(edge.source().index()).copied().flatten();
            let new_tgt = id_map.get(edge.target().index()).copied().flatten();
            if let (Some(ns), Some(nt)) = (new_src, new_tgt) {
                let eid =
                    new_cfg.add_edge_with_payload(ns, nt, edge.kind(), edge.payload().clone());
                if let Some(w) = edge.weight() {
                    new_cfg.edge_mut(eid).set_weight(Some(w));
                }
                mapping.record_edge(edge.id(), [eid]);
                mapping.record_created_edge(eid);
            } else {
                mapping.record_edge(edge.id(), []);
            }
        }

        for block in self.blocks() {
            if id_map[block.id().index()].is_none() {
                mapping.record_block(block.id(), []);
            }
        }

        (new_cfg, mapping)
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;
    use crate::edge::EdgeKind;
    use crate::test_util::MockInst;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn edge_weight_roundtrip() {
        let mut cfg = Cfg::<MockInst>::new();
        let b0 = cfg.entry();
        let b1 = cfg.new_block();
        let eid = cfg.add_weighted_edge(b0, b1, EdgeKind::ConditionalTrue, 0.75);
        assert_eq!(cfg.edge(eid).weight(), Some(0.75));
        // Default edge should have no weight.
        let eid2 = cfg.add_edge(b0, b1, EdgeKind::Fallthrough);
        assert_eq!(cfg.edge(eid2).weight(), None);
    }

    #[test]
    fn subgraph_extraction() {
        let mut cfg = Cfg::<MockInst>::new();
        let b0 = cfg.entry();
        let b1 = cfg.new_block();
        let b2 = cfg.new_block();
        cfg.add_edge(b0, b1, EdgeKind::Fallthrough);
        cfg.add_edge(b1, b2, EdgeKind::Fallthrough);

        // Extract first two blocks.
        let sub = cfg.subgraph(&[b0, b1]);
        assert_eq!(sub.num_blocks(), 2);
        // The subgraph should have an edge from block 0 to block 1.
        let succs: Vec<BlockId> = sub.successors(sub.entry()).collect();
        assert_eq!(succs.len(), 1);
    }

    #[test]
    fn subgraph_empty_input() {
        let sub = Cfg::<MockInst>::new().subgraph(&[]);
        assert_eq!(sub.num_blocks(), 1); // Cfg::new() always has an entry
    }

    #[test]
    fn remove_edge_tombstones_correctly() {
        let mut cfg = Cfg::<MockInst>::new();
        let b0 = cfg.entry();
        let b1 = cfg.new_block();
        let b2 = cfg.new_block();
        let e1 = cfg.add_edge(b0, b1, EdgeKind::ConditionalTrue);
        let e2 = cfg.add_edge(b0, b2, EdgeKind::ConditionalFalse);

        // Both edges are live.
        assert_eq!(cfg.num_edges(), 2);
        assert_eq!(cfg.edges().count(), 2);

        // Remove one edge.
        let removed = cfg.remove_edge(e1).unwrap();
        assert_eq!(removed.kind(), EdgeKind::ConditionalTrue);

        // edges() should now skip the tombstone.
        assert_eq!(cfg.edges().count(), 1);
        let remaining: Vec<&Edge> = cfg.edges().collect();
        assert_eq!(remaining[0].id(), e2);

        // Successor list should only contain e2.
        assert_eq!(cfg.successor_edges(b0).len(), 1);
        assert_eq!(cfg.successor_edges(b0)[0], e2);

        // Double-remove returns None.
        assert!(cfg.remove_edge(e1).is_none());
    }

    #[test]
    fn exit_blocks_iterator() {
        let mut cfg = Cfg::<MockInst>::new();
        let b1 = cfg.new_block();
        cfg.add_edge(cfg.entry(), b1, EdgeKind::Fallthrough);
        // b1 has no outgoing edges — it's an exit block.
        let exits: Vec<BlockId> = cfg.exit_blocks().collect();
        assert_eq!(exits, vec![b1]);
    }
}
