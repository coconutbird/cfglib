//! Copy propagation.
//!
//! Identifies instructions that are simple copies (`dst = src`) and
//! replaces all uses of `dst` with `src`, then removes the dead copy.
//!
//! The consumer implements [`CopySource`] to tell the analysis which
//! instructions are copies and how to rewrite operands.
//!
//! This is a classic SSA/def-use chain optimization that simplifies
//! redundant moves and phi-resolved copies.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::InstrInfo;
use super::def_use::DefUseChains;
use crate::block::BlockId;
use crate::cfg::Cfg;

/// Trait for instructions that can be identified as copies.
///
/// A **copy** is an instruction with exactly one def and one use,
/// where the semantics are simply `def := use` with no computation.
/// Examples: `mov dst, src`, register-register copies, phi-resolved moves.
pub trait CopySource: InstrInfo {
    /// If this instruction is a simple copy, return `Some((dst, src))`.
    ///
    /// Return `None` if the instruction is not a copy (has side effects,
    /// multiple defs, computation, etc.).
    fn as_copy(&self) -> Option<(Self::Variable, Self::Variable)>;

    /// Rewrite a use of `old` to `new` in this instruction.
    ///
    /// Called during propagation to replace operands.
    fn rewrite_use(&mut self, old: &Self::Variable, new: &Self::Variable);
}

/// Result of copy propagation.
#[derive(Debug, Clone)]
pub struct CopyPropagationStats {
    /// Number of uses rewritten.
    pub uses_rewritten: usize,
    /// Number of copy instructions removed.
    pub copies_removed: usize,
}

/// The provably value-preserving substitutions of the graph's copies,
/// with chains resolved: each admitted `dst → src` satisfies the
/// sole-definition, stable-source, and dominated-uses guards, and
/// soundness composes across links — each link's source is stable at and
/// below its copy, and dominance is transitive.
fn sound_substitutions<I: CopySource, E>(cfg: &Cfg<I, E>) -> BTreeMap<I::Variable, I::Variable> {
    let mut def_sites: BTreeMap<I::Variable, Vec<super::ProgramPoint>> = BTreeMap::new();
    let mut use_sites: BTreeMap<I::Variable, Vec<super::ProgramPoint>> = BTreeMap::new();
    for block in cfg.blocks() {
        for (inst_idx, inst) in block.instructions().iter().enumerate() {
            let point = super::ProgramPoint {
                block: block.id(),
                inst_idx,
            };
            for def in inst.defs() {
                def_sites.entry(def.clone()).or_default().push(point);
            }
            for used in inst.uses() {
                use_sites.entry(used.clone()).or_default().push(point);
            }
        }
    }
    let dom = crate::DominatorTree::compute(cfg);
    let point_dominates = |a: super::ProgramPoint, b: super::ProgramPoint| {
        if a.block == b.block {
            a.inst_idx < b.inst_idx
        } else {
            dom.dominates(a.block, b.block)
        }
    };

    let mut substitutions: BTreeMap<I::Variable, I::Variable> = BTreeMap::new();
    for block in cfg.blocks() {
        for (inst_idx, inst) in block.instructions().iter().enumerate() {
            let Some((dst, src)) = inst.as_copy() else {
                continue;
            };
            if dst == src || !dom.is_reachable(block.id()) {
                continue;
            }
            let copy_point = super::ProgramPoint {
                block: block.id(),
                inst_idx,
            };
            // The copy must be the sole definition of `dst`.
            if def_sites.get(&dst).is_none_or(|sites| sites.len() != 1) {
                continue;
            }
            // `src` must hold one stable value wherever `dst` is read.
            match def_sites.get(&src).map(Vec::as_slice) {
                None | Some([]) => {}
                Some([site]) if point_dominates(*site, copy_point) => {}
                Some(_) => continue,
            }
            // Every use of `dst` must see this copy.
            let dominated = use_sites.get(&dst).is_none_or(|sites| {
                sites
                    .iter()
                    .all(|&site| site == copy_point || point_dominates(copy_point, site))
            });
            if !dominated {
                continue;
            }
            substitutions.insert(dst, src);
        }
    }

    let targets: Vec<I::Variable> = substitutions.keys().cloned().collect();
    for dst in targets {
        let mut resolved = substitutions[&dst].clone();
        let mut seen = alloc::collections::BTreeSet::new();
        while let Some(next) = substitutions.get(&resolved).cloned() {
            if !seen.insert(next.clone()) {
                break; // cycle guard
            }
            resolved = next;
        }
        substitutions.insert(dst, resolved);
    }
    substitutions
}

/// Run copy propagation on the CFG.
///
/// 1. Build def and use site maps plus the dominator tree.
/// 2. Find copy instructions (`dst = src`) that are **provably
///    value-preserving**: the copy is `dst`'s only definition, it
///    dominates every use of `dst`, and `src` is stable — either never
///    defined in the graph (entry state: a parameter, an environment
///    value) or defined exactly once at a site dominating the copy.
/// 3. Replace the dominated uses of each such `dst` with `src`,
///    resolving copy chains (`a = b; c = a` → uses of `c` read `b`).
/// 4. Remove dead copies (whose defs have no remaining uses).
///
/// Multi-definition variables — reused storage slots, loop-carried
/// values — never propagate: the guards make the pass sound on any
/// well-formed graph, not only single-assignment form, at the cost of
/// leaving such copies in place. Single-assignment input satisfies every
/// guard, so SSA consumers see the previous behavior unchanged.
///
/// Dominance is judged at block granularity, which relies on the
/// standard well-formedness assumption that every use is reached only
/// after its definition executed (verifier-checked bytecode and
/// compiler-produced graphs guarantee this). A path that entered the
/// copy's block but left through a mid-block exceptional exit before the
/// copy cannot reach a use of the destination: the copy is the
/// destination's only definition, so such a use would read an
/// unassigned variable.
///
/// Returns the number of rewrites and removals.
pub fn copy_propagation<I: CopySource + Clone, E>(cfg: &mut Cfg<I, E>) -> CopyPropagationStats {
    let substitutions = sound_substitutions(cfg);
    if substitutions.is_empty() {
        return CopyPropagationStats {
            uses_rewritten: 0,
            copies_removed: 0,
        };
    }
    // Phase 2: rewrite uses across all blocks.
    let mut uses_rewritten = 0;
    let block_ids: Vec<BlockId> = cfg
        .blocks()
        .iter()
        .map(super::super::block::BasicBlock::id)
        .collect();
    for &bid in &block_ids {
        let insts = cfg.block_mut(bid).instructions_mut();
        for inst in insts.iter_mut() {
            for (old, new) in &substitutions {
                if inst.uses().contains(old) {
                    inst.rewrite_use(old, new);
                    uses_rewritten += 1;
                }
            }
        }
    }

    // Phase 3: remove dead copies using fresh def-use analysis.
    let chains = DefUseChains::compute(cfg);
    let mut copies_removed = 0;

    for &bid in &block_ids {
        let insts = cfg.block(bid).instructions().to_vec();
        let mut new_insts = Vec::with_capacity(insts.len());
        for (idx, inst) in insts.into_iter().enumerate() {
            if inst.as_copy().is_some() {
                let def_site = super::ProgramPoint {
                    block: bid,
                    inst_idx: idx,
                };
                if chains.uses_of(def_site).is_empty() {
                    copies_removed += 1;
                    continue; // drop the dead copy
                }
            }
            new_insts.push(inst);
        }
        *cfg.block_mut(bid).instructions_mut() = new_insts;
    }

    CopyPropagationStats {
        uses_rewritten,
        copies_removed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::edge::EdgeKind;
    use crate::test_util::{DfInst, df_copy, df_def, df_use};

    #[test]
    fn simple_copy_propagation() {
        // def r0; copy r1 = r0; use r1 → use r0, remove copy.
        let mut cfg: Cfg<DfInst> = Cfg::new();
        let exit = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_mut()
            .extend([df_def("def_r0", 0), df_copy("mov", 1, 0)]);
        cfg.block_mut(exit)
            .instructions_mut()
            .push(df_use("use_r1", 1));
        cfg.add_edge(cfg.entry(), exit, EdgeKind::Fallthrough);

        let result = copy_propagation(&mut cfg);
        assert_eq!(result.uses_rewritten, 1);
        assert_eq!(result.copies_removed, 1);

        // The use should now reference r0 instead of r1.
        let exit_inst = &cfg.block(exit).instructions()[0];
        assert_eq!(exit_inst.uses[0], 0);
    }

    #[test]
    fn copy_chain_propagation() {
        // def r0; copy r1 = r0; copy r2 = r1; use r2 → use r0.
        let mut cfg: Cfg<DfInst> = Cfg::new();
        cfg.block_mut(cfg.entry()).instructions_mut().extend([
            df_def("def_r0", 0),
            df_copy("mov1", 1, 0),
            df_copy("mov2", 2, 1),
            df_use("use_r2", 2),
        ]);

        let result = copy_propagation(&mut cfg);
        assert!(result.uses_rewritten >= 1);
        // The final use should reference r0.
        let insts = cfg.block(cfg.entry()).instructions();
        let last = insts.last().unwrap();
        assert_eq!(last.uses[0], 0);
    }

    #[test]
    fn a_redefined_source_never_propagates() {
        // def r0; copy r1 = r0; def r0; use r1 — the copy captured the
        // FIRST r0, so rewriting the use would read the second.
        let mut cfg: Cfg<DfInst> = Cfg::new();
        cfg.block_mut(cfg.entry()).instructions_mut().extend([
            df_def("def_r0", 0),
            df_copy("mov", 1, 0),
            df_def("redef_r0", 0),
            df_use("use_r1", 1),
        ]);

        let result = copy_propagation(&mut cfg);
        assert_eq!(result.uses_rewritten, 0);
        assert_eq!(result.copies_removed, 0);
        let insts = cfg.block(cfg.entry()).instructions();
        assert_eq!(insts.last().unwrap().uses[0], 1);
    }

    #[test]
    fn a_non_dominating_copy_never_propagates() {
        // entry branches; only one arm copies r1 = r0; the merge reads
        // r1 — the copy does not dominate the use.
        let mut cfg: Cfg<DfInst> = Cfg::new();
        let arm = cfg.new_block();
        let other = cfg.new_block();
        let merge = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_mut()
            .push(df_def("def_r0", 0));
        cfg.block_mut(arm)
            .instructions_mut()
            .push(df_copy("mov", 1, 0));
        cfg.block_mut(merge)
            .instructions_mut()
            .push(df_use("use_r1", 1));
        cfg.add_edge(cfg.entry(), arm, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), other, EdgeKind::ConditionalFalse);
        cfg.add_edge(arm, merge, EdgeKind::Fallthrough);
        cfg.add_edge(other, merge, EdgeKind::Fallthrough);

        let result = copy_propagation(&mut cfg);
        assert_eq!(result.uses_rewritten, 0);
        let insts = cfg.block(merge).instructions();
        assert_eq!(insts[0].uses[0], 1);
    }

    #[test]
    fn an_undefined_source_is_entry_state_and_propagates() {
        // r7 has no definition in the graph (a parameter): the copy's
        // dominated uses may read it directly.
        let mut cfg: Cfg<DfInst> = Cfg::new();
        cfg.block_mut(cfg.entry())
            .instructions_mut()
            .extend([df_copy("mov", 1, 7), df_use("use_r1", 1)]);

        let result = copy_propagation(&mut cfg);
        assert_eq!(result.uses_rewritten, 1);
        assert_eq!(result.copies_removed, 1);
        let insts = cfg.block(cfg.entry()).instructions();
        assert_eq!(insts.last().unwrap().uses[0], 7);
    }
}
