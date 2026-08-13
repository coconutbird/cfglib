//! Whole-program call graphs built on the generic directed-graph substrate.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::cfg::Cfg;
use crate::edge::EdgeKind;
use crate::graph::directed::{DirectedEdge, DirectedGraph, NodeId};
use crate::graph::scc::tarjan_scc;
use crate::graph::traverse::topological_sort;

/// Identity of a function in a [`CallGraph`].
pub type FunctionId = NodeId;

/// Payload of a function node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionNode {
    /// Symbolic function name or address label.
    pub name: String,
}

/// Payload attached to a call edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallMetadata {
    /// Whether the call transfers control without returning to its caller.
    pub is_tail_call: bool,
}

/// A directed call edge with stable identity and endpoint accessors.
pub type CallEdge = DirectedEdge<CallMetadata>;

/// A call graph linking functions through typed call edges.
#[derive(Debug, Clone)]
pub struct CallGraph {
    graph: DirectedGraph<FunctionNode, CallMetadata>,
    name_to_id: BTreeMap<String, FunctionId>,
}

impl CallGraph {
    /// Create an empty call graph.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            graph: DirectedGraph::new(),
            name_to_id: BTreeMap::new(),
        }
    }

    /// Add a function, returning the existing identity when its name is known.
    pub fn add_function(&mut self, name: &str) -> FunctionId {
        if let Some(&id) = self.name_to_id.get(name) {
            return id;
        }

        let owned_name = String::from(name);
        let id = self.graph.add_node(FunctionNode {
            name: owned_name.clone(),
        });
        self.name_to_id.insert(owned_name, id);
        id
    }

    /// Record a call from `source` to `target`.
    pub fn add_call(&mut self, source: FunctionId, target: FunctionId, is_tail_call: bool) {
        self.graph
            .add_edge(source, target, CallMetadata { is_tail_call });
    }

    /// Iterate over functions called by `function`.
    pub fn callees(&self, function: FunctionId) -> impl Iterator<Item = FunctionId> + '_ {
        self.graph.successors(function)
    }

    /// Iterate over functions that call `function`.
    pub fn callers(&self, function: FunctionId) -> impl Iterator<Item = FunctionId> + '_ {
        self.graph.predecessors(function)
    }

    /// Borrow a function payload by identity.
    #[must_use]
    pub fn function(&self, function: FunctionId) -> &FunctionNode {
        self.graph.node(function)
    }

    /// Look up a function identity by name.
    #[must_use]
    pub fn function_by_name(&self, name: &str) -> Option<FunctionId> {
        self.name_to_id.get(name).copied()
    }

    /// Return the number of functions.
    #[must_use]
    pub fn num_functions(&self) -> usize {
        self.graph.node_count()
    }

    /// Iterate over all call edges.
    pub fn edges(&self) -> impl Iterator<Item = &CallEdge> {
        self.graph.edges()
    }

    /// Return all function payloads in identity order.
    #[must_use]
    pub fn functions(&self) -> &[FunctionNode] {
        self.graph.nodes()
    }

    /// Borrow the underlying generic graph for shared graph algorithms.
    #[must_use]
    pub const fn as_directed_graph(&self) -> &DirectedGraph<FunctionNode, CallMetadata> {
        &self.graph
    }

    /// Return functions that call no other function.
    #[must_use]
    pub fn leaf_functions(&self) -> Vec<FunctionId> {
        self.graph
            .node_ids()
            .filter(|&function| self.graph.outgoing_edges(function).is_empty())
            .collect()
    }

    /// Return functions with no callers.
    #[must_use]
    pub fn root_functions(&self) -> Vec<FunctionId> {
        self.graph
            .node_ids()
            .filter(|&function| self.graph.incoming_edges(function).is_empty())
            .collect()
    }

    /// Return whether `function` is directly or mutually recursive.
    #[must_use]
    pub fn is_recursive(&self, function: FunctionId) -> bool {
        self.graph
            .successors(function)
            .any(|callee| callee == function)
            || tarjan_scc(&self.graph).component(function).nodes.len() > 1
    }

    /// Build a call graph by scanning CFG call edges.
    #[must_use]
    pub fn build_from_cfgs<I>(cfgs: &[(&str, &Cfg<I>)]) -> Self {
        let mut call_graph = Self::new();
        for &(name, _) in cfgs {
            call_graph.add_function(name);
        }

        for &(caller_name, cfg) in cfgs {
            let caller = call_graph.name_to_id[caller_name];
            for edge in cfg.edges() {
                if matches!(edge.kind(), EdgeKind::Call | EdgeKind::IndirectCall)
                    && let Some(call_site) = edge.call_site()
                    && let Some(target_name) = &call_site.target_name
                    && let Some(&callee) = call_graph.name_to_id.get(target_name.as_str())
                {
                    call_graph.add_call(caller, callee, call_site.is_tail_call);
                }
            }
        }

        call_graph
    }

    /// Return a topological order, or `None` when calls contain a cycle.
    #[must_use]
    pub fn topological_order(&self) -> Option<Vec<FunctionId>> {
        topological_sort(&self.graph)
    }

    /// Return strongly connected function groups in reverse topological order.
    #[must_use]
    pub fn strongly_connected_components(&self) -> Vec<Vec<FunctionId>> {
        tarjan_scc(&self.graph)
            .components
            .into_iter()
            .map(|component| component.nodes.into_iter().collect())
            .collect()
    }
}

impl Default for CallGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::CallSite;
    use crate::test_util::ff;
    use alloc::vec;

    #[test]
    fn build_from_cfgs_resolves_calls() {
        let mut main_cfg = Cfg::new();
        let continuation = main_cfg.new_block();
        main_cfg.block_mut(main_cfg.entry()).push(ff("call"));
        let edge = main_cfg.add_edge(main_cfg.entry(), continuation, EdgeKind::Call);
        main_cfg
            .edge_mut(edge)
            .set_call_site(Some(CallSite::named("helper")));
        let mut helper_cfg = Cfg::new();
        helper_cfg.block_mut(helper_cfg.entry()).push(ff("ret"));

        let graph = CallGraph::build_from_cfgs(&[("main", &main_cfg), ("helper", &helper_cfg)]);
        let main = graph.function_by_name("main").unwrap();
        let helper = graph.function_by_name("helper").unwrap();
        assert!(graph.callees(main).any(|callee| callee == helper));
    }

    #[test]
    fn topology_and_recursion_use_shared_algorithms() {
        let mut graph = CallGraph::new();
        let main = graph.add_function("main");
        let helper = graph.add_function("helper");
        graph.add_call(main, helper, false);
        assert_eq!(graph.topological_order(), Some(vec![main, helper]));
        assert_eq!(graph.leaf_functions(), vec![helper]);
        assert_eq!(graph.root_functions(), vec![main]);

        graph.add_call(helper, main, false);
        assert!(graph.topological_order().is_none());
        assert!(graph.is_recursive(main));
        assert!(
            graph
                .strongly_connected_components()
                .iter()
                .any(|component| component.len() == 2)
        );
    }
}
