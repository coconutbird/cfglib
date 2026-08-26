//! Exception-region structuring for the AST lifter.

extern crate alloc;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use super::super::node::{AstNode, CatchHandler};
use super::{LiftState, effective_merge, lift_block, lift_region};
use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::region::{HandlerKind, HandlerRef, Region};

/// The blocks of `inner` also admitted by the enclosing bound, so a nested
/// region never escapes the extent it was entered under.
fn intersect_bounds(
    outer: Option<&BTreeSet<BlockId>>,
    inner: &BTreeSet<BlockId>,
) -> BTreeSet<BlockId> {
    match outer {
        None => inner.clone(),
        Some(outer) => inner.intersection(outer).copied().collect(),
    }
}

/// Group regions by anchor — their first protected block in reverse
/// postorder, i.e. the region's entry in flow order rather than by block id.
pub(super) fn region_anchors<'c, I, E>(
    cfg: &'c Cfg<I, E>,
    order: &[BlockId],
) -> BTreeMap<u32, Vec<&'c Region>> {
    let mut position: BTreeMap<u32, usize> = BTreeMap::new();
    for (index, block) in order.iter().enumerate() {
        position.insert(block.0, index);
    }
    let mut anchors: BTreeMap<u32, Vec<&Region>> = BTreeMap::new();
    for region in cfg.regions() {
        let anchor = region
            .protected_blocks
            .iter()
            .min_by_key(|block| position.get(&block.0).copied().unwrap_or(usize::MAX))
            .copied();
        if let Some(anchor) = anchor {
            anchors.entry(anchor.0).or_default().push(region);
        }
    }
    for candidates in anchors.values_mut() {
        candidates.sort_by_key(|region| core::cmp::Reverse(region.protected_blocks.len()));
    }
    anchors
}

/// Handler entries and filter funclets: sweep roots that can carry code
/// without any incoming CFG edge.
pub(super) fn metadata_roots<I, E>(cfg: &Cfg<I, E>) -> Vec<BlockId> {
    cfg.regions()
        .iter()
        .flat_map(|region| region.handlers.iter())
        .flat_map(|handler| {
            let filter = match handler.kind {
                HandlerKind::Filter { filter_block } => Some(filter_block),
                _ => None,
            };
            core::iter::once(handler.entry).chain(filter)
        })
        .collect()
}

/// The complete handler extents of `region`, or `None` when any handler's
/// extent is unknown and the region must stay unstructured.
///
/// A [`HandlerBody::Known`](crate::region::HandlerBody::Known) that omits
/// its own entry is a frontend bug, distinct from a deliberate `Unknown`:
/// it debug-panics, and degrades to unstructured flow in release builds.
fn complete_handler_bodies(region: &Region) -> Option<Vec<&BTreeSet<BlockId>>> {
    region
        .handlers
        .iter()
        .map(|handler| match handler.body.blocks() {
            Some(blocks) if blocks.contains(&handler.entry) => Some(blocks),
            Some(_) => {
                debug_assert!(
                    false,
                    "HandlerBody::Known must contain its own handler entry"
                );
                None
            }
            None => None,
        })
        .collect()
}

/// Lift a try/catch region anchored at `block`, if a structurable one exists.
///
/// Among the regions anchored at `block` (outermost first), the first with
/// complete handler extents is structured; regions with unknown extents
/// remain ordinary control flow. Bounds intersect with the enclosing
/// `allowed_blocks`, so a nested region never escapes the extent it was
/// entered under.
pub(super) fn lift_try_catch<I: Clone, E>(
    cfg: &Cfg<I, E>,
    state: &mut LiftState<'_>,
    block: BlockId,
    allowed_blocks: Option<&BTreeSet<BlockId>>,
) -> Option<AstNode<I>> {
    let candidates = state.anchors.get(&block.0)?;
    let (region, handler_bodies) = candidates
        .iter()
        .find_map(|region| complete_handler_bodies(region).map(|bodies| (*region, bodies)))?;
    state.structured_regions.insert(region.id.0);

    // The region's normal continuation: the anchor's in-bound post-dominator
    // when it lies outside the region and its handlers. Inner walks stop
    // there silently — the region's `leave` is implicit, not a goto.
    let continuation = effective_merge(cfg, state, block, allowed_blocks).filter(|merge| {
        !region.protected_blocks.contains(merge)
            && region.handlers.iter().all(|handler| {
                handler.entry != *merge
                    && handler
                        .body
                        .blocks()
                        .is_none_or(|blocks| !blocks.contains(merge))
            })
    });

    // Lift the try body through the shared classification path, so an
    // anchor that is itself a branch, dispatch, or loop header keeps its
    // structure. `lift_block` never consults the anchor map, so this
    // cannot re-trigger the region check and recurse.
    let try_bound = intersect_bounds(allowed_blocks, &region.protected_blocks);
    let mut try_body = Vec::new();
    let next = lift_block(cfg, state, block, Some(&try_bound), &mut try_body);
    if let Some(next) = next {
        if !state.visited.contains(&next.0) {
            try_body.extend(lift_region(
                cfg,
                state,
                next,
                Some(&try_bound),
                continuation,
            ));
        }
    }
    // Protected successors the classified walk did not reach (for example
    // the targets of a multi-successor linear anchor) still belong to the
    // try body.
    for succ in cfg.successors(block) {
        if region.protected_blocks.contains(&succ) && !state.visited.contains(&succ.0) {
            try_body.extend(lift_region(
                cfg,
                state,
                succ,
                Some(&try_bound),
                continuation,
            ));
        }
    }

    let mut handlers = Vec::new();
    let mut finally_body = Vec::new();

    for ((index, handler), handler_blocks) in region.handlers.iter().enumerate().zip(handler_bodies)
    {
        let handler_bound = intersect_bounds(allowed_blocks, handler_blocks);
        let body = lift_region(
            cfg,
            state,
            handler.entry,
            Some(&handler_bound),
            continuation,
        );
        match handler.kind {
            // Multiple `Finally` handlers concatenate in declaration order.
            HandlerKind::Finally => finally_body.extend(body),
            _ => {
                handlers.push(CatchHandler {
                    handler: HandlerRef::new(region.id, index),
                    entry: handler.entry,
                    kind: handler.kind,
                    body,
                });
            }
        }
    }

    Some(AstNode::TryCatch {
        try_body,
        handlers,
        finally_body,
    })
}
