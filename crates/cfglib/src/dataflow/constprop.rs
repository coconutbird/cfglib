//! Constant propagation analysis.
//!
//! A **forward** dataflow analysis that tracks which locations hold
//! known constant values. Uses a simple three-level lattice per
//! location: `Top` (unknown/uninitialized) → `Const(i64)` → `Bottom`
//! (overdefined / multiple conflicting values).
//!
//! This serves as a proof-of-concept for the generic `Problem` trait
//! and can be extended with ISA-specific constant folding.

extern crate alloc;
use alloc::collections::BTreeMap;

use super::fixpoint::{self, Direction, FixpointResult, Problem};
use super::{InstrInfo, Location};
use crate::block::BlockId;
use crate::cfg::Cfg;

/// The lattice value for a single location.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConstValue {
    /// Not yet analyzed (top of lattice).
    Top,
    /// Known constant.
    Const(i64),
    /// Overdefined — seen different values on different paths.
    Bottom,
}

impl ConstValue {
    /// Meet (join) two lattice values.
    ///
    /// - Top ⊓ x = x
    /// - x ⊓ Top = x
    /// - Const(a) ⊓ Const(a) = Const(a)
    /// - Const(a) ⊓ Const(b) = Bottom  (a ≠ b)
    /// - Bottom ⊓ x = Bottom
    pub fn meet(self, other: Self) -> Self {
        match (self, other) {
            (ConstValue::Top, x) | (x, ConstValue::Top) => x,
            (ConstValue::Const(a), ConstValue::Const(b)) if a == b => ConstValue::Const(a),
            _ => ConstValue::Bottom,
        }
    }

    /// Whether this is a known constant.
    pub fn is_const(self) -> bool {
        matches!(self, ConstValue::Const(_))
    }

    /// Extract the constant value, if any.
    pub fn as_const(self) -> Option<i64> {
        match self {
            ConstValue::Const(v) => Some(v),
            _ => None,
        }
    }
}

/// Trait for instructions that can produce constant values.
///
/// This is the ISA-specific bridge: the consumer implements this
/// to tell the analysis what constant, if any, an instruction
/// produces given known-constant inputs.
pub trait ConstantFolder: InstrInfo {
    /// If this instruction produces a constant for a defined location
    /// given the current known constants, return `Some((loc, value))`.
    ///
    /// `known` maps locations to their known constant values (only
    /// entries with `Const(v)` are present).
    ///
    /// Return `None` to leave the default behavior (mark all defs as
    /// Bottom).
    fn fold_constant(&self, known: &BTreeMap<Location, i64>) -> Option<(Location, i64)>;
}

/// The constant propagation problem.
pub struct ConstPropProblem;

/// The flow fact: a map from location to lattice value.
pub type ConstFact = BTreeMap<Location, ConstValue>;

impl<I: ConstantFolder> Problem<I> for ConstPropProblem {
    type Fact = ConstFact;

    fn direction(&self) -> Direction {
        Direction::Forward
    }

    fn bottom(&self) -> Self::Fact {
        BTreeMap::new()
    }

    fn entry_fact(&self) -> Self::Fact {
        BTreeMap::new()
    }

    fn meet(&self, a: &Self::Fact, b: &Self::Fact) -> Self::Fact {
        let mut result = a.clone();
        for (&loc, &val) in b {
            let entry = result.entry(loc).or_insert(ConstValue::Top);
            *entry = entry.meet(val);
        }
        result
    }

    fn transfer(&self, cfg: &Cfg<I>, block: BlockId, input: &Self::Fact) -> Self::Fact {
        let mut state = input.clone();

        for inst in cfg.block(block).instructions() {
            // Build known-constants map for the folder.
            let known: BTreeMap<Location, i64> = state
                .iter()
                .filter_map(|(&loc, &val)| val.as_const().map(|v| (loc, v)))
                .collect();

            // Try constant folding.
            if let Some((loc, val)) = inst.fold_constant(&known) {
                state.insert(loc, ConstValue::Const(val));
            } else {
                // Default: all defs become Bottom (non-constant).
                for &d in inst.defs() {
                    state.insert(d, ConstValue::Bottom);
                }
            }
        }

        state
    }
}

/// Run constant propagation on the CFG.
pub fn constant_propagation<I: ConstantFolder>(cfg: &Cfg<I>) -> FixpointResult<ConstFact> {
    fixpoint::solve(cfg, &ConstPropProblem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::edge::EdgeKind;
    use crate::test_util::{DfInst, df_const, df_def, df_use};

    #[test]
    fn meet_top_with_const() {
        assert_eq!(
            ConstValue::Top.meet(ConstValue::Const(42)),
            ConstValue::Const(42)
        );
        assert_eq!(
            ConstValue::Const(42).meet(ConstValue::Top),
            ConstValue::Const(42)
        );
    }

    #[test]
    fn meet_same_const() {
        assert_eq!(
            ConstValue::Const(7).meet(ConstValue::Const(7)),
            ConstValue::Const(7)
        );
    }

    #[test]
    fn meet_different_consts_is_bottom() {
        assert_eq!(
            ConstValue::Const(1).meet(ConstValue::Const(2)),
            ConstValue::Bottom
        );
    }

    #[test]
    fn meet_bottom_absorbs() {
        assert_eq!(
            ConstValue::Bottom.meet(ConstValue::Const(5)),
            ConstValue::Bottom
        );
        assert_eq!(
            ConstValue::Const(5).meet(ConstValue::Bottom),
            ConstValue::Bottom
        );
        assert_eq!(ConstValue::Bottom.meet(ConstValue::Top), ConstValue::Bottom);
    }

    #[test]
    fn is_const_and_as_const() {
        assert!(ConstValue::Const(1).is_const());
        assert_eq!(ConstValue::Const(1).as_const(), Some(1));
        assert!(!ConstValue::Top.is_const());
        assert_eq!(ConstValue::Top.as_const(), None);
        assert!(!ConstValue::Bottom.is_const());
        assert_eq!(ConstValue::Bottom.as_const(), None);
    }

    #[test]
    fn constant_propagation_tracks_const_def() {
        let mut cfg: Cfg<DfInst> = Cfg::new();
        let exit = cfg.new_block();
        // Entry: const 42 → loc0, then use loc0
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .extend([df_const("load_42", 0, 42)]);
        cfg.block_mut(exit)
            .instructions_vec_mut()
            .push(df_use("use0", 0));
        cfg.add_edge(cfg.entry(), exit, EdgeKind::Fallthrough);

        let result = constant_propagation(&cfg);
        let fact_out = result.fact_out(cfg.entry());
        let loc0 = crate::dataflow::Location(0);
        assert_eq!(fact_out.get(&loc0), Some(&ConstValue::Const(42)));
    }

    #[test]
    fn constant_propagation_non_const_def_is_bottom() {
        let mut cfg: Cfg<DfInst> = Cfg::new();
        // Entry: def loc0 (non-constant)
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(df_def("generic_def", 0));

        let result = constant_propagation(&cfg);
        let fact_out = result.fact_out(cfg.entry());
        let loc0 = crate::dataflow::Location(0);
        assert_eq!(fact_out.get(&loc0), Some(&ConstValue::Bottom));
    }
}
