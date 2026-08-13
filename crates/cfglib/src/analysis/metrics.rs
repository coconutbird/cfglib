//! Graph metrics — cyclomatic complexity, nesting depth, code density.
//!
//! Provides quantitative measurements of graph complexity that are useful
//! for program analysis, code quality assessment, and heuristic-driven
//! transformation or decompilation. [`graph_metrics`] serves any rooted
//! graph view; [`cfg_metrics`] adds instruction-level measurements for a
//! [`Cfg`].

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

use crate::cfg::Cfg;
use crate::graph::dominator::DominatorTree;
use crate::graph::structure::{detect_loops, detect_loops_tagged};
use crate::graph::view::{DenseNodeId, RootedGraphView};

/// Topology-level metrics for any rooted graph view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphMetrics {
    /// Number of nodes.
    pub node_count: usize,
    /// Number of edges (counted through forward adjacency).
    pub edge_count: usize,
    /// `McCabe` cyclomatic complexity: `E - N + 2P` (P=1 for single function).
    pub cyclomatic_complexity: usize,
    /// Maximum loop nesting depth (0 = no loops), from dominance-based
    /// loop detection.
    pub max_nesting_depth: usize,
    /// Number of reachable nodes from the root.
    pub reachable_node_count: usize,
    /// Number of unreachable nodes.
    pub unreachable_node_count: usize,
    /// Number of exit nodes (nodes with no successors).
    pub exit_count: usize,
}

/// Instruction-aware metrics for a [`Cfg`].
#[derive(Debug, Clone, PartialEq)]
pub struct CfgMetrics {
    /// Topology metrics (loop depth honours [`EdgeKind::Back`] tags via
    /// [`detect_loops_tagged`]).
    ///
    /// [`EdgeKind::Back`]: crate::EdgeKind::Back
    pub graph: GraphMetrics,
    /// Total instruction count across all blocks.
    pub instruction_count: usize,
    /// Average instructions per block (0.0 for empty CFG).
    pub avg_instructions_per_block: f64,
}

/// Compute topology metrics for any rooted graph view.
#[must_use]
pub fn graph_metrics<G: RootedGraphView>(graph: &G) -> GraphMetrics {
    let n = graph.node_count();

    // Reachability from the root.
    let mut visited = vec![false; n];
    let mut stack = vec![graph.root()];
    visited[graph.root().index()] = true;
    let mut reachable_count = 1;
    while let Some(node) = stack.pop() {
        for successor in graph.successors(node) {
            if !visited[successor.index()] {
                visited[successor.index()] = true;
                reachable_count += 1;
                stack.push(successor);
            }
        }
    }

    let edge_count: usize = graph
        .node_ids()
        .map(|node| graph.successors(node).count())
        .sum();

    // Cyclomatic complexity: E - N + 2P (P=1).
    let cyclomatic = if edge_count >= reachable_count {
        edge_count - reachable_count + 2
    } else {
        1
    };

    // Nesting depth from dominance-based loop detection.
    let max_nesting = if n > 1 {
        let dom = DominatorTree::compute(graph);
        let loops = detect_loops(graph, &dom);
        loops.iter().map(|lp| lp.depth).max().unwrap_or(0)
    } else {
        0
    };

    let exit_count = graph
        .node_ids()
        .filter(|&node| graph.successors(node).next().is_none())
        .count();

    GraphMetrics {
        node_count: n,
        edge_count,
        cyclomatic_complexity: cyclomatic,
        max_nesting_depth: max_nesting,
        reachable_node_count: reachable_count,
        unreachable_node_count: n.saturating_sub(reachable_count),
        exit_count,
    }
}

/// Compute comprehensive metrics for a CFG.
///
/// # Examples
///
/// ```
/// use cfglib::{Cfg, EdgeKind, cfg_metrics};
///
/// let mut cfg = Cfg::<u32>::new();
/// let b0 = cfg.entry();
/// let b1 = cfg.new_block();
/// let b2 = cfg.new_block();
/// cfg.add_edge(b0, b1, EdgeKind::ConditionalTrue);
/// cfg.add_edge(b0, b2, EdgeKind::ConditionalFalse);
///
/// let m = cfg_metrics(&cfg);
/// assert_eq!(m.graph.node_count, 3);
/// assert_eq!(m.graph.edge_count, 2);
/// assert_eq!(m.graph.exit_count, 2);
/// ```
#[must_use]
pub fn cfg_metrics<I>(cfg: &Cfg<I>) -> CfgMetrics {
    let mut graph = graph_metrics(cfg);

    // Honour explicit back-edge tags for loop depth on CFGs.
    if cfg.num_blocks() > 1 {
        let dom = DominatorTree::compute(cfg);
        let loops = detect_loops_tagged(cfg, &dom);
        graph.max_nesting_depth = loops.iter().map(|lp| lp.depth).max().unwrap_or(0);
    }

    let n = cfg.num_blocks();
    let instruction_count: usize = cfg.blocks().iter().map(|b| b.instructions().len()).sum();
    let avg_instr = if n > 0 {
        crate::usize_to_f64(instruction_count) / crate::usize_to_f64(n)
    } else {
        0.0
    };

    CfgMetrics {
        graph,
        instruction_count,
        avg_instructions_per_block: avg_instr,
    }
}

/// Compute the nesting depth of each node.
///
/// Returns a vector indexed by dense node index, where each value is the
/// number of loops containing that node (dominance-based detection).
#[must_use]
pub fn block_nesting_depths<G: RootedGraphView>(graph: &G) -> Vec<usize> {
    let n = graph.node_count();
    let dom = DominatorTree::compute(graph);
    let loops = detect_loops(graph, &dom);
    let mut depths = vec![0usize; n];

    for lp in &loops {
        for &node in &lp.body {
            if node.index() < n {
                depths[node.index()] += 1;
            }
        }
    }

    depths
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::EdgeKind;
    use crate::test_util::ff;

    #[test]
    fn single_block_metrics() {
        let mut cfg = Cfg::new();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("a"));

        let m = cfg_metrics(&cfg);
        assert_eq!(m.graph.node_count, 1);
        assert_eq!(m.instruction_count, 1);
        assert_eq!(m.graph.cyclomatic_complexity, 1);
        assert_eq!(m.graph.max_nesting_depth, 0);
        assert_eq!(m.graph.reachable_node_count, 1);
        assert_eq!(m.graph.unreachable_node_count, 0);
    }

    #[test]
    fn diamond_cyclomatic_complexity() {
        let mut cfg = Cfg::new();
        let a = cfg.new_block();
        let b = cfg.new_block();
        let merge = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("e"));
        cfg.add_edge(cfg.entry(), a, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), b, EdgeKind::ConditionalFalse);
        cfg.add_edge(a, merge, EdgeKind::Fallthrough);
        cfg.add_edge(b, merge, EdgeKind::Fallthrough);

        let m = cfg_metrics(&cfg);
        // E=4, N=4, CC = 4-4+2 = 2
        assert_eq!(m.graph.cyclomatic_complexity, 2);
    }

    #[test]
    fn nesting_depth_with_loop() {
        let mut cfg = Cfg::new();
        let header = cfg.new_block();
        let body = cfg.new_block();
        let exit = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("e"));
        cfg.add_edge(cfg.entry(), header, EdgeKind::Fallthrough);
        cfg.add_edge(header, body, EdgeKind::ConditionalTrue);
        cfg.add_edge(header, exit, EdgeKind::ConditionalFalse);
        cfg.add_edge(body, header, EdgeKind::Back);

        let depths = block_nesting_depths(&cfg);
        assert!(depths[header.index()] >= 1);
        assert!(depths[body.index()] >= 1);
        assert_eq!(depths[exit.index()], 0);
    }

    #[test]
    fn graph_metrics_on_consumer_view() {
        use crate::graph::directed::DirectedGraph;
        use crate::graph::view::Rooted;

        let mut graph = DirectedGraph::new();
        let a = graph.add_node("a");
        let b = graph.add_node("b");
        let c = graph.add_node("c");
        graph.add_edge(a, b, ());
        graph.add_edge(b, a, ());
        assert_eq!(graph.node(c), &"c");

        let m = graph_metrics(&Rooted::new(&graph, a));
        assert_eq!(m.node_count, 3);
        assert_eq!(m.reachable_node_count, 2);
        assert_eq!(m.unreachable_node_count, 1);
        assert_eq!(m.max_nesting_depth, 0, "self-cycle a<->b is depth 0");
        assert_eq!(m.exit_count, 1, "c has no successors");
    }
}
