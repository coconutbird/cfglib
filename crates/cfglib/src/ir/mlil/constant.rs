//! Verified materialization of constants proven by sparse propagation.

extern crate alloc;

use alloc::vec::Vec;

use crate::{ConstValue, FlowControl, FlowEffect, SccpAnalysis};

use super::{
    AnalysisDialect, Function, Instruction, InstructionReplacement, Result, TypedVariable,
    VerifyDialect,
};

/// Dialect hook for spelling a proven constant as one operand-free operation.
pub trait ConstantMaterializationDialect: AnalysisDialect {
    /// Returns an operation that assigns `constant` to the instruction's sole
    /// definition without reading any runtime operand.
    ///
    /// The pass invokes this only for pure, non-throwing, single-definition
    /// fallthrough instructions whose SSA result SCCP proved constant. Return
    /// `None` when the original operation should remain as written.
    fn materialize_constant(
        instruction: &Instruction<Self>,
        constant: &Self::Constant,
    ) -> Option<Self::Operation>;
}

/// One function after proven constants have been materialized.
#[derive(Debug, Clone)]
pub struct ConstantMaterialization<D: AnalysisDialect> {
    /// The identity-preserving rebuilt function.
    pub function: Function<D>,
    /// Number of instructions replaced by constant materializations.
    pub rewritten: usize,
}

impl<D> Function<D>
where
    D: ConstantMaterializationDialect + VerifyDialect,
{
    /// Replaces eligible value computations with constants proven by SCCP.
    ///
    /// The canonical function is not mutated. Every graph and entity identity
    /// remains stable in the returned function, and the rebuilt result is
    /// fully verified.
    ///
    /// # Errors
    ///
    /// Returns a verification error when the source or rebuilt function is
    /// invalid.
    pub fn materialize_constants(&self) -> Result<ConstantMaterialization<D>> {
        let ssa = self.ssa()?;
        let constants = SccpAnalysis::compute(self.cfg(), &ssa);
        let rewritten = self.rewrite_instructions(|instruction| {
            if !instruction.effects().is_empty()
                || instruction.may_throw()
                || instruction.flow_effect() != FlowEffect::Fallthrough
                || instruction.defs().len() != 1
            {
                return None;
            }
            let point = self.instruction_point(instruction.id())?;
            let annotation = ssa.instruction(point)?;
            let [definition] = annotation.defs.as_slice() else {
                return None;
            };
            let constant = constants
                .values
                .get(definition)
                .and_then(ConstValue::as_const)?;
            let operation = D::materialize_constant(instruction, constant)?;
            let definition =
                TypedVariable::new(instruction.defs()[0], instruction.def_types()[0].clone());
            Some(InstructionReplacement::new(
                operation,
                Vec::new(),
                alloc::vec![definition],
                false,
            ))
        })?;
        Ok(ConstantMaterialization {
            function: rewritten.function,
            rewritten: rewritten.rewritten,
        })
    }
}
