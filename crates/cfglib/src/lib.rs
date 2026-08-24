//! Generic graph and dataflow framework for code intelligence and program analysis.
//!
//! [`DirectedGraph`] stores arbitrary node and edge payloads for value-flow,
//! symbol, type-relation, import, call, and grammar graphs. Algorithms consume
//! [`DirectedGraphView`] / [`RootedGraphView`], while [`EdgeGraphView`] and
//! [`FilteredEdges`] retain edge identity and data without rebuilding, so
//! consumer-owned graph stores participate without migrating their data.
//! [`breadth_first_events`] and [`depth_first_events`] expose traversal-tree
//! structure for dense graphs; [`open_breadth_first_events`] and
//! [`open_depth_first_events`] provide the corresponding discovery streams
//! for lazily generated node spaces.
//! [`Cfg<I, E>`] adds basic-block, control-flow, and caller-owned edge metadata
//! when the graph really is a program CFG, and every
//! instruction-adjacent axis — variables, constants, operators, effects,
//! branch targets, callees — is consumer-typed rather than imposed by the
//! library.
//!
//! # Quick start
//!
//! Direct structural construction is the primary front door — no trait is
//! required to build, verify, analyze, or render a CFG:
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
//! [`DirectedGraphView`] (plus [`EdgeGraphView`] for edge-sensitive algorithms
//! and [`RootedGraphView`], or the [`Rooted`] adapter, for entry-requiring
//! algorithms) to reuse the generic algorithms.
//! Instruction types implement progressively richer traits only when they
//! need CFG or dataflow facilities — every associated type below is the
//! consumer's own:
//!
//! ```text
//! DirectedGraph<N, E>       (owned arbitrary graph; no adapter trait)
//! DirectedGraphView         (existing consumer-owned graph storage)
//!   ├─ EdgeGraphView        (stable edge identity, endpoints, data)
//!   └─ RootedGraphView      (adds a distinguished entry node; `Rooted` adapts)
//!
//! FlowControl               (required only by CfgBuilder)
//!   └─ JumpTargets          (optional — explicit goto/label wiring, Target)
//!
//! InstrInfo<Variable = V>   (optional — native IR variables for dataflow)
//!   ├─ EffectInfo           (optional — side effects, Effect; purity + DCE)
//!   ├─ Predicated           (optional — guarded execution; lift_predicated)
//!   ├─ CopySource           (optional — copy propagation)
//!   ├─ ConstantFolder       (optional — constant propagation, Const)
//!   ├─ ExprInstr            (optional — expression trees, Operator + Const)
//!   └─ ValueNumberInfo      (optional — value numbering, Operator)
//!
//! DisplayInstr              (optional — rendering only: DOT, pseudocode)
//! CallInfo                  (optional — call graphs, Callee)
//! SwitchSource              (optional — switch table recovery, Target)
//! ```
//!
//! Additionally, [`Problem`] is the trait for pluggable instruction-level
//! dataflow analyses (run by [`solve_problem`]), [`NodeProblem`] its
//! node-level counterpart over any graph view (run by [`solve_node_problem`]),
//! [`EdgeProblem`] its edge-sensitive counterpart (run by
//! [`solve_edge_problem`]), [`TryEdgeProblem`] the error-preserving edge
//! variant, and [`Emitter`] the trait for linearization output.
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

pub mod analysis;
pub mod ast;
pub mod block;
pub mod builder;
pub mod cfg;
pub mod dataflow;
pub mod display;
pub mod edge;
pub mod exception;
pub mod flow;
pub mod graph;
pub mod region;
pub mod rewrite;
pub mod transform;

#[cfg(test)]
pub(crate) mod test_util;

pub use analysis::alias::{AliasSets, MemoryInfo, MemoryOp};
pub use analysis::expr::{
    BlockExprTrees, ExprInstr, ExprNode, recover_block_expressions, recover_expressions,
};
pub use analysis::metrics::{
    CfgMetrics, GraphMetrics, block_nesting_depths, cfg_block_nesting_depths,
};
pub use analysis::pattern::{CfgPattern, detect_cfg_patterns, detect_patterns};
pub use analysis::profile::{CfgProfile, set_uniform_edge_weights};
pub use analysis::purity::{Purity, block_purities, block_purity, cfg_purity};
pub use analysis::switch_table::{
    JumpTable, SwitchRecovery, SwitchSource, SwitchTargets, detect_switch_tables,
    recover_switch_tables,
};
pub use analysis::tail_call::{TailCall, detect_explicit_tail_calls, detect_tail_calls};
pub use analysis::value_numbering::{
    BlockValueNumbers, ValueNumber, ValueNumberInfo, ValueNumbering,
};
pub use ast::{AstNode, CatchHandler, SwitchCase, lift, lift_predicated};
pub use block::{BasicBlock, BlockId};
pub use builder::{BuildError, CfgBuilder, JumpResolution, resolve_jump_edges};
pub use cfg::{Cfg, Predecessors, SplitPointError, Successors};
pub use dataflow::abstract_interpretation::{
    AbstractDomain, AbstractFacts, Lattice, abstract_interpret,
};
pub use dataflow::constant_propagation::{
    ConstFact, ConstPropProblem, ConstValue, ConstantFolder, constant_propagation,
};
pub use dataflow::copy_propagation::{CopyPropagationStats, CopySource, copy_propagation};
pub use dataflow::def_use::DefUseChains;
pub use dataflow::edge_fixpoint::{
    EdgeFacts, EdgeProblem, TryEdgeProblem, solve_edge_problem, solve_edge_problem_from,
    solve_edge_problem_from_with_config, solve_edge_problem_with_config, try_solve_edge_problem,
    try_solve_edge_problem_from, try_solve_edge_problem_from_with_config,
    try_solve_edge_problem_with_config,
};
pub use dataflow::fixpoint::{
    Direction, Facts, Problem, SolveConfig, SolveError, TryProblem, TrySolveError, solve_problem,
    solve_problem_from, solve_problem_from_with_config, solve_problem_with_config,
    try_solve_problem, try_solve_problem_from, try_solve_problem_from_with_config,
    try_solve_problem_with_config,
};
pub use dataflow::liveness::{Liveness, LivenessProblem};
pub use dataflow::memory_ssa::{MemoryAccess, MemoryEffect, MemorySSA, MemoryVersion};
pub use dataflow::node_fixpoint::{
    NodeFacts, NodeProblem, TryNodeProblem, solve_node_problem, solve_node_problem_from,
    solve_node_problem_from_with_config, solve_node_problem_with_config, try_solve_node_problem,
    try_solve_node_problem_from, try_solve_node_problem_from_with_config,
    try_solve_node_problem_with_config,
};
pub use dataflow::phi_web::{PhiWeb, PhiWebs};
pub use dataflow::reaching::{ReachingDef, ReachingDefs, ReachingDefsProblem};
pub use dataflow::sccp::SccpAnalysis;
pub use dataflow::ssa::{
    DominanceFrontiers, PhiPlacement, PhiPlacements, SsaBlock, SsaForm, SsaInstruction, SsaPhi,
    SsaValue, SsaVersion,
};
pub use dataflow::ssa_destruction::{PhiCopy, copies_by_predecessor, eliminate_phis};
pub use dataflow::{DefSite, EffectInfo, InstrInfo, Predicated, ProgramPoint, UseSite, VariableId};
pub use display::DisplayInstr;
pub use edge::{Edge, EdgeId, EdgeKind};
pub use exception::{
    ClrExceptionRegion, ClrHandler, ClrHandlerKind, ExceptionDisposition, ExceptionFlow,
    ExceptionPhase, SehExceptionRegion, SehHandler, SehHandlerKind, SehRegistration,
    SehRegistrationChain, VectoredExceptionModel, VectoredHandler, VectoredHandlerId,
    VectoredHandlerKind, VectoredHandlerOrder, VehModel, install_clr_region, install_seh_region,
};
pub use flow::{CallInfo, FlowControl, FlowEffect, JumpTargets};
pub use graph::call_graph::{
    CallMetadata, FunctionNode, call_graph, find_function, is_recursive_function,
    propagate_summaries,
};
pub use graph::cdg::control_dependence_graph;
pub use graph::diff::{BlockFingerprint, BlockMatch, CfgDiff};
pub use graph::directed::{DirectedEdge, DirectedGraph, NodeId};
pub use graph::dominator::DominatorTree;
pub use graph::dot::{to_view_dot, write_view_dot};
pub use graph::edge_traverse::{
    EdgeStep, breadth_first_edges, breadth_first_edges_with, breadth_first_view_edges,
    breadth_first_view_edges_with, depth_first_edges, depth_first_edges_with,
    depth_first_view_edges, depth_first_view_edges_with, shortest_path_edges,
    shortest_path_view_edges, walk_edges, walk_view_edges,
};
pub use graph::edge_view::{DenseEdgeId, EdgeGraphView, EdgeRef, FilteredEdges};
pub use graph::eh::{EhBlockKind, EhEdge, EhEdgeKind, EhModel};
pub use graph::horn::HornClauses;
pub use graph::interval::{Interval, IntervalAnalysis};
pub use graph::keyed::KeyedGraph;
pub use graph::loop_nest::{LoopNestNode, LoopNestingTree};
pub use graph::open::{
    OpenBfsConfig, OpenBfsEvent, OpenDfsConfig, OpenDfsEvent, OpenSearchConfig, follow,
    follow_path, open_breadth_first_events, open_depth_first_events, open_search,
};
pub use graph::pdg::{DependenceKind, DependenceNode, program_dependence_graph};
pub use graph::reducible::make_reducible;
pub use graph::relax::min_label_relaxation;
pub use graph::reverse::reverse_cfg;
pub use graph::scc::{
    Scc, SccDecomposition, condensation, condensation_of, kosaraju_scc, tarjan_scc,
};
pub use graph::search::{
    BfsEvent, DfsEvent, EpochMarks, SearchConfig, SearchOrder, SearchScratch, Visit, VisitedPolicy,
    breadth_first_events, depth_first_events, search, search_with_marks, search_with_scratch,
};
pub use graph::structure::{
    BackEdge, CanonicalLoop, NaturalLoop, canonicalize_loops, detect_loops, detect_loops_tagged,
    find_back_edges, find_back_edges_tagged, insert_preheader, is_reducible, loop_exit_blocks,
};
pub use graph::traverse::{
    CommonAncestor, TraversalDirection, breadth_first, common_ancestors, depth_first_postorder,
    depth_first_preorder, nearest_common_ancestor, reachable, reverse_postorder, shortest_path,
    topological_sort,
};
pub use graph::verify::{
    SemanticValidator, SemanticVerifyReport, VerifyError, VerifyReport, verify, verify_edge_view,
    verify_view, verify_with,
};
pub use graph::view::{DenseNodeId, DirectedGraphView, Reversed, Rooted, RootedGraphView};
pub use region::{
    Cleanup, CompletionReason, Continuation, Handler, HandlerFilters, HandlerKind, HandlerMetadata,
    HandlerRef, HandlerTypes, Region, RegionId, RegionIndex,
};
pub use rewrite::RewriteMap;
pub use transform::cleanup::{
    merge_blocks, merge_blocks_mapped, remove_empty_blocks, remove_empty_blocks_mapped,
    remove_unreachable, remove_unreachable_mapped, simplify, simplify_mapped,
};
pub use transform::coloring::{ColorAssignment, color_graph, interference_graph};
pub use transform::contract::{
    contract_edge, contract_edge_mapped, split_node, split_node_at_points,
    split_node_with_payload_mapped,
};
pub use transform::critical::{
    split_critical_edges, split_critical_edges_mapped, split_critical_edges_with,
};
pub use transform::dce::dead_code_elimination;
pub use transform::linearize::{BlockOrder, Emitter, LinearInst, linearize};
pub use transform::loops::{LoopRotation, find_loop_invariants, rotate_loop};
pub use transform::pre::{PreAnalysis, eliminate_pre};
