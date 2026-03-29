//! Value numbering — local (LVN) and global (GVN).
//!
//! Identifies redundant computations by assigning the same "value number"
//! to expressions that compute identical results.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use smallvec::SmallVec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::dataflow::{InstrInfo, Location};
use crate::graph::dominator::DominatorTree;

/// A value number — opaque identifier for a computed value.
pub type ValueNumber = u32;

/// An expression key used for hash-consing.
///
/// Uses `SmallVec` to avoid heap allocation for expressions with ≤ 4
/// operands (the vast majority of real instructions).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExprKey {
    /// Instruction "opcode" or name (ISA-specific identifier).
    pub opcode: u32,
    /// Value numbers of the operands.
    pub operands: SmallVec<[ValueNumber; 4]>,
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
        let operands: SmallVec<[ValueNumber; 4]> = inst
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
/// Performs a single DFS walk over the dominator tree, maintaining
/// scoped `loc → VN` and `expr → VN` tables that are pushed on
/// entry and popped on exit. This avoids cloning maps for every
/// block and runs in O(n · α) time per instruction (where α is the
/// BTreeMap operation cost).
pub fn global_value_numbering<I: ValueNumberInfo>(
    cfg: &Cfg<I>,
    dom: &DominatorTree,
) -> ValueNumbering {
    let mut blocks = BTreeMap::new();
    let mut loc_to_vn: BTreeMap<Location, ValueNumber> = BTreeMap::new();
    let mut expr_to_vn: BTreeMap<ExprKey, ValueNumber> = BTreeMap::new();
    let mut next_vn: ValueNumber = 0;

    gvn_dfs(
        cfg,
        dom,
        cfg.entry(),
        &mut loc_to_vn,
        &mut expr_to_vn,
        &mut next_vn,
        &mut blocks,
    );

    ValueNumbering {
        blocks,
        num_values: next_vn,
    }
}

/// Recursive DFS over the dominator tree with push/pop scoping.
fn gvn_dfs<I: ValueNumberInfo>(
    cfg: &Cfg<I>,
    dom: &DominatorTree,
    bid: BlockId,
    loc_to_vn: &mut BTreeMap<Location, ValueNumber>,
    expr_to_vn: &mut BTreeMap<ExprKey, ValueNumber>,
    next_vn: &mut ValueNumber,
    blocks: &mut BTreeMap<BlockId, BlockValueNumbers>,
) {
    // Snapshot the current scope so we can restore on exit.
    let loc_snapshot: Vec<(Location, ValueNumber)> = Vec::new();
    let expr_snapshot: Vec<ExprKey> = Vec::new();
    let mut loc_added: Vec<Location> = Vec::new();
    let mut loc_overwritten: Vec<(Location, ValueNumber)> = Vec::new();
    let mut expr_added: Vec<ExprKey> = Vec::new();
    let vn_before = *next_vn;
    let _ = (loc_snapshot, expr_snapshot); // suppress unused warnings

    // Process instructions in this block.
    let insts = cfg.block(bid).instructions();
    let mut inst_vn = Vec::with_capacity(insts.len());
    let mut redundant = Vec::new();

    for (idx, inst) in insts.iter().enumerate() {
        if !inst.is_pure() || inst.defs().is_empty() {
            inst_vn.push(None);
            continue;
        }

        let operands: SmallVec<[ValueNumber; 4]> = inst
            .uses()
            .iter()
            .map(|loc| {
                if let Some(&vn) = loc_to_vn.get(loc) {
                    vn
                } else {
                    let vn = *next_vn;
                    *next_vn += 1;
                    loc_added.push(*loc);
                    loc_to_vn.insert(*loc, vn);
                    vn
                }
            })
            .collect();

        let key = ExprKey {
            opcode: inst.opcode(),
            operands,
        };

        if let Some(&existing_vn) = expr_to_vn.get(&key) {
            inst_vn.push(Some(existing_vn));
            redundant.push(idx);
            for d in inst.defs() {
                if let Some(old) = loc_to_vn.insert(*d, existing_vn) {
                    loc_overwritten.push((*d, old));
                } else {
                    loc_added.push(*d);
                }
            }
        } else {
            let vn = *next_vn;
            *next_vn += 1;
            expr_added.push(key.clone());
            expr_to_vn.insert(key, vn);
            inst_vn.push(Some(vn));
            for d in inst.defs() {
                if let Some(old) = loc_to_vn.insert(*d, vn) {
                    loc_overwritten.push((*d, old));
                } else {
                    loc_added.push(*d);
                }
            }
        }
    }

    blocks.insert(bid, BlockValueNumbers { inst_vn, redundant });

    // Recurse into dominator-tree children.
    let children = dom.children(bid);
    for child in children {
        gvn_dfs(cfg, dom, child, loc_to_vn, expr_to_vn, next_vn, blocks);
    }

    // Pop scope: undo all insertions/overwrites from this block.
    for key in expr_added {
        expr_to_vn.remove(&key);
    }
    for loc in loc_added {
        loc_to_vn.remove(&loc);
    }
    for (loc, old_vn) in loc_overwritten {
        loc_to_vn.insert(loc, old_vn);
    }
    // Note: next_vn is NOT rolled back — value numbers are global.
    let _ = vn_before;
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
    fn gvn_detects_cross_block_redundancy() {
        // Block 0: t2 = op1(loc0, loc1)
        // Block 1: t3 = op1(loc0, loc1)  ← redundant (same expr, dominator has it)
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
        // The instruction in block b should be marked redundant.
        let b_vn = &vn.blocks[&b];
        assert_eq!(
            b_vn.redundant.len(),
            1,
            "cross-block redundancy not detected"
        );
        assert_eq!(b_vn.redundant[0], 0);
        // Both instructions should share the same value number.
        let entry_vn = vn.blocks[&cfg.entry()].inst_vn[0].unwrap();
        let b_inst_vn = b_vn.inst_vn[0].unwrap();
        assert_eq!(entry_vn, b_inst_vn);
    }

    #[test]
    fn gvn_no_cross_block_without_dominance() {
        // Diamond: entry → A, entry → B. Same expr in A and B.
        // Neither dominates the other, so no redundancy.
        let mut cfg: Cfg<VnInst> = Cfg::new();
        let a = cfg.new_block();
        let b = cfg.new_block();
        cfg.block_mut(a)
            .instructions_vec_mut()
            .push(vn_inst(1, &[0, 1], &[2]));
        cfg.block_mut(b)
            .instructions_vec_mut()
            .push(vn_inst(1, &[0, 1], &[3]));
        cfg.add_edge(cfg.entry(), a, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), b, EdgeKind::ConditionalFalse);
        let dom = DominatorTree::compute(&cfg);
        let vn = global_value_numbering(&cfg, &dom);
        assert!(vn.blocks[&a].redundant.is_empty());
        assert!(vn.blocks[&b].redundant.is_empty());
    }
}
