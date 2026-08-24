//! Edge-aware traversals over owned graphs and borrowed edge views.

extern crate alloc;
use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

use super::directed::{DirectedEdge, DirectedGraph, EdgeId, NodeId};
use super::edge_view::{DenseEdgeId, EdgeGraphView, EdgeRef};
use super::traverse::TraversalDirection;
use super::view::DenseNodeId;

/// One traversed edge, with endpoints as exposed by the traversed view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeStep<N = NodeId, E = EdgeId> {
    /// The edge traversed.
    pub edge: E,
    /// The edge source in this view.
    pub source: N,
    /// The edge target in this view.
    pub target: N,
}

/// Breadth-first edge traversal over any edge-aware graph view.
#[must_use]
pub fn breadth_first_view_edges<G: EdgeGraphView>(
    graph: &G,
    start: G::NodeId,
    direction: TraversalDirection,
) -> Vec<EdgeStep<G::NodeId, G::EdgeId>> {
    walk_view_edges(graph, start, direction, None, |_| true)
}

/// Breadth-first edge traversal with a predicate and optional depth bound.
///
/// Every accepted edge leaving a reached node in `direction` is reported once
/// in adjacency order, including parallel edges and edges to visited nodes.
/// Rejected edges are neither reported nor traversed. This operates directly
/// on [`crate::FilteredEdges`] and other borrowed views without rebuilding a
/// graph.
///
/// # Panics
///
/// Panics when `start` is outside the view or the view violates its dense
/// node/edge identity contract.
#[must_use]
pub fn walk_view_edges<G: EdgeGraphView>(
    graph: &G,
    start: G::NodeId,
    direction: TraversalDirection,
    max_depth: Option<usize>,
    mut filter: impl FnMut(EdgeRef<'_, G::NodeId, G::EdgeId, G::EdgeData>) -> bool,
) -> Vec<EdgeStep<G::NodeId, G::EdgeId>> {
    assert!(
        start.index() < graph.node_count(),
        "start node is out of range"
    );
    let forward = matches!(direction, TraversalDirection::Outgoing);
    let mut steps = Vec::new();
    let mut seen_node = vec![false; graph.node_count()];
    let mut seen_edge = vec![false; graph.edge_slot_count()];
    let mut queue = VecDeque::new();
    seen_node[start.index()] = true;
    queue.push_back((start, 0));

    while let Some((node, depth)) = queue.pop_front() {
        if max_depth.is_some_and(|limit| depth >= limit) {
            continue;
        }
        let adjacency: Vec<_> = if forward {
            graph.outgoing_edges(node).collect()
        } else {
            graph.incoming_edges(node).collect()
        };
        for edge_id in adjacency {
            if seen_edge[edge_id.index()] {
                continue;
            }
            let edge = graph.edge_ref(edge_id);
            if !filter(edge) {
                continue;
            }
            seen_edge[edge_id.index()] = true;
            steps.push(EdgeStep {
                edge: edge_id,
                source: edge.source(),
                target: edge.target(),
            });
            let next = if forward {
                edge.target()
            } else {
                edge.source()
            };
            if !seen_node[next.index()] {
                seen_node[next.index()] = true;
                queue.push_back((next, depth + 1));
            }
        }
    }
    steps
}

/// The edges of one shortest path through an edge-aware view.
///
/// # Panics
///
/// Panics when either endpoint is outside the view or the view violates its
/// dense node/edge identity contract.
#[must_use]
pub fn shortest_path_view_edges<G: EdgeGraphView>(
    graph: &G,
    from: G::NodeId,
    to: G::NodeId,
    direction: TraversalDirection,
) -> Option<Vec<G::EdgeId>> {
    assert!(
        from.index() < graph.node_count(),
        "source node is out of range"
    );
    assert!(
        to.index() < graph.node_count(),
        "target node is out of range"
    );
    if from == to {
        return Some(Vec::new());
    }
    let forward = matches!(direction, TraversalDirection::Outgoing);
    let mut parent_edge = vec![None; graph.node_count()];
    let mut seen = vec![false; graph.node_count()];
    let mut queue = VecDeque::new();
    seen[from.index()] = true;
    queue.push_back(from);

    'search: while let Some(node) = queue.pop_front() {
        let adjacency: Vec<_> = if forward {
            graph.outgoing_edges(node).collect()
        } else {
            graph.incoming_edges(node).collect()
        };
        for edge_id in adjacency {
            let edge = graph.edge_ref(edge_id);
            let next = if forward {
                edge.target()
            } else {
                edge.source()
            };
            if seen[next.index()] {
                continue;
            }
            seen[next.index()] = true;
            parent_edge[next.index()] = Some(edge_id);
            if next == to {
                break 'search;
            }
            queue.push_back(next);
        }
    }

    parent_edge[to.index()]?;
    let mut path = Vec::new();
    let mut current = to;
    while let Some(edge_id) = parent_edge[current.index()] {
        path.push(edge_id);
        let edge = graph.edge_ref(edge_id);
        current = if forward {
            edge.source()
        } else {
            edge.target()
        };
    }
    path.reverse();
    Some(path)
}

/// Breadth-first edge traversal over owned [`DirectedGraph`] storage.
#[must_use]
pub fn breadth_first_edges<N, E>(
    graph: &DirectedGraph<N, E>,
    start: NodeId,
    direction: TraversalDirection,
) -> Vec<EdgeStep> {
    breadth_first_view_edges(graph, start, direction)
}

/// Compatibility wrapper retaining the owned-graph edge predicate API.
#[must_use]
pub fn walk_edges<N, E>(
    graph: &DirectedGraph<N, E>,
    start: NodeId,
    direction: TraversalDirection,
    max_depth: Option<usize>,
    mut filter: impl FnMut(&DirectedEdge<E>) -> bool,
) -> Vec<EdgeStep> {
    walk_view_edges(graph, start, direction, max_depth, |edge| {
        filter(graph.edge(edge.id()))
    })
}

/// Compatibility wrapper for a shortest path through an owned graph.
#[must_use]
pub fn shortest_path_edges<N, E>(
    graph: &DirectedGraph<N, E>,
    from: NodeId,
    to: NodeId,
    direction: TraversalDirection,
) -> Option<Vec<EdgeId>> {
    shortest_path_view_edges(graph, from, to, direction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FilteredEdges, Rooted};

    fn fixture() -> (DirectedGraph<&'static str, &'static str>, [NodeId; 3]) {
        let mut graph = DirectedGraph::new();
        let a = graph.add_node("a");
        let b = graph.add_node("b");
        let c = graph.add_node("c");
        graph.add_edge(a, b, "x");
        graph.add_edge(b, c, "y");
        graph.add_edge(a, b, "z");
        graph.add_edge(c, a, "cycle");
        (graph, [a, b, c])
    }

    #[test]
    fn edge_bfs_reports_parallel_and_cycle_edges_once() {
        let (graph, [a, _, _]) = fixture();
        let steps = breadth_first_edges(&graph, a, TraversalDirection::Outgoing);
        assert_eq!(steps.len(), 4);
        let payloads: Vec<_> = steps
            .iter()
            .map(|step| *graph.edge(step.edge).payload())
            .collect();
        assert_eq!(payloads, ["x", "z", "y", "cycle"]);
    }

    #[test]
    fn filtered_view_walk_never_rebuilds_or_renumbers() {
        let (graph, [a, _, c]) = fixture();
        let rooted = Rooted::new(&graph, a);
        let filtered = FilteredEdges::new(&rooted, |_, payload: &&str| *payload != "z");
        let steps = breadth_first_view_edges(&filtered, a, TraversalDirection::Outgoing);
        let payloads: Vec<_> = steps
            .iter()
            .map(|step| *filtered.edge_ref(step.edge).data())
            .collect();
        assert_eq!(payloads, ["x", "y", "cycle"]);
        assert_eq!(
            shortest_path_view_edges(&filtered, a, c, TraversalDirection::Outgoing)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn depth_and_predicate_filters_remain_compatible() {
        let (graph, [a, _, _]) = fixture();
        let steps = walk_edges(&graph, a, TraversalDirection::Outgoing, Some(1), |edge| {
            *edge.payload() != "z"
        });
        assert_eq!(steps.len(), 1);
        assert_eq!(*graph.edge(steps[0].edge).payload(), "x");
    }
}
