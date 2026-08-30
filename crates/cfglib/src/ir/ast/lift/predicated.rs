//! Predicated-region wrapping over a lifted tree.

extern crate alloc;
use alloc::vec::Vec;

use super::super::node::{AstNode, CatchHandler, SwitchCase};
use super::lift;
use crate::cfg::Cfg;
use crate::dataflow::Predicated;

/// Lift a [`Cfg`] and regionize predicated instructions into
/// [`AstNode::Guarded`] nodes.
///
/// Runs [`lift`], then wraps every maximal run of instructions sharing the
/// same [`Predicated::predicate`] into a `Guarded` node whose witness is the
/// run's first instruction. Unpredicated instructions stay in plain blocks.
/// Predicated runs that land in a branch/dispatch header
/// (`IfThenElse`/`Switch` `condition_instructions`) are hoisted into
/// guarded segments before the node. Two ledgered limits: a predicate on
/// the branch/dispatch instruction itself stays inline (unrepresentable as
/// a region), and predicated runs inside a loop's condition witness
/// ([`LoopKind`](super::super::LoopKind)) are not regionized — a condition
/// re-evaluates every iteration, so hoisting would change its semantics.
#[must_use]
pub fn lift_predicated<I: Clone + Predicated, E>(cfg: &Cfg<I, E>) -> AstNode<I> {
    wrap_predicated(lift(cfg)).simplify()
}

fn wrap_nodes<I: Clone + Predicated>(nodes: Vec<AstNode<I>>) -> Vec<AstNode<I>> {
    nodes.into_iter().map(wrap_predicated).collect()
}

/// Split a block's instructions into guarded segments.
fn wrap_block_runs<I: Clone + Predicated>(id: crate::BlockId, instructions: Vec<I>) -> AstNode<I> {
    let segments = predicate_runs(instructions)
        .into_iter()
        .map(|(predicate, run)| {
            guard_segment(
                predicate.as_ref(),
                AstNode::Block {
                    id,
                    instructions: run,
                },
            )
        })
        .collect();
    sequence_or_single(segments)
}

/// Split a return block's instructions into guarded segments; the final run
/// keeps its `Return` semantics (a predicated final run is a conditional
/// return, e.g. ARM `bxeq lr`).
fn wrap_return_runs<I: Clone + Predicated>(id: crate::BlockId, instructions: Vec<I>) -> AstNode<I> {
    let mut runs = predicate_runs(instructions);
    let last = runs.pop();
    let mut segments: Vec<AstNode<I>> = runs
        .into_iter()
        .map(|(predicate, run)| {
            guard_segment(
                predicate.as_ref(),
                AstNode::Block {
                    id,
                    instructions: run,
                },
            )
        })
        .collect();
    if let Some((predicate, run)) = last {
        segments.push(guard_segment(
            predicate.as_ref(),
            AstNode::Return {
                id,
                instructions: run,
            },
        ));
    }
    sequence_or_single(segments)
}

fn wrap_predicated<I: Clone + Predicated>(node: AstNode<I>) -> AstNode<I> {
    match node {
        AstNode::Block { id, instructions } => wrap_block_runs(id, instructions),
        AstNode::Return { id, instructions } => wrap_return_runs(id, instructions),
        AstNode::Sequence { body } => AstNode::Sequence {
            body: wrap_nodes(body),
        },
        AstNode::IfThenElse {
            condition,
            condition_instructions,
            then_body,
            else_body,
        } => {
            let (prefix, rest) = split_header_runs(condition, condition_instructions);
            with_prefix(
                prefix,
                AstNode::IfThenElse {
                    condition,
                    condition_instructions: rest,
                    then_body: wrap_nodes(then_body),
                    else_body: wrap_nodes(else_body),
                },
            )
        }
        AstNode::Loop { header, kind, body } => AstNode::Loop {
            header,
            kind,
            body: wrap_nodes(body),
        },
        AstNode::Switch {
            condition,
            condition_instructions,
            cases,
            default_body,
            default_edge,
        } => {
            let (prefix, rest) = split_header_runs(condition, condition_instructions);
            with_prefix(
                prefix,
                AstNode::Switch {
                    condition,
                    condition_instructions: rest,
                    cases: cases
                        .into_iter()
                        .map(|case| SwitchCase {
                            id: case.id,
                            edges: case.edges,
                            body: wrap_nodes(case.body),
                        })
                        .collect(),
                    default_body: wrap_nodes(default_body),
                    default_edge,
                },
            )
        }
        AstNode::Label { name, body } => AstNode::Label {
            name,
            body: wrap_nodes(body),
        },
        AstNode::TryCatch {
            try_body,
            handlers,
            finally_body,
        } => AstNode::TryCatch {
            try_body: wrap_nodes(try_body),
            handlers: handlers
                .into_iter()
                .map(|handler| CatchHandler {
                    handler: handler.handler,
                    entry: handler.entry,
                    kind: handler.kind,
                    body: wrap_nodes(handler.body),
                })
                .collect(),
            finally_body: wrap_nodes(finally_body),
        },
        AstNode::Guarded {
            predicate,
            when_true,
            body,
        } => AstNode::Guarded {
            predicate,
            when_true,
            body: wrap_nodes(body),
        },
        leaf @ (AstNode::Break { .. } | AstNode::Continue { .. } | AstNode::Goto { .. }) => leaf,
    }
}

/// A maximal instruction run sharing one predicate.
type PredicateRun<I> = (
    Option<(<I as crate::dataflow::InstrInfo>::Variable, bool)>,
    Vec<I>,
);

/// Group instructions into maximal runs sharing one predicate.
fn predicate_runs<I: Predicated>(instructions: Vec<I>) -> Vec<PredicateRun<I>> {
    let mut runs: Vec<PredicateRun<I>> = Vec::new();
    for instruction in instructions {
        let predicate = instruction.predicate();
        match runs.last_mut() {
            Some((run_predicate, run)) if *run_predicate == predicate => run.push(instruction),
            _ => runs.push((predicate, alloc::vec![instruction])),
        }
    }
    runs
}

/// Wrap a segment in [`AstNode::Guarded`] when its run is predicated.
fn guard_segment<I: Clone + Predicated>(
    predicate: Option<&(I::Variable, bool)>,
    segment: AstNode<I>,
) -> AstNode<I> {
    match predicate {
        Some((_, when_true)) => {
            let when_true = *when_true;
            let witness = match &segment {
                AstNode::Block { instructions, .. } | AstNode::Return { instructions, .. } => {
                    instructions[0].clone()
                }
                _ => unreachable!("guard_segment only wraps Block/Return segments"),
            };
            AstNode::Guarded {
                predicate: witness,
                when_true,
                body: alloc::vec![segment],
            }
        }
        None => segment,
    }
}

/// Collapse a single-segment vector; wrap several segments in a sequence.
fn sequence_or_single<I>(mut segments: Vec<AstNode<I>>) -> AstNode<I> {
    if segments.len() == 1 {
        segments.pop().expect("single segment")
    } else {
        AstNode::Sequence { body: segments }
    }
}

/// Split a branch/dispatch header's predicated PREFIX runs into guarded
/// segments to hoist before the node. The final run — which contains the
/// branch/dispatch instruction itself — always stays in place, and headers
/// with no predicated prefix pass through untouched.
fn split_header_runs<I: Clone + Predicated>(
    block: crate::BlockId,
    instructions: Vec<I>,
) -> (Vec<AstNode<I>>, Vec<I>) {
    let mut runs = predicate_runs(instructions);
    let Some((_, last)) = runs.pop() else {
        return (Vec::new(), Vec::new());
    };
    if runs.iter().all(|(predicate, _)| predicate.is_none()) {
        // Nothing to regionize: reassemble the original instruction list.
        let mut rest: Vec<I> = runs.into_iter().flat_map(|(_, run)| run).collect();
        rest.extend(last);
        return (Vec::new(), rest);
    }
    let prefix = runs
        .into_iter()
        .map(|(predicate, run)| {
            guard_segment(
                predicate.as_ref(),
                AstNode::Block {
                    id: block,
                    instructions: run,
                },
            )
        })
        .collect();
    (prefix, last)
}

/// Hoist `prefix` segments before `node`, or return `node` unchanged when
/// there is nothing to hoist.
fn with_prefix<I>(prefix: Vec<AstNode<I>>, node: AstNode<I>) -> AstNode<I> {
    if prefix.is_empty() {
        return node;
    }
    let mut body = prefix;
    body.push(node);
    AstNode::Sequence { body }
}
