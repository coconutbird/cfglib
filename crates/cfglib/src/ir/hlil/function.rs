//! Verified generic HLIL function storage.

extern crate alloc;

use alloc::vec::Vec;

use super::{
    Dialect, Expression, ExpressionId, ProvenanceMap, Signature, Statement, StatementId, Variable,
    VariableId, VerificationReport, VerifyDialect,
};

/// One structured semantic function.
///
/// Statements and expressions live in dense arenas and form strict trees:
/// the statement tree is rooted at [`Self::body`], and every expression is
/// referenced exactly once. Construction goes through
/// [`FunctionBuilder`](super::FunctionBuilder) (source lowering) or
/// [`lift_function`](super::lift_function) (MLIL lifting), both of which
/// verify these invariants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function<D: Dialect> {
    pub(super) variables: Vec<Variable<D>>,
    pub(super) signature: Signature<D>,
    pub(super) expressions: Vec<Expression<D>>,
    pub(super) statements: Vec<Statement<D>>,
    pub(super) body: Vec<StatementId>,
    pub(super) provenance: ProvenanceMap<D>,
}

impl<D: Dialect> Function<D> {
    /// Returns the source function identity and coordinate system.
    #[must_use]
    pub fn source(&self) -> &D::Source {
        self.provenance.source()
    }

    /// Returns the ordered parameter and return signature.
    #[must_use]
    pub const fn signature(&self) -> &Signature<D> {
        &self.signature
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

    /// Returns expressions in dense identity order.
    #[must_use]
    pub fn expressions(&self) -> &[Expression<D>] {
        &self.expressions
    }

    /// Looks up one expression.
    #[must_use]
    pub fn expression(&self, id: ExpressionId) -> Option<&Expression<D>> {
        self.expressions.get(id.index())
    }

    /// Returns statements in dense identity order.
    #[must_use]
    pub fn statements(&self) -> &[Statement<D>] {
        &self.statements
    }

    /// Looks up one statement.
    #[must_use]
    pub fn statement(&self, id: StatementId) -> Option<&Statement<D>> {
        self.statements.get(id.index())
    }

    /// Returns the top-level statement sequence.
    #[must_use]
    pub fn body(&self) -> &[StatementId] {
        &self.body
    }

    /// Returns stable source-to-HLIL provenance.
    #[must_use]
    pub const fn provenance(&self) -> &ProvenanceMap<D> {
        &self.provenance
    }
}

impl<D: VerifyDialect> Function<D> {
    /// Verifies structural, tree, label, and dialect invariants.
    #[must_use]
    pub fn verify(&self) -> VerificationReport {
        super::verify::verify_function(self)
    }
}

impl<D: Dialect> Function<D> {
    /// Visits every statement under `roots` in preorder.
    pub fn visit_statements<'f>(
        &'f self,
        roots: &[StatementId],
        visit: &mut impl FnMut(&'f Statement<D>),
    ) {
        for &root in roots {
            if let Some(statement) = self.statement(root) {
                visit(statement);
                for body in super::statement::child_bodies(statement.kind()) {
                    self.visit_statements(body, visit);
                }
            }
        }
    }
}
