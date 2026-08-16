//! Strongly connected components for any [`DirectedGraphView`].
//!
//! Both implementations are iterative — they do not consume the host call
//! stack for deeply nested code graphs — and compute the same partition. They
//! differ in how they **number** it, which is the whole reason to pick one:
//! [`tarjan_scc`] takes one `O(V + E)` pass and numbers leaves first, while
//! [`kosaraju_scc`] takes two and numbers sources first, for consumers that
//! must process a component before everything it reaches.

extern crate alloc;
use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;

use crate::graph::directed::{DirectedGraph, NodeId};
use crate::graph::view::{DenseNodeId, DirectedGraphView};

/// A maximal set of mutually reachable nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scc<N> {
    /// Nodes in this component.
    pub nodes: BTreeSet<N>,
}

impl<N: Copy + Ord> Scc<N> {
    /// Return whether this component contains one node.
    #[must_use]
    pub fn is_singleton(&self) -> bool {
        self.nodes.len() == 1
    }

    /// Return whether `node` belongs to this component.
    #[must_use]
    pub fn contains(&self, node: N) -> bool {
        self.nodes.contains(&node)
    }
}

/// Result of strongly connected component decomposition.
///
/// The partition is a property of the graph, but the **order** of
/// [`components`](Self::components) is a property of the algorithm that
/// produced it: [`tarjan_scc`] numbers them in reverse topological order
/// (leaves first), [`kosaraju_scc`] in topological order (sources first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SccResult<N> {
    /// The components, ordered by the producing algorithm's numbering
    /// contract — see [`tarjan_scc`] and [`kosaraju_scc`].
    pub components: Vec<Scc<N>>,
    component_of: Vec<usize>,
}

impl<N: DenseNodeId> SccResult<N> {
    /// Return the component index containing `node`.
    #[must_use]
    pub fn component_index(&self, node: N) -> usize {
        self.component_of[node.index()]
    }

    /// Return the component containing `node`.
    #[must_use]
    pub fn component(&self, node: N) -> &Scc<N> {
        &self.components[self.component_index(node)]
    }

    /// Return the number of components.
    #[must_use]
    pub fn len(&self) -> usize {
        self.components.len()
    }

    /// Return whether the decomposition contains no components.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }

    /// Return whether `graph` is acyclic.
    #[must_use]
    pub fn is_dag<G>(&self, graph: &G) -> bool
    where
        G: DirectedGraphView<NodeId = N>,
    {
        self.components.iter().all(|component| {
            if !component.is_singleton() {
                return false;
            }
            let Some(&node) = component.nodes.iter().next() else {
                return false;
            };
            !graph.successors(node).any(|successor| successor == node)
        })
    }
}

/// One frame of the explicit depth-first stack both decompositions walk on.
///
/// A frame's successors live in the walk's single arena rather than in a `Vec`
/// of its own: frames are pushed and popped strictly last in, first out, so
/// the frame on top of the stack always owns the arena's tail and a pop
/// truncates the arena back to where that frame's successors began. One
/// allocation for the whole decomposition, instead of one per node it enters.
///
/// The **numbering** each algorithm documents is untouched by construction:
/// the frame is handed the same successors, in the same adjacency order, read
/// left to right by the same advancing cursor. Only where they are stored
/// changes.
struct SccFrame<N> {
    node: N,
    /// Where this frame's successors start in the arena; the pop truncates to
    /// it.
    start: usize,
    /// The next successor to examine, as an arena index. The frame's
    /// successors are exhausted when it reaches the arena's length, since the
    /// top frame's region runs to the end.
    cursor: usize,
}

/// Push the frame for `node`, taking its successors onto the arena's tail.
fn enter<G: DirectedGraphView>(
    graph: &G,
    node: G::NodeId,
    arena: &mut Vec<G::NodeId>,
    calls: &mut Vec<SccFrame<G::NodeId>>,
) {
    let start = arena.len();
    arena.extend(graph.successors(node));
    calls.push(SccFrame {
        node,
        start,
        cursor: start,
    });
}

/// Collapse a graph to its component DAG.
///
/// One node per strongly connected component, carrying that component's
/// [`Scc`], in the decomposition's reverse topological order (leaves
/// first); one edge per pair of distinct components connected in the
/// source graph (deduplicated). The result is acyclic by construction, so
/// topological processing, condensed traces, and cycle summaries follow
/// directly.
#[must_use]
pub fn condensation<G: DirectedGraphView>(graph: &G) -> DirectedGraph<Scc<G::NodeId>, ()> {
    let components = tarjan_scc(graph);
    let mut condensed = DirectedGraph::with_capacity(components.len(), components.len());
    let ids: Vec<NodeId> = components
        .components
        .iter()
        .map(|component| condensed.add_node(component.clone()))
        .collect();

    let mut wired = BTreeSet::new();
    for node in graph.node_ids() {
        let from = components.component_index(node);
        for successor in graph.successors(node) {
            let to = components.component_index(successor);
            if from != to && wired.insert((from, to)) {
                condensed.add_edge(ids[from], ids[to], ());
            }
        }
    }
    condensed
}

/// Compute strongly connected components with Tarjan's algorithm, numbering
/// them in **reverse topological order of the condensation — leaves first**.
///
/// One pass, `O(V + E)`, and the right default. [`kosaraju_scc`] computes the
/// same partition with the opposite numbering (sources first) for consumers
/// that must process a component before its successors.
#[must_use]
pub fn tarjan_scc<G: DirectedGraphView>(graph: &G) -> SccResult<G::NodeId> {
    let node_count = graph.node_count();
    let mut next_index = 0_usize;
    let mut stack = Vec::new();
    let mut on_stack = vec![false; node_count];
    let mut indices = vec![usize::MAX; node_count];
    let mut lowlinks = vec![0_usize; node_count];
    let mut component_of = vec![0_usize; node_count];
    let mut components = Vec::new();
    // Every frame's successors, appended as the frame is pushed and truncated
    // away as it pops. Both are empty again when a root's walk ends, so one
    // allocation covers the whole decomposition rather than one per root.
    let mut arena: Vec<G::NodeId> = Vec::new();
    let mut calls: Vec<SccFrame<G::NodeId>> = Vec::new();

    for start in graph.node_ids() {
        if indices[start.index()] != usize::MAX {
            continue;
        }

        indices[start.index()] = next_index;
        lowlinks[start.index()] = next_index;
        next_index += 1;
        stack.push(start);
        on_stack[start.index()] = true;
        enter(graph, start, &mut arena, &mut calls);

        // Read frames through `last_mut` and copy the Copy fields out; the
        // successors are no longer among them, so no frame is ever cloned.
        while let Some(frame) = calls.last_mut() {
            let node = frame.node;
            if frame.cursor < arena.len() {
                let successor = arena[frame.cursor];
                frame.cursor += 1;

                if indices[successor.index()] == usize::MAX {
                    indices[successor.index()] = next_index;
                    lowlinks[successor.index()] = next_index;
                    next_index += 1;
                    stack.push(successor);
                    on_stack[successor.index()] = true;
                    enter(graph, successor, &mut arena, &mut calls);
                } else if on_stack[successor.index()] {
                    lowlinks[node.index()] = lowlinks[node.index()].min(indices[successor.index()]);
                }
                continue;
            }

            if lowlinks[node.index()] == indices[node.index()] {
                let mut nodes = BTreeSet::new();
                while let Some(member) = stack.pop() {
                    on_stack[member.index()] = false;
                    nodes.insert(member);
                    if member == node {
                        break;
                    }
                }

                let component_index = components.len();
                for member in &nodes {
                    component_of[member.index()] = component_index;
                }
                components.push(Scc { nodes });
            }

            let Some(finished) = calls.pop() else { break };
            arena.truncate(finished.start);
            if let Some(parent) = calls.last() {
                lowlinks[parent.node.index()] =
                    lowlinks[parent.node.index()].min(lowlinks[node.index()]);
            }
        }
    }

    SccResult {
        components,
        component_of,
    }
}

/// Compute strongly connected components with Kosaraju's algorithm, numbering
/// them in **topological order of the condensation — sources first**.
///
/// # Numbering contract
///
/// The numbering is the reason this exists beside [`tarjan_scc`], which
/// computes the same partition in one pass instead of two but numbers it
/// leaves-first. Here, for every edge `u -> v` of the graph:
///
/// ```text
/// component_index(u) <= component_index(v)
/// ```
///
/// with equality exactly when `u` and `v` are in the same component. So
/// walking [`components`](SccResult::components) in index order visits every
/// component only after every component that can reach it — the order a
/// forward closure over the condensation wants, and the order
/// [`condensation`] does *not* produce (it is built from [`tarjan_scc`], so
/// its nodes are leaves-first).
///
/// # Determinism
///
/// The exact sequence, not just its topological property, is part of the
/// contract: it is the classic two-pass algorithm, deterministic in the
/// graph's own orders.
///
/// 1. An iterative depth-first walk over roots in node-id order, taking each
///    node's successors in adjacency order, recording nodes by finish time.
/// 2. A walk of the reverse graph over those roots in reverse finish order,
///    minting one component per root that is not yet assigned.
///
/// A consumer whose work budget is keyed on the component sequence — a
/// grammar `FIRST`-set fixpoint over a symbol-dependency graph, where
/// processing a component before its dependents is what makes one pass
/// enough — depends on that, so it is stated rather than left to the
/// implementation.
///
/// # Examples
///
/// ```
/// use cfglib::{DirectedGraph, kosaraju_scc, tarjan_scc};
///
/// // entry -> (a <-> b) -> exit
/// let mut graph = DirectedGraph::<&str, ()>::new();
/// let entry = graph.add_node("entry");
/// let a = graph.add_node("a");
/// let b = graph.add_node("b");
/// let exit = graph.add_node("exit");
/// graph.add_edge(entry, a, ());
/// graph.add_edge(a, b, ());
/// graph.add_edge(b, a, ());
/// graph.add_edge(b, exit, ());
///
/// // Sources first: the entry's component is 0, the cycle 1, the exit 2.
/// let sources_first = kosaraju_scc(&graph);
/// assert_eq!(sources_first.component_index(entry), 0);
/// assert_eq!(sources_first.component_index(a), 1);
/// assert_eq!(sources_first.component_index(b), 1);
/// assert_eq!(sources_first.component_index(exit), 2);
///
/// // The same partition, numbered the other way round.
/// let leaves_first = tarjan_scc(&graph);
/// assert_eq!(leaves_first.component_index(entry), 2);
/// assert_eq!(leaves_first.component_index(exit), 0);
/// ```
#[must_use]
pub fn kosaraju_scc<G: DirectedGraphView>(graph: &G) -> SccResult<G::NodeId> {
    let node_count = graph.node_count();
    let mut visited = vec![false; node_count];
    let mut finish_order: Vec<G::NodeId> = Vec::with_capacity(node_count);
    // One arena and one frame stack for the whole pass, on the discipline
    // `SccFrame` documents.
    let mut arena: Vec<G::NodeId> = Vec::new();
    let mut calls: Vec<SccFrame<G::NodeId>> = Vec::new();

    // Pass 1: record nodes by finish time over the forward graph.
    for start in graph.node_ids() {
        if visited[start.index()] {
            continue;
        }
        visited[start.index()] = true;
        enter(graph, start, &mut arena, &mut calls);

        // Read frames through `last_mut` and copy the Copy fields out; the
        // successors are no longer among them, so no frame is ever cloned.
        while let Some(frame) = calls.last_mut() {
            if frame.cursor < arena.len() {
                let successor = arena[frame.cursor];
                frame.cursor += 1;
                if !visited[successor.index()] {
                    visited[successor.index()] = true;
                    enter(graph, successor, &mut arena, &mut calls);
                }
                continue;
            }

            finish_order.push(frame.node);
            let Some(finished) = calls.pop() else { break };
            arena.truncate(finished.start);
        }
    }

    // Pass 2: walk the reverse graph in reverse finish order. The first root
    // is in a source component of the condensation, and every root after it
    // is in a source component of what remains — which is what makes the
    // minting order topological.
    let mut component_of = vec![usize::MAX; node_count];
    let mut components: Vec<Scc<G::NodeId>> = Vec::new();
    let mut stack = Vec::new();

    for root in finish_order.into_iter().rev() {
        if component_of[root.index()] != usize::MAX {
            continue;
        }

        let index = components.len();
        let mut nodes = BTreeSet::new();
        component_of[root.index()] = index;
        stack.push(root);
        while let Some(node) = stack.pop() {
            nodes.insert(node);
            for predecessor in graph.predecessors(node) {
                if component_of[predecessor.index()] == usize::MAX {
                    component_of[predecessor.index()] = index;
                    stack.push(predecessor);
                }
            }
        }
        components.push(Scc { nodes });
    }

    SccResult {
        components,
        component_of,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::edge::EdgeKind;
    use crate::graph::directed::DirectedGraph;
    use crate::test_util::ff;

    #[test]
    fn cfg_uses_generic_scc_algorithm() {
        let mut cfg = Cfg::new();
        let next = cfg.new_block();
        cfg.block_mut(cfg.entry()).push(ff("entry"));
        cfg.block_mut(next).push(ff("next"));
        cfg.add_edge(cfg.entry(), next, EdgeKind::Fallthrough);

        let result = tarjan_scc(&cfg);
        assert_eq!(result.len(), 2);
        assert!(result.is_dag(&cfg));
    }

    #[test]
    fn condensation_collapses_cycles_to_a_dag() {
        // entry -> (a <-> b) -> exit
        let mut graph = DirectedGraph::<&str, ()>::new();
        let entry = graph.add_node("entry");
        let a = graph.add_node("a");
        let b = graph.add_node("b");
        let exit = graph.add_node("exit");
        graph.add_edge(entry, a, ());
        graph.add_edge(a, b, ());
        graph.add_edge(b, a, ());
        graph.add_edge(b, exit, ());

        let condensed = condensation(&graph);
        assert_eq!(condensed.node_count(), 3);
        assert_eq!(condensed.edge_count(), 2, "cycle-internal edges collapse");
        assert!(tarjan_scc(&condensed).is_dag(&condensed));
        let cycle_component = condensed
            .node_ids()
            .find(|&node| condensed[node].nodes.len() == 2)
            .expect("the a/b component");
        assert!(condensed[cycle_component].contains(a));
        assert!(condensed[cycle_component].contains(b));
    }

    #[test]
    fn directed_graph_cycle_forms_one_component() {
        let mut graph = DirectedGraph::<&str, ()>::new();
        let left = graph.add_node("left");
        let right = graph.add_node("right");
        graph.add_edge(left, right, ());
        graph.add_edge(right, left, ());

        let result = tarjan_scc(&graph);
        assert_eq!(result.len(), 1);
        assert!(result.component(left).contains(right));
        assert_eq!(result.component_index(left), result.component_index(right));
        assert!(!result.is_dag(&graph));
    }

    /// Shapes both algorithms must agree on: a chain, a diamond, a cycle with
    /// a tail, two cycles in series, a self-loop beside a plain node, a
    /// disconnected pair, and the empty graph.
    fn fixtures() -> Vec<(&'static str, DirectedGraph<(), ()>)> {
        fn build(node_count: usize, edges: &[(usize, usize)]) -> DirectedGraph<(), ()> {
            let mut graph = DirectedGraph::<(), ()>::new();
            for _ in 0..node_count {
                graph.add_node(());
            }
            for &(from, to) in edges {
                graph.add_edge(NodeId::from_index(from), NodeId::from_index(to), ());
            }
            graph
        }

        vec![
            ("chain", build(3, &[(0, 1), (1, 2)])),
            ("diamond", build(4, &[(0, 1), (0, 2), (1, 3), (2, 3)])),
            (
                "cycle with a tail",
                build(4, &[(0, 1), (1, 2), (2, 1), (2, 3)]),
            ),
            (
                "two cycles in series",
                build(5, &[(0, 1), (1, 2), (2, 0), (1, 3), (3, 4), (4, 3)]),
            ),
            ("self loop", build(2, &[(0, 0)])),
            ("disconnected", build(4, &[(0, 1), (2, 3)])),
            ("empty", build(0, &[])),
        ]
    }

    fn partition(result: &SccResult<NodeId>) -> BTreeSet<BTreeSet<NodeId>> {
        result
            .components
            .iter()
            .map(|component| component.nodes.clone())
            .collect()
    }

    #[test]
    fn kosaraju_and_tarjan_agree_on_the_partition() {
        for (name, graph) in fixtures() {
            let kosaraju = kosaraju_scc(&graph);
            let tarjan = tarjan_scc(&graph);
            assert_eq!(partition(&kosaraju), partition(&tarjan), "{name}");
            assert_eq!(kosaraju.len(), tarjan.len(), "{name}");
            assert_eq!(kosaraju.is_dag(&graph), tarjan.is_dag(&graph), "{name}");
            for node in graph.node_ids() {
                assert_eq!(
                    kosaraju.component(node).nodes,
                    tarjan.component(node).nodes,
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn kosaraju_numbers_components_topologically() {
        for (name, graph) in fixtures() {
            let kosaraju = kosaraju_scc(&graph);
            let tarjan = tarjan_scc(&graph);
            for edge in graph.edges() {
                let (from, to) = (edge.source(), edge.target());
                let (source, target) =
                    (kosaraju.component_index(from), kosaraju.component_index(to));
                if kosaraju.component(from).contains(to) {
                    assert_eq!(source, target, "{name}: an intra-component edge");
                    continue;
                }
                // Sources first: an edge always runs from a lower component
                // index to a higher one.
                assert!(source < target, "{name}: {source} !< {target}");
                // Tarjan numbers the very same edge the other way round.
                assert!(
                    tarjan.component_index(from) > tarjan.component_index(to),
                    "{name}"
                );
            }
        }
    }

    /// `a <-> b <-> c` is a cycle, `b` enters the `d <-> e` cycle, `f` enters
    /// `a`, and `g` is isolated. A branching frame (`b` has two successors)
    /// makes this the shape that catches a depth-first walk which loses a
    /// frame's remaining successors when a sibling subtree finishes.
    fn nested() -> (DirectedGraph<&'static str, ()>, [NodeId; 7]) {
        let mut graph = DirectedGraph::<&str, ()>::new();
        let outer_a = graph.add_node("a");
        let outer_b = graph.add_node("b");
        let outer_c = graph.add_node("c");
        let inner_d = graph.add_node("d");
        let inner_e = graph.add_node("e");
        let entry_f = graph.add_node("f");
        let lone_g = graph.add_node("g");
        graph.add_edge(outer_a, outer_b, ());
        graph.add_edge(outer_b, outer_c, ());
        graph.add_edge(outer_c, outer_a, ());
        graph.add_edge(outer_b, inner_d, ());
        graph.add_edge(inner_d, inner_e, ());
        graph.add_edge(inner_e, inner_d, ());
        graph.add_edge(entry_f, outer_a, ());
        (
            graph,
            [outer_a, outer_b, outer_c, inner_d, inner_e, entry_f, lone_g],
        )
    }

    /// The component sequence, as payload names.
    fn sequence(
        result: &SccResult<NodeId>,
        graph: &DirectedGraph<&'static str, ()>,
    ) -> Vec<Vec<&'static str>> {
        result
            .components
            .iter()
            .map(|component| component.nodes.iter().map(|&node| graph[node]).collect())
            .collect()
    }

    #[test]
    fn kosaraju_numbering_matches_the_hand_computed_two_pass_order() {
        let (graph, [_, outer_b, _, _, inner_e, entry_f, lone_g]) = nested();

        // Pass 1 finishes [c, e, d, b, a, f, g], so pass 2 walks that in
        // reverse and mints components from the roots g, f, a, d — the
        // isolated node first because it finished last.
        let result = kosaraju_scc(&graph);
        assert_eq!(
            sequence(&result, &graph),
            vec![vec!["g"], vec!["f"], vec!["a", "b", "c"], vec!["d", "e"]]
        );
        assert_eq!(result.component_index(lone_g), 0);
        assert_eq!(result.component_index(entry_f), 1);
        assert_eq!(result.component_index(outer_b), 2);
        assert_eq!(result.component_index(inner_e), 3);
    }

    #[test]
    fn tarjan_numbering_matches_the_hand_computed_leaves_first_order() {
        let (graph, [outer_a, _, _, _, inner_e, entry_f, lone_g]) = nested();

        // One pass from `a`: `c` closes the cycle back to `a` without minting,
        // then `b`'s second successor `d` mints the inner cycle first, then
        // `a` mints its own. `f` and `g` follow as later roots, in node order.
        let result = tarjan_scc(&graph);
        assert_eq!(
            sequence(&result, &graph),
            vec![vec!["d", "e"], vec!["a", "b", "c"], vec!["f"], vec!["g"]]
        );
        assert_eq!(result.component_index(inner_e), 0);
        assert_eq!(result.component_index(outer_a), 1);
        assert_eq!(result.component_index(entry_f), 2);
        assert_eq!(result.component_index(lone_g), 3);
    }

    #[test]
    fn self_edge_is_not_a_dag() {
        let mut graph = DirectedGraph::<(), ()>::new();
        let node = graph.add_node(());
        graph.add_edge(node, node, ());
        let result = tarjan_scc(&graph);
        assert!(result.component(node).is_singleton());
        assert!(!result.is_dag(&graph));
    }
}
