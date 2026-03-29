//! Exception handling (EH) modelling.
//!
//! Provides first-class support for EH control flow — landing pads,
//! cleanup blocks, and unwind edges — enabling accurate modelling of
//! try/catch/finally in decompilation and analysis.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::edge::EdgeKind;

/// Classification of a block's role in exception handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EhBlockKind {
    /// Normal code — not part of an EH construct.
    Normal,
    /// A landing pad — first block of an exception handler.
    LandingPad,
    /// A cleanup block — executes during stack unwinding (finally).
    Cleanup,
    /// A catch dispatch — selects among multiple handlers.
    CatchSwitch,
    /// A resume/rethrow point.
    Resume,
}

/// An exception handling edge annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EhEdge {
    /// Source block (may throw).
    pub from: BlockId,
    /// Target block (handler / cleanup).
    pub to: BlockId,
    /// Whether this is an unwind edge (vs normal flow).
    pub is_unwind: bool,
}

/// EH model for a CFG.
#[derive(Debug, Clone)]
pub struct EhModel {
    /// Classification of each block.
    pub block_kinds: BTreeMap<BlockId, EhBlockKind>,
    /// All EH (unwind) edges.
    pub eh_edges: Vec<EhEdge>,
    /// Landing pad → set of blocks it protects.
    pub protected_by: BTreeMap<BlockId, BTreeSet<BlockId>>,
}

/// Build an EH model by analysing edge kinds and region metadata.
///
/// Blocks reachable only via `Exception` edges are classified as
/// landing pads. Blocks that are targets of the existing `Region`
/// handlers are also incorporated.
pub fn build_eh_model<I>(cfg: &Cfg<I>) -> EhModel {
    let mut block_kinds = BTreeMap::new();
    let mut eh_edges = Vec::new();
    let mut protected_by: BTreeMap<BlockId, BTreeSet<BlockId>> = BTreeMap::new();

    // Classify from edge kinds.
    for edge in cfg.edges() {
        match edge.kind() {
            EdgeKind::ExceptionHandler | EdgeKind::ExceptionUnwind => {
                eh_edges.push(EhEdge {
                    from: edge.source(),
                    to: edge.target(),
                    is_unwind: matches!(edge.kind(), EdgeKind::ExceptionUnwind),
                });
                block_kinds
                    .entry(edge.target())
                    .or_insert(EhBlockKind::LandingPad);
                protected_by
                    .entry(edge.target())
                    .or_default()
                    .insert(edge.source());
            }
            _ => {}
        }
    }

    // Classify from region metadata.
    for region in cfg.regions() {
        for handler in &region.handlers {
            let target = handler.entry;
            block_kinds.entry(target).or_insert(match handler.kind {
                crate::region::HandlerKind::Catch | crate::region::HandlerKind::CatchAll => {
                    EhBlockKind::LandingPad
                }
                crate::region::HandlerKind::Finally => EhBlockKind::Cleanup,
                crate::region::HandlerKind::Filter { .. } => EhBlockKind::CatchSwitch,
                crate::region::HandlerKind::Fault => EhBlockKind::Cleanup,
            });
            for &bid in &region.protected_blocks {
                protected_by.entry(target).or_default().insert(bid);
            }
        }
    }

    // All remaining blocks are Normal.
    for block in cfg.blocks() {
        block_kinds.entry(block.id()).or_insert(EhBlockKind::Normal);
    }

    EhModel {
        block_kinds,
        eh_edges,
        protected_by,
    }
}

/// Returns all landing pad blocks.
pub fn landing_pads(model: &EhModel) -> Vec<BlockId> {
    model
        .block_kinds
        .iter()
        .filter(|&(_, k)| *k == EhBlockKind::LandingPad)
        .map(|(&bid, _)| bid)
        .collect()
}

/// Returns all cleanup blocks.
pub fn cleanup_blocks(model: &EhModel) -> Vec<BlockId> {
    model
        .block_kinds
        .iter()
        .filter(|&(_, k)| *k == EhBlockKind::Cleanup)
        .map(|(&bid, _)| bid)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::edge::EdgeKind;
    use crate::test_util::ff;

    #[test]
    fn no_eh_all_normal() {
        let mut cfg = Cfg::new();
        let b = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("a"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.add_edge(cfg.entry(), b, EdgeKind::Fallthrough);
        let model = build_eh_model(&cfg);
        assert!(model.eh_edges.is_empty());
        assert!(
            model
                .block_kinds
                .values()
                .all(|&k| k == EhBlockKind::Normal)
        );
    }

    #[test]
    fn exception_edge_creates_landing_pad() {
        let mut cfg = Cfg::new();
        let handler = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("call"));
        cfg.block_mut(handler)
            .instructions_vec_mut()
            .push(ff("catch"));
        cfg.add_edge(cfg.entry(), handler, EdgeKind::ExceptionHandler);
        let model = build_eh_model(&cfg);
        assert_eq!(model.eh_edges.len(), 1);
        assert_eq!(model.block_kinds[&handler], EhBlockKind::LandingPad);
        assert!(model.protected_by[&handler].contains(&cfg.entry()));
    }

    #[test]
    fn landing_pads_query() {
        let mut cfg = Cfg::new();
        let lp = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("try"));
        cfg.block_mut(lp).instructions_vec_mut().push(ff("handler"));
        cfg.add_edge(cfg.entry(), lp, EdgeKind::ExceptionHandler);
        let model = build_eh_model(&cfg);
        let pads = landing_pads(&model);
        assert_eq!(pads.len(), 1);
        assert_eq!(pads[0], lp);
    }
}
