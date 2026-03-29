//! Pattern matching — idiom recognition within the CFG.
//!
//! Identifies common structural patterns (if-then-else diamonds,
//! guarded returns, empty loops, etc.) for downstream consumers.

extern crate alloc;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::edge::EdgeKind;

/// A recognised structural pattern in the CFG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CfgPattern {
    /// Diamond: entry branches to two targets that reconverge at merge.
    Diamond {
        /// Block that branches.
        entry: BlockId,
        /// True-side block.
        then_block: BlockId,
        /// False-side block.
        else_block: BlockId,
        /// Merge point.
        merge: BlockId,
    },
    /// A single-entry, single-exit linear chain of blocks.
    Chain {
        /// Ordered list of blocks in the chain.
        blocks: Vec<BlockId>,
    },
    /// An empty block (no instructions) acting as a trampoline.
    EmptyTrampoline {
        /// The empty block.
        block: BlockId,
    },
    /// A self-loop (block has a back-edge to itself).
    SelfLoop {
        /// The looping block.
        block: BlockId,
    },
}

/// Scan a CFG for recognised patterns.
pub fn detect_patterns<I>(cfg: &Cfg<I>) -> Vec<CfgPattern> {
    let mut patterns = Vec::new();

    for block in cfg.blocks() {
        let bid = block.id();
        let succs: Vec<BlockId> = cfg.successors(bid).collect();

        // Self-loop detection.
        if succs.contains(&bid) {
            patterns.push(CfgPattern::SelfLoop { block: bid });
        }

        // Empty trampoline: no instructions, exactly one successor.
        if block.instructions().is_empty() && succs.len() == 1 && bid != cfg.entry() {
            patterns.push(CfgPattern::EmptyTrampoline { block: bid });
        }

        // Diamond detection: two successors that share a single successor.
        if succs.len() == 2 {
            let (a, b) = (succs[0], succs[1]);
            let a_succs: Vec<BlockId> = cfg.successors(a).collect();
            let b_succs: Vec<BlockId> = cfg.successors(b).collect();
            if a_succs.len() == 1 && b_succs.len() == 1 && a_succs[0] == b_succs[0] {
                // Determine which is then/else by edge kind.
                let edges: Vec<_> = cfg.successor_edges(bid).to_vec();
                let (then_b, else_b) = if edges.iter().any(|&eid| {
                    cfg.edge(eid).target() == a && cfg.edge(eid).kind() == EdgeKind::ConditionalTrue
                }) {
                    (a, b)
                } else {
                    (b, a)
                };
                patterns.push(CfgPattern::Diamond {
                    entry: bid,
                    then_block: then_b,
                    else_block: else_b,
                    merge: a_succs[0],
                });
            }
        }
    }

    // Chain detection: sequences of single-pred, single-succ blocks.
    let mut visited = alloc::collections::BTreeSet::new();
    for block in cfg.blocks() {
        let bid = block.id();
        if visited.contains(&bid) {
            continue;
        }
        let preds: Vec<BlockId> = cfg.predecessors(bid).collect();
        if preds.len() != 1 {
            continue;
        }
        let succs: Vec<BlockId> = cfg.successors(bid).collect();
        if succs.len() != 1 {
            continue;
        }

        // Walk backward to find chain start (skip self-loops).
        let mut start = bid;
        loop {
            let ps: Vec<BlockId> = cfg.predecessors(start).collect();
            if ps.len() != 1 || ps[0] == start {
                break;
            }
            let ss: Vec<BlockId> = cfg.successors(ps[0]).collect();
            if ss.len() != 1 {
                break;
            }
            start = ps[0];
        }
        // Walk forward to collect the chain.
        let mut chain = alloc::vec![start];
        visited.insert(start);
        let mut cur = start;
        loop {
            let ss: Vec<BlockId> = cfg.successors(cur).collect();
            if ss.len() != 1 {
                break;
            }
            let next = ss[0];
            if next == cur || visited.contains(&next) {
                break;
            }
            let ps: Vec<BlockId> = cfg.predecessors(next).collect();
            if ps.len() != 1 {
                break;
            }
            chain.push(next);
            visited.insert(next);
            cur = next;
        }
        if chain.len() >= 2 {
            patterns.push(CfgPattern::Chain { blocks: chain });
        }
    }

    patterns
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::edge::EdgeKind;
    use crate::test_util::ff;

    #[test]
    fn detects_diamond() {
        let mut cfg = Cfg::new();
        let a = cfg.new_block();
        let b = cfg.new_block();
        let merge = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("br"));
        cfg.block_mut(a).instructions_vec_mut().push(ff("a"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.block_mut(merge).instructions_vec_mut().push(ff("m"));
        cfg.add_edge(cfg.entry(), a, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), b, EdgeKind::ConditionalFalse);
        cfg.add_edge(a, merge, EdgeKind::Fallthrough);
        cfg.add_edge(b, merge, EdgeKind::Fallthrough);
        let pats = detect_patterns(&cfg);
        assert!(pats.iter().any(|p| matches!(p, CfgPattern::Diamond { .. })));
    }

    #[test]
    fn detects_self_loop() {
        let mut cfg = Cfg::new();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("loop"));
        cfg.add_edge(cfg.entry(), cfg.entry(), EdgeKind::Back);
        let pats = detect_patterns(&cfg);
        assert!(
            pats.iter()
                .any(|p| matches!(p, CfgPattern::SelfLoop { .. }))
        );
    }
}
