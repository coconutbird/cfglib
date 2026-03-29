//! Abstract interpretation framework.
//!
//! Generalises the fixpoint engine to work over arbitrary lattices,
//! enabling interval analysis, sign analysis, taint tracking, etc.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;

/// A lattice element for abstract interpretation.
pub trait Lattice: Clone + PartialEq {
    /// Bottom element (least precise / most conservative).
    fn bottom() -> Self;
    /// Top element (most precise / most optimistic).
    fn top() -> Self;
    /// Meet (greatest lower bound) of two elements.
    fn meet(&self, other: &Self) -> Self;
    /// Returns `true` if `self ⊑ other` in the lattice ordering.
    fn leq(&self, other: &Self) -> bool;
}

/// An abstract domain defines how instructions transform lattice values.
pub trait AbstractDomain<I>: Lattice {
    /// Transfer function: transform abstract state after executing
    /// instruction `inst`.
    fn transfer(state: &Self, inst: &I) -> Self;

    /// Initial abstract value for the entry block.
    fn entry_value() -> Self;
}

/// Result of abstract interpretation.
#[derive(Debug, Clone)]
pub struct AbstractResult<L> {
    /// Abstract state at each block entry.
    pub block_in: BTreeMap<BlockId, L>,
    /// Abstract state at each block exit.
    pub block_out: BTreeMap<BlockId, L>,
    /// Number of fixpoint iterations.
    pub iterations: usize,
}

/// Run abstract interpretation over a CFG.
///
/// Forward analysis using a worklist algorithm. The abstract domain `D`
/// determines both the lattice and the per-instruction transfer function.
pub fn abstract_interpret<I, D: AbstractDomain<I>>(cfg: &Cfg<I>) -> AbstractResult<D> {
    let rpo = cfg.reverse_postorder();
    let n = rpo.len();
    let mut block_in: BTreeMap<BlockId, D> = BTreeMap::new();
    let mut block_out: BTreeMap<BlockId, D> = BTreeMap::new();

    // Initialise.
    for &bid in &rpo {
        block_in.insert(bid, D::bottom());
        block_out.insert(bid, D::bottom());
    }
    block_in.insert(cfg.entry(), D::entry_value());

    let mut worklist: Vec<BlockId> = rpo.clone();
    let mut iterations = 0usize;
    let max_iter = n.saturating_mul(20).max(200);

    while let Some(bid) = worklist.pop() {
        iterations += 1;
        if iterations > max_iter {
            break;
        }

        // Compute IN = meet of all predecessors' OUT.
        let mut incoming = D::top();
        let mut has_pred = false;
        for pred in cfg.predecessors(bid) {
            has_pred = true;
            if let Some(out) = block_out.get(&pred) {
                incoming = incoming.meet(out);
            }
        }
        if !has_pred {
            incoming = block_in.get(&bid).cloned().unwrap_or_else(D::bottom);
        }

        // Transfer through all instructions.
        let mut state = incoming.clone();
        for inst in cfg.block(bid).instructions() {
            state = D::transfer(&state, inst);
        }

        let old_out = block_out.get(&bid).cloned().unwrap_or_else(D::bottom);
        if state != old_out {
            block_in.insert(bid, incoming);
            block_out.insert(bid, state);
            // Add successors to worklist.
            for succ in cfg.successors(bid) {
                if !worklist.contains(&succ) {
                    worklist.push(succ);
                }
            }
        }
    }

    AbstractResult {
        block_in,
        block_out,
        iterations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::edge::EdgeKind;
    use crate::test_util::ff;

    /// A trivial sign lattice: Bottom < Neg/Zero/Pos < Top.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Sign {
        Bottom,
        Neg,
        Zero,
        Pos,
        Top,
    }

    impl Lattice for Sign {
        fn bottom() -> Self {
            Sign::Bottom
        }
        fn top() -> Self {
            Sign::Top
        }
        fn meet(&self, other: &Self) -> Self {
            match (self, other) {
                (Sign::Top, x) | (x, Sign::Top) => x.clone(),
                (a, b) if a == b => a.clone(),
                _ => Sign::Bottom,
            }
        }
        fn leq(&self, other: &Self) -> bool {
            matches!(
                (self, other),
                (Sign::Bottom, _)
                    | (_, Sign::Top)
                    | (Sign::Neg, Sign::Neg)
                    | (Sign::Zero, Sign::Zero)
                    | (Sign::Pos, Sign::Pos)
            )
        }
    }

    impl AbstractDomain<crate::test_util::MockInst> for Sign {
        fn transfer(state: &Self, _inst: &crate::test_util::MockInst) -> Self {
            state.clone() // identity transfer for testing
        }
        fn entry_value() -> Self {
            Sign::Zero
        }
    }

    #[test]
    fn sign_lattice_basics() {
        assert_eq!(Sign::Top.meet(&Sign::Neg), Sign::Neg);
        assert_eq!(Sign::Neg.meet(&Sign::Pos), Sign::Bottom);
        assert!(Sign::Bottom.leq(&Sign::Top));
    }

    #[test]
    fn abstract_interpret_linear() {
        let mut cfg = Cfg::new();
        let b = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("a"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.add_edge(cfg.entry(), b, EdgeKind::Fallthrough);
        let result: AbstractResult<Sign> = abstract_interpret(&cfg);
        assert!(result.iterations > 0);
        assert!(result.block_out.contains_key(&cfg.entry()));
    }
}
