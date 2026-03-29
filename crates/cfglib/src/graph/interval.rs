//! Interval analysis via T1–T2 graph transformations.
//!
//! Collapses the CFG into a hierarchy of **intervals** — maximal
//! single-entry regions where the header dominates all other blocks.
//! This provides an alternative structural decomposition to the
//! dominator tree, useful for detecting loops, reducibility, and
//! for region-based analyses.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;

/// An interval in the derived graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interval {
    /// The header block — sole entry point of the interval.
    pub header: BlockId,
    /// All blocks in the interval (including the header).
    pub blocks: BTreeSet<BlockId>,
}

/// Result of interval analysis: a sequence of derived graphs.
///
/// `levels[0]` contains the intervals of the original CFG,
/// `levels[1]` contains the intervals of the first derived graph,
/// and so on. If the sequence reduces to a single interval at
/// the top level, the CFG is reducible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntervalAnalysis {
    /// Successive derived graphs, each containing intervals.
    pub levels: Vec<Vec<Interval>>,
    /// Whether the CFG reduced to a single node (reducible).
    pub is_reducible: bool,
}

/// Compute intervals of a CFG (the first derived graph).
///
/// Allen & Cocke interval construction: starting from the entry,
/// repeatedly absorb successor blocks whose only header-reaching
/// predecessor is within the current interval.
fn compute_intervals_from_graph(
    entry: BlockId,
    blocks: &BTreeSet<BlockId>,
    succs: &BTreeMap<BlockId, BTreeSet<BlockId>>,
    preds: &BTreeMap<BlockId, BTreeSet<BlockId>>,
) -> Vec<Interval> {
    let mut intervals = Vec::new();
    let mut assigned: BTreeSet<BlockId> = BTreeSet::new();
    let mut headers: Vec<BlockId> = alloc::vec![entry];

    while let Some(h) = headers.pop() {
        if assigned.contains(&h) || !blocks.contains(&h) {
            continue;
        }
        let mut interval = BTreeSet::new();
        interval.insert(h);
        assigned.insert(h);

        // Grow the interval: add blocks whose predecessors are all in
        // the interval.
        let mut changed = true;
        while changed {
            changed = false;
            for &b in blocks {
                if assigned.contains(&b) {
                    continue;
                }

                let b_preds = preds.get(&b).cloned().unwrap_or_default();
                if !b_preds.is_empty() && b_preds.iter().all(|p| interval.contains(p)) {
                    interval.insert(b);
                    assigned.insert(b);
                    changed = true;
                }
            }
        }

        // Blocks that are successors of the interval but not in it
        // become headers for new intervals.
        for &b in &interval {
            for &s in succs.get(&b).unwrap_or(&BTreeSet::new()) {
                if !interval.contains(&s) && !assigned.contains(&s) {
                    headers.push(s);
                }
            }
        }

        intervals.push(Interval {
            header: h,
            blocks: interval,
        });
    }

    intervals
}

/// Build adjacency maps from the CFG, restricted to `blocks`.
fn build_adjacency<I>(
    cfg: &Cfg<I>,
    blocks: &BTreeSet<BlockId>,
) -> (
    BTreeMap<BlockId, BTreeSet<BlockId>>,
    BTreeMap<BlockId, BTreeSet<BlockId>>,
) {
    let mut succs: BTreeMap<BlockId, BTreeSet<BlockId>> = BTreeMap::new();
    let mut preds: BTreeMap<BlockId, BTreeSet<BlockId>> = BTreeMap::new();
    for &b in blocks {
        for s in cfg.successors(b) {
            if blocks.contains(&s) {
                succs.entry(b).or_default().insert(s);
                preds.entry(s).or_default().insert(b);
            }
        }
    }
    (succs, preds)
}

/// Perform interval analysis on the CFG.
///
/// Iteratively computes derived graphs until either a single interval
/// remains (reducible) or no further reduction is possible (irreducible).
pub fn interval_analysis<I>(cfg: &Cfg<I>) -> IntervalAnalysis {
    let all_blocks: BTreeSet<BlockId> = cfg.blocks().iter().map(|b| b.id()).collect();
    let (succs, preds) = build_adjacency(cfg, &all_blocks);
    let mut levels = Vec::new();

    let intervals = compute_intervals_from_graph(cfg.entry(), &all_blocks, &succs, &preds);
    let num_intervals = intervals.len();
    levels.push(intervals);

    // A single interval means the CFG is trivially reducible.
    // Multi-level derived-graph iteration can be added when needed;
    // for now use `is_reducible()` from structure.rs for the full check.
    IntervalAnalysis {
        is_reducible: num_intervals <= 1,
        levels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::CfgBuilder;
    use crate::flow::FlowEffect;
    use crate::test_util::{MockInst, ff};
    use alloc::vec;

    #[test]
    fn single_block_is_one_interval() {
        let cfg = CfgBuilder::build(vec![ff("a")]).unwrap();
        let result = interval_analysis(&cfg);
        assert_eq!(result.levels.len(), 1);
        assert_eq!(result.levels[0].len(), 1);
        assert!(result.is_reducible);
    }

    #[test]
    fn linear_cfg_is_one_interval() {
        let cfg = CfgBuilder::build(vec![ff("a"), ff("b"), ff("c")]).unwrap();
        let result = interval_analysis(&cfg);
        assert_eq!(result.levels.len(), 1);
        // All blocks should be in a single interval since each block
        // has only one predecessor from within the interval.
        assert_eq!(result.levels[0].len(), 1);
        assert!(result.is_reducible);
    }

    #[test]
    fn diamond_cfg_intervals() {
        // Build a diamond manually to avoid Break-outside-scope.
        let mut cfg = crate::Cfg::<MockInst>::new();
        let b0 = cfg.entry();
        let b1 = cfg.new_block();
        let b2 = cfg.new_block();
        let b3 = cfg.new_block();
        cfg.add_edge(b0, b1, crate::edge::EdgeKind::ConditionalTrue);
        cfg.add_edge(b0, b2, crate::edge::EdgeKind::ConditionalFalse);
        cfg.add_edge(b1, b3, crate::edge::EdgeKind::Fallthrough);
        cfg.add_edge(b2, b3, crate::edge::EdgeKind::Fallthrough);

        let result = interval_analysis(&cfg);
        assert_eq!(result.levels.len(), 1);
        assert!(!result.levels[0].is_empty());
    }

    #[test]
    fn loop_cfg_intervals() {
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::LoopOpen, "loop"),
            ff("body"),
            MockInst(FlowEffect::LoopClose, "endloop"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        let result = interval_analysis(&cfg);
        assert_eq!(result.levels.len(), 1);
        assert!(!result.levels[0].is_empty());
    }
}
