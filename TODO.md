Still relevant for a CFG library:
 1. Program Dependence Graph (PDG) — Combines CDG + data dependence (def-use) into one graph. Powers program slicing, clone detection, and
advanced restructuring. We already have both halves (CDG + DefUseChains), just need to unify them.
 2. Graph serialization — Save/load CFGs via serde. No way to persist a CFG right now. Practical necessity for caching, testing with
 real-world inputs, or IPC between tools.

    8. CFG comparison / diffing — Structural comparison of two CFGs (bindiff-style).
