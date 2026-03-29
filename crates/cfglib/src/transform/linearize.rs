//! Re-linearization — serialize a CFG back to a flat instruction stream.
//!
//! The [`linearize`] function sorts blocks according to a chosen
//! [`BlockOrder`], then emits labels, instructions, and explicit
//! jumps/branches so that the resulting instruction sequence is
//! semantically equivalent to the graph.
//!
//! Because cfglib is ISA-agnostic, the caller must provide an
//! [`Emitter`] that knows how to create jump/branch/label
//! instructions for the target ISA.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::edge::EdgeKind;

/// How to order blocks in the output stream.
#[derive(Debug, Clone)]
pub enum BlockOrder {
    /// Reverse-postorder (good for structured code).
    ReversePostorder,
    /// Allocation order (block ids in ascending order).
    AllocationOrder,
    /// Caller-specified order.
    Custom(Vec<BlockId>),
}

/// A single instruction in the linearized output.
#[derive(Debug, Clone)]
pub struct LinearInst<I> {
    /// The instruction.
    pub inst: I,
    /// Which block this instruction came from.
    pub block: BlockId,
    /// Index within the block (label/jump synthetics use `usize::MAX`).
    pub index: usize,
}

/// ISA adapter for emitting jump, branch, and label instructions.
///
/// cfglib does not know how to create machine instructions, so the
/// ISA frontend must implement this trait.
pub trait Emitter<I> {
    /// Emit an unconditional jump to the given label.
    fn emit_jump(&self, target: &str) -> I;

    /// Emit a conditional branch to the given label.
    ///
    /// `condition` is the last instruction of the source block (the
    /// terminating branch instruction). The emitter can inspect it to
    /// determine the condition encoding.
    fn emit_conditional_branch(&self, condition: &I, target: &str) -> I;

    /// Emit a label pseudo-instruction.
    fn emit_label(&self, label: &str) -> I;

    /// Emit a no-op (optional — used for alignment).
    fn emit_nop(&self) -> Option<I> {
        None
    }
}

/// Produce a label name for a block.
fn block_label<I>(cfg: &Cfg<I>, id: BlockId) -> String {
    cfg.block(id)
        .label()
        .map(String::from)
        .unwrap_or_else(|| alloc::format!(".bb{}", id.0))
}

/// Linearize a CFG into a flat instruction stream.
///
/// Blocks are laid out in the specified [`BlockOrder`]. For each
/// block the function:
///
/// 1. Emits a label instruction (via [`Emitter::emit_label`]).
/// 2. Emits the block's instructions in order.
/// 3. If the block's layout successor is not its fallthrough target,
///    emits an explicit jump or branch.
///
/// Returns the instruction stream as a `Vec<LinearInst<I>>`.
pub fn linearize<I: Clone>(
    cfg: &Cfg<I>,
    order: BlockOrder,
    emitter: &dyn Emitter<I>,
) -> Vec<LinearInst<I>> {
    let sorted: Vec<BlockId> = match order {
        BlockOrder::ReversePostorder => cfg.reverse_postorder(),
        BlockOrder::AllocationOrder => (0..cfg.num_blocks())
            .map(|i| BlockId::from_raw(i as u32))
            .collect(),
        BlockOrder::Custom(ids) => ids,
    };

    let mut out: Vec<LinearInst<I>> = Vec::new();

    for (pos, &id) in sorted.iter().enumerate() {
        let block = cfg.block(id);
        let label = block_label(cfg, id);

        // 1. Label.
        out.push(LinearInst {
            inst: emitter.emit_label(&label),
            block: id,
            index: usize::MAX,
        });

        // 2. Block instructions.
        for (idx, inst) in block.instructions().iter().enumerate() {
            out.push(LinearInst {
                inst: inst.clone(),
                block: id,
                index: idx,
            });
        }

        // 3. Determine whether we need an explicit jump.
        let next_in_layout = if pos + 1 < sorted.len() {
            Some(sorted[pos + 1])
        } else {
            None
        };

        let succ_edges: Vec<_> = cfg
            .successor_edges(id)
            .iter()
            .map(|&eid| cfg.edge(eid))
            .collect();

        emit_tail_jump(cfg, id, &succ_edges, next_in_layout, emitter, &mut out);
    }

    out
}

/// Returns `true` if `kind` represents a fallthrough-like edge that
/// can be satisfied by layout adjacency (no explicit jump needed).
fn is_fallthrough_kind(kind: EdgeKind) -> bool {
    matches!(
        kind,
        EdgeKind::Fallthrough | EdgeKind::ConditionalFalse | EdgeKind::CallReturn
    )
}

/// Emit trailing jump/branch if the fallthrough doesn't reach the
/// intended successor.
fn emit_tail_jump<I: Clone>(
    cfg: &Cfg<I>,
    id: BlockId,
    succ_edges: &[&crate::edge::Edge],
    next_in_layout: Option<BlockId>,
    emitter: &dyn Emitter<I>,
    out: &mut Vec<LinearInst<I>>,
) {
    if succ_edges.is_empty() {
        return; // No successors — return/terminate block.
    }

    // Partition edges into the fallthrough candidate and everything else.
    // At most one edge can be a fallthrough (satisfied by layout adjacency).
    let fallthrough = succ_edges.iter().find(|e| is_fallthrough_kind(e.kind()));
    let branches: Vec<_> = succ_edges
        .iter()
        .filter(|e| !is_fallthrough_kind(e.kind()))
        .collect();

    // Emit explicit jumps/branches for all non-fallthrough edges.
    let last_inst = cfg.block(id).instructions().last();

    for edge in &branches {
        let label = block_label(cfg, edge.target());
        match edge.kind() {
            // Conditional edges → emit a conditional branch.
            EdgeKind::ConditionalTrue => {
                if let Some(cond) = last_inst {
                    out.push(LinearInst {
                        inst: emitter.emit_conditional_branch(cond, &label),
                        block: id,
                        index: usize::MAX,
                    });
                }
            }
            // Everything else (Jump, SwitchCase, ExceptionHandler, etc.)
            // → emit an unconditional jump.
            _ => {
                out.push(LinearInst {
                    inst: emitter.emit_jump(&label),
                    block: id,
                    index: usize::MAX,
                });
            }
        }
    }

    // Handle the fallthrough edge: emit a jump only if the layout
    // successor is not the fallthrough target.
    if let Some(ft) = fallthrough
        && next_in_layout != Some(ft.target())
    {
        let label = block_label(cfg, ft.target());
        out.push(LinearInst {
            inst: emitter.emit_jump(&label),
            block: id,
            index: usize::MAX,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::edge::EdgeKind;
    use crate::test_util::{MockInst, ff};
    use alloc::vec;

    /// A trivial emitter that produces string-based mock instructions.
    struct TestEmitter;

    impl Emitter<MockInst> for TestEmitter {
        fn emit_jump(&self, _target: &str) -> MockInst {
            MockInst(crate::flow::FlowEffect::Fallthrough, "jump")
        }
        fn emit_conditional_branch(&self, _cond: &MockInst, _target: &str) -> MockInst {
            MockInst(crate::flow::FlowEffect::Fallthrough, "branch")
        }
        fn emit_label(&self, _label: &str) -> MockInst {
            MockInst(crate::flow::FlowEffect::Fallthrough, "label")
        }
    }

    /// Collect all mnemonic names from the linearized output.
    fn mnemonics(out: &[LinearInst<MockInst>]) -> Vec<&'static str> {
        out.iter().map(|li| li.inst.1).collect()
    }

    #[test]
    fn linearize_single_block() {
        let mut cfg = Cfg::new();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("a"));
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("b"));
        let out = linearize(&cfg, BlockOrder::AllocationOrder, &TestEmitter);
        let names = mnemonics(&out);
        // Should be: label, a, b
        assert_eq!(names, vec!["label", "a", "b"]);
    }

    #[test]
    fn linearize_two_blocks_with_fallthrough() {
        let mut cfg = Cfg::new();
        let b = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("a"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.add_edge(cfg.entry(), b, EdgeKind::Fallthrough);
        let out = linearize(&cfg, BlockOrder::AllocationOrder, &TestEmitter);
        let names = mnemonics(&out);
        // Should be: label, a, label, b — no jump needed (fallthrough).
        assert_eq!(names, vec!["label", "a", "label", "b"]);
    }

    #[test]
    fn linearize_non_fallthrough_emits_jump() {
        let mut cfg = Cfg::new();
        let b = cfg.new_block();
        let c = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("a"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.block_mut(c).instructions_vec_mut().push(ff("c"));
        // entry → c (Jump), layout order is entry, b, c.
        cfg.add_edge(cfg.entry(), c, EdgeKind::Jump);
        let out = linearize(&cfg, BlockOrder::AllocationOrder, &TestEmitter);
        let names = mnemonics(&out);
        // entry's successor is c but layout next is b → needs jump.
        assert!(names.contains(&"jump"), "should emit jump: {names:?}");
    }

    #[test]
    fn linearize_conditional_branch() {
        let mut cfg = Cfg::new();
        let t = cfg.new_block();
        let f = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("cmp"));
        cfg.block_mut(t).instructions_vec_mut().push(ff("then"));
        cfg.block_mut(f).instructions_vec_mut().push(ff("else"));
        cfg.add_edge(cfg.entry(), t, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), f, EdgeKind::ConditionalFalse);
        let out = linearize(&cfg, BlockOrder::AllocationOrder, &TestEmitter);
        let names = mnemonics(&out);
        // Should have a conditional branch for the true edge.
        assert!(names.contains(&"branch"), "should emit branch: {names:?}");
    }

    #[test]
    fn linearize_rpo_order() {
        let mut cfg = Cfg::new();
        let b = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("entry"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.add_edge(cfg.entry(), b, EdgeKind::Fallthrough);
        let out = linearize(&cfg, BlockOrder::ReversePostorder, &TestEmitter);
        // In RPO for a linear chain, entry comes first.
        assert_eq!(out[0].block, cfg.entry());
    }

    #[test]
    fn linearize_custom_order() {
        let mut cfg = Cfg::new();
        let b = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("entry"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.add_edge(cfg.entry(), b, EdgeKind::Fallthrough);
        // Reverse order: b first, then entry.
        let out = linearize(
            &cfg,
            BlockOrder::Custom(alloc::vec![b, cfg.entry()]),
            &TestEmitter,
        );
        assert_eq!(out[0].block, b);
    }
}
