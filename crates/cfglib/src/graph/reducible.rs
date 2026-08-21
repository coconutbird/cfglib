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
use smallvec::SmallVec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::edge::EdgeKind;
use crate::graph::dominator::DominatorTree;
use crate::graph::traverse::{TraversalDirection, reachable};

/// Transform an irreducible CFG into a reducible one by node splitting.
///
/// Returns the number of blocks that were duplicated. If the CFG is
/// already reducible, returns 0 and makes no changes.
///
/// **Caution**: node splitting can cause exponential code growth in
/// pathological cases. For most real-world binaries the duplication
/// is modest.
///
/// # Examples
///
/// ```
/// use cfglib::{Cfg, EdgeKind};
/// use cfglib::graph::reducible::make_reducible;
///
/// // A simple reducible CFG returns 0 (no changes).
/// let mut cfg = Cfg::<u32>::new();
/// let b1 = cfg.new_block();
/// cfg.add_edge(cfg.entry(), b1, EdgeKind::Fallthrough);
/// assert_eq!(make_reducible(&mut cfg), 0);
/// ```
pub fn make_reducible<I: Clone>(cfg: &mut Cfg<I>) -> usize {
    let mut total_split = 0;

    loop {
        let dom = DominatorTree::compute(cfg);
        // Find an irreducible entry and split ONE per iteration.
        // After each split the dominator tree is stale, so we
        // must recompute before picking the next target.
        if let Some(target) = find_irreducible_entry(cfg, &dom) {
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
fn find_irreducible_entry<I>(cfg: &Cfg<I>, dom: &DominatorTree) -> Option<BlockId> {
    const WHITE: u8 = 0;
    const GRAY: u8 = 1;
    const BLACK: u8 = 2;

    let n = cfg.num_blocks();
    if n == 0 {
        return None;
    }

    let mut color = alloc::vec![WHITE; n];
    let mut stack: Vec<(BlockId, bool)> = alloc::vec![(cfg.entry(), false)];

    while let Some((node, processed)) = stack.pop() {
        if processed {
            color[node.index()] = BLACK;
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
                GRAY if !dom.dominates(succ, node) => return Some(succ),
                _ => {}
            }
        }
    }

    None
}

/// Duplicate block `target` — create a copy and redirect external
/// predecessors to the copy, keeping cycle-internal predecessors
/// on the original. This breaks the irreducible entry by giving
/// external entries their own copy of the block.
fn split_node<I: Clone>(cfg: &mut Cfg<I>, target: BlockId) {
    // Create a clone of the target block.
    let copy = cfg.new_block();
    let insts = cfg.block(target).instructions().to_vec();
    for inst in insts {
        cfg.blocks[copy.index()].instructions.push(inst);
    }

    // Partition predecessors: keep edges from blocks that target
    // can reach (they're in a cycle with target), redirect the rest
    // to the copy (they're external entries).
    let cycle_reachable = reachable(cfg, [target], TraversalDirection::Outgoing);
    let mut redirected = SmallVec::<[crate::edge::EdgeId; 4]>::new();
    {
        let edges = &mut cfg.edges;
        cfg.preds[target.index()].retain(|eid| {
            let eid = *eid;
            let edge = edges[eid.index()].as_mut().unwrap();
            // If target can reach the source, they're in a cycle — keep it.
            if cycle_reachable[edge.source.index()] {
                true
            } else {
                edge.target = copy;
                redirected.push(eid);
                false
            }
        });
    }
    cfg.preds[copy.index()].extend(redirected);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::edge::EdgeKind;
    use crate::graph::dominator::DominatorTree;
    use crate::graph::structure::is_reducible;
    use crate::test_util::ff;

    #[test]
    fn already_reducible_is_noop() {
        // Simple diamond: entry → A → merge, entry → B → merge.
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

        let dom = DominatorTree::compute(&cfg);
        assert!(is_reducible(&cfg, &dom));
        let splits = make_reducible(&mut cfg);
        assert_eq!(splits, 0);
    }

    #[test]
    fn irreducible_cycle_is_fixed() {
        // Build an irreducible CFG:
        //   entry → A, entry → B
        //   A → B, B → A   (cycle with two entries)
        let mut cfg = Cfg::new();
        let a = cfg.new_block();
        let b = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("entry"));
        cfg.block_mut(a).instructions_vec_mut().push(ff("a"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.add_edge(cfg.entry(), a, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), b, EdgeKind::ConditionalFalse);
        cfg.add_edge(a, b, EdgeKind::Fallthrough);
        cfg.add_edge(b, a, EdgeKind::Fallthrough);

        let dom = DominatorTree::compute(&cfg);
        assert!(!is_reducible(&cfg, &dom), "should be irreducible before");

        let splits = make_reducible(&mut cfg);
        assert!(splits > 0, "should have split at least one node");

        let dom2 = DominatorTree::compute(&cfg);
        assert!(is_reducible(&cfg, &dom2), "should be reducible after");
    }
}
