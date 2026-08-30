use cfglib::{
    AbstractDomain, Cfg, DirectedGraph, Direction, DominatorTree, EdgeProblem, Lattice, NodeId,
    SolveConfig, SsaForm, TryEdgeProblem, TryNodeProblem, TryProblem, abstract_interpret,
    alias_propagation, copies_by_predecessor, copy_propagation, eliminate_phis, solve_edge_problem,
    solve_edge_problem_from, solve_edge_problem_from_with_config, solve_edge_problem_with_config,
    solve_node_problem_from, solve_node_problem_from_with_config, solve_problem_from,
    solve_problem_from_with_config, try_solve_edge_problem, try_solve_edge_problem_from,
    try_solve_edge_problem_from_with_config, try_solve_edge_problem_with_config,
    try_solve_node_problem, try_solve_node_problem_from, try_solve_node_problem_from_with_config,
    try_solve_node_problem_with_config, try_solve_problem, try_solve_problem_from,
    try_solve_problem_from_with_config, try_solve_problem_with_config,
};

use super::BenchmarkSuite;
use super::fixtures::{ApiInst, dataflow_cfg};
use crate::fixtures::{CfgReachability, Reachability, branchy_cfg, branchy_graph};
use crate::harness::benchmark_case;

const NODE_COUNT: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Seen(bool);

impl Lattice for Seen {
    fn bottom() -> Self {
        Self(false)
    }

    fn top() -> Self {
        Self(true)
    }

    fn meet(&self, other: &Self) -> Self {
        Self(self.0 || other.0)
    }

    fn leq(&self, other: &Self) -> bool {
        !self.0 || other.0
    }
}

impl AbstractDomain<ApiInst> for Seen {
    fn transfer(state: &Self, _instruction: &ApiInst) -> Self {
        *state
    }

    fn entry_value() -> Self {
        Self(true)
    }
}

struct TryCfgReachability;

impl TryProblem<u32> for TryCfgReachability {
    type Fact = bool;
    type Error = ();

    fn direction(&self) -> Direction {
        Direction::Forward
    }

    fn bottom(&self) -> Self::Fact {
        false
    }

    fn entry_fact(&self) -> Result<Self::Fact, Self::Error> {
        Ok(true)
    }

    fn meet(&self, left: &Self::Fact, right: &Self::Fact) -> Result<Self::Fact, Self::Error> {
        Ok(*left || *right)
    }

    fn transfer(
        &self,
        _cfg: &Cfg<u32>,
        _block: cfglib::BlockId,
        input: &Self::Fact,
    ) -> Result<Self::Fact, Self::Error> {
        Ok(*input)
    }
}

struct TryGraphReachability;

impl TryNodeProblem<DirectedGraph<(), ()>> for TryGraphReachability {
    type Fact = bool;
    type Error = ();

    fn direction(&self) -> Direction {
        Direction::Forward
    }

    fn bottom(&self, _graph: &DirectedGraph<(), ()>) -> Self::Fact {
        false
    }

    fn boundary(&self, _graph: &DirectedGraph<(), ()>) -> Result<Self::Fact, Self::Error> {
        Ok(true)
    }

    fn meet(&self, left: &Self::Fact, right: &Self::Fact) -> Result<Self::Fact, Self::Error> {
        Ok(*left || *right)
    }

    fn transfer(
        &self,
        _graph: &DirectedGraph<(), ()>,
        _node: NodeId,
        input: &Self::Fact,
    ) -> Result<Self::Fact, Self::Error> {
        Ok(*input)
    }
}

struct EdgeReachability;

impl EdgeProblem<DirectedGraph<(), ()>> for EdgeReachability {
    type Fact = bool;

    fn direction(&self) -> Direction {
        Direction::Forward
    }

    fn bottom(&self, _graph: &DirectedGraph<(), ()>) -> Self::Fact {
        false
    }

    fn boundary(&self, graph: &DirectedGraph<(), ()>, node: NodeId) -> Option<Self::Fact> {
        graph.predecessors(node).next().is_none().then_some(true)
    }

    fn meet(&self, left: &Self::Fact, right: &Self::Fact) -> Self::Fact {
        *left || *right
    }

    fn transfer_node(
        &self,
        _graph: &DirectedGraph<(), ()>,
        _node: NodeId,
        flow_fact: &Self::Fact,
    ) -> Self::Fact {
        *flow_fact
    }
}

struct TryEdgeReachability;

impl TryEdgeProblem<DirectedGraph<(), ()>> for TryEdgeReachability {
    type Fact = bool;
    type Error = ();

    fn direction(&self) -> Direction {
        Direction::Forward
    }

    fn bottom(&self, _graph: &DirectedGraph<(), ()>) -> Self::Fact {
        false
    }

    fn boundary(
        &self,
        graph: &DirectedGraph<(), ()>,
        node: NodeId,
    ) -> Result<Option<Self::Fact>, Self::Error> {
        Ok(graph.predecessors(node).next().is_none().then_some(true))
    }

    fn meet(
        &self,
        _graph: &DirectedGraph<(), ()>,
        _node: NodeId,
        left: &Self::Fact,
        right: &Self::Fact,
    ) -> Result<Self::Fact, Self::Error> {
        Ok(*left || *right)
    }

    fn transfer_node(
        &self,
        _graph: &DirectedGraph<(), ()>,
        _node: NodeId,
        flow_fact: &Self::Fact,
    ) -> Result<Self::Fact, Self::Error> {
        Ok(*flow_fact)
    }
}

macro_rules! solver_case {
    ($suite:expr, $name:literal, $api:path, $operation:expr) => {
        benchmark_case!($suite, $name, covers[$api], $operation, |result| assert!(
            result.is_ok()
        ));
    };
}

fn copy_cfg() -> Cfg<ApiInst> {
    let mut cfg = Cfg::new();
    cfg.block_mut(cfg.entry()).push(ApiInst::constant(0, 1));
    cfg.block_mut(cfg.entry()).push(ApiInst::copy(1, 0));
    cfg.block_mut(cfg.entry()).push(ApiInst::pure(2, vec![1]));
    cfg
}

fn phi_cfg() -> Cfg<ApiInst> {
    let mut cfg = Cfg::new();
    let left = cfg.new_block();
    let right = cfg.new_block();
    let merge = cfg.new_block();
    cfg.block_mut(left).push(ApiInst::constant(0, 1));
    cfg.block_mut(right).push(ApiInst::constant(0, 2));
    cfg.block_mut(merge).push(ApiInst::pure(1, vec![0]));
    cfg.add_edge(cfg.entry(), left, cfglib::EdgeKind::ConditionalTrue);
    cfg.add_edge(cfg.entry(), right, cfglib::EdgeKind::ConditionalFalse);
    cfg.add_edge(left, merge, cfglib::EdgeKind::Fallthrough);
    cfg.add_edge(right, merge, cfglib::EdgeKind::Fallthrough);
    cfg
}

fn register_analyses(suite: &mut BenchmarkSuite<'_>) {
    let cfg = dataflow_cfg(128, 8);
    benchmark_case!(
        suite,
        "api_abstract_interpret",
        covers[abstract_interpret],
        || abstract_interpret::<_, _, Seen>(&cfg),
        |facts| assert_eq!(
            facts.fact_out(cfglib::BlockId::from_index(127)),
            Some(&Seen(true))
        )
    );

    let copies_fixture = copy_cfg();
    benchmark_case!(
        suite,
        "api_copy_propagation",
        covers[copy_propagation],
        || {
            let mut candidate = copies_fixture.clone();
            let stats = copy_propagation(&mut candidate);
            (candidate, stats)
        },
        |(_, stats)| {
            assert_eq!(stats.uses_rewritten, 1);
            assert_eq!(stats.copies_removed, 1);
        }
    );
    benchmark_case!(
        suite,
        "api_alias_propagation",
        covers[alias_propagation],
        || {
            let mut candidate = copies_fixture.clone();
            let stats = alias_propagation(&mut candidate);
            (candidate, stats)
        },
        |(_, stats)| {
            assert_eq!(stats.uses_rewritten, 1);
            assert_eq!(stats.aliases_removed, 1);
        }
    );

    let phi_fixture = phi_cfg();
    let dominators = DominatorTree::compute(&phi_fixture);
    let ssa = SsaForm::compute(&phi_fixture, &dominators);
    let copies = eliminate_phis(&ssa);
    benchmark_case!(
        suite,
        "api_eliminate_phis",
        covers[eliminate_phis],
        || eliminate_phis(&ssa),
        |result: &Vec<_>| assert_eq!(result.len(), 2)
    );
    benchmark_case!(
        suite,
        "api_copies_by_predecessor",
        covers[copies_by_predecessor],
        || copies_by_predecessor(&copies),
        |groups: &Vec<_>| assert_eq!(groups.len(), 2)
    );
}

fn register_cfg_solvers(suite: &mut BenchmarkSuite<'_>) {
    let cfg = branchy_cfg(NODE_COUNT);
    let seeds = [cfg.entry()];
    let config = SolveConfig::new();

    solver_case!(suite, "api_solve_problem_from", solve_problem_from, || {
        solve_problem_from(&cfg, &CfgReachability, &seeds)
    });
    solver_case!(
        suite,
        "api_solve_problem_from_with_config",
        solve_problem_from_with_config,
        || solve_problem_from_with_config(&cfg, &CfgReachability, &seeds, config)
    );
    solver_case!(suite, "api_try_solve_problem", try_solve_problem, || {
        try_solve_problem(&cfg, &TryCfgReachability)
    });
    solver_case!(
        suite,
        "api_try_solve_problem_from",
        try_solve_problem_from,
        || try_solve_problem_from(&cfg, &TryCfgReachability, &seeds)
    );
    solver_case!(
        suite,
        "api_try_solve_problem_with_config",
        try_solve_problem_with_config,
        || try_solve_problem_with_config(&cfg, &TryCfgReachability, config)
    );
    solver_case!(
        suite,
        "api_try_solve_problem_from_with_config",
        try_solve_problem_from_with_config,
        || try_solve_problem_from_with_config(&cfg, &TryCfgReachability, &seeds, config)
    );
}

fn register_node_solvers(suite: &mut BenchmarkSuite<'_>) {
    let graph = branchy_graph(NODE_COUNT);
    let seeds = [NodeId::from_raw(0)];
    let config = SolveConfig::new();

    solver_case!(
        suite,
        "api_solve_node_problem_from",
        solve_node_problem_from,
        || solve_node_problem_from(&graph, &Reachability, &seeds)
    );
    solver_case!(
        suite,
        "api_solve_node_problem_from_with_config",
        solve_node_problem_from_with_config,
        || solve_node_problem_from_with_config(&graph, &Reachability, &seeds, config)
    );
    solver_case!(
        suite,
        "api_try_solve_node_problem",
        try_solve_node_problem,
        || try_solve_node_problem(&graph, &TryGraphReachability)
    );
    solver_case!(
        suite,
        "api_try_solve_node_problem_from",
        try_solve_node_problem_from,
        || try_solve_node_problem_from(&graph, &TryGraphReachability, &seeds)
    );
    solver_case!(
        suite,
        "api_try_solve_node_problem_with_config",
        try_solve_node_problem_with_config,
        || try_solve_node_problem_with_config(&graph, &TryGraphReachability, config)
    );
    solver_case!(
        suite,
        "api_try_solve_node_problem_from_with_config",
        try_solve_node_problem_from_with_config,
        || try_solve_node_problem_from_with_config(&graph, &TryGraphReachability, &seeds, config)
    );
}

fn register_edge_solvers(suite: &mut BenchmarkSuite<'_>) {
    let graph = branchy_graph(NODE_COUNT);
    let seeds = [NodeId::from_raw(0)];
    let config = SolveConfig::new();

    solver_case!(suite, "api_solve_edge_problem", solve_edge_problem, || {
        solve_edge_problem(&graph, &EdgeReachability)
    });
    solver_case!(
        suite,
        "api_solve_edge_problem_from",
        solve_edge_problem_from,
        || solve_edge_problem_from(&graph, &EdgeReachability, &seeds)
    );
    solver_case!(
        suite,
        "api_solve_edge_problem_with_config",
        solve_edge_problem_with_config,
        || solve_edge_problem_with_config(&graph, &EdgeReachability, config)
    );
    solver_case!(
        suite,
        "api_solve_edge_problem_from_with_config",
        solve_edge_problem_from_with_config,
        || solve_edge_problem_from_with_config(&graph, &EdgeReachability, &seeds, config)
    );
    solver_case!(
        suite,
        "api_try_solve_edge_problem",
        try_solve_edge_problem,
        || try_solve_edge_problem(&graph, &TryEdgeReachability)
    );
    solver_case!(
        suite,
        "api_try_solve_edge_problem_from",
        try_solve_edge_problem_from,
        || try_solve_edge_problem_from(&graph, &TryEdgeReachability, &seeds)
    );
    solver_case!(
        suite,
        "api_try_solve_edge_problem_with_config",
        try_solve_edge_problem_with_config,
        || try_solve_edge_problem_with_config(&graph, &TryEdgeReachability, config)
    );
    solver_case!(
        suite,
        "api_try_solve_edge_problem_from_with_config",
        try_solve_edge_problem_from_with_config,
        || try_solve_edge_problem_from_with_config(&graph, &TryEdgeReachability, &seeds, config,)
    );
}

pub(super) fn register(suite: &mut BenchmarkSuite<'_>) {
    register_analyses(suite);
    register_cfg_solvers(suite);
    register_node_solvers(suite);
    register_edge_solvers(suite);
}
