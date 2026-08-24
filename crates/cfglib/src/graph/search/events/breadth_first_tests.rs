use super::*;
use crate::graph::directed::{DirectedGraph, NodeId};
use alloc::vec;

fn classified() -> (DirectedGraph<(), ()>, [NodeId; 4]) {
    let mut graph = DirectedGraph::new();
    let root = graph.add_node(());
    let left = graph.add_node(());
    let right = graph.add_node(());
    let merge = graph.add_node(());
    graph.add_edge(root, left, ());
    graph.add_edge(root, right, ());
    graph.add_edge(left, merge, ());
    graph.add_edge(right, merge, ());
    graph.add_edge(right, right, ());
    graph.add_edge(merge, root, ());
    (graph, [root, left, right, merge])
}

fn events(graph: &DirectedGraph<(), ()>, start: NodeId) -> Vec<BfsEvent<NodeId>> {
    let mut events = Vec::new();
    let outcome = breadth_first_events(graph, start, TraversalDirection::Outgoing, |event| {
        events.push(event);
        ControlFlow::<()>::Continue(())
    });
    assert_eq!(outcome, None);
    events
}

#[test]
fn breadth_first_events_classify_edges_in_level_order() {
    let (graph, [root, left, right, merge]) = classified();
    assert_eq!(
        events(&graph, root),
        vec![
            BfsEvent::Discover(root, 0),
            BfsEvent::TreeEdge(root, left),
            BfsEvent::Discover(left, 1),
            BfsEvent::TreeEdge(root, right),
            BfsEvent::Discover(right, 1),
            BfsEvent::TreeEdge(left, merge),
            BfsEvent::Discover(merge, 2),
            BfsEvent::NonTreeEdge(right, merge),
            BfsEvent::NonTreeEdge(right, right),
            BfsEvent::NonTreeEdge(merge, root),
        ]
    );
}

#[test]
fn breadth_first_events_follow_the_selected_axis() {
    let mut graph = DirectedGraph::<(), ()>::new();
    let first = graph.add_node(());
    let middle = graph.add_node(());
    let last = graph.add_node(());
    graph.add_edge(first, middle, ());
    graph.add_edge(middle, last, ());

    let mut events = Vec::new();
    let outcome = breadth_first_events(&graph, last, TraversalDirection::Incoming, |event| {
        events.push(event);
        ControlFlow::<()>::Continue(())
    });

    assert_eq!(outcome, None);
    assert_eq!(
        events,
        vec![
            BfsEvent::Discover(last, 0),
            BfsEvent::TreeEdge(last, middle),
            BfsEvent::Discover(middle, 1),
            BfsEvent::TreeEdge(middle, first),
            BfsEvent::Discover(first, 2),
        ]
    );
}

#[test]
fn breadth_first_events_stop_at_the_break_event() {
    let (graph, [root, left, right, merge]) = classified();
    let mut events = Vec::new();
    let outcome = breadth_first_events(&graph, root, TraversalDirection::Outgoing, |event| {
        events.push(event);
        match event {
            BfsEvent::NonTreeEdge(from, to) => ControlFlow::Break((from, to)),
            _ => ControlFlow::Continue(()),
        }
    });

    assert_eq!(outcome, Some((right, merge)));
    assert_eq!(events.last(), Some(&BfsEvent::NonTreeEdge(right, merge)));
    assert!(!events.contains(&BfsEvent::NonTreeEdge(merge, root)));
    assert!(events.contains(&BfsEvent::TreeEdge(left, merge)));
}
