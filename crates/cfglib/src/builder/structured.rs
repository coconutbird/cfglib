//! A frame-stack driver for structured control-flow markers.
//!
//! [`CfgBuilder`](super::CfgBuilder) consumes a whole
//! [`FlowControl`](crate::FlowControl) stream and owns the produced
//! [`Cfg`](crate::Cfg). Lifts that emit into a *checked* builder — an RTL or
//! MLIL function under construction, or any store that can mint blocks and
//! connect them — need the same discipline with the roles reversed: the
//! consumer walks its instruction stream, emits its own statements, and only
//! the structural bookkeeping is generic. [`StructuredWalk`] owns exactly
//! that bookkeeping: the open-block/pending-edge state, the `if`/`else`/
//! `endif` and `loop`/`endloop` frame stack, conditional and unconditional
//! `break`/`continue`, and reason-tagged transfers to caller-owned blocks —
//! reporting every structural misuse ("else outside if", "loop body ends
//! unmaterialized") as a typed [`StructuredWalkIssue`] instead of a
//! hand-rolled invariant comment.
//!
//! The walk never stores blocks it did not receive: blocks come from the
//! caller's [`StructuredSink`], and every edge is wired through it, so the
//! same driver serves any dialect's edge vocabulary via
//! [`StructuredEdge`]-to-payload mapping in the sink.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use core::fmt;
use core::mem;

/// The three edge roles a structured walk wires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StructuredEdge {
    /// The taken path of the condition that just landed.
    True,
    /// The not-taken path of the condition that just landed.
    False,
    /// An unconditional continuation.
    Fallthrough,
}

/// Block minting and edge wiring owned by the caller.
pub trait StructuredSink {
    /// The caller's block identity.
    type Block: Copy;

    /// Mints one new, empty block.
    fn new_block(&mut self) -> Self::Block;

    /// Wires one edge between existing blocks.
    fn connect(&mut self, source: Self::Block, target: Self::Block, edge: StructuredEdge);
}

/// A structural misuse of the marker stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredWalkIssue {
    /// An `else` marker arrived with no open `if` frame.
    ElseOutsideIf,
    /// A second `else` arrived in one `if` frame.
    SecondElse,
    /// An `endif` marker arrived with no open `if` frame.
    EndifOutsideIf,
    /// An `endloop` marker arrived with no open loop frame.
    EndloopOutsideLoop,
    /// A loop body ended with edges still waiting for a block.
    LoopBodyUnmaterialized,
    /// A transfer arrived with no materialized current block.
    TransferOutsideBlock,
    /// A `break` arrived with no enclosing loop frame.
    BreakOutsideLoop,
    /// A `continue` arrived with no enclosing loop frame.
    ContinueOutsideLoop,
    /// The walk finished with open structured frames.
    UnclosedFrames,
}

impl fmt::Display for StructuredWalkIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ElseOutsideIf => "else outside if",
            Self::SecondElse => "second else in one if",
            Self::EndifOutsideIf => "endif outside if",
            Self::EndloopOutsideLoop => "endloop outside loop",
            Self::LoopBodyUnmaterialized => "loop body ends unmaterialized",
            Self::TransferOutsideBlock => "transfer outside a materialized block",
            Self::BreakOutsideLoop => "break outside loop",
            Self::ContinueOutsideLoop => "continue outside loop",
            Self::UnclosedFrames => "unclosed structured region",
        })
    }
}

impl core::error::Error for StructuredWalkIssue {}

enum Frame<B> {
    If {
        /// Where the false edge resumes when no `else` intervenes, or the
        /// pending else entry.
        pending_false: Option<(B, StructuredEdge)>,
        /// Transfers waiting for the merge block.
        pending_merge: Vec<(B, StructuredEdge)>,
        /// Whether the `else` arm was entered.
        saw_else: bool,
    },
    Loop {
        /// The loop header (back-edge target).
        header: B,
        /// Transfers waiting for the loop exit.
        pending_exit: Vec<(B, StructuredEdge)>,
    },
}

/// Frame-stack state for one structured-marker walk.
///
/// Between markers, the caller lands statements via
/// [`landing`](Self::landing) and emits into the returned block; each marker
/// method updates the open-block and pending-edge state so the next landing
/// receives exactly the routes that reach it.
pub struct StructuredWalk<B> {
    /// The open block statements are appended to, if any.
    current: Option<B>,
    /// Edges waiting to enter the next materialized block.
    pending: Vec<(B, StructuredEdge)>,
    frames: Vec<Frame<B>>,
}

impl<B: Copy> StructuredWalk<B> {
    /// Creates a walk whose first landing enters `entry`'s successor chain.
    ///
    /// `entry` is the already-existing block the function begins in; the
    /// first landing materializes a fresh block wired from it, unless the
    /// caller sets it current with [`enter`](Self::enter) first to append
    /// straight into it.
    #[must_use]
    pub fn new(entry: B) -> Self {
        Self {
            current: None,
            pending: vec![(entry, StructuredEdge::Fallthrough)],
            frames: Vec::new(),
        }
    }

    /// Makes `block` the open block without wiring anything.
    pub fn enter(&mut self, block: B) {
        self.current = Some(block);
        self.pending.clear();
    }

    /// The open block, when one is materialized.
    #[must_use]
    pub const fn current(&self) -> Option<B> {
        self.current
    }

    /// Whether edges are waiting for the next materialized block.
    #[must_use]
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// The block statements land in, materializing it if needed.
    ///
    /// A materialized block receives every pending edge; landing with
    /// neither an open block nor pending edges creates an unreachable
    /// block, which the caller's later verification reports.
    pub fn landing(&mut self, sink: &mut impl StructuredSink<Block = B>) -> B {
        if let Some(block) = self.current {
            return block;
        }
        let block = sink.new_block();
        for (source, edge) in self.pending.drain(..) {
            sink.connect(source, block, edge);
        }
        self.current = Some(block);
        block
    }

    /// Opens an `if` after its condition landed in the current block.
    ///
    /// The current block's `True` edge enters the then-arm; its `False`
    /// edge waits for the `else` arm or the merge.
    ///
    /// # Errors
    ///
    /// Returns [`StructuredWalkIssue::TransferOutsideBlock`] when no block
    /// is open.
    pub fn open_if(&mut self) -> Result<(), StructuredWalkIssue> {
        let block = self
            .current
            .take()
            .ok_or(StructuredWalkIssue::TransferOutsideBlock)?;
        self.frames.push(Frame::If {
            pending_false: Some((block, StructuredEdge::False)),
            pending_merge: Vec::new(),
            saw_else: false,
        });
        self.pending = vec![(block, StructuredEdge::True)];
        Ok(())
    }

    /// Ends the then-arm and enters the `else` arm.
    ///
    /// # Errors
    ///
    /// Returns an issue when no `if` frame is open or it already saw an
    /// `else`.
    pub fn open_else(&mut self) -> Result<(), StructuredWalkIssue> {
        match self.frames.last() {
            Some(Frame::If {
                saw_else: false, ..
            }) => {}
            Some(Frame::If { saw_else: true, .. }) => {
                return Err(StructuredWalkIssue::SecondElse);
            }
            _ => return Err(StructuredWalkIssue::ElseOutsideIf),
        }
        let arm_end = self.current.take();
        let stale = mem::take(&mut self.pending);
        let Some(Frame::If {
            pending_false,
            pending_merge,
            saw_else,
        }) = self.frames.last_mut()
        else {
            return Err(StructuredWalkIssue::ElseOutsideIf);
        };
        *saw_else = true;
        if let Some(block) = arm_end {
            pending_merge.push((block, StructuredEdge::Fallthrough));
        } else {
            pending_merge.extend(stale);
        }
        let Some(false_entry) = pending_false.take() else {
            return Err(StructuredWalkIssue::ElseOutsideIf);
        };
        self.pending = vec![false_entry];
        Ok(())
    }

    /// Closes the innermost `if`, routing both arms to the next landing.
    ///
    /// # Errors
    ///
    /// Returns [`StructuredWalkIssue::EndifOutsideIf`] when no `if` frame
    /// is open.
    pub fn close_if(&mut self) -> Result<(), StructuredWalkIssue> {
        if !matches!(self.frames.last(), Some(Frame::If { .. })) {
            return Err(StructuredWalkIssue::EndifOutsideIf);
        }
        let arm_end = self.current.take();
        let stale = mem::take(&mut self.pending);
        let Some(Frame::If {
            pending_false,
            mut pending_merge,
            ..
        }) = self.frames.pop()
        else {
            return Err(StructuredWalkIssue::EndifOutsideIf);
        };
        if let Some(block) = arm_end {
            pending_merge.push((block, StructuredEdge::Fallthrough));
        } else {
            pending_merge.extend(stale);
        }
        if let Some(false_entry) = pending_false {
            pending_merge.push(false_entry);
        }
        self.pending = pending_merge;
        Ok(())
    }

    /// Opens a loop, materializing its header as the back-edge target.
    ///
    /// The preceding block falls through into the header.
    pub fn open_loop(&mut self, sink: &mut impl StructuredSink<Block = B>) -> B {
        if let Some(block) = self.current.take() {
            self.pending.push((block, StructuredEdge::Fallthrough));
        }
        let header = self.landing(sink);
        self.frames.push(Frame::Loop {
            header,
            pending_exit: Vec::new(),
        });
        header
    }

    /// Closes the innermost loop, wiring the latch back edge and routing
    /// every `break` to the next landing.
    ///
    /// # Errors
    ///
    /// Returns an issue when no loop frame is open or the body ended with
    /// edges still waiting for a block.
    pub fn close_loop(
        &mut self,
        sink: &mut impl StructuredSink<Block = B>,
    ) -> Result<(), StructuredWalkIssue> {
        if !matches!(self.frames.last(), Some(Frame::Loop { .. })) {
            return Err(StructuredWalkIssue::EndloopOutsideLoop);
        }
        if !self.pending.is_empty() {
            return Err(StructuredWalkIssue::LoopBodyUnmaterialized);
        }
        let latch = self.current.take();
        let Some(Frame::Loop {
            header,
            pending_exit,
        }) = self.frames.pop()
        else {
            return Err(StructuredWalkIssue::EndloopOutsideLoop);
        };
        if let Some(block) = latch {
            sink.connect(block, header, StructuredEdge::Fallthrough);
        }
        self.pending = pending_exit;
        Ok(())
    }

    /// A `break` out of the innermost loop; `conditional` when the
    /// condition just landed and the `False` path continues in place.
    ///
    /// # Errors
    ///
    /// Returns an issue when no block is open or no loop frame encloses
    /// the walk.
    pub fn break_out(&mut self, conditional: bool) -> Result<(), StructuredWalkIssue> {
        let Some(block) = self.current else {
            return Err(StructuredWalkIssue::TransferOutsideBlock);
        };
        let Some(Frame::Loop { pending_exit, .. }) = self
            .frames
            .iter_mut()
            .rev()
            .find(|frame| matches!(frame, Frame::Loop { .. }))
        else {
            return Err(StructuredWalkIssue::BreakOutsideLoop);
        };
        self.current = None;
        if conditional {
            pending_exit.push((block, StructuredEdge::True));
            self.pending = vec![(block, StructuredEdge::False)];
        } else {
            pending_exit.push((block, StructuredEdge::Fallthrough));
            self.pending = Vec::new();
        }
        Ok(())
    }

    /// A `continue` to the innermost loop header; `conditional` when the
    /// condition just landed and the `False` path continues in place.
    ///
    /// # Errors
    ///
    /// Returns an issue when no block is open or no loop frame encloses
    /// the walk.
    pub fn continue_loop(
        &mut self,
        sink: &mut impl StructuredSink<Block = B>,
        conditional: bool,
    ) -> Result<(), StructuredWalkIssue> {
        let Some(block) = self.current else {
            return Err(StructuredWalkIssue::TransferOutsideBlock);
        };
        let Some(Frame::Loop { header, .. }) = self
            .frames
            .iter()
            .rev()
            .find(|frame| matches!(frame, Frame::Loop { .. }))
        else {
            return Err(StructuredWalkIssue::ContinueOutsideLoop);
        };
        let header = *header;
        let edge = if conditional {
            StructuredEdge::True
        } else {
            StructuredEdge::Fallthrough
        };
        sink.connect(block, header, edge);
        self.current = None;
        self.pending = if conditional {
            vec![(block, StructuredEdge::False)]
        } else {
            Vec::new()
        };
        Ok(())
    }

    /// A conditional transfer to a caller-owned block (a conditional
    /// return's shared exit, an early-out target): the `True` path enters
    /// `target`, the `False` path continues in place.
    ///
    /// # Errors
    ///
    /// Returns [`StructuredWalkIssue::TransferOutsideBlock`] when no block
    /// is open.
    pub fn conditional_transfer(
        &mut self,
        sink: &mut impl StructuredSink<Block = B>,
        target: B,
    ) -> Result<(), StructuredWalkIssue> {
        let Some(block) = self.current else {
            return Err(StructuredWalkIssue::TransferOutsideBlock);
        };
        sink.connect(block, target, StructuredEdge::True);
        self.current = None;
        self.pending = vec![(block, StructuredEdge::False)];
        Ok(())
    }

    /// Ends the current block with no structural successor (a return, an
    /// unreachable terminator). Later statements land in a fresh block
    /// only if a transfer routes there.
    pub fn terminate(&mut self) {
        self.current = None;
        self.pending = Vec::new();
    }

    /// Finishes the walk, reporting frames left open.
    ///
    /// # Errors
    ///
    /// Returns [`StructuredWalkIssue::UnclosedFrames`] when an `if` or
    /// loop frame never closed.
    pub fn finish(self) -> Result<(), StructuredWalkIssue> {
        if self.frames.is_empty() {
            Ok(())
        } else {
            Err(StructuredWalkIssue::UnclosedFrames)
        }
    }
}

#[cfg(test)]
mod tests;
