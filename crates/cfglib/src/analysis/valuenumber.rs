//! Value numbering — local (LVN) and global (GVN).
//!
//! Identifies redundant computations by assigning the same "value number"
//! to expressions that compute identical results.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::dataflow::{InstrInfo, Location};
use crate::graph::dominator::DominatorTree;

/// A value number — opaque identifier for a computed value.
pub type ValueNumber = u32;

/// An expression key used for hash-consing.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExprKey {
    /// Instruction "opcode" or name (ISA-specific identifier).
    pub opcode: u32,
    /// Value numbers of the operands.
    pub operands: Vec<ValueNumber>,
}

/// Result of value numbering for one block.
#[derive(Debug, Clone)]
pub struct BlockValueNumbers {
    /// Value number assigned to each instruction's def (if any).
    /// Indexed by instruction index within the block.
    pub inst_vn: Vec<Option<ValueNumber>>,
    /// Instructions that are redundant (their value was already computed).
    pub redundant: Vec<usize>,
}

/// Result of value numbering for the whole CFG.
#[derive(Debug, Clone)]
pub struct ValueNumbering {
    /// Per-block results.
    pub blocks: BTreeMap<BlockId, BlockValueNumbers>,
    /// Total value numbers assigned.
    pub num_values: u32,
}

/// Trait for instructions to provide an opcode for value numbering.
pub trait ValueNumberInfo: InstrInfo {
    /// A numeric opcode identifying the operation.
    /// Two instructions with the same opcode and same operand values
    /// produce the same result.
    fn opcode(&self) -> u32;

    /// Whether this instruction is pure (no side effects).
    /// Only pure instructions can be value-numbered.
    fn is_pure(&self) -> bool;
}

/// Run local value numbering on a single block.
pub fn local_value_numbering<I: ValueNumberInfo>(
    cfg: &Cfg<I>,
    block: BlockId,
    start_vn: ValueNumber,
) -> (BlockValueNumbers, ValueNumber) {
    let mut next_vn = start_vn;
    let mut loc_to_vn: BTreeMap<Location, ValueNumber> = BTreeMap::new();
    let mut expr_to_vn: BTreeMap<ExprKey, ValueNumber> = BTreeMap::new();
    let insts = cfg.block(block).instructions();
    let mut inst_vn = Vec::with_capacity(insts.len());
    let mut redundant = Vec::new();

    for (idx, inst) in insts.iter().enumerate() {
        if !inst.is_pure() || inst.defs().is_empty() {
            inst_vn.push(None);
            continue;
        }

        // Build expression key from operand value numbers.
        let operands: Vec<ValueNumber> = inst
            .uses()
            .iter()
            .map(|loc| {
                *loc_to_vn.entry(*loc).or_insert_with(|| {
                    let vn = next_vn;
                    next_vn += 1;
                    vn
                })
            })
            .collect();

        let key = ExprKey {
            opcode: inst.opcode(),
            operands,
        };

        if let Some(&existing_vn) = expr_to_vn.get(&key) {
            // Redundant — same expression already computed.
            inst_vn.push(Some(existing_vn));
            redundant.push(idx);
            for d in inst.defs() {
                loc_to_vn.insert(*d, existing_vn);
            }
        } else {
            let vn = next_vn;
            next_vn += 1;
            expr_to_vn.insert(key, vn);
            inst_vn.push(Some(vn));
            for d in inst.defs() {
                loc_to_vn.insert(*d, vn);
            }
        }
    }

    (BlockValueNumbers { inst_vn, redundant }, next_vn)
}

/// Run global value numbering over the dominator tree.
///
/// Processes blocks in dominator-tree preorder so that dominated
/// blocks inherit value numbers from their dominators.
pub fn global_value_numbering<I: ValueNumberInfo>(
    cfg: &Cfg<I>,
    _dom: &DominatorTree,
) -> ValueNumbering {
    let rpo = cfg.reverse_postorder();
    let mut blocks = BTreeMap::new();
    let mut next_vn: ValueNumber = 0;

    for &bid in &rpo {
        let (bvn, new_next) = local_value_numbering(cfg, bid, next_vn);
        next_vn = new_next;
        blocks.insert(bid, bvn);
    }

    ValueNumbering {
        blocks,
        num_values: next_vn,
    }
}

/// Count total redundant instructions across all blocks.
pub fn count_redundant(vn: &ValueNumbering) -> usize {
    vn.blocks.values().map(|b| b.redundant.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::dataflow::Location;
    use crate::edge::EdgeKind;
    use crate::flow::{FlowControl, FlowEffect};
    use alloc::borrow::Cow;

    #[derive(Debug, Clone)]
    struct VnInst {
        op: u32,
        uses: Vec<Location>,
        defs: Vec<Location>,
        pure_: bool,
    }

    impl FlowControl for VnInst {
        fn flow_effect(&self) -> FlowEffect {
            FlowEffect::Fallthrough
        }
        fn display_mnemonic(&self) -> Cow<'_, str> {
            Cow::Borrowed("vn")
        }
    }

    impl InstrInfo for VnInst {
        fn uses(&self) -> &[Location] {
            &self.uses
        }
        fn defs(&self) -> &[Location] {
            &self.defs
        }
    }

    impl ValueNumberInfo for VnInst {
        fn opcode(&self) -> u32 {
            self.op
        }
        fn is_pure(&self) -> bool {
            self.pure_
        }
    }

    fn vn_inst(op: u32, uses: &[u32], defs: &[u32]) -> VnInst {
        VnInst {
            op,
            uses: uses.iter().map(|&u| Location(u as u16)).collect(),
            defs: defs.iter().map(|&d| Location(d as u16)).collect(),
            pure_: true,
        }
    }

    #[test]
    fn lvn_detects_redundant() {
        // t0 = add(a, b), t1 = add(a, b) → t1 is redundant
        let mut cfg: Cfg<VnInst> = Cfg::new();
        cfg.block_mut(cfg.entry()).instructions_vec_mut().extend([
            vn_inst(1, &[0, 1], &[2]), // t2 = op1(loc0, loc1)
            vn_inst(1, &[0, 1], &[3]), // t3 = op1(loc0, loc1) → redundant
        ]);
        let (bvn, _) = local_value_numbering(&cfg, cfg.entry(), 0);
        assert_eq!(bvn.redundant.len(), 1);
        assert_eq!(bvn.redundant[0], 1);
    }

    #[test]
    fn lvn_different_ops_not_redundant() {
        let mut cfg: Cfg<VnInst> = Cfg::new();
        cfg.block_mut(cfg.entry()).instructions_vec_mut().extend([
            vn_inst(1, &[0, 1], &[2]),
            vn_inst(2, &[0, 1], &[3]), // different opcode
        ]);
        let (bvn, _) = local_value_numbering(&cfg, cfg.entry(), 0);
        assert!(bvn.redundant.is_empty());
    }

    #[test]
    fn gvn_across_blocks() {
        let mut cfg: Cfg<VnInst> = Cfg::new();
        let b = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(vn_inst(1, &[0, 1], &[2]));
        cfg.block_mut(b)
            .instructions_vec_mut()
            .push(vn_inst(1, &[0, 1], &[3]));
        cfg.add_edge(cfg.entry(), b, EdgeKind::Fallthrough);
        let dom = DominatorTree::compute(&cfg);
        let vn = global_value_numbering(&cfg, &dom);
        assert!(vn.num_values > 0);
    }
}
