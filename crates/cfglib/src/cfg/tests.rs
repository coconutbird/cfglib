extern crate alloc;
extern crate std;
use super::*;
use crate::edge::EdgeKind;
use crate::test_util::MockInst;
use alloc::vec;
use alloc::vec::Vec;

#[test]
fn edge_weight_roundtrip() {
    let mut cfg = Cfg::<MockInst>::new();
    let b0 = cfg.entry();
    let b1 = cfg.new_block();
    let eid = cfg.add_weighted_edge(b0, b1, EdgeKind::ConditionalTrue, 0.75);
    assert_eq!(cfg.edge(eid).weight(), Some(0.75));
    // Default edge should have no weight.
    let eid2 = cfg.add_edge(b0, b1, EdgeKind::Fallthrough);
    assert_eq!(cfg.edge(eid2).weight(), None);
}

#[test]
fn subgraph_extraction() {
    let mut cfg = Cfg::<MockInst>::new();
    let b0 = cfg.entry();
    let b1 = cfg.new_block();
    let b2 = cfg.new_block();
    cfg.add_edge(b0, b1, EdgeKind::Fallthrough);
    cfg.add_edge(b1, b2, EdgeKind::Fallthrough);

    // Extract first two blocks.
    let sub = cfg.subgraph(&[b0, b1]);
    assert_eq!(sub.block_count(), 2);
    // The subgraph should have an edge from block 0 to block 1.
    let succs: Vec<BlockId> = sub.successors(sub.entry()).collect();
    assert_eq!(succs.len(), 1);
}

#[test]
fn subgraph_empty_input() {
    let sub = Cfg::<MockInst>::new().subgraph(&[]);
    assert_eq!(sub.block_count(), 1); // Cfg::new() always has an entry
}

#[test]
fn remove_edge_tombstones_correctly() {
    let mut cfg = Cfg::<MockInst>::new();
    let b0 = cfg.entry();
    let b1 = cfg.new_block();
    let b2 = cfg.new_block();
    let e1 = cfg.add_edge(b0, b1, EdgeKind::ConditionalTrue);
    let e2 = cfg.add_edge(b0, b2, EdgeKind::ConditionalFalse);

    // Both edges are live.
    assert_eq!(cfg.edge_count(), 2);
    assert_eq!(cfg.edges().count(), 2);

    // Remove one edge.
    let removed = cfg.remove_edge(e1).unwrap();
    assert_eq!(removed.kind(), EdgeKind::ConditionalTrue);

    // edges() should now skip the tombstone.
    assert_eq!(cfg.edges().count(), 1);
    let remaining: Vec<&Edge> = cfg.edges().collect();
    assert_eq!(remaining[0].id(), e2);

    // Successor list should only contain e2.
    assert_eq!(cfg.successor_edges(b0).len(), 1);
    assert_eq!(cfg.successor_edges(b0)[0], e2);

    // Double-remove returns None.
    assert!(cfg.remove_edge(e1).is_none());
}

#[test]
fn split_block_preserves_outgoing_edge_identity_and_metadata() {
    let mut cfg = Cfg::<u32>::new();
    let source = cfg.entry();
    let sink = cfg.new_block();
    cfg.block_mut(source).instructions_mut().extend([1, 2]);
    let outgoing = cfg.add_weighted_edge(source, sink, EdgeKind::ConditionalTrue, 0.75);

    let split = cfg.split_block(source, 1);
    let [fallthrough] = cfg.successor_edges(source) else {
        panic!("split source should have one fallthrough edge");
    };

    assert_eq!(cfg.successor_edges(split), &[outgoing]);
    assert_eq!(cfg.edge(outgoing).source(), split);
    assert_eq!(cfg.edge(outgoing).target(), sink);
    assert_eq!(cfg.edge(outgoing).kind(), EdgeKind::ConditionalTrue);
    assert_eq!(cfg.edge(outgoing).weight(), Some(0.75));
    assert_eq!(cfg.edge(*fallthrough).source(), source);
    assert_eq!(cfg.edge(*fallthrough).target(), split);
    assert_eq!(cfg.predecessor_edges(sink), &[outgoing]);
}

#[test]
fn redirect_edges_moves_predecessors_in_order() {
    let mut cfg = Cfg::<MockInst>::new();
    let old = cfg.new_block();
    let new_target = cfg.new_block();
    let first = cfg.add_edge(cfg.entry(), new_target, EdgeKind::Fallthrough);
    let second = cfg.add_edge(cfg.entry(), old, EdgeKind::ConditionalTrue);
    let third = cfg.add_weighted_edge(cfg.entry(), old, EdgeKind::ConditionalFalse, 0.25);

    cfg.redirect_edges_to(old, new_target);

    assert_eq!(cfg.predecessor_edges(old), &[]);
    assert_eq!(cfg.predecessor_edges(new_target), &[first, second, third]);
    assert_eq!(cfg.edge(second).target(), new_target);
    assert_eq!(cfg.edge(third).target(), new_target);
    assert_eq!(cfg.edge(third).weight(), Some(0.25));
}

#[test]
fn redirect_edges_to_same_block_is_a_noop() {
    let mut cfg = Cfg::<MockInst>::new();
    let target = cfg.new_block();
    let edge = cfg.add_edge(cfg.entry(), target, EdgeKind::Fallthrough);

    cfg.redirect_edges_to(target, target);

    assert_eq!(cfg.predecessor_edges(target), &[edge]);
    assert_eq!(cfg.edge(edge).target(), target);
}

#[test]
fn redirect_edges_rejects_an_invalid_target_before_mutating() {
    let mut cfg = Cfg::<MockInst>::new();
    let old_target = cfg.new_block();
    let edge = cfg.add_edge(cfg.entry(), old_target, EdgeKind::Fallthrough);
    let invalid = BlockId::from_index(cfg.block_count());

    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cfg.redirect_edges_to(old_target, invalid);
    }));

    assert!(panic.is_err());
    assert_eq!(cfg.predecessor_edges(old_target), &[edge]);
    assert_eq!(cfg.edge(edge).target(), old_target);
}

#[test]
fn exit_blocks_iterator() {
    let mut cfg = Cfg::<MockInst>::new();
    let b1 = cfg.new_block();
    cfg.add_edge(cfg.entry(), b1, EdgeKind::Fallthrough);
    // b1 has no outgoing edges — it's an exit block.
    let exits: Vec<BlockId> = cfg.exit_blocks().collect();
    assert_eq!(exits, vec![b1]);
}
