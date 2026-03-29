//! Graph coloring — interference graph and register allocation support.
//!
//! Builds an interference graph from liveness information and provides
//! a greedy graph-coloring heuristic for register allocation.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::cfg::Cfg;
use crate::dataflow::liveness::Liveness;
use crate::dataflow::{InstrInfo, Location};

/// An interference graph — undirected graph where nodes are locations
/// and edges connect simultaneously-live locations.
#[derive(Debug, Clone)]
pub struct InterferenceGraph {
    /// Adjacency sets for each location.
    pub adj: BTreeMap<Location, BTreeSet<Location>>,
}

impl InterferenceGraph {
    /// Build an interference graph from liveness data.
    pub fn build<I: InstrInfo>(cfg: &Cfg<I>, live: &Liveness) -> Self {
        let mut adj: BTreeMap<Location, BTreeSet<Location>> = BTreeMap::new();

        for block in cfg.blocks() {
            let bid = block.id();
            let live_out = live.live_out(bid);
            let locs: Vec<Location> = live_out.iter().copied().collect();
            for i in 0..locs.len() {
                for j in (i + 1)..locs.len() {
                    adj.entry(locs[i]).or_default().insert(locs[j]);
                    adj.entry(locs[j]).or_default().insert(locs[i]);
                }
            }
        }

        Self { adj }
    }

    /// Number of nodes (locations) in the interference graph.
    pub fn num_nodes(&self) -> usize {
        self.adj.len()
    }

    /// Degree of a node.
    pub fn degree(&self, loc: Location) -> usize {
        self.adj.get(&loc).map_or(0, |s| s.len())
    }
}

/// Result of graph coloring.
#[derive(Debug, Clone)]
pub struct ColorAssignment {
    /// Location → color (register number).
    pub assignment: BTreeMap<Location, u32>,
    /// Number of colors used.
    pub num_colors: u32,
}

/// Greedy graph coloring on an interference graph.
///
/// Uses simplicial elimination ordering (sort by degree, ascending).
/// Returns a color assignment.
pub fn color_graph(ig: &InterferenceGraph) -> ColorAssignment {
    // Order nodes by degree (ascending) for greedy coloring.
    let mut nodes: Vec<Location> = ig.adj.keys().copied().collect();
    nodes.sort_by_key(|loc| ig.degree(*loc));

    let mut assignment: BTreeMap<Location, u32> = BTreeMap::new();
    let mut num_colors = 0u32;

    for loc in &nodes {
        // Find the lowest color not used by neighbors.
        let mut used_colors = BTreeSet::new();
        if let Some(neighbors) = ig.adj.get(loc) {
            for n in neighbors {
                if let Some(&c) = assignment.get(n) {
                    used_colors.insert(c);
                }
            }
        }
        let mut color = 0u32;
        while used_colors.contains(&color) {
            color += 1;
        }
        assignment.insert(*loc, color);
        if color + 1 > num_colors {
            num_colors = color + 1;
        }
    }

    ColorAssignment {
        assignment,
        num_colors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triangle_needs_three_colors() {
        let mut adj = BTreeMap::new();
        let a = Location(0);
        let b = Location(1);
        let c = Location(2);
        adj.insert(a, [b, c].into_iter().collect());
        adj.insert(b, [a, c].into_iter().collect());
        adj.insert(c, [a, b].into_iter().collect());
        let ig = InterferenceGraph { adj };
        let result = color_graph(&ig);
        assert_eq!(result.num_colors, 3);
        assert_ne!(result.assignment[&a], result.assignment[&b]);
        assert_ne!(result.assignment[&a], result.assignment[&c]);
        assert_ne!(result.assignment[&b], result.assignment[&c]);
    }

    #[test]
    fn independent_nodes_one_color() {
        let mut adj = BTreeMap::new();
        adj.insert(Location(0), BTreeSet::new());
        adj.insert(Location(1), BTreeSet::new());
        let ig = InterferenceGraph { adj };
        let result = color_graph(&ig);
        assert_eq!(result.num_colors, 1);
    }

    #[test]
    fn empty_graph() {
        let ig = InterferenceGraph {
            adj: BTreeMap::new(),
        };
        let result = color_graph(&ig);
        assert_eq!(result.num_colors, 0);
    }
}
