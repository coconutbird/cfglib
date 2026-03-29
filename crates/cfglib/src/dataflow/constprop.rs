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
