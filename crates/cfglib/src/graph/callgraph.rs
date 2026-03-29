//! Call graph — interprocedural control flow.
//!
//! Links multiple [`Cfg`]s together via their [`CallSite`](crate::edge::CallSite)
//! edges to form a whole-program call graph.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::cfg::Cfg;
use crate::edge::EdgeKind;

/// Identifier for a function node in the call graph.
pub type FunctionId = usize;

/// A node in the call graph representing one function / CFG.
#[derive(Debug, Clone)]
pub struct FunctionNode {
    /// Unique identifier within the call graph.
    pub id: FunctionId,
    /// Symbolic name (e.g. function name or address label).
    pub name: String,
}

/// A call edge in the call graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallEdge {
    /// Caller function.
    pub caller: FunctionId,
    /// Callee function.
    pub callee: FunctionId,
    /// Whether this is a tail call.
    pub is_tail_call: bool,
}

/// A call graph linking multiple functions.
///
/// # Examples
///
/// ```
/// use cfglib::CallGraph;
///
/// let mut cg = CallGraph::new();
/// let main = cg.add_function("main");
/// let helper = cg.add_function("helper");
/// cg.add_call(main, helper, false);
///
/// assert_eq!(cg.num_functions(), 2);
/// assert!(cg.callees(main).contains(&helper));
/// assert!(cg.callers(helper).contains(&main));
/// ```
#[derive(Debug, Clone)]
pub struct CallGraph {
    nodes: Vec<FunctionNode>,
    name_to_id: BTreeMap<String, FunctionId>,
    out_edges: Vec<BTreeSet<FunctionId>>,
    in_edges: Vec<BTreeSet<FunctionId>>,
    edges: Vec<CallEdge>,
}

impl CallGraph {
    /// Create an empty call graph.
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            name_to_id: BTreeMap::new(),
            out_edges: Vec::new(),
            in_edges: Vec::new(),
            edges: Vec::new(),
        }
    }
    /// Add a function node, returning its id.
    pub fn add_function(&mut self, name: &str) -> FunctionId {
        if let Some(&id) = self.name_to_id.get(name) {
            return id;
        }
        let id = self.nodes.len();
        self.nodes.push(FunctionNode {
            id,
            name: String::from(name),
        });
        self.name_to_id.insert(String::from(name), id);
        self.out_edges.push(BTreeSet::new());
        self.in_edges.push(BTreeSet::new());
        id
    }
    /// Record a call from `caller` to `callee`.
    pub fn add_call(&mut self, caller: FunctionId, callee: FunctionId, tail: bool) {
        self.out_edges[caller].insert(callee);
        self.in_edges[callee].insert(caller);
        self.edges.push(CallEdge {
            caller,
            callee,
            is_tail_call: tail,
        });
    }
    /// Functions called by `id`.
    pub fn callees(&self, id: FunctionId) -> &BTreeSet<FunctionId> {
        &self.out_edges[id]
    }
    /// Functions that call `id`.
    pub fn callers(&self, id: FunctionId) -> &BTreeSet<FunctionId> {
        &self.in_edges[id]
    }
    /// Look up a function node by id.
    pub fn function(&self, id: FunctionId) -> &FunctionNode {
        &self.nodes[id]
    }
    /// Look up a function id by name.
    pub fn function_by_name(&self, n: &str) -> Option<FunctionId> {
        self.name_to_id.get(n).copied()
    }
    /// Number of function nodes.
    pub fn num_functions(&self) -> usize {
        self.nodes.len()
    }
    /// All call edges.
    pub fn edges(&self) -> &[CallEdge] {
        &self.edges
    }
    /// All function nodes.
    pub fn functions(&self) -> &[FunctionNode] {
        &self.nodes
    }
    /// Leaf functions (no callees).
    pub fn leaf_functions(&self) -> Vec<FunctionId> {
        (0..self.nodes.len())
            .filter(|&i| self.out_edges[i].is_empty())
            .collect()
    }
    /// Root functions (no callers).
    pub fn root_functions(&self) -> Vec<FunctionId> {
        (0..self.nodes.len())
            .filter(|&i| self.in_edges[i].is_empty())
            .collect()
    }
    /// Whether `id` is recursive (directly or via mutual recursion).
    pub fn is_recursive(&self, id: FunctionId) -> bool {
        self.out_edges[id].contains(&id)
            || self
                .sccs()
                .iter()
                .any(|scc| scc.len() > 1 && scc.contains(&id))
    }
    /// Build a call graph by scanning CFGs for call edges.
    pub fn build_from_cfgs<I>(cfgs: &[(&str, &Cfg<I>)]) -> Self {
        let mut cg = Self::new();
        for &(name, _) in cfgs {
            cg.add_function(name);
        }
        for &(cn, cfg) in cfgs {
            let cid = cg.name_to_id[cn];
            for edge in cfg.edges() {
                if matches!(edge.kind(), EdgeKind::Call | EdgeKind::IndirectCall)
                    && let Some(cs) = edge.call_site()
                    && let Some(tgt) = &cs.target_name
                    && let Some(&tid) = cg.name_to_id.get(tgt.as_str())
                {
                    cg.add_call(cid, tid, cs.is_tail_call);
                }
            }
        }
        cg
    }
    /// Topological order (returns `None` if cycles exist).
    pub fn topological_order(&self) -> Option<Vec<FunctionId>> {
        let n = self.nodes.len();
        let mut vis = vec![false; n];
        let mut stk = vec![false; n];
        let mut out = Vec::with_capacity(n);
        fn go(
            id: usize,
            a: &[BTreeSet<usize>],
            v: &mut [bool],
            s: &mut [bool],
            o: &mut Vec<usize>,
        ) -> bool {
            v[id] = true;
            s[id] = true;
            for &c in &a[id] {
                if s[c] {
                    return false;
                }
                if !v[c] && !go(c, a, v, s, o) {
                    return false;
                }
            }
            s[id] = false;
            o.push(id);
            true
        }
        for i in 0..n {
            if !vis[i] && !go(i, &self.out_edges, &mut vis, &mut stk, &mut out) {
                return None;
            }
        }
        out.reverse();
        Some(out)
    }
    /// Strongly connected components (Tarjan's algorithm).
    pub fn sccs(&self) -> Vec<Vec<FunctionId>> {
        let n = self.nodes.len();
        let mut state = TarjanState {
            idx: 0,
            stack: Vec::new(),
            on_stack: vec![false; n],
            indices: vec![u32::MAX; n],
            lowlinks: vec![u32::MAX; n],
            result: Vec::new(),
        };
        for i in 0..n {
            if state.indices[i] == u32::MAX {
                state.visit(i, &self.out_edges);
            }
        }
        state.result
    }
}

struct TarjanState {
    idx: u32,
    stack: Vec<usize>,
    on_stack: Vec<bool>,
    indices: Vec<u32>,
    lowlinks: Vec<u32>,
    result: Vec<Vec<usize>>,
}

impl TarjanState {
    fn visit(&mut self, v: usize, adj: &[BTreeSet<usize>]) {
        self.indices[v] = self.idx;
        self.lowlinks[v] = self.idx;
        self.idx += 1;
        self.stack.push(v);
        self.on_stack[v] = true;
        for &w in &adj[v] {
            if self.indices[w] == u32::MAX {
                self.visit(w, adj);
                self.lowlinks[v] = core::cmp::min(self.lowlinks[v], self.lowlinks[w]);
            } else if self.on_stack[w] {
                self.lowlinks[v] = core::cmp::min(self.lowlinks[v], self.indices[w]);
            }
        }
        if self.lowlinks[v] == self.indices[v] {
            let mut scc = Vec::new();
            loop {
                let w = self.stack.pop().unwrap();
                self.on_stack[w] = false;
                scc.push(w);
                if w == v {
                    break;
                }
            }
            self.result.push(scc);
        }
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
    use crate::cfg::Cfg;
    use crate::edge::{CallSite, EdgeKind};
    use crate::test_util::ff;

    #[test]
    fn build_from_cfgs_resolves_calls() {
        let mut mc = Cfg::new();
        let b = mc.new_block();
        mc.block_mut(mc.entry())
            .instructions_vec_mut()
            .push(ff("call"));
        let eid = mc.add_edge(mc.entry(), b, EdgeKind::Call);
        mc.edge_mut(eid)
            .set_call_site(Some(CallSite::named("helper")));
        let mut hc = Cfg::new();
        hc.block_mut(hc.entry())
            .instructions_vec_mut()
            .push(ff("ret"));
        let cg = CallGraph::build_from_cfgs(&[("main", &mc), ("helper", &hc)]);
        let m = cg.function_by_name("main").unwrap();
        let h = cg.function_by_name("helper").unwrap();
        assert!(cg.callees(m).contains(&h));
    }
    #[test]
    fn topo_order() {
        let mut cg = CallGraph::new();
        let a = cg.add_function("a");
        let b = cg.add_function("b");
        cg.add_call(a, b, false);
        assert_eq!(cg.topological_order().unwrap(), vec![a, b]);
    }
    #[test]
    fn cycle_no_topo() {
        let mut cg = CallGraph::new();
        let a = cg.add_function("a");
        let b = cg.add_function("b");
        cg.add_call(a, b, false);
        cg.add_call(b, a, false);
        assert!(cg.topological_order().is_none());
    }
    #[test]
    fn sccs_mutual() {
        let mut cg = CallGraph::new();
        let a = cg.add_function("a");
        let b = cg.add_function("b");
        cg.add_call(a, b, false);
        cg.add_call(b, a, false);
        assert!(cg.sccs().iter().any(|s| s.len() > 1));
    }
    #[test]
    fn leaf_and_root() {
        let mut cg = CallGraph::new();
        let m = cg.add_function("main");
        let l = cg.add_function("leaf");
        cg.add_call(m, l, false);
        assert_eq!(cg.leaf_functions(), vec![l]);
        assert_eq!(cg.root_functions(), vec![m]);
    }
}
