//! Verified generic MLIL function storage and derived analyses.

extern crate alloc;

use alloc::vec::Vec;

use crate::{
    AstNode, BlockExprTrees, Cfg, ConstFact, DefUseChains, DominatorTree, Facts, Liveness,
    ProgramPoint, SccpAnalysis, SsaForm,
};

use super::{
    AnalysisDialect, Dialect, Instruction, InstructionId, ProvenanceMap, Result, Signature,
    Variable, VariableId, VerificationReport, VerifyDialect,
};

/// One semantic function backed by a cfglib control-flow graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function<D: Dialect> {
    pub(super) cfg: Cfg<Instruction<D>, D::Edge>,
    pub(super) variables: Vec<Variable<D>>,
    pub(super) signature: Signature<D>,
    pub(super) provenance: ProvenanceMap<D>,
    pub(super) instruction_points: Vec<ProgramPoint>,
}

impl<D: Dialect> Function<D> {
    /// Returns the exact edge-bearing MLIL control-flow graph.
    #[must_use]
    pub const fn cfg(&self) -> &Cfg<Instruction<D>, D::Edge> {
        &self.cfg
    }

    /// Returns the ordered parameter and return signature.
    #[must_use]
    pub const fn signature(&self) -> &Signature<D> {
        &self.signature
    }

    /// Returns the source function identity and coordinate system.
    #[must_use]
    pub fn source(&self) -> &D::Source {
        self.provenance.source()
    }

    /// Returns variables in dense identity order.
    #[must_use]
    pub fn variables(&self) -> &[Variable<D>] {
        &self.variables
    }

    /// Looks up one declared variable.
    #[must_use]
    pub fn variable(&self, id: VariableId) -> Option<&Variable<D>> {
        self.variables.get(id.index())
    }

    /// Returns stable source-to-MLIL provenance.
    #[must_use]
    pub const fn provenance(&self) -> &ProvenanceMap<D> {
        &self.provenance
    }

    /// Returns instructions in stable identity order.
    ///
    /// Identity order is allocation order, which is independent of graph
    /// position; use [`Self::cfg`] for block-structured iteration.
    pub fn instructions(&self) -> impl Iterator<Item = &Instruction<D>> {
        self.instruction_points.iter().filter_map(|point| {
            self.cfg
                .blocks()
                .get(point.block.index())?
                .instructions()
                .get(point.inst_idx)
        })
    }

    /// Looks up one instruction by stable identity.
    #[must_use]
    pub fn instruction(&self, id: InstructionId) -> Option<&Instruction<D>> {
        let point = *self.instruction_points.get(id.index())?;
        self.cfg
            .blocks()
            .get(point.block.index())?
            .instructions()
            .get(point.inst_idx)
    }

    /// Returns the current graph location of one stable instruction.
    #[must_use]
    pub fn instruction_point(&self, id: InstructionId) -> Option<ProgramPoint> {
        self.instruction_points.get(id.index()).copied()
    }

    /// Computes a dominator tree over ordinary and exceptional control flow.
    #[must_use]
    pub fn dominators(&self) -> DominatorTree {
        DominatorTree::compute(&self.cfg)
    }

    /// Computes definition-to-use and use-to-definition chains.
    #[must_use]
    pub fn def_use(&self) -> DefUseChains {
        DefUseChains::compute(&self.cfg)
    }

    /// Computes block-entry and block-exit liveness.
    #[must_use]
    pub fn liveness(&self) -> Liveness<VariableId> {
        Liveness::compute(&self.cfg)
    }

    /// Recovers structured control flow, retaining explicit labels and gotos
    /// when the graph is irreducible.
    #[must_use]
    pub fn structured_control_flow(&self) -> AstNode<Instruction<D>> {
        crate::lift(&self.cfg)
    }

    /// Recovers structured control flow together with the exact list of
    /// constructs that degraded to unstructured flow.
    #[must_use]
    pub fn structured_control_flow_with_report(
        &self,
    ) -> (AstNode<Instruction<D>>, crate::LiftReport) {
        crate::lift_with_report(&self.cfg)
    }

    /// Reports effect-aware dead definitions and unreachable blocks.
    #[must_use]
    pub fn dead_code(&self) -> crate::DeadCode {
        crate::DeadCode::compute(&self.cfg)
    }

    /// Returns a graph with effect-free dead definitions removed.
    ///
    /// The canonical function and its stable provenance are not mutated;
    /// instruction positions in the returned view are derived data.
    #[must_use]
    pub fn dead_code_eliminated_cfg(&self) -> (Cfg<Instruction<D>, D::Edge>, usize) {
        let mut cfg = self.cfg.clone();
        let removed = crate::dead_code_elimination(&mut cfg);
        (cfg, removed)
    }
}

impl<D: VerifyDialect> Function<D> {
    /// Verifies structural, control-flow, typing, and dialect invariants.
    #[must_use]
    pub fn verify(&self) -> VerificationReport {
        super::verify::verify_function(self)
    }

    /// Computes a fully renamed SSA view while retaining the source graph.
    ///
    /// # Errors
    ///
    /// Returns a verification report if the stored function is invalid.
    pub fn ssa(&self) -> Result<SsaForm<VariableId>> {
        let report = self.verify();
        if !report.is_ok() {
            return Err(report.into());
        }
        let dominators = self.dominators();
        Ok(SsaForm::compute(&self.cfg, &dominators))
    }
}

impl<D: AnalysisDialect> Function<D> {
    /// Computes conservative forward constant facts without changing the function.
    #[must_use]
    pub fn constants(&self) -> Facts<ConstFact<VariableId, D::Constant>> {
        crate::constant_propagation(&self.cfg)
    }

    /// Recovers pure, single-use expression trees independently in each block.
    #[must_use]
    pub fn expressions(
        &self,
    ) -> Vec<BlockExprTrees<VariableId, D::ExpressionOperator, D::Constant>> {
        crate::recover_expressions(&self.cfg)
    }

    /// Returns a copy-propagated graph for presentation and downstream analysis.
    ///
    /// The canonical function and its identity-indexed provenance are not
    /// mutated. Removed copies retain no graph position in the returned view.
    #[must_use]
    pub fn copy_propagated_cfg(
        &self,
    ) -> (Cfg<Instruction<D>, D::Edge>, crate::CopyPropagationStats) {
        let mut cfg = self.cfg.clone();
        let statistics = crate::copy_propagation(&mut cfg);
        (cfg, statistics)
    }
}

impl<D: AnalysisDialect + VerifyDialect> Function<D> {
    /// Computes sparse constants over the derived SSA view.
    ///
    /// # Errors
    ///
    /// Returns a verification report if the stored function is invalid.
    pub fn sparse_constants(&self) -> Result<SccpAnalysis<VariableId, D::Constant>> {
        let ssa = self.ssa()?;
        Ok(SccpAnalysis::compute(&self.cfg, &ssa))
    }
}
