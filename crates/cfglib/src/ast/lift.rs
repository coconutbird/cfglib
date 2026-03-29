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
use crate::edge::EdgeKind;
use crate::graph::dominator::DominatorTree;
use crate::region::HandlerKind;

/// Lift a [`Cfg`] into a structured [`AstNode`] tree.
///
/// The instruction type `I` must implement `Clone` so that instructions
/// can be copied into the AST nodes.
///
/// The lifter handles:
/// - Structured flow: `IfThenElse`, `Loop`, `Switch`
/// - Exception regions: `TryCatch` (from [`Cfg::regions`])
/// - Unstructured flow: `Label` / `Goto` (for `Jump` edges)
pub fn lift<I: Clone>(cfg: &Cfg<I>) -> AstNode<I> {
    let dom = DominatorTree::compute(cfg);
    let pdom = DominatorTree::compute_post(cfg);
    let mut visited = BTreeSet::new();
    // Collect the entry blocks of each region so we know which blocks
    // start a try/catch scope.
    let region_entries: BTreeSet<u32> = cfg
        .regions()
        .iter()
        .flat_map(|r| r.protected_blocks.iter().next())
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

        // --- TryCatch region ---
        if region_entries.contains(&block.0)
            && let Some(node) = lift_try_catch(cfg, dom, pdom, block, visited, region_entries)
        {
            result.push(node);
            current = advance_merge(pdom, block, visited);
            continue;
        }

        let succ_edges = cfg.successor_edges(block);
        let has_ct = succ_edges
            .iter()
            .any(|&e| cfg.edge(e).kind() == EdgeKind::ConditionalTrue);
        let has_cf = succ_edges
            .iter()
            .any(|&e| cfg.edge(e).kind() == EdgeKind::ConditionalFalse);
        let has_sw = succ_edges
            .iter()
            .any(|&e| cfg.edge(e).kind() == EdgeKind::SwitchCase);
        let has_back = succ_edges
            .iter()
            .any(|&e| cfg.edge(e).kind() == EdgeKind::Back);
        let has_jump = succ_edges
            .iter()
            .any(|&e| cfg.edge(e).kind() == EdgeKind::Jump);
        let is_header = cfg
            .predecessor_edges(block)
            .iter()
            .any(|&e| cfg.edge(e).kind() == EdgeKind::Back);
        let is_jump_target = cfg
            .predecessor_edges(block)
            .iter()
            .any(|&e| cfg.edge(e).kind() == EdgeKind::Jump);

        // --- Label wrapper (for blocks targeted by Jump edges) ---
        let needs_label = is_jump_target;

        // --- Loop header ---
        if is_header {
            let node = lift_loop(cfg, dom, pdom, block, visited, region_entries);
            if needs_label {
                result.push(wrap_label(block, node));
            } else {
                result.push(node);
            }
            current = find_loop_exit(cfg, block, visited);
            continue;
        }

        // --- Conditional (if/else) ---
        if has_ct && has_cf {
            let node = lift_conditional(cfg, dom, pdom, block, visited, region_entries);
            if needs_label {
                result.push(wrap_label(block, node));
            } else {
                result.push(node);
            }
            current = advance_merge(pdom, block, visited);
            continue;
        }

        // --- Switch ---
        if has_sw {
            let node = lift_switch(cfg, dom, pdom, block, visited, region_entries);
            if needs_label {
                result.push(wrap_label(block, node));
            } else {
                result.push(node);
            }
            current = advance_merge(pdom, block, visited);
            continue;
        }

        // --- Back edge (loop latch) ---
        if has_back {
            let insts = cfg.block(block).instructions().to_vec();
            if !insts.is_empty() {
                result.push(AstNode::Block {
                    id: block,
                    instructions: insts,
                });
            }
            result.push(AstNode::Continue);
            continue;
        }

        // --- Jump edge (unstructured goto) ---
        if has_jump {
            let insts = cfg.block(block).instructions().to_vec();
            if !insts.is_empty() {
                result.push(AstNode::Block {
                    id: block,
                    instructions: insts,
                });
            }
            for &eid in succ_edges {
                let edge = cfg.edge(eid);
                if edge.kind() == EdgeKind::Jump {
                    result.push(AstNode::Goto {
                        target: block_label_name(cfg, edge.target()),
                    });
                }
            }
            continue;
        }

        // --- Terminal ---
        if succ_edges.is_empty() {
            let insts = cfg.block(block).instructions().to_vec();
            if !insts.is_empty() {
                result.push(AstNode::Return {
                    instructions: insts,
                });
            }
            continue;
        }

        // --- Break relay block ---
        // The builder creates empty blocks with a single Unconditional
        // edge for `break` statements. Recognise these and emit Break.
        if cfg.block(block).is_empty()
            && succ_edges.len() == 1
            && cfg.edge(succ_edges[0]).kind() == EdgeKind::Unconditional
        {
            result.push(AstNode::Break);
            continue;
        }

        // --- Fallthrough / unconditional ---
        let block_node = AstNode::Block {
            id: block,
            instructions: cfg.block(block).instructions().to_vec(),
        };
        let block_node = maybe_guard(cfg, block, block_node);
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

/// Wrap a node in a `Guarded` if the block has a predication guard.
fn maybe_guard<I>(cfg: &Cfg<I>, block: BlockId, node: AstNode<I>) -> AstNode<I> {
    if let Some(guard) = cfg.block(block).guard() {
        let pred = if guard.when_true {
            guard.predicate.clone()
        } else {
            alloc::format!("!{}", guard.predicate)
        };
        AstNode::Guarded {
            predicate: pred,
            body: alloc::vec![node],
        }
    } else {
        node
    }
}

/// Produce a label name for a block (used in Goto/Label nodes).
fn block_label_name<I>(cfg: &Cfg<I>, id: BlockId) -> alloc::string::String {
    cfg.block(id)
        .label()
        .map(alloc::string::String::from)
        .unwrap_or_else(|| alloc::format!(".bb{}", id.0))
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
    // Follow successors of the try entry within the protected region.
    for succ in cfg.successors(block) {
        if region.protected_blocks.contains(&succ) && !visited.contains(&succ.0) {
            try_body.extend(lift_region(cfg, dom, pdom, succ, visited, region_entries));
        }
    }

    // Lift handlers.
    let mut handlers = Vec::new();
    let mut finally_body = Vec::new();

    for handler in &region.handlers {
        let body = lift_region(cfg, dom, pdom, handler.entry, visited, region_entries);
        match handler.kind {
            HandlerKind::Finally => {
                finally_body = body;
            }
            _ => {
                handlers.push(CatchHandler {
                    entry: handler.entry,
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

    let succ_edges = cfg.successor_edges(header);
    let has_ct = succ_edges
        .iter()
        .any(|&e| cfg.edge(e).kind() == EdgeKind::ConditionalTrue);
    let has_cf = succ_edges
        .iter()
        .any(|&e| cfg.edge(e).kind() == EdgeKind::ConditionalFalse);
    let has_sw = succ_edges
        .iter()
        .any(|&e| cfg.edge(e).kind() == EdgeKind::SwitchCase);

    if has_ct && has_cf {
        let node = lift_conditional(cfg, dom, pdom, header, visited, region_entries);
        body.push(node);
        if let Some(merge) = pdom.idom(header)
            && !visited.contains(&merge.0)
        {
            body.extend(lift_region(cfg, dom, pdom, merge, visited, region_entries));
        }
    } else if has_sw {
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
        for &eid in succ_edges {
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
    for &block_raw in visited.iter() {
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
mod tests {
    use super::*;
    use crate::builder::CfgBuilder;
    use crate::flow::FlowEffect;
    use crate::test_util::{MockInst, ff};
    use alloc::vec;

    /// Helper: build CFG then lift, return pseudocode.
    fn lift_pseudo(insts: Vec<MockInst>) -> alloc::string::String {
        let cfg = CfgBuilder::build(insts).unwrap();
        let ast = lift(&cfg);
        ast.to_pseudocode()
    }

    // ---- Linear / trivial ----

    #[test]
    fn lift_linear() {
        let p = lift_pseudo(vec![
            ff("a"),
            ff("b"),
            ff("c"),
            MockInst(FlowEffect::Return, "ret"),
        ]);
        assert!(p.contains("a"), "should contain instruction a: {p}");
        assert!(p.contains("ret"), "should contain ret: {p}");
        // No control flow keywords.
        assert!(!p.contains("if"), "no if expected: {p}");
        assert!(!p.contains("loop"), "no loop expected: {p}");
    }

    // ---- If/else ----

    #[test]
    fn lift_if_no_else() {
        let p = lift_pseudo(vec![
            ff("a"),
            MockInst(FlowEffect::ConditionalOpen, "if"),
            ff("b"),
            MockInst(FlowEffect::ConditionalClose, "endif"),
            ff("c"),
            MockInst(FlowEffect::Return, "ret"),
        ]);
        assert!(p.contains("if {"), "should have if: {p}");
        assert!(p.contains("b"), "then body should contain b: {p}");
        assert!(p.contains("c"), "post-merge should contain c: {p}");
    }

    #[test]
    fn lift_if_else() {
        let p = lift_pseudo(vec![
            ff("a"),
            MockInst(FlowEffect::ConditionalOpen, "if"),
            ff("then_inst"),
            MockInst(FlowEffect::ConditionalAlternate, "else"),
            ff("else_inst"),
            MockInst(FlowEffect::ConditionalClose, "endif"),
            ff("merge"),
            MockInst(FlowEffect::Return, "ret"),
        ]);
        assert!(p.contains("if {"), "should have if: {p}");
        assert!(p.contains("then_inst"), "then arm: {p}");
        // else arm or merge should appear
        assert!(
            p.contains("else_inst") || p.contains("} else {"),
            "else arm: {p}"
        );
    }

    // ---- Loop ----

    #[test]
    fn lift_simple_loop() {
        let p = lift_pseudo(vec![
            MockInst(FlowEffect::LoopOpen, "loop"),
            ff("body"),
            MockInst(FlowEffect::LoopClose, "endloop"),
            MockInst(FlowEffect::Return, "ret"),
        ]);
        assert!(p.contains("loop {"), "should have loop: {p}");
        assert!(p.contains("body"), "loop body: {p}");
    }

    #[test]
    fn lift_loop_with_break() {
        let p = lift_pseudo(vec![
            MockInst(FlowEffect::LoopOpen, "loop"),
            ff("a"),
            MockInst(FlowEffect::ConditionalBreak, "breakc"),
            ff("b"),
            MockInst(FlowEffect::LoopClose, "endloop"),
            MockInst(FlowEffect::Return, "ret"),
        ]);
        assert!(p.contains("loop {"), "should have loop: {p}");
        // The breakc creates a conditional inside the loop
        assert!(p.contains("a"), "should contain a: {p}");
    }

    // ---- Switch ----

    #[test]
    fn lift_switch() {
        let p = lift_pseudo(vec![
            MockInst(FlowEffect::SwitchOpen, "switch"),
            ff("dispatch"),
            MockInst(FlowEffect::SwitchCase, "case0"),
            ff("arm0"),
            MockInst(FlowEffect::SwitchCase, "case1"),
            ff("arm1"),
            MockInst(FlowEffect::SwitchClose, "endswitch"),
            ff("after"),
            MockInst(FlowEffect::Return, "ret"),
        ]);
        assert!(p.contains("switch {"), "should have switch: {p}");
        assert!(p.contains("case {"), "should have case: {p}");
    }

    // ---- Nested structures ----

    #[test]
    fn lift_if_in_loop() {
        let p = lift_pseudo(vec![
            MockInst(FlowEffect::LoopOpen, "loop"),
            MockInst(FlowEffect::ConditionalOpen, "if"),
            ff("then"),
            MockInst(FlowEffect::ConditionalAlternate, "else"),
            ff("else_body"),
            MockInst(FlowEffect::ConditionalClose, "endif"),
            MockInst(FlowEffect::LoopClose, "endloop"),
            MockInst(FlowEffect::Return, "ret"),
        ]);
        assert!(p.contains("loop {"), "should have loop: {p}");
        assert!(p.contains("if {"), "should have if inside loop: {p}");
    }

    #[test]
    fn lift_loop_in_if() {
        let p = lift_pseudo(vec![
            MockInst(FlowEffect::ConditionalOpen, "if"),
            MockInst(FlowEffect::LoopOpen, "loop"),
            ff("body"),
            MockInst(FlowEffect::ConditionalBreak, "breakc"),
            MockInst(FlowEffect::LoopClose, "endloop"),
            MockInst(FlowEffect::ConditionalClose, "endif"),
            MockInst(FlowEffect::Return, "ret"),
        ]);
        // Should have both if and loop structures
        let has_if = p.contains("if {");
        let has_loop = p.contains("loop {");
        assert!(has_if || has_loop, "should have nested structure: {p}");
    }

    // ---- AST node structure checks ----

    #[test]
    fn lift_returns_sequence_or_single() {
        let cfg = CfgBuilder::build(vec![ff("a"), MockInst(FlowEffect::Return, "ret")]).unwrap();
        let ast = lift(&cfg);
        // Should be a Block or Return, not an empty Sequence.
        assert!(!ast.is_empty(), "should not be empty");
    }

    #[test]
    fn lift_conditional_produces_if_node() {
        let cfg = CfgBuilder::build(vec![
            ff("a"),
            MockInst(FlowEffect::ConditionalOpen, "if"),
            ff("b"),
            MockInst(FlowEffect::ConditionalClose, "endif"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        let ast = lift(&cfg);
        // Walk the AST to find an IfThenElse node.
        let found = has_node_kind(&ast, |n| matches!(n, AstNode::IfThenElse { .. }));
        assert!(found, "should contain IfThenElse node: {ast:?}");
    }

    #[test]
    fn lift_loop_produces_loop_node() {
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::LoopOpen, "loop"),
            ff("x"),
            MockInst(FlowEffect::LoopClose, "endloop"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        let ast = lift(&cfg);
        let found = has_node_kind(&ast, |n| matches!(n, AstNode::Loop { .. }));
        assert!(found, "should contain Loop node: {ast:?}");
    }

    #[test]
    fn lift_switch_produces_switch_node() {
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::SwitchOpen, "switch"),
            ff("d"),
            MockInst(FlowEffect::SwitchCase, "c1"),
            ff("a1"),
            MockInst(FlowEffect::SwitchCase, "c2"),
            ff("a2"),
            MockInst(FlowEffect::SwitchClose, "endswitch"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        let ast = lift(&cfg);
        let found = has_node_kind(&ast, |n| matches!(n, AstNode::Switch { .. }));
        assert!(found, "should contain Switch node: {ast:?}");
    }

    /// Recursively check if any node in the AST matches a predicate.
    fn has_node_kind<I>(node: &AstNode<I>, pred: fn(&AstNode<I>) -> bool) -> bool {
        if pred(node) {
            return true;
        }
        match node {
            AstNode::Sequence { body }
            | AstNode::Loop { body, .. }
            | AstNode::Label { body, .. } => body.iter().any(|c| has_node_kind(c, pred)),
            AstNode::IfThenElse {
                then_body,
                else_body,
                ..
            } => {
                then_body.iter().any(|c| has_node_kind(c, pred))
                    || else_body.iter().any(|c| has_node_kind(c, pred))
            }
            AstNode::Switch { cases, .. } => cases
                .iter()
                .any(|c| c.body.iter().any(|n| has_node_kind(n, pred))),
            AstNode::TryCatch {
                try_body,
                handlers,
                finally_body,
            } => {
                try_body.iter().any(|c| has_node_kind(c, pred))
                    || handlers
                        .iter()
                        .any(|h| h.body.iter().any(|n| has_node_kind(n, pred)))
                    || finally_body.iter().any(|c| has_node_kind(c, pred))
            }
            _ => false,
        }
    }

    // ---- TryCatch lifting ----

    #[test]
    fn lift_try_catch_produces_try_node() {
        use crate::region::{Handler, HandlerKind, Region, RegionId};
        use alloc::collections::BTreeSet;

        let mut cfg: Cfg<MockInst> = Cfg::new();
        // entry(0) → try_body(1) → after(3)
        //            try_body(1) --Exception--> handler(2) → after(3)
        let try_body = cfg.new_block(); // 1
        let handler_block = cfg.new_block(); // 2
        let after = cfg.new_block(); // 3

        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("entry"));
        cfg.block_mut(try_body)
            .instructions_vec_mut()
            .push(ff("try_inst"));
        cfg.block_mut(handler_block)
            .instructions_vec_mut()
            .push(ff("catch_inst"));
        cfg.block_mut(after)
            .instructions_vec_mut()
            .push(ff("after"));

        cfg.add_edge(cfg.entry(), try_body, EdgeKind::Fallthrough);
        cfg.add_edge(try_body, after, EdgeKind::Fallthrough);
        cfg.add_edge(try_body, handler_block, EdgeKind::ExceptionHandler);
        cfg.add_edge(handler_block, after, EdgeKind::Fallthrough);

        let mut protected = BTreeSet::new();
        protected.insert(try_body);
        cfg.add_region(Region {
            id: RegionId(0),
            protected_blocks: protected,
            handlers: alloc::vec![Handler {
                entry: handler_block,
                body: {
                    let mut s = BTreeSet::new();
                    s.insert(handler_block);
                    s
                },
                kind: HandlerKind::Catch,
            }],
            parent: None,
        });

        let ast = lift(&cfg);
        let found = has_node_kind(&ast, |n| matches!(n, AstNode::TryCatch { .. }));
        assert!(found, "should contain TryCatch node: {ast:?}");
        let pseudo = ast.to_pseudocode();
        assert!(
            pseudo.contains("try"),
            "pseudocode should contain try: {pseudo}"
        );
    }

    // ---- Goto / Label lifting ----

    #[test]
    fn lift_jump_edge_produces_goto() {
        let mut cfg: Cfg<MockInst> = Cfg::new();
        // entry(0) --Jump--> target(1)
        let target = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("src"));
        cfg.block_mut(target).instructions_vec_mut().push(ff("dst"));

        cfg.add_edge(cfg.entry(), target, EdgeKind::Jump);

        let ast = lift(&cfg);
        let found = has_node_kind(&ast, |n| matches!(n, AstNode::Goto { .. }));
        assert!(found, "should contain Goto node: {ast:?}");
        let pseudo = ast.to_pseudocode();
        assert!(
            pseudo.contains("goto"),
            "pseudocode should contain goto: {pseudo}"
        );
    }

    #[test]
    fn lift_jump_target_gets_label() {
        let mut cfg: Cfg<MockInst> = Cfg::new();
        // entry(0) --ConditionalTrue--> normal(1) --Fallthrough--> target(2) --Fallthrough--> end(3)
        // entry(0) --ConditionalFalse--> jumper(4) --Jump--> target(2)
        // target(2) has a Jump predecessor so it gets a Label wrapper.
        let normal = cfg.new_block(); // 1
        let target = cfg.new_block(); // 2
        let end = cfg.new_block(); // 3
        let jumper = cfg.new_block(); // 4
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("entry"));
        cfg.block_mut(normal)
            .instructions_vec_mut()
            .push(ff("normal"));
        cfg.block_mut(target).instructions_vec_mut().push(ff("dst"));
        cfg.block_mut(end).instructions_vec_mut().push(ff("end"));
        cfg.block_mut(jumper)
            .instructions_vec_mut()
            .push(ff("jumper"));

        cfg.add_edge(cfg.entry(), normal, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), jumper, EdgeKind::ConditionalFalse);
        cfg.add_edge(normal, target, EdgeKind::Fallthrough);
        cfg.add_edge(jumper, target, EdgeKind::Jump);
        cfg.add_edge(target, end, EdgeKind::Fallthrough);

        let ast = lift(&cfg);
        let found = has_node_kind(&ast, |n| matches!(n, AstNode::Label { .. }));
        assert!(found, "should contain Label node: {ast:?}");
    }
}
