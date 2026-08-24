//! Basic CFG cleanup passes with metadata-preserving rewrite maps.

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::edge::EdgeKind;
use crate::rewrite::RewriteMap;

/// Remove blocks unreachable from the entry block.
///
/// This compatibility entry point returns only the number of blocks made
/// unreachable. Use [`remove_unreachable_mapped`] when identities matter.
pub fn remove_unreachable<I, E>(cfg: &mut Cfg<I, E>) -> usize {
    remove_unreachable_mapped(cfg).0
}

/// Remove unreachable blocks and report every removed block and edge.
///
/// Storage slots remain allocated, matching [`Cfg`]'s stable-identity model,
/// but removed blocks have no instructions or incident edges.
pub fn remove_unreachable_mapped<I, E>(cfg: &mut Cfg<I, E>) -> (usize, RewriteMap) {
    let reachable = cfg.dfs_preorder();
    let mut is_reachable = vec![false; cfg.num_blocks()];
    for &id in &reachable {
        is_reachable[id.index()] = true;
    }

    let mut removed = 0;
    let mut mapping = RewriteMap::new();
    for (index, &reachable) in is_reachable.iter().enumerate() {
        if reachable {
            continue;
        }
        let id = BlockId::from_index(index);
        let has_instructions = !cfg.block(id).instructions().is_empty();
        let has_edges =
            !cfg.successor_edges(id).is_empty() || !cfg.predecessor_edges(id).is_empty();
        if !has_instructions && !has_edges {
            continue;
        }

        cfg.block_mut(id).instructions_vec_mut().clear();
        let mut incident: Vec<_> = cfg.successor_edges(id).to_vec();
        for &edge in cfg.predecessor_edges(id) {
            if !incident.contains(&edge) {
                incident.push(edge);
            }
        }
        for edge in incident {
            let (_, removed_edge) = cfg.remove_edge_mapped(edge);
            mapping.compose(removed_edge);
        }
        mapping.record_block(id, []);
        removed += 1;
    }
    (removed, mapping)
}

/// Merge blocks connected by a sole-successor, sole-predecessor edge.
///
/// This compatibility entry point returns only the number of merges. Use
/// [`merge_blocks_mapped`] to retain the identity relationship.
pub fn merge_blocks<I, E>(cfg: &mut Cfg<I, E>) -> usize {
    merge_blocks_mapped(cfg).0
}

/// Merge linear blocks while preserving every surviving edge identity and
/// payload.
pub fn merge_blocks_mapped<I, E>(cfg: &mut Cfg<I, E>) -> (usize, RewriteMap) {
    let mut merged = 0;
    let mut mapping = RewriteMap::new();
    let mut changed = true;
    while changed {
        changed = false;
        let order = cfg.dfs_preorder();
        for &source in &order {
            let successor_edges = cfg.successor_edges(source).to_vec();
            if successor_edges.len() != 1 {
                continue;
            }
            let connecting = successor_edges[0];
            let target = cfg.edge(connecting).target();
            if target == source || target == cfg.entry() {
                continue;
            }
            if cfg.predecessor_edges(target).len() != 1 {
                continue;
            }

            let target_instructions = core::mem::take(cfg.block_mut(target).instructions_vec_mut());
            cfg.block_mut(source)
                .instructions_vec_mut()
                .extend(target_instructions);

            let (_, removed_edge) = cfg.remove_edge_mapped(connecting);
            mapping.compose(removed_edge);
            let target_outgoing = cfg.successor_edges(target).to_vec();
            for edge in target_outgoing {
                let (_, redirected) = cfg.redirect_edge_source_mapped(edge, source);
                mapping.compose(redirected);
            }
            mapping.record_block(target, [source]);

            merged += 1;
            changed = true;
            break;
        }
    }
    (merged, mapping)
}

/// Bypass empty blocks with one fallthrough-like successor.
///
/// This compatibility entry point returns only the count. Use
/// [`remove_empty_blocks_mapped`] when clients retain graph identities.
pub fn remove_empty_blocks<I, E>(cfg: &mut Cfg<I, E>) -> usize {
    remove_empty_blocks_mapped(cfg).0
}

/// Bypass empty blocks without reallocating their incoming edges.
pub fn remove_empty_blocks_mapped<I, E>(cfg: &mut Cfg<I, E>) -> (usize, RewriteMap) {
    let mut removed = 0;
    let mut mapping = RewriteMap::new();
    let mut changed = true;
    while changed {
        changed = false;
        let order = cfg.dfs_preorder();
        for &id in &order {
            if id == cfg.entry() || !cfg.block(id).is_empty() {
                continue;
            }
            let successor_edges = cfg.successor_edges(id).to_vec();
            if successor_edges.len() != 1 {
                continue;
            }
            let outgoing = successor_edges[0];
            let edge = cfg.edge(outgoing);
            if !matches!(edge.kind(), EdgeKind::Fallthrough | EdgeKind::Unconditional) {
                continue;
            }
            let target = edge.target();
            if target == id {
                continue;
            }

            let redirected = cfg.redirect_edges_to_mapped(id, target);
            mapping.compose(redirected);
            let (_, removed_edge) = cfg.remove_edge_mapped(outgoing);
            mapping.compose(removed_edge);
            mapping.record_block(id, []);
            removed += 1;
            changed = true;
            break;
        }
    }
    (removed, mapping)
}

/// Run all simplification passes until no more changes occur.
///
/// Returns the total number of transformations applied. Use
/// [`simplify_mapped`] to retain the composed identity mapping.
///
/// # Examples
///
/// ```
/// use cfglib::{Cfg, EdgeKind, simplify};
///
/// let mut cfg = Cfg::<u32>::new();
/// let b1 = cfg.new_block();
/// let _unreachable = cfg.new_block();
/// cfg.add_edge(cfg.entry(), b1, EdgeKind::Fallthrough);
///
/// assert!(simplify(&mut cfg) > 0);
/// ```
pub fn simplify<I, E>(cfg: &mut Cfg<I, E>) -> usize {
    simplify_mapped(cfg).0
}

/// Run all simplification passes and compose their rewrite maps.
pub fn simplify_mapped<I, E>(cfg: &mut Cfg<I, E>) -> (usize, RewriteMap) {
    let mut total = 0;
    let mut mapping = RewriteMap::new();
    loop {
        let (unreachable, unreachable_map) = remove_unreachable_mapped(cfg);
        mapping.compose(unreachable_map);
        let (empty, empty_map) = remove_empty_blocks_mapped(cfg);
        mapping.compose(empty_map);
        let (merged, merged_map) = merge_blocks_mapped(cfg);
        mapping.compose(merged_map);
        let round = unreachable + empty + merged;
        if round == 0 {
            break;
        }
        total += round;
    }
    (total, mapping)
}
