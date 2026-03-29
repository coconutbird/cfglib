//! ISA-agnostic control-flow classification.
//!
//! Any instruction set that wants to build a [`Cfg`](crate::Cfg) must
//! implement [`FlowControl`] for its instruction type.

extern crate alloc;
use alloc::borrow::Cow;

/// Classification of an instruction's effect on control flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlowEffect {
    /// Normal instruction — execution falls through to the next.
    Fallthrough,
    /// Opens a conditional region (`if`).
    ///
    /// The builder **stores** this instruction in the current block.
    ConditionalOpen,
    /// Alternate branch within a conditional region (`else`).
    ///
    /// The builder **stores** this instruction in the new else block.
    ConditionalAlternate,
    /// Closes a conditional region (`endif`).
    ///
    /// The builder does **not** store this instruction in any block;
    /// it only creates the merge block and wires up edges.
    ConditionalClose,
    /// Opens a switch region. `break` inside a switch exits the switch
    /// (unlike `break` in a loop which exits the loop).
    ///
    /// The builder **stores** this instruction in the current block.
    SwitchOpen,
    /// A case/default arm inside a switch.
    ///
    /// The builder **stores** this instruction in the new case block.
    SwitchCase,
    /// Closes a switch region (`endswitch`).
    ///
    /// The builder does **not** store this instruction in any block;
    /// it only creates the merge block and wires up edges.
    SwitchClose,
    /// Opens a loop region.
    ///
    /// The builder **stores** this instruction in the current block.
    LoopOpen,
    /// Closes a loop region.
    ///
    /// The builder does **not** store this instruction in any block;
    /// it only creates the back-edge and post-loop block.
    LoopClose,
    /// Unconditional break out of the innermost loop/switch.
    Break,
    /// Conditional break.
    ConditionalBreak,
    /// Unconditional continue to loop header.
    Continue,
    /// Conditional continue.
    ConditionalContinue,
    /// Unconditional return / function terminator.
    Return,
    /// Conditional return.
    ConditionalReturn,
    /// Unconditional call to a label/address.
    Call,
    /// Conditional call.
    ConditionalCall,
    /// Terminates execution of the current invocation (e.g. `discard`).
    Terminate,
    /// A label/target that can be jumped to.
    Label,
    /// Declaration or metadata — skipped by the builder.
    Declaration,

    // ── Unstructured / CPU-ISA effects ────────────────────────────
    /// Unconditional jump to a named or address-based target.
    ///
    /// The target is obtained via [`FlowControl::jump_target`].
    Jump,
    /// Conditional jump — falls through on false, jumps on true.
    ///
    /// The target is obtained via [`FlowControl::jump_target`].
    ConditionalJump,
    /// Computed / indirect jump (target unknown at decode time).
    IndirectJump,
    /// Indirect call (target resolved at runtime).
    IndirectCall,
    /// Instruction that may throw or trap (e.g. `div`, `int 3`).
    MayThrow,
}

/// Trait that an ISA's instruction type must implement so the CFG builder
/// can classify each instruction's effect on control flow.
pub trait FlowControl {
    /// Classify this instruction's control-flow effect.
    fn flow_effect(&self) -> FlowEffect;

    /// Optional short label for display in DOT output (e.g. the mnemonic).
    fn display_mnemonic(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    /// For [`FlowEffect::Jump`] and [`FlowEffect::ConditionalJump`],
    /// returns the target label or address as a string.
    ///
    /// The default returns `None`. ISA frontends should override this
    /// for jump instructions so the builder can wire up the target
    /// edge.
    fn jump_target(&self) -> Option<Cow<'_, str>> {
        None
    }
}
