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

    // Map block → position in layout for fallthrough detection.
    let mut layout_pos = alloc::vec![usize::MAX; cfg.num_blocks()];
    for (pos, &id) in sorted.iter().enumerate() {
        layout_pos[id.index()] = pos;
    }

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
    match succ_edges.len() {
        0 => { /* no successors — return/terminate block */ }
        1 => {
            let target = succ_edges[0].target();
            if next_in_layout != Some(target) {
                let label = block_label(cfg, target);
                out.push(LinearInst {
                    inst: emitter.emit_jump(&label),
                    block: id,
                    index: usize::MAX,
                });
            }
            // else: fallthrough — no jump needed.
        }
        _ => {
            // Multiple successors — find the conditional branch.
            // Convention: ConditionalTrue / ConditionalFalse pair,
            // or Jump + Fallthrough pair.
            let true_edge = succ_edges
                .iter()
                .find(|e| matches!(e.kind(), EdgeKind::ConditionalTrue | EdgeKind::Jump));
            let false_edge = succ_edges
                .iter()
                .find(|e| matches!(e.kind(), EdgeKind::ConditionalFalse | EdgeKind::Fallthrough));

            if let Some(te) = true_edge {
                let label = block_label(cfg, te.target());
                // Use the last instruction as the condition hint.
                let block = cfg.block(id);
                if let Some(last) = block.instructions().last() {
                    out.push(LinearInst {
                        inst: emitter.emit_conditional_branch(last, &label),
                        block: id,
                        index: usize::MAX,
                    });
                }
            }

            // Emit unconditional jump for the false/fallthrough
            // edge if it's not the layout successor.
            if let Some(fe) = false_edge
                && next_in_layout != Some(fe.target())
            {
                let label = block_label(cfg, fe.target());
                out.push(LinearInst {
                    inst: emitter.emit_jump(&label),
                    block: id,
                    index: usize::MAX,
                });
            }
        }
    }
}
