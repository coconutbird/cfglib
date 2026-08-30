//! Display-only instruction rendering.
//!
//! [`DisplayInstr`] is deliberately independent of
//! [`FlowControl`](crate::FlowControl): rendering a graph for humans (DOT
//! export, pseudocode) must not require implementing flow classification.
//! Consumers that only want a picture implement this one method.

extern crate alloc;
use alloc::borrow::Cow;
use alloc::string::String;

/// Appends `depth` levels of four-space indentation.
pub(crate) fn write_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("    ");
    }
}

/// Opt-in, display-only rendering of instructions.
///
/// Used by [`Cfg::write_dot`](crate::Cfg::write_dot) and AST pseudocode
/// output. Return a short human-readable label: a machine mnemonic, a source
/// statement excerpt, or a node kind. Labels are escaped by the renderers, so
/// raw program text is safe to return.
pub trait DisplayInstr {
    /// Short label for this instruction (mnemonic, statement text, node kind).
    fn mnemonic(&self) -> Cow<'_, str>;
}

impl<T: DisplayInstr + ?Sized> DisplayInstr for &T {
    fn mnemonic(&self) -> Cow<'_, str> {
        T::mnemonic(self)
    }
}
