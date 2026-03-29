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
    pub fn dfs_postorder(&self) -> Vec<BlockId> {
        let mut visited = vec![false; self.num_blocks()];
        let mut order = Vec::with_capacity(self.num_blocks());
        Self::dfs_postorder_visit(self, self.entry, &mut visited, &mut order);
        order
    }

    fn dfs_postorder_visit(
        cfg: &Cfg<I>,
        id: BlockId,
        visited: &mut Vec<bool>,
        order: &mut Vec<BlockId>,
    ) {
        if visited[id.index()] {
            return;
        }

        visited[id.index()] = true;
        for s in cfg.successors(id) {
            Self::dfs_postorder_visit(cfg, s, visited, order);
        }

        order.push(id);
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
