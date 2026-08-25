//! CFG → AST lifting algorithm.
//!
//! Uses the dominator tree and edge classifications to reconstruct
//! structured control flow from a [`Cfg`].

extern crate alloc;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use super::node::{AstNode, CatchHandler, SwitchCase};
use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::dataflow::Predicated;
use crate::edge::EdgeKind;
use crate::graph::dominator::DominatorTree;
use crate::region::{HandlerKind, Region};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockFlowKind {
    LoopHeader,
    Conditional,
    Switch,
    BackEdge,
    Jump,
    Linear,
}

#[derive(Debug, Clone, Copy)]
struct BlockFlow {
    kind: BlockFlowKind,
    needs_label: bool,
}

struct LiftState<'a> {
    pdom: &'a DominatorTree,
    visited: &'a mut BTreeSet<u32>,
    /// Region anchor (first protected block in reverse postorder) → the
    /// regions anchored there, outermost (largest protected set) first.
    anchors: &'a BTreeMap<u32, Vec<&'a Region>>,
    /// Blocks targeted by a boundary [`AstNode::Goto`]; their eventual
    /// emission is wrapped in the matching label.
    goto_targets: BTreeSet<u32>,
}

fn has_edge_kind<I>(cfg: &Cfg<I>, edges: &[crate::EdgeId], kind: EdgeKind) -> bool {
    edges.iter().any(|&edge| cfg.edge(edge).kind() == kind)
}

/// Whether an edge transfers control exceptionally rather than sequentially.
///
/// [`EdgeKind::ExceptionLeave`] is a normal transfer out of a protected
/// region, so it stays sequential.
fn is_exception_edge(kind: EdgeKind) -> bool {
    matches!(
        kind,
        EdgeKind::ExceptionHandler
            | EdgeKind::ExceptionUnwind
            | EdgeKind::ExceptionResume
            | EdgeKind::ExceptionContinue
    )
}

/// Successors reached by sequential control flow, excluding exception edges.
fn flow_successors<I>(cfg: &Cfg<I>, block: BlockId) -> Vec<BlockId> {
    cfg.successor_edges(block)
        .iter()
        .filter(|&&edge| !is_exception_edge(cfg.edge(edge).kind()))
        .map(|&edge| cfg.edge(edge).target())
        .collect()
}

/// The blocks of `inner` also admitted by the enclosing bound, so a nested
/// region never escapes the extent it was entered under.
fn intersect_bounds(
    outer: Option<&BTreeSet<BlockId>>,
    inner: &BTreeSet<BlockId>,
) -> BTreeSet<BlockId> {
    match outer {
        None => inner.clone(),
        Some(outer) => inner.intersection(outer).copied().collect(),
    }
}

/// Group regions by anchor — their first protected block in reverse
/// postorder, i.e. the region's entry in flow order rather than by block id.
fn region_anchors<'c, I>(cfg: &'c Cfg<I>, order: &[BlockId]) -> BTreeMap<u32, Vec<&'c Region>> {
    let mut position: BTreeMap<u32, usize> = BTreeMap::new();
    for (index, block) in order.iter().enumerate() {
        position.insert(block.0, index);
    }
    let mut anchors: BTreeMap<u32, Vec<&Region>> = BTreeMap::new();
    for region in cfg.regions() {
        let anchor = region
            .protected_blocks
            .iter()
            .min_by_key(|block| position.get(&block.0).copied().unwrap_or(usize::MAX))
            .copied();
        if let Some(anchor) = anchor {
            anchors.entry(anchor.0).or_default().push(region);
        }
    }
    for candidates in anchors.values_mut() {
        candidates.sort_by_key(|region| core::cmp::Reverse(region.protected_blocks.len()));
    }
    anchors
}

fn classify_block<I>(cfg: &Cfg<I>, block: BlockId) -> BlockFlow {
    let successors = cfg.successor_edges(block);
    let predecessors = cfg.predecessor_edges(block);
    let kind = if has_edge_kind(cfg, predecessors, EdgeKind::Back) {
        BlockFlowKind::LoopHeader
    } else if has_edge_kind(cfg, successors, EdgeKind::ConditionalTrue)
        && has_edge_kind(cfg, successors, EdgeKind::ConditionalFalse)
    {
        BlockFlowKind::Conditional
    } else if has_edge_kind(cfg, successors, EdgeKind::SwitchCase) {
        BlockFlowKind::Switch
    } else if has_edge_kind(cfg, successors, EdgeKind::Back) {
        BlockFlowKind::BackEdge
    } else if has_edge_kind(cfg, successors, EdgeKind::Jump) {
        BlockFlowKind::Jump
    } else {
        BlockFlowKind::Linear
    };
    BlockFlow {
        kind,
        needs_label: has_edge_kind(cfg, predecessors, EdgeKind::Jump),
    }
}

fn push_block<I: Clone>(result: &mut Vec<AstNode<I>>, cfg: &Cfg<I>, block: BlockId) {
    let instructions = cfg.block(block).instructions().to_vec();
    if !instructions.is_empty() {
        result.push(AstNode::Block {
            id: block,
            instructions,
        });
    }
}

fn block_is_allowed(allowed_blocks: Option<&BTreeSet<BlockId>>, block: BlockId) -> bool {
    allowed_blocks.is_none_or(|blocks| blocks.contains(&block))
}

/// Push `node`, wrapped in `block`'s label when something jumps to it.
fn push_labeled<I: Clone>(
    result: &mut Vec<AstNode<I>>,
    cfg: &Cfg<I>,
    block: BlockId,
    needs_label: bool,
    node: AstNode<I>,
) {
    if needs_label {
        result.push(wrap_label(cfg, block, node));
    } else {
        result.push(node);
    }
}

/// Lift a [`Cfg`] into a structured [`AstNode`] tree.
///
/// The instruction type `I` must implement `Clone` so that instructions
/// can be copied into the AST nodes.
///
/// The lifter handles:
/// - Structured flow: `IfThenElse`, `Loop`, `Switch`
/// - Exception regions: `TryCatch` when every handler has a complete
///   [`HandlerBody`](crate::HandlerBody); multiple `Finally` handlers
///   concatenate in declaration order
/// - Unstructured flow: `Label` / `Goto` (for `Jump` edges and for control
///   transfers that leave a region or a declared handler extent)
///
/// Regions with an unknown handler extent remain ordinary control flow; the
/// lifter does not guess which blocks belong inside a handler. **No
/// reachable code is ever dropped**: a transfer refused at a region or
/// handler boundary becomes an explicit [`AstNode::Goto`], and every
/// reachable block the structured walk did not emit (refused targets,
/// handler pads entered only through exception edges, filter funclets) is
/// appended at the end under its label.
///
/// # Panics
///
/// Panics (debug) when a [`HandlerBody::Known`](crate::HandlerBody::Known)
/// omits its own handler entry — a frontend bug; release builds treat such
/// a region as unstructured flow.
#[must_use]
pub fn lift<I: Clone>(cfg: &Cfg<I>) -> AstNode<I> {
    let pdom = DominatorTree::compute_post(cfg);
    let order = cfg.reverse_postorder();
    let anchors = region_anchors(cfg, &order);
    let mut visited = BTreeSet::new();
    let mut state = LiftState {
        pdom: &pdom,
        visited: &mut visited,
        anchors: &anchors,
        goto_targets: BTreeSet::new(),
    };
    let mut body = lift_region(cfg, &mut state, cfg.entry(), None);

    // Completeness sweep: emit every reachable block the structured walk
    // refused or never reached, each under its label so the boundary
    // `Goto`s and exception edges that lead there stay resolvable. Region
    // metadata blocks (handler entries, filter funclets) are swept too —
    // a funclet is invoked by the runtime, so it can carry code without
    // any incoming CFG edge.
    let metadata_roots: Vec<BlockId> = cfg
        .regions()
        .iter()
        .flat_map(|region| region.handlers.iter())
        .flat_map(|handler| {
            let filter = match handler.kind {
                HandlerKind::Filter { filter_block } => Some(filter_block),
                _ => None,
            };
            core::iter::once(handler.entry).chain(filter)
        })
        .collect();
    while let Some(&pending) = order
        .iter()
        .chain(metadata_roots.iter())
        .find(|block| !state.visited.contains(&block.0))
    {
        // The sweep labels the sequence itself; drop any recorded target
        // so the block is not labeled twice.
        state.goto_targets.remove(&pending.0);
        let pending_body = lift_region(cfg, &mut state, pending, None);
        if !pending_body.is_empty() {
            body.push(AstNode::Label {
                name: block_label_name(cfg, pending),
                body: pending_body,
            });
        }
    }

    let ast = AstNode::Sequence { body };
    ast.simplify()
}

/// Recursively lift a region starting at `head`.
fn lift_region<I: Clone>(
    cfg: &Cfg<I>,
    state: &mut LiftState<'_>,
    head: BlockId,
    allowed_blocks: Option<&BTreeSet<BlockId>>,
) -> Vec<AstNode<I>> {
    let mut result = Vec::new();
    let mut current = Some(head);

    while let Some(block) = current {
        if state.visited.contains(&block.0) {
            break;
        }
        if !block_is_allowed(allowed_blocks, block) {
            // A transfer refused at the bound is recorded, never dropped:
            // the jump is explicit here and the completeness sweep in
            // [`lift`] emits the target under the matching label.
            state.goto_targets.insert(block.0);
            result.push(AstNode::Goto {
                target: block_label_name(cfg, block),
            });
            break;
        }

        state.visited.insert(block.0);

        if let Some(node) = lift_try_catch(cfg, state, block, allowed_blocks) {
            result.push(node);
            current = advance_merge(state.pdom, block, state.visited);
            continue;
        }

        current = lift_block(cfg, state, block, allowed_blocks, &mut result);
    }

    result
}

/// Classify one already-visited block, emit its lifted form, and return the
/// block sequential flow continues at (if any).
///
/// This is the single classification path: [`lift_region`]'s walk and
/// [`lift_try_catch`]'s try-body anchor both run it, so a region anchored at
/// a branch, dispatch, or loop header keeps its structure inside the try.
fn lift_block<I: Clone>(
    cfg: &Cfg<I>,
    state: &mut LiftState<'_>,
    block: BlockId,
    allowed_blocks: Option<&BTreeSet<BlockId>>,
    result: &mut Vec<AstNode<I>>,
) -> Option<BlockId> {
    let successor_edges = cfg.successor_edges(block);
    let flow = classify_block(cfg, block);
    let needs_label = flow.needs_label || state.goto_targets.contains(&block.0);

    if flow.kind == BlockFlowKind::LoopHeader {
        let node = lift_loop(cfg, state, block, allowed_blocks);
        push_labeled(result, cfg, block, needs_label, node);
        return find_loop_exit(cfg, block, state.visited, allowed_blocks);
    }

    if flow.kind == BlockFlowKind::Conditional {
        let node = lift_conditional(cfg, state, block, allowed_blocks);
        push_labeled(result, cfg, block, needs_label, node);
        return advance_merge(state.pdom, block, state.visited);
    }

    if flow.kind == BlockFlowKind::Switch {
        let node = lift_switch(cfg, state, block, allowed_blocks);
        push_labeled(result, cfg, block, needs_label, node);
        return advance_merge(state.pdom, block, state.visited);
    }

    if flow.kind == BlockFlowKind::BackEdge {
        push_block(result, cfg, block);
        result.push(AstNode::Continue);
        return None;
    }

    if flow.kind == BlockFlowKind::Jump {
        push_block(result, cfg, block);
        for &eid in successor_edges {
            let edge = cfg.edge(eid);
            if edge.kind() == EdgeKind::Jump {
                result.push(AstNode::Goto {
                    target: block_label_name(cfg, edge.target()),
                });
            }
        }
        return None;
    }

    if successor_edges.is_empty() {
        let insts = cfg.block(block).instructions().to_vec();
        if !insts.is_empty() {
            let node = AstNode::Return {
                id: block,
                instructions: insts,
            };
            push_labeled(result, cfg, block, needs_label, node);
        }
        return None;
    }

    // The builder creates empty blocks with a single Unconditional
    // edge for `break` statements. Recognize these and emit Break.
    if cfg.block(block).is_empty()
        && successor_edges.len() == 1
        && cfg.edge(successor_edges[0]).kind() == EdgeKind::Unconditional
    {
        result.push(AstNode::Break);
        return None;
    }

    let block_node = AstNode::Block {
        id: block,
        instructions: cfg.block(block).instructions().to_vec(),
    };
    push_labeled(result, cfg, block, needs_label, block_node);

    // Advance along the single sequential successor; exception edges
    // are not sequential flow, so they never block the advance. A
    // disallowed successor is handled by the boundary `Goto` in
    // [`lift_region`]'s walk.
    let flow_succs = flow_successors(cfg, block);
    if flow_succs.len() == 1 && !state.visited.contains(&flow_succs[0].0) {
        return Some(flow_succs[0]);
    }
    None
}

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
/// a region), and predicated runs inside a
/// [`SwitchCase`]'s `header_instructions` are not regionized (the case
/// structure has no place to hoist them to).
#[must_use]
pub fn lift_predicated<I: Clone + Predicated>(cfg: &Cfg<I>) -> AstNode<I> {
    wrap_predicated(lift(cfg)).simplify()
}

fn wrap_nodes<I: Clone + Predicated>(nodes: Vec<AstNode<I>>) -> Vec<AstNode<I>> {
    nodes.into_iter().map(wrap_predicated).collect()
}

/// Split a block's instructions into guarded segments.
fn wrap_block_runs<I: Clone + Predicated>(id: BlockId, instructions: Vec<I>) -> AstNode<I> {
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
fn wrap_return_runs<I: Clone + Predicated>(id: BlockId, instructions: Vec<I>) -> AstNode<I> {
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
        AstNode::Loop { header, body } => AstNode::Loop {
            header,
            body: wrap_nodes(body),
        },
        AstNode::Switch {
            condition,
            condition_instructions,
            cases,
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
                            header_instructions: case.header_instructions,
                            body: wrap_nodes(case.body),
                        })
                        .collect(),
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
        leaf @ (AstNode::Break | AstNode::Continue | AstNode::Goto { .. }) => leaf,
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
    block: BlockId,
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

/// Produce a label name for a block (used in Goto/Label nodes).
fn block_label_name<I>(cfg: &Cfg<I>, id: BlockId) -> alloc::string::String {
    cfg.block(id).label().map_or_else(
        || alloc::format!(".bb{}", id.0),
        alloc::string::String::from,
    )
}

/// Wrap a node in a Label node named like the [`AstNode::Goto`]s that
/// target the block.
fn wrap_label<I>(cfg: &Cfg<I>, block: BlockId, inner: AstNode<I>) -> AstNode<I> {
    AstNode::Label {
        name: block_label_name(cfg, block),
        body: alloc::vec![inner],
    }
}

/// The complete handler extents of `region`, or `None` when any handler's
/// extent is unknown and the region must stay unstructured.
///
/// A [`HandlerBody::Known`](crate::region::HandlerBody::Known) that omits
/// its own entry is a frontend bug, distinct from a deliberate `Unknown`:
/// it debug-panics, and degrades to unstructured flow in release builds.
fn complete_handler_bodies(region: &Region) -> Option<Vec<&BTreeSet<BlockId>>> {
    region
        .handlers
        .iter()
        .map(|handler| match handler.body.blocks() {
            Some(blocks) if blocks.contains(&handler.entry) => Some(blocks),
            Some(_) => {
                debug_assert!(
                    false,
                    "HandlerBody::Known must contain its own handler entry"
                );
                None
            }
            None => None,
        })
        .collect()
}

/// Lift a try/catch region anchored at `block`, if a structurable one exists.
///
/// Among the regions anchored at `block` (outermost first), the first with
/// complete handler extents is structured; regions with unknown extents
/// remain ordinary control flow. Bounds intersect with the enclosing
/// `allowed_blocks`, so a nested region never escapes the extent it was
/// entered under.
fn lift_try_catch<I: Clone>(
    cfg: &Cfg<I>,
    state: &mut LiftState<'_>,
    block: BlockId,
    allowed_blocks: Option<&BTreeSet<BlockId>>,
) -> Option<AstNode<I>> {
    let candidates = state.anchors.get(&block.0)?;
    let (region, handler_bodies) = candidates
        .iter()
        .find_map(|region| complete_handler_bodies(region).map(|bodies| (*region, bodies)))?;

    // Lift the try body through the shared classification path, so an
    // anchor that is itself a branch, dispatch, or loop header keeps its
    // structure. `lift_block` never consults the anchor map, so this
    // cannot re-trigger the region check and recurse.
    let try_bound = intersect_bounds(allowed_blocks, &region.protected_blocks);
    let mut try_body = Vec::new();
    let next = lift_block(cfg, state, block, Some(&try_bound), &mut try_body);
    if let Some(next) = next {
        if !state.visited.contains(&next.0) {
            try_body.extend(lift_region(cfg, state, next, Some(&try_bound)));
        }
    }
    // Protected successors the classified walk did not reach (for example
    // the targets of a multi-successor linear anchor) still belong to the
    // try body.
    for succ in cfg.successors(block) {
        if region.protected_blocks.contains(&succ) && !state.visited.contains(&succ.0) {
            try_body.extend(lift_region(cfg, state, succ, Some(&try_bound)));
        }
    }

    let mut handlers = Vec::new();
    let mut finally_body = Vec::new();

    for ((index, handler), handler_blocks) in region.handlers.iter().enumerate().zip(handler_bodies)
    {
        let handler_bound = intersect_bounds(allowed_blocks, handler_blocks);
        let body = lift_region(cfg, state, handler.entry, Some(&handler_bound));
        match handler.kind {
            // Multiple `Finally` handlers concatenate in declaration order.
            HandlerKind::Finally => finally_body.extend(body),
            _ => {
                handlers.push(CatchHandler {
                    handler: crate::region::HandlerRef::new(region.id, index),
                    entry: handler.entry,
                    kind: handler.kind,
                    body,
                });
            }
        }
    }

    Some(AstNode::TryCatch {
        try_body,
        handlers,
        finally_body,
    })
}

/// Get the post-dominator merge point if it hasn't been visited yet.
fn advance_merge(pdom: &DominatorTree, block: BlockId, visited: &BTreeSet<u32>) -> Option<BlockId> {
    pdom.idom(block).filter(|m| !visited.contains(&m.0))
}

/// Lift an if/else conditional starting at `block`.
fn lift_conditional<I: Clone>(
    cfg: &Cfg<I>,
    state: &mut LiftState<'_>,
    block: BlockId,
    allowed_blocks: Option<&BTreeSet<BlockId>>,
) -> AstNode<I> {
    let mut true_target = None;
    let mut false_target = None;
    for &eid in cfg.successor_edges(block) {
        match cfg.edge(eid).kind() {
            EdgeKind::ConditionalTrue => true_target = Some(cfg.edge(eid).target()),
            EdgeKind::ConditionalFalse => false_target = Some(cfg.edge(eid).target()),
            _ => {}
        }
    }

    let merge = state.pdom.idom(block);

    let then_body = match true_target {
        Some(t) if merge.is_none_or(|m| t != m) => lift_arm(cfg, state, t, merge, allowed_blocks),
        _ => Vec::new(),
    };
    let else_body = match false_target {
        Some(f) if merge.is_none_or(|m| f != m) => lift_arm(cfg, state, f, merge, allowed_blocks),
        _ => Vec::new(),
    };

    AstNode::IfThenElse {
        condition: block,
        condition_instructions: cfg.block(block).instructions().to_vec(),
        then_body,
        else_body,
    }
}

/// Lift a switch starting at `block`.
fn lift_switch<I: Clone>(
    cfg: &Cfg<I>,
    state: &mut LiftState<'_>,
    block: BlockId,
    allowed_blocks: Option<&BTreeSet<BlockId>>,
) -> AstNode<I> {
    let merge = state.pdom.idom(block);
    let mut cases = Vec::new();

    for &eid in cfg.successor_edges(block) {
        let edge = cfg.edge(eid);
        if edge.kind() == EdgeKind::SwitchCase {
            let cb = edge.target();
            if !block_is_allowed(allowed_blocks, cb) {
                // The case leaves the enclosing bound: keep the case, make
                // the jump explicit, and let the completeness sweep emit
                // the target under its label.
                state.goto_targets.insert(cb.0);
                cases.push(SwitchCase {
                    id: cb,
                    header_instructions: Vec::new(),
                    body: alloc::vec![AstNode::Goto {
                        target: block_label_name(cfg, cb),
                    }],
                });
                continue;
            }
            state.visited.insert(cb.0);
            let header_insts = cfg.block(cb).instructions().to_vec();
            let body = lift_case_body(cfg, state, cb, merge, allowed_blocks);
            cases.push(SwitchCase {
                id: cb,
                header_instructions: header_insts,
                body,
            });
        }
    }

    AstNode::Switch {
        condition: block,
        condition_instructions: cfg.block(block).instructions().to_vec(),
        cases,
    }
}

/// Lift a loop starting at `header`.
fn lift_loop<I: Clone>(
    cfg: &Cfg<I>,
    state: &mut LiftState<'_>,
    header: BlockId,
    allowed_blocks: Option<&BTreeSet<BlockId>>,
) -> AstNode<I> {
    let mut body = Vec::new();

    let successor_edges = cfg.successor_edges(header);
    let is_conditional = has_edge_kind(cfg, successor_edges, EdgeKind::ConditionalTrue)
        && has_edge_kind(cfg, successor_edges, EdgeKind::ConditionalFalse);
    let has_switch = has_edge_kind(cfg, successor_edges, EdgeKind::SwitchCase);

    if is_conditional {
        let node = lift_conditional(cfg, state, header, allowed_blocks);
        body.push(node);
        if let Some(merge) = state.pdom.idom(header) {
            if !state.visited.contains(&merge.0) {
                body.extend(lift_region(cfg, state, merge, allowed_blocks));
            }
        }
    } else if has_switch {
        let node = lift_switch(cfg, state, header, allowed_blocks);
        body.push(node);
        if let Some(merge) = state.pdom.idom(header) {
            if !state.visited.contains(&merge.0) {
                body.extend(lift_region(cfg, state, merge, allowed_blocks));
            }
        }
    } else {
        let header_insts = cfg.block(header).instructions().to_vec();
        if !header_insts.is_empty() {
            body.push(AstNode::Block {
                id: header,
                instructions: header_insts,
            });
        }
        for &eid in successor_edges {
            let edge = cfg.edge(eid);
            if edge.kind() != EdgeKind::Back && !state.visited.contains(&edge.target().0) {
                body.extend(lift_region(cfg, state, edge.target(), allowed_blocks));
            }
        }
    }

    AstNode::Loop { header, body }
}

/// Lift an arm (then/else) stopping at the merge point.
fn lift_arm<I: Clone>(
    cfg: &Cfg<I>,
    state: &mut LiftState<'_>,
    start: BlockId,
    stop: Option<BlockId>,
    allowed_blocks: Option<&BTreeSet<BlockId>>,
) -> Vec<AstNode<I>> {
    if stop.is_some_and(|s| s == start) {
        return Vec::new();
    }
    // A start outside the bound yields the boundary `Goto` from
    // `lift_region` rather than an empty arm, which would wrongly read as
    // falling through to the merge.
    lift_region(cfg, state, start, allowed_blocks)
}

/// Lift the body of a switch case from its successors.
fn lift_case_body<I: Clone>(
    cfg: &Cfg<I>,
    state: &mut LiftState<'_>,
    case_block: BlockId,
    stop: Option<BlockId>,
    allowed_blocks: Option<&BTreeSet<BlockId>>,
) -> Vec<AstNode<I>> {
    let mut body = Vec::new();
    for succ in cfg.successors(case_block) {
        if stop.is_none_or(|s| s != succ) && !state.visited.contains(&succ.0) {
            body.extend(lift_region(cfg, state, succ, allowed_blocks));
        }
    }
    body
}

/// Find the exit of a loop (block reachable via break/conditional-break
/// from within the loop body that hasn't been visited yet).
///
/// Only considers edges whose source is inside the loop (visited) and
/// whose target is outside it (not visited), so nested loops don't
/// confuse the search.
///
/// Instead of scanning every edge in the CFG, this only examines the
/// successor edges of visited (in-loop) blocks, making it proportional
/// to the loop body size rather than the entire CFG.
fn find_loop_exit<I>(
    cfg: &Cfg<I>,
    header: BlockId,
    visited: &BTreeSet<u32>,
    allowed_blocks: Option<&BTreeSet<BlockId>>,
) -> Option<BlockId> {
    // An exit inside the enclosing bound is preferred so in-bound
    // continuation code is emitted here; an out-of-bound exit is still
    // returned as a fallback and becomes a boundary `Goto` upstream.
    let mut fallback = None;
    // First pass: look for exit edges from loop-body blocks (excluding
    // the header, which is checked separately below).
    for &block_raw in visited {
        let block = BlockId(block_raw);
        if block == header {
            continue;
        }
        for &eid in cfg.successor_edges(block) {
            let edge = cfg.edge(eid);
            let is_exit_edge = matches!(
                edge.kind(),
                EdgeKind::Unconditional | EdgeKind::ConditionalTrue | EdgeKind::ConditionalFalse
            );
            if is_exit_edge && !visited.contains(&edge.target().0) {
                if block_is_allowed(allowed_blocks, edge.target()) {
                    return Some(edge.target());
                }
                fallback.get_or_insert(edge.target());
            }
        }
    }
    // Also check edges directly from the header (e.g., conditional break
    // at the header level).
    for &eid in cfg.successor_edges(header) {
        let edge = cfg.edge(eid);
        if !visited.contains(&edge.target().0) && edge.kind() != EdgeKind::Back {
            if block_is_allowed(allowed_blocks, edge.target()) {
                return Some(edge.target());
            }
            fallback.get_or_insert(edge.target());
        }
    }
    fallback
}

#[cfg(test)]
mod tests;
