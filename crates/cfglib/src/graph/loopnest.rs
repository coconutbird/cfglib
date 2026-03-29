//! Loop nesting tree — hierarchical representation of loop nesting.
//!
//! Builds a proper tree from the flat [`NaturalLoop`] list returned by
//! [`detect_loops`], enabling parent/child queries and nesting depth.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::graph::structure::NaturalLoop;

/// A node in the loop nesting tree.
#[derive(Debug, Clone)]
pub struct LoopNestNode {
    /// Index into the original `NaturalLoop` slice.
    pub loop_index: usize,
    /// Header block of this loop.
    pub header: BlockId,
    /// Parent loop index (None for outermost loops).
    pub parent: Option<usize>,
    /// Indices of directly nested child loops.
    pub children: Vec<usize>,
    /// Nesting depth (0 for outermost).
    pub depth: usize,
}

/// A loop nesting tree built from detected natural loops.
#[derive(Debug, Clone)]
pub struct LoopNestingTree {
    /// One node per natural loop, indexed by loop index.
    pub nodes: Vec<LoopNestNode>,
    /// Block → innermost containing loop index.
    block_to_loop: BTreeMap<BlockId, usize>,
}

impl LoopNestingTree {
    /// Build the nesting tree from a slice of natural loops.
    ///
    /// Loops are assumed to be sorted by depth (outermost first),
    /// which is the order returned by [`detect_loops`].
    pub fn build(loops: &[NaturalLoop]) -> Self {
        let n = loops.len();
        let mut nodes: Vec<LoopNestNode> = loops
            .iter()
            .enumerate()
            .map(|(i, lp)| LoopNestNode {
                loop_index: i,
                header: lp.header,
                parent: None,
                children: Vec::new(),
                depth: 0,
            })
            .collect();

        // Determine parent: loop i's parent is the smallest loop j
        // (j != i) whose body contains i's header.
        for i in 0..n {
            let header_i = loops[i].header;
            let mut best_parent: Option<usize> = None;
            let mut best_size = usize::MAX;
            for (j, lp_j) in loops.iter().enumerate() {
                if i == j {
                    continue;
                }
                if lp_j.body.contains(&header_i) && lp_j.body.len() < best_size {
                    best_parent = Some(j);
                    best_size = lp_j.body.len();
                }
            }
            nodes[i].parent = best_parent;
            if let Some(p) = best_parent {
                nodes[p].children.push(i);
            }
        }

        // Compute depths from parent chain.
        for i in 0..n {
            let mut d = 0;
            let mut cur = i;
            while let Some(p) = nodes[cur].parent {
                d += 1;
                cur = p;
            }
            nodes[i].depth = d;
        }

        // Map blocks to innermost loop.
        let mut block_to_loop = BTreeMap::new();
        for (i, lp) in loops.iter().enumerate() {
            for &bid in &lp.body {
                let entry = block_to_loop.entry(bid).or_insert(i);
                // Prefer deeper (more nested) loop.
                if nodes[i].depth > nodes[*entry].depth {
                    *entry = i;
                }
            }
        }

        Self {
            nodes,
            block_to_loop,
        }
    }

    /// Get the innermost loop containing a block, if any.
    pub fn innermost_loop(&self, block: BlockId) -> Option<usize> {
        self.block_to_loop.get(&block).copied()
    }

    /// Get outermost (root) loops — those with no parent.
    pub fn roots(&self) -> Vec<usize> {
        self.nodes
            .iter()
            .filter(|n| n.parent.is_none())
            .map(|n| n.loop_index)
            .collect()
    }

    /// Nesting depth of a loop (0 for outermost).
    pub fn depth(&self, loop_index: usize) -> usize {
        self.nodes[loop_index].depth
    }

    /// Number of loops.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the tree is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeSet;

    fn make_loop(header: u32, body: &[u32], depth: usize) -> NaturalLoop {
        NaturalLoop {
            header: BlockId::from_raw(header),
            body: body
                .iter()
                .map(|&b| BlockId::from_raw(b))
                .collect::<BTreeSet<_>>(),
            latches: BTreeSet::new(),
            depth,
        }
    }

    #[test]
    fn nested_loops() {
        let outer = make_loop(1, &[1, 2, 3], 0);
        let inner = make_loop(2, &[2, 3], 1);
        let tree = LoopNestingTree::build(&[outer, inner]);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree.depth(0), 0);
        assert_eq!(tree.depth(1), 1);
        assert_eq!(tree.nodes[1].parent, Some(0));
        assert!(tree.nodes[0].children.contains(&1));
    }

    #[test]
    fn innermost_loop_query() {
        let outer = make_loop(1, &[1, 2, 3], 0);
        let inner = make_loop(2, &[2, 3], 1);
        let tree = LoopNestingTree::build(&[outer, inner]);
        // Block 3 is in both loops, innermost is index 1.
        assert_eq!(tree.innermost_loop(BlockId::from_raw(3)), Some(1));
        // Block 1 is only in outer.
        assert_eq!(tree.innermost_loop(BlockId::from_raw(1)), Some(0));
    }
}
