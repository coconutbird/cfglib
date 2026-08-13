use cfglib::{
    DenseNodeId, DirectedGraph, DirectedGraphView, DominatorTree, TraversalDirection,
    breadth_first, shortest_path, tarjan_scc, topological_sort,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum FlowNode {
    Definition {
        symbol: String,
        path: String,
        start: u32,
    },
    CallResult {
        path: String,
        start: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowKind {
    Assign,
    Call,
    Return,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FlowEdge {
    kind: FlowKind,
    path: String,
    line: u32,
}

#[test]
fn symtree_like_value_flow_uses_generic_storage_and_reverse_queries() {
    let mut graph = DirectedGraph::new();
    let seed = graph.add_node(FlowNode::Definition {
        symbol: "seed".into(),
        path: "src/main.rs".into(),
        start: 7,
    });
    let parameter = graph.add_node(FlowNode::Definition {
        symbol: "value".into(),
        path: "src/helper.rs".into(),
        start: 18,
    });
    let call_result = graph.add_node(FlowNode::CallResult {
        path: "src/main.rs".into(),
        start: 42,
    });
    let output = graph.add_node(FlowNode::Definition {
        symbol: "output".into(),
        path: "src/main.rs".into(),
        start: 55,
    });

    graph.add_edge(
        seed,
        parameter,
        FlowEdge {
            kind: FlowKind::Call,
            path: "src/main.rs".into(),
            line: 4,
        },
    );
    graph.add_edge(
        parameter,
        call_result,
        FlowEdge {
            kind: FlowKind::Return,
            path: "src/helper.rs".into(),
            line: 3,
        },
    );
    let assignment = graph.add_edge(
        call_result,
        output,
        FlowEdge {
            kind: FlowKind::Assign,
            path: "src/main.rs".into(),
            line: 4,
        },
    );

    assert_eq!(
        breadth_first(&graph, seed, TraversalDirection::Outgoing),
        vec![seed, parameter, call_result, output]
    );
    assert_eq!(
        shortest_path(&graph, output, seed, TraversalDirection::Incoming),
        Some(vec![output, call_result, parameter, seed])
    );
    assert_eq!(graph[assignment].payload().kind, FlowKind::Assign);
    assert_eq!(graph[assignment].payload().path, "src/main.rs");
    assert_eq!(graph[assignment].payload().line, 4);
    assert_eq!(topological_sort(&graph).unwrap().len(), 4);
}

struct ExistingEdge {
    source: u32,
    target: u32,
}

struct ExistingGraph {
    edges: Vec<ExistingEdge>,
    outgoing: Vec<Vec<u32>>,
    incoming: Vec<Vec<u32>>,
}

impl DirectedGraphView for ExistingGraph {
    type NodeId = u32;

    fn node_count(&self) -> usize {
        self.outgoing.len()
    }

    fn node_ids(&self) -> impl Iterator<Item = Self::NodeId> + '_ {
        (0..self.outgoing.len()).map(<u32 as DenseNodeId>::from_index)
    }

    fn successors(&self, node: Self::NodeId) -> impl Iterator<Item = Self::NodeId> + '_ {
        self.outgoing[DenseNodeId::index(node)]
            .iter()
            .map(|edge| self.edges[DenseNodeId::index(*edge)].target)
    }

    fn predecessors(&self, node: Self::NodeId) -> impl Iterator<Item = Self::NodeId> + '_ {
        self.incoming[DenseNodeId::index(node)]
            .iter()
            .map(|edge| self.edges[DenseNodeId::index(*edge)].source)
    }
}

#[test]
fn existing_consumer_graph_can_adopt_algorithms_without_storage_migration() {
    let graph = ExistingGraph {
        edges: vec![
            ExistingEdge {
                source: 0,
                target: 1,
            },
            ExistingEdge {
                source: 0,
                target: 2,
            },
            ExistingEdge {
                source: 1,
                target: 3,
            },
            ExistingEdge {
                source: 2,
                target: 3,
            },
        ],
        outgoing: vec![vec![0, 1], vec![2], vec![3], vec![]],
        incoming: vec![vec![], vec![0], vec![1], vec![2, 3]],
    };

    let root = 0_u32;
    let merge = 3_u32;
    let dominators = DominatorTree::compute_from(&graph, root);
    assert_eq!(dominators.idom(merge), Some(root));
    assert!(tarjan_scc(&graph).is_dag(&graph));
    assert_eq!(
        shortest_path(&graph, root, merge, TraversalDirection::Outgoing)
            .unwrap()
            .len(),
        3
    );
}
