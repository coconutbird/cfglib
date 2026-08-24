//! Exception handling (EH) modelling.
//!
//! Provides first-class support for EH control flow — landing pads, cleanup
//! blocks, handler/unwind/leave/resume/continue edges, and stable links back to
//! caller-owned edge metadata — enabling accurate runtime-neutral analysis.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::edge::{EdgeId, EdgeKind};
use crate::region::{Cleanup, HandlerKind, HandlerRef};

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

impl From<HandlerKind> for EhBlockKind {
    fn from(kind: HandlerKind) -> Self {
        match kind {
            HandlerKind::Catch | HandlerKind::CatchAll => Self::LandingPad,
            HandlerKind::Finally | HandlerKind::Fault => Self::Cleanup,
            HandlerKind::Filter { .. } => Self::CatchSwitch,
        }
    }
}

/// The exception-control meaning retained for an [`EhEdge`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EhEdgeKind {
    /// Transfer into a selected exception handler.
    Handler,
    /// Stack-unwind transfer to a handler or cleanup.
    Unwind,
    /// Normal transfer out of a protected region.
    Leave,
    /// Continue searching or rethrow the active exception.
    Resume,
    /// Resume execution after handling the exception in-place.
    Continue,
}

impl EhEdgeKind {
    fn from_cfg(kind: EdgeKind) -> Option<Self> {
        match kind {
            EdgeKind::ExceptionHandler => Some(Self::Handler),
            EdgeKind::ExceptionUnwind => Some(Self::Unwind),
            EdgeKind::ExceptionLeave => Some(Self::Leave),
            EdgeKind::ExceptionResume => Some(Self::Resume),
            EdgeKind::ExceptionContinue => Some(Self::Continue),
            _ => None,
        }
    }

    /// Whether this is specifically a stack-unwind transfer.
    #[must_use]
    pub const fn is_unwind(self) -> bool {
        matches!(self, Self::Unwind)
    }
}

/// An exception handling edge annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EhEdge {
    /// Stable identity of the source CFG edge.
    ///
    /// Use it to recover caller-owned payload metadata from the original
    /// [`Cfg`](crate::Cfg), including exception dispositions and platform
    /// records.
    pub edge_id: EdgeId,
    /// Source block (may throw).
    pub from: BlockId,
    /// Target block (handler / cleanup).
    pub to: BlockId,
    /// Precise exception-control transfer kind.
    pub kind: EhEdgeKind,
    /// Compatibility projection of [`Self::kind`].
    ///
    /// This is `true` only for [`EhEdgeKind::Unwind`].
    pub is_unwind: bool,
}

/// EH model for a CFG.
#[derive(Debug, Clone)]
pub struct EhModel {
    /// Classification of each block.
    pub block_kinds: BTreeMap<BlockId, EhBlockKind>,
    /// All exception-control edges, including leave, rethrow, and continue.
    pub eh_edges: Vec<EhEdge>,
    /// Landing pad → set of blocks it protects.
    pub protected_by: BTreeMap<BlockId, BTreeSet<BlockId>>,
    /// Handler entry block → region/handler identities that use that entry.
    ///
    /// The identity provides a lossless route back to
    /// [`HandlerKind`] and consumer-owned
    /// [`HandlerMetadata`](crate::HandlerMetadata).
    pub handlers: BTreeMap<BlockId, Vec<HandlerRef>>,
    /// Cleanup handler entry block → what the cleanup does once its body
    /// ends, for the handlers whose frontend recorded it
    /// ([`Cfg::add_continuation`](crate::Cfg::add_continuation)).
    ///
    /// A `finally` lowered as a single shared block is entered by every route
    /// out of its region and edges to all of their destinations, so the graph
    /// alone cannot say which edge belongs to which route. The record does:
    /// [`Cleanup::resumes_for`] answers "where does control go when this
    /// cleanup was entered by a `return`", and [`Cleanup::resume_from`] names
    /// the block those edges leave (`None` when the cleanup diverges, in
    /// which case its recorded routes are unreachable).
    pub cleanups: BTreeMap<BlockId, Cleanup>,
}

mod build;
pub use build::build_eh_model;

/// Returns all landing pad blocks.
#[must_use]
pub fn landing_pads(model: &EhModel) -> Vec<BlockId> {
    model
        .block_kinds
        .iter()
        .filter(|&(_, k)| *k == EhBlockKind::LandingPad)
        .map(|(&bid, _)| bid)
        .collect()
}

/// Returns all cleanup blocks.
#[must_use]
pub fn cleanup_blocks(model: &EhModel) -> Vec<BlockId> {
    model
        .block_kinds
        .iter()
        .filter(|&(_, k)| *k == EhBlockKind::Cleanup)
        .map(|(&bid, _)| bid)
        .collect()
}

/// Returns blocks that resume, rethrow, or continue an exception.
#[must_use]
pub fn resume_blocks(model: &EhModel) -> Vec<BlockId> {
    model
        .block_kinds
        .iter()
        .filter(|&(_, kind)| *kind == EhBlockKind::Resume)
        .map(|(&block, _)| block)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::edge::EdgeKind;
    use crate::test_util::ff;

    #[test]
    fn handler_kinds_map_to_precise_block_roles() {
        assert_eq!(
            EhBlockKind::from(HandlerKind::Catch),
            EhBlockKind::LandingPad
        );
        assert_eq!(
            EhBlockKind::from(HandlerKind::CatchAll),
            EhBlockKind::LandingPad
        );
        assert_eq!(
            EhBlockKind::from(HandlerKind::Finally),
            EhBlockKind::Cleanup
        );
        assert_eq!(EhBlockKind::from(HandlerKind::Fault), EhBlockKind::Cleanup);
        assert_eq!(
            EhBlockKind::from(HandlerKind::Filter {
                filter_block: BlockId::from_raw(7),
            }),
            EhBlockKind::CatchSwitch
        );
    }

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
    fn cleanup_continuations_reach_the_model_by_entry_block() {
        use crate::region::{
            CompletionReason, Continuation, Handler, HandlerKind, Region, RegionId,
        };

        let mut cfg = Cfg::new();
        let cleanup = cfg.new_block();
        let after = cfg.new_block();
        let exit = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("try"));
        cfg.block_mut(cleanup)
            .instructions_vec_mut()
            .push(ff("finally"));
        let region = cfg.add_region(Region {
            id: RegionId::from_raw(0),
            protected_blocks: [cfg.entry()].into_iter().collect(),
            handlers: alloc::vec![Handler {
                entry: cleanup,
                body: [cleanup].into_iter().collect(),
                kind: HandlerKind::Finally,
            }],
            parent: None,
        });

        // Without records the model is exactly what it always was.
        assert!(build_eh_model(&cfg).cleanups.is_empty());

        let handler = HandlerRef::new(region, 0);
        cfg.set_cleanup_resume(handler, cleanup);
        cfg.add_continuation(
            handler,
            Continuation {
                reason: CompletionReason::Normal,
                resume: after,
            },
        );
        cfg.add_continuation(
            handler,
            Continuation {
                reason: CompletionReason::Return,
                resume: exit,
            },
        );
        // Both routes leave the same block, so the edges alone are opaque.
        cfg.add_edge(cleanup, after, EdgeKind::Fallthrough);
        cfg.add_edge(cleanup, exit, EdgeKind::Fallthrough);

        let model = build_eh_model(&cfg);
        assert_eq!(model.block_kinds[&cleanup], EhBlockKind::Cleanup);
        let recorded = &model.cleanups[&cleanup];
        assert_eq!(recorded.handler, handler);
        assert_eq!(recorded.resume_from, Some(cleanup));
        assert_eq!(
            recorded
                .resumes_for(CompletionReason::Return)
                .collect::<alloc::vec::Vec<_>>(),
            alloc::vec![exit],
            "the reason selects the route the shared edges cannot"
        );
        assert_eq!(
            recorded
                .resumes_for(CompletionReason::Normal)
                .collect::<alloc::vec::Vec<_>>(),
            alloc::vec![after]
        );
        assert!(
            recorded
                .resumes_for(CompletionReason::Transfer)
                .next()
                .is_none()
        );
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

    #[test]
    fn payload_cfg_retains_every_exception_transfer_and_edge_identity() {
        use crate::exception::{ExceptionDisposition, ExceptionFlow, ExceptionPhase};

        let mut cfg = Cfg::<(), ExceptionFlow<u32>>::new_with_edge_payload();
        let handler = cfg.new_block();
        let leave = cfg.new_block();
        let rethrow = cfg.new_block();
        let outer = cfg.new_block();
        let continue_decision = cfg.new_block();
        let resume_target = cfg.new_block();
        let entry = cfg.entry();

        let handler_edge = cfg.add_edge_with_payload(
            entry,
            handler,
            EdgeKind::ExceptionHandler,
            ExceptionFlow::exceptional(
                ExceptionPhase::Unwind,
                Some(ExceptionDisposition::ExecuteHandler),
                11,
            ),
        );
        cfg.add_edge_with_payload(
            entry,
            leave,
            EdgeKind::ExceptionLeave,
            ExceptionFlow::normal(12),
        );
        cfg.add_edge_with_payload(
            rethrow,
            outer,
            EdgeKind::ExceptionResume,
            ExceptionFlow::exceptional(
                ExceptionPhase::Search,
                Some(ExceptionDisposition::ContinueSearch),
                13,
            ),
        );
        cfg.add_edge_with_payload(
            continue_decision,
            resume_target,
            EdgeKind::ExceptionContinue,
            ExceptionFlow::exceptional(
                ExceptionPhase::Search,
                Some(ExceptionDisposition::ContinueExecution),
                14,
            ),
        );

        let model = build_eh_model(&cfg);
        assert_eq!(
            model
                .eh_edges
                .iter()
                .map(|edge| edge.kind)
                .collect::<alloc::vec::Vec<_>>(),
            alloc::vec![
                EhEdgeKind::Handler,
                EhEdgeKind::Leave,
                EhEdgeKind::Resume,
                EhEdgeKind::Continue,
            ]
        );
        assert_eq!(model.eh_edges[0].edge_id, handler_edge);
        assert_eq!(cfg[model.eh_edges[0].edge_id].payload().metadata(), &11);
        assert_eq!(
            resume_blocks(&model),
            alloc::vec![rethrow, continue_decision]
        );
    }

    #[test]
    fn explicit_cleanup_region_overrides_incoming_unwind_inference() {
        use crate::region::{Handler, HandlerKind, Region, RegionId};

        let mut cfg = Cfg::<()>::new();
        let cleanup = cfg.new_block();
        let entry = cfg.entry();
        cfg.add_edge(entry, cleanup, EdgeKind::ExceptionUnwind);
        let region = cfg.add_region(Region {
            id: RegionId::from_raw(0),
            protected_blocks: [entry].into_iter().collect(),
            handlers: alloc::vec![Handler {
                entry: cleanup,
                body: [cleanup].into_iter().collect(),
                kind: HandlerKind::Finally,
            }],
            parent: None,
        });

        let model = build_eh_model(&cfg);
        assert_eq!(model.block_kinds[&cleanup], EhBlockKind::Cleanup);
        assert_eq!(
            model.handlers[&cleanup],
            alloc::vec![HandlerRef::new(region, 0)]
        );
    }
}
