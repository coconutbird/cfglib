//! Def-use and use-def chains.
//!
//! Links every definition to all instructions that read its value, and
//! every use to all definitions that could supply its value.
//!
//! Built on top of reaching definitions analysis.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use super::reaching::{ReachingDef, ReachingDefs};
use super::{DefSite, InstrInfo, UseSite};
use crate::cfg::Cfg;

/// Def-use and use-def chain results.
#[derive(Debug, Clone)]
pub struct DefUseChains {
    /// For each definition site, the set of use sites that read it.
    pub def_use: BTreeMap<DefSite, BTreeSet<UseSite>>,
    /// For each use site, the set of definition sites that could
    /// supply the value.
    pub use_def: BTreeMap<UseSite, BTreeSet<DefSite>>,
}

impl DefUseChains {
    /// Compute def-use and use-def chains for the given CFG.
    pub fn compute<I: InstrInfo>(cfg: &Cfg<I>) -> Self {
        let reaching = ReachingDefs::compute(cfg);

        let mut def_use: BTreeMap<DefSite, BTreeSet<UseSite>> = BTreeMap::new();
        let mut use_def: BTreeMap<UseSite, BTreeSet<DefSite>> = BTreeMap::new();

        for b in cfg.blocks() {
            let block = b.id();
            let insts = cfg.block(block).instructions();

            // Track the current reaching defs as we walk forward
            // through the block, so intra-block kills are respected.
            let mut current_reaching: BTreeSet<ReachingDef> = reaching.reaching_in(block).clone();

            for (idx, inst) in insts.iter().enumerate() {
                let use_site = UseSite {
                    block,
                    inst_idx: idx,
                };

                // For each location this instruction uses, find all
                // reaching defs of that location at this point.
                for &loc in inst.uses() {
                    let suppliers: BTreeSet<DefSite> = current_reaching
                        .iter()
                        .filter(|rd| rd.location == loc)
                        .map(|rd| rd.site)
                        .collect();

                    for &def_site in &suppliers {
                        def_use.entry(def_site).or_default().insert(use_site);
                    }
                    use_def.entry(use_site).or_default().extend(suppliers);
                }

                // Apply this instruction's defs: kill + gen.
                let defs = inst.defs();
                if !defs.is_empty() {
                    let def_site = DefSite {
                        block,
                        inst_idx: idx,
                    };
                    for loc in defs {
                        current_reaching.retain(|rd| rd.location != *loc);
                    }
                    for &loc in defs {
                        current_reaching.insert(ReachingDef {
                            location: loc,
                            site: def_site,
                        });
                    }
                    // Ensure the def site exists in the map even if
                    // nothing uses it (dead def).
                    def_use.entry(def_site).or_default();
                }
            }
        }

        DefUseChains { def_use, use_def }
    }

    /// Get all use sites that read from a given definition.
    pub fn uses_of(&self, def: DefSite) -> &BTreeSet<UseSite> {
        static EMPTY: BTreeSet<UseSite> = BTreeSet::new();
        self.def_use.get(&def).unwrap_or(&EMPTY)
    }

    /// Get all definition sites that could supply a value read at a
    /// given use site.
    pub fn defs_of(&self, use_site: UseSite) -> &BTreeSet<DefSite> {
        static EMPTY: BTreeSet<DefSite> = BTreeSet::new();
        self.use_def.get(&use_site).unwrap_or(&EMPTY)
    }

    /// Definitions that have no uses (dead code candidates).
    pub fn dead_defs(&self) -> Vec<DefSite> {
        self.def_use
            .iter()
            .filter(|(_, uses)| uses.is_empty())
            .map(|(def, _)| *def)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockId;
    use crate::builder::CfgBuilder;
    use crate::dataflow::DefSite;
    use crate::test_util::{df_def as def, df_use as use_};
    use alloc::vec;

    #[test]
    fn defuse_linear_chain() {
        // bb0: def r0 (idx 0); use r0 (idx 1)
        let cfg = CfgBuilder::build(vec![def("def_r0", 0), use_("use_r0", 0)]).unwrap();
        let chains = DefUseChains::compute(&cfg);

        let def_site = DefSite {
            block: BlockId(0),
            inst_idx: 0,
        };
        let use_site = DefSite {
            block: BlockId(0),
            inst_idx: 1,
        };

        // def→use
        assert!(chains.uses_of(def_site).contains(&use_site));
        // use→def
        assert!(chains.defs_of(use_site).contains(&def_site));
    }

    #[test]
    fn defuse_dead_def_detected() {
        // bb0: def r0; def r1 — r0 never used, r1 never used
        let cfg = CfgBuilder::build(vec![def("def_r0", 0), def("def_r1", 1)]).unwrap();
        let chains = DefUseChains::compute(&cfg);
        let dead = chains.dead_defs();
        assert_eq!(dead.len(), 2, "both defs are dead");
    }

    #[test]
    fn defuse_killed_def_is_dead() {
        // bb0: def r0; def r0; use r0
        // First def is killed by second, so first is dead.
        let cfg = CfgBuilder::build(vec![def("def1", 0), def("def2", 0), use_("use", 0)]).unwrap();
        let chains = DefUseChains::compute(&cfg);
        let dead = chains.dead_defs();
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].inst_idx, 0, "first def (killed) is dead");
    }

    #[test]
    fn defuse_multiple_uses_of_one_def() {
        // bb0: def r0; use r0; use r0
        let cfg = CfgBuilder::build(vec![def("def", 0), use_("use1", 0), use_("use2", 0)]).unwrap();
        let chains = DefUseChains::compute(&cfg);
        let def_site = DefSite {
            block: BlockId(0),
            inst_idx: 0,
        };
        assert_eq!(chains.uses_of(def_site).len(), 2);
    }
}
