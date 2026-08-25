//! Minimal mock instruction for graph and transform tests.

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

/// Build a diamond CFG: entry → A, entry → B, A → merge, B → merge.
pub fn diamond_cfg() -> crate::cfg::Cfg<MockInst> {
    use crate::edge::EdgeKind;

    let mut cfg = crate::cfg::Cfg::new();
    let a = cfg.new_block();
    let b = cfg.new_block();
    let merge = cfg.new_block();
    cfg.block_mut(cfg.entry()).push(ff("entry"));
    cfg.block_mut(a).push(ff("a"));
    cfg.block_mut(b).push(ff("b"));
    cfg.block_mut(merge).push(ff("merge"));
    cfg.add_edge(cfg.entry(), a, EdgeKind::ConditionalTrue);
    cfg.add_edge(cfg.entry(), b, EdgeKind::ConditionalFalse);
    cfg.add_edge(a, merge, EdgeKind::Fallthrough);
    cfg.add_edge(b, merge, EdgeKind::Fallthrough);
    cfg
}
