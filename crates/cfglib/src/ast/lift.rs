//! CFG → AST lifting algorithm.
//!
//! Uses the dominator tree and edge classifications to reconstruct
//! structured control flow from a [`Cfg`].

extern crate alloc;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use super::node::{AstNode, CatchHandler, SwitchCase};
use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::dataflow::Predicated;
use crate::edge::EdgeKind;
use crate::graph::dominator::DominatorTree;
use crate::region::HandlerKind;

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

fn has_edge_kind<I>(cfg: &Cfg<I>, edges: &[crate::EdgeId], kind: EdgeKind) -> bool {
    edges.iter().any(|&edge| cfg.edge(edge).kind() == kind)
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

/// Lift a [`Cfg`] into a structured [`AstNode`] tree.
///
/// The instruction type `I` must implement `Clone` so that instructions
/// can be copied into the AST nodes.
///
/// The lifter handles:
/// - Structured flow: `IfThenElse`, `Loop`, `Switch`
/// - Exception regions: `TryCatch` (from [`Cfg::regions`])
/// - Unstructured flow: `Label` / `Goto` (for `Jump` edges)
#[must_use]
pub fn lift<I: Clone>(cfg: &Cfg<I>) -> AstNode<I> {
    let dom = DominatorTree::compute(cfg);
    let pdom = DominatorTree::compute_post(cfg);
    let mut visited = BTreeSet::new();
    // Collect the entry blocks of each region so we know which blocks
    // start a try/catch scope.
    let region_entries: BTreeSet<u32> = cfg
        .regions()
        .iter()
        .filter_map(|r| r.protected_blocks.iter().next())
        .map(|b| b.0)
        .collect();
    let body = lift_region(cfg, &dom, &pdom, cfg.entry(), &mut visited, &region_entries);
    let ast = AstNode::Sequence { body };
    ast.simplify()
}

/// Recursively lift a region starting at `head`.
fn lift_region<I: Clone>(
    cfg: &Cfg<I>,
    dom: &DominatorTree,
    pdom: &DominatorTree,
    head: BlockId,
    visited: &mut BTreeSet<u32>,
    region_entries: &BTreeSet<u32>,
) -> Vec<AstNode<I>> {
    let mut result = Vec::new();
    let mut current = Some(head);

    while let Some(block) = current {
        if visited.contains(&block.0) {
            break;
        }

        visited.insert(block.0);
        current = None;

        if region_entries.contains(&block.0)
            && let Some(node) = lift_try_catch(cfg, dom, pdom, block, visited, region_entries)
        {
            result.push(node);
            current = advance_merge(pdom, block, visited);
            continue;
        }

        let successor_edges = cfg.successor_edges(block);
        let flow = classify_block(cfg, block);
        let needs_label = flow.needs_label;

        if flow.kind == BlockFlowKind::LoopHeader {
            let node = lift_loop(cfg, dom, pdom, block, visited, region_entries);
            if needs_label {
                result.push(wrap_label(block, node));
            } else {
                result.push(node);
            }
            current = find_loop_exit(cfg, block, visited);
            continue;
        }

        if flow.kind == BlockFlowKind::Conditional {
            let node = lift_conditional(cfg, dom, pdom, block, visited, region_entries);
            if needs_label {
                result.push(wrap_label(block, node));
            } else {
                result.push(node);
            }
            current = advance_merge(pdom, block, visited);
            continue;
        }

        if flow.kind == BlockFlowKind::Switch {
            let node = lift_switch(cfg, dom, pdom, block, visited, region_entries);
            if needs_label {
                result.push(wrap_label(block, node));
            } else {
                result.push(node);
            }
            current = advance_merge(pdom, block, visited);
            continue;
        }

        if flow.kind == BlockFlowKind::BackEdge {
            push_block(&mut result, cfg, block);
            result.push(AstNode::Continue);
            continue;
        }

        if flow.kind == BlockFlowKind::Jump {
            push_block(&mut result, cfg, block);
            for &eid in successor_edges {
                let edge = cfg.edge(eid);
                if edge.kind() == EdgeKind::Jump {
                    result.push(AstNode::Goto {
                        target: block_label_name(cfg, edge.target()),
                    });
                }
            }
            continue;
        }

        if successor_edges.is_empty() {
            let insts = cfg.block(block).instructions().to_vec();
            if !insts.is_empty() {
                result.push(AstNode::Return {
                    id: block,
                    instructions: insts,
                });
            }
            continue;
        }

        // The builder creates empty blocks with a single Unconditional
        // edge for `break` statements. Recognise these and emit Break.
        if cfg.block(block).is_empty()
            && successor_edges.len() == 1
            && cfg.edge(successor_edges[0]).kind() == EdgeKind::Unconditional
        {
            result.push(AstNode::Break);
            continue;
        }

        let block_node = AstNode::Block {
            id: block,
            instructions: cfg.block(block).instructions().to_vec(),
        };
        if needs_label {
            result.push(wrap_label(block, block_node));
        } else {
            result.push(block_node);
        }
        let succs: Vec<BlockId> = cfg.successors(block).collect();
        if succs.len() == 1 && !visited.contains(&succs[0].0) {
            current = Some(succs[0]);
        }
    }

    result
}

/// Lift a [`Cfg`] and regionise predicated instructions into
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
/// [`SwitchCase`]'s `header_instructions` are not regionised (the case
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
        // Nothing to regionise: reassemble the original instruction list.
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

/// Wrap a node in a Label node.
fn wrap_label<I>(block: BlockId, inner: AstNode<I>) -> AstNode<I> {
    AstNode::Label {
        name: alloc::format!(".bb{}", block.0),
        body: alloc::vec![inner],
    }
}

/// Lift a try/catch region starting at `block`.
fn lift_try_catch<I: Clone>(
    cfg: &Cfg<I>,
    dom: &DominatorTree,
    pdom: &DominatorTree,
    block: BlockId,
    visited: &mut BTreeSet<u32>,
    region_entries: &BTreeSet<u32>,
) -> Option<AstNode<I>> {
    let region = cfg.protecting_region(block)?;

    // Lift the try body: emit the current block's instructions, then
    // follow successors within the protected region. We do NOT
    // un-visit and re-enter lift_region because that would re-trigger
    // the region_entries check and cause infinite recursion.
    let mut try_body = Vec::new();
    let insts = cfg.block(block).instructions().to_vec();
    if !insts.is_empty() {
        try_body.push(AstNode::Block {
            id: block,
            instructions: insts,
        });
    }
    for succ in cfg.successors(block) {
        if region.protected_blocks.contains(&succ) && !visited.contains(&succ.0) {
            try_body.extend(lift_region(cfg, dom, pdom, succ, visited, region_entries));
        }
    }

    let mut handlers = Vec::new();
    let mut finally_body = Vec::new();

    for (index, handler) in region.handlers.iter().enumerate() {
        let body = lift_region(cfg, dom, pdom, handler.entry, visited, region_entries);
        match handler.kind {
            HandlerKind::Finally => {
                finally_body = body;
            }
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
    dom: &DominatorTree,
    pdom: &DominatorTree,
    block: BlockId,
    visited: &mut BTreeSet<u32>,
    region_entries: &BTreeSet<u32>,
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

    let merge = pdom.idom(block);

    let then_body = match true_target {
        Some(t) if merge.is_none_or(|m| t != m) => {
            lift_arm(cfg, dom, pdom, t, merge, visited, region_entries)
        }
        _ => Vec::new(),
    };
    let else_body = match false_target {
        Some(f) if merge.is_none_or(|m| f != m) => {
            lift_arm(cfg, dom, pdom, f, merge, visited, region_entries)
        }
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
    dom: &DominatorTree,
    pdom: &DominatorTree,
    block: BlockId,
    visited: &mut BTreeSet<u32>,
    region_entries: &BTreeSet<u32>,
) -> AstNode<I> {
    let merge = pdom.idom(block);
    let mut cases = Vec::new();

    for &eid in cfg.successor_edges(block) {
        let edge = cfg.edge(eid);
        if edge.kind() == EdgeKind::SwitchCase {
            let cb = edge.target();
            visited.insert(cb.0);
            let header_insts = cfg.block(cb).instructions().to_vec();
            let body = lift_case_body(cfg, dom, pdom, cb, merge, visited, region_entries);
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
    dom: &DominatorTree,
    pdom: &DominatorTree,
    header: BlockId,
    visited: &mut BTreeSet<u32>,
    region_entries: &BTreeSet<u32>,
) -> AstNode<I> {
    let mut body = Vec::new();

    let successor_edges = cfg.successor_edges(header);
    let is_conditional = has_edge_kind(cfg, successor_edges, EdgeKind::ConditionalTrue)
        && has_edge_kind(cfg, successor_edges, EdgeKind::ConditionalFalse);
    let has_switch = has_edge_kind(cfg, successor_edges, EdgeKind::SwitchCase);

    if is_conditional {
        let node = lift_conditional(cfg, dom, pdom, header, visited, region_entries);
        body.push(node);
        if let Some(merge) = pdom.idom(header)
            && !visited.contains(&merge.0)
        {
            body.extend(lift_region(cfg, dom, pdom, merge, visited, region_entries));
        }
    } else if has_switch {
        let node = lift_switch(cfg, dom, pdom, header, visited, region_entries);
        body.push(node);
        if let Some(merge) = pdom.idom(header)
            && !visited.contains(&merge.0)
        {
            body.extend(lift_region(cfg, dom, pdom, merge, visited, region_entries));
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
            if edge.kind() != EdgeKind::Back && !visited.contains(&edge.target().0) {
                body.extend(lift_region(
                    cfg,
                    dom,
                    pdom,
                    edge.target(),
                    visited,
                    region_entries,
                ));
            }
        }
    }

    AstNode::Loop { header, body }
}

/// Lift an arm (then/else) stopping at the merge point.
fn lift_arm<I: Clone>(
    cfg: &Cfg<I>,
    dom: &DominatorTree,
    pdom: &DominatorTree,
    start: BlockId,
    stop: Option<BlockId>,
    visited: &mut BTreeSet<u32>,
    region_entries: &BTreeSet<u32>,
) -> Vec<AstNode<I>> {
    if stop.is_some_and(|s| s == start) {
        return Vec::new();
    }
    lift_region(cfg, dom, pdom, start, visited, region_entries)
}

/// Lift the body of a switch case from its successors.
fn lift_case_body<I: Clone>(
    cfg: &Cfg<I>,
    dom: &DominatorTree,
    pdom: &DominatorTree,
    case_block: BlockId,
    stop: Option<BlockId>,
    visited: &mut BTreeSet<u32>,
    region_entries: &BTreeSet<u32>,
) -> Vec<AstNode<I>> {
    let mut body = Vec::new();
    for succ in cfg.successors(case_block) {
        if stop.is_none_or(|s| s != succ) && !visited.contains(&succ.0) {
            body.extend(lift_region(cfg, dom, pdom, succ, visited, region_entries));
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
fn find_loop_exit<I>(cfg: &Cfg<I>, header: BlockId, visited: &BTreeSet<u32>) -> Option<BlockId> {
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
                return Some(edge.target());
            }
        }
    }
    // Also check edges directly from the header (e.g., conditional break
    // at the header level).
    for &eid in cfg.successor_edges(header) {
        let edge = cfg.edge(eid);
        if !visited.contains(&edge.target().0) && edge.kind() != EdgeKind::Back {
            return Some(edge.target());
        }
    }
    None
}

#[cfg(test)]
mod tests;
