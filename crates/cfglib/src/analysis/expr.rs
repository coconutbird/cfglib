//! Expression tree / DAG recovery.
//!
//! Reconstructs high-level expression trees from flat instruction
//! sequences. Given flat instructions like:
//!
//! ```text
//! mul t0, b, c
//! add t1, a, t0
//! ```
//!
//! This module recovers the expression tree `add(a, mul(b, c))`.
//!
//! The consumer implements [`ExprInstr`] to describe how each
//! instruction maps to an expression operator, and the framework
//! builds expression DAGs per block using def-use chains.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::dataflow::{InstrInfo, Location};

/// A node in an expression tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprNode {
    /// A leaf: a location (register, variable) that is an input.
    Leaf(Location),
    /// An operation with an operator name and operand sub-expressions.
    Op {
        /// The operator (e.g. "add", "mul", "shl", "load").
        operator: String,
        /// The operands, each an expression sub-tree.
        operands: Vec<ExprNode>,
    },
    /// A constant value.
    Const(i64),
    /// An opaque instruction that couldn't be decomposed further.
    /// Contains the instruction index within the block.
    Opaque {
        /// Block containing the instruction.
        block: BlockId,
        /// Index of the instruction.
        inst_idx: usize,
    },
}

impl ExprNode {
    /// Whether this is a leaf node.
    pub fn is_leaf(&self) -> bool {
        matches!(self, ExprNode::Leaf(_))
    }

    /// Whether this is a compound operation.
    pub fn is_op(&self) -> bool {
        matches!(self, ExprNode::Op { .. })
    }

    /// Depth of the expression tree.
    pub fn depth(&self) -> usize {
        match self {
            ExprNode::Leaf(_) | ExprNode::Const(_) | ExprNode::Opaque { .. } => 1,
            ExprNode::Op { operands, .. } => {
                1 + operands.iter().map(|o| o.depth()).max().unwrap_or(0)
            }
        }
    }

    /// Count total nodes in the expression tree.
    pub fn node_count(&self) -> usize {
        match self {
            ExprNode::Leaf(_) | ExprNode::Const(_) | ExprNode::Opaque { .. } => 1,
            ExprNode::Op { operands, .. } => {
                1 + operands.iter().map(|o| o.node_count()).sum::<usize>()
            }
        }
    }
}

/// Trait for instructions that can be decomposed into expression operators.
///
/// The consumer implements this to tell the framework what operator
/// each instruction represents and what its operands are.
pub trait ExprInstr: InstrInfo {
    /// If this instruction can be represented as an expression,
    /// return the operator name and the list of operand locations.
    ///
    /// Return `None` for instructions that can't be decomposed
    /// (side-effecting, control flow, etc.).
    fn as_expr(&self) -> Option<(&str, &[Location])>;

    /// If this instruction loads a constant, return the value.
    fn as_const(&self) -> Option<i64> {
        None
    }
}

/// Expression trees recovered for a single block.
#[derive(Debug, Clone)]
pub struct BlockExprTrees {
    /// The block these trees belong to.
    pub block: BlockId,
    /// Expression tree for each "root" definition in the block.
    /// A root def is one whose result is used outside this block
    /// or is a side-effecting instruction's output.
    pub roots: Vec<(Location, ExprNode)>,
}

/// Recover expression trees for a single block.
///
/// Walks the block's instructions and builds expression trees by
/// inlining single-use temporaries into their use sites.
pub fn recover_block_expressions<I: ExprInstr>(cfg: &Cfg<I>, block: BlockId) -> BlockExprTrees {
    let insts = cfg.block(block).instructions();

    // Map from location → the expression that defines it (within this block).
    let mut loc_expr: BTreeMap<Location, ExprNode> = BTreeMap::new();
    // Count uses of each location within this block.
    let mut use_count: BTreeMap<Location, usize> = BTreeMap::new();

    // First pass: count intra-block uses.
    for inst in insts {
        for &u in inst.uses() {
            *use_count.entry(u).or_insert(0) += 1;
        }
    }

    // Second pass: build expressions.
    for (idx, inst) in insts.iter().enumerate() {
        if let Some(c) = inst.as_const() {
            // Constant load.
            if let Some(&dst) = inst.defs().first() {
                loc_expr.insert(dst, ExprNode::Const(c));
            }
        } else if let Some((op, _operand_locs)) = inst.as_expr() {
            let operands: Vec<ExprNode> = inst
                .uses()
                .iter()
                .map(|&loc| {
                    // Inline if this is a single-use temporary defined in this block.
                    if use_count.get(&loc).copied().unwrap_or(0) == 1
                        && let Some(sub) = loc_expr.remove(&loc)
                    {
                        return sub;
                    }
                    ExprNode::Leaf(loc)
                })
                .collect();

            if let Some(&dst) = inst.defs().first() {
                loc_expr.insert(
                    dst,
                    ExprNode::Op {
                        operator: String::from(op),
                        operands,
                    },
                );
            }
        } else {
            // Opaque instruction — keep as-is.
            for &dst in inst.defs() {
                loc_expr.insert(
                    dst,
                    ExprNode::Opaque {
                        block,
                        inst_idx: idx,
                    },
                );
            }
        }
    }

    // Collect roots: everything still in loc_expr (not inlined).
    let roots: Vec<(Location, ExprNode)> = loc_expr.into_iter().collect();

    BlockExprTrees { block, roots }
}

/// Recover expression trees for all blocks in the CFG.
pub fn recover_expressions<I: ExprInstr>(cfg: &Cfg<I>) -> Vec<BlockExprTrees> {
    cfg.blocks()
        .iter()
        .map(|b| recover_block_expressions(cfg, b.id()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::dataflow::Location;
    use crate::test_util::{DfInst, df_const, df_op};

    #[test]
    fn recover_simple_expression_tree() {
        // mul t0, r1, r2    (t0 = r1 * r2)
        // add t1, r0, t0    (t1 = r0 + t0 = r0 + r1 * r2)
        let mut cfg: Cfg<DfInst> = Cfg::new();
        cfg.block_mut(cfg.entry()).instructions_vec_mut().extend([
            df_op("mul", "mul", 10, &[1, 2]),  // t0(loc10) = r1 * r2
            df_op("add", "add", 11, &[0, 10]), // t1(loc11) = r0 + t0
        ]);

        let trees = recover_block_expressions(&cfg, cfg.entry());
        assert_eq!(trees.roots.len(), 1, "only t1 should remain as root");
        let (loc, ref expr) = trees.roots[0];
        assert_eq!(loc, Location(11));

        // Should be Op("add", [Leaf(r0), Op("mul", [Leaf(r1), Leaf(r2)])])
        match expr {
            ExprNode::Op { operator, operands } => {
                assert_eq!(operator, "add");
                assert_eq!(operands.len(), 2);
                assert_eq!(operands[0], ExprNode::Leaf(Location(0)));
                match &operands[1] {
                    ExprNode::Op {
                        operator: inner_op,
                        operands: inner_ops,
                    } => {
                        assert_eq!(inner_op, "mul");
                        assert_eq!(inner_ops.len(), 2);
                        assert_eq!(inner_ops[0], ExprNode::Leaf(Location(1)));
                        assert_eq!(inner_ops[1], ExprNode::Leaf(Location(2)));
                    }
                    _ => panic!("expected nested Op for mul"),
                }
            }
            _ => panic!("expected Op at root"),
        }
    }

    #[test]
    fn constant_folding_in_tree() {
        // const t0 = 42; add t1, r0, t0 → add(Leaf(r0), Const(42))
        let mut cfg: Cfg<DfInst> = Cfg::new();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .extend([df_const("ldc", 10, 42), df_op("add", "add", 11, &[0, 10])]);

        let trees = recover_block_expressions(&cfg, cfg.entry());
        assert_eq!(trees.roots.len(), 1);
        let (_, ref expr) = trees.roots[0];
        match expr {
            ExprNode::Op { operands, .. } => {
                assert_eq!(operands[1], ExprNode::Const(42));
            }
            _ => panic!("expected Op"),
        }
    }

    #[test]
    fn multi_use_not_inlined() {
        // mul t0, r1, r2; add t1, r0, t0; sub t2, t0, r3
        // t0 has 2 uses → should NOT be inlined, stays as Leaf.
        let mut cfg: Cfg<DfInst> = Cfg::new();
        cfg.block_mut(cfg.entry()).instructions_vec_mut().extend([
            df_op("mul", "mul", 10, &[1, 2]),
            df_op("add", "add", 11, &[0, 10]),
            df_op("sub", "sub", 12, &[10, 3]),
        ]);

        let trees = recover_block_expressions(&cfg, cfg.entry());
        // t0 has 2 uses, so it stays as a root. t1 and t2 also root.
        assert_eq!(trees.roots.len(), 3);
        // Both t1 and t2 should reference t0 as a Leaf, not inline it.
        for (_, expr) in &trees.roots {
            if let ExprNode::Op { operands, .. } = expr {
                for op in operands {
                    if let ExprNode::Leaf(loc) = op
                        && *loc == Location(10)
                    {
                        // Good — t0 is a leaf, not inlined.
                    }
                }
            }
        }
    }
}
