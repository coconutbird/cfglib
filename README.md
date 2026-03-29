# cfglib

Generic, `no_std` control-flow graph library for binary analysis, decompilation, and compiler infrastructure.

`cfglib` provides an ISA-agnostic `Cfg<I>` parameterised over any instruction type `I` that implements a single trait — [`FlowControl`]. On top of that it ships a complete compiler-middle-end toolkit: dominator trees, SSA construction, dataflow analyses, value numbering, alias analysis, loop transforms, dead-code elimination, partial redundancy elimination, graph colouring, and structured AST recovery.

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

### Core graph (`Cfg<I>`)

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
| DFS / BFS | `dfs_preorder`, `dfs_postorder`, `reverse_postorder`, `bfs` | Standard traversals |
| Visitor pattern | `walk_dfs`, `walk_bfs`, `CfgVisitor` trait | Callback-driven traversal |
| Dominator tree | `DominatorTree::compute` | Cooper-Harvey-Kennedy iterative algorithm |
| Post-dominator tree | `DominatorTree::compute_post` | On the reverse CFG |
| Dominance frontiers | `DominanceFrontiers::compute` | For SSA φ-placement |
| Incremental dominators | `update_after_edge_insert`, `update_after_edge_remove` | Recompute + diff |
| Strongly connected components | `tarjan_scc` → `SccResult` | Tarjan's algorithm, reverse-topological order |
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
| SSA construction | `insert_phis` | IDF-based φ-function placement |
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

## Trait hierarchy

The only **required** trait is `FlowControl`. Everything else is opt-in depending on which analyses you need:

```text
FlowControl              (required — control-flow classification)
├── SwitchCandidate       (switch table recovery)
│
InstrInfo                 (optional — dataflow: defs/uses/effects)
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
| **cfglib** | Core generic CFG library |
| **cfglib-dxbc** | SM4/SM5 (DirectX) shader bytecode adapter |

## Writing an ISA adapter

1. Create a new crate (e.g. `cfglib-spirv`).
2. Implement `FlowControl` for your instruction type.
3. Call `CfgBuilder::build()` with your instruction stream.
4. Optionally implement `InstrInfo` (and its sub-traits) for dataflow analyses.

See `cfglib-dxbc` for a complete 138-line example.

## License

MIT
