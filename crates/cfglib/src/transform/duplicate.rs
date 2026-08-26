//! Structuring-driven tail duplication.
//!
//! Short-circuit conditions, shared side-exits, and other cross-joined
//! tails defeat tree structuring: the shared block belongs exclusively to
//! no conditional arm, so [`lift_with_report`](crate::lift_with_report)
//! emits goto residue and consumers fall back to unstructured rendering.
//! Duplicating such a tail per extra predecessor is a pure control-flow
//! unfolding — every execution path runs the same instructions in the
//! same order — after which each copy sits exclusively inside one arm and
//! the graph structures as plain nested conditionals.

extern crate alloc;

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::graph::dominator::DominatorTree;
use crate::ir::ast::GotoReason;
use crate::region::HandlerBody;

/// At most this many blocks are materialized per call.
const BLOCK_BUDGET: usize = 16;
/// Only tails at most this many instructions long duplicate.
const INSTRUCTION_LIMIT: usize = 8;
/// Re-structuring rounds; each round can expose new shared tails.
const ROUNDS: usize = 4;

/// Duplicates small shared tails that block tree structuring, returning
/// the number of blocks materialized.
///
/// Each round structures the graph, takes the goto targets the report
/// attributes to revisited (shared) tails, and copies every eligible
/// target once per extra predecessor. A tail is eligible when it is not
/// the entry, carries at most a few instructions, is no loop header, has
/// no exceptional edge in either direction, and is referenced by no
/// exception region (a copy would silently leave the region's extent).
///
/// The transform is a pure unfolding: instruction *values* are cloned, so
/// consumer-side identities embedded in them now appear at several graph
/// positions. Use it on derived presentation views, not canonical storage.
pub fn duplicate_structuring_tails<I: Clone, E: Clone>(cfg: &mut Cfg<I, E>) -> usize {
    let mut duplicated = 0usize;
    for _ in 0..ROUNDS {
        let (_, report) = crate::lift_with_report(cfg);
        let mut targets: Vec<BlockId> = report
            .gotos
            .iter()
            .filter(|diagnostic| diagnostic.reason == GotoReason::RevisitedTarget)
            .map(|diagnostic| diagnostic.target)
            .collect();
        targets.sort_unstable();
        targets.dedup();
        if targets.is_empty() {
            break;
        }
        let dominators = DominatorTree::compute(cfg);
        let region_blocks = region_referenced_blocks(cfg);
        let mut progressed = false;
        for target in targets {
            if duplicated >= BLOCK_BUDGET {
                return duplicated;
            }
            if !eligible(cfg, &dominators, &region_blocks, target) {
                continue;
            }
            let incoming = cfg.predecessor_edges(target).to_vec();
            if incoming.len() < 2 {
                continue;
            }
            // The first predecessor keeps the original block; every other
            // one gets its own copy.
            for &edge in incoming.iter().skip(1) {
                if duplicated >= BLOCK_BUDGET {
                    break;
                }
                let (source, kind, payload) = {
                    let edge = cfg.edge(edge);
                    (edge.source(), edge.kind(), edge.payload().clone())
                };
                let copy = cfg.new_block();
                let instructions = cfg.block(target).instructions().to_vec();
                for instruction in instructions {
                    cfg.block_mut(copy).push(instruction);
                }
                for successor_edge in cfg.successor_edges(target).to_vec() {
                    let (kind, target, payload) = {
                        let edge = cfg.edge(successor_edge);
                        (edge.kind(), edge.target(), edge.payload().clone())
                    };
                    cfg.add_edge_with_payload(copy, target, kind, payload);
                }
                cfg.remove_edge(edge);
                cfg.add_edge_with_payload(source, copy, kind, payload);
                duplicated += 1;
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    duplicated
}

/// Every block an exception region names: protected extents, handler
/// entries, and known handler bodies.
fn region_referenced_blocks<I, E>(cfg: &Cfg<I, E>) -> BTreeSet<BlockId> {
    let mut blocks = BTreeSet::new();
    for region in cfg.regions() {
        blocks.extend(region.protected_blocks.iter().copied());
        for handler in &region.handlers {
            blocks.insert(handler.entry);
            if let HandlerBody::Known(body) = &handler.body {
                blocks.extend(body.iter().copied());
            }
        }
    }
    blocks
}

fn eligible<I, E>(
    cfg: &Cfg<I, E>,
    dominators: &DominatorTree,
    region_blocks: &BTreeSet<BlockId>,
    target: BlockId,
) -> bool {
    if target == cfg.entry()
        || region_blocks.contains(&target)
        || cfg.block(target).instructions().len() > INSTRUCTION_LIMIT
    {
        return false;
    }
    // A loop header's copy would re-enter the loop from outside its
    // natural structure.
    let looping = cfg
        .predecessor_edges(target)
        .iter()
        .any(|&edge| dominators.dominates(target, cfg.edge(edge).source()));
    if looping {
        return false;
    }
    // Exceptional flow references blocks (and consumer payloads reference
    // throw sites) that a copy cannot honestly carry.
    let exceptional = cfg
        .successor_edges(target)
        .iter()
        .chain(cfg.predecessor_edges(target))
        .any(|&edge| {
            matches!(
                cfg.edge(edge).kind(),
                crate::EdgeKind::ExceptionHandler
                    | crate::EdgeKind::ExceptionUnwind
                    | crate::EdgeKind::ExceptionLeave
                    | crate::EdgeKind::ExceptionResume
                    | crate::EdgeKind::ExceptionContinue
            )
        });
    !exceptional
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EdgeKind;
    use crate::test_util::{DfInst, df_def};

    /// Two conditionals sharing one assignment tail — the short-circuit
    /// shape `if (a || b) shared else other`.
    fn shared_tail_cfg() -> (Cfg<DfInst>, BlockId) {
        let mut cfg = Cfg::<DfInst>::new();
        let second = cfg.new_block();
        let tail = cfg.new_block();
        let then = cfg.new_block();
        let join = cfg.new_block();
        cfg.block_mut(tail).push(df_def("shared", 0));
        cfg.block_mut(then).push(df_def("other", 0));
        cfg.add_edge(cfg.entry(), tail, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), second, EdgeKind::ConditionalFalse);
        cfg.add_edge(second, tail, EdgeKind::ConditionalTrue);
        cfg.add_edge(second, then, EdgeKind::ConditionalFalse);
        cfg.add_edge(tail, join, EdgeKind::Fallthrough);
        cfg.add_edge(then, join, EdgeKind::Fallthrough);
        (cfg, tail)
    }

    #[test]
    fn shared_tail_duplicates_until_structured() {
        let (mut cfg, tail) = shared_tail_cfg();
        let (_, before) = crate::lift_with_report(&cfg);
        assert!(
            before
                .gotos
                .iter()
                .any(|diagnostic| diagnostic.reason == GotoReason::RevisitedTarget),
            "{before:?}"
        );
        let duplicated = duplicate_structuring_tails(&mut cfg);
        assert_eq!(duplicated, 1);
        let (_, after) = crate::lift_with_report(&cfg);
        assert!(after.is_fully_structured(), "{after:?}");
        // The original tail kept one predecessor; the copy took the other.
        assert_eq!(cfg.predecessor_edges(tail).len(), 1);
    }

    #[test]
    fn loop_headers_and_protected_blocks_stay_put() {
        // A self-loop header with two outside predecessors must not copy.
        let mut cfg = Cfg::<DfInst>::new();
        let left = cfg.new_block();
        let right = cfg.new_block();
        let header = cfg.new_block();
        let exit = cfg.new_block();
        cfg.block_mut(header).push(df_def("count", 0));
        cfg.add_edge(cfg.entry(), left, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), right, EdgeKind::ConditionalFalse);
        cfg.add_edge(left, header, EdgeKind::Fallthrough);
        cfg.add_edge(right, header, EdgeKind::Fallthrough);
        cfg.add_edge(header, header, EdgeKind::Back);
        cfg.add_edge(header, exit, EdgeKind::ConditionalFalse);
        let blocks = cfg.blocks().len();
        let _ = duplicate_structuring_tails(&mut cfg);
        assert_eq!(cfg.blocks().len(), blocks, "a loop header was copied");
    }
}
