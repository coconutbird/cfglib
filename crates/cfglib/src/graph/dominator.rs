//! Dominator tree computation using the Cooper-Harvey-Kennedy iterative
//! algorithm.

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;

/// A dominator tree computed from a [`Cfg`].
#[derive(Debug, Clone)]
pub struct DominatorTree {
    /// Immediate dominator for each block. `idom[entry] == None`.
    idom: Vec<Option<BlockId>>,
}

impl DominatorTree {
    /// Compute the dominator tree for the given CFG using the iterative
    /// algorithm by Cooper, Harvey, and Kennedy.
    pub fn compute<I>(cfg: &Cfg<I>) -> Self {
        let rpo = cfg.reverse_postorder();
        let n = cfg.num_blocks();

        // Map BlockId → reverse-postorder index.
        let mut rpo_num = vec![u32::MAX; n];
        for (i, &id) in rpo.iter().enumerate() {
            rpo_num[id.index()] = i as u32;
        }

        let mut doms: Vec<Option<u32>> = vec![None; n];
        let entry_rpo = rpo_num[cfg.entry().index()] as usize;
        doms[entry_rpo] = Some(entry_rpo as u32);

        let mut changed = true;
        while changed {
            changed = false;
            for &b in &rpo {
                if b == cfg.entry() {
                    continue;
                }
                let b_rpo = rpo_num[b.index()] as usize;
                let preds: Vec<BlockId> = cfg.predecessors(b).collect();

                // Find the first processed predecessor.
                let mut new_idom = None;
                for &p in &preds {
                    let p_rpo = rpo_num[p.index()] as usize;
                    if doms[p_rpo].is_some() {
                        new_idom = Some(p_rpo as u32);
                        break;
                    }
                }
                let Some(mut new_idom) = new_idom else {
                    continue;
                };

                // Intersect with remaining processed predecessors.
                for &p in &preds {
                    let p_rpo = rpo_num[p.index()] as usize;
                    if doms[p_rpo].is_some() && p_rpo as u32 != new_idom {
                        new_idom = Self::intersect(&doms, p_rpo as u32, new_idom);
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
                idom_result[block.index()] = dom.map(|d| rpo[d as usize]);
            }
        }

        // Entry dominates itself — represented as None.
        idom_result[cfg.entry().index()] = None;

        DominatorTree { idom: idom_result }
    }

    fn intersect(doms: &[Option<u32>], mut a: u32, mut b: u32) -> u32 {
        while a != b {
            while a > b {
                a = doms[a as usize].unwrap();
            }
            while b > a {
                b = doms[b as usize].unwrap();
            }
        }
        a
    }

    /// Returns the immediate dominator of `block`, or `None` if `block`
    /// is the entry.
    pub fn idom(&self, block: BlockId) -> Option<BlockId> {
        self.idom[block.index()]
    }

    /// Returns `true` if `a` dominates `b`.
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
    pub fn children(&self, block: BlockId) -> Vec<BlockId> {
        let mut result = Vec::new();
        for (i, dom) in self.idom.iter().enumerate() {
            if *dom == Some(block) && i != block.index() {
                result.push(BlockId(i as u32));
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
            .map(|b| b.id())
            .collect();

        // If no natural exits, fall back to the last block.
        let exit_set: Vec<BlockId> = if exits.is_empty() {
            vec![BlockId((n - 1) as u32)]
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

        // Iterative DFS postorder on the reverse graph, starting from
        // the virtual exit.
        let mut visited = vec![false; total];
        let mut rpo: Vec<usize> = Vec::with_capacity(total);
        {
            let mut stack: Vec<(usize, bool)> = vec![(virt, false)];
            while let Some((node, processed)) = stack.pop() {
                if processed {
                    rpo.push(node);
                    continue;
                }
                if visited[node] {
                    continue;
                }
                visited[node] = true;
                stack.push((node, true));
                for &succ in rev_succs[node].iter().rev() {
                    if !visited[succ] {
                        stack.push((succ, false));
                    }
                }
            }
            rpo.reverse();
        }

        // Map node index → RPO position.
        let mut rpo_num = vec![u32::MAX; total];
        for (i, &node) in rpo.iter().enumerate() {
            rpo_num[node] = i as u32;
        }

        let mut doms: Vec<Option<u32>> = vec![None; total];
        let virt_rpo = rpo_num[virt] as usize;
        doms[virt_rpo] = Some(virt_rpo as u32);

        let mut changed = true;
        while changed {
            changed = false;
            for &b in &rpo {
                if b == virt {
                    continue;
                }
                let b_rpo = rpo_num[b];
                if b_rpo == u32::MAX {
                    continue;
                }
                let b_rpo = b_rpo as usize;

                // Predecessors in the reverse graph = successors in
                // the original graph + virtual exit if b is an exit.
                let mut rev_preds: Vec<usize> = cfg
                    .successors(BlockId(b as u32))
                    .map(|s| s.index())
                    .collect();
                if exit_set.iter().any(|e| e.index() == b) {
                    rev_preds.push(virt);
                }

                let mut new_idom = None;
                for &p in &rev_preds {
                    let p_rpo = rpo_num[p];
                    if p_rpo != u32::MAX && doms[p_rpo as usize].is_some() {
                        new_idom = Some(p_rpo);
                        break;
                    }
                }
                let Some(mut new_idom) = new_idom else {
                    continue;
                };

                for &p in &rev_preds {
                    let p_rpo = rpo_num[p];
                    if p_rpo != u32::MAX && doms[p_rpo as usize].is_some() && p_rpo != new_idom {
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
                let real = rpo[d as usize];
                if real == virt {
                    // Post-dominator is virtual exit → top-level.
                    // We'll clean this up below.
                    BlockId(node as u32)
                } else {
                    BlockId(real as u32)
                }
            });
        }

        // Blocks whose idom maps to themselves had the virtual exit
        // as their post-dominator — set to None.
        for i in 0..n {
            if idom_result[i] == Some(BlockId(i as u32)) {
                idom_result[i] = None;
            }
        }

        DominatorTree { idom: idom_result }
    }
}
