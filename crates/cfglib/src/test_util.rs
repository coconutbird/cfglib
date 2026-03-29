//! Shared test helpers for cfglib.
//!
//! Provides mock instruction types used across all test modules:
//!
//! - [`MockInst`] — minimal, flow-effect only (for graph/transform tests).
//! - [`DfInst`] — full-featured: flow + defs/uses + effects + optional
//!   copy semantics, expression decomposition, and constant values.
//!
//! `DfInst` implements **all** instruction traits (`FlowControl`,
//! `InstrInfo`, `CopySource`, `ExprInstr`) so test modules don't need
//! to define their own instruction types.

extern crate alloc;
use alloc::borrow::Cow;
use alloc::vec::Vec;

use crate::analysis::expr::ExprInstr;
use crate::analysis::purity::Effect;
use crate::dataflow::copyprop::CopySource;
use crate::dataflow::{InstrInfo, Location};
use crate::flow::{FlowControl, FlowEffect};

// ── MockInst (flow-only) ────────────────────────────────────────────

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

// ── DfInst (full-featured mock) ─────────────────────────────────────

/// A mock instruction that carries control-flow, data-flow, and
/// optional higher-level semantics (copy, expression, constant).
///
/// Used across all analysis and transform test modules.
#[derive(Debug, Clone)]
pub struct DfInst {
    /// Control-flow classification.
    pub effect: FlowEffect,
    /// Mnemonic label.
    pub name: &'static str,
    /// Locations read by this instruction.
    pub uses: Vec<Location>,
    /// Locations written by this instruction.
    pub defs: Vec<Location>,
    /// Side effects (memory, I/O, etc.).
    pub side_effects: Vec<Effect>,
    /// If `true`, this instruction is a simple copy (`defs[0] := uses[0]`).
    pub is_copy: bool,
    /// Expression operator name (e.g. `"add"`, `"mul"`). `None` for
    /// instructions that can't be decomposed into expressions.
    pub op: Option<&'static str>,
    /// If set, this instruction loads a constant value.
    pub constant: Option<i64>,
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
    fn effects(&self) -> &[Effect] {
        &self.side_effects
    }
}

impl CopySource for DfInst {
    fn as_copy(&self) -> Option<(Location, Location)> {
        if self.is_copy && self.defs.len() == 1 && self.uses.len() == 1 {
            Some((self.defs[0], self.uses[0]))
        } else {
            None
        }
    }
    fn rewrite_use(&mut self, old: Location, new: Location) {
        for u in &mut self.uses {
            if *u == old {
                *u = new;
            }
        }
    }
}

impl ExprInstr for DfInst {
    fn as_expr(&self) -> Option<(&str, &[Location])> {
        self.op.map(|op| (op, self.uses.as_slice()))
    }
    fn as_const(&self) -> Option<i64> {
        self.constant
    }
}

// ── DfInst constructors ─────────────────────────────────────────────

/// Default fields for a `DfInst` (no copy, no expr, no constant).
fn df_base(name: &'static str) -> DfInst {
    DfInst {
        effect: FlowEffect::Fallthrough,
        name,
        uses: Vec::new(),
        defs: Vec::new(),
        side_effects: Vec::new(),
        is_copy: false,
        op: None,
        constant: None,
    }
}

/// Create a [`DfInst`] that defines a single location.
pub fn df_def(name: &'static str, loc: u16) -> DfInst {
    DfInst {
        defs: alloc::vec![Location(loc)],
        ..df_base(name)
    }
}

/// Create a [`DfInst`] that uses a single location.
pub fn df_use(name: &'static str, loc: u16) -> DfInst {
    DfInst {
        uses: alloc::vec![Location(loc)],
        ..df_base(name)
    }
}

/// Create a plain [`DfInst`] with no defs, uses, or side effects.
pub fn df_ff(name: &'static str) -> DfInst {
    df_base(name)
}

/// Override the flow effect of a [`DfInst`].
pub fn df_with_effect(mut inst: DfInst, effect: FlowEffect) -> DfInst {
    inst.effect = effect;
    inst
}

/// Create a pure [`DfInst`] (no side effects, no defs/uses).
pub fn df_pure(name: &'static str) -> DfInst {
    df_base(name)
}

/// Create an impure [`DfInst`] with a single side effect.
pub fn df_impure(name: &'static str, e: Effect) -> DfInst {
    DfInst {
        side_effects: alloc::vec![e],
        ..df_base(name)
    }
}

/// Create a copy instruction (`dst := src`).
pub fn df_copy(name: &'static str, dst: u16, src: u16) -> DfInst {
    DfInst {
        defs: alloc::vec![Location(dst)],
        uses: alloc::vec![Location(src)],
        is_copy: true,
        ..df_base(name)
    }
}

/// Create an expression instruction (`dst = op(srcs...)`).
pub fn df_op(name: &'static str, op: &'static str, dst: u16, srcs: &[u16]) -> DfInst {
    DfInst {
        defs: alloc::vec![Location(dst)],
        uses: srcs.iter().map(|&s| Location(s)).collect(),
        op: Some(op),
        ..df_base(name)
    }
}

/// Create a constant-load instruction (`dst = constant`).
pub fn df_const(name: &'static str, dst: u16, val: i64) -> DfInst {
    DfInst {
        defs: alloc::vec![Location(dst)],
        constant: Some(val),
        ..df_base(name)
    }
}
