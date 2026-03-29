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
                let preds = cfg.predecessors(b);

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
    /// Post-dominators are computed by running the dominator algorithm
    /// on the reverse graph (predecessors become successors) starting
    /// from exit blocks (blocks with no successors).
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
        let virtual_exit = if exits.is_empty() {
            BlockId((n - 1) as u32)
        } else if exits.len() == 1 {
            exits[0]
        } else {
            // Multiple exits — we pick the first and treat it as the
            // virtual root. For structured CFGs this is usually fine.
            exits[0]
        };

        // Compute reverse-postorder on the **reversed** graph.
        let mut visited = vec![false; n];
        let mut rpo = Vec::with_capacity(n);

        fn reverse_dfs_post<I>(
            cfg: &Cfg<I>,
            block: BlockId,
            visited: &mut Vec<bool>,
            order: &mut Vec<BlockId>,
        ) {
            visited[block.index()] = true;
            for pred in cfg.predecessors(block) {
                if !visited[pred.index()] {
                    reverse_dfs_post(cfg, pred, visited, order);
                }
            }
            order.push(block);
        }

        reverse_dfs_post(cfg, virtual_exit, &mut visited, &mut rpo);
        rpo.reverse(); // Now in reverse postorder of the reversed graph.

        // Map BlockId → RPO index.
        let mut rpo_num = vec![u32::MAX; n];
        for (i, &id) in rpo.iter().enumerate() {
            rpo_num[id.index()] = i as u32;
        }

        let mut doms: Vec<Option<u32>> = vec![None; n];
        let exit_rpo = rpo_num[virtual_exit.index()] as usize;
        doms[exit_rpo] = Some(exit_rpo as u32);

        let mut changed = true;
        while changed {
            changed = false;
            for &b in &rpo {
                if b == virtual_exit {
                    continue;
                }
                let b_rpo = rpo_num[b.index()];
                if b_rpo == u32::MAX {
                    continue;
                }
                let b_rpo = b_rpo as usize;

                // In the reversed graph, successors are predecessors.
                let rev_preds = cfg.successors(b);

                let mut new_idom = None;
                for &p in &rev_preds {
                    let p_rpo = rpo_num[p.index()];
                    if p_rpo != u32::MAX && doms[p_rpo as usize].is_some() {
                        new_idom = Some(p_rpo);
                        break;
                    }
                }
                let Some(mut new_idom) = new_idom else {
                    continue;
                };

                for &p in &rev_preds {
                    let p_rpo = rpo_num[p.index()];
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

        // Convert RPO indices back to BlockIds.
        let mut idom_result = vec![None; n];
        for (rpo_idx, &dom) in doms.iter().enumerate() {
            if rpo_idx < rpo.len() {
                let block = rpo[rpo_idx];
                idom_result[block.index()] = dom.map(|d| rpo[d as usize]);
            }
        }

        // Exit post-dominates itself — represented as None.
        idom_result[virtual_exit.index()] = None;

        DominatorTree { idom: idom_result }
    }
}
