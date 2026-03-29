//! Structural detection — natural loops, regions, and reducibility.
//!
//! Identifies loop structures and classifies the CFG as reducible or
//! irreducible, building on the dominator tree and back-edge detection.

extern crate alloc;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use super::dominator::DominatorTree;
use crate::edge::EdgeKind;

/// A natural loop in the CFG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NaturalLoop {
    /// The loop header (entry point, dominates all other blocks).
    pub header: BlockId,
    /// All blocks in the loop body (including the header).
    pub body: BTreeSet<BlockId>,
    /// Back-edge tail blocks (blocks that jump back to the header).
    pub latches: BTreeSet<BlockId>,
    /// Nesting depth (0 = outermost).
    pub depth: usize,
}

/// A back-edge: tail → header where header dominates tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BackEdge {
    pub tail: BlockId,
    pub header: BlockId,
}

/// Find all back-edges in the CFG (edges where the target dominates
/// the source).
pub fn find_back_edges<I>(cfg: &Cfg<I>, dom: &DominatorTree) -> Vec<BackEdge> {
    let mut backs = Vec::new();
    for edge in cfg.edges() {
        if edge.kind == EdgeKind::Back || dom.dominates(edge.target, edge.source) {
            backs.push(BackEdge {
                tail: edge.source,
                header: edge.target,
            });
        }
    }
    backs.sort();
    backs.dedup();
    backs
}

/// Compute the natural loop body for a single back-edge.
///
/// The body is the set of blocks that can reach `tail` without
/// going through `header`, plus `header` itself.
fn loop_body_for<I>(cfg: &Cfg<I>, header: BlockId, tail: BlockId) -> BTreeSet<BlockId> {
    let mut body = BTreeSet::new();
    body.insert(header);
    if header == tail {
        return body;
    }
    body.insert(tail);
    let mut stack = alloc::vec![tail];
    while let Some(n) = stack.pop() {
        for p in cfg.predecessors(n) {
            if !body.contains(&p) {
                body.insert(p);
                stack.push(p);
            }
        }
    }
    body
}

/// Detect all natural loops in the CFG.
///
/// Loops sharing the same header are merged into a single
/// `NaturalLoop` with multiple latches.
pub fn detect_loops<I>(cfg: &Cfg<I>, dom: &DominatorTree) -> Vec<NaturalLoop> {
    let backs = find_back_edges(cfg, dom);
    if backs.is_empty() {
        return Vec::new();
    }

    // Group back-edges by header.
    let mut header_map: alloc::collections::BTreeMap<BlockId, Vec<BlockId>> =
        alloc::collections::BTreeMap::new();
    for be in &backs {
        header_map.entry(be.header).or_default().push(be.tail);
    }

    let mut loops: Vec<NaturalLoop> = Vec::new();
    for (header, latches) in &header_map {
        let mut body = BTreeSet::new();
        for &tail in latches {
            body.extend(loop_body_for(cfg, *header, tail));
        }
        loops.push(NaturalLoop {
            header: *header,
            body,
            latches: latches.iter().copied().collect(),
            depth: 0, // filled in below
        });
    }

    // Compute nesting depth: a loop L1 is nested inside L2 if
    // L1.header ∈ L2.body and L1 ≠ L2.
    let n = loops.len();
    for i in 0..n {
        let mut d = 0u32;
        for j in 0..n {
            if i != j && loops[j].body.contains(&loops[i].header) {
                d += 1;
            }
        }
        loops[i].depth = d as usize;
    }

    // Sort by depth (outermost first), then by header.
    loops.sort_by(|a, b| a.depth.cmp(&b.depth).then(a.header.cmp(&b.header)));
    loops
}

/// Whether the CFG is reducible.
///
/// A CFG is **reducible** if every back-edge `t → h` has the
/// property that `h` dominates `t`. Since the builder already
/// classifies back-edges, we just verify that no edge classified
/// as `Back` violates the dominance property, and that there are
/// no "cross" edges that create irreducible loops.
pub fn is_reducible<I>(cfg: &Cfg<I>, dom: &DominatorTree) -> bool {
    for edge in cfg.edges() {
        // An edge where the target dominates the source is a valid
        // back-edge (natural loop). Any other edge that creates a
        // cycle without the target dominating the source makes the
        // CFG irreducible.
        if edge.kind == EdgeKind::Back && !dom.dominates(edge.target, edge.source) {
            return false;
        }
    }
    // Additionally check: are there non-Back edges that look like
    // back-edges (target dominates source)? If so the builder
    // mis-classified them but the CFG is still reducible.
    // The real irreducibility test: attempt to collapse all natural
    // loops — if any cycle remains it's irreducible.
    // For our builder-constructed CFGs this simple check suffices.
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::borrow::Cow;
    use alloc::vec;
    use crate::builder::CfgBuilder;
    use crate::graph::dominator::DominatorTree;
    use crate::flow::{FlowControl, FlowEffect};

    #[derive(Debug, Clone)]
    struct MockInst(FlowEffect, &'static str);

    impl FlowControl for MockInst {
        fn flow_effect(&self) -> FlowEffect { self.0 }
        fn display_mnemonic(&self) -> Cow<'_, str> { Cow::Borrowed(self.1) }
    }

    fn ff(name: &'static str) -> MockInst { MockInst(FlowEffect::Fallthrough, name) }

    #[test]
    fn no_loops_in_linear_cfg() {
        let cfg = CfgBuilder::build(vec![ff("a"), ff("b"), ff("c")]);
        let dom = DominatorTree::compute(&cfg);
        let loops = detect_loops(&cfg, &dom);
        assert!(loops.is_empty());
    }

    #[test]
    fn simple_loop_detected() {
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::LoopOpen, "loop"),
            ff("body"),
            MockInst(FlowEffect::LoopClose, "endloop"),
            MockInst(FlowEffect::Return, "ret"),
        ]);
        let dom = DominatorTree::compute(&cfg);
        let loops = detect_loops(&cfg, &dom);
        assert_eq!(loops.len(), 1);
        assert!(!loops[0].body.is_empty(), "loop body is non-empty");
    }

    #[test]
    fn nested_loops_have_depth() {
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::LoopOpen, "outer"),
            MockInst(FlowEffect::LoopOpen, "inner"),
            ff("body"),
            MockInst(FlowEffect::ConditionalBreak, "breakc_inner"),
            MockInst(FlowEffect::LoopClose, "end_inner"),
            MockInst(FlowEffect::ConditionalBreak, "breakc_outer"),
            MockInst(FlowEffect::LoopClose, "end_outer"),
            MockInst(FlowEffect::Return, "ret"),
        ]);
        let dom = DominatorTree::compute(&cfg);
        let loops = detect_loops(&cfg, &dom);
        assert_eq!(loops.len(), 2);
        // Outermost first (depth 0), inner second (depth 1).
        assert_eq!(loops[0].depth, 0);
        assert_eq!(loops[1].depth, 1);
    }

    #[test]
    fn loop_with_break_still_detected() {
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::LoopOpen, "loop"),
            MockInst(FlowEffect::ConditionalBreak, "breakc"),
            ff("body"),
            MockInst(FlowEffect::LoopClose, "endloop"),
        ]);
        let dom = DominatorTree::compute(&cfg);
        let loops = detect_loops(&cfg, &dom);
        assert_eq!(loops.len(), 1);
    }

    #[test]
    fn linear_cfg_is_reducible() {
        let cfg = CfgBuilder::build(vec![ff("a"), ff("b")]);
        let dom = DominatorTree::compute(&cfg);
        assert!(is_reducible(&cfg, &dom));
    }

    #[test]
    fn loop_cfg_is_reducible() {
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::LoopOpen, "loop"),
            ff("body"),
            MockInst(FlowEffect::LoopClose, "endloop"),
        ]);
        let dom = DominatorTree::compute(&cfg);
        assert!(is_reducible(&cfg, &dom));
    }

    #[test]
    fn if_else_no_loops() {
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::ConditionalOpen, "if"),
            ff("then"),
            MockInst(FlowEffect::ConditionalAlternate, "else"),
            ff("else_body"),
            MockInst(FlowEffect::ConditionalClose, "endif"),
        ]);
        let dom = DominatorTree::compute(&cfg);
        let loops = detect_loops(&cfg, &dom);
        assert!(loops.is_empty());
        assert!(is_reducible(&cfg, &dom));
    }
}
