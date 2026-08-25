//! Shared test helpers for cfglib.
//!
//! Provides mock instruction types used across all test modules:
//!
//! - [`MockInst`] — minimal, flow-effect only (for graph/transform tests).
//! - [`DfInst`] — full-featured: flow + defs/uses + effects + optional
//!   copy semantics, expression decomposition, and constant values.
//!
//! `DfInst` implements **all** instruction traits (`FlowControl`,
//! `DisplayInstr`, `InstrInfo`, `EffectInfo`, `Predicated`, `CopySource`,
//! `ExprInstr`, `ConstantFolder`, `CallInfo`) so test modules don't need
//! to define their own instruction types.

mod dataflow_inst;
mod mock_inst;

pub(crate) use dataflow_inst::{
    DfInst, TestEffect, df_call, df_const, df_copy, df_def, df_ff, df_impure, df_op, df_pred,
    df_pure, df_use, df_with_effect,
};
pub(crate) use mock_inst::{MockInst, diamond_cfg, ff};
