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

/// Trait that an instruction type implements to declare its side
/// effects beyond simple register defs/uses.
///
/// If an instruction has **no** effects, the CFG region containing
/// only such instructions is considered *pure*.
pub trait SideEffects {
    /// Return the list of side effects this instruction causes.
    /// An empty vec means the instruction is pure.
    fn effects(&self) -> Vec<Effect>;
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
pub fn block_purity<I: SideEffects>(cfg: &Cfg<I>, block: BlockId) -> Purity {
    let mut all = Vec::new();
    for inst in cfg.block(block).instructions() {
        all.extend(inst.effects());
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
pub fn cfg_purity<I: SideEffects>(cfg: &Cfg<I>) -> Purity {
    let mut all = Vec::new();
    for b in cfg.blocks() {
        for inst in b.instructions() {
            all.extend(inst.effects());
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
pub fn all_block_purities<I: SideEffects>(cfg: &Cfg<I>) -> Vec<(BlockId, Purity)> {
    cfg.blocks()
        .iter()
        .map(|b| (b.id(), block_purity(cfg, b.id())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::borrow::Cow;
    use alloc::vec;
    use crate::builder::CfgBuilder;
    use crate::flow::{FlowControl, FlowEffect};

    #[derive(Debug, Clone)]
    struct PInst {
        effect: FlowEffect,
        name: &'static str,
        side: Vec<Effect>,
    }

    impl FlowControl for PInst {
        fn flow_effect(&self) -> FlowEffect { self.effect }
        fn display_mnemonic(&self) -> Cow<'_, str> { Cow::Borrowed(self.name) }
    }

    impl SideEffects for PInst {
        fn effects(&self) -> Vec<Effect> { self.side.clone() }
    }

    fn pure(name: &'static str) -> PInst {
        PInst { effect: FlowEffect::Fallthrough, name, side: vec![] }
    }

    fn impure(name: &'static str, e: Effect) -> PInst {
        PInst { effect: FlowEffect::Fallthrough, name, side: vec![e] }
    }

    #[test]
    fn pure_cfg() {
        let cfg = CfgBuilder::build(vec![pure("add"), pure("mul")]);
        assert!(cfg_purity(&cfg).is_pure());
    }

    #[test]
    fn impure_cfg() {
        let cfg = CfgBuilder::build(vec![pure("add"), impure("store", Effect::MemoryWrite)]);
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
            PInst { effect: FlowEffect::ConditionalOpen, name: "if", side: vec![] },
            impure("store", Effect::MemoryWrite),
            PInst { effect: FlowEffect::ConditionalAlternate, name: "else", side: vec![] },
            pure("nop"),
            PInst { effect: FlowEffect::ConditionalClose, name: "endif", side: vec![] },
        ]);
        // Entry block (has "add") should be pure.
        assert!(block_purity(&cfg, cfg.entry()).is_pure());
        // The whole CFG is impure because one branch stores.
        assert!(cfg_purity(&cfg).is_impure());
    }
}
