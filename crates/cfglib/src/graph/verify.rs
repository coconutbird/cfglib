//! CFG structural verification.
//!
//! Validates invariants that should always hold for a well-formed CFG:
//! entry block exists, adjacency lists are consistent, no out-of-bounds
//! IDs, and every non-entry reachable block has at least one predecessor.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;

/// A single verification failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyError {
    /// Human-readable description of the violated invariant.
    pub message: String,
}

impl core::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "CFG verify: {}", self.message)
    }
}

/// Result of running [`verify`] on a CFG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyResult {
    /// All detected violations (empty = valid).
    pub errors: Vec<VerifyError>,
}

impl VerifyResult {
    /// True when the CFG passes all invariant checks.
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// Number of violations found.
    pub fn error_count(&self) -> usize {
        self.errors.len()
    }
}

/// Validate structural invariants of a CFG.
///
/// Checks performed:
/// 1. Entry block index is within bounds.
/// 2. Every edge references valid source/target block IDs.
/// 3. Adjacency lists (`succs`/`preds`) are consistent with edges.
/// 4. Every reachable non-entry block has at least one predecessor.
/// 5. No duplicate edges in adjacency lists.
///
/// Returns a [`VerifyResult`] containing all violations found.
pub fn verify<I>(cfg: &Cfg<I>) -> VerifyResult {
    let mut errors = Vec::new();
    let n = cfg.num_blocks();

    // 1. Entry in bounds.
    if cfg.entry().index() >= n {
        errors.push(VerifyError {
            message: alloc::format!(
                "entry block {} out of bounds (num_blocks={})",
                cfg.entry(),
                n
            ),
        });
        // Can't do much more if entry is invalid.
        return VerifyResult { errors };
    }

    // 2. Edge endpoints in bounds.
    for edge in cfg.edges() {
        if edge.source().index() >= n {
            errors.push(VerifyError {
                message: alloc::format!(
                    "edge {} source {} out of bounds (num_blocks={})",
                    edge.id(),
                    edge.source(),
                    n
                ),
            });
        }
        if edge.target().index() >= n {
            errors.push(VerifyError {
                message: alloc::format!(
                    "edge {} target {} out of bounds (num_blocks={})",
                    edge.id(),
                    edge.target(),
                    n
                ),
            });
        }
    }

    // 3. Adjacency consistency: every edge in succs[source] should exist,
    //    and every edge in preds[target] should exist.
    for (block_idx, _block) in cfg.blocks().iter().enumerate() {
        let bid = BlockId::from_raw(block_idx as u32);
        for &eid in cfg.successor_edges(bid) {
            if eid.index() >= cfg.edge_slots() {
                errors.push(VerifyError {
                    message: alloc::format!("block {} successor edge {} out of bounds", bid, eid),
                });
            } else if cfg.edge(eid).source() != bid {
                errors.push(VerifyError {
                    message: alloc::format!(
                        "block {} lists edge {} as successor but edge source is {}",
                        bid,
                        eid,
                        cfg.edge(eid).source()
                    ),
                });
            }
        }
        for &eid in cfg.predecessor_edges(bid) {
            if eid.index() >= cfg.edge_slots() {
                errors.push(VerifyError {
                    message: alloc::format!("block {} predecessor edge {} out of bounds", bid, eid),
                });
            } else if cfg.edge(eid).target() != bid {
                errors.push(VerifyError {
                    message: alloc::format!(
                        "block {} lists edge {} as predecessor but edge target is {}",
                        bid,
                        eid,
                        cfg.edge(eid).target()
                    ),
                });
            }
        }
    }

    // 4. Every reachable non-entry block has a predecessor.
    let reachable = cfg.dfs_preorder();
    for &bid in &reachable {
        if bid == cfg.entry() {
            continue;
        }
        if cfg.predecessor_edges(bid).is_empty() {
            errors.push(VerifyError {
                message: alloc::format!("reachable block {} has no predecessors", bid),
            });
        }
    }

    // 5. No duplicate edge IDs in adjacency lists.
    for block_idx in 0..n {
        let bid = BlockId::from_raw(block_idx as u32);
        let succs = cfg.successor_edges(bid);
        let mut seen = alloc::collections::BTreeSet::new();
        for &eid in succs {
            if !seen.insert(eid) {
                errors.push(VerifyError {
                    message: alloc::format!("block {} has duplicate successor edge {}", bid, eid),
                });
            }
        }
        let preds = cfg.predecessor_edges(bid);
        seen.clear();
        for &eid in preds {
            if !seen.insert(eid) {
                errors.push(VerifyError {
                    message: alloc::format!("block {} has duplicate predecessor edge {}", bid, eid),
                });
            }
        }
    }

    VerifyResult { errors }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::EdgeKind;
    use crate::test_util::ff;

    #[test]
    fn valid_cfg_passes() {
        let mut cfg = Cfg::new();
        let b = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("a"));
        cfg.block_mut(b).instructions_vec_mut().push(ff("b"));
        cfg.add_edge(cfg.entry(), b, EdgeKind::Fallthrough);

        let result = verify(&cfg);
        assert!(result.is_ok(), "valid CFG should pass: {:?}", result.errors);
    }

    #[test]
    fn single_block_cfg_passes() {
        let mut cfg = Cfg::new();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("a"));
        assert!(verify(&cfg).is_ok());
    }

    #[test]
    fn diamond_cfg_passes() {
        let mut cfg = Cfg::new();
        let a = cfg.new_block();
        let b = cfg.new_block();
        let merge = cfg.new_block();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(ff("e"));
        cfg.add_edge(cfg.entry(), a, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), b, EdgeKind::ConditionalFalse);
        cfg.add_edge(a, merge, EdgeKind::Fallthrough);
        cfg.add_edge(b, merge, EdgeKind::Fallthrough);

        assert!(verify(&cfg).is_ok());
    }

    #[test]
    fn verify_error_count() {
        let cfg: Cfg<crate::test_util::MockInst> = Cfg::new();
        let result = verify(&cfg);
        assert_eq!(result.error_count(), 0);
    }
}
