//! Dominator-derived recovery of unknown handler extents.

extern crate alloc;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::graph::dominator::DominatorTree;
use crate::region::HandlerBody;

/// Promotes every reachable [`HandlerBody::Unknown`] extent to the blocks
/// dominated by its handler entry, returning how many were promoted.
///
/// A block dominated by the handler entry is reachable only through the
/// handler, so the dominated set is a **sound** extent: it can under-cover
/// (code shared with another handler or with normal flow stays outside and
/// degrades to explicit gotos during structuring) but never claims a block
/// the handler does not own. Unreachable handler entries stay `Unknown`.
///
/// Run this on a **derived** graph (a clone lifted for presentation), not
/// on canonical frontend metadata: exact tables that encode only a handler
/// entry should keep saying so.
pub fn promote_handler_extents<I, E>(cfg: &mut Cfg<I, E>) -> usize {
    let dominators = DominatorTree::compute(cfg);
    let block_ids: Vec<BlockId> = cfg.blocks().iter().map(crate::BasicBlock::id).collect();
    let mut promotions = Vec::new();
    for (region_index, region) in cfg.regions().iter().enumerate() {
        for (handler_index, handler) in region.handlers.iter().enumerate() {
            if handler.body.is_known() {
                continue;
            }
            let entry = handler.entry;
            let reachable = entry == cfg.entry() || dominators.idom(entry).is_some();
            if !reachable {
                continue;
            }
            let extent: BTreeSet<BlockId> = block_ids
                .iter()
                .copied()
                .filter(|&block| dominators.dominates(entry, block))
                .collect();
            promotions.push((region_index, handler_index, extent));
        }
    }
    let promoted = promotions.len();
    for (region_index, handler_index, extent) in promotions {
        let region_id = cfg.regions()[region_index].id;
        if let Some(region) = cfg.region_mut(region_id) {
            if let Some(handler) = region.handlers.get_mut(handler_index) {
                handler.body = HandlerBody::Known(extent);
            }
        }
    }
    promoted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::EdgeKind;
    use crate::region::{Handler, HandlerKind, Region, RegionId};
    use crate::test_util::{MockInst, ff};

    #[test]
    fn promotes_exclusive_handler_code_and_keeps_shared_joins_out() {
        let mut cfg: Cfg<MockInst> = Cfg::new();
        let protected = cfg.new_block();
        let landing = cfg.new_block();
        let handler_tail = cfg.new_block();
        let join = cfg.new_block();
        cfg.block_mut(cfg.entry()).push(ff("entry"));
        cfg.block_mut(protected).push(ff("try_inst"));
        cfg.block_mut(landing).push(ff("caught"));
        cfg.block_mut(handler_tail).push(ff("handler_tail"));
        cfg.block_mut(join).push(ff("after"));
        cfg.add_edge(cfg.entry(), protected, EdgeKind::Fallthrough);
        cfg.add_edge(protected, join, EdgeKind::Fallthrough);
        cfg.add_edge(protected, landing, EdgeKind::ExceptionUnwind);
        cfg.add_edge(landing, handler_tail, EdgeKind::Fallthrough);
        cfg.add_edge(handler_tail, join, EdgeKind::Fallthrough);
        cfg.add_region(Region {
            id: RegionId::from_raw(0),
            protected_blocks: [protected].into_iter().collect(),
            handlers: alloc::vec![Handler {
                entry: landing,
                body: HandlerBody::Unknown,
                kind: HandlerKind::CatchAll,
            }],
            parent: None,
        });

        assert_eq!(promote_handler_extents(&mut cfg), 1);
        let body = cfg.regions()[0].handlers[0]
            .body
            .blocks()
            .expect("promoted");
        assert!(body.contains(&landing));
        assert!(body.contains(&handler_tail));
        assert!(
            !body.contains(&join),
            "the shared continuation stays outside the extent"
        );

        // A second run finds nothing left to promote.
        assert_eq!(promote_handler_extents(&mut cfg), 0);
    }

    #[test]
    fn unreachable_handler_entries_stay_unknown() {
        let mut cfg: Cfg<MockInst> = Cfg::new();
        let protected = cfg.new_block();
        let funclet = cfg.new_block();
        cfg.block_mut(cfg.entry()).push(ff("entry"));
        cfg.block_mut(protected).push(ff("try_inst"));
        cfg.block_mut(funclet).push(ff("funclet"));
        cfg.add_edge(cfg.entry(), protected, EdgeKind::Fallthrough);
        cfg.add_region(Region {
            id: RegionId::from_raw(0),
            protected_blocks: [protected].into_iter().collect(),
            handlers: alloc::vec![Handler {
                entry: funclet,
                body: HandlerBody::Unknown,
                kind: HandlerKind::CatchAll,
            }],
            parent: None,
        });

        assert_eq!(promote_handler_extents(&mut cfg), 0);
        assert!(!cfg.regions()[0].handlers[0].body.is_known());
    }
}
