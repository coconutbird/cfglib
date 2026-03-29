//! DOT (Graphviz) export for control-flow graphs.

extern crate alloc;
use alloc::string::String;
use core::fmt;

use crate::cfg::Cfg;
use crate::edge::EdgeKind;
use crate::flow::FlowControl;

impl<I: FlowControl> Cfg<I> {
    /// Write the CFG in DOT format to any `fmt::Write` sink.
    pub fn write_dot(&self, w: &mut dyn fmt::Write) -> fmt::Result {
        writeln!(w, "digraph cfg {{")?;
        writeln!(
            w,
            "    node [shape=box fontname=\"monospace\" fontsize=10];"
        )?;
        writeln!(w, "    edge [fontname=\"monospace\" fontsize=9];")?;

        for block in &self.blocks {
            let id = block.id();
            let label_prefix = block
                .label()
                .map(|l| alloc::format!("{l}:\\n"))
                .unwrap_or_default();

            // Build a label with the mnemonic of each instruction.
            let mut body = String::new();
            for inst in block.instructions() {
                let m = inst.display_mnemonic();
                if !m.is_empty() {
                    if !body.is_empty() {
                        body.push_str("\\l");
                    }
                    body.push_str(&m);
                }
            }

            if body.is_empty() {
                body.push_str("(empty)");
            }

            body.push_str("\\l");

            writeln!(w, "    {id} [label=\"{label_prefix}{id}\\n{body}\"];",)?;
        }

        for edge in &self.edges {
            // Skip edges removed via remove_edge() — they are still
            // in the arena but no longer referenced by adjacency lists.
            if !self.succs[edge.source().index()].contains(&edge.id()) {
                continue;
            }
            let (color, style, lbl) = match edge.kind() {
                EdgeKind::Fallthrough => ("black", "solid", ""),
                EdgeKind::ConditionalTrue => ("green4", "solid", "T"),
                EdgeKind::ConditionalFalse => ("red", "solid", "F"),
                EdgeKind::Unconditional => ("blue", "solid", ""),
                EdgeKind::Back => ("blue", "dashed", "back"),
                EdgeKind::Call => ("purple", "solid", "call"),
                EdgeKind::CallReturn => ("purple", "dashed", "ret"),
                EdgeKind::SwitchCase => ("orange", "dotted", "case"),
                EdgeKind::Jump => ("blue", "bold", "jmp"),
                EdgeKind::IndirectJump => ("blue", "dotted", "ijmp"),
                EdgeKind::IndirectCall => ("purple", "dotted", "icall"),
                EdgeKind::ExceptionHandler => ("darkred", "solid", "handler"),
                EdgeKind::ExceptionUnwind => ("darkred", "dashed", "unwind"),
                EdgeKind::ExceptionLeave => ("darkred", "dotted", "leave"),
            };
            write!(
                w,
                "    {} -> {} [color={color} style={style}",
                edge.source(),
                edge.target(),
            )?;

            // Show weight in the label if present.
            let weight_str = edge
                .weight()
                .map(|w| alloc::format!(" ({w:.2})"))
                .unwrap_or_default();
            let full_label = alloc::format!("{lbl}{weight_str}");
            if !full_label.is_empty() {
                write!(w, " label=\"{full_label}\"")?;
            }

            // Thicker line for high-probability edges.
            if let Some(wt) = edge.weight() {
                let penwidth = 1.0 + wt * 3.0; // 1.0–4.0
                write!(w, " penwidth={penwidth:.1}")?;
            }

            writeln!(w, "];")?;
        }

        writeln!(w, "}}")
    }

    /// Produce the DOT representation as a [`String`].
    pub fn to_dot(&self) -> String {
        let mut s = String::new();
        self.write_dot(&mut s)
            .expect("fmt::Write to String cannot fail");
        s
    }
}
