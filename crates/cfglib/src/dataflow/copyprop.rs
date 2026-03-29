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

use super::defuse::DefUseChains;
use super::{InstrInfo, Location};
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
    fn as_copy(&self) -> Option<(Location, Location)>;

    /// Rewrite a use of `old` to `new` in this instruction.
    ///
    /// Called during propagation to replace operands.
    fn rewrite_use(&mut self, old: Location, new: Location);
}

/// Result of copy propagation.
#[derive(Debug, Clone)]
pub struct CopyPropResult {
    /// Number of uses rewritten.
    pub uses_rewritten: usize,
    /// Number of copy instructions removed.
    pub copies_removed: usize,
}

/// Run copy propagation on the CFG.
///
/// 1. Build def-use chains.
/// 2. Find all copy instructions (`dst = src`).
/// 3. For each copy, replace all uses of `dst` with `src`.
/// 4. Remove dead copies (whose defs have no remaining uses).
///
/// Returns the number of rewrites and removals.
pub fn copy_propagation<I: CopySource + Clone>(cfg: &mut Cfg<I>) -> CopyPropResult {
    // Phase 1: identify copies and build a substitution map.
    // We iterate to a fixpoint in case of copy chains: a = b; c = a → c = b.
    let mut subst: BTreeMap<Location, Location> = BTreeMap::new();

    for block in cfg.blocks() {
        for inst in block.instructions() {
            if let Some((dst, src)) = inst.as_copy() {
                // Resolve through existing substitutions for chains.
                let mut resolved = src;
                let mut seen = alloc::collections::BTreeSet::new();
                while let Some(&next) = subst.get(&resolved) {
                    if !seen.insert(next) {
                        break; // cycle guard
                    }
                    resolved = next;
                }
                subst.insert(dst, resolved);
            }
        }
    }

    if subst.is_empty() {
        return CopyPropResult {
            uses_rewritten: 0,
            copies_removed: 0,
        };
    }

    // Phase 2: rewrite uses across all blocks.
    let mut uses_rewritten = 0;
    let block_ids: Vec<BlockId> = cfg.blocks().iter().map(|b| b.id()).collect();

    for &bid in &block_ids {
        let insts = cfg.block_mut(bid).instructions_vec_mut();
        for inst in insts.iter_mut() {
            for (&old, &new) in &subst {
                if inst.uses().contains(&old) {
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
        *cfg.block_mut(bid).instructions_vec_mut() = new_insts;
    }

    CopyPropResult {
        uses_rewritten,
        copies_removed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::dataflow::Location;
    use crate::edge::EdgeKind;
    use crate::flow::{FlowControl, FlowEffect};
    use alloc::borrow::Cow;
    use alloc::vec;
    use alloc::vec::Vec;

    /// A test instruction that supports copy semantics.
    #[derive(Debug, Clone)]
    struct CopyInst {
        name: &'static str,
        uses: Vec<Location>,
        defs: Vec<Location>,
        is_copy: bool,
    }

    impl FlowControl for CopyInst {
        fn flow_effect(&self) -> FlowEffect {
            FlowEffect::Fallthrough
        }
        fn display_mnemonic(&self) -> Cow<'_, str> {
            Cow::Borrowed(self.name)
        }
    }

    impl InstrInfo for CopyInst {
        fn uses(&self) -> &[Location] {
            &self.uses
        }
        fn defs(&self) -> &[Location] {
            &self.defs
        }
    }

    impl CopySource for CopyInst {
        fn as_copy(&self) -> Option<(Location, Location)> {
            if self.is_copy && self.defs.len() == 1 && self.uses.len() == 1 {
                Some((self.defs[0], self.uses[0]))
            } else {
                None
            }
        }
        fn rewrite_use(&mut self, old: Location, new: Location) {
            for u in &mut self.uses {
                if *u == old {
                    *u = new;
                }
            }
        }
    }

    fn copy_inst(name: &'static str, dst: u16, src: u16) -> CopyInst {
        CopyInst {
            name,
            uses: vec![Location(src)],
            defs: vec![Location(dst)],
            is_copy: true,
        }
    }

    fn def_inst(name: &'static str, dst: u16) -> CopyInst {
        CopyInst {
            name,
            uses: vec![],
            defs: vec![Location(dst)],
            is_copy: false,
        }
    }

    fn use_inst(name: &'static str, src: u16) -> CopyInst {
        CopyInst {
            name,
            uses: vec![Location(src)],
            defs: vec![],
            is_copy: false,
        }
    }

    #[test]
    fn simple_copy_propagation() {
        // def r0; copy r1 = r0; use r1 → use r0, remove copy.
        let mut cfg: Cfg<CopyInst> = Cfg::new();
        let exit = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .extend([def_inst("def_r0", 0), copy_inst("mov", 1, 0)]);
        cfg.block_mut(exit)
            .instructions_vec_mut()
            .push(use_inst("use_r1", 1));
        cfg.add_edge(cfg.entry(), exit, EdgeKind::Fallthrough);

        let result = copy_propagation(&mut cfg);
        assert_eq!(result.uses_rewritten, 1);
        assert_eq!(result.copies_removed, 1);

        // The use should now reference r0 instead of r1.
        let exit_inst = &cfg.block(exit).instructions()[0];
        assert_eq!(exit_inst.uses[0], Location(0));
    }

    #[test]
    fn copy_chain_propagation() {
        // def r0; copy r1 = r0; copy r2 = r1; use r2 → use r0.
        let mut cfg: Cfg<CopyInst> = Cfg::new();
        cfg.block_mut(cfg.entry()).instructions_vec_mut().extend([
            def_inst("def_r0", 0),
            copy_inst("mov1", 1, 0),
            copy_inst("mov2", 2, 1),
            use_inst("use_r2", 2),
        ]);

        let result = copy_propagation(&mut cfg);
        assert!(result.uses_rewritten >= 1);
        // The final use should reference r0.
        let insts = cfg.block(cfg.entry()).instructions();
        let last = insts.last().unwrap();
        assert_eq!(last.uses[0], Location(0));
    }
}
