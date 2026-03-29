# cfglib

Generic, `no_std` control-flow graph library for binary analysis and
compiler infrastructure.

`cfglib` provides an ISA-agnostic `Cfg<I>` data structure and a full suite
of graph algorithms, dataflow analyses, optimisation passes, and
structural recovery tools. The only requirement is that the instruction
type `I` implements the [`FlowControl`] trait.

## Features

### Core
- **Generic** — parameterised over any instruction type via `FlowControl`
- **`no_std` + `alloc`** — runs in embedded/kernel environments
- **Structured flow** — `if/else`, `loop`, `switch/case` with correct `break` semantics
- **Edge metadata** — weighted edges, call-site info, guarded/predicated blocks
- **Regions** — exception / try-catch region model with handlers

### Graph algorithms
- **Traversals** — DFS (pre/post-order), BFS, reverse post-order
- **Dominator / post-dominator trees** — Cooper-Harvey-Kennedy algorithm
- **Natural loop detection** — back-edge identification, loop nesting
- **Tarjan's SCC** — strongly connected components + condensation DAG
- **Control dependence graph (CDG)** — from post-dominator tree
- **Interval analysis** — T1-T2 reduction for loop hierarchy recovery
- **Reducibility** — detection + node-splitting to make irreducible graphs reducible
- **DOT export** — Graphviz output with edge colouring and weight display

### Dataflow & SSA
- **Generic fixpoint solver** — forward/backward, any lattice
- **Reaching definitions** — which writes can reach a given point
- **Liveness** — which variables are live at each point
- **Def-use / use-def chains** — linking writers to readers
- **Constant propagation** — lattice-based forward analysis
- **Copy propagation** — chain resolution + dead copy removal
- **SSA construction** — dominance frontiers + φ-function insertion

### Analyses
- **Purity classification** — pure vs. impure blocks/CFGs
- **Switch table recovery** — reconstruct switch/case from indirect jumps
- **Expression tree recovery** — rebuild expression DAGs from flat instructions

### Transforms
- **Cleanup** — unreachable removal, block merging, empty-block bypass, simplify
- **Critical edge splitting** — required for clean SSA placement
- **Dead code elimination** — liveness-based unused-def removal
- **Loop canonicalisation** — preheader insertion, single-latch normalisation
- **Linearisation** — re-serialise a CFG to a flat instruction stream

### AST lifting
- **Structural recovery** — if/else, loops, switch, break/continue, label/goto
- **Try/catch** — from region metadata
- **Guarded blocks** — predicated execution (ARM IT, GPU wave)

## Quick start

Implement one trait for your instruction type:

```rust
use cfglib::{FlowControl, FlowEffect};

impl FlowControl for MyInstruction {
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
```

Then build and use:

```rust
use cfglib::CfgBuilder;

let cfg = CfgBuilder::build(instructions);

// Traverse
let order = cfg.dfs_preorder();

// Dominators
let dom = cfglib::DominatorTree::compute(&cfg);
assert!(dom.dominates(cfg.entry(), some_block));

// Export to Graphviz
println!("{}", cfg.to_dot());
```

## Workspace

| Crate | Description |
|---|---|
| `cfglib` | Core generic CFG library |
| `cfglib-dxbc` | SM4/SM5 shader bytecode adapter (via [d3dasm](https://github.com/coconutbird/d3dasm)) |

## Adding a new ISA adapter

1. Create a new crate (e.g. `cfglib-spirv`)
2. Implement `FlowControl` for your instruction type (newtype wrapper if needed for orphan rule)
3. Call `CfgBuilder::build()` with your instruction stream

See `cfglib-dxbc` for a complete example.

## License

MIT
