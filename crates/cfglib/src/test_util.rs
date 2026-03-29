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

// ── Data-flow mock instruction ──────────────────────────────────────

use alloc::vec::Vec;
use crate::dataflow::{InstrInfo, Location};

/// A mock instruction that carries both control-flow classification
/// **and** data-flow information (defs/uses/side-effects).
///
/// Used by data-flow analysis tests across `reaching`, `liveness`,
/// and `defuse` modules.
#[derive(Debug, Clone)]
pub struct DfInst {
    pub effect: FlowEffect,
    pub name: &'static str,
    pub uses: Vec<Location>,
    pub defs: Vec<Location>,
}

impl FlowControl for DfInst {
    fn flow_effect(&self) -> FlowEffect {
        self.effect
    }
    fn display_mnemonic(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.name)
    }
}

impl InstrInfo for DfInst {
    fn uses(&self) -> &[Location] {
        &self.uses
    }
    fn defs(&self) -> &[Location] {
        &self.defs
    }
}

/// Create a [`DfInst`] that defines a single location.
pub fn df_def(name: &'static str, loc: u16) -> DfInst {
    DfInst {
        effect: FlowEffect::Fallthrough,
        name,
        uses: Vec::new(),
        defs: alloc::vec![Location(loc)],
    }
}

/// Create a [`DfInst`] that uses a single location.
pub fn df_use(name: &'static str, loc: u16) -> DfInst {
    DfInst {
        effect: FlowEffect::Fallthrough,
        name,
        uses: alloc::vec![Location(loc)],
        defs: Vec::new(),
    }
}

/// Create a plain [`DfInst`] with no defs or uses (fallthrough).
pub fn df_ff(name: &'static str) -> DfInst {
    DfInst {
        effect: FlowEffect::Fallthrough,
        name,
        uses: Vec::new(),
        defs: Vec::new(),
    }
}

/// Override the flow effect of a [`DfInst`].
pub fn df_with_effect(mut inst: DfInst, effect: FlowEffect) -> DfInst {
    inst.effect = effect;
    inst
}
