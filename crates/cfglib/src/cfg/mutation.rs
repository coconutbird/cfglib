//! Structural mutation of [`Cfg`] — edge removal, redirection, and block splitting.

extern crate alloc;

use alloc::vec::Vec;
use smallvec::SmallVec;

use crate::block::{BasicBlock, BlockId};
use crate::edge::{Edge, EdgeId, EdgeKind};
use crate::rewrite::RewriteMap;

use super::{Cfg, SplitPointError};

impl<I, E> Cfg<I, E> {
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
    /// assert_eq!(cfg.edge_count(), 1);
    /// let removed = cfg.remove_edge(eid).unwrap();
    /// assert_eq!(removed.kind(), EdgeKind::Fallthrough);
    /// assert_eq!(cfg.edge_count(), 0);
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
        self.split_block_with_payload_inner(id, at, fallthrough_payload, None)
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
        let mut mapping = RewriteMap::new();
        let new_id =
            self.split_block_with_payload_inner(id, at, fallthrough_payload, Some(&mut mapping));
        (new_id, mapping)
    }

    fn split_block_with_payload_inner(
        &mut self,
        id: BlockId,
        at: usize,
        fallthrough_payload: E,
        mapping: Option<&mut RewriteMap>,
    ) -> BlockId {
        let tail_insts: Vec<I> = self.blocks[id.index()].instructions.split_off(at);
        let new_id = self.new_block();
        self.blocks[new_id.index()].instructions = tail_insts;

        self.move_outgoing_edges(id, new_id);

        let fallthrough =
            self.add_edge_with_payload(id, new_id, EdgeKind::Fallthrough, fallthrough_payload);

        if let Some(mapping) = mapping {
            mapping.record_block(id, [id, new_id]);
            mapping.record_created_block(new_id);
            for &edge in &self.succs[new_id.index()] {
                mapping.record_edge(edge, [edge]);
            }
            mapping.record_created_edge(fallthrough);
        }
        new_id
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
            if let Some(previous) = previous {
                if point <= previous {
                    return Err(SplitPointError::NotStrictlyIncreasing { previous, point });
                }
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
        self.redirect_edges_to_inner(old, new_target, None);
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
        let mut mapping = RewriteMap::new();
        self.redirect_edges_to_inner(old, new_target, Some(&mut mapping));
        mapping
    }

    fn redirect_edges_to_inner(
        &mut self,
        old: BlockId,
        new_target: BlockId,
        mut mapping: Option<&mut RewriteMap>,
    ) {
        let old_index = old.index();
        let new_target_index = new_target.index();
        let _ = &self.preds[old_index];
        let _ = &self.preds[new_target_index];
        if old == new_target {
            return;
        }

        for &edge in &self.preds[old_index] {
            self.edges[edge.index()]
                .as_ref()
                .expect("CFG predecessor adjacency must reference a live edge");
        }

        let incoming = core::mem::take(&mut self.preds[old_index]);
        for &edge in &incoming {
            self.edges[edge.index()]
                .as_mut()
                .expect("validated CFG edge must remain live")
                .target = new_target;
            if let Some(mapping) = mapping.as_deref_mut() {
                mapping.record_edge(edge, [edge]);
            }
        }
        if self.preds[new_target_index].is_empty() {
            self.preds[new_target_index] = incoming;
        } else {
            self.preds[new_target_index].extend(incoming);
        }
    }

    /// Move every outgoing edge of `old` to `new_source` in adjacency order.
    ///
    /// Only the source endpoint changes: edge identities, targets, kinds,
    /// weights, payloads, and predecessor adjacency remain intact. When the
    /// destination has no outgoing edges, its complete adjacency buffer moves
    /// without reallocating.
    pub(crate) fn move_outgoing_edges(&mut self, old: BlockId, new_source: BlockId) {
        let old_index = old.index();
        let new_source_index = new_source.index();
        let _ = &self.succs[old_index];
        let _ = &self.succs[new_source_index];
        if old == new_source {
            return;
        }

        for &edge in &self.succs[old_index] {
            self.edges[edge.index()]
                .as_ref()
                .expect("CFG successor adjacency must reference a live edge");
        }

        let outgoing = core::mem::take(&mut self.succs[old_index]);
        for &edge in &outgoing {
            self.edges[edge.index()]
                .as_mut()
                .expect("validated CFG edge must remain live")
                .source = new_source;
        }
        if self.succs[new_source_index].is_empty() {
            self.succs[new_source_index] = outgoing;
        } else {
            self.succs[new_source_index].extend(outgoing);
        }
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
