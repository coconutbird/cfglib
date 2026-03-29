//! Control Dependence Graph (CDG).
//!
//! Block B is **control-dependent** on block A if:
//! 1. There exists a path from A to B in the CFG, and
//! 2. B post-dominates every block on that path *except* A itself.
//!
//! Equivalently, A has multiple successors and B post-dominates one of
//! them but not A. This is computed using the post-dominator tree and
//! dominance frontiers on the reverse graph.

extern crate alloc;
use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::graph::dominator::DominatorTree;

/// The control dependence graph.
///
/// # Examples
///
/// ```
/// use cfglib::{Cfg, EdgeKind, DominatorTree, ControlDependenceGraph};
///
/// let mut cfg = Cfg::<u32>::new();
/// let b0 = cfg.entry();
/// let b1 = cfg.new_block();
/// let b2 = cfg.new_block();
/// let b3 = cfg.new_block();
/// cfg.add_edge(b0, b1, EdgeKind::ConditionalTrue);
/// cfg.add_edge(b0, b2, EdgeKind::ConditionalFalse);
/// cfg.add_edge(b1, b3, EdgeKind::Fallthrough);
/// cfg.add_edge(b2, b3, EdgeKind::Fallthrough);
///
/// let pdom = DominatorTree::compute_post(&cfg);
/// let cdg = ControlDependenceGraph::compute(&cfg, &pdom);
/// // b1 and b2 are control-dependent on b0's branch.
/// assert!(cdg.is_dependent(b1, b0));
/// assert!(cdg.is_dependent(b2, b0));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlDependenceGraph {
    /// `dependences[b]` = set of blocks that `b` is control-dependent on.
    /// i.e. "b executes conditionally because of a branch in these blocks".
    dependences: Vec<BTreeSet<BlockId>>,
    /// `dependents[a]` = set of blocks that are control-dependent on `a`.
    /// i.e. "a's branch decision controls whether these blocks execute".
    dependents: Vec<BTreeSet<BlockId>>,
}

impl ControlDependenceGraph {
    /// Compute the CDG from a CFG using its post-dominator tree.
    ///
    /// The algorithm walks every CFG edge (A → B). If B does not
    /// post-dominate A, then every block on the post-dominator tree
    /// path from B up to (but excluding) the immediate post-dominator
    /// of A is control-dependent on A.
    pub fn compute<I>(cfg: &Cfg<I>, pdom: &DominatorTree) -> Self {
        let n = cfg.num_blocks();
        let mut dependences: Vec<BTreeSet<BlockId>> = vec![BTreeSet::new(); n];
        let mut dependents: Vec<BTreeSet<BlockId>> = vec![BTreeSet::new(); n];

        for edge in cfg.edges() {
            // Skip ghost edges.
            if !cfg.successor_edges(edge.source()).contains(&edge.id()) {
                continue;
            }

            let a = edge.source();
            let b = edge.target();

            // If B post-dominates A, there's no control dependence.
            if pdom.dominates(b, a) {
                continue;
            }

            // Walk from B up the post-dominator tree to ipdom(A).
            let ipdom_a = pdom.idom(a);
            let mut runner = b;
            loop {
                // runner is control-dependent on a.
                dependences[runner.index()].insert(a);
                dependents[a.index()].insert(runner);

                match pdom.idom(runner) {
                    Some(next) if Some(next) != ipdom_a => runner = next,
                    _ => break,
                }
            }
        }

        Self {
            dependences,
            dependents,
        }
    }

    /// Blocks that `block` is control-dependent on.
    ///
    /// These are the branch points whose decisions control whether
    /// `block` executes.
    pub fn control_dependences(&self, block: BlockId) -> &BTreeSet<BlockId> {
        &self.dependences[block.index()]
    }

    /// Blocks that are control-dependent on `block`.
    ///
    /// These are blocks whose execution is controlled by the branch
    /// decision in `block`.
    pub fn control_dependents(&self, block: BlockId) -> &BTreeSet<BlockId> {
        &self.dependents[block.index()]
    }

    /// Whether `block` is control-dependent on `on`.
    pub fn is_dependent(&self, block: BlockId, on: BlockId) -> bool {
        self.dependences[block.index()].contains(&on)
    }

    /// Whether any block is control-dependent on `block` (i.e. it
    /// has a meaningful branch).
    pub fn has_dependents(&self, block: BlockId) -> bool {
        !self.dependents[block.index()].is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::edge::EdgeKind;
    use crate::graph::dominator::DominatorTree;
    use crate::test_util::ff;

    #[test]
    fn diamond_cdg() {
        let mut cfg = Cfg::new();
        let a = cfg.new_block();
        let b = cfg.new_block();
        let merge = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("entry"));
        cfg.block_mut(a).instructions_vec_mut().push(ff("a"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.block_mut(merge)
            .instructions_vec_mut()
            .push(ff("merge"));
        cfg.add_edge(cfg.entry(), a, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), b, EdgeKind::ConditionalFalse);
        cfg.add_edge(a, merge, EdgeKind::Fallthrough);
        cfg.add_edge(b, merge, EdgeKind::Fallthrough);

        let pdom = DominatorTree::compute_post(&cfg);
        let cdg = ControlDependenceGraph::compute(&cfg, &pdom);

        assert!(cdg.is_dependent(a, cfg.entry()));
        assert!(cdg.is_dependent(b, cfg.entry()));
        assert!(!cdg.is_dependent(merge, cfg.entry()));
        assert!(cdg.has_dependents(cfg.entry()));
        assert!(cdg.control_dependents(cfg.entry()).contains(&a));
        assert!(cdg.control_dependents(cfg.entry()).contains(&b));
    }

    #[test]
    fn linear_cfg_has_no_control_dependence() {
        let mut cfg = Cfg::new();
        let b = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("a"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.add_edge(cfg.entry(), b, EdgeKind::Fallthrough);

        let pdom = DominatorTree::compute_post(&cfg);
        let cdg = ControlDependenceGraph::compute(&cfg, &pdom);

        assert!(!cdg.has_dependents(cfg.entry()));
        assert!(cdg.control_dependences(b).is_empty());
    }
}
