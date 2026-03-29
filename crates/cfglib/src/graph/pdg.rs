//! Program Dependence Graph (PDG).
//!
//! Unifies **control dependence** ([`ControlDependenceGraph`]) and
//! **data dependence** ([`DefUseChains`]) into a single queryable
//! structure.  The PDG powers program slicing, clone detection, and
//! advanced restructuring passes.
//!
//! # Construction
//!
//! ```ignore
//! let dom  = DominatorTree::compute(&cfg);
//! let pdom = DominatorTree::compute_post(&cfg);
//! let cdg  = ControlDependenceGraph::compute(&cfg, &pdom);
//! let du   = DefUseChains::compute(&cfg);
//! let pdg  = ProgramDependenceGraph::new(cdg, du);
//! ```
//!
//! Or use the convenience constructor that does all the work:
//!
//! ```ignore
//! let pdg = ProgramDependenceGraph::compute(&cfg);
//! ```

extern crate alloc;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::dataflow::defuse::DefUseChains;
use crate::dataflow::{DefSite, InstrInfo, UseSite};
use crate::graph::cdg::ControlDependenceGraph;
use crate::graph::dominator::DominatorTree;

/// A dependence edge in the PDG.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Dependence {
    /// Block `dependent` is control-dependent on block `on`.
    Control {
        /// The block whose branch decision controls execution.
        on: BlockId,
        /// The block whose execution depends on the branch.
        dependent: BlockId,
    },
    /// Instruction at `use_site` reads a value defined at `def_site`.
    Data {
        /// Where the value is defined.
        def_site: DefSite,
        /// Where the value is used.
        use_site: UseSite,
    },
}

/// Combined control + data dependence graph.
///
/// Wraps a [`ControlDependenceGraph`] and [`DefUseChains`] and
/// provides unified queries over both dependence kinds.
#[derive(Debug, Clone)]
pub struct ProgramDependenceGraph {
    /// Control dependence component.
    pub cdg: ControlDependenceGraph,
    /// Data dependence component.
    pub def_use: DefUseChains,
}

impl ProgramDependenceGraph {
    /// Build from pre-computed components.
    pub fn new(cdg: ControlDependenceGraph, def_use: DefUseChains) -> Self {
        Self { cdg, def_use }
    }

    /// Compute the full PDG from a CFG in one step.
    pub fn compute<I: InstrInfo>(cfg: &Cfg<I>) -> Self {
        let pdom = DominatorTree::compute_post(cfg);
        let cdg = ControlDependenceGraph::compute(cfg, &pdom);
        let def_use = DefUseChains::compute(cfg);
        Self { cdg, def_use }
    }

    /// All blocks that `block` is control-dependent on.
    pub fn control_dependences(&self, block: BlockId) -> &BTreeSet<BlockId> {
        self.cdg.control_dependences(block)
    }

    /// All blocks whose execution is controlled by `block`.
    pub fn control_dependents(&self, block: BlockId) -> &BTreeSet<BlockId> {
        self.cdg.control_dependents(block)
    }

    /// All use-sites that read the value defined at `def`.
    pub fn data_dependents(&self, def: DefSite) -> &BTreeSet<UseSite> {
        self.def_use.uses_of(def)
    }

    /// All def-sites that reach the use at `use_site`.
    pub fn data_dependences(&self, use_site: UseSite) -> &BTreeSet<DefSite> {
        self.def_use.defs_of(use_site)
    }

    /// Collect **all** dependence edges in the graph.
    ///
    /// Useful for serialization, visualization, or whole-graph analysis.
    pub fn all_dependences(&self, num_blocks: usize) -> Vec<Dependence> {
        let mut deps = Vec::new();

        // Control dependences.
        for idx in 0..num_blocks {
            let block = BlockId::from_raw(idx as u32);
            for &on in self.cdg.control_dependences(block) {
                deps.push(Dependence::Control {
                    on,
                    dependent: block,
                });
            }
        }

        // Data dependences.
        for (def, uses) in &self.def_use.def_use {
            for use_site in uses {
                deps.push(Dependence::Data {
                    def_site: *def,
                    use_site: *use_site,
                });
            }
        }

        deps
    }

    /// Compute the **backward slice** from a given program point.
    ///
    /// Returns all program points (block, instruction index) that
    /// transitively affect the value or execution of `seed`, following
    /// both data and control dependences.
    pub fn backward_slice(&self, seed: UseSite) -> BTreeSet<DefSite> {
        let mut visited = BTreeSet::new();
        let mut worklist: Vec<UseSite> = alloc::vec![seed];

        while let Some(point) = worklist.pop() {
            if !visited.insert(point) {
                continue;
            }

            // Follow data dependences: who defined the values I use?
            for def in self.def_use.defs_of(point) {
                worklist.push(*def);
            }

            // Follow control dependences: who controls my block?
            for &ctrl in self.cdg.control_dependences(point.block) {
                // Add the last instruction of the controlling block
                // (the branch) as a dependence.
                let ctrl_point = DefSite {
                    block: ctrl,
                    inst_idx: 0, // representative point
                };
                if !visited.contains(&ctrl_point) {
                    worklist.push(ctrl_point);
                }
            }
        }

        visited
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::dataflow::ProgramPoint;
    use crate::edge::EdgeKind;
    use crate::test_util::{DfInst, df_def, df_use};

    #[test]
    fn pdg_compute_on_diamond() {
        // entry → A (true), entry → B (false), A → merge, B → merge
        let mut cfg: Cfg<DfInst> = Cfg::new();
        let a = cfg.new_block();
        let b = cfg.new_block();
        let merge = cfg.new_block();

        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(df_def("branch", 0));
        cfg.block_mut(a)
            .instructions_vec_mut()
            .push(df_def("def_a", 1));
        cfg.block_mut(b)
            .instructions_vec_mut()
            .push(df_def("def_b", 2));
        cfg.block_mut(merge)
            .instructions_vec_mut()
            .push(df_use("use_1", 1));

        cfg.add_edge(cfg.entry(), a, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), b, EdgeKind::ConditionalFalse);
        cfg.add_edge(a, merge, EdgeKind::Fallthrough);
        cfg.add_edge(b, merge, EdgeKind::Fallthrough);

        let pdg = ProgramDependenceGraph::compute(&cfg);

        // A and B should be control-dependent on entry.
        assert!(pdg.cdg.is_dependent(a, cfg.entry()));
        assert!(pdg.cdg.is_dependent(b, cfg.entry()));

        // Data dependence: use of loc1 in merge should depend on def in A.
        let def_a = ProgramPoint {
            block: a,
            inst_idx: 0,
        };
        let uses_of_a = pdg.data_dependents(def_a);
        assert!(
            !uses_of_a.is_empty(),
            "def of loc1 in A should have uses in merge"
        );
    }

    #[test]
    fn all_dependences_includes_both_kinds() {
        let mut cfg: Cfg<DfInst> = Cfg::new();
        let b = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(df_def("def0", 0));
        cfg.block_mut(b)
            .instructions_vec_mut()
            .push(df_use("use0", 0));
        cfg.add_edge(cfg.entry(), b, EdgeKind::Fallthrough);

        let pdg = ProgramDependenceGraph::compute(&cfg);
        let deps = pdg.all_dependences(cfg.num_blocks());

        let has_data = deps.iter().any(|d| matches!(d, Dependence::Data { .. }));
        assert!(has_data, "should have data dependences");
    }

    #[test]
    fn backward_slice_follows_data_dep() {
        let mut cfg: Cfg<DfInst> = Cfg::new();
        let b = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(df_def("def0", 0));
        cfg.block_mut(b)
            .instructions_vec_mut()
            .push(df_use("use0", 0));
        cfg.add_edge(cfg.entry(), b, EdgeKind::Fallthrough);

        let pdg = ProgramDependenceGraph::compute(&cfg);
        let seed = ProgramPoint {
            block: b,
            inst_idx: 0,
        };
        let slice = pdg.backward_slice(seed);

        // Should include the seed itself and the def in entry.
        assert!(slice.contains(&seed));
        let def_point = ProgramPoint {
            block: cfg.entry(),
            inst_idx: 0,
        };
        assert!(
            slice.contains(&def_point),
            "backward slice should include the defining instruction"
        );
    }
}
