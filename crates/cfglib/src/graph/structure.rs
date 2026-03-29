//! Structural detection — natural loops, regions, and reducibility.
//!
//! Identifies loop structures and classifies the CFG as reducible or
//! irreducible, building on the dominator tree and back-edge detection.

extern crate alloc;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use super::dominator::DominatorTree;
use crate::block::BlockId;
use crate::cfg::Cfg;
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
    /// The block at the tail of the back-edge (the source of the jump).
    pub tail: BlockId,
    /// The loop header block (the target of the back-edge).
    pub header: BlockId,
}

/// Find all back-edges in the CFG (edges where the target dominates
/// the source).
///
/// An edge is considered a back-edge if **either**:
/// - The builder already tagged it as [`EdgeKind::Back`] (explicit
///   structural back-edges from `loop` / `continue`), **or**
/// - The dominator tree confirms that the target dominates the source
///   (catching any natural back-edges that the builder did not tag,
///   e.g. from unstructured goto-style control flow).
///
/// The result is deduplicated, so an edge satisfying both conditions
/// appears only once.
pub fn find_back_edges<I>(cfg: &Cfg<I>, dom: &DominatorTree) -> Vec<BackEdge> {
    let mut backs = Vec::new();
    for edge in cfg.edges() {
        if edge.kind() == EdgeKind::Back || dom.dominates(edge.target(), edge.source()) {
            backs.push(BackEdge {
                tail: edge.source(),
                header: edge.target(),
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

    // Compute nesting depth in O(L × max_body) instead of O(L²):
    // Build a map from block → number of loops containing it, then
    // each loop's depth = (count of its header) − 1 (itself).
    {
        let block_count = cfg.num_blocks();
        let mut containing: Vec<u32> = alloc::vec![0; block_count];
        for lp in &loops {
            for &b in &lp.body {
                containing[b.index()] += 1;
            }
        }
        for lp in loops.iter_mut() {
            // Every loop's body includes its own header, so subtract 1.
            lp.depth = (containing[lp.header.index()] - 1) as usize;
        }
    }

    // Sort by depth (outermost first), then by header.
    loops.sort_by(|a, b| a.depth.cmp(&b.depth).then(a.header.cmp(&b.header)));
    loops
}

/// Whether the CFG is reducible.
///
/// A CFG is **reducible** if and only if every cycle in the graph
/// contains a node that dominates all other nodes in that cycle.
/// Equivalently, every retreating edge in a DFS is a back-edge
/// (target dominates source).
///
/// This implementation checks every edge in the CFG: if `target`
/// dominates `source` the edge is a natural back-edge (fine); any
/// other edge where `target` was already visited but does **not**
/// dominate `source` witnesses an irreducible cycle.
pub fn is_reducible<I>(cfg: &Cfg<I>, dom: &DominatorTree) -> bool {
    // DFS to classify edges. An edge to an ancestor that doesn't
    // dominate the source is irreducible.
    let n = cfg.num_blocks();
    if n == 0 {
        return true;
    }

    const WHITE: u8 = 0;
    const GRAY: u8 = 1;
    const BLACK: u8 = 2;

    let mut color = alloc::vec![WHITE; n];
    let mut stack: Vec<(BlockId, bool)> = alloc::vec![(cfg.entry(), false)];

    while let Some((node, processed)) = stack.pop() {
        if processed {
            color[node.index()] = BLACK;
            continue;
        }
        if color[node.index()] != WHITE {
            continue;
        }
        color[node.index()] = GRAY;
        stack.push((node, true));

        for succ in cfg.successors(node) {
            match color[succ.index()] {
                WHITE => stack.push((succ, false)),
                GRAY => {
                    // Retreating edge — must be a natural back-edge.
                    if !dom.dominates(succ, node) {
                        return false;
                    }
                }
                _ => {} // Cross/forward edge — fine.
            }
        }
    }
    true
}

// ── Loop canonicalization ───────────────────────────────────────────

/// Information about a canonicalized loop.
#[derive(Debug, Clone)]
pub struct CanonicalLoop {
    /// The original natural loop.
    pub natural_loop: NaturalLoop,
    /// The preheader block (newly inserted).
    pub preheader: BlockId,
    /// Exit blocks — blocks outside the loop that are targets of edges
    /// from inside the loop.
    pub exits: BTreeSet<BlockId>,
}

/// Insert a dedicated **preheader** block for a natural loop.
///
/// A preheader is a single-successor block that becomes the sole
/// non-backedge predecessor of the loop header. This simplifies
/// many loop transformations (LICM, unrolling, etc.).
///
/// Returns the `BlockId` of the new preheader, or `None` if a
/// preheader was not needed (single non-backedge predecessor).
pub fn insert_preheader<I: Clone>(cfg: &mut Cfg<I>, lp: &NaturalLoop) -> Option<BlockId> {
    // Collect non-backedge predecessors of the header.
    let outside_preds: Vec<crate::edge::EdgeId> = cfg
        .predecessor_edges(lp.header)
        .iter()
        .copied()
        .filter(|&eid| {
            let src = cfg.edge(eid).source();
            !lp.body.contains(&src)
        })
        .collect();

    if outside_preds.len() <= 1 {
        return None; // already canonical
    }

    let preheader = cfg.new_block();

    // Redirect all outside predecessor edges to target the preheader.
    for eid in &outside_preds {
        let edge = cfg.edge(*eid);
        let src = edge.source();
        let kind = edge.kind();
        cfg.remove_edge(*eid);
        cfg.add_edge(src, preheader, kind);
    }

    // Add fallthrough from preheader to the header.
    cfg.add_edge(preheader, lp.header, crate::edge::EdgeKind::Fallthrough);

    Some(preheader)
}

/// Identify exit blocks of a natural loop.
///
/// An exit block is any block **outside** the loop body that has a
/// predecessor inside the loop body.
pub fn loop_exit_blocks<I>(cfg: &Cfg<I>, lp: &NaturalLoop) -> BTreeSet<BlockId> {
    let mut exits = BTreeSet::new();
    for &b in &lp.body {
        for s in cfg.successors(b) {
            if !lp.body.contains(&s) {
                exits.insert(s);
            }
        }
    }
    exits
}

/// Canonicalize all loops: insert preheaders and identify exits.
pub fn canonicalize_loops<I: Clone>(cfg: &mut Cfg<I>, dom: &DominatorTree) -> Vec<CanonicalLoop> {
    let loops = detect_loops(cfg, dom);
    let mut result = Vec::new();

    for lp in loops {
        let exits = loop_exit_blocks(cfg, &lp);
        let preheader = insert_preheader(cfg, &lp).unwrap_or_else(|| {
            // No new preheader needed; use the single outside pred.
            let outside: Vec<BlockId> = cfg
                .predecessors(lp.header)
                .filter(|p| !lp.body.contains(p))
                .collect();
            outside.into_iter().next().unwrap_or(lp.header)
        });

        result.push(CanonicalLoop {
            natural_loop: lp,
            preheader,
            exits,
        });
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::CfgBuilder;
    use crate::flow::FlowEffect;
    use crate::graph::dominator::DominatorTree;
    use crate::test_util::{MockInst, ff};
    use alloc::vec;

    #[test]
    fn no_loops_in_linear_cfg() {
        let cfg = CfgBuilder::build(vec![ff("a"), ff("b"), ff("c")]).unwrap();
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
        ])
        .unwrap();
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
        ])
        .unwrap();
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
        ])
        .unwrap();
        let dom = DominatorTree::compute(&cfg);
        let loops = detect_loops(&cfg, &dom);
        assert_eq!(loops.len(), 1);
    }

    #[test]
    fn linear_cfg_is_reducible() {
        let cfg = CfgBuilder::build(vec![ff("a"), ff("b")]).unwrap();
        let dom = DominatorTree::compute(&cfg);
        assert!(is_reducible(&cfg, &dom));
    }

    #[test]
    fn loop_cfg_is_reducible() {
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::LoopOpen, "loop"),
            ff("body"),
            MockInst(FlowEffect::LoopClose, "endloop"),
        ])
        .unwrap();
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
        ])
        .unwrap();
        let dom = DominatorTree::compute(&cfg);
        let loops = detect_loops(&cfg, &dom);
        assert!(loops.is_empty());
        assert!(is_reducible(&cfg, &dom));
    }

    #[test]
    fn loop_exit_blocks_found() {
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::LoopOpen, "loop"),
            MockInst(FlowEffect::ConditionalBreak, "breakc"),
            ff("body"),
            MockInst(FlowEffect::LoopClose, "endloop"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        let dom = DominatorTree::compute(&cfg);
        let loops = detect_loops(&cfg, &dom);
        assert!(!loops.is_empty());
        let exits = loop_exit_blocks(&cfg, &loops[0]);
        assert!(
            !exits.is_empty(),
            "loop should have at least one exit block"
        );
    }

    #[test]
    fn canonicalize_loops_adds_exits() {
        let mut cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::LoopOpen, "loop"),
            MockInst(FlowEffect::ConditionalBreak, "breakc"),
            ff("body"),
            MockInst(FlowEffect::LoopClose, "endloop"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        let dom = DominatorTree::compute(&cfg);
        let canonical = canonicalize_loops(&mut cfg, &dom);
        assert!(!canonical.is_empty());
        assert!(!canonical[0].exits.is_empty());
    }
}
