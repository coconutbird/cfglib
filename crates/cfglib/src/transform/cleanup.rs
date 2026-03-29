//! Basic CFG cleanup passes — unreachable block removal, block merging,
//! empty block bypass, and combined simplification.
//!
//! All passes mutate the graph in-place and return the number of
//! blocks affected.

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
///
/// # Examples
///
/// ```
/// use cfglib::{Cfg, EdgeKind, simplify};
///
/// let mut cfg = Cfg::<u32>::new();
/// let b0 = cfg.entry();
/// let b1 = cfg.new_block();
/// let b2 = cfg.new_block(); // unreachable
/// cfg.add_edge(b0, b1, EdgeKind::Fallthrough);
///
/// let changes = simplify(&mut cfg);
/// assert!(changes > 0); // removed unreachable b2
/// ```
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
