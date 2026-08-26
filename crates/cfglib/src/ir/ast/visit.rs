//! Traversal and instruction-mapping utilities over lifted ASTs.

extern crate alloc;
use alloc::vec::Vec;

use super::node::{AstNode, CatchHandler, LoopKind, SwitchCase};

impl<I> AstNode<I> {
    /// Visits this node and every descendant in preorder.
    pub fn visit<'n>(&'n self, visit: &mut impl FnMut(&'n AstNode<I>)) {
        visit(self);
        match self {
            AstNode::Sequence { body }
            | AstNode::Label { body, .. }
            | AstNode::Guarded { body, .. }
            | AstNode::Loop { body, .. } => visit_nodes(body, visit),
            AstNode::IfThenElse {
                then_body,
                else_body,
                ..
            } => {
                visit_nodes(then_body, visit);
                visit_nodes(else_body, visit);
            }
            AstNode::Switch {
                cases,
                default_body,
                ..
            } => {
                for case in cases {
                    visit_nodes(&case.body, visit);
                }
                visit_nodes(default_body, visit);
            }
            AstNode::TryCatch {
                try_body,
                handlers,
                finally_body,
            } => {
                visit_nodes(try_body, visit);
                for handler in handlers {
                    visit_nodes(&handler.body, visit);
                }
                visit_nodes(finally_body, visit);
            }
            AstNode::Block { .. }
            | AstNode::Return { .. }
            | AstNode::Break { .. }
            | AstNode::Continue { .. }
            | AstNode::Goto { .. } => {}
        }
    }

    /// Visits every stored instruction in program-structure order.
    ///
    /// Loop and branch condition instructions are visited where their block
    /// appears — a pre-tested loop's condition before its body, a post-tested
    /// loop's condition after its body.
    pub fn for_each_instruction<'n>(&'n self, visit: &mut impl FnMut(&'n I)) {
        match self {
            AstNode::Block { instructions, .. } | AstNode::Return { instructions, .. } => {
                instructions.iter().for_each(&mut *visit);
            }
            AstNode::Sequence { body }
            | AstNode::Label { body, .. }
            | AstNode::Guarded { body, .. } => nodes_for_each(body, visit),
            AstNode::Loop { kind, body, .. } => match kind {
                LoopKind::Endless => nodes_for_each(body, visit),
                LoopKind::While { condition, .. } => {
                    condition.iter().for_each(&mut *visit);
                    nodes_for_each(body, visit);
                }
                LoopKind::DoWhile { condition, .. } => {
                    nodes_for_each(body, visit);
                    condition.iter().for_each(&mut *visit);
                }
            },
            AstNode::IfThenElse {
                condition_instructions,
                then_body,
                else_body,
                ..
            } => {
                condition_instructions.iter().for_each(&mut *visit);
                nodes_for_each(then_body, visit);
                nodes_for_each(else_body, visit);
            }
            AstNode::Switch {
                condition_instructions,
                cases,
                default_body,
                ..
            } => {
                condition_instructions.iter().for_each(&mut *visit);
                for case in cases {
                    nodes_for_each(&case.body, visit);
                }
                nodes_for_each(default_body, visit);
            }
            AstNode::TryCatch {
                try_body,
                handlers,
                finally_body,
            } => {
                nodes_for_each(try_body, visit);
                for handler in handlers {
                    nodes_for_each(&handler.body, visit);
                }
                nodes_for_each(finally_body, visit);
            }
            AstNode::Break { .. } | AstNode::Continue { .. } | AstNode::Goto { .. } => {}
        }
    }

    /// Maps every stored instruction into another payload level, preserving
    /// the complete control structure.
    ///
    /// The AST is level-agnostic, so this is the re-leveling hook: a
    /// consumer lifts structure once and then translates instruction
    /// payloads (for example flat instructions into statements of a
    /// higher-level representation).
    #[must_use]
    pub fn map_instructions<J>(self, map: &mut impl FnMut(I) -> J) -> AstNode<J> {
        match self {
            AstNode::Block { id, instructions } => AstNode::Block {
                id,
                instructions: map_all(instructions, map),
            },
            AstNode::Return { id, instructions } => AstNode::Return {
                id,
                instructions: map_all(instructions, map),
            },
            AstNode::Sequence { body } => AstNode::Sequence {
                body: map_nodes(body, map),
            },
            AstNode::Label { name, body } => AstNode::Label {
                name,
                body: map_nodes(body, map),
            },
            AstNode::Guarded {
                predicate,
                when_true,
                body,
            } => AstNode::Guarded {
                predicate: map(predicate),
                when_true,
                body: map_nodes(body, map),
            },
            AstNode::Loop { header, kind, body } => AstNode::Loop {
                header,
                kind: match kind {
                    LoopKind::Endless => LoopKind::Endless,
                    LoopKind::While {
                        condition,
                        exit_on_true,
                    } => LoopKind::While {
                        condition: map_all(condition, map),
                        exit_on_true,
                    },
                    LoopKind::DoWhile {
                        latch,
                        condition,
                        continue_on_true,
                    } => LoopKind::DoWhile {
                        latch,
                        condition: map_all(condition, map),
                        continue_on_true,
                    },
                },
                body: map_nodes(body, map),
            },
            AstNode::IfThenElse {
                condition,
                condition_instructions,
                then_body,
                else_body,
            } => AstNode::IfThenElse {
                condition,
                condition_instructions: map_all(condition_instructions, map),
                then_body: map_nodes(then_body, map),
                else_body: map_nodes(else_body, map),
            },
            AstNode::Switch {
                condition,
                condition_instructions,
                cases,
                default_body,
                default_edge,
            } => AstNode::Switch {
                condition,
                condition_instructions: map_all(condition_instructions, map),
                cases: cases
                    .into_iter()
                    .map(|case| SwitchCase {
                        id: case.id,
                        edges: case.edges,
                        body: map_nodes(case.body, map),
                    })
                    .collect(),
                default_body: map_nodes(default_body, map),
                default_edge,
            },
            AstNode::TryCatch {
                try_body,
                handlers,
                finally_body,
            } => AstNode::TryCatch {
                try_body: map_nodes(try_body, map),
                handlers: handlers
                    .into_iter()
                    .map(|handler| CatchHandler {
                        handler: handler.handler,
                        entry: handler.entry,
                        kind: handler.kind,
                        body: map_nodes(handler.body, map),
                    })
                    .collect(),
                finally_body: map_nodes(finally_body, map),
            },
            AstNode::Break { label } => AstNode::Break { label },
            AstNode::Continue { label } => AstNode::Continue { label },
            AstNode::Goto { target } => AstNode::Goto { target },
        }
    }
}

fn visit_nodes<'n, I>(nodes: &'n [AstNode<I>], visit: &mut impl FnMut(&'n AstNode<I>)) {
    for node in nodes {
        node.visit(visit);
    }
}

fn nodes_for_each<'n, I>(nodes: &'n [AstNode<I>], visit: &mut impl FnMut(&'n I)) {
    for node in nodes {
        node.for_each_instruction(visit);
    }
}

fn map_all<I, J>(instructions: Vec<I>, map: &mut impl FnMut(I) -> J) -> Vec<J> {
    instructions.into_iter().map(&mut *map).collect()
}

fn map_nodes<I, J>(nodes: Vec<AstNode<I>>, map: &mut impl FnMut(I) -> J) -> Vec<AstNode<J>> {
    nodes
        .into_iter()
        .map(|node| node.map_instructions(map))
        .collect()
}
