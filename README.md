# cfglib

Generic, `no_std` graph and dataflow framework for code intelligence, program analysis, decompilation, and compiler infrastructure.

`cfglib` has two graph storage models: an owned directed multigraph for arbitrary code-intelligence relations, and `Cfg<I, E = ()>` for graphs that genuinely need basic blocks, typed control-flow edges, caller-owned edge metadata, and exception regions. Small read-only node and edge view contracts let consumer-owned stores and zero-copy filtered views reuse the algorithms. Every instruction-adjacent axis is consumer-typed rather than imposed by the library: dataflow variables, constants, expression operators, side-effect vocabularies, branch targets, call targets, and edge provenance all come from the adapter — so x86 registers and flags, shader register components, bytecode locals, compiler IR values, and source-language symbols do not need to be flattened into a library-owned numbering scheme, and string literals or symbol ids flow through the analyses as naturally as machine words and addresses. On top of that it ships a compiler-middle-end toolkit: dominator trees, renamed SSA construction, dataflow analyses, value numbering, alias analysis, loop transforms, dead-code elimination, partial redundancy elimination, graph coloring, and structured AST recovery.

Everything is `no_std + alloc` and the core graph structure uses `SmallVec` adjacency lists with tombstone-based edge removal for cache-friendly, arena-stable IDs.

## Quick start

### Direct construction (the primary front door)

No trait is required to build, verify, analyze, or render a CFG. Source
frontends lower their syntax trees straight into blocks:

```rust
use cfglib::{Cfg, DominatorTree, EdgeKind, ReachingDefs, verify};

let mut cfg = Cfg::<Stmt>::new();
let then_block = cfg.new_block();
let merge = cfg.new_block();
cfg.block_mut(cfg.entry()).push(stmt_if);
cfg.block_mut(then_block).push(stmt_assign);
cfg.add_edge(cfg.entry(), then_block, EdgeKind::ConditionalTrue);
cfg.add_edge(cfg.entry(), merge, EdgeKind::ConditionalFalse);
cfg.add_edge(then_block, merge, EdgeKind::Fallthrough);

assert!(verify(&cfg).is_ok());
let dominators = DominatorTree::compute(&cfg);
let reaching = ReachingDefs::compute(&cfg); // needs InstrInfo on Stmt
```

### Builder construction (structured instruction streams)

Frontends with a flat, structured stream (shader bytecode, structured ISAs)
implement `FlowControl` and use `CfgBuilder`; explicit gotos are wired
afterwards through the opt-in `JumpTargets` trait:

```rust
use cfglib::{CfgBuilder, FlowControl, FlowEffect, resolve_jump_edges};

impl FlowControl for Inst {
    fn flow_effect(&self) -> FlowEffect {
        match self.opcode {
            Op::If    => FlowEffect::ConditionalOpen,
            Op::Else  => FlowEffect::ConditionalAlternate,
            Op::EndIf => FlowEffect::ConditionalClose,
            Op::Loop  => FlowEffect::LoopOpen,
            Op::End   => FlowEffect::LoopClose,
            Op::Ret   => FlowEffect::Return,
            _         => FlowEffect::Fallthrough,
        }
    }
}

let mut cfg = CfgBuilder::build(instructions).unwrap();
let resolution = resolve_jump_edges(&mut cfg); // wires goto/label edges
```

## Feature overview

### Generic graph core

`DirectedGraph<N, E>` is the single owned storage type for consumer-defined node and edge payloads, stable IDs, and forward/reverse adjacency. `DirectedGraphView` lets existing graph stores use node algorithms without first migrating their storage; `EdgeGraphView` additionally exposes stable edge identity, view-oriented endpoints, payloads, and ordered adjacency. `FilteredEdges` borrows either representation through an edge predicate without cloning or renumbering anything—for example, the same CFG can be viewed as normal-only flow or full normal-plus-exception flow. `RootedGraphView` adds a distinguished entry node for algorithms that need one (dominance, reachability metrics, loop and interval analysis), and the `Rooted` adapter roots any plain view at a chosen node. This layer is suitable for symbol/reference graphs, value-flow graphs, call graphs, type relations, import graphs, grammar dependencies, and analysis-derived relations; it has no instruction or binary-analysis concepts.

```rust
use cfglib::{DirectedGraph, Rooted, DominatorTree, TraversalDirection, shortest_path};

let mut graph = DirectedGraph::new();
let source = graph.add_node("definition");
let target = graph.add_node("call result");
let edge = graph.add_edge(source, target, ("return", "src/lib.rs", 42));

assert_eq!(graph[edge].payload().0, "return");
assert_eq!(
    shortest_path(&graph, source, target, TraversalDirection::Outgoing),
    Some(vec![source, target]),
);
let dominators = DominatorTree::compute(&Rooted::new(&graph, source));
```

### Control-flow graph (`Cfg<I>`)

| Feature | Description |
|---|---|
| Generic `Cfg<I, E = ()>` | Parameterised over any instruction and caller-owned edge payload; unit payload preserves the original API |
| `no_std` + `alloc` | Runs in embedded, kernel, and WASM environments |
| `CfgBuilder` | Builds a CFG from a flat structured instruction stream (`if/else/endif`, `loop/endloop`, `switch/case/endswitch`, `break`, `continue`) |
| Goto wiring | `resolve_jump_edges` + `JumpTargets` (consumer-typed targets: labels, addresses, syntax nodes) |
| SmallVec adjacency | Stack-allocated successor (2) / predecessor (4) lists; heap only for high fan-out |
| Tombstone edges | `remove_edge()` replaces the slot with `None`; existing `EdgeId`s remain stable |
| Edge metadata | Stable `EdgeId`, endpoints, `EdgeKind`, optional weight, and a caller-owned payload; `ExceptionFlow<M>` standardises search/unwind phase and execute/search/continue disposition while retaining platform metadata |
| Regions | Try/catch/finally regions with `Handler` and `HandlerKind` (Catch, CatchAll, Finally, Fault, Filter); `install_clr_region` and `install_seh_region` import normalized platform clauses |
| Cleanup continuations | `add_continuation` / `set_cleanup_resume` → `Cleanup` (`Continuation` + `CompletionReason`): which route out of a single shared `finally` block a resume edge belongs to |
| Handler metadata | `HandlerMetadata<M>` side table (`HandlerFilters<F>` / `HandlerTypes<T>`) — consumer-typed filter predicates and CLR caught-type tokens keyed by `HandlerRef`, with no type parameter on `Cfg` |
| Native handler state | `SehRegistrationChain<F, H>` models per-thread x86 frame registrations; `VehModel<H>` separately models ordered process-wide vectored exception/continue handlers |
| Subgraph extraction | `subgraph()` or `subgraph_mapped()` with dense O(1) block-id remapping and payload cloning |
| Block splitting | `split_block()`, mapped payload-aware variants, and validated multi-point splitting with automatic stable edge transfer |
| `serde` feature | Optional serialization support |

### Graph algorithms

| Algorithm | Function / Type | Description |
|---|---|---|
| DFS / BFS | `depth_first_preorder`, `depth_first_postorder`, `breadth_first`, and matching `Cfg` methods | Direction-selectable traversals over `DirectedGraphView`; abbreviated `Cfg::dfs_*` / `Cfg::bfs` methods remain compatibility aliases |
| Edge-aware traversal | `breadth_first_view_edges[_with]`, `depth_first_view_edges[_with]`, `shortest_path_view_edges`, and matching owned-graph wrappers | Every distinct edge once with identity + endpoints over any edge view; parallel-edge provenance; `walk_*` names remain breadth-first compatibility aliases |
| Configurable search | `search` + `SearchConfig` (order, visited policy, direction, depth bound) | First-match, pruning (`Visit::Skip`), early exit (`ControlFlow::Break`), and backtracking as configuration; `VisitedPolicy::Path` un-marks on unwind so every route to a node is reported |
| Reusable search marks | `search_with_marks` + `EpochMarks` | The same search with its visited marks in a caller-owned epoch-stamped buffer: a per-root pass allocates marks once instead of an O(node count) buffer per root, and each search still starts from a clean set (epoch bump, O(1)) |
| Reusable search scratch | `search_with_scratch` + `SearchScratch` | The same search with the marks *and* the call's own buffers — seeds, frontier, adjacency — caller-owned, for a pass whose searches are small enough that the call is the cost; marks and buffers both reset on entry |
| Traversal events | `breadth_first_events` → `BfsEvent`; `depth_first_events` → `DfsEvent` | Breadth-first discovery with tree/non-tree edges, or depth-first discover/tree/back/forward-or-cross/finish events in pinned order |
| Open-graph search | `open_search` + `OpenSearchConfig` | The same disciplines over a lazily discovered node space: successors come from a closure, no dense ids (import/re-export chases, ordered emission walks) |
| Open-graph events | `open_breadth_first_events` → `OpenBfsEvent`; `open_depth_first_events` → `OpenDfsEvent` | Breadth-first discovery/refusal events or depth-first discover/finish/refusal events over a lazily discovered node space; path policy can revisit a shared node once per route |
| Alias chase | `follow`, `follow_path` | Out-degree ≤ 1 chase with a hop bound and a full-path cycle guard; the chain, or just its end |
| Shortest path | `shortest_path` (nodes), `shortest_path_edges` (edge witness) | Forward or reverse unweighted witness path |
| Minimum-label relaxation | `min_label_relaxation` | Edge-defined label transfer to a minimum fixpoint; nodes re-expand when their label improves |
| Multi-source reachability | `reachable` | Dense `Vec<bool>` from a seed set, forward or reverse; order-insensitive |
| Nearest common ancestor | `nearest_common_ancestor` | Bidirectional BFS meet of two nodes; smallest combined distance, ties by smallest node id |
| All common ancestors | `common_ancestors` → `Vec<CommonAncestor>` | Every shared node with both hop counts, depth-bounded, in `b`'s BFS discovery order — the consumer-rankable form (MRO linearization, overload preference) |
| Horn-clause derivability | `HornClauses` | AND-OR closure (`head <- b1 & b2`): nullability, all-arguments-constant, all-callers-dead |
| Topological sort | `topological_sort` | Stable ordering or cycle detection |
| Dominator tree | `DominatorTree::compute` (rooted views), `compute_from` (explicit root) | Cooper-Harvey-Kennedy over any graph view |
| Post-dominator tree | `DominatorTree::compute_post` (CFG), `compute_post_from` (any view + explicit exits) | Virtual-exit handling built in |
| Dominance frontiers | `DominanceFrontiers::compute` | For SSA φ-placement |
| Incremental dominators | `update_after_edge_insert`, `update_after_edge_remove` | Recompute + diff |
| Strongly connected components | `tarjan_scc` → `SccDecomposition<N>`, `condensation` → component DAG | Generic iterative Tarjan algorithm, reverse-topological order (leaves first) |
| SCC in topological order | `kosaraju_scc` → `SccDecomposition<N>` | The same partition numbered sources first (`index(u) < index(v)` across every edge); the classic deterministic two-pass algorithm, for budgeted forward closures over the condensation |
| Condensation of a given decomposition | `condensation_of(graph, &SccDecomposition)` → `DirectedGraph<(), ()>` | The component DAG whose node index **is** the given decomposition's component index, either algorithm's; deduplicated edges, and in-degrees plus dependents straight off the graph (the one-pass fixpoint shape) |
| Back-edge detection | `find_back_edges` (dominance, any view), `find_back_edges_tagged` (CFG, honors `Back` tags) | |
| Natural loop detection | `detect_loops` / `detect_loops_tagged` → `Vec<NaturalLoop<N>>` | Header, body, latches, nesting depth |
| Loop nesting tree | `LoopNestingTree::compute` | Parent/child loop hierarchy |
| Control dependence graph | `control_dependence_graph` → `DirectedGraph<N, ()>` | From post-dominator tree, over any view |
| Program dependence graph | `program_dependence_graph` → `DirectedGraph<DependenceNode, DependenceKind>` | Control + def-use edges; reverse traversal performs backward slicing |
| Interval analysis | `IntervalAnalysis::compute` | T1-T2 reduction over rooted views; reducibility test |
| Reducibility transform | `make_reducible` | Node splitting for irreducible CFGs |
| Reverse CFG | `reverse_cfg` | Flip all edges, swap entry/exits |
| Call graph | `call_graph` + `CallInfo`, `propagate_summaries` (callee-first SCC fixpoint) | Consumer-typed callees; interprocedural summary scaffold |
| CFG diff | `CfgDiff::compute` | Structural comparison (bindiff-style fingerprinting), no trait bounds |
| Exception handling model | `EhModel::compute` | Payload-generic CFG input; stable source `EdgeId`, exact handler/unwind/leave/resume/continue kinds, landing pads, cleanup/resume blocks, handler identities, protected-by mapping, and cleanup continuations |
| Integrity verification | `verify`, `verify_view`, `verify_edge_view`; `verify_with` + `SemanticValidator` | Structural node/edge-view checks plus deterministic typed consumer hooks for cardinality, ordering, and provenance rules |
| DOT export | `to_dot` (`DisplayInstr`), `to_dot_with` (bound-free), `write_view_dot` (any view) | Graphviz output with escaped labels |

### Dataflow framework

| Analysis | Function / Type | Description |
|---|---|---|
| Generic fixpoint solver | `solve_problem`, `Problem` trait | Forward or backward, any lattice type |
| Node-level fixpoint | `solve_node_problem`, `NodeProblem` trait | Per-node facts over any graph view (taint, reachability-with-facts) |
| Seeded node fixpoint | `solve_node_problem_from` | Same solver, worklist seeded from a subset — incremental / dirty-region re-solves |
| Edge-sensitive fixpoint | `solve_edge_problem`, `solve_edge_problem_from`, `EdgeProblem` trait | Full or seeded per-edge transfer over any edge view; stable id/data plus physical node pre/post states and deterministic bounded-solve errors |
| Fallible edge-sensitive fixpoint | `try_solve_edge_problem`, `try_solve_edge_problem_from`, `TryEdgeProblem` trait | Preserves consumer boundary, merge, node-transfer, and edge-transfer errors separately from solver limits |
| Reaching definitions | `ReachingDefs::compute` | Which writes reach each point |
| Liveness | `Liveness::compute` | Live-in / live-out at each block |
| Def-use / use-def chains | `DefUseChains::compute` | Bidirectional def↔use links; dead-def detection |
| SSA construction | `SsaForm::compute` | IDF phi placement plus full dominator-tree renaming |
| Phi placement | `PhiPlacements::compute` | Structural IDF phase for consumers that only need placement |
| SSA deconstruction | `eliminate_phis`, `copies_by_predecessor` | φ-to-copy lowering |
| Phi webs | `PhiWebs::compute` | Congruence classes for register coalescing |
| Constant propagation | `constant_propagation`, `ConstantFolder` (associated `Const`) | Top/Const/Bottom lattice over a consumer constant domain — machine words, strings, bools, float bits |
| Sparse conditional constant propagation | `SccpAnalysis::compute` | SSA-based, marks unreachable edges |
| Copy propagation | `copy_propagation`, `CopySource` trait | Chain resolution + dead copy removal |
| Memory SSA | `MemorySSA::compute`, `MemoryEffect` trait | Memory versioning with φ-nodes |
| Abstract interpretation | `abstract_interpret`, `AbstractDomain` trait | Generic abstract domain framework |

### Higher-level analyses

| Analysis | Function / Type | Description |
|---|---|---|
| Expression tree recovery | `recover_expressions`, `ExprInstr` (associated `Operator` + `Const`) | Rebuild expression DAGs from flat instructions |
| Value numbering (local) | `BlockValueNumbers::compute` | Per-block hash-consing |
| Value numbering (global) | `ValueNumbering::compute`, `ValueNumberInfo` (associated `Operator`) | Dominator-scoped GVN over any operation identity |
| Redundancy counting | `ValueNumbering::redundant_count` | From GVN results |
| Alias analysis | `AliasSets::compute`, `MemoryInfo` trait | Union-find based alias sets |
| Purity classification | `cfg_purity`, `block_purity`, `EffectInfo` (associated `Effect`) | Consumer effect vocabularies — machine memory/IO, allocation, panics |
| Metrics | `GraphMetrics::compute` (any rooted view); `CfgMetrics::compute` | Node/edge counts, cyclomatic complexity, nesting depth, instruction density |
| Pattern detection | `detect_patterns` (any view), `detect_cfg_patterns` (adds trampolines + arm orientation) | Diamond, chain, self-loop, empty trampoline |
| Profiling | `CfgProfile::from_edge_weights`, `set_uniform_edge_weights` | Edge-weight-based hot/cold block analysis |
| Tail call detection | `detect_tail_calls` (heuristic), `detect_explicit_tail_calls` (`CallInfo` markers) | |
| Switch table recovery | `detect_switch_tables` (`SwitchSource`), `recover_switch_tables` | Consumer-typed targets: addresses, syntax nodes; dispatch → structured switch |

### Transforms

| Transform | Function | Description |
|---|---|---|
| Simplify (all-in-one) | `simplify`, `simplify_mapped` | Unreachable removal + block merging + empty bypass until stable; mapped form composes identity changes |
| Remove unreachable | `remove_unreachable` | DFS reachability pruning |
| Merge blocks | `merge_blocks` | Coalesce single-succ/single-pred chains |
| Remove empty blocks | `remove_empty_blocks` | Bypass empty fallthrough blocks |
| Critical edge splitting | `split_critical_edges`, `split_critical_edges_with` | Insert blocks on multi-succ → multi-pred edges while retaining the original edge identity/payload and mapping both halves |
| Dead code elimination | `dead_code_elimination` | Liveness-based; requires `EffectInfo` so side-effecting code is never silently deleted |
| Edge contraction | `contract_edge`, `contract_edge_mapped` | Merge two blocks connected by a single edge; mapped form preserves surviving edge identities/payloads |
| Node splitting | `split_node`, `split_node_at_points` | Split at one or several validated consumer-selected instruction boundaries |
| Loop rotation | `rotate_loop` | Top-tested → bottom-tested loop form |
| Loop invariant detection | `find_loop_invariants` | Identify hoistable instructions |
| Partial redundancy elimination | `analyze_pre`, `eliminate_pre` | GVN-based PRE |
| Graph coloring | `interference_graph`, `color_graph` | Interference builder uses `DirectedGraph`; coloring accepts any graph view |
| Linearization | `linearize`, `Emitter` trait, `BlockOrder` | Re-serialize CFG to a flat stream; emitters speak `BlockId`, naming is theirs |

### AST recovery

| Feature | Description |
|---|---|
| `lift()` → `AstNode<I>` | Recover structured control flow from a CFG |
| `lift_predicated()` | Additionally regionize `Predicated` instruction runs into `Guarded` nodes (ARM IT, GPU wavefront, CMOV) |
| If/then/else | Diamond and triangle patterns |
| Loops | While, do-while, infinite; with `break` and `continue` |
| Switch/case | Multi-way branches with fallthrough |
| Try/catch/finally | From region metadata |
| Label/goto | Fallback for irreducible control flow |
| Pseudocode | `to_pseudocode` via `DisplayInstr` — rendering never requires flow classification |

## Extension contracts

The generic graph has no consumer trait requirement when it owns the storage. `DirectedGraph`, `Cfg`, and `KeyedGraph` expose both node and edge views directly. Implement `DenseNodeId` and the three required `DirectedGraphView` methods (`node_count`, `successors`, and `predecessors`) only when adapting another graph store; implement `EdgeGraphView` when algorithms must observe stable edge identities or payloads; add `RootedGraphView` (or use `Rooted`) for entry-requiring algorithms. Instruction traits are opt-in according to which CFG and dataflow features an adapter needs — every associated type is the consumer's own:

```text
DirectedGraph<N, E>       (owned arbitrary graph; no adapter trait)
DirectedGraphView         (existing consumer-owned graph storage)
├── EdgeGraphView         (stable edge identity, endpoints, data, adjacency)
└── RootedGraphView       (adds a distinguished entry node; `Rooted` adapts)

FlowControl               (required only by CfgBuilder)
└── JumpTargets           (goto/label wiring — associated Target)

InstrInfo<Variable = V>   (optional — native IR variables, defs/uses)
├── EffectInfo            (purity, DCE — associated Effect)
├── Predicated            (guarded execution — lift_predicated)
├── CopySource            (copy propagation)
├── ConstantFolder        (constant propagation, SCCP — associated Const)
├── ExprInstr             (expression trees — associated Operator, Const)
├── ValueNumberInfo       (value numbering, PRE — associated Operator)
├── MemoryInfo            (alias analysis)
└── MemoryEffect          (memory SSA)

DisplayInstr              (rendering only — DOT, pseudocode)
CallInfo                  (call graphs, explicit tail calls — associated Callee)
SwitchSource              (switch table recovery — associated Target)
```

## Workspace

| Crate | Description |
|---|---|
| **cfglib** | Generic graph, CFG, SSA, and dataflow framework |
| **cfglib-dxbc** | SM4/SM5 CFG and component-granular SSA adapter over `dxbc` |

## Adapting a language, IR, or existing graph

For symbol, reference, value-flow, type-relation, import, or grammar graphs, store domain objects directly in `DirectedGraph<N, E>`. Projects that already own adjacency lists can instead implement `DirectedGraphView` for their store, then use traversal, shortest-path, topological-sort, SCC, dominance, loop-detection, metrics, and pattern algorithms without migrating their data. Dense `u32` and `usize` handles work directly; custom handles implement `DenseNodeId`.

For a control-flow and SSA adapter:

1. Build `Cfg<I>` directly with `new_block()` / `add_edge()`, or use `Cfg<I, E>::with_edge_payload()` plus `add_edge_with_payload()` when branch labels, handler order, continuation/call-site identity, or source provenance must survive. Structured unit-payload streams can instead implement `FlowControl` and use `CfgBuilder::build()`.
2. Optionally implement `InstrInfo` with a native `Variable` identity (and its sub-traits) for dataflow analyses.
3. Implement `DisplayInstr` when you want DOT or pseudocode output.

Existing `Cfg<I>` callers remain source-compatible: `E` defaults to `()`, and the original constructors and transform entry points remain. Payload-aware frontends opt into the new methods. `RewriteMap` uses a missing entry for an unchanged identity, an empty replacement list for removal, one replacement for retention/redirect/merge, and several ordered replacements for a split. Stable parallel `EdgeId`s plus caller payloads are also the generic continuation/call-site mechanism; cfglib does not impose a VM-specific continuation type.

The variable type only needs `Clone + Ord` — never `Copy`, never numeric. It can be an architecture enum such as `Register(Rax)` / `Flag(Zero)`, a shader structure such as `(register file, index, component)`, an interned source symbol, or an existing IR value handle. Adapters decide the atomic aliasing unit; for overlapping resources such as x86 subregisters, expose canonical units or every affected unit.

```rust
use cfglib::{Cfg, DominatorTree, InstrInfo, SsaForm};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum X86Variable { Register(u8), Flag(u8), StackSlot(i32) }

struct X86Instruction {
    uses: Vec<X86Variable>,
    defs: Vec<X86Variable>,
}

impl InstrInfo for X86Instruction {
    type Variable = X86Variable;

    fn uses(&self) -> &[Self::Variable] { &self.uses }
    fn defs(&self) -> &[Self::Variable] { &self.defs }
}

let cfg = Cfg::<X86Instruction>::new();
let dominators = DominatorTree::compute(&cfg);
let ssa = SsaForm::compute(&cfg, &dominators);
```

`SsaForm<V>` is a non-mutating view over the source CFG. It stores renamed phi results, operands, instruction uses, and instruction definitions as `SsaValue<V>`, while each `SsaInstruction` keeps a `ProgramPoint` back to the native instruction. Version `0` denotes a live-in or otherwise undefined incoming value.

`cfglib-dxbc` is the concrete shader-bytecode adapter. It derives native
register-component identities from decoded masks and swizzles, retains relative
index expressions, classifies multi-result and UAV read-modify-write operations,
and reports observable shader effects through its own `Sm4Effect` vocabulary.
Its `dxbc` dependency comes directly from the `d3dasm` Git repository;
`Cargo.lock` records the exact upstream commit used by the test suite.

The `tests/source-cfg.rs` integration test is the executable specification of
the source-language side: interned symbol variables, string/bool constants
through constant propagation, enum operators in expression trees, goto wiring
by label token, and switch recovery over syntax-node targets.

## Development

Install the git hooks once with `prek install` (or `pre-commit install`); both
tools read the same `.pre-commit-config.yaml`. Hygiene checks, the repository
policy script, `cargo fmt`, and pedantic Clippy run at commit time; the full
test and documentation suites run at push time. CI runs the same gates plus an
MSRV (1.85) check and a `no_std` target build. The complete gate list and the
workspace's naming and layout conventions live in `AGENTS.md`.

## License

MIT
