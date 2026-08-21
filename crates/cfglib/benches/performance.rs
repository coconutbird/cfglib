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
use std::collections::BTreeMap;
use std::hint::black_box;
#[cfg(cfglib_bench_alloc)]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
#[cfg(not(cfglib_bench_alloc))]
use std::time::Instant;

use cfglib::dataflow::constprop::ConstantFolder;
use cfglib::{
    BlockId, Cfg, CfgBuilder, DirectedGraph, Direction, DominanceFrontiers, DominatorTree,
    EdgeKind, FlowControl, FlowEffect, InstrInfo, NodeId, NodeProblem, Problem, Rooted,
    TraversalDirection, ValueNumberInfo, breadth_first, breadth_first_edges, build_ssa,
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

fn main() {
    const NODE_COUNT: usize = 4_096;
    const BUILDER_REGION_COUNT: usize = 2_048;

    // Cargo passes `--bench` to custom benchmark binaries. Treat only a
    // positional argument as the optional name filter.
    let filter = std::env::args()
        .skip(1)
        .find(|argument| !argument.starts_with('-'))
        .unwrap_or_default();
    let target_ms = std::env::var("CFGLIB_BENCH_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(75_u64);
    let target = Duration::from_millis(target_ms);
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

    macro_rules! bench {
        ($name:literal, $operation:expr) => {
            if filter.is_empty() || $name.contains(&filter) {
                benchmark($name, target, $operation);
            }
        };
    }

    bench!("cfg_build_branchy", || branchy_cfg(NODE_COUNT));
    bench!("cfg_builder_if_else_chain", || build_if_else_chain(
        BUILDER_REGION_COUNT
    ));
    bench!("cfg_builder_conditional_break_chain", || {
        build_conditional_break_chain(BUILDER_REGION_COUNT)
    });
    bench!("cfg_builder_two_case_switch_chain", || {
        build_two_case_switch_chain(BUILDER_REGION_COUNT)
    });
    bench!("cfg_builder_eight_case_switch_chain", || {
        build_eight_case_switch_chain(BUILDER_REGION_COUNT / 4)
    });
    bench!("directed_build_branchy", || branchy_graph(NODE_COUNT));
    bench!("cfg_depth_first_preorder", || depth_first_preorder(
        &cfg,
        cfg.entry(),
        TraversalDirection::Outgoing
    ));
    bench!("cfg_breadth_first", || breadth_first(
        &cfg,
        cfg.entry(),
        TraversalDirection::Outgoing
    ));
    bench!("directed_breadth_first_edges", || breadth_first_edges(
        &graph,
        NodeId::from_raw(0),
        TraversalDirection::Outgoing
    ));
    bench!("directed_shortest_path", || {
        shortest_path(
            &graph,
            NodeId::from_raw(0),
            NodeId::from_index(NODE_COUNT - 1),
            TraversalDirection::Outgoing,
        )
        .expect("fixture target is reachable")
    });
    bench!("directed_shortest_path_edges", || {
        shortest_path_edges(
            &graph,
            NodeId::from_raw(0),
            NodeId::from_index(NODE_COUNT - 1),
            TraversalDirection::Outgoing,
        )
        .expect("fixture target is reachable")
    });
    bench!("directed_nearest_common_ancestor", || {
        nearest_common_ancestor(
            &graph,
            NodeId::from_index(NODE_COUNT - 1),
            NodeId::from_index(NODE_COUNT - 2),
            TraversalDirection::Incoming,
        )
    });
    bench!("directed_common_ancestors", || {
        common_ancestors(
            &graph,
            NodeId::from_index(NODE_COUNT - 1),
            NodeId::from_index(NODE_COUNT - 2),
            TraversalDirection::Incoming,
            None,
        )
    });
    bench!("cfg_dominators", || DominatorTree::compute(&cfg));
    bench!("cfg_dominance_frontiers", || {
        DominanceFrontiers::compute(&cfg, &cfg_dominators)
    });
    bench!("cfg_post_dominators", || DominatorTree::compute_post(&cfg));
    bench!("cfg_post_dominators_many_exits", || {
        DominatorTree::compute_post(&many_exits)
    });
    bench!("cfg_control_dependence_graph", || {
        control_dependence_graph(&cfg, &cfg_post_dominators)
    });
    bench!("cfg_dominator_depths_linear", || {
        linear_dominators.depths()
    });
    bench!("directed_tarjan_scc", || tarjan_scc(&graph));
    bench!("directed_detect_loops_multilatch", || detect_loops(
        &multi_latch,
        &multi_latch_dominators
    ));
    bench!("cfg_interval_analysis", || interval_analysis(&cfg));
    bench!("directed_interval_reverse_id_chain", || {
        interval_analysis(&Rooted::new(&reverse_id_chain, reverse_id_root))
    });
    bench!("directed_node_fixpoint_bool", || solve_node_problem(
        &graph,
        &Reachability
    ));
    bench!("directed_node_fixpoint_wide", || solve_node_problem(
        &wide_graph,
        &WideNodeFact
    ));
    bench!("cfg_fixpoint_bool", || {
        cfglib::dataflow::fixpoint::solve(&cfg, &CfgReachability)
    });
    bench!("cfg_fixpoint_wide", || {
        cfglib::dataflow::fixpoint::solve(&wide_cfg, &WideCfgFact)
    });
    bench!("cfg_sccp_independent_constants", || sccp(
        &constants,
        &constant_ssa
    ));
    bench!("cfg_build_ssa_linear", || build_ssa(
        &linear_constant_cfg,
        &linear_constant_dominators
    ));
    bench!("cfg_place_phis_phi_storm", || place_phis(
        &phi_storm,
        &phi_storm_dominators
    ));
    bench!("cfg_build_ssa_phi_storm", || build_ssa(
        &phi_storm,
        &phi_storm_dominators
    ));
    bench!("cfg_global_value_numbering_linear", || {
        global_value_numbering(&linear_constant_cfg, &linear_constant_dominators)
    });
    bench!("cfg_constprop_independent_constants", || {
        constant_propagation(&constants)
    });
    bench!("cfg_clone_linear", || linear.clone());
    bench!("cfg_clone_merge_linear", || {
        let mut cloned = linear.clone();
        let merged = merge_blocks(&mut cloned);
        (cloned, merged)
    });
    bench!("cfg_clone_empty_chain", || empty_chain.clone());
    bench!("cfg_clone_remove_empty_chain", || {
        let mut cloned = empty_chain.clone();
        let removed = remove_empty_blocks(&mut cloned);
        (cloned, removed)
    });
    bench!("cfg_clone_high_fan_in", || high_fan_in.clone());
    bench!("cfg_clone_redirect_high_fan_in", || {
        let mut cloned = high_fan_in.clone();
        cloned.redirect_edges_to(old_target, new_target);
        cloned
    });
    bench!("cfg_clone_weighted_high_fan_out", || {
        weighted_high_fan_out.clone()
    });
    bench!("cfg_clone_merge_weighted_high_fan_out", || {
        let mut cloned = weighted_high_fan_out.clone();
        let merged = merge_blocks(&mut cloned);
        (cloned, merged)
    });
    bench!("cfg_clone_contract_weighted_high_fan_out", || {
        let mut cloned = weighted_high_fan_out.clone();
        let contracted = contract_edge(&mut cloned, fan_out_source, fan_out_target);
        (cloned, contracted)
    });
    bench!("cfg_clone_irreducible_small", || irreducible_small.clone());
    bench!("cfg_clone_make_reducible_small", || {
        let mut cloned = irreducible_small.clone();
        let splits = cfglib::graph::reducible::make_reducible(&mut cloned);
        (cloned, splits)
    });
    bench!("cfg_clone_irreducible_large", || irreducible_large.clone());
    bench!("cfg_clone_make_reducible_large", || {
        let mut cloned = irreducible_large.clone();
        let splits = cfglib::graph::reducible::make_reducible(&mut cloned);
        (cloned, splits)
    });
}
