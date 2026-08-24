extern crate alloc;

use alloc::borrow::Cow;

use crate::display::DisplayInstr;
use crate::flow::{FlowControl, FlowEffect};

/// A minimal mock instruction carrying only flow-effect and mnemonic.
#[derive(Debug, Clone)]
pub struct MockInst(pub FlowEffect, pub &'static str);

impl FlowControl for MockInst {
    fn flow_effect(&self) -> FlowEffect {
        self.0
    }
}

impl DisplayInstr for MockInst {
    fn mnemonic(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.1)
    }
}

/// Shorthand for a [`MockInst`] with [`FlowEffect::Fallthrough`].
pub fn ff(name: &'static str) -> MockInst {
    MockInst(FlowEffect::Fallthrough, name)
}
