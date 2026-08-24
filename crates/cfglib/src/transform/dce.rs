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
///
/// # Panics
///
/// Panics only if the unbounded fixpoint solve reports a step-limit error,
/// which the unbounded configuration cannot produce.
pub fn dead_code_elimination<I: crate::dataflow::EffectInfo + Clone>(cfg: &mut Cfg<I>) -> usize {
    use crate::dataflow::fixpoint;
    use crate::dataflow::liveness::LivenessProblem;

    let liveness = fixpoint::solve_problem(cfg, &LivenessProblem)
        .expect("an unbounded solve cannot exceed a step limit");
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
        *cfg.block_mut(bid).instructions_mut() = new_insts;
    }

    removed
}

/// Dead code elimination including the structure left dead: interleave
/// [`dead_code_elimination`] with [`simplify`](crate::simplify) until neither
/// changes anything.
///
/// Instruction removal can empty blocks, which is exactly the structure the
/// cleanup passes remove; the pair is iterated until neither reports a
/// change, so the result is a fixpoint of both passes regardless of what one
/// exposes for the other. Use [`remove_dead_code_mapped`] to retain the
/// composed identity mapping.
///
/// Returns the total number of instructions, blocks, and edges affected.
///
/// # Panics
///
/// Panics only if the unbounded fixpoint solve reports a step-limit error,
/// which the unbounded configuration cannot produce.
pub fn remove_dead_code<I: crate::dataflow::EffectInfo + Clone>(cfg: &mut Cfg<I>) -> usize {
    remove_dead_code_mapped(cfg).0
}

/// [`remove_dead_code`], additionally composing the structural passes'
/// rewrite maps.
///
/// Instruction indices are positions, not identities, so the map covers the
/// block and edge identity changes only — exactly as
/// [`simplify_mapped`](crate::simplify_mapped) reports them.
///
/// # Panics
///
/// Panics only if the unbounded fixpoint solve reports a step-limit error,
/// which the unbounded configuration cannot produce.
pub fn remove_dead_code_mapped<I: crate::dataflow::EffectInfo + Clone>(
    cfg: &mut Cfg<I>,
) -> (usize, crate::rewrite::RewriteMap) {
    let mut total = 0;
    let mut mapping = crate::rewrite::RewriteMap::new();
    loop {
        let instructions = dead_code_elimination(cfg);
        let (structural, round_map) = super::cleanup::simplify_mapped(cfg);
        mapping.compose(round_map);
        let round = instructions + structural;
        if round == 0 {
            break;
        }
        total += round;
    }
    (total, mapping)
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
            .instructions_mut()
            .extend([df_def("dead_def", 0), df_def("live_def", 1)]);

        cfg.block_mut(exit)
            .instructions_mut()
            .push(df_use("use1", 1));

        cfg.add_edge(cfg.entry(), exit, EdgeKind::Fallthrough);

        let removed = dead_code_elimination(&mut cfg);
        assert_eq!(removed, 1, "should remove the dead def of loc0");
        assert_eq!(cfg.block(cfg.entry()).instructions().len(), 1);
        assert_eq!(cfg.block(cfg.entry()).instructions()[0].name, "live_def");
    }

    #[test]
    fn remove_dead_code_removes_the_structure_left_dead() {
        // A block holding only a dead definition: instruction DCE empties it,
        // and the structural passes bypass and merge what remains.
        let mut cfg: Cfg<DfInst> = Cfg::new();
        let mid = cfg.new_block();
        let exit = cfg.new_block();
        cfg.block_mut(cfg.entry()).push(df_def("keep", 0));
        cfg.block_mut(mid).push(df_def("dead", 1));
        cfg.block_mut(exit).push(df_use("use", 0));
        cfg.add_edge(cfg.entry(), mid, EdgeKind::Fallthrough);
        cfg.add_edge(mid, exit, EdgeKind::Fallthrough);

        let (total, mapping) = remove_dead_code_mapped(&mut cfg);
        assert!(
            total >= 3,
            "one dead instruction plus at least a bypass and a merge, got {total}"
        );
        assert!(
            !mapping.is_empty(),
            "structural removal must report identity changes"
        );
        let entry_insts = cfg.block(cfg.entry()).instructions();
        assert_eq!(entry_insts.len(), 2, "keep + use merged into the entry");
        assert!(cfg.successors(cfg.entry()).next().is_none());
    }

    #[test]
    fn remove_dead_code_is_a_noop_on_clean_input() {
        let mut cfg: Cfg<DfInst> = Cfg::new();
        cfg.block_mut(cfg.entry()).push(df_def("keep", 0));
        cfg.block_mut(cfg.entry()).push(df_use("use", 0));
        let (total, mapping) = remove_dead_code_mapped(&mut cfg);
        assert_eq!(total, 0);
        assert!(mapping.is_empty());
    }
}
