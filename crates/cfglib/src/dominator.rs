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
}
