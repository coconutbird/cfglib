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
        writeln!(w, "    node [shape=box fontname=\"monospace\" fontsize=10];")?;
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

            writeln!(
                w,
                "    {id} [label=\"{label_prefix}{id}\\n{body}\"];",
            )?;
        }

        for edge in &self.edges {
            let (color, style, lbl) = match edge.kind() {
                EdgeKind::Fallthrough => ("black", "solid", ""),
                EdgeKind::ConditionalTrue => ("green4", "solid", "T"),
                EdgeKind::ConditionalFalse => ("red", "solid", "F"),
                EdgeKind::Unconditional => ("blue", "solid", ""),
                EdgeKind::Back => ("blue", "dashed", "back"),
                EdgeKind::Call => ("purple", "solid", "call"),
                EdgeKind::CallReturn => ("purple", "dashed", "ret"),
                EdgeKind::SwitchCase => ("orange", "dotted", "case"),
            };
            write!(
                w,
                "    {} -> {} [color={color} style={style}",
                edge.source(), edge.target(),
            )?;

            if !lbl.is_empty() {
                write!(w, " label=\"{lbl}\"")?;
            }

            writeln!(w, "];")?;
        }

        writeln!(w, "}}")
    }

    /// Produce the DOT representation as a [`String`].
    pub fn to_dot(&self) -> String {
        let mut s = String::new();
        self.write_dot(&mut s).expect("fmt::Write to String cannot fail");
        s
    }
}
