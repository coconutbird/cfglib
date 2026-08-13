//! Whole-program call-graph construction on [`DirectedGraph`].
//!
//! A call graph does not need its own storage abstraction. Functions are node
//! payloads, call details are edge payloads, and all traversal, SCC, dominance,
//! and topological algorithms operate on the returned generic graph directly.
//! Call targets come from the instructions themselves via
//! [`CallInfo`] — callee identities are consumer-typed (symbol ids, addresses,
//! names), never a library-owned string.

extern crate alloc;
use alloc::collections::BTreeMap;

use crate::cfg::Cfg;
use crate::flow::CallInfo;
use crate::graph::directed::{DirectedGraph, NodeId};
use crate::graph::scc::tarjan_scc;

/// Payload of a function node in a call graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionNode<C> {
    /// Consumer-defined function identity (symbol id, address, name).
    pub id: C,
}

/// Payload attached to a call edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallMetadata {
    /// Whether the call transfers control without returning to its caller.
    pub is_tail_call: bool,
}

/// Build a call graph by scanning the instructions of a set of keyed CFGs.
///
/// Nodes are emitted in input order. Every instruction whose
/// [`CallInfo::callee`] resolves to one of the input keys creates an
/// inter-procedural edge; calls to unknown targets remain represented in the
/// source CFG but do not create an edge.
#[must_use]
pub fn build_call_graph<I: CallInfo>(
    functions: &[(I::Callee, &Cfg<I>)],
) -> DirectedGraph<FunctionNode<I::Callee>, CallMetadata> {
    let mut graph = DirectedGraph::with_capacity(functions.len(), functions.len());
    let mut key_to_id = BTreeMap::new();

    for (key, _) in functions {
        if key_to_id.contains_key(key) {
            continue;
        }
        let id = graph.add_node(FunctionNode { id: key.clone() });
        key_to_id.insert(key.clone(), id);
    }

    for (caller_key, cfg) in functions {
        let Some(&caller) = key_to_id.get(caller_key) else {
            continue;
        };
        for block in cfg.blocks() {
            for instruction in block.instructions() {
                if let Some(callee_key) = instruction.callee()
                    && let Some(&callee) = key_to_id.get(&callee_key)
                {
                    graph.add_edge(
                        caller,
                        callee,
                        CallMetadata {
                            is_tail_call: instruction.is_tail_call(),
                        },
                    );
                }
            }
        }
    }

    graph
}

/// Find a function node by its identity.
#[must_use]
pub fn find_function<C: Ord>(
    graph: &DirectedGraph<FunctionNode<C>, CallMetadata>,
    id: &C,
) -> Option<NodeId> {
    graph.node_ids().find(|&node| graph[node].id == *id)
}

/// Return whether a function is directly or mutually recursive.
#[must_use]
pub fn is_recursive_function<C>(
    graph: &DirectedGraph<FunctionNode<C>, CallMetadata>,
    function: NodeId,
) -> bool {
    graph.successors(function).any(|callee| callee == function)
        || tarjan_scc(graph).component(function).nodes.len() > 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::traverse::topological_sort;
    use crate::test_util::{DfInst, df_call, df_ff};
    use alloc::vec;

    #[test]
    fn build_from_cfgs_resolves_calls() {
        let mut main_cfg: Cfg<DfInst> = Cfg::new();
        main_cfg
            .block_mut(main_cfg.entry())
            .push(df_call("call", "helper", false));
        let mut helper_cfg: Cfg<DfInst> = Cfg::new();
        helper_cfg.block_mut(helper_cfg.entry()).push(df_ff("ret"));

        let graph = build_call_graph(&[("main", &main_cfg), ("helper", &helper_cfg)]);
        let main = find_function(&graph, &"main").unwrap();
        let helper = find_function(&graph, &"helper").unwrap();
        assert!(graph.successors(main).any(|callee| callee == helper));
        assert!(!graph.edges().next().unwrap().payload().is_tail_call);
    }

    #[test]
    fn topology_and_recursion_use_shared_algorithms() {
        let mut graph = DirectedGraph::new();
        let main = graph.add_node(FunctionNode { id: "main" });
        let helper = graph.add_node(FunctionNode { id: "helper" });
        graph.add_edge(
            main,
            helper,
            CallMetadata {
                is_tail_call: false,
            },
        );
        assert_eq!(topological_sort(&graph), Some(vec![main, helper]));
        assert_eq!(graph.predecessors(main).count(), 0);
        assert_eq!(graph.successors(helper).count(), 0);

        graph.add_edge(
            helper,
            main,
            CallMetadata {
                is_tail_call: false,
            },
        );
        assert!(topological_sort(&graph).is_none());
        assert!(is_recursive_function(&graph, main));
    }
}
