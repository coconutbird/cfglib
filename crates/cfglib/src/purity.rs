//! Purity classification.
//!
//! Determines whether a block or entire CFG is **pure** (no observable
//! side effects) or **impure** based on the instruction-level side
//! effect declarations.
//!
//! An instruction type implements [`SideEffects`] to declare whether
//! it touches memory, I/O, or other global state beyond its explicit
//! def/use set.

extern crate alloc;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::dataflow::InstrInfo;

/// Categories of side effects an instruction may have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Effect {
    /// Reads from memory / global state.
    MemoryRead,
    /// Writes to memory / global state.
    MemoryWrite,
    /// Performs I/O (texture sample, UAV write, etc.).
    Io,
    /// Calls an external / unknown function.
    Call,
    /// Any other unclassified side effect.
    Other,
}

/// Purity verdict for a block or CFG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Purity {
    /// No side effects at all.
    Pure,
    /// Has side effects — carries the set of observed effect kinds.
    Impure(Vec<Effect>),
}

impl Purity {
    /// Returns `true` if pure.
    pub fn is_pure(&self) -> bool {
        matches!(self, Purity::Pure)
    }

    /// Returns `true` if impure.
    pub fn is_impure(&self) -> bool {
        !self.is_pure()
    }
}

/// Analyse purity of a single block.
pub fn block_purity<I: InstrInfo>(cfg: &Cfg<I>, block: BlockId) -> Purity {
    let mut all = Vec::new();
    for inst in cfg.block(block).instructions() {
        all.extend_from_slice(inst.effects());
    }
    if all.is_empty() {
        Purity::Pure
    } else {
        all.sort();
        all.dedup();
        Purity::Impure(all)
    }
}

/// Analyse purity of the entire CFG.
pub fn cfg_purity<I: InstrInfo>(cfg: &Cfg<I>) -> Purity {
    let mut all = Vec::new();
    for b in cfg.blocks() {
        for inst in b.instructions() {
            all.extend_from_slice(inst.effects());
        }
    }
    if all.is_empty() {
        Purity::Pure
    } else {
        all.sort();
        all.dedup();
        Purity::Impure(all)
    }
}

/// Collect per-block purity for every block in the CFG.
pub fn all_block_purities<I: InstrInfo>(cfg: &Cfg<I>) -> Vec<(BlockId, Purity)> {
    cfg.blocks()
        .iter()
        .map(|b| (b.id(), block_purity(cfg, b.id())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::CfgBuilder;
    use crate::flow::FlowEffect;
    use crate::test_util::{df_impure as impure, df_pure as pure, df_with_effect};
    use alloc::vec;

    #[test]
    fn pure_cfg() {
        let cfg = CfgBuilder::build(vec![pure("add"), pure("mul")]).unwrap();
        assert!(cfg_purity(&cfg).is_pure());
    }

    #[test]
    fn impure_cfg() {
        let cfg =
            CfgBuilder::build(vec![pure("add"), impure("store", Effect::MemoryWrite)]).unwrap();
        let p = cfg_purity(&cfg);
        assert!(p.is_impure());
        if let Purity::Impure(effs) = p {
            assert!(effs.contains(&Effect::MemoryWrite));
        }
    }

    #[test]
    fn mixed_block_purity() {
        let cfg = CfgBuilder::build(vec![
            pure("add"),
            df_with_effect(pure("if"), FlowEffect::ConditionalOpen),
            impure("store", Effect::MemoryWrite),
            df_with_effect(pure("else"), FlowEffect::ConditionalAlternate),
            pure("nop"),
            df_with_effect(pure("endif"), FlowEffect::ConditionalClose),
        ])
        .unwrap();
        // Entry block (has "add") should be pure.
        assert!(block_purity(&cfg, cfg.entry()).is_pure());
        // The whole CFG is impure because one branch stores.
        assert!(cfg_purity(&cfg).is_impure());
    }
}
