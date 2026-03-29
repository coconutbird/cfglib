//! Switch table reconstruction.
//!
//! Detects indirect jumps that follow a `base + index * scale` pattern
//! (x86 jump tables, ARM TBB/TBH, etc.) and recovers structured
//! [`EdgeKind::SwitchCase`] edges from them.
//!
//! The consumer implements [`SwitchCandidate`] for its instruction
//! type to describe potential indirect jumps, and this module handles
//! the CFG rewiring.

extern crate alloc;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::edge::EdgeKind;
use crate::flow::FlowControl;

/// Description of a potential switch / jump-table site.
#[derive(Debug, Clone)]
pub struct JumpTableInfo {
    /// The block containing the indirect jump.
    pub block: BlockId,
    /// Known case target addresses (resolved by the consumer).
    pub targets: Vec<u64>,
    /// Optional default / fallthrough target.
    pub default_target: Option<u64>,
}

/// Trait that an instruction type implements to expose potential
/// indirect-jump sites for switch table recovery.
///
/// The consumer inspects its instruction stream and returns any
/// detected jump-table patterns. This keeps the pattern matching
/// ISA-specific while the CFG rewiring is generic.
pub trait SwitchCandidate: FlowControl {
    /// Scan the instructions of `block` and return a `JumpTableInfo`
    /// if the block ends in an indirect jump that looks like a switch.
    fn detect_switch_table<I>(cfg: &Cfg<I>, block: BlockId) -> Option<JumpTableInfo>;
}

/// Result of switch table reconstruction.
#[derive(Debug, Clone)]
pub struct SwitchRecovery {
    /// Block that was converted from indirect jump to switch.
    pub block: BlockId,
    /// Number of case edges added.
    pub num_cases: usize,
}

/// Reconstruct switch tables from detected jump-table patterns.
///
/// For each `JumpTableInfo`, removes the existing `IndirectJump`
/// edge(s) from the block and replaces them with `SwitchCase` edges
/// to each resolved target.
///
/// The `address_to_block` function maps raw target addresses to
/// block IDs (the consumer must provide this because address-to-block
/// mapping is ISA/loader specific).
pub fn recover_switch_tables<I: FlowControl>(
    cfg: &mut Cfg<I>,
    tables: &[JumpTableInfo],
    address_to_block: impl Fn(u64) -> Option<BlockId>,
) -> Vec<SwitchRecovery> {
    let mut results = Vec::new();

    for table in tables {
        // Remove existing IndirectJump edges from this block.
        let edges_to_remove: Vec<_> = cfg
            .successor_edges(table.block)
            .iter()
            .filter(|&&eid| cfg.edge(eid).kind() == EdgeKind::IndirectJump)
            .copied()
            .collect();
        for eid in edges_to_remove {
            cfg.remove_edge(eid);
        }

        let mut num_cases = 0;

        // Add SwitchCase edges for each resolved target.
        for &target_addr in &table.targets {
            if let Some(target_block) = address_to_block(target_addr) {
                cfg.add_edge(table.block, target_block, EdgeKind::SwitchCase);
                num_cases += 1;
            }
        }

        // Add default target if present.
        if let Some(default_addr) = table.default_target
            && let Some(default_block) = address_to_block(default_addr)
        {
            cfg.add_edge(table.block, default_block, EdgeKind::Unconditional);
        }

        results.push(SwitchRecovery {
            block: table.block,
            num_cases,
        });
    }

    results
}
