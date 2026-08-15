//! Discipline-configurable search over any [`DirectedGraphView`].
//!
//! The traversals in [`traverse`](super::traverse) each answer one fixed
//! question — "every node in preorder", "every node reachable from a seed
//! set" — with one fixed discipline. Consumer walks in code intelligence
//! usually carry a *different* discipline, and that discipline is normally
//! the entire reason the walk was hand-rolled: stop at the first acceptable
//! answer, prune a subtree that cannot contain one, bound how far a chase
//! may go, or deliberately reach one node along two paths because both paths
//! are the answer (an ambiguous C++ base, two import routes to one symbol).
//!
//! [`search`] turns those into configuration: [`SearchOrder`] chooses the
//! frontier, [`VisitedPolicy`] chooses whether marks are global or per path,
//! [`SearchConfig::max_depth`] bounds expansion, the visitor's [`Visit`]
//! verdict prunes, and its [`ControlFlow::Break`] ends the walk with an
//! answer. [`depth_first_events`] serves the other family: consumers whose
//! output order *is* the traversal (cycle diagnostics, tri-color edge
//! classification).
//!
//! [`open_search`](super::open::open_search) applies the same discipline to a
//! lazily discovered node space that has no dense identities, and
//! [`open_depth_first_events`](super::open::open_depth_first_events) is this
//! module's event walk over that space — discover/finish pairs for folds,
//! with per-path marks that re-fold a shared node once per route.

extern crate alloc;
use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;
use core::ops::ControlFlow;

use crate::graph::traverse::{TraversalDirection, neighbors};
use crate::graph::view::{DenseNodeId, DirectedGraphView};

/// The frontier discipline of a [`search`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchOrder {
    /// Expand the most recently discovered node first.
    ///
    /// Successors are expanded in adjacency order — the first successor of a
    /// node is expanded before the second — matching the rev-push convention
    /// of [`depth_first_preorder`](super::traverse::depth_first_preorder).
    DepthFirst,
    /// Expand nodes in discovery order, level by level.
    BreadthFirst,
}

/// Whether a search marks a node for the whole walk or only for the path it
/// is currently on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisitedPolicy {
    /// Mark a node the first time it is visited; never visit it again.
    ///
    /// This is what every traversal in [`traverse`](super::traverse) does,
    /// and the right discipline whenever the question is about nodes.
    Global,
    /// Mark a node on entry and **un-mark it on unwind**, so a node is
    /// visited once per distinct path that reaches it.
    ///
    /// This is the backtracking discipline: when the question is about
    /// *paths* — every derivation that reaches a symbol, every base
    /// subobject that makes a name ambiguous — a globally marked node
    /// silently hides the second answer. Termination comes from the
    /// path-cycle guard (a node already on the current path is not
    /// re-entered) plus [`SearchConfig::max_depth`]; the number of simple
    /// paths can be exponential, so bound the depth on dense graphs.
    ///
    /// Requires [`SearchOrder::DepthFirst`]: a breadth-first frontier has no
    /// unwind point at which to un-mark, so the combination is rejected.
    Path,
}

/// A visitor's verdict on the node it was handed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visit {
    /// Expand this node: discover its successors (subject to the depth
    /// bound).
    Descend,
    /// Prune here: the node counts as visited, but its successors are not
    /// discovered through it.
    Skip,
}

/// The discipline a [`search`] runs under.
///
/// The fields are public and the struct is `Copy`, so a consumer that stores
/// a discipline as data can write it as a literal;
/// [`new`](SearchConfig::new) plus the two setters are the ergonomic form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchConfig {
    /// Frontier discipline: depth-first or breadth-first.
    pub order: SearchOrder,
    /// Whether visited marks are global or per path.
    pub visited: VisitedPolicy,
    /// Whether the walk follows edges forwards or backwards.
    pub direction: TraversalDirection,
    /// Maximum depth to expand from, in hops from a seed. `None` is
    /// unbounded; `Some(0)` visits the seeds alone.
    pub max_depth: Option<usize>,
}

impl SearchConfig {
    /// A globally marked, unbounded search in `order` along `direction`.
    #[must_use]
    pub const fn new(order: SearchOrder, direction: TraversalDirection) -> Self {
        Self {
            order,
            visited: VisitedPolicy::Global,
            direction,
            max_depth: None,
        }
    }

    /// Return this configuration with a different [`VisitedPolicy`].
    #[must_use]
    pub const fn with_visited(mut self, visited: VisitedPolicy) -> Self {
        self.visited = visited;
        self
    }

    /// Return this configuration bounded to `max_depth` hops from a seed.
    #[must_use]
    pub const fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = Some(max_depth);
        self
    }
}

/// Search `graph` from `seeds` under a configurable discipline, returning the
/// value the visitor broke with, or `None` when the walk ran to completion.
///
/// # Semantics
///
/// - **Seeds** are visited at depth 0 in the order given. Under
///   [`VisitedPolicy::Global`] a seed already reached from an earlier seed is
///   skipped (so duplicate seeds are visited once); under
///   [`VisitedPolicy::Path`] each seed starts a fresh path context, so a
///   repeated seed is searched again.
/// - **Depth-first** expands the first successor of a node before the second
///   (adjacency order). **Breadth-first** visits in discovery order.
/// - **Depth** counts hops from the seed the search arrived through. Under
///   breadth-first that is the hop count from the nearest seed; under
///   depth-first it is the length of the path this walk actually took, which
///   need not be the shortest one.
/// - **`max_depth` bounds expansion, not visiting**: a node at the bound is
///   visited, but its successors are not discovered through it.
/// - **[`Visit::Skip`]** prunes the same way at any depth.
/// - **[`ControlFlow::Break`]** returns immediately with `Some(value)`; no
///   further node is visited.
///
/// # Examples
///
/// The first match depends on the discipline, which is the point:
///
/// ```
/// use core::ops::ControlFlow;
///
/// use cfglib::{DirectedGraph, SearchConfig, SearchOrder, TraversalDirection, Visit, search};
///
/// //     a -> b -> d
/// //     a -> c
/// let mut graph = DirectedGraph::<&str, ()>::new();
/// let a = graph.add_node("a");
/// let b = graph.add_node("b");
/// let c = graph.add_node("c");
/// let d = graph.add_node("d");
/// graph.add_edge(a, b, ());
/// graph.add_edge(a, c, ());
/// graph.add_edge(b, d, ());
///
/// let first_leaf = |order| {
///     search(
///         &graph,
///         [a],
///         SearchConfig::new(order, TraversalDirection::Outgoing),
///         |node, _depth| {
///             if graph.successors(node).count() == 0 {
///                 return ControlFlow::Break(graph[node]);
///             }
///             ControlFlow::Continue(Visit::Descend)
///         },
///     )
/// };
///
/// assert_eq!(first_leaf(SearchOrder::DepthFirst), Some("d"));
/// assert_eq!(first_leaf(SearchOrder::BreadthFirst), Some("c"));
/// ```
///
/// # Panics
///
/// Panics when a seed is not a node in `graph`, or when `config` pairs
/// [`SearchOrder::BreadthFirst`] with [`VisitedPolicy::Path`] — a
/// breadth-first frontier has no unwind on which to un-mark.
#[must_use]
pub fn search<G: DirectedGraphView, B>(
    graph: &G,
    seeds: impl IntoIterator<Item = G::NodeId>,
    config: SearchConfig,
    visitor: impl FnMut(G::NodeId, usize) -> ControlFlow<B, Visit>,
) -> Option<B> {
    let seeds: Vec<G::NodeId> = seeds.into_iter().collect();
    for seed in &seeds {
        assert!(
            seed.index() < graph.node_count(),
            "seed node is out of range"
        );
    }

    match (config.order, config.visited) {
        (SearchOrder::DepthFirst, VisitedPolicy::Global) => {
            depth_first_global(graph, &seeds, config, visitor)
        }
        (SearchOrder::DepthFirst, VisitedPolicy::Path) => {
            depth_first_path(graph, &seeds, config, visitor)
        }
        (SearchOrder::BreadthFirst, VisitedPolicy::Global) => {
            breadth_first_global(graph, &seeds, config, visitor)
        }
        (SearchOrder::BreadthFirst, VisitedPolicy::Path) => {
            panic!(
                "VisitedPolicy::Path requires SearchOrder::DepthFirst: a breadth-first frontier never unwinds, so a path mark could never be removed"
            )
        }
    }
}

/// Whether a node at `depth` may expand under `config`.
fn may_expand(config: SearchConfig, depth: usize) -> bool {
    config.max_depth.is_none_or(|limit| depth < limit)
}

fn depth_first_global<G: DirectedGraphView, B>(
    graph: &G,
    seeds: &[G::NodeId],
    config: SearchConfig,
    mut visitor: impl FnMut(G::NodeId, usize) -> ControlFlow<B, Visit>,
) -> Option<B> {
    let mut visited = vec![false; graph.node_count()];
    // Reversed so the first seed pops first; the same convention applies to
    // successors below, so adjacency order is expansion order.
    let mut stack: Vec<(G::NodeId, usize)> = seeds.iter().rev().map(|&seed| (seed, 0)).collect();

    while let Some((node, depth)) = stack.pop() {
        if visited[node.index()] {
            continue;
        }
        visited[node.index()] = true;
        match visitor(node, depth) {
            ControlFlow::Break(value) => return Some(value),
            ControlFlow::Continue(Visit::Skip) => continue,
            ControlFlow::Continue(Visit::Descend) => {}
        }
        if !may_expand(config, depth) {
            continue;
        }
        for successor in neighbors(graph, node, config.direction).into_iter().rev() {
            if !visited[successor.index()] {
                stack.push((successor, depth + 1));
            }
        }
    }

    None
}

/// One step of a path-marked depth-first walk.
enum PathStep<N> {
    /// Enter `node` at this depth, marking it on the current path.
    Enter(N, usize),
    /// Leave `node`: the unwind point at which its path mark is removed.
    Leave(N),
}

fn depth_first_path<G: DirectedGraphView, B>(
    graph: &G,
    seeds: &[G::NodeId],
    config: SearchConfig,
    mut visitor: impl FnMut(G::NodeId, usize) -> ControlFlow<B, Visit>,
) -> Option<B> {
    let mut on_path = vec![false; graph.node_count()];
    let mut stack: Vec<PathStep<G::NodeId>> = seeds
        .iter()
        .rev()
        .map(|&seed| PathStep::Enter(seed, 0))
        .collect();

    while let Some(step) = stack.pop() {
        let (node, depth) = match step {
            // Every `Enter` pushes its own `Leave` before any successor, so
            // this pops exactly when the node's subtree is exhausted — and
            // between two seeds the path is empty again.
            PathStep::Leave(node) => {
                on_path[node.index()] = false;
                continue;
            }
            PathStep::Enter(node, depth) => (node, depth),
        };
        if on_path[node.index()] {
            continue;
        }
        on_path[node.index()] = true;
        stack.push(PathStep::Leave(node));
        match visitor(node, depth) {
            ControlFlow::Break(value) => return Some(value),
            ControlFlow::Continue(Visit::Skip) => continue,
            ControlFlow::Continue(Visit::Descend) => {}
        }
        if !may_expand(config, depth) {
            continue;
        }
        for successor in neighbors(graph, node, config.direction).into_iter().rev() {
            stack.push(PathStep::Enter(successor, depth + 1));
        }
    }

    None
}

fn breadth_first_global<G: DirectedGraphView, B>(
    graph: &G,
    seeds: &[G::NodeId],
    config: SearchConfig,
    mut visitor: impl FnMut(G::NodeId, usize) -> ControlFlow<B, Visit>,
) -> Option<B> {
    let mut visited = vec![false; graph.node_count()];
    let mut queue: VecDeque<(G::NodeId, usize)> = VecDeque::new();
    for &seed in seeds {
        if !visited[seed.index()] {
            visited[seed.index()] = true;
            queue.push_back((seed, 0));
        }
    }

    while let Some((node, depth)) = queue.pop_front() {
        match visitor(node, depth) {
            ControlFlow::Break(value) => return Some(value),
            ControlFlow::Continue(Visit::Skip) => continue,
            ControlFlow::Continue(Visit::Descend) => {}
        }
        if !may_expand(config, depth) {
            continue;
        }
        for adjacent in neighbors(graph, node, config.direction) {
            if !visited[adjacent.index()] {
                visited[adjacent.index()] = true;
                queue.push_back((adjacent, depth + 1));
            }
        }
    }

    None
}

/// An event of a [`depth_first_events`] walk.
///
/// Together these are the classic tri-color depth-first classification: the
/// edges of the walk split into the tree it builds, the back edges that close
/// a cycle, and the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DfsEvent<N> {
    /// The node was reached for the first time, at this depth.
    Discover(N, usize),
    /// The edge from the first node to the second discovered the second —
    /// an edge of the depth-first tree.
    TreeEdge(N, N),
    /// The edge from the first node to the second closed a cycle: the target
    /// is an ancestor on the current path (a self-edge is a back edge).
    BackEdge(N, N),
    /// The edge from the first node to the second reached an already
    /// finished node — a forward edge to a descendant, or a cross edge into
    /// a sibling subtree.
    ///
    /// The two are merged deliberately: telling them apart needs discovery
    /// timestamps, which a consumer that cares can record from
    /// [`Discover`](DfsEvent::Discover) itself.
    ForwardOrCross(N, N),
    /// Every edge out of the node has been classified and its subtree is
    /// complete.
    Finish(N),
}

/// One frame of the explicit depth-first stack.
struct DfsFrame<N> {
    node: N,
    depth: usize,
    successors: Vec<N>,
    cursor: usize,
}

/// The tri-color state of a node during a [`depth_first_events`] walk.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Color {
    /// Undiscovered.
    White,
    /// Discovered, on the current path, not yet finished.
    Gray,
    /// Finished.
    Black,
}

/// Walk `graph` depth-first from `start`, reporting every discovery, edge
/// classification, and finish, and returning the value the callback broke
/// with (or `None` when the walk ran to completion).
///
/// [`search`] answers questions about nodes; this answers questions whose
/// output order *is* the traversal — reporting a cycle at the edge that
/// closes it, emitting a nesting structure on discover/finish pairs,
/// classifying edges. Doing that on top of a node-yielding traversal is what
/// forces a consumer to hand-roll a walk.
///
/// # Order
///
/// The event order is deterministic and part of the API. Successors are
/// examined in adjacency order (equivalently, the rev-push convention of
/// [`depth_first_preorder`](super::traverse::depth_first_preorder)). For a
/// node `u` the walk emits [`Discover`](DfsEvent::Discover) once, then for
/// each successor `v` in adjacency order either
/// [`TreeEdge`](DfsEvent::TreeEdge) immediately followed by `v`'s own events,
/// or [`BackEdge`](DfsEvent::BackEdge) / one
/// [`ForwardOrCross`](DfsEvent::ForwardOrCross), and finally
/// [`Finish`](DfsEvent::Finish). Only nodes reachable from `start` produce
/// events.
///
/// # Examples
///
/// Reporting the cycles of a graph in the order the walk closes them:
///
/// ```
/// use core::ops::ControlFlow;
///
/// use cfglib::{DfsEvent, DirectedGraph, TraversalDirection, depth_first_events};
///
/// // a -> b -> c -> a, plus a chord a -> c.
/// let mut graph = DirectedGraph::<&str, ()>::new();
/// let a = graph.add_node("a");
/// let b = graph.add_node("b");
/// let c = graph.add_node("c");
/// graph.add_edge(a, b, ());
/// graph.add_edge(a, c, ());
/// graph.add_edge(b, c, ());
/// graph.add_edge(c, a, ());
///
/// let mut cycles = Vec::new();
/// let outcome = depth_first_events::<_, ()>(
///     &graph,
///     a,
///     TraversalDirection::Outgoing,
///     |event| {
///         if let DfsEvent::BackEdge(from, to) = event {
///             cycles.push((graph[from], graph[to]));
///         }
///         ControlFlow::Continue(())
///     },
/// );
///
/// assert_eq!(outcome, None);
/// assert_eq!(cycles, vec![("c", "a")]);
/// ```
///
/// # Panics
///
/// Panics when `start` is not a node in `graph`.
#[must_use]
pub fn depth_first_events<G: DirectedGraphView, B>(
    graph: &G,
    start: G::NodeId,
    direction: TraversalDirection,
    mut on_event: impl FnMut(DfsEvent<G::NodeId>) -> ControlFlow<B>,
) -> Option<B> {
    assert!(
        start.index() < graph.node_count(),
        "start node is out of range"
    );

    macro_rules! emit {
        ($event:expr) => {
            if let ControlFlow::Break(value) = on_event($event) {
                return Some(value);
            }
        };
    }

    let mut color = vec![Color::White; graph.node_count()];
    color[start.index()] = Color::Gray;
    emit!(DfsEvent::Discover(start, 0));
    let mut stack = vec![DfsFrame {
        node: start,
        depth: 0,
        successors: neighbors(graph, start, direction),
        cursor: 0,
    }];

    // Read frames through `last_mut` and copy only the Copy fields — cloning
    // a frame with its successor Vec per iteration would make the walk
    // O(Σ deg²).
    while let Some(frame) = stack.last_mut() {
        let node = frame.node;
        let depth = frame.depth;
        if frame.cursor < frame.successors.len() {
            let successor = frame.successors[frame.cursor];
            frame.cursor += 1;
            match color[successor.index()] {
                Color::White => {
                    emit!(DfsEvent::TreeEdge(node, successor));
                    color[successor.index()] = Color::Gray;
                    emit!(DfsEvent::Discover(successor, depth + 1));
                    stack.push(DfsFrame {
                        node: successor,
                        depth: depth + 1,
                        successors: neighbors(graph, successor, direction),
                        cursor: 0,
                    });
                }
                Color::Gray => emit!(DfsEvent::BackEdge(node, successor)),
                Color::Black => emit!(DfsEvent::ForwardOrCross(node, successor)),
            }
            continue;
        }

        color[node.index()] = Color::Black;
        emit!(DfsEvent::Finish(node));
        stack.pop();
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::directed::{DirectedGraph, NodeId};
    use alloc::vec;

    /// `a -> b -> d`, `a -> c`: depth-first reaches `d` before `c`,
    /// breadth-first the other way round.
    fn fork() -> (DirectedGraph<(), ()>, [NodeId; 4]) {
        let mut graph = DirectedGraph::<(), ()>::new();
        let a = graph.add_node(());
        let b = graph.add_node(());
        let c = graph.add_node(());
        let d = graph.add_node(());
        graph.add_edge(a, b, ());
        graph.add_edge(a, c, ());
        graph.add_edge(b, d, ());
        (graph, [a, b, c, d])
    }

    /// `a -> b`, `a -> c`, `b -> d`, `c -> d`: two paths reach `d`.
    fn diamond() -> (DirectedGraph<(), ()>, [NodeId; 4]) {
        let mut graph = DirectedGraph::<(), ()>::new();
        let a = graph.add_node(());
        let b = graph.add_node(());
        let c = graph.add_node(());
        let d = graph.add_node(());
        graph.add_edge(a, b, ());
        graph.add_edge(a, c, ());
        graph.add_edge(b, d, ());
        graph.add_edge(c, d, ());
        (graph, [a, b, c, d])
    }

    fn config(order: SearchOrder) -> SearchConfig {
        SearchConfig::new(order, TraversalDirection::Outgoing)
    }

    /// Visit order under `config`, seeded at `seeds`, descending everywhere.
    fn visit_order<G: DirectedGraphView>(
        graph: &G,
        seeds: impl IntoIterator<Item = G::NodeId>,
        config: SearchConfig,
    ) -> Vec<(G::NodeId, usize)> {
        let mut order = Vec::new();
        let outcome = search(graph, seeds, config, |node, depth| {
            order.push((node, depth));
            ControlFlow::<(), _>::Continue(Visit::Descend)
        });
        assert_eq!(outcome, None, "a descending search never breaks");
        order
    }

    fn nodes<N: Copy>(order: &[(N, usize)]) -> Vec<N> {
        order.iter().map(|&(node, _)| node).collect()
    }

    #[test]
    fn first_match_depends_on_the_search_order() {
        let (graph, [a, b, c, d]) = fork();
        let first_leaf = |order| {
            search(&graph, [a], config(order), |node, _| {
                if graph.successors(node).count() == 0 {
                    return ControlFlow::Break(node);
                }
                ControlFlow::Continue(Visit::Descend)
            })
        };

        // The same graph, the same seed, two disciplines, two answers.
        assert_eq!(first_leaf(SearchOrder::DepthFirst), Some(d));
        assert_eq!(first_leaf(SearchOrder::BreadthFirst), Some(c));

        assert_eq!(
            nodes(&visit_order(&graph, [a], config(SearchOrder::DepthFirst))),
            vec![a, b, d, c]
        );
        assert_eq!(
            nodes(&visit_order(&graph, [a], config(SearchOrder::BreadthFirst))),
            vec![a, b, c, d]
        );
    }

    #[test]
    fn path_policy_reports_every_path_to_a_shared_node() {
        let (graph, [a, b, c, d]) = diamond();
        // Globally marked, `d` is one node reached once.
        assert_eq!(
            nodes(&visit_order(&graph, [a], config(SearchOrder::DepthFirst))),
            vec![a, b, d, c]
        );
        // Path-marked, `d` is reported once per route into it — the shape of
        // an ambiguous base or a symbol reachable through two imports.
        let paths = visit_order(
            &graph,
            [a],
            config(SearchOrder::DepthFirst).with_visited(VisitedPolicy::Path),
        );
        assert_eq!(nodes(&paths), vec![a, b, d, c, d]);
        assert_eq!(paths[2], (d, 2));
        assert_eq!(paths[4], (d, 2));
    }

    #[test]
    fn skip_prunes_the_subtree_under_both_policies() {
        let (graph, [a, b, c, d]) = fork();
        for visited in [VisitedPolicy::Global, VisitedPolicy::Path] {
            let mut order = Vec::new();
            let outcome = search(
                &graph,
                [a],
                config(SearchOrder::DepthFirst).with_visited(visited),
                |node, _| {
                    order.push(node);
                    if node == b {
                        return ControlFlow::<(), _>::Continue(Visit::Skip);
                    }
                    ControlFlow::Continue(Visit::Descend)
                },
            );
            // `b` is visited, `d` (only reachable through it) is not.
            assert_eq!(outcome, None);
            assert_eq!(order, vec![a, b, c], "{visited:?}");
        }
        assert_eq!(
            nodes(&visit_order(&graph, [a], config(SearchOrder::DepthFirst))),
            vec![a, b, d, c],
            "without the skip, `d` is reached"
        );
    }

    #[test]
    fn seeds_are_searched_in_the_order_given() {
        let (graph, [a, b, c, d]) = fork();
        // Seeding `c` first puts it before the whole `a` subtree.
        assert_eq!(
            nodes(&visit_order(
                &graph,
                [c, a],
                config(SearchOrder::DepthFirst)
            )),
            vec![c, a, b, d]
        );
        assert_eq!(
            nodes(&visit_order(
                &graph,
                [a, c],
                config(SearchOrder::DepthFirst)
            )),
            vec![a, b, d, c]
        );
        // Breadth-first interleaves the seeds' levels, still in seed order.
        assert_eq!(
            nodes(&visit_order(
                &graph,
                [b, a],
                config(SearchOrder::BreadthFirst)
            )),
            vec![b, a, d, c]
        );
        // Seeds are at depth 0 even when another seed reaches them deeper.
        assert_eq!(
            visit_order(&graph, [d, a], config(SearchOrder::DepthFirst)),
            vec![(d, 0), (a, 0), (b, 1), (c, 1)]
        );
    }

    #[test]
    fn duplicate_seeds_dedup_globally_and_repeat_on_paths() {
        let (graph, [a, b, c, d]) = fork();
        assert_eq!(
            nodes(&visit_order(
                &graph,
                [a, a],
                config(SearchOrder::DepthFirst)
            )),
            vec![a, b, d, c]
        );
        // Each seed starts a fresh path context, so the second one searches
        // again from an empty path.
        assert_eq!(
            nodes(&visit_order(
                &graph,
                [a, a],
                config(SearchOrder::DepthFirst).with_visited(VisitedPolicy::Path)
            )),
            vec![a, b, d, c, a, b, d, c]
        );
    }

    #[test]
    fn cycles_terminate_under_both_policies() {
        // a -> b -> c -> a, with a self-edge on c.
        let mut graph = DirectedGraph::<(), ()>::new();
        let a = graph.add_node(());
        let b = graph.add_node(());
        let c = graph.add_node(());
        graph.add_edge(a, b, ());
        graph.add_edge(b, c, ());
        graph.add_edge(c, a, ());
        graph.add_edge(c, c, ());

        assert_eq!(
            nodes(&visit_order(&graph, [a], config(SearchOrder::DepthFirst))),
            vec![a, b, c]
        );
        assert_eq!(
            nodes(&visit_order(&graph, [a], config(SearchOrder::BreadthFirst))),
            vec![a, b, c]
        );
        // The path guard refuses to re-enter a node already on the path, so
        // the walk terminates without a global mark.
        assert_eq!(
            nodes(&visit_order(
                &graph,
                [a],
                config(SearchOrder::DepthFirst).with_visited(VisitedPolicy::Path)
            )),
            vec![a, b, c]
        );
    }

    #[test]
    fn max_depth_bounds_expansion_not_visiting() {
        // A chain a -> b -> c -> d.
        let mut graph = DirectedGraph::<(), ()>::new();
        let a = graph.add_node(());
        let b = graph.add_node(());
        let c = graph.add_node(());
        let d = graph.add_node(());
        graph.add_edge(a, b, ());
        graph.add_edge(b, c, ());
        graph.add_edge(c, d, ());

        for order in [SearchOrder::DepthFirst, SearchOrder::BreadthFirst] {
            assert_eq!(
                visit_order(&graph, [a], config(order).with_max_depth(0)),
                vec![(a, 0)],
                "{order:?}"
            );
            // The node at the bound is visited; its successor is not
            // discovered through it.
            assert_eq!(
                visit_order(&graph, [a], config(order).with_max_depth(2)),
                vec![(a, 0), (b, 1), (c, 2)],
                "{order:?}"
            );
            assert_eq!(
                visit_order(&graph, [a], config(order).with_max_depth(9)),
                vec![(a, 0), (b, 1), (c, 2), (d, 3)],
                "{order:?}"
            );
        }
    }

    #[test]
    fn break_returns_immediately() {
        let (graph, [a, b, _c, _d]) = fork();
        let mut seen = Vec::new();
        let found = search(
            &graph,
            [a],
            config(SearchOrder::DepthFirst),
            |node, depth| {
                seen.push(node);
                if node == b {
                    return ControlFlow::Break(depth);
                }
                ControlFlow::Continue(Visit::Descend)
            },
        );

        assert_eq!(found, Some(1));
        assert_eq!(seen, vec![a, b], "nothing after the break is visited");
    }

    #[test]
    fn the_incoming_direction_searches_predecessors() {
        let (graph, [a, b, _c, d]) = fork();
        assert_eq!(
            nodes(&visit_order(
                &graph,
                [d],
                SearchConfig::new(SearchOrder::DepthFirst, TraversalDirection::Incoming)
            )),
            vec![d, b, a]
        );
    }

    #[test]
    #[should_panic(expected = "VisitedPolicy::Path requires SearchOrder::DepthFirst")]
    fn breadth_first_with_path_marks_is_rejected() {
        let (graph, [a, _, _, _]) = fork();
        let _ = search(
            &graph,
            [a],
            config(SearchOrder::BreadthFirst).with_visited(VisitedPolicy::Path),
            |_, _| ControlFlow::<(), _>::Continue(Visit::Descend),
        );
    }

    #[test]
    #[should_panic(expected = "seed node is out of range")]
    fn an_out_of_range_seed_panics() {
        let (graph, _) = fork();
        let _ = search(
            &graph,
            [NodeId::from_index(9)],
            config(SearchOrder::DepthFirst),
            |_, _| ControlFlow::<(), _>::Continue(Visit::Descend),
        );
    }

    /// `a -> b`, `a -> d`, `a -> c`, `b -> c`, `c -> a`, `d -> c`: one graph
    /// carrying all four edge classes.
    fn classified() -> (DirectedGraph<(), ()>, [NodeId; 4]) {
        let mut graph = DirectedGraph::<(), ()>::new();
        let a = graph.add_node(());
        let b = graph.add_node(());
        let c = graph.add_node(());
        let d = graph.add_node(());
        graph.add_edge(a, b, ());
        graph.add_edge(a, d, ());
        graph.add_edge(a, c, ());
        graph.add_edge(b, c, ());
        graph.add_edge(c, a, ());
        graph.add_edge(d, c, ());
        (graph, [a, b, c, d])
    }

    fn events(graph: &DirectedGraph<(), ()>, start: NodeId) -> Vec<DfsEvent<NodeId>> {
        let mut log = Vec::new();
        let outcome = depth_first_events(graph, start, TraversalDirection::Outgoing, |event| {
            log.push(event);
            ControlFlow::<()>::Continue(())
        });
        assert_eq!(outcome, None);
        log
    }

    #[test]
    fn depth_first_events_classify_every_edge_in_a_pinned_order() {
        let (graph, [a, b, c, d]) = classified();
        assert_eq!(
            events(&graph, a),
            vec![
                DfsEvent::Discover(a, 0),
                DfsEvent::TreeEdge(a, b),
                DfsEvent::Discover(b, 1),
                DfsEvent::TreeEdge(b, c),
                DfsEvent::Discover(c, 2),
                // c -> a closes the cycle: `a` is an ancestor on the path.
                DfsEvent::BackEdge(c, a),
                DfsEvent::Finish(c),
                DfsEvent::Finish(b),
                DfsEvent::TreeEdge(a, d),
                DfsEvent::Discover(d, 1),
                // d -> c is a cross edge into a finished sibling subtree.
                DfsEvent::ForwardOrCross(d, c),
                DfsEvent::Finish(d),
                // a -> c is a forward edge to a finished descendant.
                DfsEvent::ForwardOrCross(a, c),
                DfsEvent::Finish(a),
            ]
        );
    }

    #[test]
    fn depth_first_events_report_self_edges_as_back_edges() {
        let mut graph = DirectedGraph::<(), ()>::new();
        let only = graph.add_node(());
        graph.add_edge(only, only, ());
        assert_eq!(
            events(&graph, only),
            vec![
                DfsEvent::Discover(only, 0),
                DfsEvent::BackEdge(only, only),
                DfsEvent::Finish(only),
            ]
        );
    }

    #[test]
    fn depth_first_events_only_cover_the_reachable_set() {
        let mut graph = DirectedGraph::<(), ()>::new();
        let a = graph.add_node(());
        let b = graph.add_node(());
        let unreached = graph.add_node(());
        graph.add_edge(a, b, ());
        graph.add_edge(unreached, a, ());

        assert_eq!(
            events(&graph, a),
            vec![
                DfsEvent::Discover(a, 0),
                DfsEvent::TreeEdge(a, b),
                DfsEvent::Discover(b, 1),
                DfsEvent::Finish(b),
                DfsEvent::Finish(a),
            ],
            "a predecessor-only node produces no events"
        );
    }

    #[test]
    fn depth_first_events_break_stops_the_walk() {
        let (graph, [a, b, c, _d]) = classified();
        let mut log = Vec::new();
        let found = depth_first_events(&graph, a, TraversalDirection::Outgoing, |event| {
            log.push(event);
            match event {
                DfsEvent::BackEdge(from, to) => ControlFlow::Break((from, to)),
                _ => ControlFlow::Continue(()),
            }
        });

        assert_eq!(found, Some((c, a)));
        assert_eq!(
            log,
            vec![
                DfsEvent::Discover(a, 0),
                DfsEvent::TreeEdge(a, b),
                DfsEvent::Discover(b, 1),
                DfsEvent::TreeEdge(b, c),
                DfsEvent::Discover(c, 2),
                DfsEvent::BackEdge(c, a),
            ]
        );
    }
}
