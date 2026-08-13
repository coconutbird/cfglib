# cfglib

Generic, `no_std` graph and dataflow framework for code intelligence, program analysis, decompilation, and compiler infrastructure.

`cfglib` provides an owned directed multigraph plus a view contract for consumer-owned graph stores. Its `Cfg<I>` layer adds control-flow semantics for instruction types that implement [`FlowControl`], while its dataflow layer is generic over each adapter's native variable identity. x86 registers and flags, shader register components, bytecode locals, compiler IR values, and source-language symbols therefore do not need to be flattened into a library-owned numbering scheme. On top of that it ships a compiler-middle-end toolkit: dominator trees, renamed SSA construction, dataflow analyses, value numbering, alias analysis, loop transforms, dead-code elimination, partial redundancy elimination, graph colouring, and structured AST recovery.

Everything is `no_std + alloc` and the core graph structure uses `SmallVec` adjacency lists with tombstone-based edge removal for cache-friendly, arena-stable IDs.

## Quick start

### 1. Implement `FlowControl`

```rust
use std::borrow::Cow;
use cfglib::{FlowControl, FlowEffect};

struct Inst { opcode: Op, /* ... */ }

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
    fn display_mnemonic(&self) -> Cow<'_, str> {
        Cow::Borrowed("inst")
    }
}
```

### 2. Build a CFG

```rust
use cfglib::CfgBuilder;

let cfg = CfgBuilder::build(instructions).unwrap();
```

### 3. Use it

```rust
use cfglib::DominatorTree;

// Traversals
let rpo = cfg.reverse_postorder();

// Dominator tree
let dom = DominatorTree::compute(&cfg);
assert!(dom.dominates(cfg.entry(), some_block));

// Export to Graphviz
println!("{}", cfg.to_dot());
```

## Feature overview

### Generic graph core

`DirectedGraph<N, E>` is an owned directed multigraph with consumer-defined node and edge payloads, stable IDs, and forward/reverse adjacency. `DirectedGraphView` also lets existing graph stores use the algorithms without first migrating their storage. This layer is suitable for symbol/reference graphs, value-flow graphs, call graphs, type relations, import graphs, and grammar dependencies; it has no instruction or binary-analysis concepts.

```rust
use cfglib::{DirectedGraph, TraversalDirection, shortest_path};

let mut graph = DirectedGraph::new();
let source = graph.add_node("definition");
let target = graph.add_node("call result");
let edge = graph.add_edge(source, target, ("return", "src/lib.rs", 42));

assert_eq!(graph[edge].payload().0, "return");
assert_eq!(
    shortest_path(&graph, source, target, TraversalDirection::Outgoing),
    Some(vec![source, target]),
);
```

### Control-flow graph (`Cfg<I>`)

| Feature | Description |
|---|---|
| Generic `Cfg<I>` | Parameterised over any instruction type via `FlowControl` |
| `no_std` + `alloc` | Runs in embedded, kernel, and WASM environments |
| `CfgBuilder` | Builds a CFG from a flat structured instruction stream (`if/else/endif`, `loop/endloop`, `switch/case/endswitch`, `break`, `continue`) |
| SmallVec adjacency | Stack-allocated successor (2) / predecessor (4) lists; heap only for high fan-out |
| Tombstone edges | `remove_edge()` replaces the slot with `None`; existing `EdgeId`s remain stable |
| Edge metadata | `EdgeKind` (13 variants: fallthrough, conditional, back, call, switch-case, exception, jump), optional weights, call-site info |
| Regions | Try/catch/finally regions with `Handler` and `HandlerKind` (Catch, CatchAll, Finally, Fault, Filter) |
| Guards | Predicated execution (ARM IT blocks, GPU wavefront control) |
| Subgraph extraction | `subgraph()` with dense O(1) block-id remapping |
| Block splitting | `split_block()` with automatic edge transfer |
| `serde` feature | Optional serialisation support |

### Graph algorithms

| Algorithm | Function / Type | Description |
|---|---|---|
| DFS / BFS | `depth_first_preorder`, `breadth_first`, CFG convenience methods | Direction-selectable traversals over `DirectedGraphView` |
| Shortest path | `shortest_path` | Forward or reverse unweighted witness path |
| Topological sort | `topological_sort` | Stable ordering or cycle detection |
| Visitor pattern | `walk_dfs`, `walk_bfs`, `CfgVisitor` trait | Callback-driven traversal |
| Dominator tree | `DominatorTree::compute_from`, `DominatorTree::compute` | Generic rooted graph or CFG convenience entry point |
| Post-dominator tree | `DominatorTree::compute_post` | On the reverse CFG |
| Dominance frontiers | `DominanceFrontiers::compute` | For SSA φ-placement |
| Incremental dominators | `update_after_edge_insert`, `update_after_edge_remove` | Recompute + diff |
| Strongly connected components | `tarjan_scc` → `SccResult<N>` | Generic iterative Tarjan algorithm, reverse-topological order |
| Back-edge detection | `find_back_edges` | Explicit `Back` edges + dominator-confirmed |
| Natural loop detection | `detect_loops` → `Vec<NaturalLoop>` | Header, body, latches, nesting depth |
| Loop nesting tree | `LoopNestingTree::build` | Parent/child loop hierarchy |
| Control dependence graph | `ControlDependenceGraph::compute` | From post-dominator tree |
| Program dependence graph | `ProgramDependenceGraph::compute` | CDG + def-use chains; backward slicing |
| Interval analysis | `interval_analysis` | T1-T2 reduction; reducibility test |
| Reducibility transform | `make_reducible` | Node splitting for irreducible CFGs |
| Reverse CFG | `reverse_cfg` | Flip all edges, swap entry/exits |
| Call graph | `CallGraph` | Inter-procedural call graph with SCC, topo-sort, recursion detection |
| CFG diff | `cfg_diff` | Structural comparison (bindiff-style fingerprinting) |
| Exception handling model | `build_eh_model` | Landing pads, cleanup blocks, protected-by mapping |
| Integrity verification | `verify` | 5 invariant checks on graph structure |
| DOT export | `to_dot`, `write_dot` | Graphviz output with edge colours and weights |

### Dataflow framework

| Analysis | Function / Type | Description |
|---|---|---|
| Generic fixpoint solver | `solve`, `Problem` trait | Forward or backward, any lattice type |
| Reaching definitions | `ReachingDefs::compute` | Which writes reach each point |
| Liveness | `Liveness::compute` | Live-in / live-out at each block |
| Def-use / use-def chains | `DefUseChains::compute` | Bidirectional def↔use links; dead-def detection |
| SSA construction | `build_ssa`, `SsaForm<V>` | IDF phi placement plus full dominator-tree renaming |
| Phi placement | `place_phis`, `PhiPlacements<V>` | Structural IDF phase for consumers that only need placement |
| SSA deconstruction | `eliminate_phis`, `copies_by_predecessor` | φ-to-copy lowering |
| Phi webs | `compute_phi_webs` | Congruence classes for register coalescing |
| Constant propagation | `constant_propagation`, `ConstantFolder` trait | Top/Const/Bottom lattice |
| Sparse conditional constant propagation | `sccp` | SSA-based, marks unreachable edges |
| Copy propagation | `copy_propagation`, `CopySource` trait | Chain resolution + dead copy removal |
| Memory SSA | `build_memory_ssa`, `MemoryEffect` trait | Memory versioning with φ-nodes |
| Abstract interpretation | `abstract_interpret`, `AbstractDomain` trait | Generic abstract domain framework |

### Higher-level analyses

| Analysis | Function / Type | Description |
|---|---|---|
| Expression tree recovery | `recover_expressions`, `ExprInstr` trait | Rebuild expression DAGs from flat instructions |
| Value numbering (local) | `local_value_numbering` | Per-block hash-consing |
| Value numbering (global) | `global_value_numbering`, `ValueNumberInfo` trait | Dominator-scoped GVN |
| Redundancy counting | `count_redundant` | From GVN results |
| Alias analysis | `alias_analysis`, `MemoryInfo` trait | Union-find based alias sets |
| Purity classification | `cfg_purity`, `block_purity` | Pure / read-only / impure |
| CFG metrics | `cfg_metrics` → `CfgMetrics` | Block/edge counts, cyclomatic complexity, fan-in/out, nesting depth |
| Pattern detection | `detect_patterns` → `Vec<CfgPattern>` | Diamond, triangle, self-loop, critical edge, hammock |
| Profiling | `CfgProfile`, `set_uniform_weights` | Edge-weight-based hot/cold block analysis |
| Tail call detection | `detect_tail_calls` | Explicit and structural tail-call identification |
| Switch table recovery | `recover_switch_tables`, `SwitchCandidate` trait | Indirect jump → structured switch reconstruction |

### Transforms

| Transform | Function | Description |
|---|---|---|
| Simplify (all-in-one) | `simplify` | Unreachable removal + block merging + empty bypass until stable |
| Remove unreachable | `remove_unreachable` | DFS reachability pruning |
| Merge blocks | `merge_blocks` | Coalesce single-succ/single-pred chains |
| Remove empty blocks | `remove_empty_blocks` | Bypass empty fallthrough blocks |
| Critical edge splitting | `split_critical_edges` | Insert blocks on multi-succ → multi-pred edges |
| Dead code elimination | `dead_code_elimination` | Liveness-based unused-def removal |
| Edge contraction | `contract_edge` | Merge two blocks connected by a single edge |
| Node splitting | `split_node` | Split a block at an instruction index |
| Loop rotation | `rotate_loop` | Top-tested → bottom-tested loop form |
| Loop invariant detection | `find_loop_invariants` | Identify hoistable instructions |
| Partial redundancy elimination | `analyse_pre`, `eliminate_pre` | GVN-based PRE |
| Graph colouring | `InterferenceGraph::build`, `color_graph` | Greedy register allocation with degree heuristic |
| Linearisation | `linearize`, `Emitter` trait, `BlockOrder` | Re-serialise CFG to a flat instruction stream |

### AST recovery

| Feature | Description |
|---|---|
| `lift()` → `AstNode<I>` | Recover structured control flow from a CFG |
| If/then/else | Diamond and triangle patterns |
| Loops | While, do-while, infinite; with `break` and `continue` |
| Switch/case | Multi-way branches with fallthrough |
| Try/catch/finally | From region metadata |
| Label/goto | Fallback for irreducible control flow |
| Guarded blocks | Predicated execution (ARM IT, GPU wavefront) |

## Extension contracts

The generic graph has no consumer trait requirement when it owns the storage. Implement `DenseNodeId` and `DirectedGraphView` only when adapting an existing graph store. Instruction traits are opt-in according to which CFG and dataflow features an adapter needs:

```text
DirectedGraph<N, E>       (owned arbitrary graph; no adapter trait)
DirectedGraphView         (existing consumer-owned graph storage)

FlowControl               (required only by CfgBuilder)
├── SwitchCandidate       (switch table recovery)
│
InstrInfo<Variable = V>   (optional — native IR variables, defs/uses/effects)
├── CopySource            (copy propagation)
├── ConstantFolder        (constant propagation, SCCP)
├── ExprInstr             (expression tree recovery)
├── ValueNumberInfo       (local/global value numbering, PRE)
├── MemoryInfo            (alias analysis)
└── MemoryEffect          (memory SSA)
```

## Workspace

| Crate | Description |
|---|---|
| **cfglib** | Generic graph, CFG, SSA, and dataflow framework |
| **cfglib-dxbc** | SM4/SM5 CFG and component-granular SSA adapter over `dxbc` |

## Adapting a language, IR, or existing graph

For symbol, reference, value-flow, type-relation, import, or grammar graphs, store domain objects directly in `DirectedGraph<N, E>`. Projects that already own adjacency lists can instead implement `DirectedGraphView` for their store, then use traversal, shortest-path, topological-sort, SCC, and dominance algorithms without migrating their data. Dense `u32` and `usize` handles work directly; custom handles implement `DenseNodeId`.

For a control-flow and SSA adapter:

1. Implement `FlowControl` for the instruction or IR operation type.
2. Call `CfgBuilder::build()` with its instruction stream, or populate a `Cfg` directly.
3. Optionally implement `InstrInfo` with a native `Variable` identity (and its sub-traits) for dataflow analyses.

The variable type only needs `Clone + Ord`. It can be an architecture enum such as `Register(Rax)` / `Flag(Zero)`, a shader structure such as `(register file, index, component)`, or an existing IR value handle. Adapters decide the atomic aliasing unit; for overlapping resources such as x86 subregisters, expose canonical units or every affected unit.

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
and reports observable shader effects. Its `dxbc` dependency comes directly
from the `d3dasm` Git repository; `Cargo.lock` records the exact upstream commit
used by the test suite.

## License

MIT
