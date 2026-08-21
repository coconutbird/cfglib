#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::too_many_lines
)]
// `cfglib_bench_alloc` is an intentionally benchmark-local cfg selected with
// `RUSTFLAGS="--cfg cfglib_bench_alloc"`; it is not a crate feature.
#![allow(unexpected_cfgs)]

use std::alloc::System;
#[cfg(cfglib_bench_alloc)]
use std::alloc::{GlobalAlloc, Layout};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::hint::black_box;
#[cfg(cfglib_bench_alloc)]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
#[cfg(not(cfglib_bench_alloc))]
use std::time::Instant;

use cfglib::dataflow::constprop::ConstantFolder;
use cfglib::{
    BlockId, Cfg, CfgBuilder, CommonAncestor, ConstValue, DenseNodeId, DirectedGraph, Direction,
    DominanceFrontiers, DominatorTree, EdgeKind, EdgeStep, FixpointResult, FlowControl, FlowEffect,
    InstrInfo, IntervalAnalysis, NaturalLoop, NodeFacts, NodeId, NodeProblem, PhiPlacements,
    Problem, ProgramPoint, Rooted, SccResult, SccpResult, SsaForm, SsaValue, TraversalDirection,
    ValueNumberInfo, ValueNumbering, breadth_first, breadth_first_edges, build_ssa,
    common_ancestors, constant_propagation, contract_edge, control_dependence_graph,
    depth_first_preorder, detect_loops, global_value_numbering, interval_analysis, merge_blocks,
    nearest_common_ancestor, place_phis, remove_empty_blocks, sccp, shortest_path,
    shortest_path_edges, solve_node_problem, tarjan_scc,
};

#[cfg(cfglib_bench_alloc)]
struct CountingAllocator;

#[cfg(cfglib_bench_alloc)]
static COUNT_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
#[cfg(cfglib_bench_alloc)]
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
#[cfg(cfglib_bench_alloc)]
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);
#[cfg(cfglib_bench_alloc)]
static LIVE_BYTES: AtomicU64 = AtomicU64::new(0);
#[cfg(cfglib_bench_alloc)]
static PEAK_LIVE_BYTES: AtomicU64 = AtomicU64::new(0);

#[cfg(cfglib_bench_alloc)]
fn record_allocation(size: usize) {
    let size = size as u64;
    let live = LIVE_BYTES.fetch_add(size, Ordering::Relaxed) + size;
    if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        ALLOCATED_BYTES.fetch_add(size, Ordering::Relaxed);
        PEAK_LIVE_BYTES.fetch_max(live, Ordering::Relaxed);
    }
}

#[cfg(cfglib_bench_alloc)]
fn record_deallocation(size: usize) {
    LIVE_BYTES.fetch_sub(size as u64, Ordering::Relaxed);
}

#[cfg(cfglib_bench_alloc)]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record_allocation(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            record_allocation(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        record_deallocation(layout.size());
    }

    unsafe fn realloc(&self, ptr: *mut u8, old: Layout, new_size: usize) -> *mut u8 {
        let pointer = unsafe { System.realloc(ptr, old, new_size) };
        if !pointer.is_null() {
            let old_size = old.size() as u64;
            let new_size = new_size as u64;
            let live = if new_size >= old_size {
                LIVE_BYTES.fetch_add(new_size - old_size, Ordering::Relaxed) + new_size - old_size
            } else {
                LIVE_BYTES.fetch_sub(old_size - new_size, Ordering::Relaxed) - old_size + new_size
            };
            if COUNT_ALLOCATIONS.load(Ordering::Relaxed) {
                ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
                ALLOCATED_BYTES.fetch_add(new_size, Ordering::Relaxed);
                PEAK_LIVE_BYTES.fetch_max(live, Ordering::Relaxed);
            }
        }
        pointer
    }
}

#[cfg(cfglib_bench_alloc)]
#[global_allocator]
static GLOBAL_ALLOCATOR: CountingAllocator = CountingAllocator;

#[cfg(not(cfglib_bench_alloc))]
#[global_allocator]
static GLOBAL_ALLOCATOR: System = System;

#[cfg(cfglib_bench_alloc)]
#[derive(Clone, Copy)]
struct AllocationSample {
    allocations: u64,
    allocated_bytes: u64,
    peak_live_bytes: u64,
}

#[cfg(cfglib_bench_alloc)]
fn allocation_sample<T>(mut operation: impl FnMut() -> T) -> AllocationSample {
    COUNT_ALLOCATIONS.store(false, Ordering::Relaxed);
    let live_before = LIVE_BYTES.load(Ordering::Relaxed);
    ALLOCATIONS.store(0, Ordering::Relaxed);
    ALLOCATED_BYTES.store(0, Ordering::Relaxed);
    PEAK_LIVE_BYTES.store(live_before, Ordering::Relaxed);
    COUNT_ALLOCATIONS.store(true, Ordering::Relaxed);
    drop(black_box(operation()));
    COUNT_ALLOCATIONS.store(false, Ordering::Relaxed);

    assert_eq!(
        LIVE_BYTES.load(Ordering::Relaxed),
        live_before,
        "benchmark operation changed live allocation bytes"
    );
    AllocationSample {
        allocations: ALLOCATIONS.load(Ordering::Relaxed),
        allocated_bytes: ALLOCATED_BYTES.load(Ordering::Relaxed),
        peak_live_bytes: PEAK_LIVE_BYTES
            .load(Ordering::Relaxed)
            .saturating_sub(live_before),
    }
}

#[cfg(not(cfglib_bench_alloc))]
fn run_iterations<T>(iterations: u64, operation: &mut impl FnMut() -> T) -> Duration {
    let start = Instant::now();
    for _ in 0..iterations {
        drop(black_box(operation()));
    }
    start.elapsed()
}

#[cfg(not(cfglib_bench_alloc))]
fn benchmark<T>(name: &str, target: Duration, mut operation: impl FnMut() -> T) {
    let mut iterations = 1_u64;
    loop {
        let elapsed = run_iterations(iterations, &mut operation);
        if elapsed >= target {
            break;
        }

        let elapsed_ns = elapsed.as_nanos().max(1);
        let target_ns = target.as_nanos();
        let scale = u64::try_from(target_ns.div_ceil(elapsed_ns)).unwrap_or(u64::MAX);
        iterations = iterations
            .saturating_mul(scale.clamp(2, 100))
            .max(iterations + 1);
    }

    let mut samples = [0.0_f64; 7];
    for sample in &mut samples {
        *sample =
            run_iterations(iterations, &mut operation).as_secs_f64() * 1e9 / iterations as f64;
    }
    samples.sort_unstable_by(f64::total_cmp);
    let median_ns = samples[samples.len() / 2];
    let minimum_ns = samples[0];

    println!("{name:<36} {median_ns:>12.1} ns/op  min {minimum_ns:>12.1}");
}

#[cfg(cfglib_bench_alloc)]
fn benchmark<T>(name: &str, _target: Duration, mut operation: impl FnMut() -> T) {
    // Exercise one unmeasured operation so lazy initialization does not become
    // part of an otherwise steady-state per-operation allocation sample.
    drop(black_box(operation()));
    let allocation = allocation_sample(operation);

    println!(
        "{name:<36} allocs {allocations:>8}  bytes {allocated_bytes:>12}  peak {peak_live_bytes:>10}",
        allocations = allocation.allocations,
        allocated_bytes = allocation.allocated_bytes,
        peak_live_bytes = allocation.peak_live_bytes,
    );
}

fn branchy_cfg(node_count: usize) -> Cfg<u32> {
    assert!(node_count > 0);
    let mut cfg = Cfg::new();
    let mut nodes = Vec::with_capacity(node_count);
    nodes.push(cfg.entry());
    for _ in 1..node_count {
        nodes.push(cfg.new_block());
    }
    for (index, &node) in nodes.iter().enumerate() {
        cfg.block_mut(node).push(index as u32);
        cfg.block_mut(node).push((index as u32).wrapping_mul(17));
    }
    for index in 0..node_count - 1 {
        cfg.add_edge(nodes[index], nodes[index + 1], EdgeKind::Fallthrough);
    }
    for index in (0..node_count.saturating_sub(2)).step_by(2) {
        cfg.add_edge(nodes[index], nodes[index + 2], EdgeKind::ConditionalTrue);
    }
    for index in (32..node_count).step_by(32) {
        cfg.add_edge(nodes[index], nodes[index - 16], EdgeKind::Back);
    }
    cfg
}

fn branchy_graph(node_count: usize) -> DirectedGraph<(), ()> {
    assert!(node_count > 0);
    let edge_capacity = node_count * 2 + node_count / 32;
    let mut graph = DirectedGraph::with_capacity(node_count, edge_capacity);
    let nodes: Vec<_> = (0..node_count).map(|_| graph.add_node(())).collect();
    for index in 0..node_count - 1 {
        graph.add_edge(nodes[index], nodes[index + 1], ());
    }
    for index in (0..node_count.saturating_sub(2)).step_by(2) {
        graph.add_edge(nodes[index], nodes[index + 2], ());
    }
    for index in (32..node_count).step_by(32) {
        graph.add_edge(nodes[index], nodes[index - 16], ());
    }
    graph
}

fn reverse_id_chain_graph(node_count: usize) -> (DirectedGraph<(), ()>, NodeId) {
    assert!(node_count > 0);
    let mut graph = DirectedGraph::with_capacity(node_count, node_count - 1);
    let nodes: Vec<_> = (0..node_count).map(|_| graph.add_node(())).collect();
    for index in 1..node_count {
        graph.add_edge(nodes[index], nodes[index - 1], ());
    }
    (graph, nodes[node_count - 1])
}

fn linear_cfg(node_count: usize) -> Cfg<u32> {
    assert!(node_count > 0);
    let mut cfg = Cfg::new();
    let mut previous = cfg.entry();
    cfg.block_mut(previous).push(0);
    for index in 1..node_count {
        let next = cfg.new_block();
        cfg.block_mut(next).push(index as u32);
        cfg.add_edge(previous, next, EdgeKind::Fallthrough);
        previous = next;
    }
    cfg
}

fn many_exit_cfg(exit_count: usize) -> Cfg<u32> {
    assert!(exit_count > 0);
    let mut cfg = Cfg::new();
    let entry = cfg.entry();
    cfg.block_mut(entry).push(0);
    for index in 0..exit_count {
        let exit = cfg.new_block();
        cfg.block_mut(exit).push((index + 1) as u32);
        cfg.add_edge(entry, exit, EdgeKind::SwitchCase);
    }
    cfg
}

fn empty_chain_cfg(node_count: usize) -> Cfg<u32> {
    let mut cfg = Cfg::new();
    let mut previous = cfg.entry();
    cfg.block_mut(previous).push(0);
    for index in 1..node_count {
        let next = cfg.new_block();
        cfg.add_edge(previous, next, EdgeKind::Fallthrough);
        previous = next;
        if index + 1 == node_count {
            cfg.block_mut(next).push(index as u32);
        }
    }
    cfg
}

fn high_fan_in_cfg(predecessor_count: usize) -> (Cfg<u32>, BlockId, BlockId) {
    let mut cfg = Cfg::new();
    let old_target = cfg.new_block();
    let new_target = cfg.new_block();
    for _ in 0..predecessor_count {
        let predecessor = cfg.new_block();
        cfg.add_edge(predecessor, old_target, EdgeKind::Unconditional);
    }
    (cfg, old_target, new_target)
}

fn weighted_high_fan_out_cfg(edge_count: usize) -> (Cfg<u32>, BlockId, BlockId) {
    assert!(edge_count >= 2);
    let mut cfg = Cfg::new();
    let source = cfg.entry();
    let target = cfg.new_block();
    let sink = cfg.new_block();
    cfg.block_mut(source).push(0);
    cfg.block_mut(target).push(1);
    cfg.block_mut(sink).push(2);
    cfg.add_edge(source, target, EdgeKind::Fallthrough);
    for index in 0..edge_count - 1 {
        let kind = if index % 2 == 0 {
            EdgeKind::ConditionalTrue
        } else {
            EdgeKind::ConditionalFalse
        };
        cfg.add_weighted_edge(target, sink, kind, index as f64 + 0.25);
    }
    cfg.add_weighted_edge(target, source, EdgeKind::Back, 0.875);
    (cfg, source, target)
}

fn irreducible_cfg(cycle_nodes: usize, external_entries: usize) -> Cfg<u32> {
    assert!(cycle_nodes >= 2);
    let mut cfg = Cfg::new();
    let cycle: Vec<_> = (0..cycle_nodes).map(|_| cfg.new_block()).collect();
    cfg.add_edge(cfg.entry(), cycle[0], EdgeKind::ConditionalTrue);
    for edge in cycle.windows(2) {
        cfg.add_edge(edge[0], edge[1], EdgeKind::Fallthrough);
    }
    cfg.add_edge(cycle[cycle_nodes - 1], cycle[0], EdgeKind::Back);

    for _ in 0..external_entries {
        let external = cfg.new_block();
        cfg.add_edge(cfg.entry(), external, EdgeKind::ConditionalFalse);
        cfg.add_edge(external, cycle[1], EdgeKind::Unconditional);
    }
    cfg
}

fn weighted_irreducible_cfg() -> Cfg<u32> {
    let mut cfg = Cfg::new();
    let entry = cfg.entry();
    let first = cfg.new_block();
    let second = cfg.new_block();
    let exit = cfg.new_block();
    for (index, block) in [entry, first, second, exit].into_iter().enumerate() {
        cfg.block_mut(block).push(index as u32);
    }

    cfg.add_edge(entry, first, EdgeKind::ConditionalTrue);
    cfg.add_weighted_edge(entry, second, EdgeKind::ConditionalFalse, 0.125);
    cfg.add_edge(first, second, EdgeKind::Fallthrough);
    cfg.add_weighted_edge(second, first, EdgeKind::Back, 0.75);
    cfg.add_weighted_edge(second, exit, EdgeKind::SwitchCase, 0.25);
    cfg
}

fn multi_latch_graph(chain_nodes: usize, latch_count: usize) -> (DirectedGraph<(), ()>, NodeId) {
    let mut graph =
        DirectedGraph::with_capacity(chain_nodes + latch_count + 1, chain_nodes + latch_count * 2);
    let header = graph.add_node(());
    let mut tail = header;
    for _ in 0..chain_nodes {
        let next = graph.add_node(());
        graph.add_edge(tail, next, ());
        tail = next;
    }
    for _ in 0..latch_count {
        let latch = graph.add_node(());
        graph.add_edge(tail, latch, ());
        graph.add_edge(latch, header, ());
    }
    (graph, header)
}

struct Reachability;

impl NodeProblem<DirectedGraph<(), ()>> for Reachability {
    type Fact = bool;

    fn direction(&self) -> Direction {
        Direction::Forward
    }

    fn bottom(&self, _graph: &DirectedGraph<(), ()>) -> bool {
        false
    }

    fn boundary(&self, _graph: &DirectedGraph<(), ()>) -> bool {
        true
    }

    fn meet(&self, a: &bool, b: &bool) -> bool {
        *a || *b
    }

    fn transfer(&self, _graph: &DirectedGraph<(), ()>, _node: NodeId, input: &bool) -> bool {
        *input
    }
}

struct CfgReachability;

impl Problem<u32> for CfgReachability {
    type Fact = bool;

    fn direction(&self) -> Direction {
        Direction::Forward
    }

    fn bottom(&self) -> bool {
        false
    }

    fn entry_fact(&self) -> bool {
        true
    }

    fn meet(&self, a: &bool, b: &bool) -> bool {
        *a || *b
    }

    fn transfer(&self, _cfg: &Cfg<u32>, _block: BlockId, input: &bool) -> bool {
        *input
    }
}

const WIDE_FACT_WORDS: usize = 256;

struct WideCfgFact;

impl Problem<u32> for WideCfgFact {
    type Fact = Vec<u64>;

    fn direction(&self) -> Direction {
        Direction::Forward
    }

    fn bottom(&self) -> Self::Fact {
        vec![0; WIDE_FACT_WORDS]
    }

    fn entry_fact(&self) -> Self::Fact {
        vec![u64::MAX; WIDE_FACT_WORDS]
    }

    fn meet(&self, a: &Self::Fact, b: &Self::Fact) -> Self::Fact {
        a.iter().zip(b).map(|(left, right)| left | right).collect()
    }

    fn transfer(&self, _cfg: &Cfg<u32>, _block: BlockId, input: &Self::Fact) -> Self::Fact {
        input.clone()
    }
}

struct WideNodeFact;

impl NodeProblem<DirectedGraph<(), ()>> for WideNodeFact {
    type Fact = Vec<u64>;

    fn direction(&self) -> Direction {
        Direction::Forward
    }

    fn bottom(&self, _graph: &DirectedGraph<(), ()>) -> Self::Fact {
        vec![0; WIDE_FACT_WORDS]
    }

    fn boundary(&self, _graph: &DirectedGraph<(), ()>) -> Self::Fact {
        vec![u64::MAX; WIDE_FACT_WORDS]
    }

    fn meet(&self, a: &Self::Fact, b: &Self::Fact) -> Self::Fact {
        a.iter().zip(b).map(|(left, right)| left | right).collect()
    }

    fn transfer(
        &self,
        _graph: &DirectedGraph<(), ()>,
        _node: NodeId,
        input: &Self::Fact,
    ) -> Self::Fact {
        input.clone()
    }
}

struct ConstantInst {
    defs: Vec<u32>,
    uses: Vec<u32>,
    value: u64,
}

impl InstrInfo for ConstantInst {
    type Variable = u32;

    fn uses(&self) -> &[u32] {
        &self.uses
    }

    fn defs(&self) -> &[u32] {
        &self.defs
    }
}

impl ConstantFolder for ConstantInst {
    type Const = u64;

    fn fold_constant(&self, _known: &BTreeMap<u32, u64>) -> Option<(u32, u64)> {
        Some((self.defs[0], self.value))
    }
}

impl ValueNumberInfo for ConstantInst {
    type Operation = u64;

    fn operation(&self) -> u64 {
        self.value
    }

    fn is_pure(&self) -> bool {
        true
    }
}

fn independent_constants(instruction_count: usize) -> Cfg<ConstantInst> {
    let mut cfg = Cfg::new();
    for variable in 0..instruction_count as u32 {
        cfg.block_mut(cfg.entry()).push(ConstantInst {
            defs: vec![variable],
            uses: Vec::new(),
            value: u64::from(variable),
        });
    }
    cfg
}

fn linear_constants(block_count: usize) -> Cfg<ConstantInst> {
    let mut cfg = Cfg::new();
    let mut block = cfg.entry();
    for index in 0..block_count {
        cfg.block_mut(block).push(ConstantInst {
            defs: vec![0],
            uses: Vec::new(),
            value: index as u64,
        });
        if index + 1 < block_count {
            let next = cfg.new_block();
            cfg.add_edge(block, next, EdgeKind::Fallthrough);
            block = next;
        }
    }
    cfg
}

fn phi_storm_cfg(layer_count: usize, variable_count: usize) -> Cfg<ConstantInst> {
    let mut cfg = Cfg::new();
    let mut branch = cfg.entry();
    for layer in 0..layer_count {
        let left = cfg.new_block();
        let right = cfg.new_block();
        let merge = cfg.new_block();
        cfg.add_edge(branch, left, EdgeKind::ConditionalTrue);
        cfg.add_edge(branch, right, EdgeKind::ConditionalFalse);
        cfg.add_edge(left, merge, EdgeKind::Fallthrough);
        cfg.add_edge(right, merge, EdgeKind::Fallthrough);
        for variable in 0..variable_count as u32 {
            for block in [left, right] {
                cfg.block_mut(block).push(ConstantInst {
                    defs: vec![variable],
                    uses: Vec::new(),
                    value: ((layer as u64) << 32) | u64::from(variable),
                });
            }
        }
        branch = merge;
    }
    cfg
}

#[derive(Clone, Copy)]
struct BuilderInst(FlowEffect);

impl FlowControl for BuilderInst {
    fn flow_effect(&self) -> FlowEffect {
        self.0
    }
}

fn build_if_else_chain(region_count: usize) -> Cfg<BuilderInst> {
    CfgBuilder::build((0..region_count).flat_map(|_| {
        [
            BuilderInst(FlowEffect::ConditionalOpen),
            BuilderInst(FlowEffect::Fallthrough),
            BuilderInst(FlowEffect::ConditionalAlternate),
            BuilderInst(FlowEffect::Fallthrough),
            BuilderInst(FlowEffect::ConditionalClose),
        ]
    }))
    .expect("synthetic conditionals are balanced")
}

fn build_conditional_break_chain(region_count: usize) -> Cfg<BuilderInst> {
    CfgBuilder::build((0..region_count).flat_map(|_| {
        [
            BuilderInst(FlowEffect::LoopOpen),
            BuilderInst(FlowEffect::ConditionalBreak),
            BuilderInst(FlowEffect::LoopClose),
        ]
    }))
    .expect("synthetic loops are balanced")
}

fn build_two_case_switch_chain(region_count: usize) -> Cfg<BuilderInst> {
    CfgBuilder::build((0..region_count).flat_map(|_| {
        [
            BuilderInst(FlowEffect::SwitchOpen),
            BuilderInst(FlowEffect::Fallthrough),
            BuilderInst(FlowEffect::SwitchCase),
            BuilderInst(FlowEffect::Fallthrough),
            BuilderInst(FlowEffect::SwitchClose),
        ]
    }))
    .expect("synthetic switches are balanced")
}

fn build_eight_case_switch_chain(region_count: usize) -> Cfg<BuilderInst> {
    CfgBuilder::build((0..region_count).flat_map(|_| {
        (0..17).map(|position| {
            let effect = match position {
                0 => FlowEffect::SwitchOpen,
                16 => FlowEffect::SwitchClose,
                odd if odd % 2 == 1 => FlowEffect::Fallthrough,
                _ => FlowEffect::SwitchCase,
            };
            BuilderInst(effect)
        })
    }))
    .expect("synthetic switches are balanced")
}

fn configuration_error(message: &str) -> ! {
    eprintln!("cfglib benchmark configuration error: {message}");
    std::process::exit(2);
}

fn benchmark_target() -> (u64, Duration) {
    let target_ms = match std::env::var("CFGLIB_BENCH_MS") {
        Ok(value) => match value.parse::<u64>() {
            Ok(0) => configuration_error("CFGLIB_BENCH_MS must be greater than zero"),
            Ok(value) => value,
            Err(_) => configuration_error("CFGLIB_BENCH_MS must be a positive integer"),
        },
        Err(std::env::VarError::NotPresent) => 75,
        Err(std::env::VarError::NotUnicode(_)) => {
            configuration_error("CFGLIB_BENCH_MS must be valid UTF-8")
        }
    };
    (target_ms, Duration::from_millis(target_ms))
}

fn run_semantic_oracle<T>(operation: &mut impl FnMut() -> T, oracle: impl FnOnce(&T)) {
    let result = operation();
    oracle(&result);
    drop(result);
}

fn assert_cfg_shape<I>(cfg: &Cfg<I>, expected_blocks: usize, expected_edges: usize) {
    assert_eq!(
        cfg.num_blocks(),
        expected_blocks,
        "unexpected CFG block count"
    );
    assert_eq!(cfg.num_edges(), expected_edges, "unexpected CFG edge count");
    let verification = cfglib::verify(cfg);
    assert!(
        verification.is_ok(),
        "invalid benchmark CFG: {:?}",
        verification.errors
    );
}

fn assert_directed_shape<N, E>(
    graph: &DirectedGraph<N, E>,
    expected_nodes: usize,
    expected_edges: usize,
) {
    assert_eq!(
        graph.node_count(),
        expected_nodes,
        "unexpected directed-graph node count"
    );
    assert_eq!(
        graph.edge_count(),
        expected_edges,
        "unexpected directed-graph edge count"
    );

    let mut outgoing_count = 0;
    let mut incoming_count = 0;
    for node in graph.node_ids() {
        let outgoing: BTreeSet<_> = graph.outgoing_edges(node).iter().copied().collect();
        let incoming: BTreeSet<_> = graph.incoming_edges(node).iter().copied().collect();
        assert_eq!(
            outgoing.len(),
            graph.outgoing_edges(node).len(),
            "duplicate outgoing edge identity"
        );
        assert_eq!(
            incoming.len(),
            graph.incoming_edges(node).len(),
            "duplicate incoming edge identity"
        );
        for &edge in graph.outgoing_edges(node) {
            assert_eq!(graph.edge(edge).source(), node);
        }
        for &edge in graph.incoming_edges(node) {
            assert_eq!(graph.edge(edge).target(), node);
        }
        outgoing_count += outgoing.len();
        incoming_count += incoming.len();
    }
    assert_eq!(outgoing_count, expected_edges);
    assert_eq!(incoming_count, expected_edges);
    for edge in graph.edges() {
        assert!(graph.outgoing_edges(edge.source()).contains(&edge.id()));
        assert!(graph.incoming_edges(edge.target()).contains(&edge.id()));
    }
}

fn assert_dense_permutation<N>(nodes: &[N], expected_count: usize)
where
    N: Copy + DenseNodeId + core::fmt::Debug,
{
    assert_eq!(nodes.len(), expected_count);
    let mut seen = vec![false; expected_count];
    for &node in nodes {
        assert!(
            node.index() < expected_count,
            "node is out of range: {node:?}"
        );
        assert!(!seen[node.index()], "duplicate node in traversal: {node:?}");
        seen[node.index()] = true;
    }
    assert!(seen.into_iter().all(core::convert::identity));
}

fn branchy_edge_count(node_count: usize) -> usize {
    (node_count - 1) + node_count.saturating_sub(2).div_ceil(2) + node_count.saturating_sub(1) / 32
}

fn has_cfg_edge<I>(cfg: &Cfg<I>, source: BlockId, target: BlockId, kind: EdgeKind) -> bool {
    cfg.successor_edges(source).iter().any(|&edge| {
        let edge = cfg.edge(edge);
        edge.target() == target && edge.kind() == kind
    })
}

fn has_directed_edge<N, E>(graph: &DirectedGraph<N, E>, source: NodeId, target: NodeId) -> bool {
    graph
        .outgoing_edges(source)
        .iter()
        .any(|&edge| graph.edge(edge).target() == target)
}

fn assert_branchy_cfg(cfg: &Cfg<u32>, node_count: usize) {
    assert_cfg_shape(cfg, node_count, branchy_edge_count(node_count));
    for index in 0..node_count {
        let block = BlockId::from_raw(index as u32);
        assert_eq!(
            cfg.block(block).instructions(),
            &[index as u32, (index as u32).wrapping_mul(17)]
        );
        if index + 1 < node_count {
            assert!(has_cfg_edge(
                cfg,
                block,
                BlockId::from_raw((index + 1) as u32),
                EdgeKind::Fallthrough
            ));
        }
    }
    for index in (0..node_count.saturating_sub(2)).step_by(2) {
        assert!(has_cfg_edge(
            cfg,
            BlockId::from_raw(index as u32),
            BlockId::from_raw((index + 2) as u32),
            EdgeKind::ConditionalTrue
        ));
    }
    for index in (32..node_count).step_by(32) {
        assert!(has_cfg_edge(
            cfg,
            BlockId::from_raw(index as u32),
            BlockId::from_raw((index - 16) as u32),
            EdgeKind::Back
        ));
    }
}

fn assert_branchy_graph(graph: &DirectedGraph<(), ()>, node_count: usize) {
    assert_directed_shape(graph, node_count, branchy_edge_count(node_count));
    for index in 0..node_count - 1 {
        assert!(has_directed_edge(
            graph,
            NodeId::from_index(index),
            NodeId::from_index(index + 1)
        ));
    }
    for index in (0..node_count.saturating_sub(2)).step_by(2) {
        assert!(has_directed_edge(
            graph,
            NodeId::from_index(index),
            NodeId::from_index(index + 2)
        ));
    }
    for index in (32..node_count).step_by(32) {
        assert!(has_directed_edge(
            graph,
            NodeId::from_index(index),
            NodeId::from_index(index - 16)
        ));
    }
}

fn assert_builder_cfg(
    cfg: &Cfg<BuilderInst>,
    expected_blocks: usize,
    expected_edges: usize,
    expected_effects: &[(FlowEffect, usize)],
    expected_edge_kinds: &[(EdgeKind, usize)],
) {
    assert_cfg_shape(cfg, expected_blocks, expected_edges);
    assert_eq!(cfg.dfs_preorder().len(), expected_blocks);

    let instructions: Vec<_> = cfg
        .blocks()
        .iter()
        .flat_map(cfglib::BasicBlock::instructions)
        .collect();
    assert_eq!(
        instructions.len(),
        expected_effects.iter().map(|(_, count)| count).sum()
    );
    for &(effect, expected) in expected_effects {
        assert_eq!(
            instructions
                .iter()
                .filter(|instruction| instruction.0 == effect)
                .count(),
            expected,
            "unexpected {effect:?} instruction count"
        );
    }
    assert!(instructions.iter().all(|instruction| {
        expected_effects
            .iter()
            .any(|(effect, _)| instruction.0 == *effect)
    }));
    for &(kind, expected) in expected_edge_kinds {
        assert_eq!(
            cfg.edges().filter(|edge| edge.kind() == kind).count(),
            expected,
            "unexpected {kind:?} edge count"
        );
    }
    assert_eq!(
        expected_edge_kinds
            .iter()
            .map(|(_, count)| count)
            .sum::<usize>(),
        expected_edges
    );
}

fn assert_linear_cfg(cfg: &Cfg<u32>, node_count: usize) {
    assert_cfg_shape(cfg, node_count, node_count - 1);
    for index in 0..node_count {
        let block = BlockId::from_raw(index as u32);
        assert_eq!(cfg.block(block).instructions(), &[index as u32]);
        if index + 1 < node_count {
            assert_eq!(cfg.successor_edges(block).len(), 1);
            let edge = cfg.edge(cfg.successor_edges(block)[0]);
            assert_eq!(edge.target(), BlockId::from_raw((index + 1) as u32));
            assert_eq!(edge.kind(), EdgeKind::Fallthrough);
        } else {
            assert_eq!(cfg.successor_edges(block).len(), 0);
        }
    }
}

fn assert_empty_chain(cfg: &Cfg<u32>, node_count: usize) {
    assert_cfg_shape(cfg, node_count, node_count - 1);
    assert_eq!(cfg.block(cfg.entry()).instructions(), &[0]);
    for index in 1..node_count - 1 {
        assert_eq!(
            cfg.block(BlockId::from_raw(index as u32))
                .instructions()
                .len(),
            0
        );
    }
    assert_eq!(
        cfg.block(BlockId::from_raw((node_count - 1) as u32))
            .instructions(),
        &[(node_count - 1) as u32]
    );
}

fn assert_high_fan_in(
    cfg: &Cfg<u32>,
    predecessor_count: usize,
    old_target: BlockId,
    new_target: BlockId,
    redirected: bool,
) {
    assert_cfg_shape(cfg, predecessor_count + 3, predecessor_count);
    let expected_target = if redirected { new_target } else { old_target };
    let empty_target = if redirected { old_target } else { new_target };
    assert_eq!(cfg.predecessor_edges(empty_target).len(), 0);
    assert_eq!(
        cfg.predecessor_edges(expected_target).len(),
        predecessor_count
    );
    for (index, &edge_id) in cfg.predecessor_edges(expected_target).iter().enumerate() {
        assert_eq!(edge_id.index(), index);
        let edge = cfg.edge(edge_id);
        assert_eq!(edge.source(), BlockId::from_raw((index + 3) as u32));
        assert_eq!(edge.target(), expected_target);
        assert_eq!(edge.kind(), EdgeKind::Unconditional);
        assert!(edge.weight().is_none());
    }
}

fn assert_weighted_fan_out(
    cfg: &Cfg<u32>,
    edge_count: usize,
    source: BlockId,
    target: BlockId,
    merged: bool,
    target_retains_instructions: bool,
) {
    let live_edges = if merged { edge_count } else { edge_count + 1 };
    assert_cfg_shape(cfg, 3, live_edges);
    let sink = BlockId::from_raw(2);
    let outgoing_source = if merged { source } else { target };

    if merged {
        assert_eq!(cfg.block(source).instructions(), &[0, 1]);
        if target_retains_instructions {
            assert_eq!(cfg.block(target).instructions(), &[1]);
        } else {
            assert_eq!(cfg.block(target).instructions().len(), 0);
        }
        assert_eq!(cfg.successor_edges(target).len(), 0);
    } else {
        assert_eq!(cfg.block(source).instructions(), &[0]);
        assert_eq!(cfg.block(target).instructions(), &[1]);
        let connecting = cfg.edge(cfglib::EdgeId::from_raw(0));
        assert_eq!(connecting.source(), source);
        assert_eq!(connecting.target(), target);
        assert_eq!(connecting.kind(), EdgeKind::Fallthrough);
    }
    assert_eq!(cfg.block(sink).instructions(), &[2]);
    assert_eq!(cfg.successor_edges(outgoing_source).len(), edge_count);

    assert_weighted_outgoing_edges(cfg, edge_count, outgoing_source, source);
}

fn assert_weighted_outgoing_edges(
    cfg: &Cfg<u32>,
    edge_count: usize,
    outgoing_source: BlockId,
    back_target: BlockId,
) {
    let sink = BlockId::from_raw(2);
    for index in 0..edge_count - 1 {
        let id = cfglib::EdgeId::from_raw((index + 1) as u32);
        let edge = cfg.edge(id);
        assert_eq!(edge.id(), id);
        assert_eq!(edge.source(), outgoing_source);
        assert_eq!(edge.target(), sink);
        assert_eq!(
            edge.kind(),
            if index % 2 == 0 {
                EdgeKind::ConditionalTrue
            } else {
                EdgeKind::ConditionalFalse
            }
        );
        assert_eq!(
            edge.weight().map(f64::to_bits),
            Some((index as f64 + 0.25).to_bits())
        );
    }
    let back = cfg.edge(cfglib::EdgeId::from_raw(edge_count as u32));
    assert_eq!(back.source(), outgoing_source);
    assert_eq!(back.target(), back_target);
    assert_eq!(back.kind(), EdgeKind::Back);
    assert_eq!(back.weight().map(f64::to_bits), Some(0.875_f64.to_bits()));
}

fn assert_split_weighted_fan_out(
    cfg: &Cfg<u32>,
    edge_count: usize,
    source: BlockId,
    target: BlockId,
    split: BlockId,
) {
    assert_cfg_shape(cfg, 4, edge_count + 2);
    assert_eq!(split, BlockId::from_raw(3));
    assert_eq!(cfg.block(source).instructions(), &[0]);
    assert_eq!(cfg.block(target).instructions(), &[1]);
    assert_eq!(cfg.block(split).instructions().len(), 0);
    assert_eq!(cfg.block(BlockId::from_raw(2)).instructions(), &[2]);

    let connecting = cfg.edge(cfglib::EdgeId::from_raw(0));
    assert_eq!(connecting.source(), source);
    assert_eq!(connecting.target(), target);
    assert_eq!(connecting.kind(), EdgeKind::Fallthrough);
    assert!(connecting.weight().is_none());

    let [fallthrough] = cfg.successor_edges(target) else {
        panic!("split source should have one outgoing edge");
    };
    assert_eq!(fallthrough.index(), edge_count + 1);
    let fallthrough = cfg.edge(*fallthrough);
    assert_eq!(fallthrough.source(), target);
    assert_eq!(fallthrough.target(), split);
    assert_eq!(fallthrough.kind(), EdgeKind::Fallthrough);
    assert!(fallthrough.weight().is_none());
    assert_eq!(cfg.successor_edges(split).len(), edge_count);
    assert_weighted_outgoing_edges(cfg, edge_count, split, source);
}

fn assert_weighted_irreducible(cfg: &Cfg<u32>, made_reducible: bool) {
    let split_count = usize::from(made_reducible);
    assert_cfg_shape(cfg, 4 + split_count, 5 + 2 * split_count);
    for index in 0..4 {
        assert_eq!(
            cfg.block(BlockId::from_index(index)).instructions(),
            &[index as u32]
        );
    }

    let dominators = DominatorTree::compute(cfg);
    assert_eq!(
        cfglib::graph::structure::is_reducible(cfg, &dominators),
        made_reducible
    );

    let entry = BlockId::from_raw(0);
    let first = BlockId::from_raw(1);
    let second = BlockId::from_raw(2);
    let exit = BlockId::from_raw(3);
    let redirected_target = if made_reducible {
        BlockId::from_raw(4)
    } else {
        second
    };
    let expected = [
        (entry, first, EdgeKind::ConditionalTrue, None),
        (
            entry,
            redirected_target,
            EdgeKind::ConditionalFalse,
            Some(0.125_f64),
        ),
        (first, second, EdgeKind::Fallthrough, None),
        (second, first, EdgeKind::Back, Some(0.75_f64)),
        (second, exit, EdgeKind::SwitchCase, Some(0.25_f64)),
    ];
    for (index, (source, target, kind, weight)) in expected.into_iter().enumerate() {
        let edge = cfg.edge(cfglib::EdgeId::from_raw(index as u32));
        assert_eq!(edge.source(), source);
        assert_eq!(edge.target(), target);
        assert_eq!(edge.kind(), kind);
        assert_eq!(edge.weight().map(f64::to_bits), weight.map(f64::to_bits));
    }

    if made_reducible {
        let copy = BlockId::from_raw(4);
        assert_eq!(cfg.block(copy).instructions(), &[2]);
        assert_eq!(
            cfg.successor_edges(copy),
            &[cfglib::EdgeId::from_raw(5), cfglib::EdgeId::from_raw(6)]
        );
        for (index, (target, kind, weight)) in [
            (first, EdgeKind::Back, 0.75_f64),
            (exit, EdgeKind::SwitchCase, 0.25_f64),
        ]
        .into_iter()
        .enumerate()
        {
            let edge = cfg.edge(cfglib::EdgeId::from_raw((5 + index) as u32));
            assert_eq!(edge.source(), copy);
            assert_eq!(edge.target(), target);
            assert_eq!(edge.kind(), kind);
            assert_eq!(edge.weight().map(f64::to_bits), Some(weight.to_bits()));
        }
    }
}

fn assert_irreducible_fixture(
    cfg: &Cfg<u32>,
    cycle_nodes: usize,
    external_entries: usize,
    expected_splits: usize,
) {
    let original_blocks = 1 + cycle_nodes + external_entries;
    let original_edges = cycle_nodes + 1 + external_entries * 2;
    assert_cfg_shape(
        cfg,
        original_blocks + expected_splits,
        original_edges + expected_splits,
    );
    assert!(
        cfg.blocks()
            .iter()
            .all(|block| block.instructions().is_empty())
    );

    let dominators = DominatorTree::compute(cfg);
    assert_eq!(
        cfglib::graph::structure::is_reducible(cfg, &dominators),
        expected_splits > 0
    );

    let cycle_entry = BlockId::from_raw(2);
    if expected_splits > 0 {
        assert_eq!(expected_splits, cycle_nodes - 1);
        let first_copy = BlockId::from_index(original_blocks);
        for index in 0..external_entries {
            let external = BlockId::from_index(1 + cycle_nodes + index);
            let outgoing = cfg.successor_edges(external);
            assert_eq!(outgoing.len(), 1);
            let edge = cfg.edge(outgoing[0]);
            assert_eq!(edge.target(), first_copy);
            assert_eq!(edge.kind(), EdgeKind::Unconditional);
            assert!(edge.weight().is_none());
        }
        assert_eq!(cfg.predecessor_edges(cycle_entry).len(), 1);
        for split_index in 0..expected_splits {
            let original = BlockId::from_index(2 + split_index);
            let copy = BlockId::from_index(original_blocks + split_index);
            let original_outgoing = cfg.successor_edges(original);
            let copied_outgoing = cfg.successor_edges(copy);
            assert_eq!(original_outgoing.len(), 1);
            assert_eq!(copied_outgoing.len(), 1);
            let original_edge = cfg.edge(original_outgoing[0]);
            let copied_edge = cfg.edge(copied_outgoing[0]);
            let expected_target = if split_index + 1 < expected_splits {
                BlockId::from_index(original_blocks + split_index + 1)
            } else {
                BlockId::from_index(1)
            };
            assert_eq!(copied_edge.source(), copy);
            assert_eq!(copied_edge.target(), expected_target);
            assert_eq!(copied_edge.kind(), original_edge.kind());
            assert_eq!(
                copied_edge.weight().map(f64::to_bits),
                original_edge.weight().map(f64::to_bits)
            );
            assert_eq!(copied_edge.id().index(), original_edges + split_index);
        }
    } else {
        assert_eq!(
            cfg.predecessor_edges(cycle_entry).len(),
            external_entries + 1
        );
    }
}

fn directed_distances(
    graph: &DirectedGraph<(), ()>,
    start: NodeId,
    direction: TraversalDirection,
) -> (Vec<usize>, Vec<NodeId>) {
    let mut distances = vec![usize::MAX; graph.node_count()];
    let mut order = Vec::with_capacity(graph.node_count());
    let mut queue = VecDeque::new();
    distances[start.index()] = 0;
    queue.push_back(start);
    while let Some(node) = queue.pop_front() {
        order.push(node);
        let adjacent = match direction {
            TraversalDirection::Outgoing => graph.outgoing_edges(node),
            TraversalDirection::Incoming => graph.incoming_edges(node),
        };
        for &edge_id in adjacent {
            let edge = graph.edge(edge_id);
            let next = match direction {
                TraversalDirection::Outgoing => edge.target(),
                TraversalDirection::Incoming => edge.source(),
            };
            if distances[next.index()] == usize::MAX {
                distances[next.index()] = distances[node.index()] + 1;
                queue.push_back(next);
            }
        }
    }
    (distances, order)
}

fn reference_cfg_preorder(cfg: &Cfg<u32>) -> Vec<BlockId> {
    let mut visited = vec![false; cfg.num_blocks()];
    let mut order = Vec::with_capacity(cfg.num_blocks());
    let mut stack = vec![cfg.entry()];
    while let Some(block) = stack.pop() {
        if visited[block.index()] {
            continue;
        }
        visited[block.index()] = true;
        order.push(block);
        let successors: Vec<_> = cfg.successors(block).collect();
        stack.extend(
            successors
                .into_iter()
                .rev()
                .filter(|successor| !visited[successor.index()]),
        );
    }
    order
}

fn reference_cfg_breadth_first(cfg: &Cfg<u32>) -> Vec<BlockId> {
    let mut visited = vec![false; cfg.num_blocks()];
    let mut order = Vec::with_capacity(cfg.num_blocks());
    let mut queue = VecDeque::from([cfg.entry()]);
    visited[cfg.entry().index()] = true;
    while let Some(block) = queue.pop_front() {
        order.push(block);
        for successor in cfg.successors(block) {
            if !visited[successor.index()] {
                visited[successor.index()] = true;
                queue.push_back(successor);
            }
        }
    }
    order
}

fn assert_edge_traversal(steps: &[EdgeStep], graph: &DirectedGraph<(), ()>) {
    assert_eq!(steps.len(), graph.edge_count());
    let mut seen = vec![false; graph.edge_slot_count()];
    for step in steps {
        assert!(!seen[step.edge.index()], "edge traversal repeated an edge");
        seen[step.edge.index()] = true;
        let edge = graph.edge(step.edge);
        assert_eq!(step.source, edge.source());
        assert_eq!(step.target, edge.target());
    }
    assert!(graph.edges().all(|edge| seen[edge.id().index()]));

    let mut expected = Vec::with_capacity(graph.edge_count());
    let mut expanded = vec![false; graph.node_count()];
    let mut queue = VecDeque::from([NodeId::from_raw(0)]);
    expanded[0] = true;
    while let Some(node) = queue.pop_front() {
        for &edge_id in graph.outgoing_edges(node) {
            let edge = graph.edge(edge_id);
            expected.push(EdgeStep {
                edge: edge_id,
                source: edge.source(),
                target: edge.target(),
            });
            if !expanded[edge.target().index()] {
                expanded[edge.target().index()] = true;
                queue.push_back(edge.target());
            }
        }
    }
    assert_eq!(steps, expected);
}

fn assert_node_path(path: &[NodeId], graph: &DirectedGraph<(), ()>, from: NodeId, to: NodeId) {
    assert_eq!(path.first(), Some(&from));
    assert_eq!(path.last(), Some(&to));
    let (distances, _) = directed_distances(graph, from, TraversalDirection::Outgoing);
    assert_eq!(path.len(), distances[to.index()] + 1);
    assert!(
        path.windows(2)
            .all(|pair| has_directed_edge(graph, pair[0], pair[1]))
    );
}

fn assert_edge_path(
    path: &[cfglib::graph::directed::EdgeId],
    graph: &DirectedGraph<(), ()>,
    from: NodeId,
    to: NodeId,
) {
    let (distances, _) = directed_distances(graph, from, TraversalDirection::Outgoing);
    assert_eq!(path.len(), distances[to.index()]);
    let mut current = from;
    let mut seen = BTreeSet::new();
    for &edge_id in path {
        assert!(seen.insert(edge_id), "shortest path repeated an edge");
        let edge = graph.edge(edge_id);
        assert_eq!(edge.source(), current);
        current = edge.target();
    }
    assert_eq!(current, to);
}

fn assert_common_ancestor_results(
    results: &[CommonAncestor<NodeId>],
    graph: &DirectedGraph<(), ()>,
    a: NodeId,
    b: NodeId,
) {
    let (from_a, _) = directed_distances(graph, a, TraversalDirection::Incoming);
    let (from_b, b_order) = directed_distances(graph, b, TraversalDirection::Incoming);
    let expected: Vec<_> = b_order
        .into_iter()
        .filter(|node| from_a[node.index()] != usize::MAX)
        .map(|node| CommonAncestor {
            node,
            from_a: from_a[node.index()],
            from_b: from_b[node.index()],
        })
        .collect();
    assert_eq!(results, expected);
}

fn assert_branchy_dominators(dominators: &DominatorTree, node_count: usize) {
    for index in 0..node_count {
        let block = BlockId::from_index(index);
        assert!(dominators.is_reachable(block));
        if index == 0 {
            assert_eq!(dominators.idom(block), None);
            assert_eq!(dominators.depth(block), Some(0));
        } else {
            let parent = ((index - 1) / 2) * 2;
            assert_eq!(dominators.idom(block), Some(BlockId::from_index(parent)));
            assert_eq!(dominators.depth(block), Some(index.div_ceil(2)));
        }
    }
}

fn assert_dominance_frontiers(
    frontiers: &DominanceFrontiers,
    cfg: &Cfg<u32>,
    dominators: &DominatorTree,
) {
    let mut expected = vec![BTreeSet::new(); cfg.num_blocks()];
    for block in cfg.blocks() {
        if cfg.predecessor_edges(block.id()).len() < 2 {
            continue;
        }
        let root = dominators.idom(block.id()).unwrap_or(block.id());
        for predecessor in cfg.predecessors(block.id()) {
            let mut runner = predecessor;
            while runner != root {
                expected[runner.index()].insert(block.id());
                let Some(parent) = dominators.idom(runner) else {
                    break;
                };
                runner = parent;
            }
        }
    }
    for block in cfg.blocks() {
        assert_eq!(
            frontiers.frontier(block.id()),
            &expected[block.id().index()]
        );
    }
}

fn assert_branchy_post_dominators(dominators: &DominatorTree, node_count: usize) {
    for index in 0..node_count {
        let block = BlockId::from_index(index);
        assert!(dominators.is_reachable(block));
        let expected = if index + 1 == node_count {
            None
        } else if index % 2 == 0 {
            Some(BlockId::from_index((index + 2).min(node_count - 1)))
        } else {
            Some(BlockId::from_index(index + 1))
        };
        assert_eq!(dominators.idom(block), expected);
    }
}

fn assert_control_dependence_graph(
    result: &DirectedGraph<BlockId, ()>,
    cfg: &Cfg<u32>,
    post_dominators: &DominatorTree,
) {
    assert_eq!(result.node_count(), cfg.num_blocks());
    for node in result.node_ids() {
        assert_eq!(*result.node(node), BlockId::from_index(node.index()));
    }

    let mut expected = BTreeSet::new();
    for controller in cfg.blocks() {
        let controller = controller.id();
        for target in cfg.successors(controller) {
            if post_dominators.dominates(target, controller) {
                continue;
            }
            let immediate = post_dominators.idom(controller);
            let mut dependent = target;
            loop {
                expected.insert((controller, dependent));
                match post_dominators.idom(dependent) {
                    Some(next) if Some(next) != immediate => dependent = next,
                    _ => break,
                }
            }
        }
    }
    let actual: BTreeSet<_> = result
        .edges()
        .map(|edge| (*result.node(edge.source()), *result.node(edge.target())))
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(result.edge_count(), expected.len());
}

fn assert_cfg_intervals(result: &IntervalAnalysis, cfg: &Cfg<u32>) {
    assert_eq!(result.levels.len(), 1);
    assert_ne!(result.levels[0].len(), 0);
    let mut assigned = BTreeSet::new();
    for interval in &result.levels[0] {
        assert!(interval.blocks.contains(&interval.header));
        for &block in &interval.blocks {
            assert!(assigned.insert(block), "block appeared in two intervals");
            if block != interval.header {
                assert!(
                    cfg.predecessors(block)
                        .all(|predecessor| interval.blocks.contains(&predecessor)),
                    "non-header interval block has an external predecessor"
                );
            }
        }
    }
    assert_eq!(assigned.len(), cfg.num_blocks());
    assert_eq!(result.is_reducible, result.levels[0].len() <= 1);
}

fn assert_reverse_chain_intervals(
    result: &IntervalAnalysis<NodeId>,
    node_count: usize,
    root: NodeId,
) {
    assert_eq!(result.levels.len(), 1);
    assert_eq!(result.levels[0].len(), 1);
    assert!(result.is_reducible);
    let interval = &result.levels[0][0];
    assert_eq!(interval.header, root);
    assert_eq!(interval.blocks.len(), node_count);
    assert!((0..node_count).all(|index| interval.blocks.contains(&NodeId::from_index(index))));
}

fn assert_bool_node_facts(facts: &NodeFacts<bool>, node_count: usize) {
    for index in 0..node_count {
        let node = NodeId::from_index(index);
        assert!(*facts.fact_in(node));
        assert!(*facts.fact_out(node));
    }
}

fn assert_wide_node_facts(facts: &NodeFacts<Vec<u64>>, node_count: usize) {
    for index in 0..node_count {
        let node = NodeId::from_index(index);
        for fact in [facts.fact_in(node), facts.fact_out(node)] {
            assert_eq!(fact.len(), WIDE_FACT_WORDS);
            assert!(fact.iter().all(|&word| word == u64::MAX));
        }
    }
}

fn assert_bool_cfg_facts(facts: &FixpointResult<bool>, block_count: usize) {
    assert_eq!(facts.block_in.len(), block_count);
    assert_eq!(facts.block_out.len(), block_count);
    assert!(facts.block_in.iter().all(|fact| *fact));
    assert!(facts.block_out.iter().all(|fact| *fact));
}

fn assert_wide_cfg_facts(facts: &FixpointResult<Vec<u64>>, block_count: usize) {
    assert_eq!(facts.block_in.len(), block_count);
    assert_eq!(facts.block_out.len(), block_count);
    for fact in facts.block_in.iter().chain(&facts.block_out) {
        assert_eq!(fact.len(), WIDE_FACT_WORDS);
        assert!(fact.iter().all(|&word| word == u64::MAX));
    }
}

fn assert_linear_ssa(ssa: &SsaForm<u32>, block_count: usize) {
    assert_eq!(ssa.blocks().len(), block_count);
    assert_eq!(ssa.phis().count(), 0);
    for index in 0..block_count {
        let block = BlockId::from_index(index);
        let ssa_block = ssa.block(block);
        assert_eq!(ssa_block.block, block);
        assert_eq!(ssa_block.phis.len(), 0);
        assert_eq!(ssa_block.instructions.len(), 1);
        let instruction = &ssa_block.instructions[0];
        assert_eq!(instruction.point, ProgramPoint { block, inst_idx: 0 });
        assert_eq!(instruction.uses.len(), 0);
        assert_eq!(instruction.defs, [SsaValue::new(0, index + 1)]);
    }
    assert_eq!(ssa.max_version(&0), block_count);
}

fn assert_phi_placements(
    placements: &PhiPlacements<u32>,
    layer_count: usize,
    variable_count: usize,
) {
    assert_eq!(placements.len(), layer_count * variable_count);
    for layer in 0..layer_count {
        let left = BlockId::from_index(3 * layer + 1);
        let right = BlockId::from_index(3 * layer + 2);
        let merge = BlockId::from_index(3 * layer + 3);
        let at_merge = placements.at(merge);
        assert_eq!(at_merge.len(), variable_count);
        for (variable, placement) in at_merge.iter().enumerate() {
            assert_eq!(placement.variable, variable as u32);
            assert_eq!(placement.predecessors, [left, right]);
        }
    }
    for index in 0..=3 * layer_count {
        if index % 3 != 0 || index == 0 {
            assert_eq!(placements.at(BlockId::from_index(index)).len(), 0);
        }
    }
}

fn assert_phi_ssa(
    ssa: &SsaForm<u32>,
    source: &Cfg<ConstantInst>,
    layer_count: usize,
    variable_count: usize,
) {
    assert_eq!(ssa.blocks().len(), source.num_blocks());
    assert_eq!(ssa.phis().count(), layer_count * variable_count);
    let mut definitions = BTreeSet::new();
    for block in source.blocks() {
        let ssa_block = ssa.block(block.id());
        assert_eq!(ssa_block.block, block.id());
        assert_eq!(ssa_block.instructions.len(), block.instructions().len());
        for (index, annotation) in ssa_block.instructions.iter().enumerate() {
            assert_eq!(
                annotation.point,
                ProgramPoint {
                    block: block.id(),
                    inst_idx: index
                }
            );
            assert_eq!(annotation.uses.len(), 0);
            assert_eq!(annotation.defs.len(), 1);
            assert!(annotation.defs[0].version > 0);
            assert!(definitions.insert(annotation.defs[0].clone()));
        }
        for phi in &ssa_block.phis {
            assert!(phi.result.version > 0);
            assert!(definitions.insert(phi.result.clone()));
            assert_eq!(
                phi.operands.len(),
                source.predecessor_edges(block.id()).len()
            );
            assert_eq!(
                phi.operands
                    .iter()
                    .map(|(block, _)| *block)
                    .collect::<Vec<_>>(),
                source.predecessors(block.id()).collect::<Vec<_>>()
            );
            assert!(
                phi.operands
                    .iter()
                    .all(|(_, value)| value.variable == phi.result.variable)
            );
        }
    }
    assert_eq!(definitions.len(), layer_count * variable_count * 3);
    for variable in 0..variable_count as u32 {
        assert_eq!(ssa.max_version(&variable), layer_count * 3);
    }
}

fn main() {
    const NODE_COUNT: usize = 4_096;
    const BUILDER_REGION_COUNT: usize = 2_048;

    // Cargo passes `--bench` to custom benchmark binaries. Treat only a
    // positional argument as the optional name filter.
    let filter = std::env::args()
        .skip(1)
        .find(|argument| !argument.starts_with('-'))
        .unwrap_or_default();
    let (target_ms, target) = benchmark_target();
    #[cfg(cfglib_bench_alloc)]
    let _ = target_ms;
    let cfg = branchy_cfg(NODE_COUNT);
    let cfg_dominators = DominatorTree::compute(&cfg);
    let cfg_post_dominators = DominatorTree::compute_post(&cfg);
    let graph = branchy_graph(NODE_COUNT);
    let (reverse_id_chain, reverse_id_root) = reverse_id_chain_graph(NODE_COUNT);
    let wide_cfg = branchy_cfg(1_024);
    let wide_graph = branchy_graph(1_024);
    let linear = linear_cfg(NODE_COUNT / 2);
    let many_exits = many_exit_cfg(NODE_COUNT - 1);
    let linear_dominators = DominatorTree::compute(&linear);
    let empty_chain = empty_chain_cfg(NODE_COUNT / 2);
    let (high_fan_in, old_target, new_target) = high_fan_in_cfg(NODE_COUNT);
    let (weighted_high_fan_out, fan_out_source, fan_out_target) =
        weighted_high_fan_out_cfg(NODE_COUNT);
    let irreducible_small = irreducible_cfg(2, 512);
    let irreducible_large = irreducible_cfg(512, 512);
    let weighted_irreducible = weighted_irreducible_cfg();
    let (multi_latch, multi_latch_root) = multi_latch_graph(2_048, 256);
    let multi_latch_dominators =
        DominatorTree::compute(&Rooted::new(&multi_latch, multi_latch_root));
    let constants = independent_constants(1_024);
    let constant_dominators = DominatorTree::compute(&constants);
    let constant_ssa = build_ssa(&constants, &constant_dominators);
    let linear_constant_cfg = linear_constants(2_048);
    let linear_constant_dominators = DominatorTree::compute(&linear_constant_cfg);
    let phi_storm = phi_storm_cfg(32, 128);
    let phi_storm_dominators = DominatorTree::compute(&phi_storm);

    println!(
        "cfglib synthetic benchmark: {NODE_COUNT} nodes, {} CFG edges, {} graph edges",
        cfg.num_edges(),
        graph.edge_count()
    );
    #[cfg(cfglib_bench_alloc)]
    println!(
        "mode: allocation instrumentation (counting wrapper over System; CPU timing disabled)"
    );
    #[cfg(not(cfglib_bench_alloc))]
    println!("mode: CPU (direct System global allocator; no counting wrapper)");
    #[cfg(not(cfglib_bench_alloc))]
    println!("timing: median of 7 samples; target >= {target_ms} ms/sample");
    #[cfg(cfglib_bench_alloc)]
    println!(
        "memory: allocations and requested bytes per operation; peak is incremental live bytes"
    );

    let mut registered = 0_usize;
    let mut matched = 0_usize;
    macro_rules! bench {
        ($name:literal, $operation:expr, $oracle:expr) => {{
            registered += 1;
            if filter.is_empty() || $name.contains(&filter) {
                matched += 1;
                let mut operation = $operation;
                run_semantic_oracle(&mut operation, $oracle);
                benchmark($name, target, operation);
            }
        }};
    }

    bench!(
        "cfg_build_branchy",
        || branchy_cfg(NODE_COUNT),
        |result: &Cfg<u32>| assert_branchy_cfg(result, NODE_COUNT)
    );
    bench!(
        "cfg_builder_if_else_chain",
        || build_if_else_chain(BUILDER_REGION_COUNT),
        |result: &Cfg<BuilderInst>| assert_builder_cfg(
            result,
            1 + 3 * BUILDER_REGION_COUNT,
            4 * BUILDER_REGION_COUNT,
            &[
                (FlowEffect::ConditionalOpen, BUILDER_REGION_COUNT),
                (FlowEffect::ConditionalAlternate, BUILDER_REGION_COUNT),
                (FlowEffect::Fallthrough, 2 * BUILDER_REGION_COUNT),
            ],
            &[
                (EdgeKind::ConditionalTrue, BUILDER_REGION_COUNT),
                (EdgeKind::ConditionalFalse, BUILDER_REGION_COUNT),
                (EdgeKind::Fallthrough, 2 * BUILDER_REGION_COUNT),
            ],
        )
    );
    bench!(
        "cfg_builder_conditional_break_chain",
        || { build_conditional_break_chain(BUILDER_REGION_COUNT) },
        |result: &Cfg<BuilderInst>| assert_builder_cfg(
            result,
            1 + 4 * BUILDER_REGION_COUNT,
            5 * BUILDER_REGION_COUNT,
            &[
                (FlowEffect::LoopOpen, BUILDER_REGION_COUNT),
                (FlowEffect::ConditionalBreak, BUILDER_REGION_COUNT),
            ],
            &[
                (EdgeKind::Fallthrough, BUILDER_REGION_COUNT),
                (EdgeKind::ConditionalTrue, BUILDER_REGION_COUNT),
                (EdgeKind::ConditionalFalse, BUILDER_REGION_COUNT),
                (EdgeKind::Back, BUILDER_REGION_COUNT),
                (EdgeKind::Unconditional, BUILDER_REGION_COUNT),
            ],
        )
    );
    bench!(
        "cfg_builder_two_case_switch_chain",
        || { build_two_case_switch_chain(BUILDER_REGION_COUNT) },
        |result: &Cfg<BuilderInst>| assert_builder_cfg(
            result,
            1 + 3 * BUILDER_REGION_COUNT,
            4 * BUILDER_REGION_COUNT,
            &[
                (FlowEffect::SwitchOpen, BUILDER_REGION_COUNT),
                (FlowEffect::SwitchCase, BUILDER_REGION_COUNT),
                (FlowEffect::Fallthrough, 2 * BUILDER_REGION_COUNT),
            ],
            &[
                (EdgeKind::SwitchCase, 2 * BUILDER_REGION_COUNT),
                (EdgeKind::Fallthrough, 2 * BUILDER_REGION_COUNT),
            ],
        )
    );
    bench!(
        "cfg_builder_eight_case_switch_chain",
        || { build_eight_case_switch_chain(BUILDER_REGION_COUNT / 4) },
        |result: &Cfg<BuilderInst>| {
            let regions = BUILDER_REGION_COUNT / 4;
            assert_builder_cfg(
                result,
                1 + 9 * regions,
                16 * regions,
                &[
                    (FlowEffect::SwitchOpen, regions),
                    (FlowEffect::SwitchCase, 7 * regions),
                    (FlowEffect::Fallthrough, 8 * regions),
                ],
                &[
                    (EdgeKind::SwitchCase, 8 * regions),
                    (EdgeKind::Fallthrough, 8 * regions),
                ],
            );
        }
    );
    bench!(
        "directed_build_branchy",
        || branchy_graph(NODE_COUNT),
        |result: &DirectedGraph<(), ()>| assert_branchy_graph(result, NODE_COUNT)
    );
    bench!(
        "cfg_depth_first_preorder",
        || depth_first_preorder(&cfg, cfg.entry(), TraversalDirection::Outgoing),
        |result: &Vec<BlockId>| {
            assert_dense_permutation(result, NODE_COUNT);
            assert_eq!(*result, reference_cfg_preorder(&cfg));
        }
    );
    bench!(
        "cfg_breadth_first",
        || breadth_first(&cfg, cfg.entry(), TraversalDirection::Outgoing),
        |result: &Vec<BlockId>| {
            assert_dense_permutation(result, NODE_COUNT);
            assert_eq!(*result, reference_cfg_breadth_first(&cfg));
        }
    );
    bench!(
        "directed_breadth_first_edges",
        || breadth_first_edges(&graph, NodeId::from_raw(0), TraversalDirection::Outgoing),
        |result: &Vec<EdgeStep>| assert_edge_traversal(result, &graph)
    );
    bench!(
        "directed_shortest_path",
        || {
            shortest_path(
                &graph,
                NodeId::from_raw(0),
                NodeId::from_index(NODE_COUNT - 1),
                TraversalDirection::Outgoing,
            )
            .expect("fixture target is reachable")
        },
        |result: &Vec<NodeId>| assert_node_path(
            result,
            &graph,
            NodeId::from_raw(0),
            NodeId::from_index(NODE_COUNT - 1),
        )
    );
    bench!(
        "directed_shortest_path_edges",
        || {
            shortest_path_edges(
                &graph,
                NodeId::from_raw(0),
                NodeId::from_index(NODE_COUNT - 1),
                TraversalDirection::Outgoing,
            )
            .expect("fixture target is reachable")
        },
        |result: &Vec<cfglib::graph::directed::EdgeId>| assert_edge_path(
            result,
            &graph,
            NodeId::from_raw(0),
            NodeId::from_index(NODE_COUNT - 1),
        )
    );
    bench!(
        "directed_nearest_common_ancestor",
        || {
            nearest_common_ancestor(
                &graph,
                NodeId::from_index(NODE_COUNT - 1),
                NodeId::from_index(NODE_COUNT - 2),
                TraversalDirection::Incoming,
            )
        },
        |result: &Option<NodeId>| {
            let a = NodeId::from_index(NODE_COUNT - 1);
            let b = NodeId::from_index(NODE_COUNT - 2);
            let (from_a, _) = directed_distances(&graph, a, TraversalDirection::Incoming);
            let (from_b, _) = directed_distances(&graph, b, TraversalDirection::Incoming);
            let expected = graph
                .node_ids()
                .filter(|node| {
                    from_a[node.index()] != usize::MAX && from_b[node.index()] != usize::MAX
                })
                .min_by_key(|node| (from_a[node.index()] + from_b[node.index()], node.index()));
            assert_eq!(*result, expected);
        }
    );
    bench!(
        "directed_common_ancestors",
        || {
            common_ancestors(
                &graph,
                NodeId::from_index(NODE_COUNT - 1),
                NodeId::from_index(NODE_COUNT - 2),
                TraversalDirection::Incoming,
                None,
            )
        },
        |result: &Vec<CommonAncestor<NodeId>>| assert_common_ancestor_results(
            result,
            &graph,
            NodeId::from_index(NODE_COUNT - 1),
            NodeId::from_index(NODE_COUNT - 2),
        )
    );
    bench!(
        "cfg_dominators",
        || DominatorTree::compute(&cfg),
        |result: &DominatorTree| assert_branchy_dominators(result, NODE_COUNT)
    );
    bench!(
        "cfg_dominance_frontiers",
        || { DominanceFrontiers::compute(&cfg, &cfg_dominators) },
        |result: &DominanceFrontiers| assert_dominance_frontiers(result, &cfg, &cfg_dominators,)
    );
    bench!(
        "cfg_post_dominators",
        || DominatorTree::compute_post(&cfg),
        |result: &DominatorTree| assert_branchy_post_dominators(result, NODE_COUNT)
    );
    bench!(
        "cfg_post_dominators_many_exits",
        || { DominatorTree::compute_post(&many_exits) },
        |result: &DominatorTree| {
            for index in 0..NODE_COUNT {
                let block = BlockId::from_index(index);
                assert!(result.is_reachable(block));
                assert_eq!(result.idom(block), None);
            }
        }
    );
    bench!(
        "cfg_control_dependence_graph",
        || { control_dependence_graph(&cfg, &cfg_post_dominators) },
        |result: &DirectedGraph<BlockId, ()>| assert_control_dependence_graph(
            result,
            &cfg,
            &cfg_post_dominators,
        )
    );
    bench!(
        "cfg_dominator_depths_linear",
        || { linear_dominators.depths() },
        |result: &Vec<usize>| {
            assert_eq!(result.len(), NODE_COUNT / 2);
            assert!(result.iter().copied().eq(0..NODE_COUNT / 2));
        }
    );
    bench!(
        "directed_tarjan_scc",
        || tarjan_scc(&graph),
        |result: &SccResult<NodeId>| {
            let cycle_count = (NODE_COUNT - 1) / 32;
            assert_eq!(result.len(), NODE_COUNT - 16 * cycle_count);
            let mut seen = vec![false; NODE_COUNT];
            for (component_index, component) in result.components.iter().enumerate() {
                assert!(!component.nodes.is_empty());
                for &node in &component.nodes {
                    assert!(!seen[node.index()]);
                    seen[node.index()] = true;
                    assert_eq!(result.component_index(node), component_index);
                }
            }
            assert!(seen.into_iter().all(core::convert::identity));
            for cycle in 1..=cycle_count {
                let end = cycle * 32;
                let component = result.component(NodeId::from_index(end));
                assert_eq!(component.nodes.len(), 17);
                assert!(
                    (end - 16..=end).all(|index| component.contains(NodeId::from_index(index)))
                );
            }
            assert_eq!(
                result
                    .components
                    .iter()
                    .filter(|component| component.nodes.len() == 17)
                    .count(),
                cycle_count
            );
            assert!(
                result
                    .components
                    .iter()
                    .all(|component| { component.nodes.len() == 1 || component.nodes.len() == 17 })
            );
        }
    );
    bench!(
        "directed_detect_loops_multilatch",
        || detect_loops(&multi_latch, &multi_latch_dominators),
        |result: &Vec<NaturalLoop<NodeId>>| {
            assert_eq!(result.len(), 1);
            let natural_loop = &result[0];
            assert_eq!(natural_loop.header, multi_latch_root);
            assert_eq!(natural_loop.depth, 0);
            assert_eq!(natural_loop.body.len(), 2_048 + 256 + 1);
            assert!(
                (0..=2_048 + 256)
                    .all(|index| natural_loop.body.contains(&NodeId::from_index(index)))
            );
            assert_eq!(natural_loop.latches.len(), 256);
            assert!(
                (2_049..=2_304)
                    .all(|index| natural_loop.latches.contains(&NodeId::from_index(index)))
            );
        }
    );
    bench!(
        "cfg_interval_analysis",
        || interval_analysis(&cfg),
        |result: &IntervalAnalysis| assert_cfg_intervals(result, &cfg)
    );
    bench!(
        "directed_interval_reverse_id_chain",
        || { interval_analysis(&Rooted::new(&reverse_id_chain, reverse_id_root)) },
        |result: &IntervalAnalysis<NodeId>| assert_reverse_chain_intervals(
            result,
            NODE_COUNT,
            reverse_id_root,
        )
    );
    bench!(
        "directed_node_fixpoint_bool",
        || solve_node_problem(&graph, &Reachability),
        |result: &NodeFacts<bool>| assert_bool_node_facts(result, NODE_COUNT)
    );
    bench!(
        "directed_node_fixpoint_wide",
        || solve_node_problem(&wide_graph, &WideNodeFact),
        |result: &NodeFacts<Vec<u64>>| assert_wide_node_facts(result, 1_024)
    );
    bench!(
        "cfg_fixpoint_bool",
        || { cfglib::dataflow::fixpoint::solve(&cfg, &CfgReachability) },
        |result: &FixpointResult<bool>| assert_bool_cfg_facts(result, NODE_COUNT)
    );
    bench!(
        "cfg_fixpoint_wide",
        || { cfglib::dataflow::fixpoint::solve(&wide_cfg, &WideCfgFact) },
        |result: &FixpointResult<Vec<u64>>| assert_wide_cfg_facts(result, 1_024)
    );
    bench!(
        "cfg_sccp_independent_constants",
        || sccp(&constants, &constant_ssa),
        |result: &SccpResult<u32, u64>| {
            assert_eq!(result.reachable_blocks, BTreeSet::from([constants.entry()]));
            assert!(result.executable_edges.is_empty());
            assert_eq!(result.values.len(), 1_024);
            for variable in 0..1_024_u32 {
                assert_eq!(
                    result.values.get(&SsaValue::new(variable, 1)),
                    Some(&ConstValue::Const(u64::from(variable)))
                );
            }
        }
    );
    bench!(
        "cfg_build_ssa_linear",
        || build_ssa(&linear_constant_cfg, &linear_constant_dominators),
        |result: &SsaForm<u32>| assert_linear_ssa(result, 2_048)
    );
    bench!(
        "cfg_place_phis_phi_storm",
        || place_phis(&phi_storm, &phi_storm_dominators),
        |result: &PhiPlacements<u32>| assert_phi_placements(result, 32, 128)
    );
    bench!(
        "cfg_build_ssa_phi_storm",
        || build_ssa(&phi_storm, &phi_storm_dominators),
        |result: &SsaForm<u32>| assert_phi_ssa(result, &phi_storm, 32, 128)
    );
    bench!(
        "cfg_global_value_numbering_linear",
        || { global_value_numbering(&linear_constant_cfg, &linear_constant_dominators) },
        |result: &ValueNumbering| {
            assert_eq!(result.blocks.len(), 2_048);
            assert_eq!(result.num_values, 2_048);
            for index in 0..2_048 {
                let block = result
                    .blocks
                    .get(&BlockId::from_index(index))
                    .expect("value numbering omitted a block");
                assert_eq!(block.inst_vn, [Some(index as u32)]);
                assert_eq!(block.redundant.len(), 0);
            }
        }
    );
    bench!(
        "cfg_constprop_independent_constants",
        || { constant_propagation(&constants) },
        |result: &FixpointResult<BTreeMap<u32, ConstValue<u64>>>| {
            assert_eq!(result.block_in.len(), 1);
            assert_eq!(result.block_out.len(), 1);
            assert!(result.block_in[0].is_empty());
            assert_eq!(result.block_out[0].len(), 1_024);
            for variable in 0..1_024_u32 {
                assert_eq!(
                    result.block_out[0].get(&variable),
                    Some(&ConstValue::Const(u64::from(variable)))
                );
            }
        }
    );
    bench!("cfg_clone_linear", || linear.clone(), |result: &Cfg<
        u32,
    >| {
        assert_linear_cfg(result, NODE_COUNT / 2);
    });
    bench!(
        "cfg_clone_merge_linear",
        || {
            let mut cloned = linear.clone();
            let merged = merge_blocks(&mut cloned);
            (cloned, merged)
        },
        |(result, merged): &(Cfg<u32>, usize)| {
            assert_eq!(*merged, NODE_COUNT / 2 - 1);
            assert_cfg_shape(result, NODE_COUNT / 2, 0);
            assert_eq!(
                result.block(result.entry()).instructions(),
                &(0..NODE_COUNT as u32 / 2).collect::<Vec<_>>()
            );
            assert!(
                result.blocks()[1..]
                    .iter()
                    .all(|block| block.instructions().is_empty())
            );
        }
    );
    bench!(
        "cfg_clone_empty_chain",
        || empty_chain.clone(),
        |result: &Cfg<u32>| assert_empty_chain(result, NODE_COUNT / 2)
    );
    bench!(
        "cfg_clone_remove_empty_chain",
        || {
            let mut cloned = empty_chain.clone();
            let removed = remove_empty_blocks(&mut cloned);
            (cloned, removed)
        },
        |(result, removed): &(Cfg<u32>, usize)| {
            assert_eq!(*removed, NODE_COUNT / 2 - 2);
            assert_cfg_shape(result, NODE_COUNT / 2, 1);
            assert_eq!(result.block(result.entry()).instructions(), &[0]);
            let last = BlockId::from_index(NODE_COUNT / 2 - 1);
            assert_eq!(
                result.block(last).instructions(),
                &[(NODE_COUNT / 2 - 1) as u32]
            );
            let edge = result.edge(cfglib::EdgeId::from_raw(0));
            assert_eq!(edge.source(), result.entry());
            assert_eq!(edge.target(), last);
            assert_eq!(edge.kind(), EdgeKind::Fallthrough);
            assert!(edge.weight().is_none());
        }
    );
    bench!(
        "cfg_clone_high_fan_in",
        || high_fan_in.clone(),
        |result: &Cfg<u32>| assert_high_fan_in(result, NODE_COUNT, old_target, new_target, false,)
    );
    bench!(
        "cfg_clone_redirect_high_fan_in",
        || {
            let mut cloned = high_fan_in.clone();
            cloned.redirect_edges_to(old_target, new_target);
            cloned
        },
        |result: &Cfg<u32>| assert_high_fan_in(result, NODE_COUNT, old_target, new_target, true,)
    );
    bench!(
        "cfg_clone_weighted_high_fan_out",
        || { weighted_high_fan_out.clone() },
        |result: &Cfg<u32>| assert_weighted_fan_out(
            result,
            NODE_COUNT,
            fan_out_source,
            fan_out_target,
            false,
            false,
        )
    );
    bench!(
        "cfg_clone_split_weighted_high_fan_out",
        || {
            let mut cloned = weighted_high_fan_out.clone();
            let split = cloned.split_block(fan_out_target, 1);
            (cloned, split)
        },
        |(result, split): &(Cfg<u32>, BlockId)| assert_split_weighted_fan_out(
            result,
            NODE_COUNT,
            fan_out_source,
            fan_out_target,
            *split,
        )
    );
    bench!(
        "cfg_clone_merge_weighted_high_fan_out",
        || {
            let mut cloned = weighted_high_fan_out.clone();
            let merged = merge_blocks(&mut cloned);
            (cloned, merged)
        },
        |(result, merged): &(Cfg<u32>, usize)| {
            assert_eq!(*merged, 1);
            assert_weighted_fan_out(
                result,
                NODE_COUNT,
                fan_out_source,
                fan_out_target,
                true,
                false,
            );
        }
    );
    bench!(
        "cfg_clone_contract_weighted_high_fan_out",
        || {
            let mut cloned = weighted_high_fan_out.clone();
            let contracted = contract_edge(&mut cloned, fan_out_source, fan_out_target);
            (cloned, contracted)
        },
        |(result, contracted): &(Cfg<u32>, bool)| {
            assert!(*contracted);
            assert_weighted_fan_out(
                result,
                NODE_COUNT,
                fan_out_source,
                fan_out_target,
                true,
                true,
            );
        }
    );
    bench!(
        "cfg_clone_irreducible_small",
        || irreducible_small.clone(),
        |result: &Cfg<u32>| assert_irreducible_fixture(result, 2, 512, 0)
    );
    bench!(
        "cfg_clone_make_reducible_small",
        || {
            let mut cloned = irreducible_small.clone();
            let splits = cfglib::graph::reducible::make_reducible(&mut cloned);
            (cloned, splits)
        },
        |(result, splits): &(Cfg<u32>, usize)| {
            assert_eq!(*splits, 1);
            assert_irreducible_fixture(result, 2, 512, 1);
        }
    );
    bench!(
        "cfg_clone_irreducible_large",
        || irreducible_large.clone(),
        |result: &Cfg<u32>| assert_irreducible_fixture(result, 512, 512, 0)
    );
    bench!(
        "cfg_clone_make_reducible_large",
        || {
            let mut cloned = irreducible_large.clone();
            let splits = cfglib::graph::reducible::make_reducible(&mut cloned);
            (cloned, splits)
        },
        |(result, splits): &(Cfg<u32>, usize)| {
            assert_eq!(*splits, 511);
            assert_irreducible_fixture(result, 512, 512, 511);
        }
    );
    bench!(
        "cfg_clone_weighted_irreducible",
        || weighted_irreducible.clone(),
        |result: &Cfg<u32>| assert_weighted_irreducible(result, false)
    );
    bench!(
        "cfg_clone_make_reducible_weighted",
        || {
            let mut cloned = weighted_irreducible.clone();
            let splits = cfglib::graph::reducible::make_reducible(&mut cloned);
            (cloned, splits)
        },
        |(result, splits): &(Cfg<u32>, usize)| {
            assert_eq!(*splits, 1);
            assert_weighted_irreducible(result, true);
        }
    );

    assert_eq!(registered, 49, "benchmark registration count changed");
    if !filter.is_empty() && matched == 0 {
        configuration_error(&format!(
            "benchmark filter `{filter}` matched no registered cases"
        ));
    }
}
