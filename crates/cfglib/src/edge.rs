//! Edges connecting basic blocks in a control-flow graph.

use crate::block::BlockId;

pub use crate::graph::directed::EdgeId;

/// The kind of a control-flow edge.
///
/// The vocabulary is universal, not machine-specific: every variant has both
/// a source-language and a machine reading (e.g. [`Jump`](Self::Jump) is a
/// `goto` or a `jmp`).
///
/// # Which kinds algorithms interpret
///
/// Dominance, loop detection, SCC, and traversals are purely structural —
/// they never read kinds, so consumers may choose kinds freely for their own
/// purposes. The kind-sensitive surfaces are: AST lifting (block
/// classification via [`Back`](Self::Back),
/// [`ConditionalTrue`](Self::ConditionalTrue) /
/// [`ConditionalFalse`](Self::ConditionalFalse),
/// [`SwitchCase`](Self::SwitchCase), [`Jump`](Self::Jump)), the `_tagged`
/// loop detectors ([`Back`](Self::Back)), linearization (fallthrough-like
/// kinds vs branches), switch recovery
/// ([`IndirectJump`](Self::IndirectJump) removal), the exception model
/// (the `Exception*` kinds), diffing (kind discriminants in fingerprints),
/// and DOT styling.
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

    /// Direct explicit jump: a source `goto`, a machine `jmp` / `b`.
    ///
    /// Distinct from [`Unconditional`](Self::Unconditional): `Jump` records
    /// an explicit branch instruction, `Unconditional` a synthesized
    /// structured transfer (break, switch exit).
    Jump,
    /// Computed / indirect jump: a source computed goto or lowered `match`
    /// dispatch, a machine `jmp [rax]` through a jump table.
    IndirectJump,
    /// Indirect call: source dynamic dispatch or a function-pointer call, a
    /// machine `call [vtable]`.
    IndirectCall,
    /// Edge into an exception-handler entry block.
    ExceptionHandler,
    /// Edge from a potentially-throwing instruction to a handler.
    ExceptionUnwind,
    /// Edge from a protected region to the normal continuation.
    ExceptionLeave,
    /// Edge from a resume/rethrow point to the next exception dispatcher.
    ExceptionResume,
    /// Edge that resumes execution after an exception was handled in-place.
    ///
    /// Windows SEH and VEH use this for `EXCEPTION_CONTINUE_EXECUTION`: the
    /// target is the exact block at which execution resumes.
    ExceptionContinue,
}

impl EdgeKind {
    /// Whether the edge transfers control exceptionally rather than
    /// sequentially.
    ///
    /// [`ExceptionLeave`](Self::ExceptionLeave) is a normal transfer out
    /// of a protected region, so it stays sequential.
    #[must_use]
    pub const fn is_exceptional(self) -> bool {
        matches!(
            self,
            EdgeKind::ExceptionHandler
                | EdgeKind::ExceptionUnwind
                | EdgeKind::ExceptionResume
                | EdgeKind::ExceptionContinue
        )
    }
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
            EdgeKind::ExceptionResume => "resume",
            EdgeKind::ExceptionContinue => "continue_exception",
        };
        f.write_str(label)
    }
}

/// A directed edge between two basic blocks.
///
/// `E` is consumer-owned metadata. The default unit payload preserves the
/// original `Cfg<I>` surface, while frontends that need switch labels, handler
/// identities, continuation tokens, or source provenance use `Cfg<I, E>`.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Edge<E = ()> {
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
    /// Consumer-defined metadata.
    pub(crate) payload: E,
}

impl<E: PartialEq> PartialEq for Edge<E> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.source == other.source
            && self.target == other.target
            && self.kind == other.kind
            && self.weight.map(f64::to_bits) == other.weight.map(f64::to_bits)
            && self.payload == other.payload
    }
}

impl<E: Eq> Eq for Edge<E> {}

impl<E> Edge<E> {
    /// The edge's unique identifier.
    #[inline]
    #[must_use]
    pub fn id(&self) -> EdgeId {
        self.id
    }

    /// The source block of this edge.
    #[inline]
    #[must_use]
    pub fn source(&self) -> BlockId {
        self.source
    }

    /// The target block of this edge.
    #[inline]
    #[must_use]
    pub fn target(&self) -> BlockId {
        self.target
    }

    /// The classification of this edge.
    #[inline]
    #[must_use]
    pub fn kind(&self) -> EdgeKind {
        self.kind
    }

    /// The branch weight / probability, if set.
    #[inline]
    #[must_use]
    pub fn weight(&self) -> Option<f64> {
        self.weight
    }

    /// Set the branch weight / probability.
    #[inline]
    pub fn set_weight(&mut self, w: Option<f64>) {
        self.weight = w;
    }

    /// The consumer-defined edge metadata.
    #[inline]
    #[must_use]
    pub const fn payload(&self) -> &E {
        &self.payload
    }

    /// Mutable access to the consumer-defined edge metadata.
    #[inline]
    pub const fn payload_mut(&mut self) -> &mut E {
        &mut self.payload
    }

    /// Consume the edge and return its consumer-defined metadata.
    #[inline]
    #[must_use]
    pub fn into_payload(self) -> E {
        self.payload
    }
}
