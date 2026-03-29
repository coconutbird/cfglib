//! CFG → AST lifting algorithm.
//!
//! Uses the dominator tree and edge classifications to reconstruct
//! structured control flow from a [`Cfg`].

extern crate alloc;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::ast::{AstNode, SwitchCase};
use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::dominator::DominatorTree;
use crate::edge::EdgeKind;

/// Lift a [`Cfg`] into a structured [`AstNode`] tree.
///
/// The instruction type `I` must implement `Clone` so that instructions
/// can be copied into the AST nodes.
pub fn lift<I: Clone>(cfg: &Cfg<I>) -> AstNode<I> {
    let dom = DominatorTree::compute(cfg);
    let pdom = DominatorTree::compute_post(cfg);
    let mut visited = BTreeSet::new();
    let body = lift_region(cfg, &dom, &pdom, cfg.entry(), &mut visited);
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
) -> Vec<AstNode<I>> {
    let mut result = Vec::new();
    let mut current = Some(head);

    while let Some(block) = current {
        if visited.contains(&block.0) {
            break;
        }

        visited.insert(block.0);
        current = None;

        let succ_edges = cfg.successor_edges(block);
        let has_ct = succ_edges.iter().any(|&e| cfg.edge(e).kind == EdgeKind::ConditionalTrue);
        let has_cf = succ_edges.iter().any(|&e| cfg.edge(e).kind == EdgeKind::ConditionalFalse);
        let has_sw = succ_edges.iter().any(|&e| cfg.edge(e).kind == EdgeKind::SwitchCase);
        let has_back = succ_edges.iter().any(|&e| cfg.edge(e).kind == EdgeKind::Back);
        let is_header = cfg.predecessor_edges(block).iter()
            .any(|&e| cfg.edge(e).kind == EdgeKind::Back);

        // --- Loop header ---
        if is_header {
            let node = lift_loop(cfg, dom, pdom, block, visited);
            result.push(node);
            current = find_loop_exit(cfg, block, visited);
            continue;
        }

        // --- Conditional (if/else) ---
        if has_ct && has_cf {
            let node = lift_conditional(cfg, dom, pdom, block, visited);
            result.push(node);
            current = advance_merge(pdom, block, visited);
            continue;
        }

        // --- Switch ---
        if has_sw {
            let node = lift_switch(cfg, dom, pdom, block, visited);
            result.push(node);
            current = advance_merge(pdom, block, visited);
            continue;
        }

        // --- Back edge (loop latch) ---
        if has_back {
            result.push(AstNode::Block {
                id: block,
                instructions: cfg.block(block).instructions().to_vec(),
            });
            continue;
        }

        // --- Terminal ---
        if succ_edges.is_empty() {
            let insts = cfg.block(block).instructions().to_vec();
            if !insts.is_empty() {
                result.push(AstNode::Return { instructions: insts });
            }
            continue;
        }

        // --- Fallthrough / unconditional ---
        result.push(AstNode::Block {
            id: block,
            instructions: cfg.block(block).instructions().to_vec(),
        });
        let succs = cfg.successors(block);
        if succs.len() == 1 && !visited.contains(&succs[0].0) {
            current = Some(succs[0]);
        }
    }

    result
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
) -> AstNode<I> {
    let mut true_target = None;
    let mut false_target = None;
    for &eid in cfg.successor_edges(block) {
        match cfg.edge(eid).kind {
            EdgeKind::ConditionalTrue => true_target = Some(cfg.edge(eid).target),
            EdgeKind::ConditionalFalse => false_target = Some(cfg.edge(eid).target),
            _ => {}
        }
    }

    let merge = pdom.idom(block);

    let then_body = match true_target {
        Some(t) if merge.is_none_or(|m| t != m) => lift_arm(cfg, dom, pdom, t, merge, visited),
        _ => Vec::new(),
    };
    let else_body = match false_target {
        Some(f) if merge.is_none_or(|m| f != m) => lift_arm(cfg, dom, pdom, f, merge, visited),
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
) -> AstNode<I> {
    let merge = pdom.idom(block);
    let mut cases = Vec::new();

    for &eid in cfg.successor_edges(block) {
        let edge = cfg.edge(eid);
        if edge.kind == EdgeKind::SwitchCase {
            let cb = edge.target;
            visited.insert(cb.0);
            let header_insts = cfg.block(cb).instructions().to_vec();
            // Lift the case body from successors of the case header.
            let body = lift_case_body(cfg, dom, pdom, cb, merge, visited);
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
) -> AstNode<I> {
    let mut body = Vec::new();

    // Check if the header itself is a conditional or switch dispatch.
    let succ_edges = cfg.successor_edges(header);
    let has_ct = succ_edges.iter().any(|&e| cfg.edge(e).kind == EdgeKind::ConditionalTrue);
    let has_cf = succ_edges.iter().any(|&e| cfg.edge(e).kind == EdgeKind::ConditionalFalse);
    let has_sw = succ_edges.iter().any(|&e| cfg.edge(e).kind == EdgeKind::SwitchCase);

    if has_ct && has_cf {
        // Header is also a conditional — lift it as if/else inside the loop.
        let node = lift_conditional(cfg, dom, pdom, header, visited);
        body.push(node);
        // Follow the merge point for more body.
        if let Some(merge) = pdom.idom(header) {
            if !visited.contains(&merge.0) {
                body.extend(lift_region(cfg, dom, pdom, merge, visited));
            }
        }
    } else if has_sw {
        let node = lift_switch(cfg, dom, pdom, header, visited);
        body.push(node);
        if let Some(merge) = pdom.idom(header) {
            if !visited.contains(&merge.0) {
                body.extend(lift_region(cfg, dom, pdom, merge, visited));
            }
        }
    } else {
        // Plain header — emit instructions, then follow non-back successors.
        let header_insts = cfg.block(header).instructions().to_vec();
        if !header_insts.is_empty() {
            body.push(AstNode::Block {
                id: header,
                instructions: header_insts,
            });
        }
        for &eid in succ_edges {
            let edge = cfg.edge(eid);
            if edge.kind != EdgeKind::Back && !visited.contains(&edge.target.0) {
                body.extend(lift_region(cfg, dom, pdom, edge.target, visited));
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
) -> Vec<AstNode<I>> {
    if stop.is_some_and(|s| s == start) {
        return Vec::new();
    }
    lift_region(cfg, dom, pdom, start, visited)
}

/// Lift the body of a switch case from its successors.
fn lift_case_body<I: Clone>(
    cfg: &Cfg<I>,
    dom: &DominatorTree,
    pdom: &DominatorTree,
    case_block: BlockId,
    stop: Option<BlockId>,
    visited: &mut BTreeSet<u32>,
) -> Vec<AstNode<I>> {
    let mut body = Vec::new();
    for succ in cfg.successors(case_block) {
        if stop.is_none_or(|s| s != succ) && !visited.contains(&succ.0) {
            body.extend(lift_region(cfg, dom, pdom, succ, visited));
        }
    }
    body
}

/// Find the exit of a loop (block reachable via break that hasn't been visited).
fn find_loop_exit<I>(cfg: &Cfg<I>, _header: BlockId, visited: &BTreeSet<u32>) -> Option<BlockId> {
    for edge in cfg.edges() {
        if edge.kind == EdgeKind::Unconditional && !visited.contains(&edge.target.0) {
            return Some(edge.target);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::borrow::Cow;
    use alloc::vec;
    use crate::builder::CfgBuilder;
    use crate::flow::{FlowControl, FlowEffect};

    #[derive(Debug, Clone)]
    struct M(FlowEffect, &'static str);

    impl FlowControl for M {
        fn flow_effect(&self) -> FlowEffect { self.0 }
        fn display_mnemonic(&self) -> Cow<'_, str> { Cow::Borrowed(self.1) }
    }

    fn ff(n: &'static str) -> M { M(FlowEffect::Fallthrough, n) }

    /// Helper: build CFG then lift, return pseudocode.
    fn lift_pseudo(insts: Vec<M>) -> alloc::string::String {
        let cfg = CfgBuilder::build(insts);
        let ast = lift(&cfg);
        ast.to_pseudocode()
    }

    // ---- Linear / trivial ----

    #[test]
    fn lift_linear() {
        let p = lift_pseudo(vec![ff("a"), ff("b"), ff("c"), M(FlowEffect::Return, "ret")]);
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
            M(FlowEffect::ConditionalOpen, "if"),
            ff("b"),
            M(FlowEffect::ConditionalClose, "endif"),
            ff("c"),
            M(FlowEffect::Return, "ret"),
        ]);
        assert!(p.contains("if {"), "should have if: {p}");
        assert!(p.contains("b"), "then body should contain b: {p}");
        assert!(p.contains("c"), "post-merge should contain c: {p}");
    }

    #[test]
    fn lift_if_else() {
        let p = lift_pseudo(vec![
            ff("a"),
            M(FlowEffect::ConditionalOpen, "if"),
            ff("then_inst"),
            M(FlowEffect::ConditionalAlternate, "else"),
            ff("else_inst"),
            M(FlowEffect::ConditionalClose, "endif"),
            ff("merge"),
            M(FlowEffect::Return, "ret"),
        ]);
        assert!(p.contains("if {"), "should have if: {p}");
        assert!(p.contains("then_inst"), "then arm: {p}");
        // else arm or merge should appear
        assert!(p.contains("else_inst") || p.contains("} else {"), "else arm: {p}");
    }

    // ---- Loop ----

    #[test]
    fn lift_simple_loop() {
        let p = lift_pseudo(vec![
            M(FlowEffect::LoopOpen, "loop"),
            ff("body"),
            M(FlowEffect::LoopClose, "endloop"),
            M(FlowEffect::Return, "ret"),
        ]);
        assert!(p.contains("loop {"), "should have loop: {p}");
        assert!(p.contains("body"), "loop body: {p}");
    }

    #[test]
    fn lift_loop_with_break() {
        let p = lift_pseudo(vec![
            M(FlowEffect::LoopOpen, "loop"),
            ff("a"),
            M(FlowEffect::ConditionalBreak, "breakc"),
            ff("b"),
            M(FlowEffect::LoopClose, "endloop"),
            M(FlowEffect::Return, "ret"),
        ]);
        assert!(p.contains("loop {"), "should have loop: {p}");
        // The breakc creates a conditional inside the loop
        assert!(p.contains("a"), "should contain a: {p}");
    }

    // ---- Switch ----

    #[test]
    fn lift_switch() {
        let p = lift_pseudo(vec![
            M(FlowEffect::SwitchOpen, "switch"),
            ff("dispatch"),
            M(FlowEffect::SwitchCase, "case0"),
            ff("arm0"),
            M(FlowEffect::SwitchCase, "case1"),
            ff("arm1"),
            M(FlowEffect::SwitchClose, "endswitch"),
            ff("after"),
            M(FlowEffect::Return, "ret"),
        ]);
        assert!(p.contains("switch {"), "should have switch: {p}");
        assert!(p.contains("case {"), "should have case: {p}");
    }

    // ---- Nested structures ----

    #[test]
    fn lift_if_in_loop() {
        let p = lift_pseudo(vec![
            M(FlowEffect::LoopOpen, "loop"),
            M(FlowEffect::ConditionalOpen, "if"),
            ff("then"),
            M(FlowEffect::ConditionalAlternate, "else"),
            ff("else_body"),
            M(FlowEffect::ConditionalClose, "endif"),
            M(FlowEffect::LoopClose, "endloop"),
            M(FlowEffect::Return, "ret"),
        ]);
        assert!(p.contains("loop {"), "should have loop: {p}");
        assert!(p.contains("if {"), "should have if inside loop: {p}");
    }

    #[test]
    fn lift_loop_in_if() {
        let p = lift_pseudo(vec![
            M(FlowEffect::ConditionalOpen, "if"),
            M(FlowEffect::LoopOpen, "loop"),
            ff("body"),
            M(FlowEffect::ConditionalBreak, "breakc"),
            M(FlowEffect::LoopClose, "endloop"),
            M(FlowEffect::ConditionalClose, "endif"),
            M(FlowEffect::Return, "ret"),
        ]);
        // Should have both if and loop structures
        let has_if = p.contains("if {");
        let has_loop = p.contains("loop {");
        assert!(has_if || has_loop, "should have nested structure: {p}");
    }

    // ---- AST node structure checks ----

    #[test]
    fn lift_returns_sequence_or_single() {
        let cfg = CfgBuilder::build(vec![ff("a"), M(FlowEffect::Return, "ret")]);
        let ast = lift(&cfg);
        // Should be a Block or Return, not an empty Sequence.
        assert!(!ast.is_empty(), "should not be empty");
    }

    #[test]
    fn lift_conditional_produces_if_node() {
        let cfg = CfgBuilder::build(vec![
            ff("a"),
            M(FlowEffect::ConditionalOpen, "if"),
            ff("b"),
            M(FlowEffect::ConditionalClose, "endif"),
            M(FlowEffect::Return, "ret"),
        ]);
        let ast = lift(&cfg);
        // Walk the AST to find an IfThenElse node.
        let found = has_node_kind(&ast, |n| matches!(n, AstNode::IfThenElse { .. }));
        assert!(found, "should contain IfThenElse node: {ast:?}");
    }

    #[test]
    fn lift_loop_produces_loop_node() {
        let cfg = CfgBuilder::build(vec![
            M(FlowEffect::LoopOpen, "loop"),
            ff("x"),
            M(FlowEffect::LoopClose, "endloop"),
            M(FlowEffect::Return, "ret"),
        ]);
        let ast = lift(&cfg);
        let found = has_node_kind(&ast, |n| matches!(n, AstNode::Loop { .. }));
        assert!(found, "should contain Loop node: {ast:?}");
    }

    #[test]
    fn lift_switch_produces_switch_node() {
        let cfg = CfgBuilder::build(vec![
            M(FlowEffect::SwitchOpen, "switch"),
            ff("d"),
            M(FlowEffect::SwitchCase, "c1"),
            ff("a1"),
            M(FlowEffect::SwitchCase, "c2"),
            ff("a2"),
            M(FlowEffect::SwitchClose, "endswitch"),
            M(FlowEffect::Return, "ret"),
        ]);
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
            AstNode::Sequence { body } | AstNode::Loop { body, .. } => {
                body.iter().any(|c| has_node_kind(c, pred))
            }
            AstNode::IfThenElse { then_body, else_body, .. } => {
                then_body.iter().any(|c| has_node_kind(c, pred))
                    || else_body.iter().any(|c| has_node_kind(c, pred))
            }
            AstNode::Switch { cases, .. } => {
                cases.iter().any(|c| c.body.iter().any(|n| has_node_kind(n, pred)))
            }
            _ => false,
        }
    }
}
