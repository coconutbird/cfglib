extern crate alloc;

use super::super::lift;
use super::{contains_block, find_try_catch, find_try_handlers, has_goto_to, has_node_kind};
use crate::cfg::Cfg;
use crate::edge::EdgeKind;
use crate::ir::ast::AstNode;
use crate::test_util::{MockInst, ff};

#[test]
fn lift_try_catch_produces_try_node() {
    use crate::region::{Handler, HandlerBody, HandlerKind, Region, RegionId};
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
            body: HandlerBody::known([handler_block]),
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
    let handlers = find_try_handlers(&ast).expect("known body produces structured handlers");
    assert!(
        handlers[0]
            .body
            .iter()
            .any(|node| contains_block(node, handler_block))
    );
    assert!(
        !handlers[0]
            .body
            .iter()
            .any(|node| contains_block(node, after)),
        "handler lifting must stop at the declared complete extent: {ast:?}"
    );
}

#[test]
fn lift_does_not_invent_structure_for_unknown_handler_body() {
    use crate::region::{Handler, HandlerBody, HandlerKind, Region, RegionId};

    let mut cfg: Cfg<MockInst> = Cfg::new();
    let try_body = cfg.new_block();
    let handler = cfg.new_block();
    let after = cfg.new_block();
    let entry = cfg.entry();

    cfg.block_mut(entry).instructions_mut().push(ff("entry"));
    cfg.block_mut(try_body)
        .instructions_mut()
        .push(ff("try_inst"));
    cfg.block_mut(handler)
        .instructions_mut()
        .push(ff("catch_inst"));
    cfg.block_mut(after).instructions_mut().push(ff("after"));
    cfg.add_edge(entry, try_body, EdgeKind::Fallthrough);
    cfg.add_edge(try_body, after, EdgeKind::Fallthrough);
    cfg.add_edge(try_body, handler, EdgeKind::ExceptionHandler);
    cfg.add_edge(handler, after, EdgeKind::Fallthrough);
    cfg.add_region(Region {
        id: RegionId::from_raw(0),
        protected_blocks: [try_body].into_iter().collect(),
        handlers: alloc::vec![Handler {
            entry: handler,
            body: HandlerBody::unknown(),
            kind: HandlerKind::Catch,
        }],
        parent: None,
    });

    let ast = lift(&cfg);
    assert!(
        !has_node_kind(&ast, |node| matches!(node, AstNode::TryCatch { .. })),
        "unknown extents must remain unstructured: {ast:?}"
    );
    assert!(
        contains_block(&ast, try_body),
        "ordinary CFG lifting still retains the protected block: {ast:?}"
    );
    assert!(
        contains_block(&ast, after),
        "the normal continuation survives an unknown extent: {ast:?}"
    );
    assert!(
        contains_block(&ast, handler),
        "the handler code survives an unknown extent: {ast:?}"
    );
}

#[test]
fn lift_preserves_fault_and_filter_handler_kinds() {
    use crate::region::{Handler, HandlerBody, HandlerKind, Region, RegionId};
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
                body: HandlerBody::known([fault]),
                kind: HandlerKind::Fault,
            },
            Handler {
                entry: filtered_handler,
                body: HandlerBody::known([filtered_handler]),
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
fn inner_unknown_region_does_not_suppress_an_outer_known_one() {
    use crate::region::{Handler, HandlerBody, HandlerKind, Region, RegionId};

    let mut cfg: Cfg<MockInst> = Cfg::new();
    let b0 = cfg.new_block();
    let b1 = cfg.new_block();
    let known_handler = cfg.new_block();
    let unknown_handler = cfg.new_block();
    let after = cfg.new_block();
    cfg.block_mut(cfg.entry()).push(ff("entry"));
    cfg.block_mut(b0).push(ff("b0_inst"));
    cfg.block_mut(b1).push(ff("b1_inst"));
    cfg.block_mut(known_handler).push(ff("known_catch"));
    cfg.block_mut(unknown_handler).push(ff("unknown_catch"));
    cfg.block_mut(after).push(ff("after"));
    cfg.add_edge(cfg.entry(), b0, EdgeKind::Fallthrough);
    cfg.add_edge(b0, b1, EdgeKind::Fallthrough);
    cfg.add_edge(b1, after, EdgeKind::Fallthrough);
    cfg.add_edge(b1, known_handler, EdgeKind::ExceptionHandler);
    cfg.add_edge(known_handler, after, EdgeKind::Fallthrough);
    cfg.add_edge(b0, unknown_handler, EdgeKind::ExceptionHandler);
    cfg.add_edge(unknown_handler, after, EdgeKind::Fallthrough);

    let outer_id = cfg.add_region(Region {
        id: RegionId::from_raw(0),
        protected_blocks: [b0, b1].into_iter().collect(),
        handlers: alloc::vec![Handler {
            entry: known_handler,
            body: HandlerBody::known([known_handler]),
            kind: HandlerKind::Catch,
        }],
        parent: None,
    });
    cfg.add_region(Region {
        id: RegionId::from_raw(0),
        protected_blocks: [b0].into_iter().collect(),
        handlers: alloc::vec![Handler {
            entry: unknown_handler,
            body: HandlerBody::unknown(),
            kind: HandlerKind::Catch,
        }],
        parent: Some(outer_id),
    });

    let ast = lift(&cfg);
    assert!(
        has_node_kind(&ast, |node| matches!(node, AstNode::TryCatch { .. })),
        "the complete outer region still structures: {ast:?}"
    );
    for block in [b0, b1, known_handler, unknown_handler, after] {
        assert!(
            contains_block(&ast, block),
            "no block may be dropped: {block:?} in {ast:?}"
        );
    }
}

#[test]
fn conditional_leaving_a_region_records_the_exit() {
    use crate::region::{Handler, HandlerBody, HandlerKind, Region, RegionId};

    let mut cfg: Cfg<MockInst> = Cfg::new();
    let t0 = cfg.new_block();
    let t1 = cfg.new_block();
    let exit_true = cfg.new_block();
    let exit_false = cfg.new_block();
    let handler = cfg.new_block();
    cfg.block_mut(cfg.entry()).push(ff("entry"));
    cfg.block_mut(t0).push(ff("t0_inst"));
    cfg.block_mut(t1).push(ff("t1_inst"));
    cfg.block_mut(exit_true).push(ff("early_return"));
    cfg.block_mut(exit_false).push(ff("late_return"));
    cfg.block_mut(handler).push(ff("catch_inst"));
    cfg.add_edge(cfg.entry(), t0, EdgeKind::Fallthrough);
    cfg.add_edge(t0, t1, EdgeKind::Fallthrough);
    cfg.add_edge(t1, exit_true, EdgeKind::ConditionalTrue);
    cfg.add_edge(t1, exit_false, EdgeKind::ConditionalFalse);
    cfg.add_edge(t0, handler, EdgeKind::ExceptionHandler);

    cfg.add_region(Region {
        id: RegionId::from_raw(0),
        protected_blocks: [t0, t1].into_iter().collect(),
        handlers: alloc::vec![Handler {
            entry: handler,
            body: HandlerBody::known([handler]),
            kind: HandlerKind::Catch,
        }],
        parent: None,
    });

    let ast = lift(&cfg);
    for block in [exit_true, exit_false] {
        assert!(
            contains_block(&ast, block),
            "code on a region-leaving arm may not be dropped: {block:?} in {ast:?}"
        );
    }
}

#[test]
fn switch_case_leaving_a_region_stays_as_a_goto() {
    use crate::region::{Handler, HandlerBody, HandlerKind, Region, RegionId};

    let mut cfg: Cfg<MockInst> = Cfg::new();
    let t0 = cfg.new_block();
    let selector = cfg.new_block();
    let case_in = cfg.new_block();
    let case_out = cfg.new_block();
    let handler = cfg.new_block();
    cfg.block_mut(cfg.entry()).push(ff("entry"));
    cfg.block_mut(t0).push(ff("t0_inst"));
    cfg.block_mut(selector).push(ff("selector"));
    cfg.block_mut(case_in).push(ff("case_in_inst"));
    cfg.block_mut(case_out).push(ff("case_out_inst"));
    cfg.block_mut(handler).push(ff("catch_inst"));
    cfg.add_edge(cfg.entry(), t0, EdgeKind::Fallthrough);
    cfg.add_edge(t0, selector, EdgeKind::Fallthrough);
    cfg.add_edge(selector, case_in, EdgeKind::SwitchCase);
    cfg.add_edge(selector, case_out, EdgeKind::SwitchCase);
    cfg.add_edge(t0, handler, EdgeKind::ExceptionHandler);

    cfg.add_region(Region {
        id: RegionId::from_raw(0),
        protected_blocks: [t0, selector, case_in].into_iter().collect(),
        handlers: alloc::vec![Handler {
            entry: handler,
            body: HandlerBody::known([handler]),
            kind: HandlerKind::Catch,
        }],
        parent: None,
    });

    let ast = lift(&cfg);
    let mut case_count = None;
    ast.visit(&mut |node| {
        if let AstNode::Switch { cases, .. } = node {
            case_count = Some(cases.len());
        }
    });
    assert_eq!(case_count, Some(2), "both cases survive: {ast:?}");
    assert!(
        has_goto_to(&ast, &cfg, case_out),
        "the out-of-region case is an explicit jump: {ast:?}"
    );
    assert!(
        contains_block(&ast, case_out),
        "the out-of-region case body may not be dropped: {ast:?}"
    );
}

#[test]
fn handler_tail_beyond_the_declared_extent_is_preserved() {
    use crate::region::{Handler, HandlerBody, HandlerKind, Region, RegionId};

    let mut cfg: Cfg<MockInst> = Cfg::new();
    let try_body = cfg.new_block();
    let handler = cfg.new_block();
    let handler_tail = cfg.new_block();
    cfg.block_mut(cfg.entry()).push(ff("entry"));
    cfg.block_mut(try_body).push(ff("try_inst"));
    cfg.block_mut(handler).push(ff("catch_inst"));
    cfg.block_mut(handler_tail).push(ff("ht_handler_tail"));
    cfg.add_edge(cfg.entry(), try_body, EdgeKind::Fallthrough);
    cfg.add_edge(try_body, handler, EdgeKind::ExceptionHandler);
    cfg.add_edge(handler, handler_tail, EdgeKind::Fallthrough);

    cfg.add_region(Region {
        id: RegionId::from_raw(0),
        protected_blocks: [try_body].into_iter().collect(),
        handlers: alloc::vec![Handler {
            entry: handler,
            body: HandlerBody::known([handler]),
            kind: HandlerKind::Catch,
        }],
        parent: None,
    });

    let ast = lift(&cfg);
    let handlers = find_try_handlers(&ast).expect("region structures");
    assert!(
        !handlers[0]
            .body
            .iter()
            .any(|node| contains_block(node, handler_tail)),
        "handler lifting stops at the declared extent: {ast:?}"
    );
    assert!(
        handlers[0]
            .body
            .iter()
            .any(|node| has_goto_to(node, &cfg, handler_tail)),
        "the boundary crossing is an explicit jump: {ast:?}"
    );
    assert!(
        contains_block(&ast, handler_tail),
        "the tail beyond the extent may not be dropped: {ast:?}"
    );
}

#[test]
fn linear_chain_leaving_a_region_records_the_exit() {
    use crate::region::{Handler, HandlerBody, HandlerKind, Region, RegionId};

    let mut cfg: Cfg<MockInst> = Cfg::new();
    let t0 = cfg.new_block();
    let t1 = cfg.new_block();
    let continuation = cfg.new_block();
    let handler = cfg.new_block();
    cfg.block_mut(cfg.entry()).push(ff("entry"));
    cfg.block_mut(t0).push(ff("t0_inst"));
    cfg.block_mut(t1).push(ff("t1_inst"));
    cfg.block_mut(continuation).push(ff("post_try"));
    cfg.block_mut(handler).push(ff("catch_inst"));
    cfg.add_edge(cfg.entry(), t0, EdgeKind::Fallthrough);
    cfg.add_edge(t0, t1, EdgeKind::Fallthrough);
    cfg.add_edge(t1, continuation, EdgeKind::Fallthrough);
    cfg.add_edge(t0, handler, EdgeKind::ExceptionHandler);
    cfg.add_edge(handler, continuation, EdgeKind::Fallthrough);

    cfg.add_region(Region {
        id: RegionId::from_raw(0),
        protected_blocks: [t0, t1].into_iter().collect(),
        handlers: alloc::vec![Handler {
            entry: handler,
            body: HandlerBody::known([handler]),
            kind: HandlerKind::Catch,
        }],
        parent: None,
    });

    let ast = lift(&cfg);
    assert!(
        contains_block(&ast, continuation),
        "the post-try continuation may not be dropped: {ast:?}"
    );
}

#[test]
fn nested_region_respects_the_enclosing_extent() {
    use crate::region::{Handler, HandlerBody, HandlerKind, Region, RegionId};

    let mut cfg: Cfg<MockInst> = Cfg::new();
    let try_body = cfg.new_block();
    let h0 = cfg.new_block();
    let h1 = cfg.new_block();
    let escape = cfg.new_block();
    let nested_handler = cfg.new_block();
    cfg.block_mut(cfg.entry()).push(ff("entry"));
    cfg.block_mut(try_body).push(ff("try_inst"));
    cfg.block_mut(h0).push(ff("h0_inst"));
    cfg.block_mut(h1).push(ff("h1_inst"));
    cfg.block_mut(escape).push(ff("x_escape_inst"));
    cfg.block_mut(nested_handler).push(ff("nested_catch"));
    cfg.add_edge(cfg.entry(), try_body, EdgeKind::Fallthrough);
    cfg.add_edge(try_body, h0, EdgeKind::ExceptionHandler);
    cfg.add_edge(h0, h1, EdgeKind::Fallthrough);
    cfg.add_edge(h1, escape, EdgeKind::Fallthrough);
    cfg.add_edge(h1, nested_handler, EdgeKind::ExceptionHandler);

    let outer_id = cfg.add_region(Region {
        id: RegionId::from_raw(0),
        protected_blocks: [try_body].into_iter().collect(),
        handlers: alloc::vec![Handler {
            entry: h0,
            body: HandlerBody::known([h0, h1]),
            kind: HandlerKind::Catch,
        }],
        parent: None,
    });
    cfg.add_region(Region {
        id: RegionId::from_raw(0),
        protected_blocks: [h1, escape].into_iter().collect(),
        handlers: alloc::vec![Handler {
            entry: nested_handler,
            body: HandlerBody::known([nested_handler]),
            kind: HandlerKind::Catch,
        }],
        parent: Some(outer_id),
    });

    let ast = lift(&cfg);
    let handlers = find_try_handlers(&ast).expect("outer region structures");
    assert!(
        !handlers[0]
            .body
            .iter()
            .any(|node| contains_block(node, escape)),
        "a nested region may not escape the enclosing extent: {ast:?}"
    );
    assert!(
        contains_block(&ast, escape),
        "the escaping block may not be dropped: {ast:?}"
    );
}

#[test]
fn region_anchor_follows_flow_order_not_block_ids() {
    use crate::region::{Handler, HandlerBody, HandlerKind, Region, RegionId};

    let mut cfg: Cfg<MockInst> = Cfg::new();
    // Allocate the interior block first so its id is smaller than the
    // region's flow entry.
    let interior = cfg.new_block();
    let flow_entry = cfg.new_block();
    let handler = cfg.new_block();
    let after = cfg.new_block();
    cfg.block_mut(cfg.entry()).push(ff("entry"));
    cfg.block_mut(interior).push(ff("interior_inst"));
    cfg.block_mut(flow_entry).push(ff("entry_inst"));
    cfg.block_mut(handler).push(ff("catch_inst"));
    cfg.block_mut(after).push(ff("after"));
    cfg.add_edge(cfg.entry(), flow_entry, EdgeKind::Fallthrough);
    cfg.add_edge(flow_entry, interior, EdgeKind::Fallthrough);
    cfg.add_edge(interior, after, EdgeKind::Fallthrough);
    cfg.add_edge(flow_entry, handler, EdgeKind::ExceptionHandler);
    cfg.add_edge(handler, after, EdgeKind::Fallthrough);

    cfg.add_region(Region {
        id: RegionId::from_raw(0),
        protected_blocks: [interior, flow_entry].into_iter().collect(),
        handlers: alloc::vec![Handler {
            entry: handler,
            body: HandlerBody::known([handler]),
            kind: HandlerKind::Catch,
        }],
        parent: None,
    });

    let ast = lift(&cfg);
    assert!(
        has_node_kind(&ast, |node| matches!(node, AstNode::TryCatch { .. })),
        "the region structures from its flow entry: {ast:?}"
    );
    for block in [flow_entry, interior, handler, after] {
        assert!(
            contains_block(&ast, block),
            "no block may be dropped: {block:?} in {ast:?}"
        );
    }
}

#[test]
fn multiple_finally_handlers_concatenate() {
    use crate::region::{Handler, HandlerBody, HandlerKind, Region, RegionId};

    let mut cfg: Cfg<MockInst> = Cfg::new();
    let try_body = cfg.new_block();
    let first_finally = cfg.new_block();
    let second_finally = cfg.new_block();
    cfg.block_mut(cfg.entry()).push(ff("entry"));
    cfg.block_mut(try_body).push(ff("try_inst"));
    cfg.block_mut(first_finally).push(ff("first_cleanup"));
    cfg.block_mut(second_finally).push(ff("second_cleanup"));
    cfg.add_edge(cfg.entry(), try_body, EdgeKind::Fallthrough);
    cfg.add_edge(try_body, first_finally, EdgeKind::ExceptionUnwind);
    cfg.add_edge(try_body, second_finally, EdgeKind::ExceptionUnwind);

    cfg.add_region(Region {
        id: RegionId::from_raw(0),
        protected_blocks: [try_body].into_iter().collect(),
        handlers: alloc::vec![
            Handler {
                entry: first_finally,
                body: HandlerBody::known([first_finally]),
                kind: HandlerKind::Finally,
            },
            Handler {
                entry: second_finally,
                body: HandlerBody::known([second_finally]),
                kind: HandlerKind::Finally,
            },
        ],
        parent: None,
    });

    let ast = lift(&cfg);
    let finally = match find_try_catch(&ast) {
        Some(AstNode::TryCatch { finally_body, .. }) => finally_body,
        other => panic!("expected a TryCatch, got {other:?}"),
    };
    let all = AstNode::Sequence {
        body: finally.clone(),
    };
    assert!(
        contains_block(&all, first_finally),
        "the first cleanup may not be discarded: {ast:?}"
    );
    assert!(
        contains_block(&all, second_finally),
        "the second cleanup follows in declaration order: {ast:?}"
    );
}

#[test]
fn filter_funclet_code_is_emitted() {
    use crate::region::{Handler, HandlerBody, HandlerKind, Region, RegionId};

    let mut cfg: Cfg<MockInst> = Cfg::new();
    let try_body = cfg.new_block();
    let handler = cfg.new_block();
    // The funclet is invoked by the runtime: no incoming CFG edge at all.
    let filter_block = cfg.new_block();
    cfg.block_mut(cfg.entry()).push(ff("entry"));
    cfg.block_mut(try_body).push(ff("try_inst"));
    cfg.block_mut(handler).push(ff("except_inst"));
    cfg.block_mut(filter_block).push(ff("filter_predicate"));
    cfg.add_edge(cfg.entry(), try_body, EdgeKind::Fallthrough);
    cfg.add_edge(try_body, handler, EdgeKind::ExceptionHandler);

    cfg.add_region(Region {
        id: RegionId::from_raw(0),
        protected_blocks: [try_body].into_iter().collect(),
        handlers: alloc::vec![Handler {
            entry: handler,
            body: HandlerBody::known([handler]),
            kind: HandlerKind::Filter { filter_block },
        }],
        parent: None,
    });

    let ast = lift(&cfg);
    assert!(
        contains_block(&ast, filter_block),
        "the filter funclet's predicate code may not be dropped: {ast:?}"
    );
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "HandlerBody::Known must contain its own handler entry")]
fn a_known_body_omitting_its_entry_is_a_frontend_bug() {
    use crate::region::{Handler, HandlerBody, HandlerKind, Region, RegionId};

    let mut cfg: Cfg<MockInst> = Cfg::new();
    let try_body = cfg.new_block();
    let handler = cfg.new_block();
    let elsewhere = cfg.new_block();
    cfg.block_mut(try_body).push(ff("try_inst"));
    cfg.add_edge(cfg.entry(), try_body, EdgeKind::Fallthrough);
    cfg.add_edge(try_body, handler, EdgeKind::ExceptionHandler);

    cfg.add_region(Region {
        id: RegionId::from_raw(0),
        protected_blocks: [try_body].into_iter().collect(),
        handlers: alloc::vec![Handler {
            entry: handler,
            body: HandlerBody::known([elsewhere]),
            kind: HandlerKind::Catch,
        }],
        parent: None,
    });

    let _ = lift(&cfg);
}

#[test]
fn try_anchored_at_a_conditional_keeps_the_branch_structure() {
    use crate::region::{Handler, HandlerBody, HandlerKind, Region, RegionId};

    let mut cfg: Cfg<MockInst> = Cfg::new();
    let branch = cfg.new_block();
    let then_arm = cfg.new_block();
    let else_arm = cfg.new_block();
    let merge = cfg.new_block();
    let handler = cfg.new_block();
    cfg.block_mut(cfg.entry()).push(ff("entry"));
    cfg.block_mut(branch).push(ff("branch_inst"));
    cfg.block_mut(then_arm).push(ff("then_inst"));
    cfg.block_mut(else_arm).push(ff("else_inst"));
    cfg.block_mut(merge).push(ff("merge_inst"));
    cfg.block_mut(handler).push(ff("catch_inst"));
    cfg.add_edge(cfg.entry(), branch, EdgeKind::Fallthrough);
    cfg.add_edge(branch, then_arm, EdgeKind::ConditionalTrue);
    cfg.add_edge(branch, else_arm, EdgeKind::ConditionalFalse);
    cfg.add_edge(then_arm, merge, EdgeKind::Fallthrough);
    cfg.add_edge(else_arm, merge, EdgeKind::Fallthrough);
    cfg.add_edge(branch, handler, EdgeKind::ExceptionHandler);

    cfg.add_region(Region {
        id: RegionId::from_raw(0),
        protected_blocks: [branch, then_arm, else_arm, merge].into_iter().collect(),
        handlers: alloc::vec![Handler {
            entry: handler,
            body: HandlerBody::known([handler]),
            kind: HandlerKind::Catch,
        }],
        parent: None,
    });

    let ast = lift(&cfg);
    let try_body = match find_try_catch(&ast) {
        Some(AstNode::TryCatch { try_body, .. }) => try_body,
        other => panic!("expected a TryCatch, got {other:?}"),
    };
    let inside = AstNode::Sequence {
        body: try_body.clone(),
    };
    assert!(
        has_node_kind(&inside, |node| matches!(node, AstNode::IfThenElse { .. })),
        "a branch anchoring a region keeps its structure inside the try: {ast:?}"
    );
    for block in [then_arm, else_arm, merge] {
        assert!(
            contains_block(&inside, block),
            "the try body carries the whole branch: {block:?} in {ast:?}"
        );
    }
}

#[test]
fn try_anchored_at_a_switch_selector_keeps_the_dispatch() {
    use crate::region::{Handler, HandlerBody, HandlerKind, Region, RegionId};

    let mut cfg: Cfg<MockInst> = Cfg::new();
    let selector = cfg.new_block();
    let case_a = cfg.new_block();
    let case_b = cfg.new_block();
    let handler = cfg.new_block();
    cfg.block_mut(cfg.entry()).push(ff("entry"));
    cfg.block_mut(selector).push(ff("selector"));
    cfg.block_mut(case_a).push(ff("case_a_inst"));
    cfg.block_mut(case_b).push(ff("case_b_inst"));
    cfg.block_mut(handler).push(ff("catch_inst"));
    cfg.add_edge(cfg.entry(), selector, EdgeKind::Fallthrough);
    cfg.add_edge(selector, case_a, EdgeKind::SwitchCase);
    cfg.add_edge(selector, case_b, EdgeKind::SwitchCase);
    cfg.add_edge(selector, handler, EdgeKind::ExceptionHandler);

    cfg.add_region(Region {
        id: RegionId::from_raw(0),
        protected_blocks: [selector, case_a, case_b].into_iter().collect(),
        handlers: alloc::vec![Handler {
            entry: handler,
            body: HandlerBody::known([handler]),
            kind: HandlerKind::Catch,
        }],
        parent: None,
    });

    let ast = lift(&cfg);
    let try_body = match find_try_catch(&ast) {
        Some(AstNode::TryCatch { try_body, .. }) => try_body,
        other => panic!("expected a TryCatch, got {other:?}"),
    };
    let inside = AstNode::Sequence {
        body: try_body.clone(),
    };
    assert!(
        has_node_kind(&inside, |node| matches!(node, AstNode::Switch { .. })),
        "a dispatch anchoring a region keeps its structure inside the try: {ast:?}"
    );
    let mut case_ids = alloc::vec::Vec::new();
    inside.visit(&mut |node| {
        if let AstNode::Switch { cases, .. } = node {
            case_ids.extend(cases.iter().map(|case| case.id));
        }
    });
    assert_eq!(
        case_ids,
        alloc::vec![case_a, case_b],
        "the try body carries every case: {ast:?}"
    );
}

#[test]
fn try_anchored_at_a_loop_header_keeps_the_loop() {
    use crate::region::{Handler, HandlerBody, HandlerKind, Region, RegionId};

    let mut cfg: Cfg<MockInst> = Cfg::new();
    let header = cfg.new_block();
    let latch = cfg.new_block();
    let handler = cfg.new_block();
    cfg.block_mut(cfg.entry()).push(ff("entry"));
    cfg.block_mut(header).push(ff("header_inst"));
    cfg.block_mut(latch).push(ff("latch_inst"));
    cfg.block_mut(handler).push(ff("catch_inst"));
    cfg.add_edge(cfg.entry(), header, EdgeKind::Fallthrough);
    cfg.add_edge(header, latch, EdgeKind::Fallthrough);
    cfg.add_edge(latch, header, EdgeKind::Back);
    cfg.add_edge(header, handler, EdgeKind::ExceptionHandler);

    cfg.add_region(Region {
        id: RegionId::from_raw(0),
        protected_blocks: [header, latch].into_iter().collect(),
        handlers: alloc::vec![Handler {
            entry: handler,
            body: HandlerBody::known([handler]),
            kind: HandlerKind::Catch,
        }],
        parent: None,
    });

    let ast = lift(&cfg);
    let try_body = match find_try_catch(&ast) {
        Some(AstNode::TryCatch { try_body, .. }) => try_body,
        other => panic!("expected a TryCatch, got {other:?}"),
    };
    let inside = AstNode::Sequence {
        body: try_body.clone(),
    };
    assert!(
        has_node_kind(&inside, |node| matches!(node, AstNode::Loop { .. })),
        "a loop header anchoring a region keeps its structure inside the try: {ast:?}"
    );
    assert!(
        contains_block(&inside, latch),
        "the loop body stays inside the try: {ast:?}"
    );
}
