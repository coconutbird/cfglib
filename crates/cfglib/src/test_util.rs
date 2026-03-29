//! Shared test helpers for cfglib.
//!
//! Provides a minimal [`MockInst`] type that implements [`FlowControl`]
//! for use in unit tests across all modules.

extern crate alloc;
use alloc::borrow::Cow;

use crate::flow::{FlowControl, FlowEffect};

/// A minimal mock instruction carrying only flow-effect and mnemonic.
#[derive(Debug, Clone)]
pub struct MockInst(pub FlowEffect, pub &'static str);

impl FlowControl for MockInst {
    fn flow_effect(&self) -> FlowEffect {
        self.0
    }
    fn display_mnemonic(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.1)
    }
}

/// Shorthand for a [`MockInst`] with [`FlowEffect::Fallthrough`].
pub fn ff(name: &'static str) -> MockInst {
    MockInst(FlowEffect::Fallthrough, name)
}
