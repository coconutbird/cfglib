//! CFG structural verification.
//!
//! Validates invariants that should always hold for a well-formed CFG:
//! entry block exists, adjacency lists are consistent, no out-of-bounds
//! IDs, and every non-entry reachable block has at least one predecessor.

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

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
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// Number of violations found.
    #[must_use]
    pub fn error_count(&self) -> usize {
        self.errors.len()
    }
}

fn verify_edge_endpoints<I>(cfg: &Cfg<I>, block_count: usize, errors: &mut Vec<VerifyError>) {
    for edge in cfg.edges() {
        if edge.source().index() >= block_count {
            errors.push(VerifyError {
                message: alloc::format!(
                    "edge {} source {} out of bounds (num_blocks={})",
                    edge.id(),
                    edge.source(),
                    block_count
                ),
            });
        }
        if edge.target().index() >= block_count {
            errors.push(VerifyError {
                message: alloc::format!(
                    "edge {} target {} out of bounds (num_blocks={})",
                    edge.id(),
                    edge.target(),
                    block_count
                ),
            });
        }
    }
}

fn verify_adjacency<I>(cfg: &Cfg<I>, errors: &mut Vec<VerifyError>) {
    for block in cfg.blocks() {
        let block_id = block.id();
        for &edge_id in cfg.successor_edges(block_id) {
            if edge_id.index() >= cfg.edge_slots() {
                errors.push(VerifyError {
                    message: alloc::format!(
                        "block {block_id} successor edge {edge_id} out of bounds"
                    ),
                });
            } else if cfg.edge(edge_id).source() != block_id {
                errors.push(VerifyError {
                    message: alloc::format!(
                        "block {} lists edge {} as successor but edge source is {}",
                        block_id,
                        edge_id,
                        cfg.edge(edge_id).source()
                    ),
                });
            }
        }
        for &edge_id in cfg.predecessor_edges(block_id) {
            if edge_id.index() >= cfg.edge_slots() {
                errors.push(VerifyError {
                    message: alloc::format!(
                        "block {block_id} predecessor edge {edge_id} out of bounds"
                    ),
                });
            } else if cfg.edge(edge_id).target() != block_id {
                errors.push(VerifyError {
                    message: alloc::format!(
                        "block {} lists edge {} as predecessor but edge target is {}",
                        block_id,
                        edge_id,
                        cfg.edge(edge_id).target()
                    ),
                });
            }
        }
    }
}

fn verify_unique_adjacency<I>(cfg: &Cfg<I>, errors: &mut Vec<VerifyError>) {
    for block in cfg.blocks() {
        let block_id = block.id();
        let mut seen = alloc::collections::BTreeSet::new();
        for &edge_id in cfg.successor_edges(block_id) {
            if !seen.insert(edge_id) {
                errors.push(VerifyError {
                    message: alloc::format!(
                        "block {block_id} has duplicate successor edge {edge_id}"
                    ),
                });
            }
        }
        seen.clear();
        for &edge_id in cfg.predecessor_edges(block_id) {
            if !seen.insert(edge_id) {
                errors.push(VerifyError {
                    message: alloc::format!(
                        "block {block_id} has duplicate predecessor edge {edge_id}"
                    ),
                });
            }
        }
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
///
/// # Examples
///
/// ```
/// use cfglib::{Cfg, EdgeKind, verify};
///
/// let mut cfg = Cfg::<u32>::new();
/// let b1 = cfg.new_block();
/// cfg.add_edge(cfg.entry(), b1, EdgeKind::Fallthrough);
///
/// let result = verify(&cfg);
/// assert!(result.is_ok());
/// ```
#[must_use]
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

    verify_edge_endpoints(cfg, n, &mut errors);
    verify_adjacency(cfg, &mut errors);

    // 4. Every reachable non-entry block has a predecessor.
    let reachable = cfg.dfs_preorder();
    for &bid in &reachable {
        if bid == cfg.entry() {
            continue;
        }
        if cfg.predecessor_edges(bid).is_empty() {
            errors.push(VerifyError {
                message: alloc::format!("reachable block {bid} has no predecessors"),
            });
        }
    }

    verify_unique_adjacency(cfg, &mut errors);

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
