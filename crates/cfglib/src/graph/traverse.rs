//! Graph traversal iterators: DFS, BFS, and derived orderings.

extern crate alloc;
use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;

impl<I> Cfg<I> {
    /// Depth-first preorder traversal starting from the entry block.
    pub fn dfs_preorder(&self) -> Vec<BlockId> {
        let mut visited = vec![false; self.num_blocks()];
        let mut order = Vec::with_capacity(self.num_blocks());
        let mut stack = vec![self.entry];

        while let Some(id) = stack.pop() {
            if visited[id.index()] {
                continue;
            }

            visited[id.index()] = true;
            order.push(id);

            // Push successors in reverse so the first successor is visited first.
            // Collect into a small buffer to reverse iteration order.
            let succs: Vec<BlockId> = self.successors(id).collect();
            for &s in succs.iter().rev() {
                if !visited[s.index()] {
                    stack.push(s);
                }
            }
        }
        order
    }

    /// Depth-first postorder traversal starting from the entry block.
    ///
    /// Uses an explicit stack to avoid stack overflow on deep graphs.
    pub fn dfs_postorder(&self) -> Vec<BlockId> {
        let mut visited = vec![false; self.num_blocks()];
        let mut order = Vec::with_capacity(self.num_blocks());
        let mut stack: Vec<(BlockId, bool)> = vec![(self.entry, false)];

        while let Some((id, processed)) = stack.pop() {
            if processed {
                order.push(id);
                continue;
            }
            if visited[id.index()] {
                continue;
            }
            visited[id.index()] = true;
            stack.push((id, true));
            // Push successors in reverse so leftmost is visited first.
            let succs: Vec<BlockId> = self.successors(id).collect();
            for &s in succs.iter().rev() {
                if !visited[s.index()] {
                    stack.push((s, false));
                }
            }
        }

        order
    }

    /// Reverse postorder — a topological ordering useful for data-flow analysis.
    pub fn reverse_postorder(&self) -> Vec<BlockId> {
        let mut rpo = self.dfs_postorder();
        rpo.reverse();
        rpo
    }

    /// Breadth-first traversal starting from the entry block.
    pub fn bfs(&self) -> Vec<BlockId> {
        let mut visited = vec![false; self.num_blocks()];
        let mut order = Vec::with_capacity(self.num_blocks());
        let mut queue = VecDeque::new();

        visited[self.entry.index()] = true;
        queue.push_back(self.entry);

        while let Some(id) = queue.pop_front() {
            order.push(id);
            for s in self.successors(id) {
                if !visited[s.index()] {
                    visited[s.index()] = true;
                    queue.push_back(s);
                }
            }
        }
        order
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    use alloc::vec;

    use crate::cfg::Cfg;
    use crate::edge::EdgeKind;
    use crate::test_util::ff;

    #[test]
    fn single_block_traversals() {
        let mut cfg = Cfg::new();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("a"));

        assert_eq!(cfg.dfs_preorder(), vec![cfg.entry()]);
        assert_eq!(cfg.dfs_postorder(), vec![cfg.entry()]);
        assert_eq!(cfg.reverse_postorder(), vec![cfg.entry()]);
        assert_eq!(cfg.bfs(), vec![cfg.entry()]);
    }

    #[test]
    fn linear_chain_order() {
        let mut cfg = Cfg::new();
        let b = cfg.new_block();
        let c = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("a"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.block_mut(c).instructions_vec_mut().push(ff("c"));
        cfg.add_edge(cfg.entry(), b, EdgeKind::Fallthrough);
        cfg.add_edge(b, c, EdgeKind::Fallthrough);

        let pre = cfg.dfs_preorder();
        assert_eq!(pre, vec![cfg.entry(), b, c]);

        let post = cfg.dfs_postorder();
        assert_eq!(post, vec![c, b, cfg.entry()]);

        let rpo = cfg.reverse_postorder();
        assert_eq!(rpo, vec![cfg.entry(), b, c]);

        let bfs = cfg.bfs();
        assert_eq!(bfs, vec![cfg.entry(), b, c]);
    }

    #[test]
    fn diamond_traversal_visits_all() {
        let mut cfg = Cfg::new();
        let a = cfg.new_block();
        let b = cfg.new_block();
        let merge = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("entry"));
        cfg.block_mut(a).instructions_vec_mut().push(ff("a"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.block_mut(merge)
            .instructions_vec_mut()
            .push(ff("merge"));
        cfg.add_edge(cfg.entry(), a, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), b, EdgeKind::ConditionalFalse);
        cfg.add_edge(a, merge, EdgeKind::Fallthrough);
        cfg.add_edge(b, merge, EdgeKind::Fallthrough);

        let pre = cfg.dfs_preorder();
        assert_eq!(pre.len(), 4);
        assert_eq!(pre[0], cfg.entry());
        assert!(pre.contains(&a));
        assert!(pre.contains(&b));
        assert!(pre.contains(&merge));

        let bfs_order = cfg.bfs();
        assert_eq!(bfs_order.len(), 4);
        assert_eq!(bfs_order[0], cfg.entry());
    }

    #[test]
    fn unreachable_block_not_visited() {
        let mut cfg = Cfg::new();
        let reachable = cfg.new_block();
        let orphan = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("entry"));
        cfg.block_mut(reachable)
            .instructions_vec_mut()
            .push(ff("r"));
        cfg.block_mut(orphan)
            .instructions_vec_mut()
            .push(ff("dead"));
        cfg.add_edge(cfg.entry(), reachable, EdgeKind::Fallthrough);

        let pre = cfg.dfs_preorder();
        assert_eq!(pre.len(), 2);
        assert!(!pre.contains(&orphan));

        let bfs_order = cfg.bfs();
        assert_eq!(bfs_order.len(), 2);
        assert!(!bfs_order.contains(&orphan));
    }

    #[test]
    fn postorder_visits_children_before_parent() {
        let mut cfg: Cfg<crate::test_util::MockInst> = Cfg::new();
        let b = cfg.new_block();
        let c = cfg.new_block();
        cfg.add_edge(cfg.entry(), b, EdgeKind::Fallthrough);
        cfg.add_edge(b, c, EdgeKind::Fallthrough);

        let post = cfg.dfs_postorder();
        let pos_entry = post.iter().position(|&x| x == cfg.entry()).unwrap();
        let pos_c = post.iter().position(|&x| x == c).unwrap();
        assert!(
            pos_c < pos_entry,
            "child c should appear before entry in postorder"
        );
    }
}
