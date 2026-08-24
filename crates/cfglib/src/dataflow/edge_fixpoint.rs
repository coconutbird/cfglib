//! Edge-sensitive fixpoint dataflow over borrowed graph views.

extern crate alloc;

use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;

use crate::dataflow::fixpoint::Direction;
use crate::graph::edge_view::{DenseEdgeId, EdgeGraphView, EdgeRef};
use crate::graph::view::DenseNodeId;

/// A dataflow problem whose transfer can distinguish individual edges.
///
/// In forward analysis, `node_input` is the physical pre-state and
/// `node_output` is the physical post-state. An exceptional edge can therefore
/// transfer from `node_input`, while a normal edge uses `node_output`. The same
/// two physical facts are supplied during backward analysis; the default edge
/// transfer uses the fact flowing in the selected analysis direction.
pub trait EdgeProblem<G: EdgeGraphView> {
    /// Lattice element propagated through nodes and edges.
    type Fact: Clone + PartialEq;

    /// Analysis direction.
    fn direction(&self) -> Direction;

    /// Bottom value for nodes and live edges.
    fn bottom(&self, graph: &G) -> Self::Fact;

    /// Optional boundary fact for a node.
    ///
    /// Forward problems normally seed entries; backward problems normally
    /// seed exits. Returning facts for several nodes supports handler entries,
    /// coroutine resumptions, and graphs with multiple roots.
    fn boundary(&self, _graph: &G, _node: G::NodeId) -> Option<Self::Fact> {
        None
    }

    /// Meet/join operator, invoked in stable adjacency order.
    fn meet(&self, left: &Self::Fact, right: &Self::Fact) -> Self::Fact;

    /// Transfer through one node in the selected analysis direction.
    fn transfer_node(&self, graph: &G, node: G::NodeId, flow_fact: &Self::Fact) -> Self::Fact;

    /// Transfer onto one stable edge.
    ///
    /// The borrowed edge exposes its identity, view-oriented endpoints, and
    /// consumer data. Implementations may select either physical node state or
    /// construct a distinct edge fact from payload metadata.
    fn transfer_edge(
        &self,
        _graph: &G,
        _edge: EdgeRef<'_, G::NodeId, G::EdgeId, G::EdgeData>,
        node_input: &Self::Fact,
        node_output: &Self::Fact,
    ) -> Self::Fact {
        match self.direction() {
            Direction::Forward => node_output.clone(),
            Direction::Backward => node_input.clone(),
        }
    }
}

/// Deterministic solver limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EdgeSolveConfig {
    max_steps: Option<usize>,
}

impl EdgeSolveConfig {
    /// An unbounded solve.
    #[must_use]
    pub const fn unbounded() -> Self {
        Self { max_steps: None }
    }

    /// Stop before processing more than `limit` worklist entries.
    #[must_use]
    pub const fn with_step_limit(limit: usize) -> Self {
        Self {
            max_steps: Some(limit),
        }
    }

    /// Configured worklist-entry limit, if any.
    #[must_use]
    pub const fn max_steps(self) -> Option<usize> {
        self.max_steps
    }
}

/// A bounded edge-sensitive solve did not reach a fixpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeSolveError {
    /// The deterministic worklist still contained a node at the limit.
    StepLimitExceeded {
        /// Configured limit.
        limit: usize,
        /// Worklist entries already processed.
        steps: usize,
        /// Dense index of the next node that would have been processed.
        pending_node: usize,
    },
}

impl core::fmt::Display for EdgeSolveError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::StepLimitExceeded {
                limit,
                steps,
                pending_node,
            } => write!(
                formatter,
                "edge dataflow step limit {limit} exceeded after {steps} steps; next node is {pending_node}"
            ),
        }
    }
}

/// Node and edge facts at a completed fixpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeFacts<F> {
    node_input: Vec<F>,
    node_output: Vec<F>,
    edge: Vec<Option<F>>,
    steps: usize,
}

impl<F> EdgeFacts<F> {
    /// Physical input fact for `node`.
    #[must_use]
    pub fn fact_in<N: DenseNodeId>(&self, node: N) -> &F {
        &self.node_input[node.index()]
    }

    /// Physical output fact for `node`.
    #[must_use]
    pub fn fact_out<N: DenseNodeId>(&self, node: N) -> &F {
        &self.node_output[node.index()]
    }

    /// Fact on a live edge in the solved view, or `None` for a tombstone or an
    /// edge excluded by a filtered view.
    #[must_use]
    pub fn fact_on<E: DenseEdgeId>(&self, edge: E) -> Option<&F> {
        self.edge.get(edge.index()).and_then(Option::as_ref)
    }

    /// Edge facts indexed by dense edge slot, retaining `None` tombstones.
    #[must_use]
    pub fn edge_facts(&self) -> &[Option<F>] {
        &self.edge
    }

    /// Number of worklist entries processed.
    #[must_use]
    pub const fn steps(&self) -> usize {
        self.steps
    }
}

/// Solve an edge-sensitive problem to a fixpoint without a step limit.
///
/// # Errors
///
/// The unbounded configuration cannot produce a solver-limit error. The
/// `Result` matches [`solve_edge_problem_with_config`] so callers can switch
/// configurations without changing result handling.
pub fn solve_edge_problem<G, P>(
    graph: &G,
    problem: &P,
) -> Result<EdgeFacts<P::Fact>, EdgeSolveError>
where
    G: EdgeGraphView,
    P: EdgeProblem<G>,
{
    solve_edge_problem_with_config(graph, problem, EdgeSolveConfig::unbounded())
}

/// Solve an edge-sensitive problem with deterministic bounded iteration.
///
/// # Errors
///
/// Returns [`EdgeSolveError::StepLimitExceeded`] when work remains at the
/// configured limit.
pub fn solve_edge_problem_with_config<G, P>(
    graph: &G,
    problem: &P,
    config: EdgeSolveConfig,
) -> Result<EdgeFacts<P::Fact>, EdgeSolveError>
where
    G: EdgeGraphView,
    P: EdgeProblem<G>,
{
    let bottom = problem.bottom(graph);
    let mut node_input = vec![bottom.clone(); graph.node_count()];
    let mut node_output = vec![bottom.clone(); graph.node_count()];
    let mut edge = vec![None; graph.edge_slot_count()];
    for edge_id in graph.edge_ids() {
        edge[edge_id.index()] = Some(bottom.clone());
    }

    let mut worklist: BTreeSet<_> = (0..graph.node_count()).collect();
    let mut steps = 0;
    while let Some(node_index) = worklist.pop_first() {
        if let Some(limit) = config.max_steps
            && steps >= limit
        {
            return Err(EdgeSolveError::StepLimitExceeded {
                limit,
                steps,
                pending_node: node_index,
            });
        }
        steps += 1;
        let node = G::NodeId::from_index(node_index);

        match problem.direction() {
            Direction::Forward => solve_forward_node(
                graph,
                problem,
                node,
                &bottom,
                &mut node_input,
                &mut node_output,
                &mut edge,
                &mut worklist,
            ),
            Direction::Backward => solve_backward_node(
                graph,
                problem,
                node,
                &bottom,
                &mut node_input,
                &mut node_output,
                &mut edge,
                &mut worklist,
            ),
        }
    }

    Ok(EdgeFacts {
        node_input,
        node_output,
        edge,
        steps,
    })
}

fn meet_edges<G, P>(
    graph: &G,
    problem: &P,
    node: G::NodeId,
    bottom: &P::Fact,
    edge_facts: &[Option<P::Fact>],
    incoming: bool,
) -> P::Fact
where
    G: EdgeGraphView,
    P: EdgeProblem<G>,
{
    let mut merged = problem.boundary(graph, node);
    let edges: Vec<_> = if incoming {
        graph.incoming_edges(node).collect()
    } else {
        graph.outgoing_edges(node).collect()
    };
    for edge in edges {
        let fact = edge_facts[edge.index()]
            .as_ref()
            .expect("adjacency contains an edge excluded from the view");
        merged = Some(match merged {
            Some(current) => problem.meet(&current, fact),
            None => fact.clone(),
        });
    }
    merged.unwrap_or_else(|| bottom.clone())
}

#[allow(clippy::too_many_arguments)]
fn solve_forward_node<G, P>(
    graph: &G,
    problem: &P,
    node: G::NodeId,
    bottom: &P::Fact,
    node_input: &mut [P::Fact],
    node_output: &mut [P::Fact],
    edge_facts: &mut [Option<P::Fact>],
    worklist: &mut BTreeSet<usize>,
) where
    G: EdgeGraphView,
    P: EdgeProblem<G>,
{
    let input = meet_edges(graph, problem, node, bottom, edge_facts, true);
    let output = problem.transfer_node(graph, node, &input);
    node_input[node.index()] = input;
    node_output[node.index()] = output;

    let outgoing: Vec<_> = graph.outgoing_edges(node).collect();
    for edge_id in outgoing {
        let edge_ref = graph.edge_ref(edge_id);
        let new_fact = problem.transfer_edge(
            graph,
            edge_ref,
            &node_input[node.index()],
            &node_output[node.index()],
        );
        let fact = edge_facts[edge_id.index()]
            .as_mut()
            .expect("adjacency contains an edge excluded from the view");
        if new_fact != *fact {
            *fact = new_fact;
            worklist.insert(edge_ref.target().index());
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn solve_backward_node<G, P>(
    graph: &G,
    problem: &P,
    node: G::NodeId,
    bottom: &P::Fact,
    node_input: &mut [P::Fact],
    node_output: &mut [P::Fact],
    edge_facts: &mut [Option<P::Fact>],
    worklist: &mut BTreeSet<usize>,
) where
    G: EdgeGraphView,
    P: EdgeProblem<G>,
{
    let output = meet_edges(graph, problem, node, bottom, edge_facts, false);
    let input = problem.transfer_node(graph, node, &output);
    node_output[node.index()] = output;
    node_input[node.index()] = input;

    let incoming: Vec<_> = graph.incoming_edges(node).collect();
    for edge_id in incoming {
        let edge_ref = graph.edge_ref(edge_id);
        let new_fact = problem.transfer_edge(
            graph,
            edge_ref,
            &node_input[node.index()],
            &node_output[node.index()],
        );
        let fact = edge_facts[edge_id.index()]
            .as_mut()
            .expect("adjacency contains an edge excluded from the view");
        if new_fact != *fact {
            *fact = new_fact;
            worklist.insert(edge_ref.source().index());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlockId, Cfg, Edge, EdgeKind};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Route {
        Normal,
        Handler(u8),
        Unwind,
    }

    struct PreAndPost {
        entry: BlockId,
    }

    impl EdgeProblem<Cfg<(), Route>> for PreAndPost {
        type Fact = u16;

        fn direction(&self) -> Direction {
            Direction::Forward
        }

        fn bottom(&self, _graph: &Cfg<(), Route>) -> Self::Fact {
            0
        }

        fn boundary(&self, _graph: &Cfg<(), Route>, node: BlockId) -> Option<Self::Fact> {
            (node == self.entry).then_some(1)
        }

        fn meet(&self, left: &Self::Fact, right: &Self::Fact) -> Self::Fact {
            left | right
        }

        fn transfer_node(
            &self,
            _graph: &Cfg<(), Route>,
            node: BlockId,
            input: &Self::Fact,
        ) -> Self::Fact {
            if node == self.entry {
                input | 2
            } else {
                *input
            }
        }

        fn transfer_edge(
            &self,
            _graph: &Cfg<(), Route>,
            edge: EdgeRef<'_, BlockId, crate::EdgeId, Edge<Route>>,
            node_input: &Self::Fact,
            node_output: &Self::Fact,
        ) -> Self::Fact {
            match *edge.data().payload() {
                Route::Normal => *node_output,
                Route::Handler(order) => node_input | (1 << (order + 2)),
                Route::Unwind => node_input | (1 << 8),
            }
        }
    }

    #[test]
    fn normal_and_exceptional_edges_receive_different_physical_states() {
        let mut cfg = Cfg::<(), Route>::new_with_edge_payload();
        let normal_target = cfg.new_block();
        let first_handler = cfg.new_block();
        let second_handler = cfg.new_block();
        let unwind_target = cfg.new_block();
        let entry = cfg.entry();
        let normal =
            cfg.add_edge_with_payload(entry, normal_target, EdgeKind::Fallthrough, Route::Normal);
        let handler_zero = cfg.add_edge_with_payload(
            entry,
            first_handler,
            EdgeKind::ExceptionHandler,
            Route::Handler(0),
        );
        let handler_one = cfg.add_edge_with_payload(
            entry,
            second_handler,
            EdgeKind::ExceptionHandler,
            Route::Handler(1),
        );
        let unwind = cfg.add_edge_with_payload(
            entry,
            unwind_target,
            EdgeKind::ExceptionUnwind,
            Route::Unwind,
        );

        let facts = solve_edge_problem(&cfg, &PreAndPost { entry }).unwrap();
        assert_eq!(facts.fact_in(entry), &1);
        assert_eq!(facts.fact_out(entry), &3);
        assert_eq!(facts.fact_on(normal), Some(&3));
        assert_eq!(facts.fact_on(handler_zero), Some(&5));
        assert_eq!(facts.fact_on(handler_one), Some(&9));
        assert_eq!(facts.fact_on(unwind), Some(&257));
        assert_eq!(
            cfg.successor_edges(entry),
            [normal, handler_zero, handler_one, unwind]
        );
    }

    #[test]
    fn step_limit_failure_is_structured_and_deterministic() {
        let cfg = Cfg::<(), Route>::new_with_edge_payload();
        let error = solve_edge_problem_with_config(
            &cfg,
            &PreAndPost { entry: cfg.entry() },
            EdgeSolveConfig::with_step_limit(0),
        )
        .unwrap_err();
        assert_eq!(
            error,
            EdgeSolveError::StepLimitExceeded {
                limit: 0,
                steps: 0,
                pending_node: 0,
            }
        );
    }
}
