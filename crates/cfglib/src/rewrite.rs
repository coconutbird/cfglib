//! Explicit identity remapping for control-flow graph rewrites.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::{BlockId, EdgeId};

/// Old-to-new block and edge identities produced by one graph rewrite.
///
/// Only affected old identities appear in the maps. A missing entry therefore
/// means "unchanged"; an entry with no replacements means "removed"; one
/// replacement means retained or redirected; several replacements mean an
/// identity expanded, as when one block or edge is split. Every identity
/// allocated by the rewrite is listed separately, including a new identity
/// that is also one of an old identity's replacements.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RewriteMap {
    blocks: BTreeMap<BlockId, Vec<BlockId>>,
    edges: BTreeMap<EdgeId, Vec<EdgeId>>,
    created_blocks: Vec<BlockId>,
    created_edges: Vec<EdgeId>,
}

impl RewriteMap {
    /// Create an empty identity rewrite.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            blocks: BTreeMap::new(),
            edges: BTreeMap::new(),
            created_blocks: Vec::new(),
            created_edges: Vec::new(),
        }
    }

    /// Whether the rewrite records no affected or created identities.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
            && self.edges.is_empty()
            && self.created_blocks.is_empty()
            && self.created_edges.is_empty()
    }

    /// Replacements of an affected old block, or `None` when it was unchanged.
    #[must_use]
    pub fn block_replacements(&self, old: BlockId) -> Option<&[BlockId]> {
        self.blocks.get(&old).map(Vec::as_slice)
    }

    /// Replacements of an affected old edge, or `None` when it was unchanged.
    #[must_use]
    pub fn edge_replacements(&self, old: EdgeId) -> Option<&[EdgeId]> {
        self.edges.get(&old).map(Vec::as_slice)
    }

    /// Blocks allocated without an old identity.
    #[must_use]
    pub fn created_blocks(&self) -> &[BlockId] {
        &self.created_blocks
    }

    /// Edges allocated without an old identity.
    #[must_use]
    pub fn created_edges(&self) -> &[EdgeId] {
        &self.created_edges
    }

    /// Iterate over all explicitly affected old blocks in identity order.
    pub fn block_mappings(&self) -> impl Iterator<Item = (BlockId, &[BlockId])> {
        self.blocks
            .iter()
            .map(|(&old, replacements)| (old, replacements.as_slice()))
    }

    /// Iterate over all explicitly affected old edges in identity order.
    pub fn edge_mappings(&self) -> impl Iterator<Item = (EdgeId, &[EdgeId])> {
        self.edges
            .iter()
            .map(|(&old, replacements)| (old, replacements.as_slice()))
    }

    /// Record the complete replacement set of one old block.
    pub fn record_block(&mut self, old: BlockId, replacements: impl IntoIterator<Item = BlockId>) {
        self.blocks.insert(old, unique(replacements));
    }

    /// Record the complete replacement set of one old edge.
    pub fn record_edge(&mut self, old: EdgeId, replacements: impl IntoIterator<Item = EdgeId>) {
        self.edges.insert(old, unique(replacements));
    }

    /// Record a newly allocated block with no old identity.
    pub fn record_created_block(&mut self, block: BlockId) {
        if !self.created_blocks.contains(&block) {
            self.created_blocks.push(block);
        }
    }

    /// Record a newly allocated edge with no old identity.
    pub fn record_created_edge(&mut self, edge: EdgeId) {
        if !self.created_edges.contains(&edge) {
            self.created_edges.push(edge);
        }
    }

    /// Compose `next`, which ran after this rewrite, into this mapping.
    ///
    /// Replacements already recorded here are chased through `next`; mappings
    /// introduced only by `next` are then added. This preserves a direct view
    /// from identities before the first rewrite to identities after the last.
    pub fn compose(&mut self, next: Self) {
        compose_axis(&mut self.blocks, &next.blocks);
        compose_axis(&mut self.edges, &next.edges);

        for (old, replacements) in next.blocks {
            self.blocks.entry(old).or_insert(replacements);
        }
        for (old, replacements) in next.edges {
            self.edges.entry(old).or_insert(replacements);
        }

        self.created_blocks = remap_created(self.created_blocks.drain(..), &self.blocks);
        self.created_edges = remap_created(self.created_edges.drain(..), &self.edges);
        for block in next.created_blocks {
            if !self.created_blocks.contains(&block) {
                self.created_blocks.push(block);
            }
        }
        for edge in next.created_edges {
            if !self.created_edges.contains(&edge) {
                self.created_edges.push(edge);
            }
        }
    }
}

fn unique<T: Copy + PartialEq>(values: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut result = Vec::new();
    for value in values {
        if !result.contains(&value) {
            result.push(value);
        }
    }
    result
}

fn compose_axis<T: Copy + Ord>(current: &mut BTreeMap<T, Vec<T>>, next: &BTreeMap<T, Vec<T>>) {
    for replacements in current.values_mut() {
        let mut composed = Vec::new();
        for replacement in replacements.drain(..) {
            if let Some(next_replacements) = next.get(&replacement) {
                for &value in next_replacements {
                    if !composed.contains(&value) {
                        composed.push(value);
                    }
                }
            } else if !composed.contains(&replacement) {
                composed.push(replacement);
            }
        }
        *replacements = composed;
    }
}

fn remap_created<T: Copy + Ord>(
    created: impl Iterator<Item = T>,
    mappings: &BTreeMap<T, Vec<T>>,
) -> Vec<T> {
    let mut remapped = Vec::new();
    for identity in created {
        let replacements = mappings
            .get(&identity)
            .map_or_else(|| core::slice::from_ref(&identity), Vec::as_slice);
        for &replacement in replacements {
            if !remapped.contains(&replacement) {
                remapped.push(replacement);
            }
        }
    }
    remapped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_chases_replacements_and_removals() {
        let a = BlockId::from_index(0);
        let b = BlockId::from_index(1);
        let c = BlockId::from_index(2);
        let d = BlockId::from_index(3);
        let mut first = RewriteMap::new();
        first.record_block(a, [b, c]);
        first.record_created_block(c);

        let mut second = RewriteMap::new();
        second.record_block(b, [d]);
        second.record_block(c, []);
        first.compose(second);

        assert_eq!(first.block_replacements(a), Some([d].as_slice()));
        assert_eq!(first.block_replacements(c), Some([].as_slice()));
        assert!(first.created_blocks().is_empty());
    }
}
