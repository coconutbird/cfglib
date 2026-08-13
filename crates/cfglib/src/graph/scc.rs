//! Strongly connected components for any [`DirectedGraphView`].
//!
//! The iterative Tarjan implementation runs in `O(V + E)` and does not consume
//! the host call stack for deeply nested code graphs.

extern crate alloc;
use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;

use crate::graph::directed::{DenseNodeId, DirectedGraphView};

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SccResult<N> {
    /// Components in reverse topological order, with leaves first.
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

/// Compute strongly connected components with Tarjan's algorithm.
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

    for start in graph.node_ids() {
        if indices[start.index()] != usize::MAX {
            continue;
        }

        indices[start.index()] = next_index;
        lowlinks[start.index()] = next_index;
        next_index += 1;
        stack.push(start);
        on_stack[start.index()] = true;
        let mut calls = vec![(start, graph.successors(start).collect::<Vec<_>>(), 0_usize)];

        while let Some((node, successors, successor_index)) = calls.last().cloned() {
            if successor_index < successors.len() {
                let successor = successors[successor_index];
                let Some(frame) = calls.last_mut() else {
                    break;
                };
                frame.2 += 1;

                if indices[successor.index()] == usize::MAX {
                    indices[successor.index()] = next_index;
                    lowlinks[successor.index()] = next_index;
                    next_index += 1;
                    stack.push(successor);
                    on_stack[successor.index()] = true;
                    calls.push((successor, graph.successors(successor).collect(), 0));
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

            calls.pop();
            if let Some((parent, _, _)) = calls.last() {
                lowlinks[parent.index()] = lowlinks[parent.index()].min(lowlinks[node.index()]);
            }
        }
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
