//! CFG builder — converts a flat instruction stream into a [`Cfg`]
//! using a scope-stack approach for structured control flow.

extern crate alloc;
use alloc::vec::Vec;
use core::fmt;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::edge::EdgeKind;
use crate::flow::{FlowControl, FlowEffect};

/// Error returned when the instruction stream contains mismatched or
/// unexpected structured control-flow markers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    /// An `else` was encountered without a matching `if` scope.
    UnmatchedElse,
    /// An `endif` was encountered without a matching `if` scope.
    UnmatchedEndIf,
    /// An `endswitch` was encountered without a matching `switch` scope.
    UnmatchedEndSwitch,
    /// A `case` / `default` was encountered outside a `switch` scope.
    UnmatchedSwitchCase,
    /// An `endloop` was encountered without a matching `loop` scope.
    UnmatchedEndLoop,
    /// A `break` was encountered outside any breakable scope.
    BreakOutsideScope,
    /// A `continue` was encountered outside any loop scope.
    ContinueOutsideLoop,
    /// Scopes were still open at the end of the instruction stream.
    UnclosedScopes {
        /// Number of scopes remaining.
        remaining: usize,
    },
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnmatchedElse => write!(f, "encountered `else` without matching `if`"),
            Self::UnmatchedEndIf => write!(f, "encountered `endif` without matching `if`"),
            Self::UnmatchedEndSwitch => {
                write!(f, "encountered `endswitch` without matching `switch`")
            }
            Self::UnmatchedSwitchCase => write!(f, "encountered `case` outside a `switch`"),
            Self::UnmatchedEndLoop => write!(f, "encountered `endloop` without matching `loop`"),
            Self::BreakOutsideScope => write!(f, "`break` outside any breakable scope"),
            Self::ContinueOutsideLoop => write!(f, "`continue` outside any loop scope"),
            Self::UnclosedScopes { remaining } => {
                write!(f, "{remaining} unclosed scope(s) at end of input")
            }
        }
    }
}

/// Scope frames pushed onto the builder's stack to track structured regions.
enum Scope {
    /// An `if` / `else` / `endif` region.
    If {
        /// Block before the `if` (ends with the conditional branch).
        pre_block: BlockId,
        /// Blocks that need an edge to the merge point (end of each arm).
        arm_exits: Vec<BlockId>,
    },
    /// A `loop` / `endloop` region.
    Loop {
        /// The loop header block.
        header: BlockId,
        /// Blocks that break out of this loop (need edges to post-loop).
        break_exits: Vec<BlockId>,
    },
    /// A `switch` / `case` / `endswitch` region.
    ///
    /// `break` inside a switch exits the switch (jumps to post-merge),
    /// unlike `break` in a loop which exits the loop.
    Switch {
        /// Block before the `switch` (ends with the dispatch).
        pre_block: BlockId,
        /// Blocks that need an edge to the merge point (end of each case arm).
        arm_exits: Vec<BlockId>,
        /// Blocks that explicitly break out of this switch.
        break_exits: Vec<BlockId>,
    },
}

/// Builds a [`Cfg<I>`] from an iterator of instructions that implement
/// [`FlowControl`].
pub struct CfgBuilder;

impl CfgBuilder {
    /// Build a CFG from a flat instruction stream.
    ///
    /// Returns an error if the instruction stream contains mismatched
    /// structured control-flow markers (e.g. `else` without `if`).
    ///
    /// Declarations ([`FlowEffect::Declaration`]) are stored in the
    /// current block but do not affect control flow.
    pub fn build<I: FlowControl>(
        instructions: impl IntoIterator<Item = I>,
    ) -> Result<Cfg<I>, BuildError> {
        let mut cfg = Cfg {
            blocks: Vec::new(),
            edges: Vec::new(),
            succs: Vec::new(),
            preds: Vec::new(),
            entry: BlockId(0),
            regions: Vec::new(),
        };

        let mut current = cfg.new_block();
        let mut scopes: Vec<Scope> = Vec::new();

        for inst in instructions {
            match inst.flow_effect() {
                FlowEffect::Declaration => {
                    cfg.block_mut(current).instructions.push(inst);
                }

                FlowEffect::Fallthrough => {
                    cfg.block_mut(current).instructions.push(inst);
                }

                FlowEffect::ConditionalOpen => {
                    cfg.block_mut(current).instructions.push(inst);
                    let true_block = cfg.new_block();
                    cfg.add_edge(current, true_block, EdgeKind::ConditionalTrue);
                    scopes.push(Scope::If {
                        pre_block: current,
                        arm_exits: Vec::new(),
                    });
                    current = true_block;
                }

                FlowEffect::ConditionalAlternate => match scopes.last_mut() {
                    Some(Scope::If {
                        pre_block,
                        arm_exits,
                    }) => {
                        arm_exits.push(current);
                        let alt_block = cfg.new_block();
                        cfg.add_edge(*pre_block, alt_block, EdgeKind::ConditionalFalse);
                        cfg.block_mut(alt_block).instructions.push(inst);
                        current = alt_block;
                    }
                    _ => return Err(BuildError::UnmatchedElse),
                },

                FlowEffect::ConditionalClose => match scopes.pop() {
                    Some(Scope::If {
                        pre_block,
                        mut arm_exits,
                    }) => {
                        let merge = cfg.new_block();
                        arm_exits.push(current);
                        for exit in &arm_exits {
                            cfg.add_edge(*exit, merge, EdgeKind::Fallthrough);
                        }
                        let has_false_edge = cfg
                            .successor_edges(pre_block)
                            .iter()
                            .any(|&eid| cfg.edge(eid).kind() == EdgeKind::ConditionalFalse);
                        if !has_false_edge {
                            cfg.add_edge(pre_block, merge, EdgeKind::ConditionalFalse);
                        }
                        current = merge;
                    }
                    _ => return Err(BuildError::UnmatchedEndIf),
                },

                FlowEffect::SwitchOpen => {
                    cfg.block_mut(current).instructions.push(inst);
                    let first_case = cfg.new_block();
                    cfg.add_edge(current, first_case, EdgeKind::SwitchCase);
                    scopes.push(Scope::Switch {
                        pre_block: current,
                        arm_exits: Vec::new(),
                        break_exits: Vec::new(),
                    });
                    current = first_case;
                }

                FlowEffect::SwitchCase => match scopes.last_mut() {
                    Some(Scope::Switch {
                        pre_block,
                        arm_exits,
                        ..
                    }) => {
                        arm_exits.push(current);
                        let case_block = cfg.new_block();
                        cfg.add_edge(*pre_block, case_block, EdgeKind::SwitchCase);
                        cfg.block_mut(case_block).instructions.push(inst);
                        current = case_block;
                    }
                    _ => return Err(BuildError::UnmatchedSwitchCase),
                },

                FlowEffect::SwitchClose => match scopes.pop() {
                    Some(Scope::Switch {
                        mut arm_exits,
                        break_exits,
                        ..
                    }) => {
                        let merge = cfg.new_block();
                        arm_exits.push(current);
                        for exit in &arm_exits {
                            cfg.add_edge(*exit, merge, EdgeKind::Fallthrough);
                        }
                        for brk in &break_exits {
                            cfg.add_edge(*brk, merge, EdgeKind::Unconditional);
                        }
                        current = merge;
                    }
                    _ => return Err(BuildError::UnmatchedEndSwitch),
                },

                FlowEffect::LoopOpen => {
                    cfg.block_mut(current).instructions.push(inst);
                    let header = cfg.new_block();
                    cfg.add_edge(current, header, EdgeKind::Fallthrough);
                    scopes.push(Scope::Loop {
                        header,
                        break_exits: Vec::new(),
                    });
                    current = header;
                }

                FlowEffect::LoopClose => match scopes.pop() {
                    Some(Scope::Loop {
                        header,
                        break_exits,
                    }) => {
                        cfg.add_edge(current, header, EdgeKind::Back);
                        let post_loop = cfg.new_block();
                        for brk in &break_exits {
                            cfg.add_edge(*brk, post_loop, EdgeKind::Unconditional);
                        }
                        current = post_loop;
                    }
                    _ => return Err(BuildError::UnmatchedEndLoop),
                },

                FlowEffect::Break => {
                    cfg.block_mut(current).instructions.push(inst);
                    let found = scopes.iter_mut().rev().any(|scope| match scope {
                        Scope::Loop { break_exits, .. } | Scope::Switch { break_exits, .. } => {
                            break_exits.push(current);
                            true
                        }
                        _ => false,
                    });
                    if !found {
                        return Err(BuildError::BreakOutsideScope);
                    }
                    current = cfg.new_block();
                }

                FlowEffect::ConditionalBreak => {
                    cfg.block_mut(current).instructions.push(inst);
                    let break_block = cfg.new_block();
                    let cont_block = cfg.new_block();
                    cfg.add_edge(current, break_block, EdgeKind::ConditionalTrue);
                    cfg.add_edge(current, cont_block, EdgeKind::ConditionalFalse);
                    let found = scopes.iter_mut().rev().any(|scope| match scope {
                        Scope::Loop { break_exits, .. } | Scope::Switch { break_exits, .. } => {
                            break_exits.push(break_block);
                            true
                        }
                        _ => false,
                    });
                    if !found {
                        return Err(BuildError::BreakOutsideScope);
                    }
                    current = cont_block;
                }

                FlowEffect::Continue => {
                    cfg.block_mut(current).instructions.push(inst);
                    let found = scopes.iter().rev().any(|scope| {
                        if let Scope::Loop { header, .. } = scope {
                            cfg.add_edge(current, *header, EdgeKind::Back);
                            true
                        } else {
                            false
                        }
                    });
                    if !found {
                        return Err(BuildError::ContinueOutsideLoop);
                    }
                    current = cfg.new_block();
                }

                FlowEffect::ConditionalContinue => {
                    cfg.block_mut(current).instructions.push(inst);
                    let cont_block = cfg.new_block();
                    let found = scopes.iter().rev().any(|scope| {
                        if let Scope::Loop { header, .. } = scope {
                            cfg.add_edge(current, *header, EdgeKind::ConditionalTrue);
                            true
                        } else {
                            false
                        }
                    });
                    if !found {
                        return Err(BuildError::ContinueOutsideLoop);
                    }
                    cfg.add_edge(current, cont_block, EdgeKind::ConditionalFalse);
                    current = cont_block;
                }

                FlowEffect::Return => {
                    cfg.block_mut(current).instructions.push(inst);
                    current = cfg.new_block();
                }

                FlowEffect::ConditionalReturn => {
                    cfg.block_mut(current).instructions.push(inst);
                    let ret_block = cfg.new_block();
                    let cont_block = cfg.new_block();
                    cfg.add_edge(current, ret_block, EdgeKind::ConditionalTrue);
                    cfg.add_edge(current, cont_block, EdgeKind::ConditionalFalse);
                    current = cont_block;
                }

                FlowEffect::Call | FlowEffect::ConditionalCall => {
                    cfg.block_mut(current).instructions.push(inst);
                }

                FlowEffect::Terminate => {
                    cfg.block_mut(current).instructions.push(inst);
                    current = cfg.new_block();
                }

                FlowEffect::Label => {
                    let label_block = cfg.new_block();
                    if !cfg.block(current).is_empty() {
                        cfg.add_edge(current, label_block, EdgeKind::Fallthrough);
                    }
                    cfg.block_mut(label_block).instructions.push(inst);
                    current = label_block;
                }

                // ── Unstructured / CPU-ISA flow ──────────────────
                FlowEffect::Jump => {
                    // Unconditional jump — terminates the current block.
                    // The target edge must be wired by the ISA adapter
                    // after the builder finishes (via `add_edge`).
                    cfg.block_mut(current).instructions.push(inst);
                    current = cfg.new_block();
                }

                FlowEffect::ConditionalJump => {
                    // Conditional jump — splits into taken/not-taken.
                    // The taken target edge must be wired by the ISA
                    // adapter after the builder finishes.
                    cfg.block_mut(current).instructions.push(inst);
                    let cont_block = cfg.new_block();
                    cfg.add_edge(current, cont_block, EdgeKind::ConditionalFalse);
                    current = cont_block;
                }

                FlowEffect::IndirectJump => {
                    // Computed jump — terminates the block. All possible
                    // targets must be wired by the ISA adapter.
                    cfg.block_mut(current).instructions.push(inst);
                    current = cfg.new_block();
                }

                FlowEffect::IndirectCall => {
                    // Indirect call — stays in the block (like Call).
                    cfg.block_mut(current).instructions.push(inst);
                }

                FlowEffect::MayThrow => {
                    // Potentially-throwing instruction — stays in block.
                    // Exception edges are added by the ISA adapter or
                    // region model.
                    cfg.block_mut(current).instructions.push(inst);
                }
            }
        }

        if !scopes.is_empty() {
            return Err(BuildError::UnclosedScopes {
                remaining: scopes.len(),
            });
        }

        // Remove trailing empty blocks with no predecessors.
        Self::trim_trailing_empty(&mut cfg);

        Ok(cfg)
    }

    /// Remove empty blocks at the end that have no predecessors (dead code
    /// artefacts from the builder).
    fn trim_trailing_empty<I>(cfg: &mut Cfg<I>) {
        while cfg.blocks.len() > 1 {
            let last = BlockId((cfg.blocks.len() - 1) as u32);
            if cfg.block(last).is_empty() && cfg.predecessor_edges(last).is_empty() {
                cfg.blocks.pop();
                cfg.succs.pop();
                cfg.preds.pop();
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{MockInst, ff};
    use alloc::vec;

    #[test]
    fn linear_block() {
        let cfg = CfgBuilder::build(vec![ff("a"), ff("b"), ff("c")]).unwrap();
        assert_eq!(cfg.num_blocks(), 1);
        assert_eq!(cfg.num_edges(), 0);
        assert_eq!(cfg.block(cfg.entry()).instructions().len(), 3);
    }

    #[test]
    fn single_return() {
        let cfg = CfgBuilder::build(vec![ff("a"), MockInst(FlowEffect::Return, "ret")]).unwrap();
        // One block with instructions, trailing empty block trimmed.
        assert_eq!(cfg.num_blocks(), 1);
        assert_eq!(cfg.block(cfg.entry()).instructions().len(), 2);
    }

    #[test]
    fn if_endif_no_else() {
        // a; if; b; endif; c
        let cfg = CfgBuilder::build(vec![
            ff("a"),
            MockInst(FlowEffect::ConditionalOpen, "if"),
            ff("b"),
            MockInst(FlowEffect::ConditionalClose, "endif"),
            ff("c"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        // bb0: [a, if]
        // bb1: [b]  (true arm)
        // bb2: []   (merge — c, ret)
        assert!(cfg.num_blocks() >= 3);
        // Entry has two successors: true arm + false arm (merge).
        assert_eq!(cfg.successor_edges(cfg.entry()).len(), 2);
    }

    #[test]
    fn if_else_endif() {
        // a; if; b; else; c; endif; d; ret
        let cfg = CfgBuilder::build(vec![
            ff("a"),
            MockInst(FlowEffect::ConditionalOpen, "if"),
            ff("b"),
            MockInst(FlowEffect::ConditionalAlternate, "else"),
            ff("c"),
            MockInst(FlowEffect::ConditionalClose, "endif"),
            ff("d"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        // bb0: [a, if] → true(bb1), false(bb2)
        // bb1: [b]     → merge(bb3)
        // bb2: [else, c] → merge(bb3)
        // bb3: [d, ret]
        assert!(cfg.num_blocks() >= 4);
        assert_eq!(cfg.successor_edges(cfg.entry()).len(), 2);
    }

    #[test]
    fn simple_loop() {
        // loop; a; endloop; ret
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::LoopOpen, "loop"),
            ff("a"),
            MockInst(FlowEffect::LoopClose, "endloop"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        // bb0: [loop]       → fallthrough to bb1 (header)
        // bb1: [a]          → back to bb1 (header)
        // bb2: [ret]        (post-loop, unreachable without break)
        assert!(cfg.num_blocks() >= 2);
    }

    #[test]
    fn loop_with_break() {
        // loop; a; break; endloop; ret
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::LoopOpen, "loop"),
            ff("a"),
            MockInst(FlowEffect::Break, "break"),
            MockInst(FlowEffect::LoopClose, "endloop"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        // post-loop block should be reachable from the break.
        assert!(cfg.num_blocks() >= 3);
    }

    #[test]
    fn loop_with_conditional_break() {
        // loop; a; breakc; b; endloop; ret
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::LoopOpen, "loop"),
            ff("a"),
            MockInst(FlowEffect::ConditionalBreak, "breakc"),
            ff("b"),
            MockInst(FlowEffect::LoopClose, "endloop"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        // The breakc block should have two successors:
        // - true → break_block (which goes to post-loop)
        // - false → continue block (with b)
        assert!(cfg.num_blocks() >= 4);
    }

    #[test]
    fn declarations_are_stored() {
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::Declaration, "dcl_temps"),
            ff("a"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        // Declarations are included in the block.
        assert_eq!(cfg.num_blocks(), 1);
        assert_eq!(cfg.block(cfg.entry()).instructions().len(), 3);
    }

    #[test]
    fn dot_output() {
        let cfg = CfgBuilder::build(vec![
            ff("add"),
            MockInst(FlowEffect::ConditionalOpen, "if"),
            ff("mul"),
            MockInst(FlowEffect::ConditionalClose, "endif"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        let dot = cfg.to_dot();
        assert!(dot.contains("digraph cfg"));
        assert!(dot.contains("bb0"));
        assert!(dot.contains("green4")); // conditional true edge
    }

    #[test]
    fn traversal_preorder() {
        let cfg = CfgBuilder::build(vec![
            ff("a"),
            MockInst(FlowEffect::ConditionalOpen, "if"),
            ff("b"),
            MockInst(FlowEffect::ConditionalClose, "endif"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        let pre = cfg.dfs_preorder();
        // Entry should be first.
        assert_eq!(pre[0], cfg.entry());
        // All reachable blocks should be visited.
        assert!(pre.len() >= 3);
    }

    #[test]
    fn dominator_tree_linear() {
        let cfg = CfgBuilder::build(vec![
            ff("a"),
            MockInst(FlowEffect::ConditionalOpen, "if"),
            ff("b"),
            MockInst(FlowEffect::ConditionalClose, "endif"),
            ff("c"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        let dom = crate::graph::dominator::DominatorTree::compute(&cfg);
        // Entry dominates all blocks.
        for b in cfg.blocks() {
            assert!(dom.dominates(cfg.entry(), b.id()));
        }
    }

    #[test]
    fn continue_jumps_to_header() {
        // loop; a; continue; b; endloop; ret
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::LoopOpen, "loop"),
            ff("a"),
            MockInst(FlowEffect::Continue, "continue"),
            ff("b"),
            MockInst(FlowEffect::LoopClose, "endloop"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        // The continue should create a back-edge to the header.
        let has_back = cfg.edges().iter().any(|e| e.kind() == EdgeKind::Back);
        assert!(has_back);
        assert!(cfg.num_blocks() >= 3);
    }

    #[test]
    fn conditional_continue() {
        // loop; a; continuec; b; endloop; ret
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::LoopOpen, "loop"),
            ff("a"),
            MockInst(FlowEffect::ConditionalContinue, "continuec"),
            ff("b"),
            MockInst(FlowEffect::LoopClose, "endloop"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        // continuec block has two successors: true->header, false->continue
        let has_cond_true = cfg
            .edges()
            .iter()
            .any(|e| e.kind() == EdgeKind::ConditionalTrue);
        let has_cond_false = cfg
            .edges()
            .iter()
            .any(|e| e.kind() == EdgeKind::ConditionalFalse);
        assert!(has_cond_true);
        assert!(has_cond_false);
        assert!(cfg.num_blocks() >= 4);
    }

    #[test]
    fn conditional_return() {
        // a; retc; b; ret
        let cfg = CfgBuilder::build(vec![
            ff("a"),
            MockInst(FlowEffect::ConditionalReturn, "retc"),
            ff("b"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        // retc splits into ret_block (terminal) and cont_block (with b).
        let has_cond_true = cfg
            .edges()
            .iter()
            .any(|e| e.kind() == EdgeKind::ConditionalTrue);
        let has_cond_false = cfg
            .edges()
            .iter()
            .any(|e| e.kind() == EdgeKind::ConditionalFalse);
        assert!(has_cond_true);
        assert!(has_cond_false);
        assert!(cfg.num_blocks() >= 3);
    }

    #[test]
    fn terminate_ends_block() {
        // a; abort; b; ret
        let cfg = CfgBuilder::build(vec![
            ff("a"),
            MockInst(FlowEffect::Terminate, "abort"),
            ff("b"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        // abort terminates the block; b starts a new (unreachable) block.
        assert!(cfg.num_blocks() >= 2);
    }

    #[test]
    fn label_splits_block() {
        // a; label; b; ret
        let cfg = CfgBuilder::build(vec![
            ff("a"),
            MockInst(FlowEffect::Label, "label_0"),
            ff("b"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        // The label should split into two blocks with a fallthrough edge.
        assert!(cfg.num_blocks() >= 2);
        let has_fallthrough = cfg
            .edges()
            .iter()
            .any(|e| e.kind() == EdgeKind::Fallthrough);
        assert!(has_fallthrough);
    }

    #[test]
    fn switch_with_cases() {
        // switch; a; case; b; case; c; endswitch; d; ret
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::SwitchOpen, "switch"),
            ff("a"),
            MockInst(FlowEffect::SwitchCase, "case"),
            ff("b"),
            MockInst(FlowEffect::SwitchCase, "default"),
            ff("c"),
            MockInst(FlowEffect::SwitchClose, "endswitch"),
            ff("d"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        // switch block dispatches to multiple case arms.
        let switch_edges: Vec<_> = cfg
            .edges()
            .iter()
            .filter(|e| e.kind() == EdgeKind::SwitchCase)
            .collect();
        assert!(switch_edges.len() >= 2); // at least first case + case + default
        assert!(cfg.num_blocks() >= 5);
    }

    #[test]
    fn switch_break_exits_switch() {
        // switch; a; break; case; b; endswitch; c; ret
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::SwitchOpen, "switch"),
            ff("a"),
            MockInst(FlowEffect::Break, "break"),
            MockInst(FlowEffect::SwitchCase, "case"),
            ff("b"),
            MockInst(FlowEffect::SwitchClose, "endswitch"),
            ff("c"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        // The break should wire to the post-switch merge block.
        let unconditional_edges: Vec<_> = cfg
            .edges()
            .iter()
            .filter(|e| e.kind() == EdgeKind::Unconditional)
            .collect();
        assert!(!unconditional_edges.is_empty());
        assert!(cfg.num_blocks() >= 4);
    }

    #[test]
    fn nested_if_in_loop() {
        // loop; if; a; else; b; endif; endloop; ret
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::LoopOpen, "loop"),
            MockInst(FlowEffect::ConditionalOpen, "if"),
            ff("a"),
            MockInst(FlowEffect::ConditionalAlternate, "else"),
            ff("b"),
            MockInst(FlowEffect::ConditionalClose, "endif"),
            MockInst(FlowEffect::LoopClose, "endloop"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        assert!(cfg.num_blocks() >= 5);
        let has_back = cfg.edges().iter().any(|e| e.kind() == EdgeKind::Back);
        assert!(has_back);
    }

    #[test]
    fn nested_loop_in_if() {
        // if; loop; a; breakc; endloop; endif; ret
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::ConditionalOpen, "if"),
            MockInst(FlowEffect::LoopOpen, "loop"),
            ff("a"),
            MockInst(FlowEffect::ConditionalBreak, "breakc"),
            MockInst(FlowEffect::LoopClose, "endloop"),
            MockInst(FlowEffect::ConditionalClose, "endif"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        assert!(cfg.num_blocks() >= 5);
        let has_back = cfg.edges().iter().any(|e| e.kind() == EdgeKind::Back);
        assert!(has_back);
    }

    #[test]
    fn switch_inside_loop_break_exits_switch() {
        // loop; switch; a; break; case; b; endswitch; breakc; endloop; ret
        let cfg = CfgBuilder::build(vec![
            MockInst(FlowEffect::LoopOpen, "loop"),
            MockInst(FlowEffect::SwitchOpen, "switch"),
            ff("a"),
            MockInst(FlowEffect::Break, "break"), // exits switch, not loop
            MockInst(FlowEffect::SwitchCase, "case"),
            ff("b"),
            MockInst(FlowEffect::SwitchClose, "endswitch"),
            MockInst(FlowEffect::ConditionalBreak, "breakc"), // exits loop
            MockInst(FlowEffect::LoopClose, "endloop"),
            MockInst(FlowEffect::Return, "ret"),
        ])
        .unwrap();
        // The break inside the switch should exit the switch.
        // The breakc after endswitch should exit the loop.
        assert!(cfg.num_blocks() >= 6);
        let has_back = cfg.edges().iter().any(|e| e.kind() == EdgeKind::Back);
        assert!(has_back);
    }
}
