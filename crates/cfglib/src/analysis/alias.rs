//! Alias analysis — Steensgaard-style unification-based points-to analysis.
//!
//! Groups memory locations into alias sets using union-find. Two locations
//! are in the same alias set if they may refer to the same memory.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::cfg::Cfg;
use crate::dataflow::{InstrInfo, Location};

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
    /// Returns `(base_location, op)` pairs.
    fn memory_ops(&self) -> &[(Location, MemoryOp)];
}

/// Union-Find structure for alias set computation.
#[derive(Debug, Clone)]
pub struct AliasSets {
    parent: Vec<usize>,
    rank: Vec<usize>,
    loc_to_id: BTreeMap<Location, usize>,
    id_to_loc: Vec<Location>,
}

impl AliasSets {
    /// Create empty alias sets.
    pub fn new() -> Self {
        Self {
            parent: Vec::new(),
            rank: Vec::new(),
            loc_to_id: BTreeMap::new(),
            id_to_loc: Vec::new(),
        }
    }

    fn get_or_insert(&mut self, loc: Location) -> usize {
        if let Some(&id) = self.loc_to_id.get(&loc) {
            return id;
        }
        let id = self.parent.len();
        self.parent.push(id);
        self.rank.push(0);
        self.loc_to_id.insert(loc, id);
        self.id_to_loc.push(loc);
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
        if self.rank[ra] < self.rank[rb] {
            self.parent[ra] = rb;
        } else if self.rank[ra] > self.rank[rb] {
            self.parent[rb] = ra;
        } else {
            self.parent[rb] = ra;
            self.rank[ra] += 1;
        }
    }

    /// Check if two locations may alias.
    pub fn may_alias(&mut self, a: Location, b: Location) -> bool {
        let Some(&ia) = self.loc_to_id.get(&a) else {
            return false;
        };
        let Some(&ib) = self.loc_to_id.get(&b) else {
            return false;
        };
        self.find(ia) == self.find(ib)
    }

    /// Get the alias set (representative) for a location.
    pub fn alias_set(&mut self, loc: Location) -> Option<Location> {
        let &id = self.loc_to_id.get(&loc)?;
        let rep = self.find(id);
        Some(self.id_to_loc[rep])
    }

    /// Merge two locations into the same alias set.
    pub fn merge(&mut self, a: Location, b: Location) {
        let ia = self.get_or_insert(a);
        let ib = self.get_or_insert(b);
        self.union(ia, ib);
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

impl Default for AliasSets {
    fn default() -> Self {
        Self::new()
    }
}

/// Run Steensgaard-style alias analysis on a CFG.
///
/// Unifies locations that are stored to/loaded from the same base.
/// This is a flow-insensitive, context-insensitive analysis.
pub fn alias_analysis<I: MemoryInfo>(cfg: &Cfg<I>) -> AliasSets {
    let mut sets = AliasSets::new();

    // Register all locations.
    for block in cfg.blocks() {
        for inst in block.instructions() {
            for d in inst.defs() {
                sets.get_or_insert(*d);
            }
            for u in inst.uses() {
                sets.get_or_insert(*u);
            }
        }
    }

    // Unify locations involved in the same memory operations.
    for block in cfg.blocks() {
        for inst in block.instructions() {
            let ops = inst.memory_ops();
            if ops.len() >= 2 {
                let first = ops[0].0;
                for &(loc, _) in &ops[1..] {
                    sets.merge(first, loc);
                }
            }
            // Also unify defs with store targets.
            for &(mem_loc, op) in ops {
                if op == MemoryOp::Store {
                    for d in inst.defs() {
                        sets.merge(mem_loc, *d);
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
        let a = Location(0);
        let b = Location(1);
        sets.merge(a, b);
        assert!(sets.may_alias(a, b));
    }

    #[test]
    fn unrelated_not_aliased() {
        let mut sets = AliasSets::new();
        sets.get_or_insert(Location(0));
        sets.get_or_insert(Location(1));
        assert!(!sets.may_alias(Location(0), Location(1)));
    }

    #[test]
    fn transitive_alias() {
        let mut sets = AliasSets::new();
        sets.merge(Location(0), Location(1));
        sets.merge(Location(1), Location(2));
        assert!(sets.may_alias(Location(0), Location(2)));
    }

    #[test]
    fn num_sets_correct() {
        let mut sets = AliasSets::new();
        sets.get_or_insert(Location(0));
        sets.get_or_insert(Location(1));
        sets.get_or_insert(Location(2));
        assert_eq!(sets.num_sets(), 3);
        sets.merge(Location(0), Location(1));
        assert_eq!(sets.num_sets(), 2);
    }
}
