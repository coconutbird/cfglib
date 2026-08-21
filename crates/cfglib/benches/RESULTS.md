# Local CFG performance results

These results compare repository `HEAD` (`5331ad5`) with the uncommitted local
optimization tree on 2026-08-21. No result or source change was pushed.

## Method

- CPU: AMD Ryzen AI MAX+ 395, logical CPU 2, Rust 1.96.0 / LLVM 22.1.2,
  release profile, median of seven samples.
- CPU mode links directly to `std::alloc::System`; no allocation observer is
  present in the binary. Every complete result is passed through `black_box`
  and dropped inside the timed region.
- Memory mode is a separate build. It performs one warm-up and measures one
  complete operation using a counting wrapper over `System`.
- Fixtures are synthetic and deterministic. Most graph cases use 4,096 nodes;
  the benchmark source documents each smaller specialized fixture.
- The same benchmark source and manifest entry were applied to the baseline
  worktree. Allocation-sensitive comparisons marked with `*` were run as
  isolated filters at 300 ms per sample; the others are from the pinned full
  matrix at 150 ms per sample.

See [README.md](README.md) for exact commands and metric definitions.

## CPU results

| Workload | HEAD | Local | Change |
|---|---:|---:|---:|
| Dominators, branchy CFG | 98.75 us | 48.43 us | 2.04x |
| Dominance frontiers | 82.35 us | 54.63 us | 1.51x |
| Post-dominators | 137.60 us | 97.57 us | 1.41x |
| Post-dominators, many exits* | 188.79 us | 162.11 us | 1.16x |
| Control-dependence graph* | 2.335 ms | 87.27 us | 26.75x |
| Dominator depths, reverse-ID chain | 2.059 ms | 3.72 us | 553.95x |
| Natural loops, many latches | 17.510 ms | 568.45 us | 30.80x |
| Interval analysis, branchy CFG | 32.503 ms | 128.23 us | 253.47x |
| Interval analysis, reverse-ID chain | 315.389 ms | 109.20 us | 2,888x |
| Node fixpoint, Boolean facts* | 62.78 us | 17.22 us | 3.65x |
| Node fixpoint, 256-word facts* | 559.36 us | 536.12 us | 1.04x |
| CFG fixpoint, Boolean facts* | 96.46 us | 36.58 us | 2.64x |
| CFG fixpoint, 256-word facts* | 578.34 us | 546.44 us | 1.06x |
| SCCP, 1,024 independent constants | 11.265 ms | 53.59 us | 210.23x |
| SSA construction, linear CFG | 1.759 ms | 158.53 us | 11.10x |
| Phi placement, phi storm | 484.18 us | 185.28 us | 2.61x |
| Full SSA, phi storm | 1.226 ms | 912.01 us | 1.34x |
| Global value numbering | 2.191 ms | 432.70 us | 5.06x |
| Constant propagation | 4.937 ms | 122.61 us | 40.27x |
| Merge linear block chain | 6.529 ms | 88.81 us | 73.51x |
| Remove empty block chain | 6.957 ms | 76.17 us | 91.33x |
| Redirect 4,096 incoming edges | 5.974 ms | 49.93 us | 119.67x |
| Merge weighted high-fanout block | 18.218 ms | 10.35 us | 1,761x |
| Contract weighted high-fanout edge | 18.207 ms | 4.58 us | 3,973x |
| Make small irreducible CFG reducible | 146.01 us | 23.64 us | 6.18x |
| Make large irreducible CFG reducible | 21.950 ms | 9.975 ms | 2.20x |

Traversal improvements were smaller but consistent: edge BFS improved 12.6%,
edge shortest path 11.3%, nearest common ancestor 15.5%, and all common
ancestors 19.5%.

## Allocation pressure

`Bytes` is total requested bytes, not RSS. `Peak` is incremental live requested
bytes over the operation's starting point.

| Workload | Allocations (HEAD -> local) | Bytes (HEAD -> local) | Peak (HEAD -> local) |
|---|---:|---:|---:|
| Dominators | 8,210 -> 20 | 417,752 -> 286,712 | 147,456 -> 147,456 |
| Post-dominators | 8,231 -> 23 | 914,478 -> 503,858 | 557,172 -> 196,672 |
| Control-dependence graph | 790 -> 265 | 435,064 -> 421,904 | 422,872 -> 417,792 |
| Natural loops | 88,234 -> 433 | 6,153,396 -> 65,556 | 53,884 -> 41,036 |
| Interval analysis | 539,489 -> 656 | 30,655,100 -> 61,452 | 1,037,280 -> 56,888 |
| Reverse-ID intervals | 8,397,854 -> 685 | 470,712,140 -> 51,676 | 1,031,916 -> 51,580 |
| CFG fixpoint, wide facts | 4,877 -> 4,241 | 9,856,032 -> 8,740,856 | 4,267,024 -> 4,267,024 |
| Constant propagation | 116,719 -> 691 | 75,479,433 -> 138,258 | 117,004 -> 111,221 |
| Merge linear chain | 14,347 -> 2,066 | 21,479,284 -> 329,700 | 382,928 -> 315,360 |
| Remove empty chain | 10,240 -> 10 | 21,305,292 -> 305,148 | 305,148 -> 305,148 |
| Redirect high fan-in | 16 -> 6 | 655,664 -> 622,928 | 639,312 -> 622,928 |
| Merge weighted fanout | 56 -> 26 | 590,294 -> 213,407 | 344,499 -> 197,003 |
| Contract weighted fanout | 25 -> 11 | 475,600 -> 164,240 | 328,104 -> 164,236 |
| Make large CFG reducible | 1,337,345 -> 16,911 | 103,608,396 -> 61,322,615 | 393,504 -> 393,504 |

## Retained implementation changes

- Reused direct adjacency iterators and dense parent/mark tables in dominance,
  traversal, shortest-path, ancestor, and control-dependence analyses.
- Memoized dominator depths in two passes; CDG uses `u32` depths only when the
  node count proves the sentinel cannot alias a real depth, otherwise falling
  back to `usize`.
- Replaced repeated loop-body reachability searches with one multi-source
  reverse traversal, and rewrote interval discovery around predecessor
  counters and a worklist.
- Used RPO/FIFO dense worklists for fixpoint problems, batched SCCP rescans,
  maintained constant facts incrementally, and replaced repeated dominator
  child scans with linked dense scratch in SSA and GVN.
- Used epoch-marked dense scratch for phi placement and CDG deduplication while
  preserving historical public edge ordering for arbitrary consumer ID `Ord`.
- Made block cleanup single-pass and added private bulk incoming/outgoing edge
  moves that preserve edge IDs, ordering, kinds, and weights.
- Combined irreducibility detection with target discovery and replaced repeated
  predecessor filtering/reachability searches with one partition and bitmap.
- Kept post-dominance as a reversed view with a private `usize` virtual exit,
  so bounded consumer ID types are never asked to represent a synthetic node.

## Rejected or deferred avenues

- `SmallVec` builder arm storage reduced allocations, but an isolated
  direct-`System` run regressed the common if/else fixture from 293.24 us to
  341.68 us (+16.5%); it was reverted. An inline break-exit variant was also
  slower.
- Alternative Tarjan component materialization/assignment strategies and
  exact-priority heap/bitmap worklists lost on CPU or memory in their focused
  A/B runs; they were reverted.
- A dense post-dominator exit bitmap bought too little over sorted binary
  membership for its extra live storage; sparse/unsorted lists retain linear
  lookup and dense sorted lists use binary lookup.
- A cached live-edge counter would add representation and serialization
  invariants for only one production caller, so it was deferred.
- Whole-graph arenas/CSR would conflict with stable edge IDs, slice adjacency,
  mutable instruction `Vec` access, or serde shape. Transformation-local
  scratch was densified instead.
- No hot loop exposed a portable dense arithmetic kernel where explicit SSE
  beat LLVM's code generation; measured hotspots were allocation, graph
  traversal, repeated scans, and worklist order.
- Advanced controlled node splitting can reduce pathological irreducible-graph
  work but adds code growth and minimum-split complexity. The retained simple
  splitter is now 2.2x faster on the large adversarial fixture, and empirical
  work reports irreducibility as uncommon, so the larger algorithm was not
  justified in this cycle.

Research consulted: [Tarjan on reducibility](https://doi.org/10.1145/800125.804040),
[Janssen and Corporaal on controlled node splitting](https://www.cs.tufts.edu/comp/150FP/archive/johan-jansson/node-splitting.pdf),
[Cytron et al. on SSA and control dependence](https://rsim.cs.uiuc.edu/arch/qual_papers/compilers/toplas91.pdf),
[IBM on loops, dominators, and frontiers](https://research.ibm.com/publications/on-loops-dominators-and-dominance-frontiers),
and [Stanier on irreducibility in practice](https://doi.org/10.1002/spe.1059).

## Limitations

- These are synthetic fixtures on one machine, not application traces.
- CPU frequency boost was enabled. Pinning and medians reduce noise, but small
  differences should be confirmed on target hardware with an empirical corpus.
- Linux `perf` counters were unavailable (`perf_event_paranoid=4`); Callgrind
  was used during hotspot discovery, while final comparisons use wall-clock
  time and allocator-request metrics.
- Rust 1.85 was not installed. The library passes no-default-feature and
  all-feature checks on Rust 1.96, but this cycle did not execute the declared
  MSRV toolchain locally.
