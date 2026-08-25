//! Edge contraction and instruction-neutral node splitting.

extern crate alloc;

use crate::block::BlockId;
use crate::cfg::{Cfg, SplitPointError};
use crate::rewrite::RewriteMap;

/// Contract an edge by merging `target` into `source`.
///
/// This compatibility entry point reports only whether contraction happened.
/// Use [`contract_edge_mapped`] when clients retain graph identities.
pub fn contract_edge<I, E>(cfg: &mut Cfg<I, E>, source: BlockId, target: BlockId) -> bool {
    contract_edge_mapped(cfg, source, target).is_some()
}

/// Contract an edge while retaining all surviving outgoing edge identities
/// and payloads.
///
/// Returns `None` unless `source` has exactly one outgoing edge, that edge
/// targets `target`, and it is `target`'s sole incoming edge. The connecting
/// edge maps to removal, `target` maps to `source`, and redirected outgoing
/// edges map to themselves.
pub fn contract_edge_mapped<I, E>(
    cfg: &mut Cfg<I, E>,
    source: BlockId,
    target: BlockId,
) -> Option<RewriteMap> {
    if source == target || target == cfg.entry() {
        return None;
    }
    let source_outgoing = cfg.successor_edges(source);
    let target_incoming = cfg.predecessor_edges(target);
    if source_outgoing.len() != 1 || target_incoming.len() != 1 {
        return None;
    }
    let connecting = source_outgoing[0];
    if target_incoming[0] != connecting || cfg.edge(connecting).target() != target {
        return None;
    }

    let target_label = cfg.block(target).label().map(alloc::string::String::from);
    let target_instructions = core::mem::take(cfg.block_mut(target).instructions_mut());
    cfg.block_mut(source)
        .instructions_mut()
        .extend(target_instructions);
    if cfg.block(source).label().is_none() {
        if let Some(label) = target_label {
            cfg.block_mut(source).set_label(label);
        }
    }

    let mut mapping = RewriteMap::new();
    let (_, removed) = cfg.remove_edge_mapped(connecting);
    mapping.compose(removed);
    let target_outgoing = cfg.successor_edges(target).to_vec();
    for edge in target_outgoing {
        let (_, redirected) = cfg.redirect_edge_source_mapped(edge, source);
        mapping.compose(redirected);
    }
    mapping.record_block(target, [source]);
    Some(mapping)
}

/// Split a block at one instruction index with a default edge payload.
pub fn split_node<I, E>(cfg: &mut Cfg<I, E>, block: BlockId, at: usize) -> BlockId
where
    E: Default,
{
    cfg.split_block(block, at)
}

/// Split a block at one instruction index and return its rewrite mapping.
pub fn split_node_with_payload_mapped<I, E>(
    cfg: &mut Cfg<I, E>,
    block: BlockId,
    at: usize,
    payload: E,
) -> (BlockId, RewriteMap) {
    cfg.split_block_with_payload_mapped(block, at, payload)
}

/// Split a block at several original instruction boundaries.
///
/// This frontend-neutral primitive is suitable for materializing individual
/// throwing instructions, invoke sites, or any other consumer-selected
/// program points without encoding instruction semantics in cfglib.
///
/// # Errors
///
/// Returns [`SplitPointError`] when a point is out of bounds or the points are
/// not strictly increasing.
pub fn split_node_at_points<I, E>(
    cfg: &mut Cfg<I, E>,
    block: BlockId,
    points: impl IntoIterator<Item = (usize, E)>,
) -> Result<(alloc::vec::Vec<BlockId>, RewriteMap), SplitPointError> {
    cfg.split_block_at_points_with_payloads(block, points)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::EdgeKind;
    use crate::test_util::ff;

    #[test]
    fn contract_retains_outgoing_identity_and_payload() {
        let mut cfg = Cfg::<_, &'static str>::with_edge_payload();
        let target = cfg.new_block();
        let exit = cfg.new_block();
        cfg.block_mut(cfg.entry()).push(ff("a"));
        cfg.block_mut(target).push(ff("b"));
        let connecting =
            cfg.add_edge_with_payload(cfg.entry(), target, EdgeKind::Fallthrough, "join");
        let outgoing = cfg.add_edge_with_payload(target, exit, EdgeKind::Jump, "provenance");

        let entry = cfg.entry();
        let mapping = contract_edge_mapped(&mut cfg, entry, target).unwrap();
        assert_eq!(mapping.edge_replacements(connecting), Some([].as_slice()));
        assert_eq!(
            mapping.edge_replacements(outgoing),
            Some([outgoing].as_slice())
        );
        assert_eq!(mapping.block_replacements(target), Some([entry].as_slice()));
        assert_eq!(cfg.edge(outgoing).source(), entry);
        assert_eq!(cfg.edge(outgoing).payload(), &"provenance");
    }

    #[test]
    fn split_points_are_atomic_and_instruction_neutral() {
        let mut cfg = Cfg::<_, u8>::with_edge_payload();
        let entry = cfg.entry();
        cfg.block_mut(entry)
            .instructions_mut()
            .extend([ff("a"), ff("may_throw"), ff("c")]);

        let error = split_node_at_points(&mut cfg, entry, [(2, 2), (1, 1)]).unwrap_err();
        assert!(matches!(
            error,
            SplitPointError::NotStrictlyIncreasing {
                previous: 2,
                point: 1
            }
        ));
        assert_eq!(cfg.block_count(), 1, "validation precedes mutation");

        let (blocks, mapping) = split_node_at_points(&mut cfg, entry, [(1, 10), (2, 20)]).unwrap();
        assert_eq!(blocks.len(), 3);
        assert_eq!(cfg.block(blocks[1]).instructions()[0].1, "may_throw");
        assert_eq!(mapping.block_replacements(entry), Some(blocks.as_slice()));
        let first_edge = cfg.successor_edges(blocks[0])[0];
        let second_edge = cfg.successor_edges(blocks[1])[0];
        assert_eq!(cfg.edge(first_edge).payload(), &10);
        assert_eq!(cfg.edge(second_edge).payload(), &20);
    }
}
