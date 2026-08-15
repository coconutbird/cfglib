//! Generic graph and dataflow framework for code intelligence and program analysis.
//!
//! [`DirectedGraph`] stores arbitrary node and edge payloads for value-flow,
//! symbol, type-relation, import, call, and grammar graphs. Algorithms consume
//! [`DirectedGraphView`] / [`RootedGraphView`], so consumer-owned graph stores
//! participate without migrating their data. [`Cfg<I>`] adds basic-block and
//! control-flow semantics when the graph really is a program CFG, and every
//! instruction-adjacent axis — variables, constants, operators, effects,
//! branch targets, callees — is consumer-typed rather than imposed by the
//! library.
//!
//! # Quick start
//!
//! Direct structural construction is the primary front door — no trait is
//! required to build, verify, analyse, or render a CFG:
//!
//! ```rust
//! use cfglib::{Cfg, DominatorTree, EdgeKind, verify};
//!
//! // A source frontend lowers its syntax tree straight into blocks.
//! let mut cfg = Cfg::<&'static str>::new();
//! let then_block = cfg.new_block();
//! let merge = cfg.new_block();
//! cfg.block_mut(cfg.entry()).push("if x > 0");
//! cfg.block_mut(then_block).push("y = x");
//! cfg.add_edge(cfg.entry(), then_block, EdgeKind::ConditionalTrue);
//! cfg.add_edge(cfg.entry(), merge, EdgeKind::ConditionalFalse);
//! cfg.add_edge(then_block, merge, EdgeKind::Fallthrough);
//!
//! assert!(verify(&cfg).is_ok());
//! let dominators = DominatorTree::compute(&cfg);
//! assert!(dominators.dominates(cfg.entry(), merge));
//! ```
//!
//! Frontends with a flat, structured instruction stream (shader bytecode,
//! structured ISAs) can instead implement [`FlowControl`] and use
//! [`CfgBuilder::build`]; [`resolve_jump_edges`] wires explicit gotos
//! afterwards via [`JumpTargets`].
//!
//! # Extension contracts
//!
//! [`DirectedGraph`] owns arbitrary graph storage without requiring a consumer
//! trait. Existing graph stores implement [`DenseNodeId`] and
//! [`DirectedGraphView`] (plus [`RootedGraphView`], or the [`Rooted`]
//! adapter, for entry-requiring algorithms) to reuse the generic algorithms.
//! Instruction types implement progressively richer traits only when they
//! need CFG or dataflow facilities — every associated type below is the
//! consumer's own:
//!
//! ```text
//! DirectedGraph<N, E>       (owned arbitrary graph; no adapter trait)
//! DirectedGraphView         (existing consumer-owned graph storage)
//!   └─ RootedGraphView     (adds a distinguished entry node; `Rooted` adapts)
//!
//! FlowControl               (required only by CfgBuilder)
//!   └─ JumpTargets         (optional — explicit goto/label wiring, Target)
//!
//! InstrInfo<Variable = V>  (optional — native IR variables for dataflow)
//!   ├─ EffectInfo          (optional — side effects, Effect; purity + DCE)
//!   ├─ Predicated          (optional — guarded execution; lift_predicated)
//!   ├─ CopySource          (optional — copy propagation)
//!   ├─ ConstantFolder      (optional — constant propagation, Const)
//!   ├─ ExprInstr           (optional — expression trees, Operator + Const)
//!   └─ ValueNumberInfo     (optional — value numbering, Operation)
//!
//! DisplayInstr              (optional — rendering only: DOT, pseudocode)
//! CallInfo                  (optional — call graphs, Callee)
//! SwitchSource              (optional — switch table recovery, Target)
//! ```
//!
//! Additionally, [`Problem`] is the trait for pluggable instruction-level
//! dataflow analyses, [`NodeProblem`] its node-level counterpart over any
//! graph view, and [`Emitter`] the trait for linearization output.
//!
//! # Contracts
//!
//! - Blocks may be empty, may lack explicit terminator instructions, and
//!   unreachable blocks are legal (dead code after a return/goto).
//! - [`Cfg::blocks`] iterates in allocation order and [`Cfg::edges`] in
//!   insertion order; both orders are stable and part of the API.
//! - [`ProgramPoint`] instruction indices are positions, not identities:
//!   [`Cfg::split_block`] and instruction edits invalidate them. Persist
//!   consumer-keyed results (e.g. by syntax-node id), not program points.
//! - Analyses index node identities densely; consumer block anchors
//!   (source ranges, syntax nodes) belong in dense-indexed side tables or
//!   in the instruction payload itself.

#![no_std]
#![warn(missing_docs)]

pub(crate) fn usize_to_f64(value: usize) -> f64 {
    if let Ok(value) = u32::try_from(value) {
        return f64::from(value);
    }

    let half = usize_to_f64(value / 2);
    half * 2.0 + f64::from(u8::from(value & 1 == 1))
}

// ── Modules ─────────────────────────────────────────────────────────

// Core types.
pub mod block;
pub mod builder;
pub mod cfg;
pub mod display;
pub mod edge;
pub mod flow;
pub mod region;

// Graph algorithms.
pub mod graph;

// Dataflow framework, analyses, and SSA.
pub mod dataflow;

// Higher-level analyses (switch recovery, expression trees, purity).
pub mod analysis;

// AST lifting / structural recovery.
pub mod ast;

// Transforms (cleanup, critical edges, DCE, linearization).
pub mod transform;

// Shared test utilities (crate-internal).
#[cfg(test)]
pub(crate) mod test_util;

// ── Re-exports: Core ────────────────────────────────────────────────

pub use analysis::purity::{Purity, all_block_purities, block_purity, cfg_purity};
pub use analysis::switch_table::{
    JumpTable, SwitchRecovery, SwitchSource, detect_switch_tables, recover_switch_tables,
};
pub use ast::{AstNode, lift, lift_predicated};
pub use block::{BasicBlock, BlockId};
pub use builder::{BuildError, CfgBuilder, JumpResolution, resolve_jump_edges};
pub use cfg::Cfg;
pub use display::DisplayInstr;
pub use edge::{Edge, EdgeId, EdgeKind};
pub use flow::{CallInfo, FlowControl, FlowEffect, JumpTargets};
pub use region::{
    Cleanup, CompletionReason, Continuation, Handler, HandlerFilters, HandlerKind, HandlerRef,
    Region, RegionId,
};

// ── Re-exports: Dataflow framework & SSA ────────────────────────────

pub use dataflow::defuse::DefUseChains;
pub use dataflow::fixpoint::{Direction, FixpointResult, Problem};
pub use dataflow::liveness::Liveness;
pub use dataflow::node_fixpoint::{NodeFacts, NodeProblem, solve_node_problem};
pub use dataflow::reaching::{ReachingDef, ReachingDefs};
pub use dataflow::ssa::{
    DominanceFrontiers, PhiPlacement, PhiPlacements, SsaBlock, SsaForm, SsaInstruction, SsaPhi,
    SsaValue, SsaVersion, build_ssa, place_phis,
};
pub use dataflow::{DefSite, EffectInfo, InstrInfo, Predicated, ProgramPoint, UseSite, VariableId};

// ── Re-exports: Graph algorithms ────────────────────────────────────

pub use graph::callgraph::{
    CallMetadata, FunctionNode, build_call_graph, find_function, is_recursive_function,
    propagate_summaries,
};
pub use graph::cdg::control_dependence_graph;
pub use graph::diff::{BlockFingerprint, BlockMatch, CfgDiff, cfg_diff};
pub use graph::directed::{DirectedEdge, DirectedGraph, NodeId};
pub use graph::dominator::DominatorTree;
pub use graph::dot::write_view_dot;
pub use graph::edge_traverse::{EdgeStep, breadth_first_edges, shortest_path_edges, walk_edges};
pub use graph::eh::{EhBlockKind, EhEdge, EhModel, build_eh_model, cleanup_blocks, landing_pads};
pub use graph::horn::HornClauses;
pub use graph::inc_dom::{IncrementalUpdate, update_after_edge_insert, update_after_edge_remove};
pub use graph::interval::{Interval, IntervalAnalysis, interval_analysis};
pub use graph::keyed::KeyedGraph;
pub use graph::loopnest::{LoopNestNode, LoopNestingTree};
pub use graph::pdg::{DependenceKind, DependenceNode, program_dependence_graph};
pub use graph::reverse::reverse_cfg;
pub use graph::scc::{Scc, SccResult, condensation, tarjan_scc};
pub use graph::structure::{
    BackEdge, CanonicalLoop, NaturalLoop, canonicalize_loops, detect_loops, detect_loops_tagged,
    find_back_edges, find_back_edges_tagged, insert_preheader, loop_exit_blocks,
};
pub use graph::traverse::{
    CommonAncestor, TraversalDirection, breadth_first, common_ancestors, depth_first_postorder,
    depth_first_preorder, nearest_common_ancestor, reachable, reverse_postorder, shortest_path,
    topological_sort,
};
pub use graph::verify::{VerifyError, VerifyResult, verify, verify_view};
pub use graph::view::{DenseNodeId, DirectedGraphView, Reversed, Rooted, RootedGraphView};
pub use region::RegionIndex;

// ── Re-exports: Analyses ────────────────────────────────────────────

pub use analysis::alias::{AliasSets, MemoryInfo, MemoryOp, alias_analysis};
pub use analysis::expr::{
    BlockExprTrees, ExprInstr, ExprNode, recover_block_expressions, recover_expressions,
};
pub use analysis::metrics::{
    CfgMetrics, GraphMetrics, block_nesting_depths, cfg_block_nesting_depths, cfg_metrics,
    graph_metrics,
};
pub use analysis::pattern::{CfgPattern, detect_cfg_patterns, detect_patterns};
pub use analysis::profile::CfgProfile;
pub use analysis::tailcall::{TailCall, detect_explicit_tail_calls, detect_tail_calls};
pub use analysis::valuenumber::{
    BlockValueNumbers, ValueNumber, ValueNumberInfo, ValueNumbering, count_redundant,
    global_value_numbering, local_value_numbering,
};
pub use dataflow::abs_int::{AbstractDomain, AbstractResult, Lattice, abstract_interpret};
pub use dataflow::constprop::{ConstPropProblem, ConstValue, ConstantFolder, constant_propagation};
pub use dataflow::copyprop::{CopyPropResult, CopySource, copy_propagation};
pub use dataflow::memssa::{MemoryAccess, MemoryEffect, MemorySSA, build_memory_ssa};
pub use dataflow::phi_web::{PhiWeb, PhiWebs, compute_phi_webs};
pub use dataflow::sccp::{SccpResult, sccp};
pub use dataflow::ssa_destruct::{PhiCopy, copies_by_predecessor, eliminate_phis};

// ── Re-exports: Transforms & linearization ──────────────────────────

pub use transform::coloring::{ColorAssignment, build_interference_graph, color_graph};
pub use transform::contract::{contract_edge, split_node};
pub use transform::loops::{RotationResult, find_loop_invariants, rotate_loop};
pub use transform::pre::{PreResult, analyse_pre, eliminate_pre};
pub use transform::{
    BlockOrder, Emitter, LinearInst, dead_code_elimination, linearize, merge_blocks,
    remove_empty_blocks, remove_unreachable, simplify, split_critical_edges,
};
