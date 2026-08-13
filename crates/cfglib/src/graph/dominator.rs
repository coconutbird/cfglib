//! Dominator tree computation using the Cooper-Harvey-Kennedy iterative
//! algorithm.

extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::graph::directed::DirectedGraph;
use crate::graph::traverse::{TraversalDirection, reverse_postorder};
use crate::graph::view::{DenseNodeId, DirectedGraphView, RootedGraphView};

/// A dominator tree computed from a rooted directed graph.
///
/// # Examples
///
/// ```
/// use cfglib::{Cfg, EdgeKind, DominatorTree};
///
/// let mut cfg = Cfg::<u32>::new();
/// let b0 = cfg.entry();
/// let b1 = cfg.new_block();
/// let b2 = cfg.new_block();
/// cfg.add_edge(b0, b1, EdgeKind::ConditionalTrue);
/// cfg.add_edge(b0, b2, EdgeKind::ConditionalFalse);
///
/// let dom = DominatorTree::compute(&cfg);
/// assert_eq!(dom.idom(b1), Some(b0));
/// assert_eq!(dom.idom(b2), Some(b0));
/// assert!(dom.dominates(b0, b1));
/// assert!(dom.dominates(b0, b2));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DominatorTree<N = BlockId> {
    /// Immediate dominator for each node. The root has no parent.
    idom: Vec<Option<N>>,
    reachable: Vec<bool>,
}

impl<N: DenseNodeId> DominatorTree<N> {
    /// Compute the dominator tree of a rooted graph view using the iterative
    /// algorithm by Cooper, Harvey, and Kennedy.
    #[must_use]
    pub fn compute<G>(graph: &G) -> Self
    where
        G: RootedGraphView<NodeId = N>,
    {
        Self::compute_from(graph, graph.root())
    }

    /// Compute dominators for any directed graph view from an explicit root.
    #[must_use]
    pub fn compute_from<G>(graph: &G, root: N) -> Self
    where
        G: DirectedGraphView<NodeId = N>,
    {
        let order = reverse_postorder(graph, root, TraversalDirection::Outgoing);
        let node_count = graph.node_count();
        let mut order_index = vec![usize::MAX; node_count];
        for (index, node) in order.iter().copied().enumerate() {
            order_index[node.index()] = index;
        }

        let mut dominators = vec![None; order.len()];
        dominators[order_index[root.index()]] = Some(order_index[root.index()]);

        let mut changed = true;
        while changed {
            changed = false;
            for node in order.iter().copied().filter(|node| *node != root) {
                let node_order = order_index[node.index()];
                let predecessors: Vec<N> = graph.predecessors(node).collect();
                let mut new_parent = predecessors.iter().find_map(|predecessor| {
                    let predecessor_order = order_index[predecessor.index()];
                    (predecessor_order != usize::MAX && dominators[predecessor_order].is_some())
                        .then_some(predecessor_order)
                });
                let Some(mut new_parent_index) = new_parent.take() else {
                    continue;
                };

                for predecessor in predecessors {
                    let predecessor_order = order_index[predecessor.index()];
                    if predecessor_order != usize::MAX
                        && dominators[predecessor_order].is_some()
                        && predecessor_order != new_parent_index
                    {
                        new_parent_index =
                            Self::intersect(&dominators, predecessor_order, new_parent_index);
                    }
                }

                if dominators[node_order] != Some(new_parent_index) {
                    dominators[node_order] = Some(new_parent_index);
                    changed = true;
                }
            }
        }

        let mut immediate = vec![None; node_count];
        for (index, parent) in dominators.into_iter().enumerate() {
            let node = order[index];
            immediate[node.index()] = parent.map(|parent_index| order[parent_index]);
        }
        immediate[root.index()] = None;
        let mut reachable = vec![false; node_count];
        for node in order {
            reachable[node.index()] = true;
        }
        Self {
            idom: immediate,
            reachable,
        }
    }

    fn intersect(dominators: &[Option<usize>], mut left: usize, mut right: usize) -> usize {
        while left != right {
            while left > right {
                left = dominators[left].expect("processed dominator must have a parent");
            }
            while right > left {
                right = dominators[right].expect("processed dominator must have a parent");
            }
        }
        left
    }

    /// Return the immediate dominator of `node`, or `None` for a root.
    #[must_use]
    pub fn idom(&self, node: N) -> Option<N> {
        self.idom[node.index()]
    }

    /// Return whether `dominator` dominates `node`.
    #[must_use]
    pub fn dominates(&self, dominator: N, node: N) -> bool {
        if dominator == node {
            return true;
        }

        let mut current = node;
        while let Some(parent) = self.idom(current) {
            if parent == dominator {
                return true;
            }
            if parent == current {
                break;
            }
            current = parent;
        }
        false
    }

    /// Return nodes whose immediate dominator is `node`.
    #[must_use]
    pub fn children(&self, node: N) -> Vec<N> {
        self.idom
            .iter()
            .enumerate()
            .filter(|(index, parent)| **parent == Some(node) && *index != node.index())
            .map(|(index, _)| N::from_index(index))
            .collect()
    }

    /// Return a node's depth in the dominator tree.
    #[must_use]
    pub fn depth(&self, node: N) -> Option<usize> {
        if !self.reachable[node.index()] {
            return None;
        }
        let mut depth = 0;
        let mut current = node;
        loop {
            match self.idom[current.index()] {
                None => return Some(depth),
                Some(parent) if parent == current => return Some(depth),
                Some(parent) => {
                    depth += 1;
                    current = parent;
                }
            }
        }
    }

    /// Return depths indexed by dense node index.
    #[must_use]
    pub fn depths(&self) -> Vec<usize> {
        (0..self.idom.len())
            .map(|index| self.depth(N::from_index(index)).unwrap_or(usize::MAX))
            .collect()
    }
}

impl DominatorTree<BlockId> {
    /// Compute the **post-dominator** tree for the given CFG.
    ///
    /// Post-dominators are computed by introducing a virtual exit node
    /// connected from all exit blocks (blocks with no successors), then
    /// running the dominator algorithm on the reverse graph starting
    /// from that virtual exit.
    ///
    /// This correctly handles CFGs with multiple exit points.
    #[must_use]
    pub fn compute_post<I>(cfg: &Cfg<I>) -> Self {
        let node_count = cfg.num_blocks();
        if node_count == 0 {
            return DominatorTree {
                idom: Vec::new(),
                reachable: Vec::new(),
            };
        }

        let mut exits: Vec<BlockId> = cfg.exit_blocks().collect();
        if exits.is_empty() {
            exits.push(BlockId::from_index(node_count - 1));
        }

        let mut reverse = DirectedGraph::with_capacity(
            node_count + 1,
            cfg.num_edges().saturating_add(exits.len()),
        );
        let nodes: Vec<_> = (0..node_count).map(|_| reverse.add_node(())).collect();
        let virtual_exit = reverse.add_node(());
        for edge in cfg.edges() {
            reverse.add_edge(
                nodes[edge.target().index()],
                nodes[edge.source().index()],
                (),
            );
        }
        for exit in exits {
            reverse.add_edge(virtual_exit, nodes[exit.index()], ());
        }

        let reverse_dominators = DominatorTree::compute_from(&reverse, virtual_exit);
        let idom = nodes
            .iter()
            .map(|&node| {
                reverse_dominators.idom(node).and_then(|parent| {
                    (parent != virtual_exit).then(|| BlockId::from_index(parent.index()))
                })
            })
            .collect();
        let reachable = reverse_dominators.reachable[..node_count].to_vec();
        DominatorTree { idom, reachable }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::edge::EdgeKind;
    use crate::test_util::MockInst;

    #[test]
    fn single_block_cfg() {
        let cfg: Cfg<MockInst> = Cfg::new();
        let dom = DominatorTree::compute(&cfg);
        assert_eq!(dom.idom(cfg.entry()), None);
        assert!(dom.dominates(cfg.entry(), cfg.entry()));
        assert!(dom.children(cfg.entry()).is_empty());
    }

    #[test]
    fn linear_chain_dominance() {
        let mut cfg: Cfg<MockInst> = Cfg::new();
        let b1 = cfg.new_block();
        let b2 = cfg.new_block();
        cfg.add_edge(cfg.entry(), b1, EdgeKind::Fallthrough);
        cfg.add_edge(b1, b2, EdgeKind::Fallthrough);
        let dom = DominatorTree::compute(&cfg);
        assert!(dom.dominates(cfg.entry(), b1));
        assert!(dom.dominates(cfg.entry(), b2));
        assert!(dom.dominates(b1, b2));
        assert!(!dom.dominates(b2, b1));
        assert_eq!(dom.idom(b1), Some(cfg.entry()));
        assert_eq!(dom.idom(b2), Some(b1));
    }

    #[test]
    fn diamond_idom_at_merge() {
        let mut cfg: Cfg<MockInst> = Cfg::new();
        let a = cfg.new_block();
        let b = cfg.new_block();
        let merge = cfg.new_block();
        cfg.add_edge(cfg.entry(), a, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), b, EdgeKind::ConditionalFalse);
        cfg.add_edge(a, merge, EdgeKind::Fallthrough);
        cfg.add_edge(b, merge, EdgeKind::Fallthrough);
        let dom = DominatorTree::compute(&cfg);
        // Merge block's idom should be entry (not a or b).
        assert_eq!(dom.idom(merge), Some(cfg.entry()));
        assert!(dom.dominates(cfg.entry(), a));
        assert!(dom.dominates(cfg.entry(), b));
        assert!(!dom.dominates(a, b));
        assert!(!dom.dominates(b, a));
    }

    #[test]
    fn self_loop_dominance() {
        let mut cfg: Cfg<MockInst> = Cfg::new();
        cfg.add_edge(cfg.entry(), cfg.entry(), EdgeKind::Back);
        let dom = DominatorTree::compute(&cfg);
        assert_eq!(dom.idom(cfg.entry()), None);
        assert!(dom.dominates(cfg.entry(), cfg.entry()));
    }

    #[test]
    fn unreachable_block_not_dominated() {
        let mut cfg: Cfg<MockInst> = Cfg::new();
        let unreachable = cfg.new_block();
        let dom = DominatorTree::compute(&cfg);
        // Entry still dominates itself.
        assert!(dom.dominates(cfg.entry(), cfg.entry()));
        // Unreachable block has no idom.
        assert_eq!(dom.idom(unreachable), None);
    }

    #[test]
    fn depth_computation() {
        let mut cfg: Cfg<MockInst> = Cfg::new();
        let b1 = cfg.new_block();
        let b2 = cfg.new_block();
        cfg.add_edge(cfg.entry(), b1, EdgeKind::Fallthrough);
        cfg.add_edge(b1, b2, EdgeKind::Fallthrough);
        let dom = DominatorTree::compute(&cfg);
        assert_eq!(dom.depth(cfg.entry()), Some(0));
        assert_eq!(dom.depth(b1), Some(1));
        assert_eq!(dom.depth(b2), Some(2));
    }

    #[test]
    fn children_returns_immediate_children() {
        let mut cfg: Cfg<MockInst> = Cfg::new();
        let a = cfg.new_block();
        let b = cfg.new_block();
        let c = cfg.new_block();
        cfg.add_edge(cfg.entry(), a, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), b, EdgeKind::ConditionalFalse);
        cfg.add_edge(a, c, EdgeKind::Fallthrough);
        let dom = DominatorTree::compute(&cfg);
        let mut entry_children = dom.children(cfg.entry());
        entry_children.sort();
        assert_eq!(entry_children.len(), 2);
        assert!(entry_children.contains(&a));
        assert!(entry_children.contains(&b));
        assert_eq!(dom.children(a), vec![c]);
    }
}
