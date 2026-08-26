//! Indented pseudocode rendering for lifted ASTs.

extern crate alloc;
use alloc::string::String;
use core::fmt::{self, Write as _};

use crate::display::DisplayInstr;
use crate::region::HandlerKind;

use super::node::{AstNode, CatchHandler, LoopKind, SwitchCase};

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

fn write_loop<I: DisplayInstr>(
    out: &mut String,
    kind: &LoopKind<I>,
    body: &[AstNode<I>],
    depth: usize,
) {
    match kind {
        LoopKind::Endless => {
            write_indent(out, depth);
            out.push_str("loop {\n");
            write_nodes(out, body, depth + 1);
            write_indent(out, depth);
            out.push_str("}\n");
        }
        LoopKind::While {
            condition,
            exit_on_true,
        } => {
            write_indent(out, depth);
            out.push_str("while {\n");
            write_insts(out, condition, depth + 1);
            write_indent(out, depth + 1);
            out.push_str(if *exit_on_true {
                "break if cond;\n"
            } else {
                "break if !cond;\n"
            });
            write_nodes(out, body, depth + 1);
            write_indent(out, depth);
            out.push_str("}\n");
        }
        LoopKind::DoWhile {
            condition,
            continue_on_true,
            ..
        } => {
            write_indent(out, depth);
            out.push_str("do {\n");
            write_nodes(out, body, depth + 1);
            write_insts(out, condition, depth + 1);
            write_indent(out, depth);
            out.push_str(if *continue_on_true {
                "} while (cond)\n"
            } else {
                "} while (!cond)\n"
            });
        }
    }
}

fn write_switch<I: DisplayInstr>(
    out: &mut String,
    condition_instructions: &[I],
    cases: &[SwitchCase<I>],
    default_body: &[AstNode<I>],
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
        write_nodes(out, &case.body, depth + 2);
        write_indent(out, depth);
        out.push_str("  }\n");
    }
    if !default_body.is_empty() {
        write_indent(out, depth);
        out.push_str("  default {\n");
        write_nodes(out, default_body, depth + 2);
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

fn write_transfer(out: &mut String, keyword: &str, label: Option<&String>, depth: usize) {
    write_indent(out, depth);
    out.push_str(keyword);
    if let Some(label) = label {
        out.push(' ');
        out.push_str(label);
    }
    out.push_str(";\n");
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
        AstNode::Loop { kind, body, .. } => write_loop(out, kind, body, depth),
        AstNode::Switch {
            condition_instructions,
            cases,
            default_body,
            ..
        } => write_switch(out, condition_instructions, cases, default_body, depth),
        AstNode::Break { label } => write_transfer(out, "break", label.as_ref(), depth),
        AstNode::Continue { label } => write_transfer(out, "continue", label.as_ref(), depth),
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
