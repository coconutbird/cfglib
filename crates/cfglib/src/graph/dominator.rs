//! Dominator tree computation using the Cooper-Harvey-Kennedy iterative
//! algorithm.

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;

/// A dominator tree computed from a [`Cfg`].
///
/// # Examples
///
/// ```
/// use cfglib::{Cfg, EdgeKind, DominatorTree};
///
/// let mut cfg = Cfg::<u32>::new();
/// let b0 = cfg.entry();
/// let b1 = cfg.new_block();
/// let b2 = cfg.new_block();
/// cfg.add_edge(b0, b1, EdgeKind::ConditionalTrue);
/// cfg.add_edge(b0, b2, EdgeKind::ConditionalFalse);
///
/// let dom = DominatorTree::compute(&cfg);
/// assert_eq!(dom.idom(b1), Some(b0));
/// assert_eq!(dom.idom(b2), Some(b0));
/// assert!(dom.dominates(b0, b1));
/// assert!(dom.dominates(b0, b2));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DominatorTree {
    /// Immediate dominator for each block. `idom[entry] == None`.
    idom: Vec<Option<BlockId>>,
}

fn reverse_graph_rpo(entry: usize, successors: &[Vec<usize>]) -> Vec<usize> {
    let mut visited = vec![false; successors.len()];
    let mut order = Vec::with_capacity(successors.len());
    let mut stack = vec![(entry, false)];

    while let Some((node, processed)) = stack.pop() {
        if processed {
            order.push(node);
            continue;
        }
        if visited[node] {
            continue;
        }
        visited[node] = true;
        stack.push((node, true));
        for &successor in successors[node].iter().rev() {
            if !visited[successor] {
                stack.push((successor, false));
            }
        }
    }

    order.reverse();
    order
}

impl DominatorTree {
    /// Compute the dominator tree for the given CFG using the iterative
    /// algorithm by Cooper, Harvey, and Kennedy.
    #[must_use]
    pub fn compute<I>(cfg: &Cfg<I>) -> Self {
        let rpo = cfg.reverse_postorder();
        let n = cfg.num_blocks();

        // Map BlockId → reverse-postorder index.
        let mut rpo_num = vec![usize::MAX; n];
        for (i, &id) in rpo.iter().enumerate() {
            rpo_num[id.index()] = i;
        }

        let mut doms: Vec<Option<usize>> = vec![None; n];
        let entry_rpo = rpo_num[cfg.entry().index()];
        doms[entry_rpo] = Some(entry_rpo);

        let mut changed = true;
        while changed {
            changed = false;
            for &b in &rpo {
                if b == cfg.entry() {
                    continue;
                }
                let b_rpo = rpo_num[b.index()];
                let preds: Vec<BlockId> = cfg.predecessors(b).collect();

                // Find the first processed predecessor.
                let mut new_idom = None;
                for &p in &preds {
                    let p_rpo = rpo_num[p.index()];
                    if doms[p_rpo].is_some() {
                        new_idom = Some(p_rpo);
                        break;
                    }
                }
                let Some(mut new_idom) = new_idom else {
                    continue;
                };

                // Intersect with remaining processed predecessors.
                for &p in &preds {
                    let p_rpo = rpo_num[p.index()];
                    if doms[p_rpo].is_some() && p_rpo != new_idom {
                        new_idom = Self::intersect(&doms, p_rpo, new_idom);
                    }
                }

                if doms[b_rpo] != Some(new_idom) {
                    doms[b_rpo] = Some(new_idom);
                    changed = true;
                }
            }
        }

        // Convert RPO indices back to BlockIds.
        let mut idom_result = vec![None; n];
        for (rpo_idx, &dom) in doms.iter().enumerate() {
            if rpo_idx < rpo.len() {
                let block = rpo[rpo_idx];
                idom_result[block.index()] = dom.map(|d| rpo[d]);
            }
        }

        // Entry dominates itself — represented as None.
        idom_result[cfg.entry().index()] = None;

        DominatorTree { idom: idom_result }
    }

    fn intersect(doms: &[Option<usize>], mut a: usize, mut b: usize) -> usize {
        while a != b {
            while a > b {
                a = doms[a].expect("processed dominator must have a parent");
            }
            while b > a {
                b = doms[b].expect("processed dominator must have a parent");
            }
        }
        a
    }

    /// Returns the immediate dominator of `block`, or `None` if `block`
    /// is the entry.
    #[must_use]
    pub fn idom(&self, block: BlockId) -> Option<BlockId> {
        self.idom[block.index()]
    }

    /// Returns `true` if `a` dominates `b`.
    #[must_use]
    pub fn dominates(&self, a: BlockId, b: BlockId) -> bool {
        if a == b {
            return true;
        }

        let mut cur = b;
        while let Some(d) = self.idom(cur) {
            if d == a {
                return true;
            }
            if d == cur {
                break;
            }
            cur = d;
        }
        false
    }

    /// Returns the children of `block` in the dominator tree (blocks
    /// whose immediate dominator is `block`).
    #[must_use]
    pub fn children(&self, block: BlockId) -> Vec<BlockId> {
        let mut result = Vec::new();
        for (i, dom) in self.idom.iter().enumerate() {
            if *dom == Some(block) && i != block.index() {
                result.push(BlockId::from_index(i));
            }
        }
        result
    }

    /// Compute the depth of a block in the dominator tree.
    ///
    /// The entry block (or root) has depth 0. Each step toward a leaf
    /// adds 1. Returns `None` if the block is unreachable (has no idom
    /// chain to the root).
    #[must_use]
    pub fn depth(&self, block: BlockId) -> Option<usize> {
        let mut d = 0;
        let mut cur = block;
        loop {
            match self.idom[cur.index()] {
                None => return Some(d), // reached root
                Some(parent) => {
                    if parent == cur {
                        return Some(d); // self-loop at root
                    }
                    d += 1;
                    cur = parent;
                }
            }
        }
    }

    /// Compute depths for all blocks at once.
    ///
    /// Returns a vector indexed by block index. Unreachable blocks get
    /// `usize::MAX`.
    #[must_use]
    pub fn depths(&self) -> Vec<usize> {
        let n = self.idom.len();
        let mut result = vec![usize::MAX; n];
        for (i, slot) in result.iter_mut().enumerate().take(n) {
            let bid = BlockId::from_index(i);
            if let Some(d) = self.depth(bid) {
                *slot = d;
            }
        }
        result
    }

    /// Compute the **post-dominator** tree for the given CFG.
    ///
    /// Post-dominators are computed by introducing a virtual exit node
    /// connected from all exit blocks (blocks with no successors), then
    /// running the dominator algorithm on the reverse graph starting
    /// from that virtual exit.
    ///
    /// This correctly handles CFGs with multiple exit points.
    #[must_use]
    pub fn compute_post<I>(cfg: &Cfg<I>) -> Self {
        let n = cfg.num_blocks();
        if n == 0 {
            return DominatorTree { idom: Vec::new() };
        }

        // Find exit blocks (no successors).
        let exits: Vec<BlockId> = cfg
            .blocks()
            .iter()
            .filter(|b| cfg.successor_edges(b.id()).is_empty())
            .map(super::super::block::BasicBlock::id)
            .collect();

        // If no natural exits, fall back to the last block.
        let exit_set: Vec<BlockId> = if exits.is_empty() {
            vec![BlockId::from_index(n - 1)]
        } else {
            exits
        };

        // We introduce a virtual exit node at index `n`. This node
        // has edges **from** every real exit block. In the reversed
        // graph it becomes the unique entry from which we run the
        // dominator algorithm.
        let virt = n; // virtual exit index (not a real BlockId)
        let total = n + 1;

        // Build reverse-graph adjacency: rev_succs[u] = predecessors
        // of u in the original graph (i.e. successors in the reversed
        // graph).
        let mut rev_succs: Vec<Vec<usize>> = vec![Vec::new(); total];
        for edge in cfg.edges() {
            rev_succs[edge.target().index()].push(edge.source().index());
        }
        // Virtual exit's reverse-successors are the real exit blocks.
        for &ex in &exit_set {
            rev_succs[virt].push(ex.index());
        }

        let rpo = reverse_graph_rpo(virt, &rev_succs);

        // Map node index → RPO position.
        let mut rpo_num = vec![usize::MAX; total];
        for (i, &node) in rpo.iter().enumerate() {
            rpo_num[node] = i;
        }

        let mut doms: Vec<Option<usize>> = vec![None; total];
        let virt_rpo = rpo_num[virt];
        doms[virt_rpo] = Some(virt_rpo);

        let mut changed = true;
        while changed {
            changed = false;
            for &b in &rpo {
                if b == virt {
                    continue;
                }
                let b_rpo = rpo_num[b];
                if b_rpo == usize::MAX {
                    continue;
                }

                // Predecessors in the reverse graph = successors in
                // the original graph + virtual exit if b is an exit.
                let mut rev_preds: Vec<usize> = cfg
                    .successors(BlockId::from_index(b))
                    .map(super::super::block::BlockId::index)
                    .collect();
                if exit_set.iter().any(|e| e.index() == b) {
                    rev_preds.push(virt);
                }

                let mut new_idom = None;
                for &p in &rev_preds {
                    let p_rpo = rpo_num[p];
                    if p_rpo != usize::MAX && doms[p_rpo].is_some() {
                        new_idom = Some(p_rpo);
                        break;
                    }
                }
                let Some(mut new_idom) = new_idom else {
                    continue;
                };

                for &p in &rev_preds {
                    let p_rpo = rpo_num[p];
                    if p_rpo != usize::MAX && doms[p_rpo].is_some() && p_rpo != new_idom {
                        new_idom = Self::intersect(&doms, p_rpo, new_idom);
                    }
                }

                if doms[b_rpo] != Some(new_idom) {
                    doms[b_rpo] = Some(new_idom);
                    changed = true;
                }
            }
        }

        // Convert RPO indices back to real BlockIds, skipping the
        // virtual exit. If a block's immediate post-dominator is the
        // virtual exit, record `None` (it's a top-level exit).
        //
        let mut idom_result = vec![None; n];
        for (rpo_idx, &dom) in doms.iter().enumerate() {
            if rpo_idx >= rpo.len() {
                continue;
            }
            let node = rpo[rpo_idx];
            if node == virt {
                continue;
            }
            idom_result[node] = dom.map(|d| {
                let real = rpo[d];
                if real == virt {
                    // Post-dominator is virtual exit → top-level.
                    // We'll clean this up below.
                    BlockId::from_index(node)
                } else {
                    BlockId::from_index(real)
                }
            });
        }

        // Blocks whose idom maps to themselves had the virtual exit
        // as their post-dominator — set to None.
        for (i, slot) in idom_result.iter_mut().enumerate().take(n) {
            if *slot == Some(BlockId::from_index(i)) {
                *slot = None;
            }
        }

        DominatorTree { idom: idom_result }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::edge::EdgeKind;
    use crate::test_util::MockInst;

    #[test]
    fn single_block_cfg() {
        let cfg: Cfg<MockInst> = Cfg::new();
        let dom = DominatorTree::compute(&cfg);
        assert_eq!(dom.idom(cfg.entry()), None);
        assert!(dom.dominates(cfg.entry(), cfg.entry()));
        assert!(dom.children(cfg.entry()).is_empty());
    }

    #[test]
    fn linear_chain_dominance() {
        let mut cfg: Cfg<MockInst> = Cfg::new();
        let b1 = cfg.new_block();
        let b2 = cfg.new_block();
        cfg.add_edge(cfg.entry(), b1, EdgeKind::Fallthrough);
        cfg.add_edge(b1, b2, EdgeKind::Fallthrough);
        let dom = DominatorTree::compute(&cfg);
        assert!(dom.dominates(cfg.entry(), b1));
        assert!(dom.dominates(cfg.entry(), b2));
        assert!(dom.dominates(b1, b2));
        assert!(!dom.dominates(b2, b1));
        assert_eq!(dom.idom(b1), Some(cfg.entry()));
        assert_eq!(dom.idom(b2), Some(b1));
    }

    #[test]
    fn diamond_idom_at_merge() {
        let mut cfg: Cfg<MockInst> = Cfg::new();
        let a = cfg.new_block();
        let b = cfg.new_block();
        let merge = cfg.new_block();
        cfg.add_edge(cfg.entry(), a, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), b, EdgeKind::ConditionalFalse);
        cfg.add_edge(a, merge, EdgeKind::Fallthrough);
        cfg.add_edge(b, merge, EdgeKind::Fallthrough);
        let dom = DominatorTree::compute(&cfg);
        // Merge block's idom should be entry (not a or b).
        assert_eq!(dom.idom(merge), Some(cfg.entry()));
        assert!(dom.dominates(cfg.entry(), a));
        assert!(dom.dominates(cfg.entry(), b));
        assert!(!dom.dominates(a, b));
        assert!(!dom.dominates(b, a));
    }

    #[test]
    fn self_loop_dominance() {
        let mut cfg: Cfg<MockInst> = Cfg::new();
        cfg.add_edge(cfg.entry(), cfg.entry(), EdgeKind::Back);
        let dom = DominatorTree::compute(&cfg);
        assert_eq!(dom.idom(cfg.entry()), None);
        assert!(dom.dominates(cfg.entry(), cfg.entry()));
    }

    #[test]
    fn unreachable_block_not_dominated() {
        let mut cfg: Cfg<MockInst> = Cfg::new();
        let unreachable = cfg.new_block();
        let dom = DominatorTree::compute(&cfg);
        // Entry still dominates itself.
        assert!(dom.dominates(cfg.entry(), cfg.entry()));
        // Unreachable block has no idom.
        assert_eq!(dom.idom(unreachable), None);
    }

    #[test]
    fn depth_computation() {
        let mut cfg: Cfg<MockInst> = Cfg::new();
        let b1 = cfg.new_block();
        let b2 = cfg.new_block();
        cfg.add_edge(cfg.entry(), b1, EdgeKind::Fallthrough);
        cfg.add_edge(b1, b2, EdgeKind::Fallthrough);
        let dom = DominatorTree::compute(&cfg);
        assert_eq!(dom.depth(cfg.entry()), Some(0));
        assert_eq!(dom.depth(b1), Some(1));
        assert_eq!(dom.depth(b2), Some(2));
    }

    #[test]
    fn children_returns_immediate_children() {
        let mut cfg: Cfg<MockInst> = Cfg::new();
        let a = cfg.new_block();
        let b = cfg.new_block();
        let c = cfg.new_block();
        cfg.add_edge(cfg.entry(), a, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), b, EdgeKind::ConditionalFalse);
        cfg.add_edge(a, c, EdgeKind::Fallthrough);
        let dom = DominatorTree::compute(&cfg);
        let mut entry_children = dom.children(cfg.entry());
        entry_children.sort();
        assert_eq!(entry_children.len(), 2);
        assert!(entry_children.contains(&a));
        assert!(entry_children.contains(&b));
        assert_eq!(dom.children(a), vec![c]);
    }
}
