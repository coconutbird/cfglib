//! Graph metrics — cyclomatic complexity, nesting depth, code density.
//!
//! Provides quantitative measurements of CFG complexity that are useful
//! for binary analysis, code quality assessment, and heuristic-driven
//! decompilation.

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

use crate::cfg::Cfg;
use crate::graph::dominator::DominatorTree;
use crate::graph::structure::detect_loops;

/// Collected metrics for a CFG.
#[derive(Debug, Clone, PartialEq)]
pub struct CfgMetrics {
    /// Number of basic blocks.
    pub block_count: usize,
    /// Number of edges (live, non-removed).
    pub edge_count: usize,
    /// Total instruction count across all blocks.
    pub instruction_count: usize,
    /// McCabe cyclomatic complexity: `E - N + 2P` (P=1 for single function).
    pub cyclomatic_complexity: usize,
    /// Maximum loop nesting depth (0 = no loops).
    pub max_nesting_depth: usize,
    /// Average instructions per block (0.0 for empty CFG).
    pub avg_instructions_per_block: f64,
    /// Number of reachable blocks from entry.
    pub reachable_block_count: usize,
    /// Number of unreachable blocks.
    pub unreachable_block_count: usize,
    /// Number of exit blocks (blocks with no successors).
    pub exit_count: usize,
}

/// Compute comprehensive metrics for a CFG.
pub fn cfg_metrics<I>(cfg: &Cfg<I>) -> CfgMetrics {
    let n = cfg.num_blocks();
    let reachable = cfg.dfs_preorder();
    let reachable_count = reachable.len();

    // Count live edges.
    let edge_count = cfg.edges().count();

    // Instruction count.
    let instruction_count: usize = cfg.blocks().iter().map(|b| b.instructions().len()).sum();

    // Cyclomatic complexity: E - N + 2P (P=1).
    let cyclomatic = if edge_count >= reachable_count {
        edge_count - reachable_count + 2
    } else {
        1
    };

    // Nesting depth from loop detection.
    let max_nesting = if n > 1 {
        let dom = DominatorTree::compute(cfg);
        let loops = detect_loops(cfg, &dom);
        loops.iter().map(|lp| lp.depth).max().unwrap_or(0)
    } else {
        0
    };

    let avg_instr = if n > 0 {
        instruction_count as f64 / n as f64
    } else {
        0.0
    };

    let exit_count = cfg.exit_blocks().count();

    CfgMetrics {
        block_count: n,
        edge_count,
        instruction_count,
        cyclomatic_complexity: cyclomatic,
        max_nesting_depth: max_nesting,
        avg_instructions_per_block: avg_instr,
        reachable_block_count: reachable_count,
        unreachable_block_count: n.saturating_sub(reachable_count),
        exit_count,
    }
}

/// Compute the nesting depth of each block.
///
/// Returns a vector indexed by block index, where each value is the
/// number of loops containing that block.
pub fn block_nesting_depths<I>(cfg: &Cfg<I>) -> Vec<usize> {
    let n = cfg.num_blocks();
    let dom = DominatorTree::compute(cfg);
    let loops = detect_loops(cfg, &dom);
    let mut depths = vec![0usize; n];

    for lp in &loops {
        for &bid in &lp.body {
            if bid.index() < n {
                depths[bid.index()] += 1;
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
        assert_eq!(m.block_count, 1);
        assert_eq!(m.instruction_count, 1);
        assert_eq!(m.cyclomatic_complexity, 1);
        assert_eq!(m.max_nesting_depth, 0);
        assert_eq!(m.reachable_block_count, 1);
        assert_eq!(m.unreachable_block_count, 0);
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
        assert_eq!(m.cyclomatic_complexity, 2);
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
}
