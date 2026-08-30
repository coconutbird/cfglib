//! Explicit may-alias equivalence classes.
//!
//! Consumers can populate [`AliasSets`] from language-specific points-to or
//! binding information and pass it directly to [`MemorySSA`](crate::MemorySSA).

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::dataflow::VariableId;
use crate::memory::MemoryAlias;

/// Caller-populated may-alias equivalence classes.
///
/// Equal locations always alias. Unequal locations alias only after they have
/// been joined with [`AliasSets::merge`], directly or transitively. Passing an
/// instance to [`MemorySSA`](crate::MemorySSA) therefore promises that every
/// unmerged pair is disjoint.
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

    /// Registers one location as a singleton alias set.
    pub fn insert(&mut self, variable: V) {
        self.get_or_insert(variable);
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

    fn representative(&self, mut id: usize) -> usize {
        while self.parent[id] != id {
            id = self.parent[id];
        }
        id
    }

    /// Checks whether two locations are in the same may-alias class.
    #[must_use]
    pub fn may_alias(&self, left: &V, right: &V) -> bool {
        if left == right {
            return true;
        }
        let Some(&left_id) = self.variable_to_id.get(left) else {
            return false;
        };
        let Some(&right_id) = self.variable_to_id.get(right) else {
            return false;
        };
        self.representative(left_id) == self.representative(right_id)
    }

    /// Gets the alias-set representative for a registered location.
    #[must_use]
    pub fn alias_set(&self, variable: &V) -> Option<&V> {
        let &id = self.variable_to_id.get(variable)?;
        let representative = self.representative(id);
        self.id_to_variable.get(representative)
    }

    /// Merge two variables into the same alias set.
    pub fn merge(&mut self, left: V, right: V) {
        let left_id = self.get_or_insert(left);
        let right_id = self.get_or_insert(right);
        self.union(left_id, right_id);
    }

    /// Returns the number of registered alias sets.
    #[must_use]
    pub fn set_count(&self) -> usize {
        let n = self.parent.len();
        let mut roots = alloc::collections::BTreeSet::new();
        for i in 0..n {
            roots.insert(self.representative(i));
        }
        roots.len()
    }
}

impl<V: VariableId> Default for AliasSets<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: VariableId> MemoryAlias<V> for AliasSets<V> {
    fn may_alias(&self, left: &V, right: &V) -> bool {
        Self::may_alias(self, left, right)
    }
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
        sets.insert(0_u16);
        sets.insert(1_u16);
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
        sets.insert(0_u16);
        sets.insert(1_u16);
        sets.insert(2_u16);
        assert_eq!(sets.set_count(), 3);
        sets.merge(0, 1);
        assert_eq!(sets.set_count(), 2);
    }
}
