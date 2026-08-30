//! Verified generic HLIL function storage.

extern crate alloc;

use alloc::vec::Vec;

use super::{
    Dialect, Expression, ExpressionId, ExpressionKind, ProvenanceMap, Signature, Statement,
    StatementId, Variable, VariableId, VerificationReport, VerifyDialect,
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

    /// Visits `root`'s expression tree in postorder: operands before parents.
    ///
    /// Missing operand identities are skipped. Verified functions store each
    /// expression tree with operands preceding parents, so postorder here is
    /// also a valid evaluation order.
    pub fn visit_expression_tree<'f>(
        &'f self,
        root: ExpressionId,
        visit: &mut impl FnMut(&'f Expression<D>),
    ) {
        let Some(expression) = self.expression(root) else {
            return;
        };
        if let ExpressionKind::Operation { operands, .. } = expression.kind() {
            for &operand in operands {
                self.visit_expression_tree(operand, visit);
            }
        }
        visit(expression);
    }

    /// Whether two expression trees are structurally identical.
    ///
    /// The same variable reads, equal constants, and equal operations over
    /// pairwise structurally identical operands. A missing identity never
    /// compares equal. Verified functions store operands before parents, so
    /// the recursion is bounded.
    #[must_use]
    pub fn expressions_equal(&self, left: ExpressionId, right: ExpressionId) -> bool {
        let (Some(left), Some(right)) = (self.expression(left), self.expression(right)) else {
            return false;
        };
        match (left.kind(), right.kind()) {
            (ExpressionKind::Variable(left), ExpressionKind::Variable(right)) => left == right,
            (ExpressionKind::Constant(left), ExpressionKind::Constant(right)) => left == right,
            (
                ExpressionKind::Operation {
                    operation: left_operation,
                    operands: left_operands,
                },
                ExpressionKind::Operation {
                    operation: right_operation,
                    operands: right_operands,
                },
            ) => {
                left_operation == right_operation
                    && left_operands.len() == right_operands.len()
                    && left_operands
                        .iter()
                        .zip(right_operands)
                        .all(|(&left, &right)| self.expressions_equal(left, right))
            }
            _ => false,
        }
    }

    /// The compound form of one assignment, when it has one.
    ///
    /// Recognizes `target = operation(target, operand)` structurally: an
    /// [`Assign`](super::StatementKind::Assign) whose value is a binary
    /// operation whose **first** operand is structurally identical to the
    /// assignment target. Renderers spell the result `target op= operand`
    /// with the dialect's compound spelling; whether a commutative
    /// operation with the target second also qualifies stays the caller's
    /// judgment. Deciding by structure — never by comparing rendered text —
    /// keeps the rewrite immune to parenthesization and casts.
    #[must_use]
    pub fn compound_assignment(
        &self,
        statement: StatementId,
    ) -> Option<(&D::Operation, ExpressionId)> {
        let statement = self.statement(statement)?;
        let super::StatementKind::Assign { target, value } = statement.kind() else {
            return None;
        };
        let value = self.expression(*value)?;
        let ExpressionKind::Operation {
            operation,
            operands,
        } = value.kind()
        else {
            return None;
        };
        let [first, operand] = operands.as_slice() else {
            return None;
        };
        self.expressions_equal(*first, *target)
            .then_some((operation, *operand))
    }
}
