//! Subgraph extraction from a [`Cfg`].

extern crate alloc;

use alloc::vec::Vec;

use crate::block::BlockId;
use crate::rewrite::RewriteMap;

use super::Cfg;

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
    /// assert_eq!(sub.block_count(), 2);
    /// assert_eq!(sub.edge_count(), 1); // b1→b2 dropped
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
            let empty = Self::with_edge_payload();
            mapping.record_created_block(empty.entry());
            return (empty, mapping);
        }

        let mut new_cfg = Self::with_edge_payload();

        // Map old BlockId → new BlockId via dense Vec (O(1) lookup).
        let mut id_map: Vec<Option<BlockId>> = alloc::vec![None; self.block_count()];
        id_map[blocks[0].index()] = Some(new_cfg.entry());
        mapping.record_block(blocks[0], [new_cfg.entry()]);
        mapping.record_created_block(new_cfg.entry());

        let src = &self.blocks[blocks[0].index()];
        for inst in src.instructions() {
            new_cfg.block_mut(new_cfg.entry()).push(inst.clone());
        }
        if let Some(lbl) = src.label() {
            new_cfg.block_mut(new_cfg.entry()).set_label(lbl);
        }

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
