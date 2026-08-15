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

/// Hop counts from `start` to every node reachable by walking `direction`
/// edges, `None` for the unreachable ones. `start` itself is at distance 0.
fn breadth_first_distances<G: DirectedGraphView>(
    graph: &G,
    start: G::NodeId,
    direction: TraversalDirection,
) -> Vec<Option<usize>> {
    let mut distances = vec![None; graph.node_count()];
    let mut queue = VecDeque::new();
    distances[start.index()] = Some(0);
    queue.push_back((start, 0_usize));

    while let Some((node, depth)) = queue.pop_front() {
        for adjacent in neighbors(graph, node, direction) {
            if distances[adjacent.index()].is_none() {
                distances[adjacent.index()] = Some(depth + 1);
                queue.push_back((adjacent, depth + 1));
            }
        }
    }

    distances
}

/// Return the nearest node reachable from both `a` and `b` by walking
/// `direction` edges — the meeting point of two outward searches.
///
/// With [`TraversalDirection::Incoming`] this is the classic *nearest common
/// ancestor*: the closest node from which both `a` and `b` can be reached
/// (a shared dominator-like join in a CFG, a shared caller in a call graph, a
/// shared origin in a value-flow graph). With
/// [`TraversalDirection::Outgoing`] it is the mirror question — the closest
/// node both can reach, such as a shared sink or merge point. The graph need
/// not be a tree or a DAG; cycles terminate the searches like every other
/// traversal here.
///
/// Both endpoints are at distance 0 from themselves, so `b` is the answer
/// whenever it is reachable from `a` at all, and `a == b` answers `a`.
/// Returns `None` when no node is reachable from both.
///
/// # Determinism
///
/// Candidates are ranked by **smallest combined distance first; ties broken
/// by smallest node id.** The combined distance of a candidate is its hop
/// count from `a` plus its hop count from `b`. The answer therefore never
/// depends on adjacency order or on which endpoint was passed first.
///
/// # Examples
///
/// ```
/// use cfglib::{DirectedGraph, TraversalDirection, nearest_common_ancestor};
///
/// //   root         `left` and `right` share one predecessor
/// //   /  \
/// // left right
/// let mut graph = DirectedGraph::<&str, ()>::new();
/// let root = graph.add_node("root");
/// let left = graph.add_node("left");
/// let right = graph.add_node("right");
/// graph.add_edge(root, left, ());
/// graph.add_edge(root, right, ());
///
/// // Walking predecessors from both leaves meets at the root.
/// assert_eq!(
///     nearest_common_ancestor(&graph, left, right, TraversalDirection::Incoming),
///     Some(root)
/// );
///
/// // Forward, `left` is reachable from `root` and from itself, so the meet
/// // is `left` at a combined distance of 1.
/// assert_eq!(
///     nearest_common_ancestor(&graph, root, left, TraversalDirection::Outgoing),
///     Some(left)
/// );
///
/// // Two leaves have no common successor.
/// assert_eq!(
///     nearest_common_ancestor(&graph, left, right, TraversalDirection::Outgoing),
///     None
/// );
/// ```
///
/// # Panics
///
/// Panics when either endpoint is not a node in `graph`.
#[must_use]
pub fn nearest_common_ancestor<G: DirectedGraphView>(
    graph: &G,
    a: G::NodeId,
    b: G::NodeId,
    direction: TraversalDirection,
) -> Option<G::NodeId> {
    assert!(a.index() < graph.node_count(), "node `a` is out of range");
    assert!(b.index() < graph.node_count(), "node `b` is out of range");
    let from_a = breadth_first_distances(graph, a, direction);
    let from_b = breadth_first_distances(graph, b, direction);

    // `min` over `(combined distance, node id)` *is* the documented
    // tie-break: node ids are dense and ordered, so the tuple ordering
    // ranks by distance and settles ties on the smaller id.
    graph
        .node_ids()
        .filter_map(|node| {
            let reached_from_a = from_a[node.index()]?;
            let reached_from_b = from_b[node.index()]?;
            Some((reached_from_a + reached_from_b, node))
        })
        .min()
        .map(|(_, node)| node)
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

    /// `root -> mid`, `mid -> left`, `mid -> right`, both legs into `bottom`.
    /// `root` has the smallest id but is the *farther* common ancestor.
    fn diamond() -> (DirectedGraph<(), ()>, [NodeId; 5]) {
        let mut graph = DirectedGraph::<(), ()>::new();
        let root = graph.add_node(());
        let mid = graph.add_node(());
        let left = graph.add_node(());
        let right = graph.add_node(());
        let bottom = graph.add_node(());
        graph.add_edge(root, mid, ());
        graph.add_edge(mid, left, ());
        graph.add_edge(mid, right, ());
        graph.add_edge(left, bottom, ());
        graph.add_edge(right, bottom, ());
        (graph, [root, mid, left, right, bottom])
    }

    #[test]
    fn nearest_common_ancestor_meets_at_the_closest_shared_node() {
        let (graph, [_root, mid, left, right, bottom]) = diamond();
        // `mid` (combined 2) beats `root` (combined 4) even though `root`
        // has the smaller id: distance ranks first.
        assert_eq!(
            nearest_common_ancestor(&graph, left, right, TraversalDirection::Incoming),
            Some(mid)
        );
        // Forward, the same two legs merge at the bottom.
        assert_eq!(
            nearest_common_ancestor(&graph, left, right, TraversalDirection::Outgoing),
            Some(bottom)
        );
        // The answer does not depend on which endpoint is passed first.
        assert_eq!(
            nearest_common_ancestor(&graph, right, left, TraversalDirection::Incoming),
            Some(mid)
        );
        assert_eq!(
            nearest_common_ancestor(&graph, bottom, mid, TraversalDirection::Incoming),
            Some(mid)
        );
    }

    #[test]
    fn nearest_common_ancestor_treats_endpoints_as_distance_zero() {
        let (graph, [root, _, left, _, bottom]) = diamond();
        // A node is its own meet, in either direction.
        assert_eq!(
            nearest_common_ancestor(&graph, root, root, TraversalDirection::Outgoing),
            Some(root)
        );
        assert_eq!(
            nearest_common_ancestor(&graph, bottom, bottom, TraversalDirection::Incoming),
            Some(bottom)
        );
        // `left` is reachable from `root`, so the meet is `left` itself.
        assert_eq!(
            nearest_common_ancestor(&graph, root, left, TraversalDirection::Outgoing),
            Some(left)
        );
        assert_eq!(
            nearest_common_ancestor(&graph, left, root, TraversalDirection::Outgoing),
            Some(left)
        );
    }

    #[test]
    fn nearest_common_ancestor_breaks_ties_by_smallest_node_id() {
        let mut graph = DirectedGraph::<(), ()>::new();
        let first_sink = graph.add_node(());
        let second_sink = graph.add_node(());
        let left = graph.add_node(());
        let right = graph.add_node(());
        // Adjacency deliberately offers the higher-id sink first, so a
        // discovery-order answer would pick `second_sink`.
        graph.add_edge(left, second_sink, ());
        graph.add_edge(left, first_sink, ());
        graph.add_edge(right, second_sink, ());
        graph.add_edge(right, first_sink, ());

        // Both sinks sit at combined distance 2; the smaller id wins.
        assert_eq!(
            nearest_common_ancestor(&graph, left, right, TraversalDirection::Outgoing),
            Some(first_sink)
        );
        assert_eq!(
            nearest_common_ancestor(&graph, right, left, TraversalDirection::Outgoing),
            Some(first_sink)
        );
        assert_eq!(
            nearest_common_ancestor(
                &graph,
                first_sink,
                second_sink,
                TraversalDirection::Incoming
            ),
            Some(left)
        );
    }

    #[test]
    fn nearest_common_ancestor_without_a_shared_node_is_none() {
        let mut graph = DirectedGraph::<(), ()>::new();
        let start = graph.add_node(());
        let lonely = graph.add_node(());
        let end = graph.add_node(());
        graph.add_edge(start, end, ());

        assert_eq!(
            nearest_common_ancestor(&graph, start, lonely, TraversalDirection::Outgoing),
            None
        );
        assert_eq!(
            nearest_common_ancestor(&graph, start, lonely, TraversalDirection::Incoming),
            None
        );
        // Two nodes with no shared successor, both inside the connected part.
        assert_eq!(
            nearest_common_ancestor(&graph, end, lonely, TraversalDirection::Incoming),
            None
        );
    }

    #[test]
    fn nearest_common_ancestor_terminates_on_cycles() {
        let (mut graph, [_, mid, left, right, bottom]) = diamond();
        graph.add_edge(bottom, mid, ());

        // Every node is now reachable from both legs; `bottom` (1 + 1) is
        // the closest forward meet and `mid` (1 + 1) the closest backward one.
        assert_eq!(
            nearest_common_ancestor(&graph, left, right, TraversalDirection::Outgoing),
            Some(bottom)
        );
        assert_eq!(
            nearest_common_ancestor(&graph, left, right, TraversalDirection::Incoming),
            Some(mid)
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
