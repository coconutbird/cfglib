//! Irreducible-to-reducible CFG transformation via node splitting.
//!
//! An irreducible CFG contains cycles with multiple entry points.
//! [`make_reducible`] eliminates these by duplicating the secondary
//! entry nodes so that every cycle has a single dominating header.
//!
//! The algorithm is iterative: after each round of splitting, the
//! dominator tree is recomputed and the CFG is re-checked. The loop
//! terminates when the CFG is reducible.

extern crate alloc;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::edge::EdgeKind;
use crate::graph::dominator::DominatorTree;
use crate::graph::structure::is_reducible;

/// Transform an irreducible CFG into a reducible one by node splitting.
///
/// Returns the number of blocks that were duplicated. If the CFG is
/// already reducible, returns 0 and makes no changes.
///
/// **Caution**: node splitting can cause exponential code growth in
/// pathological cases. For most real-world binaries the duplication
/// is modest.
pub fn make_reducible<I: Clone>(cfg: &mut Cfg<I>) -> usize {
    let mut total_split = 0;

    loop {
        let dom = DominatorTree::compute(cfg);
        if is_reducible(cfg, &dom) {
            break;
        }

        // Find irreducible entries and split ONE per iteration.
        // After each split the dominator tree is stale, so we
        // must recompute before picking the next target.
        let irreducible_targets = find_irreducible_entries(cfg, &dom);

        if let Some(&target) = irreducible_targets.first() {
            split_node(cfg, target);
            total_split += 1;
        } else {
            break; // Safety valve.
        }
    }

    total_split
}

/// Find blocks that are irreducible loop entries — targets of
/// retreating edges that don't dominate their source.
fn find_irreducible_entries<I>(cfg: &Cfg<I>, dom: &DominatorTree) -> Vec<BlockId> {
    let n = cfg.num_blocks();
    if n == 0 {
        return Vec::new();
    }

    const WHITE: u8 = 0;
    const GRAY: u8 = 1;

    let mut color = alloc::vec![WHITE; n];
    let mut stack: Vec<(BlockId, bool)> = alloc::vec![(cfg.entry(), false)];
    let mut targets = Vec::new();
    let mut seen = alloc::vec![false; n];

    while let Some((node, processed)) = stack.pop() {
        if processed {
            color[node.index()] = 2; // BLACK
            continue;
        }
        if color[node.index()] != WHITE {
            continue;
        }
        color[node.index()] = GRAY;
        stack.push((node, true));

        for succ in cfg.successors(node) {
            match color[succ.index()] {
                WHITE => stack.push((succ, false)),
                GRAY => {
                    if !dom.dominates(succ, node) && !seen[succ.index()] {
                        targets.push(succ);
                        seen[succ.index()] = true;
                    }
                }
                _ => {}
            }
        }
    }

    targets
}

/// Duplicate block `target` — create a copy and redirect all
/// non-dominator predecessors to the copy. This breaks the
/// irreducible entry by giving the secondary entries their own
/// copy of the block.
fn split_node<I: Clone>(cfg: &mut Cfg<I>, target: BlockId) {
    let dom = DominatorTree::compute(cfg);

    // Create a clone of the target block.
    let copy = cfg.new_block();
    let insts = cfg.block(target).instructions().to_vec();
    for inst in insts {
        cfg.blocks[copy.index()].instructions.push(inst);
    }

    // Collect predecessors of `target` and decide which to redirect.
    let pred_edges: Vec<crate::edge::EdgeId> = cfg.predecessor_edges(target).to_vec();
    let to_redirect: Vec<crate::edge::EdgeId> = pred_edges
        .iter()
        .filter(|&&eid| {
            let src = cfg.edge(eid).source();
            // Keep edges from nodes dominated by target (natural back-edges).
            // Redirect everything else to the copy.
            !dom.dominates(target, src)
        })
        .copied()
        .collect();

    for eid in to_redirect {
        cfg.edges[eid.index()].target = copy;
        cfg.preds[target.index()].retain(|&e| e != eid);
        cfg.preds[copy.index()].push(eid);
    }

    // Clone outgoing edges from target to copy.
    let outgoing: Vec<(BlockId, EdgeKind)> = cfg
        .successor_edges(target)
        .iter()
        .map(|&eid| {
            let e = cfg.edge(eid);
            (e.target(), e.kind())
        })
        .collect();

    for (succ, kind) in outgoing {
        cfg.add_edge(copy, succ, kind);
    }
}
