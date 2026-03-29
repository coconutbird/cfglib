//! Edges connecting basic blocks in a control-flow graph.

extern crate alloc;

use crate::block::BlockId;

/// Opaque identifier for an edge within a [`Cfg`](crate::Cfg).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EdgeId(pub(crate) u32);

impl EdgeId {
    /// Returns the raw index.
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl core::fmt::Display for EdgeId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "e{}", self.0)
    }
}

/// The kind of a control-flow edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum EdgeKind {
    /// Sequential fallthrough to the next block.
    Fallthrough,
    /// Taken branch of a conditional (the "true" path).
    ConditionalTrue,
    /// Not-taken branch of a conditional (the "false" / merge path).
    ConditionalFalse,
    /// Unconditional jump (structured break/switch exit).
    Unconditional,
    /// Back-edge to a loop header.
    Back,
    /// Edge to a call target.
    Call,
    /// Return edge from a call site.
    CallReturn,
    /// Edge for a switch/case arm.
    SwitchCase,

    // ── Unstructured / CPU-ISA edges ──────────────────────────────
    /// Direct jump (goto / `jmp` / `b`).
    Jump,
    /// Computed / indirect jump (`jmp [rax]`, jump table).
    IndirectJump,
    /// Indirect call (`call [vtable]`).
    IndirectCall,
    /// Edge into an exception-handler entry block.
    ExceptionHandler,
    /// Edge from a potentially-throwing instruction to a handler.
    ExceptionUnwind,
    /// Edge from a protected region to the normal continuation.
    ExceptionLeave,
}

impl core::fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let label = match self {
            EdgeKind::Fallthrough => "fallthrough",
            EdgeKind::ConditionalTrue => "true",
            EdgeKind::ConditionalFalse => "false",
            EdgeKind::Unconditional => "unconditional",
            EdgeKind::Back => "back",
            EdgeKind::Call => "call",
            EdgeKind::CallReturn => "call_return",
            EdgeKind::SwitchCase => "case",
            EdgeKind::Jump => "jump",
            EdgeKind::IndirectJump => "indirect_jump",
            EdgeKind::IndirectCall => "indirect_call",
            EdgeKind::ExceptionHandler => "handler",
            EdgeKind::ExceptionUnwind => "unwind",
            EdgeKind::ExceptionLeave => "leave",
        };
        f.write_str(label)
    }
}

/// A directed edge between two basic blocks.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Edge {
    /// Edge identity.
    pub(crate) id: EdgeId,
    /// Source block.
    pub(crate) source: BlockId,
    /// Target block.
    pub(crate) target: BlockId,
    /// Classification.
    pub(crate) kind: EdgeKind,
    /// Optional branch weight / probability (0.0–1.0).
    ///
    /// When set, this indicates the likelihood of this edge being taken
    /// relative to other outgoing edges of the same source block.
    /// Used by the linearizer for hot-path layout and by DOT output
    /// for visual emphasis.
    pub(crate) weight: Option<f64>,
    /// Optional call-site metadata for `Call` / `IndirectCall` edges.
    pub(crate) call_site: Option<CallSite>,
}

impl PartialEq for Edge {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.source == other.source
            && self.target == other.target
            && self.kind == other.kind
            && self.weight.map(f64::to_bits) == other.weight.map(f64::to_bits)
            && self.call_site == other.call_site
    }
}

impl Eq for Edge {}

/// Metadata attached to a call edge describing the call target.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CallSite {
    /// Symbolic name or address of the call target (e.g. function name).
    pub target_name: Option<alloc::string::String>,
    /// Raw target address, if known.
    pub target_address: Option<u64>,
    /// Calling convention hint (ISA-specific string, e.g. "cdecl", "aapcs").
    pub calling_convention: Option<alloc::string::String>,
    /// Whether this is a tail call (no return edge expected).
    pub is_tail_call: bool,
}

impl CallSite {
    /// Create a call site with just a target name.
    pub fn named(name: &str) -> Self {
        Self {
            target_name: Some(alloc::string::String::from(name)),
            target_address: None,
            calling_convention: None,
            is_tail_call: false,
        }
    }

    /// Create a call site with a raw address.
    pub fn at_address(addr: u64) -> Self {
        Self {
            target_name: None,
            target_address: Some(addr),
            calling_convention: None,
            is_tail_call: false,
        }
    }
}

impl Edge {
    /// The edge's unique identifier.
    #[inline]
    pub fn id(&self) -> EdgeId {
        self.id
    }

    /// The source block of this edge.
    #[inline]
    pub fn source(&self) -> BlockId {
        self.source
    }

    /// The target block of this edge.
    #[inline]
    pub fn target(&self) -> BlockId {
        self.target
    }

    /// The classification of this edge.
    #[inline]
    pub fn kind(&self) -> EdgeKind {
        self.kind
    }

    /// The branch weight / probability, if set.
    #[inline]
    pub fn weight(&self) -> Option<f64> {
        self.weight
    }

    /// Set the branch weight / probability.
    #[inline]
    pub fn set_weight(&mut self, w: Option<f64>) {
        self.weight = w;
    }

    /// The call-site metadata, if this is a call edge.
    #[inline]
    pub fn call_site(&self) -> Option<&CallSite> {
        self.call_site.as_ref()
    }

    /// Set call-site metadata.
    #[inline]
    pub fn set_call_site(&mut self, cs: Option<CallSite>) {
        self.call_site = cs;
    }
}
