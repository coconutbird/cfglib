//! Tail call detection.
//!
//! Identifies blocks that end with a call immediately followed by a
//! return (or where the call edge's [`CallSite`] is already marked
//! `is_tail_call`). These are candidates for tail call optimization.

extern crate alloc;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::edge::EdgeKind;
use crate::flow::{FlowControl, FlowEffect};

/// A detected tail call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailCall {
    /// The block containing the tail call.
    pub block: BlockId,
    /// Index of the call instruction within the block (if identifiable).
    pub inst_idx: Option<usize>,
    /// Whether this was explicitly marked via [`CallSite::is_tail_call`].
    pub explicit: bool,
}

/// Detect potential tail calls in a CFG.
///
/// A block is considered a tail call candidate if:
/// 1. It has a `Call` or `IndirectCall` outgoing edge whose `CallSite`
///    has `is_tail_call == true`, OR
/// 2. Its only successor is an exit block (return) and the block's
///    last instruction is a call.
///
/// Returns all detected tail call sites.
pub fn detect_tail_calls<I: FlowControl>(cfg: &Cfg<I>) -> Vec<TailCall> {
    let mut results = Vec::new();
    let exit_blocks: alloc::collections::BTreeSet<BlockId> =
        cfg.exit_blocks().into_iter().collect();

    for block in cfg.blocks() {
        let bid = block.id();

        // Check 1: explicit tail call markers on edges.
        for &eid in cfg.successor_edges(bid) {
            let edge = cfg.edge(eid);
            if matches!(edge.kind(), EdgeKind::Call | EdgeKind::IndirectCall)
                && let Some(cs) = edge.call_site()
                && cs.is_tail_call
            {
                results.push(TailCall {
                    block: bid,
                    inst_idx: None,
                    explicit: true,
                });
            }
        }

        // Check 2: heuristic — block calls then immediately returns.
        let succs: Vec<BlockId> = cfg.successors(bid).collect();
        if succs.len() == 1 && exit_blocks.contains(&succs[0]) {
            // Check if last instruction is a call.
            if let Some(last) = block.instructions().last() {
                let effect = last.flow_effect();
                if matches!(effect, FlowEffect::Call | FlowEffect::ConditionalCall) {
                    // Check not already found via explicit marker.
                    if !results.iter().any(|tc| tc.block == bid) {
                        let idx = block.instructions().len().saturating_sub(1);
                        results.push(TailCall {
                            block: bid,
                            inst_idx: Some(idx),
                            explicit: false,
                        });
                    }
                }
            }
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::CallSite;
    use crate::test_util::ff;

    #[test]
    fn explicit_tail_call_detected() {
        let mut cfg = Cfg::new();
        let target = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("call"));

        let eid = cfg.add_edge(cfg.entry(), target, EdgeKind::Call);
        let mut cs = CallSite::named("foo");
        cs.is_tail_call = true;
        cfg.edge_mut(eid).set_call_site(Some(cs));

        let tails = detect_tail_calls(&cfg);
        assert_eq!(tails.len(), 1);
        assert!(tails[0].explicit);
    }

    #[test]
    fn no_tail_calls_in_simple_cfg() {
        let mut cfg = Cfg::new();
        let b = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("a"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.add_edge(cfg.entry(), b, EdgeKind::Fallthrough);

        let tails = detect_tail_calls(&cfg);
        assert!(tails.is_empty());
    }
}
