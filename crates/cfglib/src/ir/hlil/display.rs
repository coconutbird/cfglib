//! Indented pseudocode rendering for HLIL functions.

extern crate alloc;

use alloc::string::String;
use core::fmt::{self, Write as _};

use super::statement::HandlerKind;
use super::{Dialect, ExpressionId, ExpressionKind, Function, StatementId, StatementKind};

struct ConstantDisplay<'a, D: Dialect>(&'a D::Constant);

impl<D: Dialect> fmt::Display for ConstantDisplay<'_, D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        D::fmt_constant(formatter, self.0)
    }
}

impl<D: Dialect> Function<D> {
    /// Render this function's body as indented pseudocode.
    #[must_use]
    pub fn to_pseudocode(&self) -> String {
        let mut out = String::new();
        self.write_body(&mut out, &self.body, 0);
        out
    }

    fn write_expression(&self, out: &mut String, id: ExpressionId) {
        let Some(expression) = self.expression(id) else {
            let _ = write!(out, "<missing {id}>");
            return;
        };
        match expression.kind() {
            ExpressionKind::Variable(variable) => {
                let _ = write!(out, "{variable}");
            }
            ExpressionKind::Constant(constant) => {
                let _ = write!(out, "{}", ConstantDisplay::<D>(constant));
            }
            ExpressionKind::Operation {
                operation,
                operands,
            } => {
                out.push_str(D::mnemonic(operation));
                out.push('(');
                for (position, &operand) in operands.iter().enumerate() {
                    if position != 0 {
                        out.push_str(", ");
                    }
                    self.write_expression(out, operand);
                }
                out.push(')');
            }
        }
    }

    fn write_body(&self, out: &mut String, body: &[StatementId], depth: usize) {
        for &statement in body {
            self.write_statement(out, statement, depth);
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one exhaustive match arm per statement shape; splitting it \
                  would scatter the rendering of a single closed vocabulary"
    )]
    fn write_statement(&self, out: &mut String, id: StatementId, depth: usize) {
        let Some(statement) = self.statement(id) else {
            write_indent(out, depth);
            let _ = writeln!(out, "<missing {id}>");
            return;
        };
        write_indent(out, depth);
        match statement.kind() {
            StatementKind::Expression(expression) => {
                self.write_expression(out, *expression);
                out.push_str(";\n");
            }
            StatementKind::Assign { target, value } => {
                self.write_expression(out, *target);
                out.push_str(" = ");
                self.write_expression(out, *value);
                out.push_str(";\n");
            }
            StatementKind::If {
                condition,
                then_body,
                else_body,
            } => {
                out.push_str("if (");
                self.write_expression(out, *condition);
                out.push_str(") {\n");
                self.write_body(out, then_body, depth + 1);
                if !else_body.is_empty() {
                    write_indent(out, depth);
                    out.push_str("} else {\n");
                    self.write_body(out, else_body, depth + 1);
                }
                write_indent(out, depth);
                out.push_str("}\n");
            }
            StatementKind::While { condition, body } => {
                out.push_str("while (");
                self.write_expression(out, *condition);
                out.push_str(") {\n");
                self.write_body(out, body, depth + 1);
                write_indent(out, depth);
                out.push_str("}\n");
            }
            StatementKind::DoWhile { body, condition } => {
                out.push_str("do {\n");
                self.write_body(out, body, depth + 1);
                write_indent(out, depth);
                out.push_str("} while (");
                self.write_expression(out, *condition);
                out.push_str(");\n");
            }
            StatementKind::Loop { body } => {
                out.push_str("loop {\n");
                self.write_body(out, body, depth + 1);
                write_indent(out, depth);
                out.push_str("}\n");
            }
            StatementKind::For {
                initializer,
                condition,
                update,
                body,
            } => {
                out.push_str("for (\n");
                self.write_body(out, initializer, depth + 1);
                write_indent(out, depth);
                out.push_str("; ");
                if let Some(condition) = condition {
                    self.write_expression(out, *condition);
                }
                out.push_str(" ;\n");
                self.write_body(out, update, depth + 1);
                write_indent(out, depth);
                out.push_str(") {\n");
                self.write_body(out, body, depth + 1);
                write_indent(out, depth);
                out.push_str("}\n");
            }
            StatementKind::Switch {
                scrutinee,
                cases,
                default_body,
            } => {
                out.push_str("switch (");
                self.write_expression(out, *scrutinee);
                out.push_str(") {\n");
                for case in cases {
                    write_indent(out, depth);
                    out.push_str("  case ");
                    for (position, value) in case.values.iter().enumerate() {
                        if position != 0 {
                            out.push_str(", ");
                        }
                        let _ = write!(out, "{}", ConstantDisplay::<D>(value));
                    }
                    out.push_str(": {\n");
                    self.write_body(out, &case.body, depth + 2);
                    write_indent(out, depth);
                    out.push_str("  }\n");
                }
                if !default_body.is_empty() {
                    write_indent(out, depth);
                    out.push_str("  default: {\n");
                    self.write_body(out, default_body, depth + 2);
                    write_indent(out, depth);
                    out.push_str("  }\n");
                }
                write_indent(out, depth);
                out.push_str("}\n");
            }
            StatementKind::Break { label } => {
                out.push_str("break");
                if let Some(label) = label {
                    out.push(' ');
                    out.push_str(label);
                }
                out.push_str(";\n");
            }
            StatementKind::Continue { label } => {
                out.push_str("continue");
                if let Some(label) = label {
                    out.push(' ');
                    out.push_str(label);
                }
                out.push_str(";\n");
            }
            StatementKind::Return { values } => {
                out.push_str("return");
                for (position, &value) in values.iter().enumerate() {
                    out.push_str(if position == 0 { " " } else { ", " });
                    self.write_expression(out, value);
                }
                out.push_str(";\n");
            }
            StatementKind::Labeled { label, body } => {
                out.push_str(label);
                out.push_str(": {\n");
                self.write_body(out, body, depth + 1);
                write_indent(out, depth);
                out.push_str("}\n");
            }
            StatementKind::Goto { label } => {
                out.push_str("goto ");
                out.push_str(label);
                out.push_str(";\n");
            }
            StatementKind::Try {
                body,
                handlers,
                finally_body,
            } => {
                out.push_str("try {\n");
                self.write_body(out, body, depth + 1);
                for handler in handlers {
                    write_indent(out, depth);
                    match &handler.kind {
                        HandlerKind::Catch => out.push_str("} catch ("),
                        HandlerKind::CatchAll => out.push_str("} catch (..."),
                        HandlerKind::Fault => out.push_str("} fault ("),
                        HandlerKind::Filter { .. } => out.push_str("} filter ("),
                    }
                    for (position, caught) in handler.caught_types.iter().enumerate() {
                        if position != 0 {
                            out.push_str(" | ");
                        }
                        let _ = write!(out, "{caught:?}");
                    }
                    out.push(')');
                    if let Some(binding) = handler.binding {
                        let _ = write!(out, " {binding}");
                    }
                    out.push_str(" {\n");
                    if let HandlerKind::Filter { filter_body } = &handler.kind {
                        write_indent(out, depth + 1);
                        out.push_str("when {\n");
                        self.write_body(out, filter_body, depth + 2);
                        write_indent(out, depth + 1);
                        out.push_str("}\n");
                    }
                    self.write_body(out, &handler.body, depth + 1);
                }
                if !finally_body.is_empty() {
                    write_indent(out, depth);
                    out.push_str("} finally {\n");
                    self.write_body(out, finally_body, depth + 1);
                }
                write_indent(out, depth);
                out.push_str("}\n");
            }
            StatementKind::Region {
                operation,
                operands,
                body,
            } => {
                out.push('@');
                out.push_str(D::mnemonic(operation));
                out.push('(');
                for (position, &operand) in operands.iter().enumerate() {
                    if position != 0 {
                        out.push_str(", ");
                    }
                    self.write_expression(out, operand);
                }
                out.push_str(") {\n");
                self.write_body(out, body, depth + 1);
                write_indent(out, depth);
                out.push_str("}\n");
            }
        }
    }
}

fn write_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("    ");
    }
}
