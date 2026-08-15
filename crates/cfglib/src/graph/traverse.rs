//! Generic directed-graph traversals and ordering algorithms.

extern crate alloc;
use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::graph::view::{DenseNodeId, DirectedGraphView};

/// Direction in which a graph traversal follows edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraversalDirection {
    /// Follow edges from source to target.
    Outgoing,
    /// Follow edges from target to source.
    Incoming,
}

fn neighbors<G: DirectedGraphView>(
    graph: &G,
    node: G::NodeId,
    direction: TraversalDirection,
) -> Vec<G::NodeId> {
    match direction {
        TraversalDirection::Outgoing => graph.successors(node).collect(),
        TraversalDirection::Incoming => graph.predecessors(node).collect(),
    }
}

/// Return nodes in depth-first preorder from `start`.
///
/// # Panics
///
/// Panics when `start` is not a node in `graph`.
#[must_use]
pub fn depth_first_preorder<G: DirectedGraphView>(
    graph: &G,
    start: G::NodeId,
    direction: TraversalDirection,
) -> Vec<G::NodeId> {
    assert!(
        start.index() < graph.node_count(),
        "start node is out of range"
    );
    let mut visited = vec![false; graph.node_count()];
    let mut order = Vec::with_capacity(graph.node_count());
    let mut stack = vec![start];

    while let Some(node) = stack.pop() {
        if visited[node.index()] {
            continue;
        }
        visited[node.index()] = true;
        order.push(node);

        for successor in neighbors(graph, node, direction).into_iter().rev() {
            if !visited[successor.index()] {
                stack.push(successor);
            }
        }
    }

    order
}

/// Return nodes in depth-first postorder from `start`.
///
/// # Panics
///
/// Panics when `start` is not a node in `graph`.
#[must_use]
pub fn depth_first_postorder<G: DirectedGraphView>(
    graph: &G,
    start: G::NodeId,
    direction: TraversalDirection,
) -> Vec<G::NodeId> {
    assert!(
        start.index() < graph.node_count(),
        "start node is out of range"
    );
    let mut visited = vec![false; graph.node_count()];
    let mut order = Vec::with_capacity(graph.node_count());
    let mut stack = vec![(start, false)];

    while let Some((node, processed)) = stack.pop() {
        if processed {
            order.push(node);
            continue;
        }
        if visited[node.index()] {
            continue;
        }

        visited[node.index()] = true;
        stack.push((node, true));
        for successor in neighbors(graph, node, direction).into_iter().rev() {
            if !visited[successor.index()] {
                stack.push((successor, false));
            }
        }
    }

    order
}

/// Return reverse postorder from `start`.
#[must_use]
pub fn reverse_postorder<G: DirectedGraphView>(
    graph: &G,
    start: G::NodeId,
    direction: TraversalDirection,
) -> Vec<G::NodeId> {
    let mut order = depth_first_postorder(graph, start, direction);
    order.reverse();
    order
}

/// Return nodes in breadth-first order from `start`.
///
/// # Panics
///
/// Panics when `start` is not a node in `graph`.
#[must_use]
pub fn breadth_first<G: DirectedGraphView>(
    graph: &G,
    start: G::NodeId,
    direction: TraversalDirection,
) -> Vec<G::NodeId> {
    assert!(
        start.index() < graph.node_count(),
        "start node is out of range"
    );
    let mut visited = vec![false; graph.node_count()];
    let mut order = Vec::with_capacity(graph.node_count());
    let mut queue = VecDeque::new();
    visited[start.index()] = true;
    queue.push_back(start);

    while let Some(node) = queue.pop_front() {
        order.push(node);
        for adjacent in neighbors(graph, node, direction) {
            if !visited[adjacent.index()] {
                visited[adjacent.index()] = true;
                queue.push_back(adjacent);
            }
        }
    }

    order
}

/// Return a shortest unweighted path from `start` to `goal`.
///
/// The result includes both endpoints. Parallel edges do not affect the path.
///
/// # Panics
///
/// Panics when either endpoint is not a node in `graph`.
#[must_use]
pub fn shortest_path<G: DirectedGraphView>(
    graph: &G,
    start: G::NodeId,
    goal: G::NodeId,
    direction: TraversalDirection,
) -> Option<Vec<G::NodeId>> {
    assert!(
        start.index() < graph.node_count(),
        "start node is out of range"
    );
    assert!(
        goal.index() < graph.node_count(),
        "goal node is out of range"
    );
    let mut previous = vec![None; graph.node_count()];
    let mut visited = vec![false; graph.node_count()];
    let mut queue = VecDeque::new();
    visited[start.index()] = true;
    queue.push_back(start);

    while let Some(node) = queue.pop_front() {
        if node == goal {
            let mut path = vec![goal];
            let mut current = goal;
            while current != start {
                current = previous[current.index()]?;
                path.push(current);
            }
            path.reverse();
            return Some(path);
        }

        for adjacent in neighbors(graph, node, direction) {
            if !visited[adjacent.index()] {
                visited[adjacent.index()] = true;
                previous[adjacent.index()] = Some(node);
                queue.push_back(adjacent);
            }
        }
    }

    None
}

/// Mark every node reachable from `seeds` by walking `direction` edges.
///
/// Returns a dense `Vec<bool>` indexed by node id, `true` for every seed and
/// every node reachable from one. Duplicate seeds are fine. The result is a
/// set, so it is deterministic and independent of seed order — unlike the
/// order-yielding traversals above, which answer a single-start question.
///
/// Multi-source reachability is the shape most whole-program queries take:
/// live code from a set of roots, the callees of an entry-point set, the
/// nodes a set of definitions can flow to, and (with
/// [`TraversalDirection::Incoming`]) everything that can reach a set of
/// sinks.
///
/// # Examples
///
/// ```
/// use cfglib::{DirectedGraph, TraversalDirection, reachable};
///
/// let mut graph = DirectedGraph::<&str, ()>::new();
/// let main = graph.add_node("main");
/// let helper = graph.add_node("helper");
/// let orphan = graph.add_node("orphan");
/// graph.add_edge(main, helper, ());
///
/// let live = reachable(&graph, [main], TraversalDirection::Outgoing);
/// assert_eq!(live, vec![true, true, false]);
///
/// // Seeding the orphan too covers the whole graph.
/// let live = reachable(&graph, [main, orphan], TraversalDirection::Outgoing);
/// assert_eq!(live, vec![true, true, true]);
/// ```
///
/// # Panics
///
/// Panics when a seed is not a node in `graph`.
#[must_use]
pub fn reachable<G: DirectedGraphView>(
    graph: &G,
    seeds: impl IntoIterator<Item = G::NodeId>,
    direction: TraversalDirection,
) -> Vec<bool> {
    let mut visited = vec![false; graph.node_count()];
    let mut stack = Vec::new();

    for seed in seeds {
        assert!(
            seed.index() < graph.node_count(),
            "seed node is out of range"
        );
        if !visited[seed.index()] {
            visited[seed.index()] = true;
            stack.push(seed);
        }
    }

    while let Some(node) = stack.pop() {
        for adjacent in neighbors(graph, node, direction) {
            if !visited[adjacent.index()] {
                visited[adjacent.index()] = true;
                stack.push(adjacent);
            }
        }
    }

    visited
}

/// Return a topological ordering, or `None` when the graph contains a cycle.
#[must_use]
pub fn topological_sort<G: DirectedGraphView>(graph: &G) -> Option<Vec<G::NodeId>> {
    let mut incoming_counts = vec![0_usize; graph.node_count()];
    for node in graph.node_ids() {
        incoming_counts[node.index()] = graph.predecessors(node).count();
    }

    let mut queue: VecDeque<G::NodeId> = graph
        .node_ids()
        .filter(|node| incoming_counts[node.index()] == 0)
        .collect();
    let mut order = Vec::with_capacity(graph.node_count());

    while let Some(node) = queue.pop_front() {
        order.push(node);
        for successor in graph.successors(node) {
            let count = &mut incoming_counts[successor.index()];
            *count -= 1;
            if *count == 0 {
                queue.push_back(successor);
            }
        }
    }

    (order.len() == graph.node_count()).then_some(order)
}

impl<I> Cfg<I> {
    /// Depth-first preorder traversal starting from the entry block.
    #[must_use]
    pub fn dfs_preorder(&self) -> Vec<BlockId> {
        depth_first_preorder(self, self.entry(), TraversalDirection::Outgoing)
    }

    /// Depth-first postorder traversal starting from the entry block.
    #[must_use]
    pub fn dfs_postorder(&self) -> Vec<BlockId> {
        depth_first_postorder(self, self.entry(), TraversalDirection::Outgoing)
    }

    /// Reverse postorder starting from the entry block.
    #[must_use]
    pub fn reverse_postorder(&self) -> Vec<BlockId> {
        reverse_postorder(self, self.entry(), TraversalDirection::Outgoing)
    }

    /// Breadth-first traversal starting from the entry block.
    #[must_use]
    pub fn bfs(&self) -> Vec<BlockId> {
        breadth_first(self, self.entry(), TraversalDirection::Outgoing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::EdgeKind;
    use crate::graph::directed::{DirectedGraph, NodeId};
    use crate::test_util::ff;
    use alloc::vec;

    #[test]
    fn cfg_traversal_methods_delegate_to_generic_algorithms() {
        let mut cfg = Cfg::new();
        let middle = cfg.new_block();
        let last = cfg.new_block();
        cfg.block_mut(cfg.entry()).push(ff("entry"));
        cfg.block_mut(middle).push(ff("middle"));
        cfg.block_mut(last).push(ff("last"));
        cfg.add_edge(cfg.entry(), middle, EdgeKind::Fallthrough);
        cfg.add_edge(middle, last, EdgeKind::Fallthrough);

        assert_eq!(cfg.dfs_preorder(), vec![cfg.entry(), middle, last]);
        assert_eq!(cfg.dfs_postorder(), vec![last, middle, cfg.entry()]);
        assert_eq!(cfg.reverse_postorder(), vec![cfg.entry(), middle, last]);
        assert_eq!(cfg.bfs(), vec![cfg.entry(), middle, last]);
    }

    #[test]
    fn directed_graph_can_be_walked_in_both_directions() {
        let mut graph = DirectedGraph::<&str, ()>::new();
        let first = graph.add_node("first");
        let second = graph.add_node("second");
        let third = graph.add_node("third");
        graph.add_edge(first, second, ());
        graph.add_edge(second, third, ());

        assert_eq!(
            breadth_first(&graph, first, TraversalDirection::Outgoing),
            vec![first, second, third]
        );
        assert_eq!(
            breadth_first(&graph, third, TraversalDirection::Incoming),
            vec![third, second, first]
        );
        assert_eq!(
            shortest_path(&graph, first, third, TraversalDirection::Outgoing),
            Some(vec![first, second, third])
        );
    }

    /// `a -> b -> c` with a `c -> b` back edge, plus a disconnected `d`.
    fn reach_fixture() -> (DirectedGraph<(), ()>, [NodeId; 4]) {
        let mut graph = DirectedGraph::<(), ()>::new();
        let a = graph.add_node(());
        let b = graph.add_node(());
        let c = graph.add_node(());
        let d = graph.add_node(());
        graph.add_edge(a, b, ());
        graph.add_edge(b, c, ());
        graph.add_edge(c, b, ());
        (graph, [a, b, c, d])
    }

    #[test]
    fn reachable_from_no_seeds_marks_nothing() {
        let (graph, _) = reach_fixture();
        assert_eq!(
            reachable(&graph, [], TraversalDirection::Outgoing),
            vec![false; 4]
        );

        // An empty graph yields an empty table rather than panicking.
        let empty = DirectedGraph::<(), ()>::new();
        assert!(reachable(&empty, [], TraversalDirection::Outgoing).is_empty());
    }

    #[test]
    fn reachable_unions_multiple_sources_and_terminates_on_cycles() {
        let (graph, [a, _, c, d]) = reach_fixture();
        // The b <-> c cycle terminates; d stays unreached.
        assert_eq!(
            reachable(&graph, [a], TraversalDirection::Outgoing),
            vec![true, true, true, false]
        );
        // A second seed unions in, and duplicate seeds change nothing.
        assert_eq!(
            reachable(&graph, [a, d, a, d], TraversalDirection::Outgoing),
            vec![true; 4]
        );
        // Order-insensitive: the answer is a set.
        assert_eq!(
            reachable(&graph, [d, c], TraversalDirection::Outgoing),
            reachable(&graph, [c, d], TraversalDirection::Outgoing)
        );
        assert_eq!(
            reachable(&graph, [c], TraversalDirection::Outgoing),
            vec![false, true, true, false]
        );
    }

    #[test]
    fn reachable_walks_predecessors_in_the_incoming_direction() {
        let (graph, [a, _, c, _]) = reach_fixture();
        assert_eq!(
            reachable(&graph, [c], TraversalDirection::Incoming),
            vec![true, true, true, false]
        );
        assert_eq!(
            reachable(&graph, [a], TraversalDirection::Incoming),
            vec![true, false, false, false]
        );
    }

    #[test]
    fn reachable_handles_self_loops() {
        let mut graph = DirectedGraph::<(), ()>::new();
        let only = graph.add_node(());
        let other = graph.add_node(());
        graph.add_edge(only, only, ());
        assert_eq!(
            reachable(&graph, [only], TraversalDirection::Outgoing),
            vec![true, false]
        );
        // A self-loop is not a reason to be reachable from elsewhere.
        assert_eq!(
            reachable(&graph, [other], TraversalDirection::Outgoing),
            vec![false, true]
        );
    }

    #[test]
    fn topological_sort_rejects_cycles() {
        let mut graph = DirectedGraph::<(), ()>::new();
        let left = graph.add_node(());
        let right = graph.add_node(());
        graph.add_edge(left, right, ());
        assert_eq!(topological_sort(&graph), Some(vec![left, right]));
        graph.add_edge(right, left, ());
        assert!(topological_sort(&graph).is_none());
    }
}
