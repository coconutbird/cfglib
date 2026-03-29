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
    ConditionalOpen,
    /// Alternate branch within a conditional region (`else`).
    ConditionalAlternate,
    /// Closes a conditional region (`endif`).
    ConditionalClose,
    /// Opens a switch region. `break` inside a switch exits the switch
    /// (unlike `break` in a loop which exits the loop).
    SwitchOpen,
    /// A case/default arm inside a switch.
    SwitchCase,
    /// Closes a switch region (`endswitch`).
    SwitchClose,
    /// Opens a loop region.
    LoopOpen,
    /// Closes a loop region.
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
}
