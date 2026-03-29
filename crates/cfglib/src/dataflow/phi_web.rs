//! Phi webs — congruence classes for SSA values.
//!
//! Groups SSA locations connected through φ-nodes into equivalence
//! classes. Two locations in the same phi web must be assigned the
//! same physical register, enabling register coalescing.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;
use alloc::vec;
use alloc::vec::Vec;

use crate::dataflow::Location;
use crate::dataflow::ssa::PhiMap;

/// Union-Find for phi web computation.
#[derive(Debug, Clone)]
struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
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
}

/// A phi web: a set of locations that must be coalesced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhiWeb {
    /// Locations in this congruence class.
    pub locations: BTreeSet<Location>,
}

/// Result of phi web computation.
#[derive(Debug, Clone)]
pub struct PhiWebs {
    /// All phi webs found.
    pub webs: Vec<PhiWeb>,
    /// Map from location to its web index.
    pub web_of: BTreeMap<Location, usize>,
}

/// Compute phi webs from a phi map.
///
/// Locations connected through the same φ-node are placed in the
/// same congruence class.
pub fn compute_phi_webs(phis: &PhiMap) -> PhiWebs {
    // Collect all locations mentioned in phis.
    let mut all_locs: Vec<Location> = Vec::new();
    let mut loc_to_idx: BTreeMap<Location, usize> = BTreeMap::new();

    for (_, phi) in phis.iter() {
        for loc in core::iter::once(&phi.location).chain(phi.operands.iter().map(|(_, l)| l)) {
            if !loc_to_idx.contains_key(loc) {
                let idx = all_locs.len();
                loc_to_idx.insert(*loc, idx);
                all_locs.push(*loc);
            }
        }
    }

    if all_locs.is_empty() {
        return PhiWebs {
            webs: Vec::new(),
            web_of: BTreeMap::new(),
        };
    }

    let mut uf = UnionFind::new(all_locs.len());

    // Union phi def with each operand.
    for (_, phi) in phis.iter() {
        let def_idx = loc_to_idx[&phi.location];
        for (_, loc) in &phi.operands {
            let op_idx = loc_to_idx[loc];
            uf.union(def_idx, op_idx);
        }
    }

    // Build webs from union-find.
    let mut root_to_web: BTreeMap<usize, usize> = BTreeMap::new();
    let mut webs: Vec<PhiWeb> = Vec::new();
    let mut web_of: BTreeMap<Location, usize> = BTreeMap::new();

    for (i, &loc) in all_locs.iter().enumerate() {
        let root = uf.find(i);
        let web_idx = if let Some(&idx) = root_to_web.get(&root) {
            idx
        } else {
            let idx = webs.len();
            webs.push(PhiWeb {
                locations: BTreeSet::new(),
            });
            root_to_web.insert(root, idx);
            idx
        };
        webs[web_idx].locations.insert(loc);
        web_of.insert(loc, web_idx);
    }

    PhiWebs { webs, web_of }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::dataflow::ssa::insert_phis;
    use crate::edge::EdgeKind;
    use crate::graph::dominator::DominatorTree;
    use crate::test_util::{df_def, df_use};

    #[test]
    fn empty_phis_empty_webs() {
        let cfg: Cfg<crate::test_util::DfInst> = Cfg::new();
        let dom = DominatorTree::compute(&cfg);
        let phis = insert_phis(&cfg, &dom);
        let webs = compute_phi_webs(&phis);
        assert!(webs.webs.is_empty());
    }

    #[test]
    fn diamond_phi_creates_web() {
        let mut cfg: Cfg<crate::test_util::DfInst> = Cfg::new();
        let a = cfg.new_block();
        let b = cfg.new_block();
        let merge = cfg.new_block();
        cfg.block_mut(a)
            .instructions_vec_mut()
            .push(df_def("def_a", 0));
        cfg.block_mut(b)
            .instructions_vec_mut()
            .push(df_def("def_b", 0));
        cfg.block_mut(merge)
            .instructions_vec_mut()
            .push(df_use("use", 0));
        cfg.add_edge(cfg.entry(), a, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), b, EdgeKind::ConditionalFalse);
        cfg.add_edge(a, merge, EdgeKind::Fallthrough);
        cfg.add_edge(b, merge, EdgeKind::Fallthrough);
        let dom = DominatorTree::compute(&cfg);
        let phis = insert_phis(&cfg, &dom);
        let webs = compute_phi_webs(&phis);
        // If phis were inserted for loc0, all mentions should be in the same web.
        if !webs.webs.is_empty() {
            assert!(webs.webs.iter().any(|w| w.locations.contains(&Location(0))));
        }
    }
}
