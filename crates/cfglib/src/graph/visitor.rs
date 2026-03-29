//! Visitor trait — generic infrastructure for CFG traversals.
//!
//! Provides a [`CfgVisitor`] trait with default DFS and BFS traversal
//! drivers, allowing users to plug in custom logic per-block and per-edge.

extern crate alloc;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::edge::EdgeId;

/// Trait for visiting blocks and edges in a CFG.
///
/// Implement whichever methods you need; all default to no-ops.
pub trait CfgVisitor<I> {
    /// Called when a block is first discovered.
    fn visit_block(&mut self, _cfg: &Cfg<I>, _block: BlockId) {}

    /// Called for each outgoing edge from a block.
    fn visit_edge(&mut self, _cfg: &Cfg<I>, _edge: EdgeId) {}

    /// Called when all successors of a block have been visited (DFS only).
    fn finish_block(&mut self, _cfg: &Cfg<I>, _block: BlockId) {}
}

/// Drive a depth-first traversal over the CFG, invoking visitor callbacks.
///
/// # Examples
///
/// ```
/// use cfglib::{Cfg, EdgeKind, BlockId, CfgVisitor, walk_dfs};
///
/// struct Counter(usize);
/// impl CfgVisitor<u32> for Counter {
///     fn visit_block(&mut self, _cfg: &Cfg<u32>, _block: BlockId) {
///         self.0 += 1;
///     }
/// }
///
/// let mut cfg = Cfg::<u32>::new();
/// let b1 = cfg.new_block();
/// cfg.add_edge(cfg.entry(), b1, EdgeKind::Fallthrough);
///
/// let mut counter = Counter(0);
/// walk_dfs(&cfg, &mut counter);
/// assert_eq!(counter.0, 2);
/// ```
pub fn walk_dfs<I, V: CfgVisitor<I>>(cfg: &Cfg<I>, visitor: &mut V) {
    let mut visited = BTreeSet::new();
    let mut stack = alloc::vec![cfg.entry()];

    while let Some(bid) = stack.pop() {
        if !visited.insert(bid) {
            continue;
        }
        visitor.visit_block(cfg, bid);

        let edges = cfg.successor_edges(bid);
        // Push in reverse so the first successor is visited first.
        let succs: Vec<BlockId> = edges
            .iter()
            .map(|&eid| {
                visitor.visit_edge(cfg, eid);
                cfg.edge(eid).target()
            })
            .collect();

        for &s in succs.iter().rev() {
            if !visited.contains(&s) {
                stack.push(s);
            }
        }
        visitor.finish_block(cfg, bid);
    }
}

/// Drive a breadth-first traversal over the CFG, invoking visitor callbacks.
pub fn walk_bfs<I, V: CfgVisitor<I>>(cfg: &Cfg<I>, visitor: &mut V) {
    let mut visited = BTreeSet::new();
    let mut queue = alloc::collections::VecDeque::new();
    queue.push_back(cfg.entry());
    visited.insert(cfg.entry());

    while let Some(bid) = queue.pop_front() {
        visitor.visit_block(cfg, bid);

        for &eid in cfg.successor_edges(bid) {
            visitor.visit_edge(cfg, eid);
            let target = cfg.edge(eid).target();
            if visited.insert(target) {
                queue.push_back(target);
            }
        }
        visitor.finish_block(cfg, bid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::edge::EdgeKind;
    use crate::test_util::ff;

    struct BlockCounter {
        count: usize,
    }
    impl<I> CfgVisitor<I> for BlockCounter {
        fn visit_block(&mut self, _cfg: &Cfg<I>, _block: BlockId) {
            self.count += 1;
        }
    }

    #[test]
    fn dfs_visits_all_blocks() {
        let mut cfg = Cfg::new();
        let a = cfg.new_block();
        let b = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("e"));
        cfg.block_mut(a).instructions_vec_mut().push(ff("a"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.add_edge(cfg.entry(), a, EdgeKind::Fallthrough);
        cfg.add_edge(a, b, EdgeKind::Fallthrough);
        let mut v = BlockCounter { count: 0 };
        walk_dfs(&cfg, &mut v);
        assert_eq!(v.count, 3);
    }

    #[test]
    fn bfs_visits_all_blocks() {
        let mut cfg = Cfg::new();
        let a = cfg.new_block();
        let b = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("e"));
        cfg.block_mut(a).instructions_vec_mut().push(ff("a"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.add_edge(cfg.entry(), a, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), b, EdgeKind::ConditionalFalse);
        let mut v = BlockCounter { count: 0 };
        walk_bfs(&cfg, &mut v);
        assert_eq!(v.count, 3);
    }

    struct EdgeCounter {
        count: usize,
    }
    impl<I> CfgVisitor<I> for EdgeCounter {
        fn visit_edge(&mut self, _cfg: &Cfg<I>, _edge: EdgeId) {
            self.count += 1;
        }
    }

    #[test]
    fn visitor_sees_edges() {
        let mut cfg = Cfg::new();
        let a = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("e"));
        cfg.block_mut(a).instructions_vec_mut().push(ff("a"));
        cfg.add_edge(cfg.entry(), a, EdgeKind::Fallthrough);
        let mut v = EdgeCounter { count: 0 };
        walk_dfs(&cfg, &mut v);
        assert_eq!(v.count, 1);
    }
}
