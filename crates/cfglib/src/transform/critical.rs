//! Critical edge splitting.
//!
//! A critical edge is one from a block with multiple successors to a
//! block with multiple predecessors. Splitting such edges by inserting
//! an empty block in between is required for correct SSA phi placement
//! and simplifies many transformations.

extern crate alloc;
use alloc::vec::Vec;

use crate::cfg::Cfg;
use crate::edge::EdgeKind;

/// Split **critical edges** in the CFG.
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
