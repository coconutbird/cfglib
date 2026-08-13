//! Alias analysis — Steensgaard-style unification-based points-to analysis.
//!
//! Groups memory locations into alias sets using union-find. Two locations
//! are in the same alias set if they may refer to the same memory.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::cfg::Cfg;
use crate::dataflow::{InstrInfo, VariableId};

/// A memory access kind for alias analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryOp {
    /// A load from a memory location.
    Load,
    /// A store to a memory location.
    Store,
}

/// Trait for instructions that access memory.
pub trait MemoryInfo: InstrInfo {
    /// Memory accesses performed by this instruction.
    /// Returns `(base_variable, op)` pairs.
    fn memory_ops(&self) -> &[(Self::Variable, MemoryOp)];
}

/// Union-Find structure for alias set computation.
#[derive(Debug, Clone)]
pub struct AliasSets<V> {
    parent: Vec<usize>,
    rank: Vec<usize>,
    variable_to_id: BTreeMap<V, usize>,
    id_to_variable: Vec<V>,
}

impl<V: VariableId> AliasSets<V> {
    /// Create empty alias sets.
    #[must_use]
    pub fn new() -> Self {
        Self {
            parent: Vec::new(),
            rank: Vec::new(),
            variable_to_id: BTreeMap::new(),
            id_to_variable: Vec::new(),
        }
    }

    fn get_or_insert(&mut self, variable: V) -> usize {
        if let Some(&id) = self.variable_to_id.get(&variable) {
            return id;
        }
        let id = self.parent.len();
        self.parent.push(id);
        self.rank.push(0);
        self.variable_to_id.insert(variable.clone(), id);
        self.id_to_variable.push(variable);
        id
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]]; // path compression
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra == rb {
            return;
        }
        match self.rank[ra].cmp(&self.rank[rb]) {
            Ordering::Less => self.parent[ra] = rb,
            Ordering::Greater => self.parent[rb] = ra,
            Ordering::Equal => {
                self.parent[rb] = ra;
                self.rank[ra] += 1;
            }
        }
    }

    /// Check if two variables may alias.
    pub fn may_alias(&mut self, left: &V, right: &V) -> bool {
        let Some(&left_id) = self.variable_to_id.get(left) else {
            return false;
        };
        let Some(&right_id) = self.variable_to_id.get(right) else {
            return false;
        };
        self.find(left_id) == self.find(right_id)
    }

    /// Get the alias-set representative for a variable.
    pub fn alias_set(&mut self, variable: &V) -> Option<&V> {
        let &id = self.variable_to_id.get(variable)?;
        let representative = self.find(id);
        self.id_to_variable.get(representative)
    }

    /// Merge two variables into the same alias set.
    pub fn merge(&mut self, left: V, right: V) {
        let left_id = self.get_or_insert(left);
        let right_id = self.get_or_insert(right);
        self.union(left_id, right_id);
    }

    /// Number of distinct alias sets.
    pub fn num_sets(&mut self) -> usize {
        let n = self.parent.len();
        let mut roots = alloc::collections::BTreeSet::new();
        for i in 0..n {
            roots.insert(self.find(i));
        }
        roots.len()
    }
}

impl<V: VariableId> Default for AliasSets<V> {
    fn default() -> Self {
        Self::new()
    }
}

/// Run Steensgaard-style alias analysis on a CFG.
///
/// Unifies locations that are stored to/loaded from the same base.
/// This is a flow-insensitive, context-insensitive analysis.
#[must_use]
pub fn alias_analysis<I: MemoryInfo>(cfg: &Cfg<I>) -> AliasSets<I::Variable> {
    let mut sets = AliasSets::new();

    // Register all locations.
    for block in cfg.blocks() {
        for inst in block.instructions() {
            for d in inst.defs() {
                sets.get_or_insert(d.clone());
            }
            for u in inst.uses() {
                sets.get_or_insert(u.clone());
            }
        }
    }

    // Unify locations involved in the same memory operations.
    for block in cfg.blocks() {
        for inst in block.instructions() {
            let ops = inst.memory_ops();
            if ops.len() >= 2 {
                let first = &ops[0].0;
                for (variable, _) in &ops[1..] {
                    sets.merge(first.clone(), variable.clone());
                }
            }
            // Also unify defs with store targets.
            for (memory_variable, op) in ops {
                if *op == MemoryOp::Store {
                    for d in inst.defs() {
                        sets.merge(memory_variable.clone(), d.clone());
                    }
                }
            }
        }
    }

    sets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_creates_alias() {
        let mut sets = AliasSets::new();
        let a = 0_u16;
        let b = 1_u16;
        sets.merge(a, b);
        assert!(sets.may_alias(&a, &b));
    }

    #[test]
    fn unrelated_not_aliased() {
        let mut sets = AliasSets::new();
        sets.get_or_insert(0_u16);
        sets.get_or_insert(1_u16);
        assert!(!sets.may_alias(&0, &1));
    }

    #[test]
    fn transitive_alias() {
        let mut sets = AliasSets::new();
        sets.merge(0_u16, 1);
        sets.merge(1, 2);
        assert!(sets.may_alias(&0, &2));
    }

    #[test]
    fn num_sets_correct() {
        let mut sets = AliasSets::new();
        sets.get_or_insert(0_u16);
        sets.get_or_insert(1_u16);
        sets.get_or_insert(2_u16);
        assert_eq!(sets.num_sets(), 3);
        sets.merge(0, 1);
        assert_eq!(sets.num_sets(), 2);
    }
}
