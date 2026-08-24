//! AST lifting — reconstruct structured control flow from a [`Cfg`](crate::Cfg).
//!
//! Takes a flat control-flow graph and produces a tree of [`AstNode`]s
//! representing `if/else`, `loop`, `switch`, and linear sequences.
//! This is essentially the core of what a decompiler does.

extern crate alloc;
use alloc::vec::Vec;

use crate::block::BlockId;
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

    /// A loop (`while` / `do-while`).
    Loop {
        /// The loop header block in the CFG.
        header: BlockId,
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
    },

    /// Unconditional break out of the innermost loop/switch.
    Break,

    /// Continue to the loop header.
    Continue,

    /// Return / terminate.
    Return {
        /// The block this came from in the CFG.
        id: BlockId,
        /// Instructions in the return block (includes the return itself).
        instructions: Vec<I>,
    },

    // ── Unstructured / CPU-ISA nodes ────────────────────────────────
    /// A label target (used after irreducible CFG lowering).
    Label {
        /// The label name.
        name: alloc::string::String,
        /// Body following the label.
        body: Vec<AstNode<I>>,
    },

    /// An unconditional goto (used for irreducible control flow).
    Goto {
        /// Target label name.
        target: alloc::string::String,
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

/// A single non-finally handler arm inside an [`AstNode::TryCatch`].
///
/// The historical name is retained for compatibility; [`Self::kind`]
/// distinguishes catch, catch-all, fault, and filter arms.
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
    /// Instructions at the start of this case (e.g. the `case` opcode).
    pub header_instructions: Vec<I>,
    /// The body of this case arm.
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
                // Unwrap single-element sequences.
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
                then_body: then_body.into_iter().map(AstNode::simplify).collect(),
                else_body: else_body.into_iter().map(AstNode::simplify).collect(),
            },
            AstNode::Loop { header, body } => AstNode::Loop {
                header,
                body: body.into_iter().map(AstNode::simplify).collect(),
            },
            AstNode::Switch {
                condition,
                condition_instructions,
                cases,
            } => AstNode::Switch {
                condition,
                condition_instructions,
                cases: cases
                    .into_iter()
                    .map(|c| SwitchCase {
                        id: c.id,
                        header_instructions: c.header_instructions,
                        body: c.body.into_iter().map(AstNode::simplify).collect(),
                    })
                    .collect(),
            },
            AstNode::Label { name, body } => AstNode::Label {
                name,
                body: body.into_iter().map(AstNode::simplify).collect(),
            },
            AstNode::TryCatch {
                try_body,
                handlers,
                finally_body,
            } => AstNode::TryCatch {
                try_body: try_body.into_iter().map(AstNode::simplify).collect(),
                handlers: handlers
                    .into_iter()
                    .map(|h| CatchHandler {
                        handler: h.handler,
                        entry: h.entry,
                        kind: h.kind,
                        body: h.body.into_iter().map(AstNode::simplify).collect(),
                    })
                    .collect(),
                finally_body: finally_body.into_iter().map(AstNode::simplify).collect(),
            },
            AstNode::Guarded {
                predicate,
                when_true,
                body,
            } => AstNode::Guarded {
                predicate,
                when_true,
                body: body.into_iter().map(AstNode::simplify).collect(),
            },
            other => other,
        }
    }
}

use alloc::string::String;
use core::fmt::{self, Write as _};

use crate::display::DisplayInstr;

impl<I: DisplayInstr> AstNode<I> {
    /// Render this AST as indented pseudocode.
    #[must_use]
    pub fn to_pseudocode(&self) -> String {
        let mut out = String::new();
        write_node(&mut out, self, 0);
        out
    }
}

fn write_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("    ");
    }
}

fn write_insts<I: DisplayInstr>(out: &mut String, insts: &[I], depth: usize) {
    for inst in insts {
        let m = inst.mnemonic();
        if !m.is_empty() {
            write_indent(out, depth);
            out.push_str(&m);
            out.push('\n');
        }
    }
}

fn write_nodes<I: DisplayInstr>(out: &mut String, nodes: &[AstNode<I>], depth: usize) {
    for node in nodes {
        write_node(out, node, depth);
    }
}

fn write_if_then_else<I: DisplayInstr>(
    out: &mut String,
    condition_instructions: &[I],
    then_body: &[AstNode<I>],
    else_body: &[AstNode<I>],
    depth: usize,
) {
    if condition_instructions.len() > 1 {
        write_insts(
            out,
            &condition_instructions[..condition_instructions.len() - 1],
            depth,
        );
    }
    write_indent(out, depth);
    out.push_str("if {\n");
    write_nodes(out, then_body, depth + 1);
    if !else_body.is_empty() {
        write_indent(out, depth);
        out.push_str("} else {\n");
        write_nodes(out, else_body, depth + 1);
    }
    write_indent(out, depth);
    out.push_str("}\n");
}

fn write_switch<I: DisplayInstr>(
    out: &mut String,
    condition_instructions: &[I],
    cases: &[SwitchCase<I>],
    depth: usize,
) {
    if condition_instructions.len() > 1 {
        write_insts(
            out,
            &condition_instructions[..condition_instructions.len() - 1],
            depth,
        );
    }
    write_indent(out, depth);
    out.push_str("switch {\n");
    for case in cases {
        write_indent(out, depth);
        out.push_str("  case {\n");
        write_insts(out, &case.header_instructions, depth + 2);
        write_nodes(out, &case.body, depth + 2);
        write_indent(out, depth);
        out.push_str("  }\n");
    }
    write_indent(out, depth);
    out.push_str("}\n");
}

fn write_try_catch<I: DisplayInstr>(
    out: &mut String,
    try_body: &[AstNode<I>],
    handlers: &[CatchHandler<I>],
    finally_body: &[AstNode<I>],
    depth: usize,
) {
    write_indent(out, depth);
    out.push_str("try {\n");
    write_nodes(out, try_body, depth + 1);
    for handler in handlers {
        write_indent(out, depth);
        match handler.kind {
            HandlerKind::Catch => out.push_str("} catch {\n"),
            HandlerKind::CatchAll => out.push_str("} catch (...) {\n"),
            HandlerKind::Finally => out.push_str("} finally {\n"),
            HandlerKind::Fault => out.push_str("} fault {\n"),
            HandlerKind::Filter { filter_block } => {
                out.push_str("} filter (");
                write!(out, ".bb{}", filter_block.index())
                    .expect("writing to a String cannot fail");
                out.push_str(") {\n");
            }
        }
        write_nodes(out, &handler.body, depth + 1);
    }
    if !finally_body.is_empty() {
        write_indent(out, depth);
        out.push_str("} finally {\n");
        write_nodes(out, finally_body, depth + 1);
    }
    write_indent(out, depth);
    out.push_str("}\n");
}

fn write_node<I: DisplayInstr>(out: &mut String, node: &AstNode<I>, depth: usize) {
    match node {
        AstNode::Block { instructions, .. } | AstNode::Return { instructions, .. } => {
            write_insts(out, instructions, depth);
        }
        AstNode::Sequence { body } => {
            write_nodes(out, body, depth);
        }
        AstNode::IfThenElse {
            condition_instructions,
            then_body,
            else_body,
            ..
        } => write_if_then_else(out, condition_instructions, then_body, else_body, depth),
        AstNode::Loop { body, .. } => {
            write_indent(out, depth);
            out.push_str("loop {\n");
            write_nodes(out, body, depth + 1);
            write_indent(out, depth);
            out.push_str("}\n");
        }
        AstNode::Switch {
            condition_instructions,
            cases,
            ..
        } => write_switch(out, condition_instructions, cases, depth),
        AstNode::Break => {
            write_indent(out, depth);
            out.push_str("break;\n");
        }
        AstNode::Continue => {
            write_indent(out, depth);
            out.push_str("continue;\n");
        }
        AstNode::Label { name, body } => {
            write_indent(out, depth);
            out.push_str(name);
            out.push_str(":\n");
            write_nodes(out, body, depth + 1);
        }
        AstNode::Goto { target } => {
            write_indent(out, depth);
            out.push_str("goto ");
            out.push_str(target);
            out.push_str(";\n");
        }
        AstNode::TryCatch {
            try_body,
            handlers,
            finally_body,
        } => write_try_catch(out, try_body, handlers, finally_body, depth),
        AstNode::Guarded {
            predicate,
            when_true,
            body,
        } => {
            write_indent(out, depth);
            out.push_str("@guarded(");
            if !when_true {
                out.push('!');
            }
            out.push_str(&predicate.mnemonic());
            out.push_str(") {\n");
            write_nodes(out, body, depth + 1);
            write_indent(out, depth);
            out.push_str("}\n");
        }
    }
}

impl<I: DisplayInstr> fmt::Display for AstNode<I> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_pseudocode())
    }
}
