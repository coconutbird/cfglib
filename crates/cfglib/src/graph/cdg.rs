//! Control-dependence analysis on the generic directed-graph substrate.
//!
//! Node `B` is control-dependent on node `A` when `A` selects whether `B`
//! executes. The resulting graph has one node per source node and an edge
//! `A -> B` for that relation; node payloads retain the original identity.

extern crate alloc;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::graph::directed::DirectedGraph;
use crate::graph::dominator::DominatorTree;
use crate::graph::view::{DenseNodeId, DirectedGraphView};

/// Compute control dependences from a graph view and its post-dominator
/// tree.
///
/// The returned node identities are allocated in source node order, so a
/// source node and its graph node have the same dense index. The node
/// payload remains the authoritative link back to the source graph.
///
/// # Examples
///
/// ```
/// use cfglib::{
///     Cfg, DominatorTree, EdgeKind, NodeId, control_dependence_graph,
/// };
///
/// let mut cfg = Cfg::<u32>::new();
/// let branch = cfg.entry();
/// let left = cfg.new_block();
/// let right = cfg.new_block();
/// let merge = cfg.new_block();
/// cfg.add_edge(branch, left, EdgeKind::ConditionalTrue);
/// cfg.add_edge(branch, right, EdgeKind::ConditionalFalse);
/// cfg.add_edge(left, merge, EdgeKind::Fallthrough);
/// cfg.add_edge(right, merge, EdgeKind::Fallthrough);
///
/// let post_dominators = DominatorTree::compute_post(&cfg);
/// let dependences = control_dependence_graph(&cfg, &post_dominators);
/// let branch_node = NodeId::from_index(branch.index());
/// let controlled: Vec<_> = dependences
///     .successors(branch_node)
///     .map(|node| dependences[node])
///     .collect();
/// assert!(controlled.contains(&left));
/// assert!(controlled.contains(&right));
/// ```
#[must_use]
pub fn control_dependence_graph<G: DirectedGraphView>(
    source: &G,
    post_dominators: &DominatorTree<G::NodeId>,
) -> DirectedGraph<G::NodeId, ()> {
    let mut graph = DirectedGraph::with_capacity(source.node_count(), source.node_count());
    let nodes: Vec<_> = source.node_ids().map(|node| graph.add_node(node)).collect();
    let mut dependences = BTreeSet::new();

    for controller in source.node_ids() {
        // Control dependence is defined through post-dominance; a node the
        // post-dominator computation never reached (it cannot reach any
        // exit) has NO post-dominance facts, and emitting its edges would
        // fabricate "A selects whether B executes" claims for straight-line
        // code in exit-unreachable regions.
        if !post_dominators.is_reachable(controller) {
            continue;
        }
        for target in source.successors(controller) {
            if !post_dominators.is_reachable(target) {
                continue;
            }
            if post_dominators.dominates(target, controller) {
                continue;
            }

            let immediate_post_dominator = post_dominators.idom(controller);
            let mut dependent = target;
            loop {
                dependences.insert((controller, dependent));
                match post_dominators.idom(dependent) {
                    Some(next) if Some(next) != immediate_post_dominator => dependent = next,
                    _ => break,
                }
            }
        }
    }

    for (controller, dependent) in dependences {
        graph.add_edge(nodes[controller.index()], nodes[dependent.index()], ());
    }
    graph
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockId;
    use crate::cfg::Cfg;
    use crate::edge::EdgeKind;
    use crate::graph::directed::NodeId;
    use crate::test_util::ff;

    fn node(block: BlockId) -> NodeId {
        NodeId::from_index(block.index())
    }

    #[test]
    fn diamond_control_dependences_are_edges() {
        let mut cfg = Cfg::new();
        let left = cfg.new_block();
        let right = cfg.new_block();
        let merge = cfg.new_block();
        cfg.block_mut(cfg.entry()).push(ff("entry"));
        cfg.block_mut(left).push(ff("left"));
        cfg.block_mut(right).push(ff("right"));
        cfg.block_mut(merge).push(ff("merge"));
        cfg.add_edge(cfg.entry(), left, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), right, EdgeKind::ConditionalFalse);
        cfg.add_edge(left, merge, EdgeKind::Fallthrough);
        cfg.add_edge(right, merge, EdgeKind::Fallthrough);

        let post_dominators = DominatorTree::compute_post(&cfg);
        let graph = control_dependence_graph(&cfg, &post_dominators);
        let controlled: BTreeSet<_> = graph
            .successors(node(cfg.entry()))
            .map(|dependent| graph[dependent])
            .collect();

        assert_eq!(controlled, BTreeSet::from([left, right]));
        assert_eq!(graph.predecessors(node(merge)).count(), 0);
    }

    #[test]
    fn exit_unreachable_regions_emit_no_dependences() {
        // With no exits, no node has post-dominance facts; straight-line
        // edges must not become fabricated control dependences.
        use crate::graph::directed::DirectedGraph;
        let mut graph: DirectedGraph<(), ()> = DirectedGraph::new();
        let a = graph.add_node(());
        let b = graph.add_node(());
        graph.add_edge(a, b, ());

        let post = DominatorTree::compute_post_from(&graph, &[]);
        let cdg = control_dependence_graph(&graph, &post);
        assert_eq!(cdg.edge_count(), 0);
    }

    #[test]
    fn linear_cfg_has_no_control_dependence_edges() {
        let mut cfg = Cfg::new();
        let next = cfg.new_block();
        cfg.block_mut(cfg.entry()).push(ff("entry"));
        cfg.block_mut(next).push(ff("next"));
        cfg.add_edge(cfg.entry(), next, EdgeKind::Fallthrough);

        let post_dominators = DominatorTree::compute_post(&cfg);
        let graph = control_dependence_graph(&cfg, &post_dominators);

        assert_eq!(graph.node_count(), cfg.num_blocks());
        assert_eq!(graph.edge_count(), 0);
    }
}
