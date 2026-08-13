//! Generic interference graphs and graph coloring.

extern crate alloc;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use crate::cfg::Cfg;
use crate::dataflow::liveness::Liveness;
use crate::dataflow::{InstrInfo, VariableId};

/// An undirected interference graph whose nodes are IR-defined variables.
#[derive(Debug, Clone)]
pub struct InterferenceGraph<V> {
    /// Adjacency set for each variable.
    pub adjacency: BTreeMap<V, BTreeSet<V>>,
}

impl<V: VariableId> InterferenceGraph<V> {
    /// Build an interference graph from block liveness.
    #[must_use]
    pub fn build<I: InstrInfo<Variable = V>>(cfg: &Cfg<I>, live: &Liveness<V>) -> Self {
        let mut adjacency: BTreeMap<V, BTreeSet<V>> = BTreeMap::new();

        for block in cfg.blocks() {
            let variables: Vec<V> = live.live_out(block.id()).iter().cloned().collect();
            for (index, variable) in variables.iter().enumerate() {
                adjacency.entry(variable.clone()).or_default();
                for other in variables.iter().skip(index + 1) {
                    adjacency
                        .entry(variable.clone())
                        .or_default()
                        .insert(other.clone());
                    adjacency
                        .entry(other.clone())
                        .or_default()
                        .insert(variable.clone());
                }
            }
        }

        Self { adjacency }
    }

    /// Return the number of variables in the graph.
    #[must_use]
    pub fn len(&self) -> usize {
        self.adjacency.len()
    }

    /// Return whether the graph has no variables.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.adjacency.is_empty()
    }

    /// Return the degree of `variable`.
    #[must_use]
    pub fn degree(&self, variable: &V) -> usize {
        self.adjacency.get(variable).map_or(0, BTreeSet::len)
    }
}

/// Result of greedy graph coloring.
#[derive(Debug, Clone)]
pub struct ColorAssignment<V> {
    /// Assigned color for each variable.
    pub assignment: BTreeMap<V, u32>,
    /// Number of colors used.
    pub num_colors: u32,
}

/// Greedily color an interference graph in ascending-degree order.
#[must_use]
pub fn color_graph<V: VariableId>(graph: &InterferenceGraph<V>) -> ColorAssignment<V> {
    let mut variables: Vec<V> = graph.adjacency.keys().cloned().collect();
    variables.sort_by_key(|variable| graph.degree(variable));

    let mut assignment = BTreeMap::new();
    let mut num_colors = 0;
    for variable in variables {
        let mut used_colors = BTreeSet::new();
        if let Some(neighbors) = graph.adjacency.get(&variable) {
            for neighbor in neighbors {
                if let Some(&color) = assignment.get(neighbor) {
                    used_colors.insert(color);
                }
            }
        }

        let mut color = 0;
        while used_colors.contains(&color) {
            color += 1;
        }
        assignment.insert(variable, color);
        num_colors = num_colors.max(color + 1);
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
        let adjacency = BTreeMap::from([
            (0_u16, BTreeSet::from([1, 2])),
            (1, BTreeSet::from([0, 2])),
            (2, BTreeSet::from([0, 1])),
        ]);
        let result = color_graph(&InterferenceGraph { adjacency });
        assert_eq!(result.num_colors, 3);
        assert_ne!(result.assignment[&0], result.assignment[&1]);
        assert_ne!(result.assignment[&0], result.assignment[&2]);
        assert_ne!(result.assignment[&1], result.assignment[&2]);
    }

    #[test]
    fn independent_nodes_share_one_color() {
        let adjacency = BTreeMap::from([(0_u16, BTreeSet::new()), (1, BTreeSet::new())]);
        assert_eq!(color_graph(&InterferenceGraph { adjacency }).num_colors, 1);
    }

    #[test]
    fn empty_graph_uses_no_colors() {
        let graph = InterferenceGraph::<u16> {
            adjacency: BTreeMap::new(),
        };
        assert_eq!(color_graph(&graph).num_colors, 0);
    }
}
