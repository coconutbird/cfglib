//! Sparse Conditional Constant Propagation (SCCP).
//!
//! SCCP operates on the generic renamed values in [`SsaForm`] while asking
//! the source instruction adapter to fold native instructions.

extern crate alloc;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::dataflow::VariableId;
use crate::dataflow::constprop::{ConstValue, ConstantFolder};
use crate::dataflow::ssa::{SsaForm, SsaValue};

/// Result of SCCP analysis.
#[derive(Debug, Clone)]
pub struct SccpResult<V> {
    /// Lattice value computed for each renamed SSA value.
    pub values: BTreeMap<SsaValue<V>, ConstValue>,
    /// CFG edges proven executable.
    pub executable_edges: BTreeSet<(BlockId, BlockId)>,
    /// CFG blocks proven reachable.
    pub reachable_blocks: BTreeSet<BlockId>,
}

fn update_value<V: VariableId>(
    values: &mut BTreeMap<SsaValue<V>, ConstValue>,
    worklist: &mut Vec<SsaValue<V>>,
    value: &SsaValue<V>,
    candidate: ConstValue,
) {
    let previous = values.get(value).copied().unwrap_or(ConstValue::Top);
    let next = previous.meet(candidate);
    if next != previous {
        values.insert(value.clone(), next);
        worklist.push(value.clone());
    }
}

fn evaluate_block<I: ConstantFolder>(
    cfg: &Cfg<I>,
    ssa: &SsaForm<I::Variable>,
    block: BlockId,
    values: &mut BTreeMap<SsaValue<I::Variable>, ConstValue>,
    worklist: &mut Vec<SsaValue<I::Variable>>,
) {
    for (instruction, annotation) in cfg
        .block(block)
        .instructions()
        .iter()
        .zip(&ssa.block(block).instructions)
    {
        let known: BTreeMap<I::Variable, i64> = annotation
            .uses
            .iter()
            .filter_map(|value| {
                values
                    .get(value)
                    .copied()
                    .and_then(ConstValue::as_const)
                    .map(|constant| (value.variable.clone(), constant))
            })
            .collect();

        if let Some((variable, constant)) = instruction.fold_constant(&known) {
            if let Some(definition) = annotation
                .defs
                .iter()
                .find(|definition| definition.variable == variable)
            {
                update_value(values, worklist, definition, ConstValue::Const(constant));
            }
        } else {
            for definition in &annotation.defs {
                update_value(values, worklist, definition, ConstValue::Bottom);
            }
        }
    }
}

/// Run sparse conditional constant propagation over a renamed SSA form.
///
/// `ssa` must have been built from `cfg`. The current control-flow adapter
/// exposes reachability but not branch predicates, so SCCP conservatively marks
/// every successor of a reachable block executable.
#[must_use]
pub fn sccp<I: ConstantFolder>(
    cfg: &Cfg<I>,
    ssa: &SsaForm<I::Variable>,
) -> SccpResult<I::Variable> {
    let mut values = BTreeMap::new();
    let mut executable_edges = BTreeSet::new();
    let mut reachable_blocks = BTreeSet::new();
    let mut cfg_worklist = Vec::new();
    let mut ssa_worklist = Vec::new();

    reachable_blocks.insert(cfg.entry());
    cfg_worklist.extend(
        cfg.successors(cfg.entry())
            .map(|target| (cfg.entry(), target)),
    );
    evaluate_block(cfg, ssa, cfg.entry(), &mut values, &mut ssa_worklist);

    while !cfg_worklist.is_empty() || !ssa_worklist.is_empty() {
        while let Some((source, target)) = cfg_worklist.pop() {
            if !executable_edges.insert((source, target)) {
                continue;
            }

            let newly_reachable = reachable_blocks.insert(target);
            for phi in &ssa.block(target).phis {
                let mut candidate = ConstValue::Top;
                for (predecessor, operand) in &phi.operands {
                    if executable_edges.contains(&(*predecessor, target)) {
                        candidate =
                            candidate.meet(values.get(operand).copied().unwrap_or(ConstValue::Top));
                    }
                }
                update_value(&mut values, &mut ssa_worklist, &phi.result, candidate);
            }

            if newly_reachable {
                evaluate_block(cfg, ssa, target, &mut values, &mut ssa_worklist);
                cfg_worklist.extend(cfg.successors(target).map(|next| (target, next)));
            }
        }

        while ssa_worklist.pop().is_some() {
            for &block in &reachable_blocks {
                evaluate_block(cfg, ssa, block, &mut values, &mut ssa_worklist);
            }
        }
    }

    SccpResult {
        values,
        executable_edges,
        reachable_blocks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataflow::ssa::build_ssa;
    use crate::edge::EdgeKind;
    use crate::graph::dominator::DominatorTree;
    use crate::test_util::{DfInst, df_const, df_def, df_use};

    fn analyse(cfg: &Cfg<DfInst>) -> SccpResult<u16> {
        let dom = DominatorTree::compute(cfg);
        let ssa = build_ssa(cfg, &dom);
        sccp(cfg, &ssa)
    }

    #[test]
    fn entry_is_reachable() {
        let cfg = Cfg::<DfInst>::new();
        assert!(analyse(&cfg).reachable_blocks.contains(&cfg.entry()));
    }

    #[test]
    fn linear_cfg_is_reachable() {
        let mut cfg = Cfg::<DfInst>::new();
        let next = cfg.new_block();
        cfg.block_mut(cfg.entry()).push(df_def("def", 0));
        cfg.block_mut(next).push(df_use("use", 0));
        cfg.add_edge(cfg.entry(), next, EdgeKind::Fallthrough);
        assert!(analyse(&cfg).reachable_blocks.contains(&next));
    }

    #[test]
    fn constants_are_keyed_by_ssa_value() {
        let mut cfg = Cfg::<DfInst>::new();
        cfg.block_mut(cfg.entry()).push(df_const("constant", 0, 42));
        let dom = DominatorTree::compute(&cfg);
        let ssa = build_ssa(&cfg, &dom);
        let definition = ssa.block(cfg.entry()).instructions[0].defs[0].clone();
        let result = sccp(&cfg, &ssa);
        assert_eq!(result.values[&definition], ConstValue::Const(42));
    }

    #[test]
    fn unreachable_block_is_excluded() {
        let mut cfg = Cfg::<DfInst>::new();
        let reachable = cfg.new_block();
        let unreachable = cfg.new_block();
        cfg.add_edge(cfg.entry(), reachable, EdgeKind::Fallthrough);
        let result = analyse(&cfg);
        assert!(result.reachable_blocks.contains(&reachable));
        assert!(!result.reachable_blocks.contains(&unreachable));
    }
}
