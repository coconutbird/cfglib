extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use super::{StructuredEdge, StructuredSink, StructuredWalk, StructuredWalkIssue};

/// Mints sequential block numbers and records every wired edge.
#[derive(Default)]
struct Recorder {
    blocks: usize,
    edges: Vec<(usize, usize, StructuredEdge)>,
}

impl Recorder {
    fn entry(&mut self) -> usize {
        self.new_block()
    }
}

impl StructuredSink for Recorder {
    type Block = usize;

    fn new_block(&mut self) -> usize {
        let block = self.blocks;
        self.blocks += 1;
        block
    }

    fn connect(&mut self, source: usize, target: usize, edge: StructuredEdge) {
        self.edges.push((source, target, edge));
    }
}

#[test]
fn an_if_else_wires_a_diamond() {
    let mut sink = Recorder::default();
    let entry = sink.entry();
    let mut walk = StructuredWalk::new(entry);
    walk.enter(entry);

    walk.open_if().unwrap(); // condition landed in entry
    let then_arm = walk.landing(&mut sink);
    walk.open_else().unwrap();
    let else_arm = walk.landing(&mut sink);
    walk.close_if().unwrap();
    let merge = walk.landing(&mut sink);
    walk.finish().unwrap();

    assert_eq!((then_arm, else_arm, merge), (1, 2, 3));
    assert_eq!(
        sink.edges,
        vec![
            (entry, then_arm, StructuredEdge::True),
            (entry, else_arm, StructuredEdge::False),
            (then_arm, merge, StructuredEdge::Fallthrough),
            (else_arm, merge, StructuredEdge::Fallthrough),
        ],
    );
}

#[test]
fn an_if_without_else_routes_false_to_the_merge() {
    let mut sink = Recorder::default();
    let entry = sink.entry();
    let mut walk = StructuredWalk::new(entry);
    walk.enter(entry);

    walk.open_if().unwrap();
    let then_arm = walk.landing(&mut sink);
    walk.close_if().unwrap();
    let merge = walk.landing(&mut sink);
    walk.finish().unwrap();

    assert_eq!(
        sink.edges,
        vec![
            (entry, then_arm, StructuredEdge::True),
            (then_arm, merge, StructuredEdge::Fallthrough),
            (entry, merge, StructuredEdge::False),
        ],
    );
}

#[test]
fn a_loop_wires_header_latch_and_conditional_break() {
    let mut sink = Recorder::default();
    let entry = sink.entry();
    let mut walk = StructuredWalk::new(entry);
    walk.enter(entry);

    let header = walk.open_loop(&mut sink);
    // The break condition lands inside the body.
    let body = walk.landing(&mut sink);
    walk.break_out(true).unwrap();
    let after_break = walk.landing(&mut sink);
    walk.close_loop(&mut sink).unwrap();
    let exit = walk.landing(&mut sink);
    walk.finish().unwrap();

    assert_eq!(header, body, "the header is the first body landing");
    assert_eq!(
        sink.edges,
        vec![
            (entry, header, StructuredEdge::Fallthrough),
            (body, after_break, StructuredEdge::False),
            (after_break, header, StructuredEdge::Fallthrough),
            (body, exit, StructuredEdge::True),
        ],
    );
}

#[test]
fn continue_routes_to_the_innermost_header() {
    let mut sink = Recorder::default();
    let entry = sink.entry();
    let mut walk = StructuredWalk::new(entry);
    walk.enter(entry);

    let header = walk.open_loop(&mut sink);
    walk.continue_loop(&mut sink, true).unwrap();
    let rest = walk.landing(&mut sink);
    walk.close_loop(&mut sink).unwrap();
    walk.landing(&mut sink);
    walk.finish().unwrap();

    assert!(sink.edges.contains(&(header, header, StructuredEdge::True)));
    assert!(sink.edges.contains(&(header, rest, StructuredEdge::False)));
}

#[test]
fn conditional_transfer_diverts_true_and_continues_false() {
    let mut sink = Recorder::default();
    let entry = sink.entry();
    let shared_exit = sink.new_block();
    let mut walk = StructuredWalk::new(entry);
    walk.enter(entry);

    walk.conditional_transfer(&mut sink, shared_exit).unwrap();
    let continuation = walk.landing(&mut sink);
    walk.finish().unwrap();

    assert_eq!(
        sink.edges,
        vec![
            (entry, shared_exit, StructuredEdge::True),
            (entry, continuation, StructuredEdge::False),
        ],
    );
}

#[test]
fn terminate_ends_the_chain_without_successors() {
    let mut sink = Recorder::default();
    let entry = sink.entry();
    let mut walk = StructuredWalk::new(entry);
    walk.enter(entry);
    walk.terminate();
    assert!(walk.current().is_none());
    assert!(!walk.has_pending());
    walk.finish().unwrap();
    assert!(sink.edges.is_empty());
}

#[test]
fn structural_misuse_is_reported_without_corrupting_state() {
    let mut sink = Recorder::default();
    let entry = sink.entry();
    let mut walk = StructuredWalk::new(entry);
    walk.enter(entry);

    assert_eq!(walk.open_else(), Err(StructuredWalkIssue::ElseOutsideIf));
    assert_eq!(walk.close_if(), Err(StructuredWalkIssue::EndifOutsideIf));
    assert_eq!(
        walk.close_loop(&mut sink),
        Err(StructuredWalkIssue::EndloopOutsideLoop)
    );
    assert_eq!(
        walk.break_out(false),
        Err(StructuredWalkIssue::BreakOutsideLoop)
    );
    assert_eq!(
        walk.continue_loop(&mut sink, false),
        Err(StructuredWalkIssue::ContinueOutsideLoop)
    );

    // The failed markers left the walk usable.
    walk.open_if().unwrap();
    assert_eq!(walk.open_else(), Ok(()));
    assert_eq!(walk.open_else(), Err(StructuredWalkIssue::SecondElse));
    walk.close_if().unwrap();
    walk.landing(&mut sink);
    walk.finish().unwrap();
}

#[test]
fn unclosed_frames_fail_finish() {
    let mut sink = Recorder::default();
    let entry = sink.entry();
    let mut walk = StructuredWalk::new(entry);
    walk.enter(entry);
    walk.open_if().unwrap();
    assert_eq!(walk.finish(), Err(StructuredWalkIssue::UnclosedFrames));
}

#[test]
fn an_unmaterialized_loop_body_is_rejected() {
    let mut sink = Recorder::default();
    let entry = sink.entry();
    let mut walk = StructuredWalk::new(entry);
    walk.enter(entry);
    let _header = walk.open_loop(&mut sink);
    walk.break_out(true).unwrap();
    // The False path never landed: pending edges have nowhere to go.
    assert_eq!(
        walk.close_loop(&mut sink),
        Err(StructuredWalkIssue::LoopBodyUnmaterialized)
    );
}
