//! Node-level fixpoint dataflow over any [`DirectedGraphView`].
//!
//! The instruction fixpoint in [`fixpoint`](super::fixpoint) is bound to
//! [`Cfg`](crate::Cfg) blocks; this is its graph-shaped counterpart: one
//! fact per node, meet over the in-edges (out-edges backward), transfer
//! per node. It serves analyses over non-CFG graphs — taint or
//! reachability-with-facts over a value-flow graph, closure over an import
//! or include graph — anywhere the graph is the program representation.

extern crate alloc;
use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

use super::fixpoint::Direction;
use crate::graph::view::{DenseNodeId, DirectedGraphView};

/// A node-level dataflow problem over a graph view `G`.
///
/// Termination requires the usual contract: `meet` and `transfer` monotone
/// over a finite-height fact lattice.
pub trait NodeProblem<G: DirectedGraphView> {
    /// The per-node dataflow fact.
    type Fact: Clone + PartialEq;

    /// Whether facts flow along edges ([`Direction::Forward`]) or against
    /// them ([`Direction::Backward`]).
    fn direction(&self) -> Direction;

    /// The initial fact for every node.
    fn bottom(&self, graph: &G) -> Self::Fact;

    /// The fact entering boundary nodes (no in-edges forward; no out-edges
    /// backward).
    fn boundary(&self, graph: &G) -> Self::Fact;

    /// Combine two facts at a join point.
    fn meet(&self, a: &Self::Fact, b: &Self::Fact) -> Self::Fact;

    /// The fact leaving `node`, given the met fact entering it.
    fn transfer(&self, graph: &G, node: G::NodeId, input: &Self::Fact) -> Self::Fact;
}

/// The solved per-node facts of a [`NodeProblem`].
#[derive(Debug, Clone)]
pub struct NodeFacts<F> {
    input: Vec<F>,
    output: Vec<F>,
}

impl<F> NodeFacts<F> {
    /// The met fact entering `node`.
    #[must_use]
    pub fn fact_in<N: DenseNodeId>(&self, node: N) -> &F {
        &self.input[node.index()]
    }

    /// The transferred fact leaving `node`.
    #[must_use]
    pub fn fact_out<N: DenseNodeId>(&self, node: N) -> &F {
        &self.output[node.index()]
    }
}

/// Solve a [`NodeProblem`] to fixpoint with a worklist.
#[must_use]
pub fn solve_node_problem<G, P>(graph: &G, problem: &P) -> NodeFacts<P::Fact>
where
    G: DirectedGraphView,
    P: NodeProblem<G>,
{
    let node_count = graph.node_count();
    let forward = matches!(problem.direction(), Direction::Forward);
    let boundary = problem.boundary(graph);
    let mut input = vec![problem.bottom(graph); node_count];
    let mut output = vec![problem.bottom(graph); node_count];

    let mut queued = vec![true; node_count];
    let mut worklist: VecDeque<G::NodeId> = graph.node_ids().collect();

    while let Some(node) = worklist.pop_front() {
        queued[node.index()] = false;

        let upstream: Vec<G::NodeId> = if forward {
            graph.predecessors(node).collect()
        } else {
            graph.successors(node).collect()
        };
        let mut met: Option<P::Fact> = None;
        for from in upstream {
            let fact = &output[from.index()];
            met = Some(match met {
                Some(current) => problem.meet(&current, fact),
                None => fact.clone(),
            });
        }
        let met = met.unwrap_or_else(|| boundary.clone());

        let transferred = problem.transfer(graph, node, &met);
        input[node.index()] = met;
        if transferred != output[node.index()] {
            output[node.index()] = transferred;
            let downstream: Vec<G::NodeId> = if forward {
                graph.successors(node).collect()
            } else {
                graph.predecessors(node).collect()
            };
            for next in downstream {
                if !queued[next.index()] {
                    queued[next.index()] = true;
                    worklist.push_back(next);
                }
            }
        }
    }

    NodeFacts { input, output }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::directed::{DirectedGraph, NodeId};

    /// Forward taint: a node is tainted when it is a source or any
    /// predecessor's output is tainted — over a value-flow-shaped graph.
    struct Taint {
        sources: alloc::vec::Vec<NodeId>,
    }

    impl<E> NodeProblem<DirectedGraph<&'static str, E>> for Taint {
        type Fact = bool;

        fn direction(&self) -> Direction {
            Direction::Forward
        }

        fn bottom(&self, _graph: &DirectedGraph<&'static str, E>) -> bool {
            false
        }

        fn boundary(&self, _graph: &DirectedGraph<&'static str, E>) -> bool {
            false
        }

        fn meet(&self, a: &bool, b: &bool) -> bool {
            *a || *b
        }

        fn transfer(
            &self,
            _graph: &DirectedGraph<&'static str, E>,
            node: NodeId,
            input: &bool,
        ) -> bool {
            *input || self.sources.contains(&node)
        }
    }

    #[test]
    fn taint_propagates_through_cycles() {
        let mut graph: DirectedGraph<&'static str, ()> = DirectedGraph::new();
        let source = graph.add_node("source");
        let a = graph.add_node("a");
        let b = graph.add_node("b");
        let clean = graph.add_node("clean");
        graph.add_edge(source, a, ());
        graph.add_edge(a, b, ());
        graph.add_edge(b, a, ()); // cycle
        graph.add_edge(clean, b, ());

        let facts = solve_node_problem(
            &graph,
            &Taint {
                sources: alloc::vec![source],
            },
        );
        assert!(*facts.fact_out(source));
        assert!(*facts.fact_out(a), "reached through the source");
        assert!(*facts.fact_out(b), "reached through the cycle");
        assert!(!*facts.fact_out(clean), "no path from the source");
        assert!(*facts.fact_in(b), "met input at b includes the tainted a");
    }
}
