//! AST lifting — reconstruct structured control flow from a [`Cfg`](crate::Cfg).
//!
//! Takes a flat control-flow graph and produces a tree of [`AstNode`]s
//! representing `if/else`, loops, `switch`, and linear sequences.
//! This is essentially the core of what a decompiler does.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::edge::EdgeId;
use crate::region::{HandlerKind, HandlerRef};

/// A node in the reconstructed AST.
///
/// Generic over the instruction type `I`, matching [`Cfg<I>`](crate::Cfg).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AstNode<I> {
    /// A basic block — leaf node containing the original instructions.
    Block {
        /// The block this came from in the CFG.
        id: BlockId,
        /// The instructions in this block.
        instructions: Vec<I>,
    },

    /// A linear sequence of statements executed one after another.
    Sequence {
        /// The ordered list of child nodes.
        body: Vec<AstNode<I>>,
    },

    /// A conditional branch (`if / else`).
    IfThenElse {
        /// The block containing the condition (last instruction is the branch).
        condition: BlockId,
        /// Instructions in the condition block.
        condition_instructions: Vec<I>,
        /// The "true" arm.
        then_body: Vec<AstNode<I>>,
        /// The "false" arm (empty if there's no `else`).
        else_body: Vec<AstNode<I>>,
    },

    /// A loop, classified as pre-tested, post-tested, or endless.
    Loop {
        /// The loop header block in the CFG.
        header: BlockId,
        /// The recognized loop shape and its condition witness.
        kind: LoopKind<I>,
        /// The body of the loop.
        body: Vec<AstNode<I>>,
    },

    /// A multi-way branch (`switch / case`).
    Switch {
        /// The block containing the switch dispatch.
        condition: BlockId,
        /// Instructions in the dispatch block.
        condition_instructions: Vec<I>,
        /// The individual case arms.
        cases: Vec<SwitchCase<I>>,
        /// The arm taken when no case matches (empty when the default
        /// transfers straight to the switch continuation).
        default_body: Vec<AstNode<I>>,
        /// The dispatch edge taken when no case matches, when one exists.
        /// Case-key metadata lives on the caller's edge payloads.
        default_edge: Option<EdgeId>,
    },

    /// Break out of a loop or switch.
    Break {
        /// Enclosing loop label for a multi-level break; `None` breaks the
        /// innermost loop or switch.
        label: Option<String>,
    },

    /// Continue to a loop's continue point (its header, or a post-tested
    /// loop's condition).
    Continue {
        /// Enclosing loop label for a multi-level continue; `None` continues
        /// the innermost loop.
        label: Option<String>,
    },

    /// Return / terminate.
    Return {
        /// The block this came from in the CFG.
        id: BlockId,
        /// Instructions in the return block (includes the return itself).
        instructions: Vec<I>,
    },

    /// A label target (used for irreducible control flow and labeled loops).
    Label {
        /// The label name.
        name: String,
        /// Body following the label.
        body: Vec<AstNode<I>>,
    },

    /// An unconditional goto (used for irreducible control flow).
    Goto {
        /// Target label name.
        target: String,
    },

    /// A try region with catch, filter, fault, and/or finally handlers.
    TryCatch {
        /// The protected body (try block).
        try_body: Vec<AstNode<I>>,
        /// Handler arms.
        handlers: Vec<CatchHandler<I>>,
        /// Finally body (empty if no finally).
        finally_body: Vec<AstNode<I>>,
    },

    /// A predicated/guarded region — executes only when a condition
    /// holds (ARM IT blocks, GPU wave predication, CMOV).
    Guarded {
        /// A witness instruction carrying the guard; its
        /// [`Predicated::predicate`](crate::Predicated::predicate) names the
        /// condition variable, and its rendering labels the region.
        predicate: I,
        /// Whether the body executes when the predicate is *true*.
        when_true: bool,
        /// The guarded body.
        body: Vec<AstNode<I>>,
    },
}

/// The recognized control shape of one [`AstNode::Loop`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopKind<I> {
    /// No recognized pre- or post-test; iteration ends only through an
    /// inner `Break`, `Return`, or `Goto`.
    Endless,

    /// Pre-tested loop (`while`): the header evaluates `condition` before
    /// every iteration and exits along one conditional arm.
    While {
        /// The header's instructions, ending in the conditional branch.
        condition: Vec<I>,
        /// Whether the `ConditionalTrue` edge leaves the loop (a source
        /// `while (!c)` shape); `false` means the true edge iterates.
        exit_on_true: bool,
    },

    /// Post-tested loop (`do/while`): the latch evaluates `condition` after
    /// every iteration and either returns to the header or exits.
    DoWhile {
        /// The latch block evaluating the condition. For a single-block
        /// loop this is the header itself and the body is empty.
        latch: BlockId,
        /// The latch's instructions, ending in the conditional branch.
        condition: Vec<I>,
        /// Whether the `ConditionalTrue` edge returns to the header.
        continue_on_true: bool,
    },
}

/// A single non-finally handler arm inside an [`AstNode::TryCatch`].
///
/// The historical name is retained for compatibility; [`Self::kind`]
/// distinguishes catch, catch-all, fault, and filter arms. Consumer handler
/// metadata (catch types, filter predicates, ordering) stays in
/// consumer-keyed side tables such as
/// [`HandlerMetadata`](crate::HandlerMetadata), addressed by
/// [`Self::handler`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatchHandler<I> {
    /// Stable identity of the source CFG handler.
    pub handler: HandlerRef,
    /// The entry block of the handler.
    pub entry: BlockId,
    /// Exact catch, catch-all, fault, or filter classification.
    pub kind: HandlerKind,
    /// The body of the handler.
    pub body: Vec<AstNode<I>>,
}

/// A single case arm inside a [`AstNode::Switch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchCase<I> {
    /// The case block ID from the CFG.
    pub id: BlockId,
    /// The dispatch edges selecting this arm, in dispatch order. Case-key
    /// metadata lives on the caller's edge payloads, so a consumer recovers
    /// exact keys through [`Cfg::edge`](crate::Cfg::edge).
    pub edges: Vec<EdgeId>,
    /// The complete body of this case arm, beginning with the case block's
    /// own instructions.
    pub body: Vec<AstNode<I>>,
}

impl<I> AstNode<I> {
    /// Returns `true` if this is an empty sequence.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        matches!(self, AstNode::Sequence { body } if body.is_empty())
    }

    /// Flatten nested single-element sequences.
    #[must_use]
    pub fn simplify(self) -> Self {
        match self {
            AstNode::Sequence { mut body } => {
                body = body.into_iter().map(AstNode::simplify).collect();
                if body.len() == 1 {
                    body.pop().unwrap_or(AstNode::Sequence { body: Vec::new() })
                } else {
                    AstNode::Sequence { body }
                }
            }
            AstNode::IfThenElse {
                condition,
                condition_instructions,
                then_body,
                else_body,
            } => AstNode::IfThenElse {
                condition,
                condition_instructions,
                then_body: simplify_nodes(then_body),
                else_body: simplify_nodes(else_body),
            },
            AstNode::Loop { header, kind, body } => AstNode::Loop {
                header,
                kind,
                body: simplify_nodes(body),
            },
            AstNode::Switch {
                condition,
                condition_instructions,
                cases,
                default_body,
                default_edge,
            } => AstNode::Switch {
                condition,
                condition_instructions,
                cases: cases
                    .into_iter()
                    .map(|case| SwitchCase {
                        id: case.id,
                        edges: case.edges,
                        body: simplify_nodes(case.body),
                    })
                    .collect(),
                default_body: simplify_nodes(default_body),
                default_edge,
            },
            AstNode::Label { name, body } => AstNode::Label {
                name,
                body: simplify_nodes(body),
            },
            AstNode::TryCatch {
                try_body,
                handlers,
                finally_body,
            } => AstNode::TryCatch {
                try_body: simplify_nodes(try_body),
                handlers: handlers
                    .into_iter()
                    .map(|handler| CatchHandler {
                        handler: handler.handler,
                        entry: handler.entry,
                        kind: handler.kind,
                        body: simplify_nodes(handler.body),
                    })
                    .collect(),
                finally_body: simplify_nodes(finally_body),
            },
            AstNode::Guarded {
                predicate,
                when_true,
                body,
            } => AstNode::Guarded {
                predicate,
                when_true,
                body: simplify_nodes(body),
            },
            other => other,
        }
    }
}

fn simplify_nodes<I>(nodes: Vec<AstNode<I>>) -> Vec<AstNode<I>> {
    nodes.into_iter().map(AstNode::simplify).collect()
}
