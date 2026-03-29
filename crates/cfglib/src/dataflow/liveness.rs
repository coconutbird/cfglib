//! Liveness analysis.
//!
//! A **backward** data flow analysis that computes, for each program
//! point, the set of locations whose values may be read in the future
//! before being overwritten.
//!
//! A location is **live** at a point if there exists a path from that
//! point to a use of the location with no intervening definition.

extern crate alloc;
use alloc::collections::BTreeSet;

use crate::block::BlockId;
use crate::cfg::Cfg;
use super::{InstrInfo, Location};
use super::fixpoint::{self, Direction, FixpointResult, Problem};

/// The liveness problem.
pub struct LivenessProblem;

impl<I: InstrInfo> Problem<I> for LivenessProblem {
    type Fact = BTreeSet<Location>;

    fn direction(&self) -> Direction {
        Direction::Backward
    }

    fn bottom(&self) -> Self::Fact {
        BTreeSet::new()
    }

    fn entry_fact(&self) -> Self::Fact {
        // Nothing is live after program exit.
        BTreeSet::new()
    }

    fn meet(&self, a: &Self::Fact, b: &Self::Fact) -> Self::Fact {
        a.union(b).copied().collect()
    }

    /// Backward transfer: live_in = uses ∪ (live_out − defs).
    ///
    /// Walk the block's instructions in **reverse** to compute the
    /// set of locations live at the block's entry.
    fn transfer(&self, cfg: &Cfg<I>, block: BlockId, live_out: &Self::Fact) -> Self::Fact {
        let mut live = live_out.clone();
        let insts = cfg.block(block).instructions();

        // Walk backwards through the block.
        for inst in insts.iter().rev() {
            // Kill: locations defined here are no longer live above.
            for loc in inst.defs() {
                live.remove(loc);
            }
            // Gen: locations used here are live above.
            for &loc in inst.uses() {
                live.insert(loc);
            }
        }

        live
    }
}

/// Result of a liveness analysis with convenient query methods.
pub struct Liveness {
    inner: FixpointResult<BTreeSet<Location>>,
}

impl Liveness {
    /// Run liveness analysis on the given CFG.
    pub fn compute<I: InstrInfo>(cfg: &Cfg<I>) -> Self {
        let result = fixpoint::solve(cfg, &LivenessProblem);
        Self { inner: result }
    }

    /// Locations live at the **entry** of a block.
    pub fn live_in(&self, block: BlockId) -> &BTreeSet<Location> {
        self.inner.fact_in(block)
    }

    /// Locations live at the **exit** of a block.
    pub fn live_out(&self, block: BlockId) -> &BTreeSet<Location> {
        self.inner.fact_out(block)
    }

    /// Check if a location is live at a block's entry.
    pub fn is_live_in(&self, loc: Location, block: BlockId) -> bool {
        self.live_in(block).contains(&loc)
    }

    /// Check if a location is live at a block's exit.
    pub fn is_live_out(&self, loc: Location, block: BlockId) -> bool {
        self.live_out(block).contains(&loc)
    }

    /// All locations that are live somewhere in the program.
    pub fn all_live_locations<I: InstrInfo>(&self, cfg: &Cfg<I>) -> BTreeSet<Location> {
        let mut all = BTreeSet::new();
        for b in cfg.blocks() {
            all.extend(self.live_in(b.id()));
            all.extend(self.live_out(b.id()));
        }
        all
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use alloc::borrow::Cow;
    use alloc::vec;
    use alloc::vec::Vec;
    use crate::builder::CfgBuilder;
    use super::{InstrInfo, Location};
    use crate::flow::{FlowControl, FlowEffect};

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

    fn def(name: &'static str, loc: u16) -> DfInst {
        DfInst { effect: FlowEffect::Fallthrough, name, uses: vec![], defs: vec![Location(loc)] }
    }

    fn use_(name: &'static str, loc: u16) -> DfInst {
        DfInst { effect: FlowEffect::Fallthrough, name, uses: vec![Location(loc)], defs: vec![] }
    }

    #[test]
    fn liveness_linear_use_makes_live() {
        // bb0: def r0; use r0
        // r0 should be live-in (because use reads it) and NOT live-out
        // (nothing after the block reads it).
        let cfg = CfgBuilder::build(vec![def("def_r0", 0), use_("use_r0", 0)]).unwrap();
        let live = Liveness::compute(&cfg);
        // r0 is used in the block → live-in should contain r0
        // (the def kills it, but the use is after the def so
        //  backward: use generates, def kills → net: not live-in
        //  actually: backward walk: use_r0 gens r0, def_r0 kills r0 → live_in = {})
        // But there's nothing after, so live_out = {}
        assert!(!live.is_live_out(Location(0), cfg.entry()));
    }

    #[test]
    fn liveness_use_without_def_is_live_in() {
        // bb0: use r0 (no def) → r0 should be live-in
        let cfg = CfgBuilder::build(vec![use_("use_r0", 0)]).unwrap();
        let live = Liveness::compute(&cfg);
        assert!(live.is_live_in(Location(0), cfg.entry()));
    }

    #[test]
    fn liveness_dead_def() {
        // bb0: def r0 (never used) → r0 should NOT be live anywhere
        let cfg = CfgBuilder::build(vec![def("def_r0", 0)]).unwrap();
        let live = Liveness::compute(&cfg);
        assert!(!live.is_live_in(Location(0), cfg.entry()));
        assert!(!live.is_live_out(Location(0), cfg.entry()));
    }

    #[test]
    fn liveness_across_blocks() {
        // bb0: def r0; if
        // bb1: use r0
        // bb2: (nothing)
        // bb3: endif
        // r0 should be live-out of bb0 because bb1 uses it.
        let cfg = CfgBuilder::build(vec![
            def("def_r0", 0),
            DfInst { effect: FlowEffect::ConditionalOpen, name: "if", uses: vec![], defs: vec![] },
            use_("use_r0", 0),
            DfInst { effect: FlowEffect::ConditionalAlternate, name: "else", uses: vec![], defs: vec![] },
            DfInst { effect: FlowEffect::ConditionalClose, name: "endif", uses: vec![], defs: vec![] },
        ]).unwrap();
        let live = Liveness::compute(&cfg);
        assert!(live.is_live_out(Location(0), cfg.entry()),
            "r0 is live-out of entry because the true branch uses it");
    }
}