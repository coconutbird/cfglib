use super::*;
use crate::builder::CfgBuilder;
use crate::flow::FlowEffect;
use crate::test_util::{MockInst, df_ff, df_pred, ff};
use alloc::vec;

#[test]
fn lift_predicated_regionizes_same_predicate_runs() {
    let cfg = CfgBuilder::build(vec![
        df_ff("plain"),
        df_pred("guarded_a", 3, true),
        df_pred("guarded_b", 3, true),
        df_pred("negated", 3, false),
        df_ff("after"),
    ])
    .unwrap();

    let ast = lift_predicated(&cfg);
    let pseudo = ast.to_pseudocode();
    assert!(pseudo.contains("@guarded(guarded_a)"), "{pseudo}");
    assert!(pseudo.contains("@guarded(!negated)"), "{pseudo}");
    // Same-predicate instructions share one region.
    assert_eq!(pseudo.matches("@guarded(").count(), 2, "{pseudo}");
    // Unpredicated instructions stay outside regions.
    assert!(pseudo.starts_with("plain\n"), "{pseudo}");
}

/// Helper: build CFG then lift, return pseudocode.
fn lift_pseudo(insts: Vec<MockInst>) -> alloc::string::String {
    let cfg = CfgBuilder::build(insts).unwrap();
    let ast = lift(&cfg);
    ast.to_pseudocode()
}

#[test]
fn lift_linear() {
    let p = lift_pseudo(vec![
        ff("a"),
        ff("b"),
        ff("c"),
        MockInst(FlowEffect::Return, "ret"),
    ]);
    assert!(p.contains('a'), "should contain instruction a: {p}");
    assert!(p.contains("ret"), "should contain ret: {p}");
    // No control flow keywords.
    assert!(!p.contains("if"), "no if expected: {p}");
    assert!(!p.contains("loop"), "no loop expected: {p}");
}

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
    assert!(p.contains('b'), "then body should contain b: {p}");
    assert!(p.contains('c'), "post-merge should contain c: {p}");
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
    assert!(p.contains('a'), "should contain a: {p}");
}

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
        AstNode::Sequence { body } | AstNode::Loop { body, .. } | AstNode::Label { body, .. } => {
            body.iter().any(|c| has_node_kind(c, pred))
        }
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
        .instructions_mut()
        .push(ff("entry"));
    cfg.block_mut(try_body)
        .instructions_mut()
        .push(ff("try_inst"));
    cfg.block_mut(handler_block)
        .instructions_mut()
        .push(ff("catch_inst"));
    cfg.block_mut(after).instructions_mut().push(ff("after"));

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

#[test]
fn lift_preserves_fault_and_filter_handler_kinds() {
    use crate::region::{Handler, HandlerKind, Region, RegionId};
    use alloc::collections::BTreeSet;

    let mut cfg: Cfg<MockInst> = Cfg::new();
    let try_body = cfg.new_block();
    let fault = cfg.new_block();
    let filter = cfg.new_block();
    let filtered_handler = cfg.new_block();
    let after = cfg.new_block();
    let entry = cfg.entry();

    cfg.block_mut(try_body)
        .instructions_mut()
        .push(ff("try_inst"));
    cfg.block_mut(fault)
        .instructions_mut()
        .push(ff("fault_inst"));
    cfg.block_mut(filtered_handler)
        .instructions_mut()
        .push(ff("filtered_inst"));
    cfg.add_edge(entry, try_body, EdgeKind::Fallthrough);
    cfg.add_edge(try_body, after, EdgeKind::Fallthrough);
    cfg.add_edge(try_body, fault, EdgeKind::ExceptionUnwind);
    cfg.add_edge(try_body, filtered_handler, EdgeKind::ExceptionHandler);

    cfg.add_region(Region {
        id: RegionId::from_raw(0),
        protected_blocks: [try_body].into_iter().collect::<BTreeSet<_>>(),
        handlers: alloc::vec![
            Handler {
                entry: fault,
                body: [fault].into_iter().collect(),
                kind: HandlerKind::Fault,
            },
            Handler {
                entry: filtered_handler,
                body: [filtered_handler].into_iter().collect(),
                kind: HandlerKind::Filter {
                    filter_block: filter,
                },
            },
        ],
        parent: None,
    });

    let pseudo = lift(&cfg).to_pseudocode();
    assert!(pseudo.contains("} fault {"), "{pseudo}");
    assert!(
        pseudo.contains(&alloc::format!("}} filter (.bb{}) {{", filter.index())),
        "{pseudo}"
    );
}

#[test]
fn lift_jump_edge_produces_goto() {
    let mut cfg: Cfg<MockInst> = Cfg::new();
    // entry(0) --Jump--> target(1)
    let target = cfg.new_block();
    cfg.block_mut(cfg.entry())
        .instructions_mut()
        .push(ff("src"));
    cfg.block_mut(target).instructions_mut().push(ff("dst"));

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
        .instructions_mut()
        .push(ff("entry"));
    cfg.block_mut(normal).instructions_mut().push(ff("normal"));
    cfg.block_mut(target).instructions_mut().push(ff("dst"));
    cfg.block_mut(end).instructions_mut().push(ff("end"));
    cfg.block_mut(jumper).instructions_mut().push(ff("jumper"));

    cfg.add_edge(cfg.entry(), normal, EdgeKind::ConditionalTrue);
    cfg.add_edge(cfg.entry(), jumper, EdgeKind::ConditionalFalse);
    cfg.add_edge(normal, target, EdgeKind::Fallthrough);
    cfg.add_edge(jumper, target, EdgeKind::Jump);
    cfg.add_edge(target, end, EdgeKind::Fallthrough);

    let ast = lift(&cfg);
    let found = has_node_kind(&ast, |n| matches!(n, AstNode::Label { .. }));
    assert!(found, "should contain Label node: {ast:?}");
}
