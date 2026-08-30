//! Stable identities for MLIL entities.

use crate::identity::define_dense_id;
use crate::{BlockId, EdgeId};

define_dense_id! {
    /// Stable identity of one mutable MLIL variable before SSA renaming.
    pub struct VariableId(u32);
    display = "v";
    raw;
}

define_dense_id! {
    /// Stable identity of one MLIL instruction within a function.
    pub struct InstructionId(u32);
    display = "i";
    raw;
}

/// Stable identity of an MLIL entity that can originate from source code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntityId {
    /// Control-flow block.
    Block(BlockId),
    /// Stable control-flow edge.
    Edge(EdgeId),
    /// Semantic instruction.
    Instruction(InstructionId),
    /// Mutable pre-SSA variable.
    Variable(VariableId),
}
