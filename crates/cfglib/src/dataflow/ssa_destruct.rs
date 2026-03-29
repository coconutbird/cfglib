//! SSA destruction (phi elimination).
//!
//! Converts out of SSA form by replacing phi nodes with parallel copy
//! instructions placed on incoming edges. This is the inverse of
//! [`insert_phis`](super::ssa::insert_phis).
//!
//! The consumer is responsible for inserting the actual copy instructions
//! into the CFG; this module computes **what** copies are needed and
//! **where** they should go.

extern crate alloc;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::dataflow::Location;
use crate::dataflow::ssa::PhiMap;

/// A copy to be inserted on a specific CFG edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhiCopy {
    /// The predecessor block (source of the edge).
    pub from_block: BlockId,
    /// The block containing the phi node (target of the edge).
    pub to_block: BlockId,
    /// The destination location (the phi's target variable).
    pub dst: Location,
    /// The source location (the operand from `from_block`).
    pub src: Location,
}

/// Compute the copies needed to eliminate all phi nodes.
///
/// For each phi node at a merge point, generates a [`PhiCopy`] for
/// every incoming edge. The copies should be placed at the **end** of
/// the predecessor block (or on a critical edge split block).
///
/// # Example
///
/// ```ignore
/// let dom = DominatorTree::compute(&cfg);
/// let phis = insert_phis(&cfg, &dom);
/// let copies = eliminate_phis(&phis);
/// // Insert copy instructions for each PhiCopy into the CFG.
/// ```
pub fn eliminate_phis(phi_map: &PhiMap) -> Vec<PhiCopy> {
    let mut copies = Vec::new();

    for (block, phi) in phi_map.iter() {
        for &(pred_block, src_loc) in &phi.operands {
            copies.push(PhiCopy {
                from_block: pred_block,
                to_block: block,
                dst: phi.location,
                src: src_loc,
            });
        }
    }

    copies
}

/// Group phi copies by the predecessor block they should be placed in.
///
/// Returns `(predecessor_block, copies)` pairs. Copies within each group
/// form a parallel assignment and may need sequencing if there are
/// circular dependencies.
pub fn copies_by_predecessor(copies: &[PhiCopy]) -> Vec<(BlockId, Vec<&PhiCopy>)> {
    use alloc::collections::BTreeMap;
    let mut map: BTreeMap<BlockId, Vec<&PhiCopy>> = BTreeMap::new();
    for copy in copies {
        map.entry(copy.from_block).or_default().push(copy);
    }
    map.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::dataflow::ssa::insert_phis;
    use crate::edge::EdgeKind;
    use crate::graph::dominator::DominatorTree;
    use crate::test_util::{DfInst, df_def, df_use};

    #[test]
    fn eliminate_phis_on_diamond() {
        // entry(def loc0) → a, entry → b, a → merge(use loc0), b → merge
        let mut cfg: Cfg<DfInst> = Cfg::new();
        let a = cfg.new_block();
        let b = cfg.new_block();
        let merge = cfg.new_block();

        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(df_def("def0", 0));
        cfg.block_mut(a)
            .instructions_vec_mut()
            .push(df_def("def_a", 0));
        cfg.block_mut(b)
            .instructions_vec_mut()
            .push(df_def("def_b", 0));
        cfg.block_mut(merge)
            .instructions_vec_mut()
            .push(df_use("use0", 0));

        cfg.add_edge(cfg.entry(), a, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), b, EdgeKind::ConditionalFalse);
        cfg.add_edge(a, merge, EdgeKind::Fallthrough);
        cfg.add_edge(b, merge, EdgeKind::Fallthrough);

        let dom = DominatorTree::compute(&cfg);
        let phis = insert_phis(&cfg, &dom);
        let copies = eliminate_phis(&phis);

        // Phi at merge for loc0 should generate copies from a and b.
        let merge_copies: Vec<_> = copies.iter().filter(|c| c.to_block == merge).collect();
        assert!(
            !merge_copies.is_empty(),
            "phi at merge should generate copies"
        );
    }

    #[test]
    fn copies_grouped_by_predecessor() {
        let mut cfg: Cfg<DfInst> = Cfg::new();
        let a = cfg.new_block();
        let b = cfg.new_block();
        let merge = cfg.new_block();

        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(df_def("def0", 0));
        cfg.block_mut(a)
            .instructions_vec_mut()
            .push(df_def("def_a", 0));
        cfg.block_mut(b)
            .instructions_vec_mut()
            .push(df_def("def_b", 0));
        cfg.block_mut(merge)
            .instructions_vec_mut()
            .push(df_use("use0", 0));

        cfg.add_edge(cfg.entry(), a, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), b, EdgeKind::ConditionalFalse);
        cfg.add_edge(a, merge, EdgeKind::Fallthrough);
        cfg.add_edge(b, merge, EdgeKind::Fallthrough);

        let dom = DominatorTree::compute(&cfg);
        let phis = insert_phis(&cfg, &dom);
        let copies = eliminate_phis(&phis);
        let grouped = copies_by_predecessor(&copies);

        // Should have at least one group.
        assert!(!grouped.is_empty());
    }
}
