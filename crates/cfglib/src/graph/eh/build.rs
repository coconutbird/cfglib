extern crate alloc;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::region::{Cleanup, HandlerRef};

use super::{EhBlockKind, EhEdge, EhEdgeKind, EhModel};

/// Build an EH model by analysing edge kinds and region metadata.
///
/// Targets of handler/unwind edges are classified as landing pads. Sources of
/// resume/continue edges are classified as resume points. Explicit [`Region`]
/// metadata is authoritative, so a `finally` or `fault` target remains a
/// cleanup even when an unwind edge also reaches it.
///
/// [`Region`]: crate::Region
///
/// Cleanup records the frontend attached to a handler
/// ([`Cfg::add_continuation`]) are carried into [`EhModel::cleanups`], keyed
/// by that handler's entry block, so an analysis reads cleanup-then-continue
/// structure instead of a fan of indistinguishable out-edges.
///
/// # Examples
///
/// ```
/// use cfglib::{Cfg, EdgeKind, build_eh_model};
///
/// let mut cfg = Cfg::<u32>::new();
/// let b0 = cfg.entry();
/// let b1 = cfg.new_block();
/// cfg.add_edge(b0, b1, EdgeKind::Fallthrough);
///
/// let model = build_eh_model(&cfg);
/// // No exception edges, so no landing pads.
/// assert!(model.eh_edges.is_empty());
/// ```
#[must_use]
pub fn build_eh_model<I, E>(cfg: &Cfg<I, E>) -> EhModel {
    EhModelBuilder::from_cfg(cfg).finish()
}

#[derive(Default)]
struct EhModelBuilder {
    block_kinds: BTreeMap<BlockId, EhBlockKind>,
    eh_edges: Vec<EhEdge>,
    protected_by: BTreeMap<BlockId, BTreeSet<BlockId>>,
    handlers: BTreeMap<BlockId, Vec<HandlerRef>>,
    cleanups: BTreeMap<BlockId, Cleanup>,
}

impl EhModelBuilder {
    fn from_cfg<I, E>(cfg: &Cfg<I, E>) -> Self {
        let mut builder = Self::default();
        builder.classify_edges(cfg);
        builder.classify_regions(cfg);
        builder.fill_normal_blocks(cfg);
        builder
    }

    fn classify_edges<I, E>(&mut self, cfg: &Cfg<I, E>) {
        for edge in cfg.edges() {
            let Some(kind) = EhEdgeKind::from_cfg(edge.kind()) else {
                continue;
            };
            self.eh_edges.push(EhEdge {
                edge_id: edge.id(),
                from: edge.source(),
                to: edge.target(),
                kind,
                is_unwind: kind.is_unwind(),
            });
            match kind {
                EhEdgeKind::Handler | EhEdgeKind::Unwind => {
                    self.block_kinds
                        .entry(edge.target())
                        .or_insert(EhBlockKind::LandingPad);
                    self.protected_by
                        .entry(edge.target())
                        .or_default()
                        .insert(edge.source());
                }
                EhEdgeKind::Resume | EhEdgeKind::Continue => {
                    self.block_kinds
                        .entry(edge.source())
                        .or_insert(EhBlockKind::Resume);
                }
                EhEdgeKind::Leave => {}
            }
        }
    }

    fn classify_regions<I, E>(&mut self, cfg: &Cfg<I, E>) {
        for region in cfg.regions() {
            for (index, handler) in region.handlers.iter().enumerate() {
                let target = handler.entry;
                let handler_ref = HandlerRef::new(region.id, index);
                self.handlers.entry(target).or_default().push(handler_ref);
                if let Some(cleanup) = cfg.cleanup(handler_ref) {
                    self.cleanups.insert(target, cleanup.clone());
                }
                // Region metadata is more precise than the role inferred from
                // an exception edge, so this intentionally overwrites it.
                self.block_kinds.insert(target, handler.kind.into());
                self.protected_by
                    .entry(target)
                    .or_default()
                    .extend(region.protected_blocks.iter().copied());
            }
        }
    }

    fn fill_normal_blocks<I, E>(&mut self, cfg: &Cfg<I, E>) {
        for block in cfg.blocks() {
            self.block_kinds
                .entry(block.id())
                .or_insert(EhBlockKind::Normal);
        }
    }

    fn finish(self) -> EhModel {
        EhModel {
            block_kinds: self.block_kinds,
            eh_edges: self.eh_edges,
            protected_by: self.protected_by,
            handlers: self.handlers,
            cleanups: self.cleanups,
        }
    }
}
