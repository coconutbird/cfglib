//! Memory SSA — extends SSA to memory operations.
//!
//! Introduces `MemoryDef`, `MemoryUse`, and `MemoryPhi` nodes that
//! track memory versioning through the CFG, enabling alias-aware
//! optimizations.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::cfg::Cfg;
use crate::dataflow::{InstrInfo, ProgramPoint};
use crate::graph::dominator::DominatorTree;

/// A memory version identifier.
pub type MemoryVersion = u32;

/// A node in the Memory SSA graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryAccess {
    /// A memory definition (store or call with side effects).
    Def {
        /// The new memory version produced.
        version: MemoryVersion,
        /// The memory version this def clobbers.
        clobbers: MemoryVersion,
        /// Where in the program this def occurs.
        point: ProgramPoint,
    },
    /// A memory use (load).
    Use {
        /// The memory version being read.
        version: MemoryVersion,
        /// Where in the program this use occurs.
        point: ProgramPoint,
    },
    /// A memory phi at a join point.
    Phi {
        /// The new version produced by the phi.
        version: MemoryVersion,
        /// Operand versions from each predecessor.
        operands: Vec<(BlockId, MemoryVersion)>,
        /// Block where this phi lives.
        block: BlockId,
    },
}

/// Trait for instructions to declare memory side effects.
pub trait MemoryEffect: InstrInfo {
    /// Does this instruction read from memory?
    fn reads_memory(&self) -> bool;
    /// Does this instruction write to memory?
    fn writes_memory(&self) -> bool;
}

/// Result of Memory SSA construction.
#[derive(Debug, Clone)]
pub struct MemorySSA {
    /// All memory accesses in program order per block.
    pub accesses: BTreeMap<BlockId, Vec<MemoryAccess>>,
    /// Next available version number.
    pub num_versions: MemoryVersion,
}

/// Build Memory SSA for a CFG.
///
/// Assigns memory version numbers to all memory-accessing instructions
/// and inserts memory phis at join points where memory versions differ.
pub fn build_memory_ssa<I: MemoryEffect>(cfg: &Cfg<I>, dom: &DominatorTree) -> MemorySSA {
    let rpo = cfg.reverse_postorder();
    let mut next_ver: MemoryVersion = 1; // version 0 = initial (entry) memory
    let mut accesses: BTreeMap<BlockId, Vec<MemoryAccess>> = BTreeMap::new();
    let mut block_out_ver: BTreeMap<BlockId, MemoryVersion> = BTreeMap::new();

    // First pass: assign versions within each block.
    for &bid in &rpo {
        let mut cur_ver: MemoryVersion = if let Some(idom) = dom.idom(bid) {
            block_out_ver.get(&idom).copied().unwrap_or(0)
        } else {
            0 // entry block starts with version 0
        };

        // Check if we need a memory phi (multiple preds with different versions).
        let preds: Vec<BlockId> = cfg.predecessors(bid).collect();
        if preds.len() > 1 {
            let pred_vers: Vec<(BlockId, MemoryVersion)> = preds
                .iter()
                .map(|&p| (p, block_out_ver.get(&p).copied().unwrap_or(0)))
                .collect();
            let all_same = pred_vers.windows(2).all(|w| w[0].1 == w[1].1);
            if !all_same {
                let phi_ver = next_ver;
                next_ver += 1;
                accesses.entry(bid).or_default().push(MemoryAccess::Phi {
                    version: phi_ver,
                    operands: pred_vers,
                    block: bid,
                });
                cur_ver = phi_ver;
            }
        }

        let block_accesses = accesses.entry(bid).or_default();
        for (idx, inst) in cfg.block(bid).instructions().iter().enumerate() {
            let point = ProgramPoint {
                block: bid,
                inst_idx: idx,
            };
            if inst.writes_memory() {
                let new_ver = next_ver;
                next_ver += 1;
                block_accesses.push(MemoryAccess::Def {
                    version: new_ver,
                    clobbers: cur_ver,
                    point,
                });
                cur_ver = new_ver;
            } else if inst.reads_memory() {
                block_accesses.push(MemoryAccess::Use {
                    version: cur_ver,
                    point,
                });
            }
        }

        block_out_ver.insert(bid, cur_ver);
    }

    MemorySSA {
        accesses,
        num_versions: next_ver,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::Cfg;
    use crate::dataflow::Location;
    use crate::edge::EdgeKind;
    use crate::flow::{FlowControl, FlowEffect};
    use crate::graph::dominator::DominatorTree;
    use alloc::borrow::Cow;

    #[derive(Debug, Clone)]
    struct MemInst {
        reads: bool,
        writes: bool,
    }
    impl FlowControl for MemInst {
        fn flow_effect(&self) -> FlowEffect {
            FlowEffect::Fallthrough
        }
        fn display_mnemonic(&self) -> Cow<'_, str> {
            Cow::Borrowed("mem")
        }
    }
    impl InstrInfo for MemInst {
        fn uses(&self) -> &[Location] {
            &[]
        }
        fn defs(&self) -> &[Location] {
            &[]
        }
    }
    impl MemoryEffect for MemInst {
        fn reads_memory(&self) -> bool {
            self.reads
        }
        fn writes_memory(&self) -> bool {
            self.writes
        }
    }

    #[test]
    fn store_creates_memory_def() {
        let mut cfg: Cfg<MemInst> = Cfg::new();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(MemInst {
                reads: false,
                writes: true,
            });
        let dom = DominatorTree::compute(&cfg);
        let mssa = build_memory_ssa(&cfg, &dom);
        let accs = mssa.accesses.get(&cfg.entry()).unwrap();
        assert_eq!(accs.len(), 1);
        assert!(matches!(
            accs[0],
            MemoryAccess::Def {
                version: 1,
                clobbers: 0,
                ..
            }
        ));
    }

    #[test]
    fn load_creates_memory_use() {
        let mut cfg: Cfg<MemInst> = Cfg::new();
        cfg.block_mut(cfg.entry())
            .instructions_vec_mut()
            .push(MemInst {
                reads: true,
                writes: false,
            });
        let dom = DominatorTree::compute(&cfg);
        let mssa = build_memory_ssa(&cfg, &dom);
        let accs = mssa.accesses.get(&cfg.entry()).unwrap();
        assert_eq!(accs.len(), 1);
        assert!(matches!(accs[0], MemoryAccess::Use { version: 0, .. }));
    }

    #[test]
    fn diamond_inserts_memory_phi() {
        let mut cfg: Cfg<MemInst> = Cfg::new();
        let a = cfg.new_block();
        let b = cfg.new_block();
        let merge = cfg.new_block();
        cfg.block_mut(a).instructions_vec_mut().push(MemInst {
            reads: false,
            writes: true,
        });
        cfg.block_mut(b).instructions_vec_mut().push(MemInst {
            reads: false,
            writes: true,
        });
        cfg.add_edge(cfg.entry(), a, EdgeKind::ConditionalTrue);
        cfg.add_edge(cfg.entry(), b, EdgeKind::ConditionalFalse);
        cfg.add_edge(a, merge, EdgeKind::Fallthrough);
        cfg.add_edge(b, merge, EdgeKind::Fallthrough);
        let dom = DominatorTree::compute(&cfg);
        let mssa = build_memory_ssa(&cfg, &dom);
        let merge_accs = mssa.accesses.get(&merge).unwrap();
        assert!(
            merge_accs
                .iter()
                .any(|a| matches!(a, MemoryAccess::Phi { .. }))
        );
    }
}
