//! Partial Redundancy Elimination (PRE).
//!
//! Identifies and eliminates partially redundant computations using
//! value numbering and dominance information.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::analysis::valuenumber::{ValueNumber, ValueNumberInfo, global_value_numbering};
use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::graph::dominator::DominatorTree;

/// Result of PRE analysis (which instructions are fully redundant).
#[derive(Debug, Clone)]
pub struct PreResult {
    /// Map of (block, instruction index) → the value number that is redundant.
    pub redundant: Vec<(BlockId, usize, ValueNumber)>,
    /// Number of eliminated instructions.
    pub eliminated: usize,
}

/// Analyse the CFG for partially redundant expressions.
///
/// This implements a simplified "lazy code motion" style PRE:
/// 1. Run local value numbering per block.
/// 2. Propagate available expressions along dominator tree edges.
/// 3. Mark as redundant any expression whose VN is already available
///    from a dominating block.
pub fn analyse_pre<I: ValueNumberInfo>(cfg: &Cfg<I>, dom: &DominatorTree) -> PreResult {
    let rpo = cfg.reverse_postorder();
    let gvn = global_value_numbering(cfg, dom);

    let mut available: BTreeMap<BlockId, BTreeSet<ValueNumber>> = BTreeMap::new();
    let mut redundant = Vec::new();

    for &bid in &rpo {
        // Inherit available VNs from immediate dominator.
        let mut avail = if let Some(idom) = dom.idom(bid) {
            available.get(&idom).cloned().unwrap_or_default()
        } else {
            BTreeSet::new()
        };

        if let Some(bvn) = gvn.blocks.get(&bid) {
            for (idx, vn_opt) in bvn.inst_vn.iter().enumerate() {
                if let Some(vn) = vn_opt {
                    if avail.contains(vn) {
                        redundant.push((bid, idx, *vn));
                    } else {
                        avail.insert(*vn);
                    }
                }
            }
        }

        available.insert(bid, avail);
    }

    let eliminated = redundant.len();
    PreResult {
        redundant,
        eliminated,
    }
}

/// Apply PRE by removing redundant instructions (in-place).
///
/// Returns the number of instructions removed.
pub fn eliminate_pre<I: ValueNumberInfo + Clone>(cfg: &mut Cfg<I>, dom: &DominatorTree) -> usize {
    let result = analyse_pre(cfg, dom);

    // Collect removals per block (reverse order to preserve indices).
    let mut per_block: BTreeMap<BlockId, Vec<usize>> = BTreeMap::new();
    for &(bid, idx, _) in &result.redundant {
        per_block.entry(bid).or_default().push(idx);
    }

    for (bid, mut indices) in per_block {
        indices.sort_unstable();
        indices.reverse();
        for idx in indices {
            cfg.block_mut(bid).instructions_vec_mut().remove(idx);
        }
    }

    result.eliminated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::valuenumber::ValueNumberInfo;
    use crate::cfg::Cfg;
    use crate::dataflow::{InstrInfo, Location};
    use crate::edge::EdgeKind;
    use crate::flow::{FlowControl, FlowEffect};
    use crate::graph::dominator::DominatorTree;
    use alloc::borrow::Cow;

    #[derive(Debug, Clone)]
    struct PreInst {
        op: u32,
        uses: Vec<Location>,
        defs: Vec<Location>,
    }
    impl FlowControl for PreInst {
        fn flow_effect(&self) -> FlowEffect {
            FlowEffect::Fallthrough
        }
        fn display_mnemonic(&self) -> Cow<'_, str> {
            Cow::Borrowed("pre")
        }
    }
    impl InstrInfo for PreInst {
        fn uses(&self) -> &[Location] {
            &self.uses
        }
        fn defs(&self) -> &[Location] {
            &self.defs
        }
    }
    impl ValueNumberInfo for PreInst {
        fn opcode(&self) -> u32 {
            self.op
        }
        fn is_pure(&self) -> bool {
            true
        }
    }
    fn pi(op: u32, u: &[u16], d: &[u16]) -> PreInst {
        PreInst {
            op,
            uses: u.iter().map(|&x| Location(x)).collect(),
            defs: d.iter().map(|&x| Location(x)).collect(),
        }
    }

    #[test]
    fn pre_detects_within_block_redundancy() {
        // Same expression computed twice within one block.
        let mut cfg: Cfg<PreInst> = Cfg::new();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .extend([pi(1, &[0, 1], &[2]), pi(1, &[0, 1], &[3])]);
        let dom = DominatorTree::compute(&cfg);
        let result = analyse_pre(&cfg, &dom);
        assert_eq!(result.eliminated, 1);
    }

    #[test]
    fn pre_no_redundancy() {
        let mut cfg: Cfg<PreInst> = Cfg::new();
        let b = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(pi(1, &[0, 1], &[2]));
        cfg.block_mut(b)
            .instructions_vec_mut()
            .push(pi(2, &[0, 1], &[3]));
        cfg.add_edge(cfg.entry(), b, EdgeKind::Fallthrough);
        let dom = DominatorTree::compute(&cfg);
        let result = analyse_pre(&cfg, &dom);
        assert_eq!(result.eliminated, 0);
    }
}
