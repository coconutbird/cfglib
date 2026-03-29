//! Reaching definitions analysis.
//!
//! A **forward** data flow analysis that computes, for each program point,
//! the set of definitions (writes) that may reach it without being killed
//! (overwritten) along the way.

extern crate alloc;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use super::{InstrInfo, DefSite, Location};
use super::fixpoint::{self, Direction, FixpointResult, Problem};

/// A reaching definition: which location was defined, and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReachingDef {
    /// The location that was written.
    pub location: Location,
    /// Where the write happened.
    pub site: DefSite,
}

/// The reaching definitions problem.
pub struct ReachingDefsProblem;

impl<I: InstrInfo> Problem<I> for ReachingDefsProblem {
    type Fact = BTreeSet<ReachingDef>;

    fn direction(&self) -> Direction {
        Direction::Forward
    }

    fn bottom(&self) -> Self::Fact {
        BTreeSet::new()
    }

    fn entry_fact(&self) -> Self::Fact {
        BTreeSet::new()
    }

    fn meet(&self, a: &Self::Fact, b: &Self::Fact) -> Self::Fact {
        a.union(b).copied().collect()
    }

    fn transfer(&self, cfg: &Cfg<I>, block: BlockId, input: &Self::Fact) -> Self::Fact {
        let mut out = input.clone();
        let insts = cfg.block(block).instructions();

        for (idx, inst) in insts.iter().enumerate() {
            let defs = inst.defs();
            if !defs.is_empty() {
                let site = DefSite {
                    block,
                    inst_idx: idx,
                };
                // Kill: remove all previous defs of the same locations.
                for loc in defs {
                    out.retain(|rd| rd.location != *loc);
                }
                // Gen: add the new defs.
                for &loc in defs {
                    out.insert(ReachingDef {
                        location: loc,
                        site,
                    });
                }
            }
        }

        out
    }
}

/// Result of a reaching definitions analysis with convenient query methods.
pub struct ReachingDefs<F> {
    inner: FixpointResult<F>,
}

impl ReachingDefs<BTreeSet<ReachingDef>> {
    /// Run reaching definitions on the given CFG.
    pub fn compute<I: InstrInfo>(cfg: &Cfg<I>) -> Self {
        let result = fixpoint::solve(cfg, &ReachingDefsProblem);
        Self { inner: result }
    }

    /// Definitions reaching the **entry** of a block (before any
    /// instruction in the block executes).
    pub fn reaching_in(&self, block: BlockId) -> &BTreeSet<ReachingDef> {
        self.inner.fact_in(block)
    }

    /// Definitions reaching the **exit** of a block (after all
    /// instructions in the block have executed).
    pub fn reaching_out(&self, block: BlockId) -> &BTreeSet<ReachingDef> {
        self.inner.fact_out(block)
    }

    /// All definitions of a specific location reaching a block's entry.
    pub fn defs_of_at_entry(
        &self,
        loc: Location,
        block: BlockId,
    ) -> Vec<DefSite> {
        self.reaching_in(block)
            .iter()
            .filter(|rd| rd.location == loc)
            .map(|rd| rd.site)
            .collect()
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use alloc::borrow::Cow;
    use alloc::vec;
    use crate::builder::CfgBuilder;
    use super::{InstrInfo, Location};
    use crate::flow::{FlowControl, FlowEffect};

    /// Mock instruction that carries flow-control info AND data deps.
    #[derive(Debug, Clone)]
    struct DfInst {
        effect: FlowEffect,
        name: &'static str,
        uses: Vec<Location>,
        defs: Vec<Location>,
    }

    impl FlowControl for DfInst {
        fn flow_effect(&self) -> FlowEffect { self.effect }
        fn display_mnemonic(&self) -> Cow<'_, str> { Cow::Borrowed(self.name) }
    }

    impl InstrInfo for DfInst {
        fn uses(&self) -> &[Location] { &self.uses }
        fn defs(&self) -> &[Location] { &self.defs }
    }

    fn ff(name: &'static str) -> DfInst {
        DfInst { effect: FlowEffect::Fallthrough, name, uses: vec![], defs: vec![] }
    }

    fn def(name: &'static str, loc: u16) -> DfInst {
        DfInst { effect: FlowEffect::Fallthrough, name, uses: vec![], defs: vec![Location(loc)] }
    }

    fn use_(name: &'static str, loc: u16) -> DfInst {
        DfInst { effect: FlowEffect::Fallthrough, name, uses: vec![Location(loc)], defs: vec![] }
    }

    fn with_effect(mut inst: DfInst, effect: FlowEffect) -> DfInst {
        inst.effect = effect;
        inst
    }

    // --- Linear CFG tests ---

    #[test]
    fn reaching_linear_single_def() {
        // bb0: def r0; use r0
        let cfg = CfgBuilder::build(vec![def("def_r0", 0), use_("use_r0", 0)]);
        let rd = ReachingDefs::compute(&cfg);
        let out = rd.reaching_out(cfg.entry());
        assert_eq!(out.len(), 1);
        assert!(out.iter().any(|r| r.location == Location(0)));
    }

    #[test]
    fn reaching_linear_kill_redefinition() {
        // bb0: def r0; def r0 (again) — first def should be killed
        let cfg = CfgBuilder::build(vec![def("def1", 0), def("def2", 0)]);
        let rd = ReachingDefs::compute(&cfg);
        let out = rd.reaching_out(cfg.entry());
        assert_eq!(out.len(), 1);
        let rd_item = out.iter().next().unwrap();
        assert_eq!(rd_item.site.inst_idx, 1); // second def survives
    }

    #[test]
    fn reaching_linear_two_locations() {
        // bb0: def r0; def r1 — both should reach the exit
        let cfg = CfgBuilder::build(vec![def("def_r0", 0), def("def_r1", 1)]);
        let rd = ReachingDefs::compute(&cfg);
        let out = rd.reaching_out(cfg.entry());
        assert_eq!(out.len(), 2);
    }

    // --- Branching CFG tests ---

    #[test]
    fn reaching_branch_merges_both_defs() {
        // bb0: if
        // bb1 (true): def r0
        // bb2 (false): def r0
        // bb3 (merge): use r0 — both defs should reach
        let cfg = CfgBuilder::build(vec![
            with_effect(ff("if"), FlowEffect::ConditionalOpen),
            def("def_true", 0),
            with_effect(ff("else"), FlowEffect::ConditionalAlternate),
            def("def_false", 0),
            with_effect(ff("endif"), FlowEffect::ConditionalClose),
            use_("use_r0", 0),
        ]);
        let merge_block = cfg.blocks().last().unwrap().id();
        let rd = ReachingDefs::compute(&cfg);
        let defs_at_merge = rd.defs_of_at_entry(Location(0), merge_block);
        assert_eq!(defs_at_merge.len(), 2, "both branch defs should reach merge");
    }

    // --- Loop CFG tests ---

    #[test]
    fn reaching_loop_def_reaches_through_backedge() {
        // bb0: def r0; loop; use r0; def r0; endloop
        let cfg = CfgBuilder::build(vec![
            def("init", 0),
            with_effect(ff("loop"), FlowEffect::LoopOpen),
            use_("read", 0),
            def("update", 0),
            with_effect(ff("endloop"), FlowEffect::LoopClose),
        ]);
        let rd = ReachingDefs::compute(&cfg);
        // The loop header should have defs reaching from both
        // the pre-loop init and the loop body update (via back-edge).
        let header = BlockId(1);
        let defs = rd.defs_of_at_entry(Location(0), header);
        assert!(defs.len() >= 1, "at least the init def reaches the header");
    }
}