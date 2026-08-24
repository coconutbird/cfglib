//! Normalized CLR exception clauses.

extern crate alloc;

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::region::{Handler, HandlerKind, HandlerRef, HandlerTypes, Region, RegionId};

/// CLR handler-clause kind with a consumer-owned caught-type identity.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ClrHandlerKind<T> {
    /// A typed catch clause.
    Catch {
        /// Metadata token or other frontend identity of the caught type.
        ty: T,
    },
    /// A catch-all clause without a type identity.
    CatchAll,
    /// A `finally` clause.
    Finally,
    /// A CLR `fault` clause.
    Fault,
    /// A filter clause whose predicate has its own CFG block.
    Filter {
        /// Entry block of the filter predicate.
        filter_block: BlockId,
    },
}

/// One normalized CLR exception handler.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ClrHandler<T> {
    /// Entry block of the handler body.
    pub entry: BlockId,
    /// Every block belonging to the handler body.
    pub body: BTreeSet<BlockId>,
    /// CLR clause kind.
    pub kind: ClrHandlerKind<T>,
}

/// One normalized CLR protected region and its clauses.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ClrExceptionRegion<T> {
    /// Blocks covered by the CLR try range.
    pub protected_blocks: BTreeSet<BlockId>,
    /// Handler clauses in metadata order.
    pub handlers: Vec<ClrHandler<T>>,
    /// Enclosing region, imported before this one, when nested.
    pub parent: Option<RegionId>,
}

/// Install a normalized CLR region into `cfg`.
///
/// Typed catches become [`HandlerKind::Catch`], with their consumer-owned type
/// identities placed in `handler_types`. Filter, finally, and fault clauses
/// retain their native classifications in the resulting [`Region`].
pub fn install_clr_region<I, E, T>(
    cfg: &mut Cfg<I, E>,
    handler_types: &mut HandlerTypes<T>,
    region: ClrExceptionRegion<T>,
) -> RegionId {
    let mut typed_handlers = Vec::new();
    let mut handlers = Vec::with_capacity(region.handlers.len());

    for (index, handler) in region.handlers.into_iter().enumerate() {
        let kind = match handler.kind {
            ClrHandlerKind::Catch { ty } => {
                typed_handlers.push((index, ty));
                HandlerKind::Catch
            }
            ClrHandlerKind::CatchAll => HandlerKind::CatchAll,
            ClrHandlerKind::Finally => HandlerKind::Finally,
            ClrHandlerKind::Fault => HandlerKind::Fault,
            ClrHandlerKind::Filter { filter_block } => HandlerKind::Filter { filter_block },
        };
        handlers.push(Handler {
            entry: handler.entry,
            body: handler.body,
            kind,
        });
    }

    let region_id = cfg.add_region(Region {
        id: RegionId::from_raw(0),
        protected_blocks: region.protected_blocks,
        handlers,
        parent: region.parent,
    });

    for (index, ty) in typed_handlers {
        let replaced = handler_types.set(HandlerRef::new(region_id, index), ty);
        debug_assert!(
            replaced.is_none(),
            "new CLR handler unexpectedly had metadata"
        );
    }

    region_id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clr_clauses_keep_fault_filter_and_catch_type() {
        let mut cfg = Cfg::<()>::new();
        let catch = cfg.new_block();
        let fault = cfg.new_block();
        let filter = cfg.new_block();
        let filtered_handler = cfg.new_block();
        let entry = cfg.entry();
        let mut types = HandlerTypes::new();

        let region = install_clr_region(
            &mut cfg,
            &mut types,
            ClrExceptionRegion {
                protected_blocks: [entry].into_iter().collect(),
                handlers: alloc::vec![
                    ClrHandler {
                        entry: catch,
                        body: [catch].into_iter().collect(),
                        kind: ClrHandlerKind::Catch { ty: 0x0200_0001 },
                    },
                    ClrHandler {
                        entry: fault,
                        body: [fault].into_iter().collect(),
                        kind: ClrHandlerKind::Fault,
                    },
                    ClrHandler {
                        entry: filtered_handler,
                        body: [filtered_handler].into_iter().collect(),
                        kind: ClrHandlerKind::Filter {
                            filter_block: filter,
                        },
                    },
                ],
                parent: None,
            },
        );

        assert_eq!(types.get(HandlerRef::new(region, 0)), Some(&0x0200_0001));
        assert_eq!(cfg.regions()[0].handlers[1].kind, HandlerKind::Fault);
        assert_eq!(
            cfg.regions()[0].handlers[2].kind,
            HandlerKind::Filter {
                filter_block: filter
            }
        );
    }
}
