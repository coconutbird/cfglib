//! SSA (Static Single Assignment) construction.
//!
//! Computes dominance frontiers and inserts φ-functions at merge points
//! using the classic Cytron et al. algorithm. This module provides the
//! structural metadata; the actual renaming of variables is left to the
//! consumer, since it depends on the instruction representation.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::dataflow::{InstrInfo, Location};
use crate::graph::dominator::DominatorTree;

/// The dominance frontier of every block.
#[derive(Debug, Clone)]
pub struct DominanceFrontiers {
    /// `df[b]` = set of blocks in the dominance frontier of `b`.
    frontiers: Vec<BTreeSet<BlockId>>,
}

impl DominanceFrontiers {
    /// Compute dominance frontiers using the algorithm from
    /// Cooper, Harvey & Kennedy (2001) — "A Simple, Fast Dominance Algorithm".
    pub fn compute<I>(cfg: &Cfg<I>, dom: &DominatorTree) -> Self {
        let n = cfg.num_blocks();
        let mut frontiers = vec![BTreeSet::new(); n];

        for b in cfg.blocks() {
            let preds: Vec<BlockId> = cfg.predecessors(b.id()).collect();
            if preds.len() < 2 {
                continue; // only merge points contribute to DF
            }
            for &p in &preds {
                let mut runner = p;
                while runner != dom.idom(b.id()).unwrap_or(b.id()) {
                    frontiers[runner.index()].insert(b.id());
                    match dom.idom(runner) {
                        Some(next) => runner = next,
                        None => break,
                    }
                }
            }
        }

        Self { frontiers }
    }

    /// The dominance frontier set for `block`.
    pub fn frontier(&self, block: BlockId) -> &BTreeSet<BlockId> {
        &self.frontiers[block.index()]
    }
}

/// A φ-function placed at the start of a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhiNode {
    /// The location (variable) this φ merges.
    pub location: Location,
    /// One operand per predecessor, in predecessor order.
    /// Initially all operands are the same as `location`;
    /// the consumer renames them during the renaming pass.
    pub operands: Vec<(BlockId, Location)>,
}

/// Result of φ-insertion: which blocks get which φ-functions.
#[derive(Debug, Clone)]
pub struct PhiMap {
    /// `phis[block_index]` = list of φ-functions at that block.
    phis: Vec<Vec<PhiNode>>,
}

impl PhiMap {
    /// φ-functions at the given block.
    pub fn phis_at(&self, block: BlockId) -> &[PhiNode] {
        &self.phis[block.index()]
    }

    /// Total number of φ-functions across all blocks.
    pub fn total_phis(&self) -> usize {
        self.phis.iter().map(|v| v.len()).sum()
    }

    /// Iterate over all (block, phi) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (BlockId, &PhiNode)> {
        self.phis
            .iter()
            .enumerate()
            .flat_map(|(i, phis)| phis.iter().map(move |phi| (BlockId(i as u32), phi)))
    }
}

/// Insert φ-functions for all locations defined in the CFG.
///
/// Uses the iterated dominance frontier (IDF) algorithm:
/// for each location `v`, find all blocks that define `v`, then
/// iteratively add φ-functions at dominance frontier blocks until
/// convergence.
pub fn insert_phis<I: InstrInfo>(cfg: &Cfg<I>, dom: &DominatorTree) -> PhiMap {
    let n = cfg.num_blocks();
    let df = DominanceFrontiers::compute(cfg, dom);

    // Collect def-sites per location.
    let mut def_sites: BTreeMap<Location, BTreeSet<BlockId>> = BTreeMap::new();
    for block in cfg.blocks() {
        for inst in block.instructions() {
            for &loc in inst.defs() {
                def_sites.entry(loc).or_default().insert(block.id());
            }
        }
    }

    let mut phis: Vec<Vec<PhiNode>> = vec![Vec::new(); n];

    for (&loc, defs) in &def_sites {
        // IDF computation via worklist.
        let mut worklist: Vec<BlockId> = defs.iter().copied().collect();
        let mut has_phi: BTreeSet<BlockId> = BTreeSet::new();
        let mut ever_on_worklist: BTreeSet<BlockId> = defs.clone();

        while let Some(x) = worklist.pop() {
            for &y in df.frontier(x) {
                if has_phi.insert(y) {
                    // Insert a φ at y.
                    let preds: Vec<BlockId> = cfg.predecessors(y).collect();
                    phis[y.index()].push(PhiNode {
                        location: loc,
                        operands: preds.into_iter().map(|p| (p, loc)).collect(),
                    });
                    if ever_on_worklist.insert(y) {
                        worklist.push(y);
                    }
                }
            }
        }
    }

    PhiMap { phis }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::CfgBuilder;
    use crate::dataflow::Location;
    use crate::graph::dominator::DominatorTree;
    use crate::test_util::{DfInst, df_def, df_use};
    use alloc::vec;

    #[test]
    fn no_phis_in_linear_cfg() {
        let cfg = CfgBuilder::build(vec![df_def("def r0", 0), df_use("use r0", 0)]).unwrap();
        let dom = DominatorTree::compute(&cfg);
        let phis = insert_phis(&cfg, &dom);
        assert_eq!(phis.total_phis(), 0);
    }

    #[test]
    fn phi_at_merge_point() {
        use crate::edge::EdgeKind;
        // Build a diamond manually: both branches define r0, merge needs a phi.
        let mut cfg = crate::Cfg::<DfInst>::new();
        let b0 = cfg.entry();
        let b1 = cfg.new_block();
        let b2 = cfg.new_block();
        let b3 = cfg.new_block();
        cfg.add_edge(b0, b1, EdgeKind::ConditionalTrue);
        cfg.add_edge(b0, b2, EdgeKind::ConditionalFalse);
        cfg.add_edge(b1, b3, EdgeKind::Fallthrough);
        cfg.add_edge(b2, b3, EdgeKind::Fallthrough);
        // b1 defines r0
        cfg.block_mut(b1).push(df_def("def r0 then", 0));
        // b2 defines r0
        cfg.block_mut(b2).push(df_def("def r0 else", 0));
        // b3 uses r0
        cfg.block_mut(b3).push(df_use("use r0", 0));

        let dom = DominatorTree::compute(&cfg);
        let phis = insert_phis(&cfg, &dom);
        // There should be at least one phi for location 0 at the merge block (b3).
        assert!(
            phis.total_phis() >= 1,
            "expected phi at merge point, got {}",
            phis.total_phis()
        );
        let has_loc0_phi = phis.iter().any(|(_, phi)| phi.location == Location(0));
        assert!(has_loc0_phi, "expected phi for location 0");
    }

    #[test]
    fn dominance_frontiers_linear() {
        let cfg = CfgBuilder::build(vec![df_def("a", 0), df_def("b", 1)]).unwrap();
        let dom = DominatorTree::compute(&cfg);
        let df = DominanceFrontiers::compute(&cfg, &dom);
        // In a linear CFG, all dominance frontiers should be empty.
        for b in cfg.blocks() {
            assert!(df.frontier(b.id()).is_empty());
        }
    }
}
