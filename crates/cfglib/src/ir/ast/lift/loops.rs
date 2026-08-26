//! Natural-loop lifting: shape classification, bodies, and follows.
//!
//! A loop's body is lifted inside the natural-loop block set, so exits
//! resolve to `Break` / `Continue` against the loop stack instead of being
//! absorbed into the body — the follow block is returned to the caller as
//! the loop's sequential continuation.

extern crate alloc;
use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;

use super::super::node::{AstNode, LoopKind};
use super::{
    LiftState, LoopContext, advance_merge, block_is_allowed, block_label_name, has_edge_kind,
    is_back_edge, is_exception_edge, lift_conditional, lift_region, lift_switch, push_block,
};
use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::edge::EdgeKind;
use crate::graph::structure::NaturalLoop;

/// The two conditional-branch targets of a block, when it ends in one.
struct ConditionalTargets {
    true_target: BlockId,
    false_target: BlockId,
}

fn conditional_targets<I, E>(cfg: &Cfg<I, E>, block: BlockId) -> Option<ConditionalTargets> {
    let mut true_target = None;
    let mut false_target = None;
    for &eid in cfg.successor_edges(block) {
        match cfg.edge(eid).kind() {
            EdgeKind::ConditionalTrue => true_target = Some(cfg.edge(eid).target()),
            EdgeKind::ConditionalFalse => false_target = Some(cfg.edge(eid).target()),
            _ => {}
        }
    }
    Some(ConditionalTargets {
        true_target: true_target?,
        false_target: false_target?,
    })
}

/// The recognized shape of one natural loop before its body is lifted.
enum LoopShape {
    /// Pre-tested: the header's conditional exits the loop on one arm.
    While {
        body_start: BlockId,
        exit: BlockId,
        exit_on_true: bool,
    },
    /// Post-tested: the unique latch's conditional returns or exits.
    DoWhile {
        latch: BlockId,
        exit: BlockId,
        continue_on_true: bool,
    },
    /// Post-tested single-block loop: the header is its own latch.
    SelfLoop {
        exit: BlockId,
        continue_on_true: bool,
    },
    /// No recognized test.
    Endless,
}

fn classify_loop<I, E>(cfg: &Cfg<I, E>, natural: &NaturalLoop, header: BlockId) -> LoopShape {
    if let Some(targets) = conditional_targets(cfg, header) {
        let true_inside = natural.body.contains(&targets.true_target);
        let false_inside = natural.body.contains(&targets.false_target);
        if true_inside != false_inside {
            let (exit, body_start) = if true_inside {
                (targets.false_target, targets.true_target)
            } else {
                (targets.true_target, targets.false_target)
            };
            if body_start == header {
                // The in-loop arm is the back edge itself: the test runs
                // after the block's instructions, a single-block do/while.
                return LoopShape::SelfLoop {
                    exit,
                    continue_on_true: true_inside,
                };
            }
            return LoopShape::While {
                body_start,
                exit,
                exit_on_true: !true_inside,
            };
        }
    }
    if natural.latches.len() == 1 {
        let latch = *natural
            .latches
            .iter()
            .next()
            .expect("a one-element set has a first element");
        if latch != header {
            if let Some(targets) = conditional_targets(cfg, latch) {
                let true_back = targets.true_target == header;
                let false_back = targets.false_target == header;
                if true_back != false_back {
                    let other = if true_back {
                        targets.false_target
                    } else {
                        targets.true_target
                    };
                    if !natural.body.contains(&other) {
                        return LoopShape::DoWhile {
                            latch,
                            exit: other,
                            continue_on_true: true_back,
                        };
                    }
                }
            }
        }
    }
    LoopShape::Endless
}

/// The deterministic follow of a loop with no recognized test: the first
/// sequential exit target scanning body blocks in identity order, preferring
/// targets inside the enclosing bound.
fn endless_follow<I, E>(
    cfg: &Cfg<I, E>,
    natural: &NaturalLoop,
    allowed_blocks: Option<&BTreeSet<BlockId>>,
) -> Option<BlockId> {
    let mut fallback = None;
    for &block in &natural.body {
        for &eid in cfg.successor_edges(block) {
            let edge = cfg.edge(eid);
            let sequential = matches!(
                edge.kind(),
                EdgeKind::Fallthrough
                    | EdgeKind::Unconditional
                    | EdgeKind::ConditionalTrue
                    | EdgeKind::ConditionalFalse
                    | EdgeKind::Jump
            );
            if sequential && !natural.body.contains(&edge.target()) {
                if block_is_allowed(allowed_blocks, edge.target()) {
                    return Some(edge.target());
                }
                fallback.get_or_insert(edge.target());
            }
        }
    }
    fallback
}

/// Lift the loop anchored at `header` (already visited by the caller) and
/// return the lifted node with the loop's sequential continuation.
pub(super) fn lift_loop<I: Clone, E>(
    cfg: &Cfg<I, E>,
    state: &mut LiftState<'_>,
    header: BlockId,
    allowed_blocks: Option<&BTreeSet<BlockId>>,
) -> (AstNode<I>, Option<BlockId>) {
    let fallback;
    let natural = if let Some(natural) = state.loops.get(&header.0) {
        natural
    } else {
        // The classification and the loop map derive from the same back
        // edges, so a header without a loop is a library bug — degrade
        // to a single-block loop rather than dropping code.
        debug_assert!(false, "loop header {header} has no natural loop");
        fallback = NaturalLoop {
            header,
            body: [header].into_iter().collect(),
            latches: [header].into_iter().collect(),
            depth: 0,
        };
        &fallback
    };
    let bound: BTreeSet<BlockId> = match allowed_blocks {
        None => natural.body.clone(),
        Some(allowed) => natural.body.intersection(allowed).copied().collect(),
    };

    let shape = classify_loop(cfg, natural, header);
    let (follow, continue_target) = match &shape {
        LoopShape::While { exit, .. } | LoopShape::SelfLoop { exit, .. } => (Some(*exit), header),
        LoopShape::DoWhile { latch, exit, .. } => (Some(*exit), *latch),
        LoopShape::Endless => (endless_follow(cfg, natural, allowed_blocks), header),
    };

    state.loop_stack.push(LoopContext {
        header: header.0,
        follow: follow.map(|block| block.0),
        continue_target: continue_target.0,
        labeled: false,
    });

    let (kind, mut body) = match shape {
        LoopShape::While {
            body_start,
            exit_on_true,
            ..
        } => (
            LoopKind::While {
                condition: cfg.block(header).instructions().to_vec(),
                exit_on_true,
            },
            lift_region(cfg, state, body_start, Some(&bound), None),
        ),
        LoopShape::SelfLoop {
            continue_on_true, ..
        } => (
            LoopKind::DoWhile {
                latch: header,
                condition: cfg.block(header).instructions().to_vec(),
                continue_on_true,
            },
            Vec::new(),
        ),
        LoopShape::DoWhile {
            latch,
            continue_on_true,
            ..
        } => {
            // The latch is the condition, not part of the body: pre-visit
            // it so the body walk ends there silently, and transfers to it
            // resolve as `continue`.
            state.visited.insert(latch.0);
            let body = lift_header_body(cfg, state, header, &bound, Some(latch));
            (
                LoopKind::DoWhile {
                    latch,
                    condition: cfg.block(latch).instructions().to_vec(),
                    continue_on_true,
                },
                body,
            )
        }
        LoopShape::Endless => (
            LoopKind::Endless,
            lift_header_body(cfg, state, header, &bound, None),
        ),
    };

    // A trailing plain `continue` at the structural end of the body is the
    // loop's own back edge — implicit in the representation.
    if matches!(body.last(), Some(AstNode::Continue { label: None })) {
        body.pop();
    }

    let context = state
        .loop_stack
        .pop()
        .expect("the loop context pushed above is still on the stack");
    let node = AstNode::Loop { header, kind, body };
    let node = if context.labeled {
        state.labeled_blocks.insert(header.0);
        AstNode::Label {
            name: block_label_name(cfg, header),
            body: vec![node],
        }
    } else {
        node
    };
    (node, follow)
}

/// Lift a loop body whose header's own instructions belong inside it (the
/// post-tested and endless shapes): emit the header's structure manually —
/// it is already visited — then continue the ordinary walk.
fn lift_header_body<I: Clone, E>(
    cfg: &Cfg<I, E>,
    state: &mut LiftState<'_>,
    header: BlockId,
    bound: &BTreeSet<BlockId>,
    stop: Option<BlockId>,
) -> Vec<AstNode<I>> {
    let mut body = Vec::new();
    let successor_edges = cfg.successor_edges(header);
    let is_conditional = has_edge_kind(cfg, successor_edges, EdgeKind::ConditionalTrue)
        && has_edge_kind(cfg, successor_edges, EdgeKind::ConditionalFalse);
    let has_switch = has_edge_kind(cfg, successor_edges, EdgeKind::SwitchCase);

    if is_conditional {
        let node = lift_conditional(cfg, state, header, Some(bound));
        body.push(node);
        if let Some(merge) = advance_merge(cfg, state, header, Some(bound)) {
            body.extend(lift_region(cfg, state, merge, Some(bound), stop));
        }
    } else if has_switch {
        let node = lift_switch(cfg, state, header, Some(bound));
        body.push(node);
        if let Some(merge) = advance_merge(cfg, state, header, Some(bound)) {
            body.extend(lift_region(cfg, state, merge, Some(bound), stop));
        }
    } else {
        push_block(&mut body, cfg, header);
        for &eid in successor_edges {
            let edge = cfg.edge(eid);
            if !is_back_edge(cfg, state.back_edges, eid) && !is_exception_edge(edge.kind()) {
                body.extend(lift_region(cfg, state, edge.target(), Some(bound), stop));
            }
        }
    }
    body
}
