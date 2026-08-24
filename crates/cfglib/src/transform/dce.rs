//! Dead code elimination (DCE).
//!
//! Removes instructions whose definitions are never used. Uses
//! liveness analysis to identify instructions that define variables
//! which are not live after the instruction. Instructions with side
//! effects (non-empty [`EffectInfo::effects`](crate::EffectInfo::effects))
//! are always kept — requiring [`EffectInfo`](crate::EffectInfo) here is
//! deliberate: a consumer must state an effect vocabulary before this pass
//! will delete anything, which prevents silently removing side-effecting
//! statements.

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;

/// Dead code elimination: remove instructions whose definitions
/// are never used.
///
/// Returns the number of instructions removed.
///
/// # Examples
///
/// ```
/// # use cfglib::{Cfg, EdgeKind, EffectInfo, InstrInfo};
/// # #[derive(Debug, Clone)]
/// # struct Inst { uses: Vec<u16>, defs: Vec<u16> }
/// # impl InstrInfo for Inst {
/// #     type Variable = u16;
/// #     fn uses(&self) -> &[u16] { &self.uses }
/// #     fn defs(&self) -> &[u16] { &self.defs }
/// # }
/// # impl EffectInfo for Inst {
/// #     type Effect = &'static str;
/// #     fn effects(&self) -> &[&'static str] { &[] }
/// # }
/// use cfglib::dead_code_elimination;
///
/// let mut cfg = Cfg::<Inst>::new();
/// let b0 = cfg.entry();
/// // Dead definition: defines r0 but nothing uses it.
/// cfg.block_mut(b0).push(Inst { uses: vec![], defs: vec![0] });
///
/// let removed = dead_code_elimination(&mut cfg);
/// assert_eq!(removed, 1);
/// ```
pub fn dead_code_elimination<I: crate::dataflow::EffectInfo + Clone>(cfg: &mut Cfg<I>) -> usize {
    use crate::dataflow::fixpoint;
    use crate::dataflow::liveness::LivenessProblem;

    let liveness = fixpoint::solve_problem(cfg, &LivenessProblem);
    let mut removed = 0;

    // Phase 1: compute which instructions to keep per block.
    let block_ids: Vec<BlockId> = cfg
        .blocks()
        .iter()
        .map(super::super::block::BasicBlock::id)
        .collect();
    let mut replacements: Vec<(BlockId, Vec<I>)> = Vec::new();

    for &bid in &block_ids {
        let live_out = liveness.fact_out(bid).clone();
        let insts = cfg.block(bid).instructions().to_vec();
        let mut live = live_out;
        let mut keep = vec![true; insts.len()];

        for (i, inst) in insts.iter().enumerate().rev() {
            let has_side_effect = !inst.effects().is_empty();
            let defs_live = inst.defs().iter().any(|d| live.contains(d));

            if !has_side_effect && !inst.defs().is_empty() && !defs_live {
                keep[i] = false;
                removed += 1;
            } else {
                for d in inst.defs() {
                    live.remove(d);
                }
                for u in inst.uses() {
                    live.insert(u.clone());
                }
            }
        }

        if keep.iter().any(|&k| !k) {
            let new_insts: Vec<I> = insts
                .into_iter()
                .zip(keep.iter())
                .filter(|(_, k)| **k)
                .map(|(inst, _)| inst)
                .collect();
            replacements.push((bid, new_insts));
        }
    }

    // Phase 2: apply replacements.
    for (bid, new_insts) in replacements {
        *cfg.block_mut(bid).instructions_vec_mut() = new_insts;
    }

    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::EdgeKind;
    use crate::test_util::{DfInst, df_def, df_use};

    #[test]
    fn dead_code_elimination_removes_unused_def() {
        let mut cfg: Cfg<DfInst> = Cfg::new();
        let exit = cfg.new_block();

        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .extend([df_def("dead_def", 0), df_def("live_def", 1)]);

        cfg.block_mut(exit)
            .instructions_vec_mut()
            .push(df_use("use1", 1));

        cfg.add_edge(cfg.entry(), exit, EdgeKind::Fallthrough);

        let removed = dead_code_elimination(&mut cfg);
        assert_eq!(removed, 1, "should remove the dead def of loc0");
        assert_eq!(cfg.block(cfg.entry()).instructions().len(), 1);
        assert_eq!(cfg.block(cfg.entry()).instructions()[0].name, "live_def");
    }
}
