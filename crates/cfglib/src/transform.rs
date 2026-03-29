//! CFG transformation passes — cleanup and simplification.
//!
//! All passes mutate the graph in-place and return the number of
//! blocks or edges affected.

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::edge::EdgeKind;

/// Remove blocks unreachable from the entry block.
///
/// Unreachable blocks have their instructions cleared and all
/// incident edges removed, turning them into dead slots in the
/// arena. Returns the number of blocks made unreachable.
pub fn remove_unreachable<I>(cfg: &mut Cfg<I>) -> usize {
    let reachable = cfg.dfs_preorder();
    let n = cfg.num_blocks();
    let mut is_reachable = vec![false; n];
    for &id in &reachable {
        is_reachable[id.index()] = true;
    }

    let mut removed = 0;
    for (i, &reachable) in is_reachable.iter().enumerate() {
        if !reachable {
            let id = BlockId::from_raw(i as u32);
            let has_insts = !cfg.block(id).instructions().is_empty();
            let has_edges =
                !cfg.successor_edges(id).is_empty() || !cfg.predecessor_edges(id).is_empty();
            if !has_insts && !has_edges {
                continue; // Already dead — nothing to clean up.
            }
            // Clear instructions.
            cfg.block_mut(id).instructions_vec_mut().clear();
            // Remove all outgoing edges.
            let out: Vec<_> = cfg.successor_edges(id).to_vec();
            for eid in out {
                cfg.remove_edge(eid);
            }
            // Remove all incoming edges.
            let inc: Vec<_> = cfg.predecessor_edges(id).to_vec();
            for eid in inc {
                cfg.remove_edge(eid);
            }
            removed += 1;
        }
    }
    removed
}

/// Merge a block with its sole successor when:
/// - the block has exactly one successor, and
/// - that successor has exactly one predecessor.
///
/// Returns the number of merges performed.
pub fn merge_blocks<I>(cfg: &mut Cfg<I>) -> usize {
    let mut merged = 0;
    let mut changed = true;
    while changed {
        changed = false;
        let order = cfg.dfs_preorder();
        for &id in &order {
            let succ_edges = cfg.successor_edges(id).to_vec();
            if succ_edges.len() != 1 {
                continue;
            }
            let target = cfg.edge(succ_edges[0]).target();
            if target == id {
                continue; // self-loop
            }
            if cfg.predecessor_edges(target).len() != 1 {
                continue;
            }
            // Merge: append target's instructions to id.
            let target_insts: Vec<I> = cfg
                .block_mut(target)
                .instructions_vec_mut()
                .drain(..)
                .collect();
            cfg.block_mut(id)
                .instructions_vec_mut()
                .extend(target_insts);

            // Remove the connecting edge.
            cfg.remove_edge(succ_edges[0]);

            // Transfer target's outgoing edges to id.
            let target_out: Vec<_> = cfg.successor_edges(target).to_vec();
            for eid in target_out {
                let edge = cfg.edge(eid);
                let kind = edge.kind();
                let dest = edge.target();
                cfg.remove_edge(eid);
                cfg.add_edge(id, dest, kind);
            }

            merged += 1;
            changed = true;
            break; // restart scan — indices may have shifted
        }
    }
    merged
}

/// Remove empty blocks that have a single unconditional/fallthrough
/// successor by redirecting predecessors to the successor.
///
/// Returns the number of blocks bypassed.
pub fn remove_empty_blocks<I>(cfg: &mut Cfg<I>) -> usize {
    let mut removed = 0;
    let mut changed = true;
    while changed {
        changed = false;
        let order = cfg.dfs_preorder();
        for &id in &order {
            if id == cfg.entry() {
                continue;
            }
            if !cfg.block(id).is_empty() {
                continue;
            }
            let succ_edges = cfg.successor_edges(id).to_vec();
            if succ_edges.len() != 1 {
                continue;
            }
            let edge = cfg.edge(succ_edges[0]);
            if !matches!(edge.kind(), EdgeKind::Fallthrough | EdgeKind::Unconditional) {
                continue;
            }
            let target = edge.target();
            // Redirect all predecessors of `id` to `target`.
            cfg.redirect_edges_to(id, target);
            // Remove the outgoing edge.
            cfg.remove_edge(succ_edges[0]);
            removed += 1;
            changed = true;
            break;
        }
    }
    removed
}

/// Run all simplification passes until no more changes occur.
///
/// Returns the total number of transformations applied.
pub fn simplify<I>(cfg: &mut Cfg<I>) -> usize {
    let mut total = 0;
    loop {
        let r = remove_unreachable(cfg);
        let e = remove_empty_blocks(cfg);
        let m = merge_blocks(cfg);
        let round = r + e + m;
        if round == 0 {
            break;
        }
        total += round;
    }
    total
}

/// Split **critical edges** in the CFG.
///
/// A critical edge is one from a block with multiple successors to a
/// block with multiple predecessors. Splitting such edges by inserting
/// an empty block in between is required for correct SSA phi placement
/// and simplifies many transformations.
///
/// Returns the number of edges split.
pub fn split_critical_edges<I: Clone>(cfg: &mut Cfg<I>) -> usize {
    // Collect critical edges first (can't mutate while iterating).
    let mut critical = Vec::new();
    for block in cfg.blocks() {
        let bid = block.id();
        let succ_edges = cfg.successor_edges(bid);
        if succ_edges.len() < 2 {
            continue; // not a multi-successor block
        }
        for &eid in succ_edges {
            let target = cfg.edge(eid).target();
            if cfg.predecessor_edges(target).len() >= 2 {
                critical.push(eid);
            }
        }
    }

    let mut split_count = 0;
    for eid in critical {
        let edge = cfg.edge(eid);
        let src = edge.source();
        let tgt = edge.target();
        let kind = edge.kind();
        let weight = edge.weight();

        // Remove old edge.
        cfg.remove_edge(eid);

        // Insert new empty block.
        let mid = cfg.new_block();
        let e1 = cfg.add_edge(src, mid, kind);
        cfg.add_edge(mid, tgt, EdgeKind::Fallthrough);

        // Preserve weight on the first half.
        if let Some(w) = weight {
            cfg.edge_mut(e1).set_weight(Some(w));
        }

        split_count += 1;
    }

    split_count
}

/// Dead code elimination — remove instructions whose definitions
/// are never used.
///
/// Uses liveness analysis to identify instructions that define
/// locations which are not live after the instruction. Instructions
/// with side effects (non-empty `effects()`) are always kept.
///
/// Returns the number of instructions removed.
pub fn dead_code_elimination<I: crate::dataflow::InstrInfo + Clone>(cfg: &mut Cfg<I>) -> usize {
    use crate::dataflow::fixpoint;
    use crate::dataflow::liveness::LivenessProblem;

    let liveness = fixpoint::solve(cfg, &LivenessProblem);
    let mut removed = 0;

    // Phase 1: compute which instructions to keep per block.
    let block_ids: Vec<BlockId> = cfg.blocks().iter().map(|b| b.id()).collect();
    let mut replacements: Vec<(BlockId, Vec<I>)> = Vec::new();

    for &bid in &block_ids {
        let live_out = liveness.fact_out(bid).clone();
        let insts = cfg.block(bid).instructions().to_vec();
        let mut live = live_out;
        let mut keep = vec![true; insts.len()];

        for (i, inst) in insts.iter().enumerate().rev() {
            let has_side_effect = !inst.effects().is_empty();
            let defs_live = inst.defs().iter().any(|d| live.contains(d));

            if !has_side_effect && !inst.defs().is_empty() && !defs_live {
                keep[i] = false;
                removed += 1;
            } else {
                for d in inst.defs() {
                    live.remove(d);
                }
                for u in inst.uses() {
                    live.insert(*u);
                }
            }
        }

        if keep.iter().any(|&k| !k) {
            let new_insts: Vec<I> = insts
                .into_iter()
                .zip(keep.iter())
                .filter(|(_, k)| **k)
                .map(|(inst, _)| inst)
                .collect();
            replacements.push((bid, new_insts));
        }
    }

    // Phase 2: apply replacements.
    for (bid, new_insts) in replacements {
        *cfg.block_mut(bid).instructions_vec_mut() = new_insts;
    }

    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::edge::EdgeKind;
    use crate::test_util::{MockInst, ff};

    /// Build a diamond CFG: entry → A, entry → B, A → merge, B → merge.
    fn diamond_cfg() -> Cfg<MockInst> {
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
        cfg
    }

    #[test]
    fn remove_unreachable_noop_when_all_reachable() {
        let mut cfg = diamond_cfg();
        let removed = remove_unreachable(&mut cfg);
        assert_eq!(removed, 0);
    }

    #[test]
    fn remove_unreachable_removes_disconnected_block() {
        let mut cfg = diamond_cfg();
        // Add an unreachable block.
        let orphan = cfg.new_block();
        cfg.block_mut(orphan)
            .instructions_vec_mut()
            .push(ff("dead"));
        let removed = remove_unreachable(&mut cfg);
        assert_eq!(removed, 1);
        assert!(cfg.block(orphan).instructions().is_empty());
    }

    #[test]
    fn merge_blocks_merges_linear_chain() {
        let mut cfg = Cfg::new();
        let b = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("a"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.add_edge(cfg.entry(), b, EdgeKind::Fallthrough);
        let merged = merge_blocks(&mut cfg);
        assert_eq!(merged, 1);
        // entry should now contain both instructions.
        assert_eq!(cfg.block(cfg.entry()).instructions().len(), 2);
    }

    #[test]
    fn merge_blocks_does_not_merge_when_multiple_predecessors() {
        let mut cfg = diamond_cfg();
        // merge block has 2 predecessors — should not merge.
        let merged = merge_blocks(&mut cfg);
        assert_eq!(merged, 0);
    }

    #[test]
    fn merge_blocks_skips_self_loop() {
        let mut cfg = Cfg::new();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("a"));
        cfg.add_edge(cfg.entry(), cfg.entry(), EdgeKind::Back);
        let merged = merge_blocks(&mut cfg);
        assert_eq!(merged, 0);
    }

    #[test]
    fn remove_empty_blocks_bypasses_empty_block() {
        let mut cfg = Cfg::new();
        let empty = cfg.new_block();
        let target = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("entry"));
        cfg.block_mut(target)
            .instructions_vec_mut()
            .push(ff("target"));
        cfg.add_edge(cfg.entry(), empty, EdgeKind::Fallthrough);
        cfg.add_edge(empty, target, EdgeKind::Fallthrough);
        let removed = remove_empty_blocks(&mut cfg);
        assert_eq!(removed, 1);
        // entry should now go directly to target.
        let succs: Vec<_> = cfg.successors(cfg.entry()).collect();
        assert_eq!(succs.len(), 1);
        assert_eq!(succs[0], target);
    }

    #[test]
    fn remove_empty_blocks_does_not_remove_entry() {
        // Entry block is empty but should be preserved.
        let mut cfg = Cfg::new();
        let b = cfg.new_block();
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.add_edge(cfg.entry(), b, EdgeKind::Fallthrough);
        let removed = remove_empty_blocks(&mut cfg);
        assert_eq!(removed, 0);
    }

    #[test]
    fn simplify_runs_all_passes() {
        let mut cfg = Cfg::new();
        let empty = cfg.new_block();
        let b = cfg.new_block();
        let orphan = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("entry"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.block_mut(orphan)
            .instructions_vec_mut()
            .push(ff("dead"));
        cfg.add_edge(cfg.entry(), empty, EdgeKind::Fallthrough);
        cfg.add_edge(empty, b, EdgeKind::Fallthrough);
        // orphan has no incoming edges — unreachable.
        let total = simplify(&mut cfg);
        assert!(
            total > 0,
            "simplify should perform at least 1 transformation"
        );
        // orphan should be cleared.
        assert!(cfg.block(orphan).instructions().is_empty());
    }

    #[test]
    fn split_critical_edges_on_diamond() {
        // Diamond: entry → A, entry → B, A → merge, B → merge.
        // Edges A→merge and B→merge are critical (A/B have 1 succ, but
        // entry has 2 succs and merge has 2 preds → entry→A and entry→B
        // are actually not critical because A and B have 1 pred each).
        // But the edges entry→A and entry→B go to blocks with 1 pred,
        // so they're not critical. A→merge and B→merge: A has 1 succ,
        // so not critical. So a basic diamond has NO critical edges.
        let mut cfg = diamond_cfg();
        let split = split_critical_edges(&mut cfg);
        assert_eq!(split, 0, "basic diamond has no critical edges");
    }

    #[test]
    fn split_critical_edges_inserts_block() {
        // Make an actual critical edge: entry has 2 succs (A, B)
        // and both A and B have 2 preds.
        let mut cfg = Cfg::new();
        let a = cfg.new_block();
        let b = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("entry"));
        cfg.block_mut(a).instructions_vec_mut().push(ff("a"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        // entry → a (true), entry → b (false).
        cfg.add_edge(cfg.entry(), a, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), b, EdgeKind::ConditionalFalse);
        // Create another block c that also targets a and b.
        let c = cfg.new_block();
        cfg.block_mut(c).instructions_vec_mut().push(ff("c"));
        cfg.add_edge(c, a, EdgeKind::ConditionalTrue);
        cfg.add_edge(c, b, EdgeKind::ConditionalFalse);

        let orig_blocks = cfg.num_blocks();
        let split = split_critical_edges(&mut cfg);
        // All 4 edges are critical (2-succ src → 2-pred target).
        assert_eq!(split, 4);
        assert_eq!(cfg.num_blocks(), orig_blocks + 4);
    }

    #[test]
    fn dead_code_elimination_removes_unused_def() {
        use crate::test_util::{DfInst, df_def, df_use};

        let mut cfg: Cfg<DfInst> = Cfg::new();
        let exit = cfg.new_block();

        // Entry: def loc0 (dead), def loc1 (used in exit).
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .extend([df_def("dead_def", 0), df_def("live_def", 1)]);

        // Exit: use loc1.
        cfg.block_mut(exit)
            .instructions_vec_mut()
            .push(df_use("use1", 1));

        cfg.add_edge(cfg.entry(), exit, EdgeKind::Fallthrough);

        let removed = dead_code_elimination(&mut cfg);
        assert_eq!(removed, 1, "should remove the dead def of loc0");
        assert_eq!(cfg.block(cfg.entry()).instructions().len(), 1);
        assert_eq!(cfg.block(cfg.entry()).instructions()[0].name, "live_def");
    }
}
