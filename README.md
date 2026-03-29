# cfglib

Generic, `no_std` control-flow graph library for binary analysis.

`cfglib` provides an ISA-agnostic `Cfg<I>` data structure and a scope-stack
builder that converts any flat instruction stream into a structured CFG.
The only requirement is that the instruction type implements the `FlowControl`
trait.

## Features

- **Generic** — parameterised over any instruction type via `FlowControl`
- **`no_std` + `alloc`** — runs in embedded/kernel environments
- **Structured flow** — `if/else`, `loop`, `switch/case` with correct `break` semantics
- **Traversals** — DFS (pre/post-order), BFS, reverse post-order
- **Dominator tree** — Cooper-Harvey-Kennedy algorithm
- **DOT export** — Graphviz output with edge coloring (green=true, red=false, blue dashed=back-edge)

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
