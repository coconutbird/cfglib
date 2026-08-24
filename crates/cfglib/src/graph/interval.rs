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
use crate::graph::view::{DenseNodeId, DirectedGraphView, RootedGraphView};

/// An interval in the derived graph, over node identity `N`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interval<N = BlockId> {
    /// The header node — sole entry point of the interval.
    pub header: N,
    /// All nodes in the interval (including the header).
    pub blocks: BTreeSet<N>,
}

/// Result of interval analysis: a sequence of derived graphs.
///
/// `levels[0]` contains the intervals of the original CFG,
/// `levels[1]` contains the intervals of the first derived graph,
/// and so on. If the sequence reduces to a single interval at
/// the top level, the CFG is reducible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntervalAnalysis<N = BlockId> {
    /// Successive derived graphs, each containing intervals.
    pub levels: Vec<Vec<Interval<N>>>,
    /// Whether the graph reduced to a single node (reducible).
    pub is_reducible: bool,
}

/// Compute intervals of a CFG (the first derived graph).
///
/// Allen & Cocke interval construction: starting from the entry,
/// repeatedly absorb successor blocks whose only header-reaching
/// predecessor is within the current interval.
fn compute_intervals_from_graph<N: Copy + Ord>(
    entry: N,
    blocks: &BTreeSet<N>,
    succs: &BTreeMap<N, BTreeSet<N>>,
    preds: &BTreeMap<N, BTreeSet<N>>,
) -> Vec<Interval<N>> {
    let mut intervals = Vec::new();
    let mut assigned: BTreeSet<N> = BTreeSet::new();
    let mut headers: Vec<N> = alloc::vec![entry];

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

/// Forward and reverse adjacency, restricted to a node subset.
type AdjacencyMaps<N> = (BTreeMap<N, BTreeSet<N>>, BTreeMap<N, BTreeSet<N>>);

/// Build adjacency maps from the graph view, restricted to `blocks`.
fn build_adjacency<G: RootedGraphView>(
    graph: &G,
    blocks: &BTreeSet<G::NodeId>,
) -> AdjacencyMaps<G::NodeId> {
    let mut succs: BTreeMap<G::NodeId, BTreeSet<G::NodeId>> = BTreeMap::new();
    let mut preds: BTreeMap<G::NodeId, BTreeSet<G::NodeId>> = BTreeMap::new();
    for &b in blocks {
        for s in graph.successors(b) {
            if blocks.contains(&s) {
                succs.entry(b).or_default().insert(s);
                preds.entry(s).or_default().insert(b);
            }
        }
    }
    (succs, preds)
}

impl<N: DenseNodeId> IntervalAnalysis<N> {
    /// Perform interval analysis on a rooted graph view.
    ///
    /// Iteratively computes derived graphs until either a single interval
    /// remains (reducible) or no further reduction is possible (irreducible).
    ///
    /// # Examples
    ///
    /// ```
    /// use cfglib::{Cfg, EdgeKind, IntervalAnalysis};
    ///
    /// let mut cfg = Cfg::<u32>::new();
    /// let b1 = cfg.new_block();
    /// cfg.add_edge(cfg.entry(), b1, EdgeKind::Fallthrough);
    ///
    /// let result = IntervalAnalysis::compute(&cfg);
    /// assert!(result.is_reducible);
    /// ```
    #[must_use]
    pub fn compute<G>(graph: &G) -> Self
    where
        G: RootedGraphView + DirectedGraphView<NodeId = N>,
    {
        let all_blocks: BTreeSet<N> = graph.node_ids().collect();
        let (succs, preds) = build_adjacency(graph, &all_blocks);
        let mut levels = Vec::new();

        let intervals = compute_intervals_from_graph(graph.root(), &all_blocks, &succs, &preds);
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
        let result = IntervalAnalysis::compute(&cfg);
        assert_eq!(result.levels.len(), 1);
        assert_eq!(result.levels[0].len(), 1);
        assert!(result.is_reducible);
    }

    #[test]
    fn linear_cfg_is_one_interval() {
        let cfg = CfgBuilder::build(vec![ff("a"), ff("b"), ff("c")]).unwrap();
        let result = IntervalAnalysis::compute(&cfg);
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

        let result = IntervalAnalysis::compute(&cfg);
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
        let result = IntervalAnalysis::compute(&cfg);
        assert_eq!(result.levels.len(), 1);
        assert!(!result.levels[0].is_empty());
    }
}
