//! The [`search`](super::search::search) disciplines over an *open* node
//! space — one discovered lazily, with no dense identities.
//!
//! [`DirectedGraphView`](super::view::DirectedGraphView) requires the graph to
//! exist: dense ids in `0..node_count`, adjacency on demand. Plenty of real
//! walks have no such graph. An import chase probes a live resolver and mints
//! the next node from the answer; a re-export chase walks a `(file, name)`
//! space whose edges are barrel files it has not read yet; an emission walk
//! interleaves children by source offset as it computes them. Materialising
//! those spaces to walk them is exactly backwards — the walk is what decides
//! which nodes exist.
//!
//! [`open_search`] takes the successor relation as a closure and everything
//! else from [`OpenSearchConfig`], so an open walk gets the same first-match,
//! pruning, depth-bounding, and backtracking discipline as a dense one. The
//! configuration is deliberately *reduced*: there is no `direction`, because
//! the closure defines the edges and a direction field could only lie.
//!
//! [`open_breadth_first_events`] exposes the breadth-first tree as nodes are
//! discovered and reports routes refused by its global marks.
//!
//! [`open_depth_first_events`] walks the same open space for a *fold*. A search
//! can only stop; a fold computes a node's answer from its subtrees' answers,
//! which needs the unwind — so the walk reports
//! [`Discover`](OpenDfsEvent::Discover) and [`Finish`](OpenDfsEvent::Finish)
//! pairs (and the re-entries its mark policy refuses) instead of nodes. C++
//! member lookup is the shape: a class answers with its own declaration if it
//! has one, else with the answer of exactly one yielding base subobject, and
//! two yielding subobjects are an ambiguity — so a diamond's shared base must
//! be folded once *per route*, which is what [`VisitedPolicy::Path`] means
//! here.
//!
//! [`follow`] and [`follow_path`] are the degenerate case that shows up just
//! as often: an out-degree ≤ 1 chase (import alias to import alias, symlink to
//! symlink, type alias to type alias) which needs a hop bound and a cycle
//! guard and nothing else.

extern crate alloc;
use alloc::collections::{BTreeSet, VecDeque};
use alloc::vec::Vec;
use core::ops::ControlFlow;

use crate::graph::search::{SearchOrder, Visit, VisitedPolicy};

mod breadth_first;
pub use breadth_first::{OpenBfsConfig, OpenBfsEvent, open_breadth_first_events};

/// The discipline an [`open_search`] runs under.
///
/// This is [`SearchConfig`](super::search::SearchConfig) minus its
/// `direction`: in an open space the successor closure *is* the edge
/// relation, so a walk runs backwards by being given a closure that reports
/// predecessors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenSearchConfig {
    /// Frontier discipline: depth-first or breadth-first.
    pub order: SearchOrder,
    /// Whether visited marks are global or per path.
    pub visited: VisitedPolicy,
    /// Maximum depth to expand from, in hops from a seed. `None` is
    /// unbounded; `Some(0)` visits the seeds alone.
    pub max_depth: Option<usize>,
}

impl OpenSearchConfig {
    /// A globally marked, unbounded search in `order`.
    #[must_use]
    pub const fn new(order: SearchOrder) -> Self {
        Self {
            order,
            visited: VisitedPolicy::Global,
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

/// Search a lazily discovered node space from `seeds`, returning the value the
/// visitor broke with, or `None` when the walk ran to completion.
///
/// `successors` is handed a node and a scratch buffer, **cleared before every
/// call**, and pushes that node's successors *in the order they should be
/// explored*. Nodes are compared and marked by `Ord`, so identity is whatever
/// the consumer's node type says it is — a `(file, name)` pair, a resolved
/// module handle, an interned id.
///
/// # Semantics
///
/// Identical to [`search`](super::search::search), with visited marks held in
/// a `BTreeSet` instead of a dense table:
///
/// - **Seeds** are visited at depth 0 in the order given; deduplicated under
///   [`VisitedPolicy::Global`], each starting a fresh path context under
///   [`VisitedPolicy::Path`].
/// - **Depth-first** explores the first pushed successor first.
///   **Breadth-first** visits in discovery order.
/// - **`max_depth` bounds expansion, not visiting.**
/// - **[`Visit::Skip`]** prunes a node's successors; the closure is not even
///   called for it, which matters when discovering successors costs a file
///   read.
/// - **[`ControlFlow::Break`]** returns immediately with `Some(value)`.
///
/// # Examples
///
/// Chasing a name through re-export barrels, named re-exports before stars,
/// stopping at the first module that defines it:
///
/// ```
/// use core::ops::ControlFlow;
///
/// use cfglib::{OpenSearchConfig, SearchOrder, Visit, open_search};
///
/// // Module 0 re-exports `Widget` from module 1 by name and has a star
/// // re-export of module 2; module 2 stars module 3. Both 1 and 3 define
/// // `Widget`, so the push order decides which one wins.
/// let successors = |node: &(u32, &'static str), out: &mut Vec<(u32, &'static str)>| {
///     let (module, name) = *node;
///     match module {
///         0 => {
///             out.push((1, name)); // named re-export first
///             out.push((2, name)); // then the star
///         }
///         2 => out.push((3, name)),
///         _ => {}
///     }
/// };
///
/// let defines = |&(module, _): &(u32, &str)| module == 1 || module == 3;
/// let found = open_search(
///     [(0, "Widget")],
///     OpenSearchConfig::new(SearchOrder::DepthFirst),
///     successors,
///     |node, _depth| {
///         if defines(node) {
///             return ControlFlow::Break(*node);
///         }
///         ControlFlow::Continue(Visit::Descend)
///     },
/// );
///
/// assert_eq!(found, Some((1, "Widget")));
/// ```
///
/// # Panics
///
/// Panics when `config` pairs [`SearchOrder::BreadthFirst`] with
/// [`VisitedPolicy::Path`] — a breadth-first frontier has no unwind on which
/// to un-mark.
#[must_use]
pub fn open_search<N: Clone + Ord, B>(
    seeds: impl IntoIterator<Item = N>,
    config: OpenSearchConfig,
    successors: impl FnMut(&N, &mut Vec<N>),
    visitor: impl FnMut(&N, usize) -> ControlFlow<B, Visit>,
) -> Option<B> {
    let seeds: Vec<N> = seeds.into_iter().collect();
    match (config.order, config.visited) {
        (SearchOrder::DepthFirst, VisitedPolicy::Global) => {
            open_depth_first_global(seeds, config, successors, visitor)
        }
        (SearchOrder::DepthFirst, VisitedPolicy::Path) => {
            open_depth_first_path(seeds, config, successors, visitor)
        }
        (SearchOrder::BreadthFirst, VisitedPolicy::Global) => {
            open_breadth_first_global(seeds, config, successors, visitor)
        }
        (SearchOrder::BreadthFirst, VisitedPolicy::Path) => {
            panic!(
                "VisitedPolicy::Path requires SearchOrder::DepthFirst: a breadth-first frontier never unwinds, so a path mark could never be removed"
            )
        }
    }
}

/// Whether a node at `depth` may expand under a `max_depth` bound.
fn may_expand(max_depth: Option<usize>, depth: usize) -> bool {
    max_depth.is_none_or(|limit| depth < limit)
}

fn open_depth_first_global<N: Clone + Ord, B>(
    seeds: Vec<N>,
    config: OpenSearchConfig,
    mut successors: impl FnMut(&N, &mut Vec<N>),
    mut visitor: impl FnMut(&N, usize) -> ControlFlow<B, Visit>,
) -> Option<B> {
    let mut visited: BTreeSet<N> = BTreeSet::new();
    // Reversed so the first seed pops first; the same convention applies to
    // successors below, so push order is expansion order.
    let mut stack: Vec<(N, usize)> = seeds.into_iter().rev().map(|seed| (seed, 0)).collect();
    let mut discovered = Vec::new();

    while let Some((node, depth)) = stack.pop() {
        if !visited.insert(node.clone()) {
            continue;
        }
        match visitor(&node, depth) {
            ControlFlow::Break(value) => return Some(value),
            ControlFlow::Continue(Visit::Skip) => continue,
            ControlFlow::Continue(Visit::Descend) => {}
        }
        if !may_expand(config.max_depth, depth) {
            continue;
        }
        discovered.clear();
        successors(&node, &mut discovered);
        for next in discovered.drain(..).rev() {
            if !visited.contains(&next) {
                stack.push((next, depth + 1));
            }
        }
    }

    None
}

/// One step of a path-marked open depth-first walk.
enum PathStep<N> {
    /// Enter `node` at this depth, marking it on the current path.
    Enter(N, usize),
    /// Leave `node`: the unwind point at which its path mark is removed.
    Leave(N),
}

fn open_depth_first_path<N: Clone + Ord, B>(
    seeds: Vec<N>,
    config: OpenSearchConfig,
    mut successors: impl FnMut(&N, &mut Vec<N>),
    mut visitor: impl FnMut(&N, usize) -> ControlFlow<B, Visit>,
) -> Option<B> {
    let mut on_path: BTreeSet<N> = BTreeSet::new();
    let mut stack: Vec<PathStep<N>> = seeds
        .into_iter()
        .rev()
        .map(|seed| PathStep::Enter(seed, 0))
        .collect();
    let mut discovered = Vec::new();

    while let Some(step) = stack.pop() {
        let (node, depth) = match step {
            // Every `Enter` pushes its own `Leave` before any successor, so
            // this pops exactly when the node's subtree is exhausted — and
            // between two seeds the path is empty again.
            PathStep::Leave(node) => {
                on_path.remove(&node);
                continue;
            }
            PathStep::Enter(node, depth) => (node, depth),
        };
        if !on_path.insert(node.clone()) {
            continue;
        }
        stack.push(PathStep::Leave(node.clone()));
        match visitor(&node, depth) {
            ControlFlow::Break(value) => return Some(value),
            ControlFlow::Continue(Visit::Skip) => continue,
            ControlFlow::Continue(Visit::Descend) => {}
        }
        if !may_expand(config.max_depth, depth) {
            continue;
        }
        discovered.clear();
        successors(&node, &mut discovered);
        for next in discovered.drain(..).rev() {
            stack.push(PathStep::Enter(next, depth + 1));
        }
    }

    None
}

fn open_breadth_first_global<N: Clone + Ord, B>(
    seeds: Vec<N>,
    config: OpenSearchConfig,
    mut successors: impl FnMut(&N, &mut Vec<N>),
    mut visitor: impl FnMut(&N, usize) -> ControlFlow<B, Visit>,
) -> Option<B> {
    let mut visited: BTreeSet<N> = BTreeSet::new();
    let mut queue: VecDeque<(N, usize)> = VecDeque::new();
    for seed in seeds {
        if visited.insert(seed.clone()) {
            queue.push_back((seed, 0));
        }
    }
    let mut discovered = Vec::new();

    while let Some((node, depth)) = queue.pop_front() {
        match visitor(&node, depth) {
            ControlFlow::Break(value) => return Some(value),
            ControlFlow::Continue(Visit::Skip) => continue,
            ControlFlow::Continue(Visit::Descend) => {}
        }
        if !may_expand(config.max_depth, depth) {
            continue;
        }
        discovered.clear();
        successors(&node, &mut discovered);
        for next in discovered.drain(..) {
            if visited.insert(next.clone()) {
                queue.push_back((next, depth + 1));
            }
        }
    }

    None
}

/// The discipline an [`open_depth_first_events`] walk runs under.
///
/// This is [`OpenSearchConfig`] minus its `order`: the walk is depth-first by
/// construction, because a [`Finish`](OpenDfsEvent::Finish) *is* an unwind and
/// a breadth-first frontier never unwinds. As with `direction`, the field is
/// absent rather than checked — it could only lie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenDfsConfig {
    /// Whether marks are held for the whole walk or only for the current
    /// path.
    pub visited: VisitedPolicy,
    /// Maximum depth to expand from, in hops from a seed. `None` is
    /// unbounded; `Some(0)` visits the seeds alone.
    pub max_depth: Option<usize>,
}

impl OpenDfsConfig {
    /// An unbounded walk marked under `visited`.
    ///
    /// The policy is the argument because it is the semantics: a fold that
    /// asks about *paths* (every base subobject that yields a name) needs
    /// [`VisitedPolicy::Path`], and one that asks about *nodes* (every module
    /// reached, once) needs [`VisitedPolicy::Global`].
    #[must_use]
    pub const fn new(visited: VisitedPolicy) -> Self {
        Self {
            visited,
            max_depth: None,
        }
    }

    /// Return this configuration bounded to `max_depth` hops from a seed.
    #[must_use]
    pub const fn with_max_depth(mut self, max_depth: usize) -> Self {
        self.max_depth = Some(max_depth);
        self
    }
}

/// An event of an [`open_depth_first_events`] walk.
///
/// Nodes are reported by reference: the walk owns its frontier, and a node in
/// an open space is usually a compound key — a `(file, name)` pair, a resolved
/// handle — that a consumer should clone only when it keeps one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenDfsEvent<'a, N> {
    /// The node was entered for the first time — under
    /// [`VisitedPolicy::Path`], for the first time *on this path* — at this
    /// depth in hops from its seed.
    ///
    /// This is the only event whose [`Visit`] verdict is read.
    Discover(&'a N, usize),
    /// Every successor of the node has finished, so its subtree is complete.
    ///
    /// Exactly one `Finish` follows every
    /// [`Discover`](OpenDfsEvent::Discover) that the walk does not break out
    /// from under, and the two nest strictly, so a visitor-side stack pushed
    /// on discovery is reduced here.
    Finish(&'a N),
    /// The walk declined to enter the node, at the depth it was reached from:
    /// its mark was already set, so it produces no
    /// [`Discover`](OpenDfsEvent::Discover), no
    /// [`Finish`](OpenDfsEvent::Finish), and no successors.
    ///
    /// Under [`VisitedPolicy::Path`] the node is already on the current path,
    /// which is the cycle guard — a consumer diagnosing a cyclic inheritance
    /// or import graph reports it from here. Under [`VisitedPolicy::Global`]
    /// it was entered earlier in the walk, and the event is the fold
    /// contribution this route does *not* get.
    ///
    /// The two are one event deliberately: distinguishing "on the path" from
    /// "already finished" under `Global` needs the tri-color state a dense
    /// [`depth_first_events`](super::search::depth_first_events) keeps, and a
    /// visitor that wants it already has the gray interval — it is exactly
    /// between a node's `Discover` and its `Finish`.
    Refused(&'a N, usize),
}

/// One frame of the explicit stack an [`open_depth_first_events`] walk keeps.
///
/// The successors themselves live in the walk's single arena, not in a `Vec`
/// per frame: frames are pushed and popped strictly last in, first out, so the
/// top frame always owns the arena's tail and a pop truncates back to where
/// that frame's successors began. One allocation for the walk (two with the
/// scratch buffer the closure is handed), instead of one per entered node.
struct OpenDfsFrame<N> {
    /// The node the frame stands on.
    node: N,
    /// Its depth in hops from the seed.
    depth: usize,
    /// Where its successors start in the arena, as the closure pushed them;
    /// the region is empty when the node was pruned or sat at the depth bound.
    start: usize,
    /// The next successor to enter, as an arena index. The frame's successors
    /// are exhausted when it reaches the arena's length, since the top frame's
    /// region runs to the end.
    cursor: usize,
}

/// Walk a lazily discovered node space depth-first from `seeds`, reporting
/// every discovery, every finish, and every refused re-entry, and returning
/// the value the callback broke with (or `None` when the walk ran to
/// completion).
///
/// [`open_search`] answers questions about nodes and can only stop; this
/// answers questions that are *folds*, where a node's answer is computed from
/// its subtrees' answers and therefore only exists on the unwind. C++ member
/// lookup is the motivating shape: a class answers with its own declaration
/// if it has one, else with the answer of exactly one yielding base
/// subobject, and two yielding subobjects make the name ambiguous. Under
/// [`VisitedPolicy::Path`] the marks release at
/// [`Finish`](OpenDfsEvent::Finish), so a diamond's shared base is folded
/// once per route through it — and that re-fold is precisely what makes the
/// ambiguity visible. A globally marked walk answers the same question
/// "unambiguous", confidently and wrongly.
///
/// `successors` is handed a node and a scratch buffer, **cleared before every
/// call**, and pushes that node's successors *in the order they should be
/// explored*, exactly as in [`open_search`]. Nodes are compared and marked by
/// `Ord`.
///
/// # Events
///
/// The event order is deterministic and part of the API. The walk is
/// iterative — an explicit frame stack, no recursion — so a deep space costs
/// heap, not stack.
///
/// - **[`Discover`](OpenDfsEvent::Discover)`(node, depth)`** is emitted when
///   the walk enters a node, seeds first at depth 0 in the order given and
///   then successors in the order the closure pushed them.
/// - **[`Finish`](OpenDfsEvent::Finish)`(node)`** is emitted when the node's
///   last successor has finished, strictly last in, first out. Every
///   `Discover` is matched by exactly one `Finish`, the walk breaking being
///   the one exception (below).
/// - **[`Refused`](OpenDfsEvent::Refused)`(node, depth)`** replaces that pair
///   when the mark policy declines the re-entry. The re-entry is *reported*
///   rather than silently dropped, because for a fold it is a real answer:
///   a cycle under [`VisitedPolicy::Path`], a lost contribution under
///   [`VisitedPolicy::Global`].
/// - **[`Visit::Skip`]** prunes: the successor closure is not called for that
///   node — which matters when discovering successors costs a file read —
///   and the node **still finishes**, immediately, as a leaf. Pruning is a
///   statement about the subtree, not about the node, and a fold that had to
///   reduce a pruned node somewhere other than its `Finish` would need two
///   reduction sites for one answer. (This is the C++ rule itself: a class
///   that declares the name hides every base declaration, so its bases are
///   never searched, and its own declaration is the answer it finishes with.)
/// - The verdict returned for a `Finish` or a `Refused` is **ignored**; there
///   is nothing left to prune. Return
///   `ControlFlow::Continue(`[`Visit::Descend`]`)`.
/// - **`max_depth` bounds expansion, not visiting**: a node at the bound is
///   discovered and finished, and its successors are never discovered.
/// - **Seeds**: under [`VisitedPolicy::Global`] a seed already reached from
///   an earlier seed is `Refused` at depth 0; under [`VisitedPolicy::Path`]
///   the stack is empty between seeds, so every mark has been released and a
///   repeated seed is walked again.
/// - **[`ControlFlow::Break`]** returns immediately with `Some(value)`: the
///   frames still on the stack do **not** finish, so a visitor that breaks
///   owns whatever it broke with rather than a completed fold.
///
/// # Examples
///
/// The C++ ambiguity, folded on the unwind — a visitor-side stack pushed at
/// `Discover` and reduced at `Finish`:
///
/// ```
/// use core::ops::ControlFlow;
///
/// use cfglib::{OpenDfsConfig, OpenDfsEvent, Visit, VisitedPolicy, open_depth_first_events};
///
/// // The classic non-virtual diamond: `d` derives from `b1` and `b2`, both
/// // derive from `base`, and only `base` declares the member.
/// let bases = |class: &&'static str, out: &mut Vec<&'static str>| match *class {
///     "d" => out.extend(["b1", "b2"]),
///     "b1" | "b2" => out.push("base"),
///     _ => {}
/// };
///
/// #[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// enum Lookup {
///     Nothing,
///     Found(&'static str),
///     Ambiguous,
/// }
///
/// // Exactly one yielding base subobject answers; two are an ambiguity.
/// let merge = |answer: Lookup, from_base: Lookup| match (answer, from_base) {
///     (Lookup::Nothing, other) | (other, Lookup::Nothing) => other,
///     _ => Lookup::Ambiguous,
/// };
///
/// let mut frames: Vec<Lookup> = Vec::new();
/// let mut answer = Lookup::Nothing;
/// let outcome = open_depth_first_events::<_, ()>(
///     ["d"],
///     OpenDfsConfig::new(VisitedPolicy::Path),
///     bases,
///     |event| {
///         match event {
///             OpenDfsEvent::Discover(class, _) => {
///                 if *class == "base" {
///                     // A declaration hides the bases below it: prune, and
///                     // finish immediately with this answer.
///                     frames.push(Lookup::Found(*class));
///                     return ControlFlow::Continue(Visit::Skip);
///                 }
///                 frames.push(Lookup::Nothing);
///             }
///             OpenDfsEvent::Finish(_) => {
///                 let found = frames.pop().unwrap_or(Lookup::Nothing);
///                 match frames.last_mut() {
///                     Some(parent) => *parent = merge(*parent, found),
///                     None => answer = found,
///                 }
///             }
///             OpenDfsEvent::Refused(..) => {}
///         }
///         ControlFlow::Continue(Visit::Descend)
///     },
/// );
///
/// assert_eq!(outcome, None);
/// // `base` was folded once per route, which is what makes it ambiguous.
/// assert_eq!(answer, Lookup::Ambiguous);
/// ```
#[must_use]
pub fn open_depth_first_events<N: Clone + Ord, B>(
    seeds: impl IntoIterator<Item = N>,
    config: OpenDfsConfig,
    mut successors: impl FnMut(&N, &mut Vec<N>),
    mut on_event: impl FnMut(OpenDfsEvent<'_, N>) -> ControlFlow<B, Visit>,
) -> Option<B> {
    let mut marks: BTreeSet<N> = BTreeSet::new();
    let mut stack: Vec<OpenDfsFrame<N>> = Vec::new();
    // Every frame's successors, appended as the frame is pushed and truncated
    // away as it pops, plus the buffer the closure writes into — which has to
    // be a separate one, because the closure is promised an empty `Vec` (a
    // consumer that sorts its successors would otherwise sort the arena).
    let mut arena: Vec<N> = Vec::new();
    let mut scratch: Vec<N> = Vec::new();

    // Enter a node: discover it and push its frame, or report the re-entry
    // the mark policy refuses. Used for seeds and successors alike, so the
    // two cannot drift apart.
    macro_rules! enter {
        ($node:expr, $depth:expr) => {{
            let node: N = $node;
            let depth: usize = $depth;
            if marks.insert(node.clone()) {
                let verdict = match on_event(OpenDfsEvent::Discover(&node, depth)) {
                    ControlFlow::Break(value) => return Some(value),
                    ControlFlow::Continue(verdict) => verdict,
                };
                scratch.clear();
                if matches!(verdict, Visit::Descend) && may_expand(config.max_depth, depth) {
                    successors(&node, &mut scratch);
                }
                let start = arena.len();
                arena.append(&mut scratch);
                stack.push(OpenDfsFrame {
                    node,
                    depth,
                    start,
                    cursor: start,
                });
            } else if let ControlFlow::Break(value) = on_event(OpenDfsEvent::Refused(&node, depth))
            {
                return Some(value);
            }
        }};
    }

    for seed in seeds {
        enter!(seed, 0);
        while let Some(frame) = stack.last_mut() {
            if frame.cursor < arena.len() {
                let next = arena[frame.cursor].clone();
                let depth = frame.depth + 1;
                frame.cursor += 1;
                enter!(next, depth);
                continue;
            }
            let Some(finished) = stack.pop() else { break };
            arena.truncate(finished.start);
            if matches!(config.visited, VisitedPolicy::Path) {
                marks.remove(&finished.node);
            }
            if let ControlFlow::Break(value) = on_event(OpenDfsEvent::Finish(&finished.node)) {
                return Some(value);
            }
        }
    }

    None
}

/// Follow an out-degree ≤ 1 chase from `seed` and return where it ends.
///
/// `step` reports the next node, or `None` at the end of the chain. The chase
/// stops — returning the **last** node it stood on — when `step` yields
/// `None`, when `max_hops` steps have been taken, or when the next node is
/// already on the path walked so far.
///
/// The cycle guard is the **full path**, not just the seed: an alias chain
/// that re-enters itself three hops in is a cycle even though it never
/// returns to where it started. `max_hops` keeps that scan small (and is the
/// real bound for a chase over an untrusted graph).
///
/// # Examples
///
/// ```
/// use cfglib::follow;
///
/// // A chain of type aliases: 0 -> 1 -> 2, and 2 is the real definition.
/// let chain = |node: &u32| match node {
///     0 => Some(1),
///     1 => Some(2),
///     _ => None,
/// };
/// assert_eq!(follow(0, 16, chain), 2);
///
/// // A cycle stops at the last node before it closes.
/// let cyclic = |node: &u32| Some((node + 1) % 3);
/// assert_eq!(follow(0, 16, cyclic), 2);
///
/// // So does the hop bound.
/// assert_eq!(follow(0, 1, cyclic), 1);
/// ```
#[must_use]
pub fn follow<N: PartialEq + Clone>(
    seed: N,
    max_hops: usize,
    step: impl FnMut(&N) -> Option<N>,
) -> N {
    let last = seed.clone();
    follow_path(seed, max_hops, step).pop().unwrap_or(last)
}

/// Follow an out-degree ≤ 1 chase from `seed` and return the whole chain,
/// `seed` first.
///
/// The chain is what [`follow`] walks — same termination, same full-path
/// cycle guard — kept instead of discarded, for a consumer that reports the
/// route (a diagnostic naming every alias between a use and its definition)
/// or inspects it. The result always contains `seed`, so it is never empty,
/// and holds at most `max_hops + 1` nodes.
///
/// # Examples
///
/// ```
/// use cfglib::follow_path;
///
/// let cyclic = |node: &u32| Some((node + 1) % 3);
/// assert_eq!(follow_path(0, 16, cyclic), vec![0, 1, 2]);
/// assert_eq!(follow_path(0, 1, cyclic), vec![0, 1]);
///
/// let end = |_: &u32| None;
/// assert_eq!(follow_path(7, 16, end), vec![7]);
/// ```
#[must_use]
pub fn follow_path<N: PartialEq + Clone>(
    seed: N,
    max_hops: usize,
    mut step: impl FnMut(&N) -> Option<N>,
) -> Vec<N> {
    let mut chain = Vec::new();
    chain.push(seed);

    for _ in 0..max_hops {
        let Some(current) = chain.last() else { break };
        let Some(next) = step(current) else { break };
        if chain.contains(&next) {
            break;
        }
        chain.push(next);
    }

    chain
}

#[cfg(test)]
mod tests;
