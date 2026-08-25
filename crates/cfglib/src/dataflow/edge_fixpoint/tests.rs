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
    let mut cfg = Cfg::<(), Route>::with_edge_payload();
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
    let cfg = Cfg::<(), Route>::with_edge_payload();
    let error = solve_edge_problem_with_config(
        &cfg,
        &PreAndPost { entry: cfg.entry() },
        SolveConfig::with_step_limit(0),
    )
    .unwrap_err();
    assert_eq!(
        error,
        SolveError::StepLimitExceeded {
            limit: 0,
            steps: 0,
            pending_node: 0,
        }
    );
}
