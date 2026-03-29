//! Dead code elimination (DCE).
//!
//! Removes instructions whose definitions are never used. Uses
//! liveness analysis to identify instructions that define locations
//! which are not live after the instruction. Instructions with side
//! effects (non-empty `effects()`) are always kept.

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;

/// Dead code elimination — remove instructions whose definitions
/// are never used.
///
/// Returns the number of instructions removed.
///
/// # Examples
///
/// ```
/// # use cfglib::{Cfg, EdgeKind, Location, InstrInfo};
/// # #[derive(Debug, Clone)]
/// # struct Inst { uses: Vec<Location>, defs: Vec<Location> }
/// # impl InstrInfo for Inst {
/// #     fn uses(&self) -> &[Location] { &self.uses }
/// #     fn defs(&self) -> &[Location] { &self.defs }
/// # }
/// use cfglib::dead_code_elimination;
///
/// let mut cfg = Cfg::<Inst>::new();
/// let b0 = cfg.entry();
/// // Dead definition: defines r0 but nothing uses it.
/// cfg.block_mut(b0).push(Inst { uses: vec![], defs: vec![Location(0)] });
///
/// let removed = dead_code_elimination(&mut cfg);
/// assert_eq!(removed, 1);
/// ```
pub fn dead_code_elimination<I: crate::dataflow::InstrInfo + Clone>(cfg: &mut Cfg<I>) -> usize {
    use crate::dataflow::fixpoint;
    use crate::dataflow::liveness::LivenessProblem;

    let liveness = fixpoint::solve(cfg, &LivenessProblem);
    let mut removed = 0;

    // Phase 1: compute which instructions to keep per block.
    let block_ids: Vec<BlockId> = cfg.blocks().iter().map(|b| b.id()).collect();
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
                    live.insert(*u);
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
