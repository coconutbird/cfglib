use super::*;
use crate::builder::CfgBuilder;
use crate::flow::FlowEffect;
use crate::ir::ast::{LiftReport, LoopKind};
use crate::test_util::{MockInst, df_ff, df_pred, ff};
use alloc::vec;
use alloc::vec::Vec;

/// Check whether any node in the AST matches a predicate, using the
/// exhaustive public preorder walk.
fn has_node_kind<I>(node: &AstNode<I>, pred: impl Fn(&AstNode<I>) -> bool) -> bool {
    let mut found = false;
    node.visit(&mut |candidate| {
        found = found || pred(candidate);
    });
    found
}

fn find_try_handlers<I>(node: &AstNode<I>) -> Option<&[crate::ir::ast::CatchHandler<I>]> {
    find_try_catch(node).and_then(|found| match found {
        AstNode::TryCatch { handlers, .. } => Some(handlers.as_slice()),
        _ => None,
    })
}

fn find_try_catch<I>(node: &AstNode<I>) -> Option<&AstNode<I>> {
    let mut found = None;
    node.visit(&mut |candidate| {
        if found.is_none() && matches!(candidate, AstNode::TryCatch { .. }) {
            found = Some(candidate);
        }
    });
    found
}

fn contains_block<I>(node: &AstNode<I>, block: BlockId) -> bool {
    has_node_kind(
        node,
        |candidate| matches!(candidate, AstNode::Block { id, .. } | AstNode::Return { id, .. } if *id == block),
    )
}

fn has_goto_to<I, E>(node: &AstNode<I>, cfg: &Cfg<I, E>, block: BlockId) -> bool {
    let name = super::block_label_name(cfg, block);
    has_node_kind(
        node,
        |candidate| matches!(candidate, AstNode::Goto { target } if *target == name),
    )
}

fn find_loops<I: Clone>(node: &AstNode<I>) -> Vec<AstNode<I>> {
    let mut found = Vec::new();
    node.visit(&mut |candidate| {
        if matches!(candidate, AstNode::Loop { .. }) {
            found.push(candidate.clone());
        }
    });
    found
}

/// A machine-shaped while loop:
/// entry → header{cond}; header --true--> body --fallthrough--> header;
/// header --false--> exit{return}.
fn while_shaped_cfg() -> (Cfg<MockInst>, BlockId, BlockId, BlockId) {
    let mut cfg: Cfg<MockInst> = Cfg::new();
    let header = cfg.new_block();
    let body = cfg.new_block();
    let exit = cfg.new_block();
    cfg.block_mut(cfg.entry()).push(ff("entry"));
    cfg.block_mut(header).push(ff("condition"));
    cfg.block_mut(body).push(ff("body_inst"));
    cfg.block_mut(exit)
        .push(MockInst(FlowEffect::Return, "after_loop"));
    cfg.add_edge(cfg.entry(), header, EdgeKind::Fallthrough);
    cfg.add_edge(header, body, EdgeKind::ConditionalTrue);
    cfg.add_edge(header, exit, EdgeKind::ConditionalFalse);
    cfg.add_edge(body, header, EdgeKind::Jump);
    (cfg, header, body, exit)
}

#[test]
fn machine_while_loop_is_classified_pre_tested() {
    let (cfg, header, body, exit) = while_shaped_cfg();
    let (ast, report) = lift_with_report(&cfg);
    let loops = find_loops(&ast);
    assert_eq!(loops.len(), 1, "{ast:?}");
    match &loops[0] {
        AstNode::Loop {
            header: found_header,
            kind: LoopKind::While { exit_on_true, .. },
            body: loop_body,
        } => {
            assert_eq!(*found_header, header);
            assert!(!exit_on_true, "the true edge iterates: {ast:?}");
            let inner = AstNode::Sequence {
                body: loop_body.clone(),
            };
            assert!(contains_block(&inner, body));
            assert!(
                !contains_block(&inner, exit),
                "the loop exit may not be absorbed into the body: {ast:?}"
            );
        }
        other => panic!("expected a pre-tested loop, got {other:?}"),
    }
    assert!(
        contains_block(&ast, exit),
        "the continuation follows the loop: {ast:?}"
    );
    assert!(report.is_fully_structured(), "{report:?}");
}

#[test]
fn machine_do_while_loop_is_classified_post_tested() {
    // entry → body; body → latch{cond}; latch --true--> body; --false--> exit.
    let mut cfg: Cfg<MockInst> = Cfg::new();
    let body = cfg.new_block();
    let latch = cfg.new_block();
    let exit = cfg.new_block();
    cfg.block_mut(cfg.entry()).push(ff("entry"));
    cfg.block_mut(body).push(ff("body_inst"));
    cfg.block_mut(latch).push(ff("latch_cond"));
    cfg.block_mut(exit)
        .push(MockInst(FlowEffect::Return, "after_loop"));
    cfg.add_edge(cfg.entry(), body, EdgeKind::Fallthrough);
    cfg.add_edge(body, latch, EdgeKind::Fallthrough);
    cfg.add_edge(latch, body, EdgeKind::ConditionalTrue);
    cfg.add_edge(latch, exit, EdgeKind::ConditionalFalse);

    let (ast, report) = lift_with_report(&cfg);
    let loops = find_loops(&ast);
    assert_eq!(loops.len(), 1, "{ast:?}");
    match &loops[0] {
        AstNode::Loop {
            kind:
                LoopKind::DoWhile {
                    latch: found_latch,
                    continue_on_true,
                    ..
                },
            body: loop_body,
            ..
        } => {
            assert_eq!(*found_latch, latch);
            assert!(continue_on_true, "{ast:?}");
            let inner = AstNode::Sequence {
                body: loop_body.clone(),
            };
            assert!(contains_block(&inner, body));
            assert!(
                !contains_block(&inner, latch),
                "the latch is the condition, not body: {ast:?}"
            );
        }
        other => panic!("expected a post-tested loop, got {other:?}"),
    }
    assert!(contains_block(&ast, exit), "{ast:?}");
    assert!(report.is_fully_structured(), "{report:?}");
}

#[test]
fn loop_exit_from_the_body_becomes_a_break() {
    // while-shaped loop whose body also exits conditionally (a break).
    let mut cfg: Cfg<MockInst> = Cfg::new();
    let header = cfg.new_block();
    let body = cfg.new_block();
    let tail = cfg.new_block();
    let exit = cfg.new_block();
    cfg.block_mut(cfg.entry()).push(ff("entry"));
    cfg.block_mut(header).push(ff("condition"));
    cfg.block_mut(body).push(ff("maybe_break"));
    cfg.block_mut(tail).push(ff("tail_inst"));
    cfg.block_mut(exit)
        .push(MockInst(FlowEffect::Return, "after_loop"));
    cfg.add_edge(cfg.entry(), header, EdgeKind::Fallthrough);
    cfg.add_edge(header, body, EdgeKind::ConditionalTrue);
    cfg.add_edge(header, exit, EdgeKind::ConditionalFalse);
    cfg.add_edge(body, exit, EdgeKind::ConditionalTrue);
    cfg.add_edge(body, tail, EdgeKind::ConditionalFalse);
    cfg.add_edge(tail, header, EdgeKind::Jump);

    let (ast, report) = lift_with_report(&cfg);
    assert!(
        has_node_kind(&ast, |node| matches!(node, AstNode::Break { label: None })),
        "the in-body exit resolves to a break: {ast:?}"
    );
    assert!(
        !has_node_kind(&ast, |node| matches!(node, AstNode::Goto { .. })),
        "{ast:?}"
    );
    assert!(report.is_fully_structured(), "{report:?}");
}

#[test]
fn multi_level_exit_becomes_a_labeled_break() {
    // outer: header_o conditional-exits; inner loop's body exits BOTH loops.
    let mut cfg: Cfg<MockInst> = Cfg::new();
    let outer_header = cfg.new_block();
    let inner_header = cfg.new_block();
    let inner_body = cfg.new_block();
    let outer_latch = cfg.new_block();
    let exit = cfg.new_block();
    cfg.block_mut(cfg.entry()).push(ff("entry"));
    cfg.block_mut(outer_header).push(ff("outer_cond"));
    cfg.block_mut(inner_header).push(ff("inner_cond"));
    cfg.block_mut(inner_body).push(ff("inner_inst"));
    cfg.block_mut(outer_latch).push(ff("outer_latch_inst"));
    cfg.block_mut(exit)
        .push(MockInst(FlowEffect::Return, "after"));
    cfg.add_edge(cfg.entry(), outer_header, EdgeKind::Fallthrough);
    cfg.add_edge(outer_header, inner_header, EdgeKind::ConditionalTrue);
    cfg.add_edge(outer_header, exit, EdgeKind::ConditionalFalse);
    cfg.add_edge(inner_header, inner_body, EdgeKind::ConditionalTrue);
    cfg.add_edge(inner_header, outer_latch, EdgeKind::ConditionalFalse);
    // The inner body either exits everything or continues the inner loop.
    cfg.add_edge(inner_body, exit, EdgeKind::ConditionalTrue);
    cfg.add_edge(inner_body, inner_header, EdgeKind::ConditionalFalse);
    cfg.add_edge(outer_latch, outer_header, EdgeKind::Jump);

    let (ast, report) = lift_with_report(&cfg);
    assert!(
        has_node_kind(&ast, |node| matches!(
            node,
            AstNode::Break { label: Some(_) }
        )),
        "the two-level exit is a labeled break: {ast:?}"
    );
    assert!(
        has_node_kind(&ast, |node| matches!(node, AstNode::Label { .. })),
        "the outer loop carries the label: {ast:?}"
    );
    assert!(
        !has_node_kind(&ast, |node| matches!(node, AstNode::Goto { .. })),
        "{ast:?}"
    );
    assert!(report.is_fully_structured(), "{report:?}");
}

#[test]
fn if_else_merge_is_not_absorbed_into_an_arm() {
    let mut cfg: Cfg<MockInst> = Cfg::new();
    let then_arm = cfg.new_block();
    let else_arm = cfg.new_block();
    let merge = cfg.new_block();
    cfg.block_mut(cfg.entry()).push(ff("cond"));
    cfg.block_mut(then_arm).push(ff("then_inst"));
    cfg.block_mut(else_arm).push(ff("else_inst"));
    cfg.block_mut(merge)
        .push(MockInst(FlowEffect::Return, "merge_inst"));
    cfg.add_edge(cfg.entry(), then_arm, EdgeKind::ConditionalTrue);
    cfg.add_edge(cfg.entry(), else_arm, EdgeKind::ConditionalFalse);
    cfg.add_edge(then_arm, merge, EdgeKind::Fallthrough);
    cfg.add_edge(else_arm, merge, EdgeKind::Fallthrough);

    let ast = lift(&cfg);
    let mut checked = false;
    ast.visit(&mut |node| {
        if let AstNode::IfThenElse {
            then_body,
            else_body,
            ..
        } = node
        {
            checked = true;
            let then_all = AstNode::Sequence {
                body: then_body.clone(),
            };
            let else_all = AstNode::Sequence {
                body: else_body.clone(),
            };
            assert!(contains_block(&then_all, then_arm), "{node:?}");
            assert!(contains_block(&else_all, else_arm), "{node:?}");
            assert!(
                !contains_block(&then_all, merge) && !contains_block(&else_all, merge),
                "the merge belongs after the conditional, not in an arm: {node:?}"
            );
        }
    });
    assert!(checked, "expected an IfThenElse: {ast:?}");
    assert!(
        contains_block(&ast, merge),
        "the merge follows the conditional: {ast:?}"
    );
}

#[test]
fn switch_records_case_edges_and_default_arm() {
    let mut cfg: Cfg<MockInst> = Cfg::new();
    let case_a = cfg.new_block();
    let case_b = cfg.new_block();
    let default_arm = cfg.new_block();
    let merge = cfg.new_block();
    cfg.block_mut(cfg.entry()).push(ff("dispatch"));
    cfg.block_mut(case_a).push(ff("case_a_inst"));
    cfg.block_mut(case_b).push(ff("case_b_inst"));
    cfg.block_mut(default_arm).push(ff("default_inst"));
    cfg.block_mut(merge)
        .push(MockInst(FlowEffect::Return, "after"));
    let edge_a = cfg.add_edge(cfg.entry(), case_a, EdgeKind::SwitchCase);
    let edge_a2 = cfg.add_edge(cfg.entry(), case_a, EdgeKind::SwitchCase);
    let edge_b = cfg.add_edge(cfg.entry(), case_b, EdgeKind::SwitchCase);
    let default_edge = cfg.add_edge(cfg.entry(), default_arm, EdgeKind::Fallthrough);
    cfg.add_edge(case_a, merge, EdgeKind::Unconditional);
    cfg.add_edge(case_b, merge, EdgeKind::Unconditional);
    cfg.add_edge(default_arm, merge, EdgeKind::Unconditional);

    let (ast, report) = lift_with_report(&cfg);
    let mut checked = false;
    ast.visit(&mut |node| {
        if let AstNode::Switch {
            cases,
            default_body,
            default_edge: found_default,
            ..
        } = node
        {
            checked = true;
            assert_eq!(cases.len(), 2, "{node:?}");
            assert_eq!(cases[0].id, case_a);
            assert_eq!(cases[0].edges, vec![edge_a, edge_a2], "{node:?}");
            assert_eq!(cases[1].edges, vec![edge_b], "{node:?}");
            assert_eq!(*found_default, Some(default_edge), "{node:?}");
            let default_all = AstNode::Sequence {
                body: default_body.clone(),
            };
            assert!(
                contains_block(&default_all, default_arm),
                "the default arm is captured: {node:?}"
            );
        }
    });
    assert!(checked, "expected a Switch: {ast:?}");
    assert!(contains_block(&ast, merge), "{ast:?}");
    assert!(report.is_fully_structured(), "{report:?}");
}

#[test]
fn lift_accepts_caller_owned_edge_payloads() {
    let mut cfg = Cfg::<MockInst, &'static str>::with_edge_payload();
    let exit = cfg.new_block();
    cfg.block_mut(cfg.entry()).push(ff("entry"));
    cfg.block_mut(exit)
        .push(MockInst(FlowEffect::Return, "return"));
    cfg.add_edge_with_payload(cfg.entry(), exit, EdgeKind::Fallthrough, "source identity");

    let pseudo = lift(&cfg).to_pseudocode();
    assert!(pseudo.contains("entry"), "{pseudo}");
    assert!(pseudo.contains("return"), "{pseudo}");
}

#[test]
fn lift_recovers_a_payload_bearing_loop_from_ordinary_jump_edges() {
    let mut cfg = Cfg::<MockInst, &'static str>::with_edge_payload();
    let header = cfg.new_block();
    let body = cfg.new_block();
    let exit = cfg.new_block();
    cfg.block_mut(header).push(ff("condition"));
    cfg.block_mut(body).push(ff("body"));
    cfg.block_mut(exit)
        .push(MockInst(FlowEffect::Return, "return"));
    cfg.add_edge_with_payload(cfg.entry(), header, EdgeKind::Fallthrough, "entry");
    cfg.add_edge_with_payload(header, body, EdgeKind::ConditionalTrue, "true");
    cfg.add_edge_with_payload(header, exit, EdgeKind::ConditionalFalse, "false");
    cfg.add_edge_with_payload(body, header, EdgeKind::Jump, "native goto");

    let ast = lift(&cfg);
    assert!(
        has_node_kind(&ast, |node| matches!(node, AstNode::Loop { .. })),
        "dominance must recover a loop without rewriting its native jump edge: {ast:?}"
    );
    assert!(
        !has_node_kind(&ast, |node| matches!(
            node,
            AstNode::Label { .. } | AstNode::Goto { .. }
        )),
        "the natural loop should not degrade to labels and gotos: {ast:?}"
    );
}

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
    assert!(p.contains("body"), "loop body: {p}");
    assert!(
        p.contains("loop {") || p.contains("while {") || p.contains("do {"),
        "should have a loop: {p}"
    );
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
    assert!(p.contains('a'), "should contain a: {p}");
    assert!(p.contains("ret"), "the continuation survives: {p}");
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
    assert!(p.contains("arm0"), "{p}");
    assert!(p.contains("arm1"), "{p}");
    assert!(p.contains("after"), "{p}");
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
    assert!(p.contains("if {"), "should have if inside loop: {p}");
    assert!(p.contains("then"), "{p}");
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
    let has_if = p.contains("if {");
    let has_loop = p.contains("loop {") || p.contains("while {") || p.contains("do {");
    assert!(has_if || has_loop, "should have nested structure: {p}");
}

#[test]
fn lift_returns_sequence_or_single() {
    let cfg = CfgBuilder::build(vec![ff("a"), MockInst(FlowEffect::Return, "ret")]).unwrap();
    let ast = lift(&cfg);
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

#[test]
fn lift_jump_edge_produces_goto_and_reports_it() {
    let mut cfg: Cfg<MockInst> = Cfg::new();
    // entry(0) --Jump--> target(1); backward gotos have no loop context.
    let target = cfg.new_block();
    cfg.block_mut(cfg.entry())
        .instructions_mut()
        .push(ff("src"));
    cfg.block_mut(target).instructions_mut().push(ff("dst"));

    cfg.add_edge(cfg.entry(), target, EdgeKind::Jump);

    let (ast, report) = lift_with_report(&cfg);
    assert!(
        has_node_kind(&ast, |n| matches!(n, AstNode::Goto { .. })),
        "should contain Goto node: {ast:?}"
    );
    let pseudo = ast.to_pseudocode();
    assert!(pseudo.contains("goto"), "{pseudo}");
    assert_eq!(report.gotos.len(), 1, "{report:?}");
    assert_eq!(report.gotos[0].target, target);
    assert_eq!(report.gotos[0].reason, super::GotoReason::ExplicitJump);
    assert!(!report.is_fully_structured());
}

#[test]
fn lift_jump_target_gets_label() {
    let mut cfg: Cfg<MockInst> = Cfg::new();
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

    let (ast, report) = lift_with_report(&cfg);
    assert!(
        has_node_kind(&ast, |n| matches!(n, AstNode::Label { .. })),
        "should contain Label node: {ast:?}"
    );
    assert!(
        report.unresolved_labels.is_empty(),
        "the goto target is anchored: {report:?}"
    );
}

#[test]
fn structured_lifts_report_no_degradation() {
    let cfg = CfgBuilder::build(vec![
        ff("a"),
        MockInst(FlowEffect::ConditionalOpen, "if"),
        ff("b"),
        MockInst(FlowEffect::ConditionalClose, "endif"),
        MockInst(FlowEffect::Return, "ret"),
    ])
    .unwrap();
    let (_, report) = lift_with_report(&cfg);
    assert_eq!(report, LiftReport::default(), "{report:?}");
}

#[test]
fn map_instructions_preserves_structure() {
    let (cfg, header, ..) = while_shaped_cfg();
    let ast = lift(&cfg);
    let mut names = Vec::new();
    ast.for_each_instruction(&mut |inst: &MockInst| names.push(inst.1));
    assert!(names.contains(&"condition"), "{names:?}");
    assert!(names.contains(&"body_inst"), "{names:?}");

    let mapped = ast.map_instructions(&mut |inst| inst.1);
    let loops = {
        let mut found = Vec::new();
        mapped.visit(&mut |candidate| {
            if let AstNode::Loop {
                header: found_header,
                kind: LoopKind::While { condition, .. },
                ..
            } = candidate
            {
                found.push((*found_header, condition.clone()));
            }
        });
        found
    };
    assert_eq!(loops.len(), 1, "{mapped:?}");
    assert_eq!(loops[0].0, header);
    assert_eq!(loops[0].1, vec!["condition"], "{mapped:?}");
}

/// Exception-region lifting tests, split out to respect the source-size
/// policy.
mod regions;
