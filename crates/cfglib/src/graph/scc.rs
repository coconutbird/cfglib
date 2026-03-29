//! Strongly Connected Components via Tarjan's algorithm.
//!
//! Computes SCCs in a single DFS pass with O(V + E) complexity.
//! The result is returned in reverse topological order (leaves first),
//! which is the natural order for bottom-up analyses.

extern crate alloc;
use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;

/// A strongly connected component — a maximal set of mutually
/// reachable blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scc {
    /// The blocks in this SCC.
    pub blocks: BTreeSet<BlockId>,
}

impl Scc {
    /// Whether this SCC is trivial (single block, no self-loop).
    pub fn is_trivial(&self) -> bool {
        self.blocks.len() == 1
    }

    /// Whether the given block is in this SCC.
    pub fn contains(&self, block: BlockId) -> bool {
        self.blocks.contains(&block)
    }
}

/// Result of Tarjan's SCC computation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SccResult {
    /// SCCs in **reverse topological order** (leaves first).
    pub sccs: Vec<Scc>,
    /// Map from block index → SCC index in `sccs`.
    scc_of: Vec<usize>,
}

impl SccResult {
    /// The SCC index for a given block.
    pub fn scc_index(&self, block: BlockId) -> usize {
        self.scc_of[block.index()]
    }

    /// The SCC containing a given block.
    pub fn scc_for(&self, block: BlockId) -> &Scc {
        &self.sccs[self.scc_of[block.index()]]
    }

    /// Number of SCCs.
    pub fn num_sccs(&self) -> usize {
        self.sccs.len()
    }

    /// Whether the graph is a DAG (every SCC is trivial).
    pub fn is_dag<I>(&self, cfg: &Cfg<I>) -> bool {
        self.sccs.iter().all(|scc| {
            if scc.blocks.len() != 1 {
                return false;
            }
            // Check for self-loop.
            let b = *scc.blocks.iter().next().unwrap();
            !cfg.successors(b).any(|s| s == b)
        })
    }
}

/// Compute strongly connected components using Tarjan's algorithm.
pub fn tarjan_scc<I>(cfg: &Cfg<I>) -> SccResult {
    let n = cfg.num_blocks();

    let mut index_counter: u32 = 0;
    let mut stack: Vec<BlockId> = Vec::new();
    let mut on_stack = vec![false; n];
    let mut indices = vec![u32::MAX; n]; // u32::MAX = undefined
    let mut lowlinks = vec![0u32; n];
    let mut scc_of = vec![0usize; n];
    let mut sccs: Vec<Scc> = Vec::new();

    // Iterative Tarjan's using an explicit call stack.
    for start_raw in 0..n as u32 {
        let start = BlockId(start_raw);
        if indices[start.index()] != u32::MAX {
            continue;
        }

        // (node, successor_iterator_index, is_root_call)
        let mut call_stack: Vec<(BlockId, Vec<BlockId>, usize)> = Vec::new();
        // Push initial frame.
        indices[start.index()] = index_counter;
        lowlinks[start.index()] = index_counter;
        index_counter += 1;
        stack.push(start);
        on_stack[start.index()] = true;
        let succs: Vec<BlockId> = cfg.successors(start).collect();
        call_stack.push((start, succs, 0));

        while let Some((v, ref succs_list, si)) = call_stack.last().cloned() {
            if si < succs_list.len() {
                let w = succs_list[si];
                // Advance iterator.
                call_stack.last_mut().unwrap().2 = si + 1;

                if indices[w.index()] == u32::MAX {
                    // Not yet visited — recurse.
                    indices[w.index()] = index_counter;
                    lowlinks[w.index()] = index_counter;
                    index_counter += 1;
                    stack.push(w);
                    on_stack[w.index()] = true;
                    let w_succs: Vec<BlockId> = cfg.successors(w).collect();
                    call_stack.push((w, w_succs, 0));
                } else if on_stack[w.index()] {
                    let vl = lowlinks[v.index()];
                    let wi = indices[w.index()];
                    lowlinks[v.index()] = vl.min(wi);
                }
            } else {
                // Done with v's successors.
                if lowlinks[v.index()] == indices[v.index()] {
                    // v is the root of an SCC.
                    let mut scc_blocks = BTreeSet::new();
                    loop {
                        let w = stack.pop().unwrap();
                        on_stack[w.index()] = false;
                        scc_blocks.insert(w);
                        if w == v {
                            break;
                        }
                    }
                    let scc_idx = sccs.len();
                    for &b in &scc_blocks {
                        scc_of[b.index()] = scc_idx;
                    }
                    sccs.push(Scc { blocks: scc_blocks });
                }
                call_stack.pop();
                // Update parent's lowlink.
                if let Some((parent, _, _)) = call_stack.last() {
                    let pl = lowlinks[parent.index()];
                    let vl = lowlinks[v.index()];
                    lowlinks[parent.index()] = pl.min(vl);
                }
            }
        }
    }

    SccResult { sccs, scc_of }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::edge::EdgeKind;
    use crate::test_util::ff;

    #[test]
    fn linear_cfg_has_trivial_sccs() {
        let mut cfg = Cfg::new();
        let b = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("a"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.add_edge(cfg.entry(), b, EdgeKind::Fallthrough);

        let result = tarjan_scc(&cfg);
        assert_eq!(result.num_sccs(), 2);
        assert!(result.is_dag(&cfg));
    }

    #[test]
    fn self_loop_is_not_dag() {
        let mut cfg = Cfg::new();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("loop"));
        cfg.add_edge(cfg.entry(), cfg.entry(), EdgeKind::Jump);

        let result = tarjan_scc(&cfg);
        assert_eq!(result.num_sccs(), 1);
        assert!(!result.is_dag(&cfg));
    }

    #[test]
    fn two_node_cycle() {
        let mut cfg = Cfg::new();
        let b = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("a"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.add_edge(cfg.entry(), b, EdgeKind::Fallthrough);
        cfg.add_edge(b, cfg.entry(), EdgeKind::Jump);

        let result = tarjan_scc(&cfg);
        assert_eq!(result.num_sccs(), 1);
        assert!(result.sccs[0].contains(cfg.entry()));
        assert!(result.sccs[0].contains(b));
        assert_eq!(result.scc_index(cfg.entry()), result.scc_index(b));
    }
}
