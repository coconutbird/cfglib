//! Edge contraction and node splitting.
//!
//! Graph rewriting primitives that complement the existing block
//! merging in [`super::cleanup`].

extern crate alloc;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;

/// Contract an edge by merging the target block into the source block.
///
/// The edge `(source → target)` is removed, the target's instructions
/// are appended to the source, and all outgoing edges of the target
/// are redirected to originate from the source.
///
/// Returns `true` if contraction was performed, `false` if the edge
/// cannot be contracted (e.g., target has other predecessors, or
/// source has other successors).
///
/// Requires `I: Clone` because instruction vectors are manipulated.
///
/// # Examples
///
/// ```
/// use cfglib::{Cfg, EdgeKind, contract_edge};
///
/// let mut cfg = Cfg::<u32>::new();
/// let b0 = cfg.entry();
/// let b1 = cfg.new_block();
/// cfg.add_edge(b0, b1, EdgeKind::Fallthrough);
///
/// // b0 has 1 succ, b1 has 1 pred — contractible.
/// assert!(contract_edge(&mut cfg, b0, b1));
/// ```
pub fn contract_edge<I: Clone>(cfg: &mut Cfg<I>, source: BlockId, target: BlockId) -> bool {
    // Target must have exactly one predecessor (source).
    if cfg.predecessor_edges(target).len() != 1 {
        return false;
    }
    // Source must have exactly one successor (target).
    if cfg.successor_edges(source).len() != 1 {
        return false;
    }
    // Don't contract self-loops.
    if source == target {
        return false;
    }

    // Append target's instructions to source.
    let target_instrs = cfg.block(target).instructions().to_vec();
    cfg.block_mut(source)
        .instructions_vec_mut()
        .extend(target_instrs);

    // Copy label if source doesn't have one.
    if cfg.block(source).label().is_none()
        && let Some(lbl) = cfg.block(target).label()
    {
        let owned = alloc::string::String::from(lbl);
        cfg.block_mut(source).set_label(owned);
    }

    // Remove the edge source → target.
    let edge_to_remove: Vec<_> = cfg
        .successor_edges(source)
        .iter()
        .copied()
        .filter(|&eid| cfg.edge(eid).target() == target)
        .collect();
    for eid in edge_to_remove {
        cfg.remove_edge(eid);
    }

    // Redirect target's outgoing edges to source.
    let target_out: Vec<_> = cfg.successor_edges(target).to_vec();
    for &eid in &target_out {
        let e = cfg.edge(eid);
        let dest = e.target();
        let kind = e.kind();
        cfg.remove_edge(eid);
        cfg.add_edge(source, dest, kind);
    }

    true
}

/// Split a block at a given instruction index, creating a new block.
///
/// This is a thin wrapper around [`Cfg::split_block`] that also
/// reconnects edges properly.
///
/// Returns the new block containing instructions from `at` onward.
pub fn split_node<I: Clone>(cfg: &mut Cfg<I>, block: BlockId, at: usize) -> BlockId {
    cfg.split_block(block, at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::EdgeKind;
    use crate::test_util::ff;

    #[test]
    fn contract_linear_chain() {
        let mut cfg = Cfg::new();
        let b = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("a"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.add_edge(cfg.entry(), b, EdgeKind::Fallthrough);

        let entry = cfg.entry();
        let ok = contract_edge(&mut cfg, entry, b);
        assert!(ok);
        assert_eq!(cfg.block(entry).instructions().len(), 2);
    }

    #[test]
    fn contract_refuses_multi_pred() {
        let mut cfg: Cfg<crate::test_util::MockInst> = Cfg::new();
        let a = cfg.new_block();
        let b = cfg.new_block();
        let merge = cfg.new_block();
        cfg.add_edge(cfg.entry(), a, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), b, EdgeKind::ConditionalFalse);
        cfg.add_edge(a, merge, EdgeKind::Fallthrough);
        cfg.add_edge(b, merge, EdgeKind::Fallthrough);

        // merge has 2 predecessors — cannot contract.
        assert!(!contract_edge(&mut cfg, a, merge));
    }

    #[test]
    fn split_node_works() {
        let mut cfg = Cfg::new();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .extend([ff("a"), ff("b"), ff("c")]);

        let entry = cfg.entry();
        let new_block = split_node(&mut cfg, entry, 1);
        assert_eq!(cfg.block(entry).instructions().len(), 1);
        assert_eq!(cfg.block(new_block).instructions().len(), 2);
    }
}
