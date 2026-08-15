# cfglib

Generic, `no_std` graph and dataflow framework for code intelligence, program analysis, decompilation, and compiler infrastructure.

`cfglib` has two graph storage models: an owned directed multigraph for arbitrary code-intelligence relations, and `Cfg<I>` for graphs that genuinely need basic blocks, typed control-flow edges, and exception regions. A small read-only view contract lets consumer-owned stores reuse the algorithms. Every instruction-adjacent axis is consumer-typed rather than imposed by the library: dataflow variables, constants, expression operators, side-effect vocabularies, branch targets, and call targets all come from the adapter — so x86 registers and flags, shader register components, bytecode locals, compiler IR values, and source-language symbols do not need to be flattened into a library-owned numbering scheme, and string literals or symbol ids flow through the analyses as naturally as machine words and addresses. On top of that it ships a compiler-middle-end toolkit: dominator trees, renamed SSA construction, dataflow analyses, value numbering, alias analysis, loop transforms, dead-code elimination, partial redundancy elimination, graph colouring, and structured AST recovery.

Everything is `no_std + alloc` and the core graph structure uses `SmallVec` adjacency lists with tombstone-based edge removal for cache-friendly, arena-stable IDs.

## Quick start

### Direct construction (the primary front door)

No trait is required to build, verify, analyse, or render a CFG. Source
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

`DirectedGraph<N, E>` is the single owned storage type for consumer-defined node and edge payloads, stable IDs, and forward/reverse adjacency. `DirectedGraphView` lets existing graph stores use the algorithms without first migrating their storage; dense node IDs give it a default node iterator, so adapters only expose a node count plus successor and predecessor iteration. `RootedGraphView` adds a distinguished entry node for algorithms that need one (dominance, reachability metrics, loop and interval analysis), and the `Rooted` adapter roots any plain view at a chosen node. This layer is suitable for symbol/reference graphs, value-flow graphs, call graphs, type relations, import graphs, grammar dependencies, and analysis-derived relations; it has no instruction or binary-analysis concepts.

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
| Generic `Cfg<I>` | Parameterised over any instruction type; no trait needed for direct construction |
| `no_std` + `alloc` | Runs in embedded, kernel, and WASM environments |
| `CfgBuilder` | Builds a CFG from a flat structured instruction stream (`if/else/endif`, `loop/endloop`, `switch/case/endswitch`, `break`, `continue`) |
| Goto wiring | `resolve_jump_edges` + `JumpTargets` (consumer-typed targets: labels, addresses, syntax nodes) |
| SmallVec adjacency | Stack-allocated successor (2) / predecessor (4) lists; heap only for high fan-out |
| Tombstone edges | `remove_edge()` replaces the slot with `None`; existing `EdgeId`s remain stable |
| Edge metadata | `EdgeKind` (14 variants: fallthrough, conditional, back, call, switch-case, exception, jump) plus optional weights |
| Regions | Try/catch/finally regions with `Handler` and `HandlerKind` (Catch, CatchAll, Finally, Fault, Filter) |
| Cleanup continuations | `add_continuation` / `set_cleanup_resume` → `Cleanup` (`Continuation` + `CompletionReason`): which route out of a single shared `finally` block a resume edge belongs to |
| Handler filters | `HandlerFilters<F>` side table — consumer-typed filter predicates (C# `catch … when`) keyed by `HandlerRef`, with no type parameter on `Cfg` |
| Subgraph extraction | `subgraph()` with dense O(1) block-id remapping |
| Block splitting | `split_block()` with automatic edge transfer |
| `serde` feature | Optional serialisation support |

### Graph algorithms

| Algorithm | Function / Type | Description |
|---|---|---|
| DFS / BFS | `depth_first_preorder`, `breadth_first`, CFG convenience methods | Direction-selectable traversals over `DirectedGraphView` |
| Edge-aware traversal | `breadth_first_edges`, `walk_edges` (filtered + depth-bounded) | Every distinct edge once with identity + endpoints; parallel-edge provenance |
| Configurable search | `search` + `SearchConfig` (order, visited policy, direction, depth bound) | First-match, pruning (`Visit::Skip`), early exit (`ControlFlow::Break`), and backtracking as configuration; `VisitedPolicy::Path` un-marks on unwind so every route to a node is reported |
| Depth-first events | `depth_first_events` → `DfsEvent` | Discover / tree / back / forward-or-cross / finish in a pinned order — tri-color edge classification, cycle diagnostics in traversal order |
| Open-graph search | `open_search` + `OpenSearchConfig` | The same disciplines over a lazily discovered node space: successors come from a closure, no dense ids (import/re-export chases, ordered emission walks) |
| Alias chase | `follow`, `follow_path` | Out-degree ≤ 1 chase with a hop bound and a full-path cycle guard; the chain, or just its end |
| Shortest path | `shortest_path` (nodes), `shortest_path_edges` (edge witness) | Forward or reverse unweighted witness path |
| Multi-source reachability | `reachable` | Dense `Vec<bool>` from a seed set, forward or reverse; order-insensitive |
| Nearest common ancestor | `nearest_common_ancestor` | Bidirectional BFS meet of two nodes; smallest combined distance, ties by smallest node id |
| All common ancestors | `common_ancestors` → `Vec<CommonAncestor>` | Every shared node with both hop counts, depth-bounded, in `b`'s BFS discovery order — the consumer-rankable form (MRO linearization, overload preference) |
| Horn-clause derivability | `HornClauses` | AND-OR closure (`head <- b1 & b2`): nullability, all-arguments-constant, all-callers-dead |
| Topological sort | `topological_sort` | Stable ordering or cycle detection |
| Dominator tree | `DominatorTree::compute` (rooted views), `compute_from` (explicit root) | Cooper-Harvey-Kennedy over any graph view |
| Post-dominator tree | `DominatorTree::compute_post` (CFG), `compute_post_from` (any view + explicit exits) | Virtual-exit handling built in |
| Dominance frontiers | `DominanceFrontiers::compute` | For SSA φ-placement |
| Incremental dominators | `update_after_edge_insert`, `update_after_edge_remove` | Recompute + diff |
| Strongly connected components | `tarjan_scc` → `SccResult<N>`, `condensation` → component DAG | Generic iterative Tarjan algorithm, reverse-topological order (leaves first) |
| SCC in topological order | `kosaraju_scc` → `SccResult<N>` | The same partition numbered sources first (`index(u) < index(v)` across every edge); the classic deterministic two-pass algorithm, for budgeted forward closures over the condensation |
| Back-edge detection | `find_back_edges` (dominance, any view), `find_back_edges_tagged` (CFG, honours `Back` tags) | |
| Natural loop detection | `detect_loops` / `detect_loops_tagged` → `Vec<NaturalLoop<N>>` | Header, body, latches, nesting depth |
| Loop nesting tree | `LoopNestingTree::build` | Parent/child loop hierarchy |
| Control dependence graph | `control_dependence_graph` → `DirectedGraph<N, ()>` | From post-dominator tree, over any view |
| Program dependence graph | `program_dependence_graph` → `DirectedGraph<DependenceNode, DependenceKind>` | Control + def-use edges; reverse traversal performs backward slicing |
| Interval analysis | `interval_analysis` | T1-T2 reduction over rooted views; reducibility test |
| Reducibility transform | `make_reducible` | Node splitting for irreducible CFGs |
| Reverse CFG | `reverse_cfg` | Flip all edges, swap entry/exits |
| Call graph | `build_call_graph` + `CallInfo`, `propagate_summaries` (callee-first SCC fixpoint) | Consumer-typed callees; interprocedural summary scaffold |
| CFG diff | `cfg_diff` | Structural comparison (bindiff-style fingerprinting), no trait bounds |
| Exception handling model | `build_eh_model` | Landing pads, cleanup blocks, protected-by mapping, cleanup continuations by completion reason |
| Integrity verification | `verify` (CFG storage), `verify_view` (consumer view contract) | |
| DOT export | `to_dot` (`DisplayInstr`), `to_dot_with` (bound-free), `write_view_dot` (any view) | Graphviz output with escaped labels |

### Dataflow framework

| Analysis | Function / Type | Description |
|---|---|---|
| Generic fixpoint solver | `solve`, `Problem` trait | Forward or backward, any lattice type |
| Node-level fixpoint | `solve_node_problem`, `NodeProblem` trait | Per-node facts over any graph view (taint, reachability-with-facts) |
| Seeded node fixpoint | `solve_node_problem_from` | Same solver, worklist seeded from a subset — incremental / dirty-region re-solves |
| Reaching definitions | `ReachingDefs::compute` | Which writes reach each point |
| Liveness | `Liveness::compute` | Live-in / live-out at each block |
| Def-use / use-def chains | `DefUseChains::compute` | Bidirectional def↔use links; dead-def detection |
| SSA construction | `build_ssa`, `SsaForm<V>` | IDF phi placement plus full dominator-tree renaming |
| Phi placement | `place_phis`, `PhiPlacements<V>` | Structural IDF phase for consumers that only need placement |
| SSA deconstruction | `eliminate_phis`, `copies_by_predecessor` | φ-to-copy lowering |
| Phi webs | `compute_phi_webs` | Congruence classes for register coalescing |
| Constant propagation | `constant_propagation`, `ConstantFolder` (associated `Const`) | Top/Const/Bottom lattice over a consumer constant domain — machine words, strings, bools, float bits |
| Sparse conditional constant propagation | `sccp` → `SccpResult<V, C>` | SSA-based, marks unreachable edges |
| Copy propagation | `copy_propagation`, `CopySource` trait | Chain resolution + dead copy removal |
| Memory SSA | `build_memory_ssa`, `MemoryEffect` trait | Memory versioning with φ-nodes |
| Abstract interpretation | `abstract_interpret`, `AbstractDomain` trait | Generic abstract domain framework |

### Higher-level analyses

| Analysis | Function / Type | Description |
|---|---|---|
| Expression tree recovery | `recover_expressions`, `ExprInstr` (associated `Operator` + `Const`) | Rebuild expression DAGs from flat instructions |
| Value numbering (local) | `local_value_numbering` | Per-block hash-consing |
| Value numbering (global) | `global_value_numbering`, `ValueNumberInfo` (associated `Operation`) | Dominator-scoped GVN over any operation identity |
| Redundancy counting | `count_redundant` | From GVN results |
| Alias analysis | `alias_analysis`, `MemoryInfo` trait | Union-find based alias sets |
| Purity classification | `cfg_purity`, `block_purity`, `EffectInfo` (associated `Effect`) | Consumer effect vocabularies — machine memory/IO, allocation, panics |
| Metrics | `graph_metrics` (any rooted view) → `GraphMetrics`; `cfg_metrics` → `CfgMetrics` | Node/edge counts, cyclomatic complexity, nesting depth, instruction density |
| Pattern detection | `detect_patterns` (any view), `detect_cfg_patterns` (adds trampolines + arm orientation) | Diamond, chain, self-loop, empty trampoline |
| Profiling | `CfgProfile`, `set_uniform_weights` | Edge-weight-based hot/cold block analysis |
| Tail call detection | `detect_tail_calls` (heuristic), `detect_explicit_tail_calls` (`CallInfo` markers) | |
| Switch table recovery | `detect_switch_tables` (`SwitchSource`), `recover_switch_tables` | Consumer-typed targets: addresses, syntax nodes; dispatch → structured switch |

### Transforms

| Transform | Function | Description |
|---|---|---|
| Simplify (all-in-one) | `simplify` | Unreachable removal + block merging + empty bypass until stable |
| Remove unreachable | `remove_unreachable` | DFS reachability pruning |
| Merge blocks | `merge_blocks` | Coalesce single-succ/single-pred chains |
| Remove empty blocks | `remove_empty_blocks` | Bypass empty fallthrough blocks |
| Critical edge splitting | `split_critical_edges` | Insert blocks on multi-succ → multi-pred edges |
| Dead code elimination | `dead_code_elimination` | Liveness-based; requires `EffectInfo` so side-effecting code is never silently deleted |
| Edge contraction | `contract_edge` | Merge two blocks connected by a single edge |
| Node splitting | `split_node` | Split a block at an instruction index |
| Loop rotation | `rotate_loop` | Top-tested → bottom-tested loop form |
| Loop invariant detection | `find_loop_invariants` | Identify hoistable instructions |
| Partial redundancy elimination | `analyse_pre`, `eliminate_pre` | GVN-based PRE |
| Graph colouring | `build_interference_graph`, `color_graph` | Interference builder uses `DirectedGraph`; coloring accepts any graph view |
| Linearisation | `linearize`, `Emitter` trait, `BlockOrder` | Re-serialise CFG to a flat stream; emitters speak `BlockId`, naming is theirs |

### AST recovery

| Feature | Description |
|---|---|
| `lift()` → `AstNode<I>` | Recover structured control flow from a CFG |
| `lift_predicated()` | Additionally regionise `Predicated` instruction runs into `Guarded` nodes (ARM IT, GPU wavefront, CMOV) |
| If/then/else | Diamond and triangle patterns |
| Loops | While, do-while, infinite; with `break` and `continue` |
| Switch/case | Multi-way branches with fallthrough |
| Try/catch/finally | From region metadata |
| Label/goto | Fallback for irreducible control flow |
| Pseudocode | `to_pseudocode` via `DisplayInstr` — rendering never requires flow classification |

## Extension contracts

The generic graph has no consumer trait requirement when it owns the storage. Implement `DenseNodeId` and the three required `DirectedGraphView` methods (`node_count`, `successors`, and `predecessors`) only when adapting an existing graph store; add `RootedGraphView` (or use `Rooted`) for entry-requiring algorithms. Instruction traits are opt-in according to which CFG and dataflow features an adapter needs — every associated type is the consumer's own:

```text
DirectedGraph<N, E>       (owned arbitrary graph; no adapter trait)
DirectedGraphView         (existing consumer-owned graph storage)
└── RootedGraphView       (adds a distinguished entry node; `Rooted` adapts)

FlowControl               (required only by CfgBuilder)
└── JumpTargets           (goto/label wiring — associated Target)

InstrInfo<Variable = V>   (optional — native IR variables, defs/uses)
├── EffectInfo            (purity, DCE — associated Effect)
├── Predicated            (guarded execution — lift_predicated)
├── CopySource            (copy propagation)
├── ConstantFolder        (constant propagation, SCCP — associated Const)
├── ExprInstr             (expression trees — associated Operator, Const)
├── ValueNumberInfo       (value numbering, PRE — associated Operation)
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

1. Build the `Cfg` directly with `new_block()` / `add_edge()` (source frontends, decoded binaries), or implement `FlowControl` and use `CfgBuilder::build()` (structured streams).
2. Optionally implement `InstrInfo` with a native `Variable` identity (and its sub-traits) for dataflow analyses.
3. Implement `DisplayInstr` when you want DOT or pseudocode output.

The variable type only needs `Clone + Ord` — never `Copy`, never numeric. It can be an architecture enum such as `Register(Rax)` / `Flag(Zero)`, a shader structure such as `(register file, index, component)`, an interned source symbol, or an existing IR value handle. Adapters decide the atomic aliasing unit; for overlapping resources such as x86 subregisters, expose canonical units or every affected unit.

```rust
use cfglib::{Cfg, DominatorTree, InstrInfo, build_ssa};

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
let ssa = build_ssa(&cfg, &dominators);
```

`SsaForm<V>` is a non-mutating view over the source CFG. It stores renamed phi results, operands, instruction uses, and instruction definitions as `SsaValue<V>`, while each `SsaInstruction` keeps a `ProgramPoint` back to the native instruction. Version `0` denotes a live-in or otherwise undefined incoming value.

`cfglib-dxbc` is the concrete shader-bytecode adapter. It derives native
register-component identities from decoded masks and swizzles, retains relative
index expressions, classifies multi-result and UAV read-modify-write operations,
and reports observable shader effects through its own `Sm4Effect` vocabulary.
Its `dxbc` dependency comes directly from the `d3dasm` Git repository;
`Cargo.lock` records the exact upstream commit used by the test suite.

The `tests/source_cfg.rs` integration test is the executable specification of
the source-language side: interned symbol variables, string/bool constants
through constant propagation, enum operators in expression trees, goto wiring
by label token, and switch recovery over syntax-node targets.

## License

MIT
