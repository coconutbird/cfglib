//! Structural placement of presentation-level variable declarations.

extern crate alloc;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec;
use alloc::vec::Vec;

use super::statement::{ChildBodyKind, child_body_entries, expression_references};
use super::{Dialect, ExpressionId, ExpressionKind, Function, StatementId, VariableId};

/// Lexically narrow declaration anchors for HLIL variables.
///
/// Each referenced variable is anchored before the first statement in the
/// narrowest lexical body containing all of its occurrences. A `for`
/// initializer or update is part of the containing loop syntax rather than a
/// declaration body, so variables referenced there anchor before the `for`.
///
/// This analysis is deliberately structural. A source renderer remains
/// responsible for excluding variables declared by a signature or handler,
/// and for hoisting live-in variables or satisfying language-specific
/// declaration restrictions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariablePlacements {
    placements: Vec<Option<StatementId>>,
    before: Vec<Vec<VariableId>>,
}

impl VariablePlacements {
    /// Computes declaration anchors for every referenced variable.
    #[must_use]
    pub fn compute<D: Dialect>(function: &Function<D>) -> Self {
        let mut analyzer = Analyzer {
            function,
            placements: vec![None; function.variables().len()],
        };
        analyzer.analyze_body(function.body());

        let placements = analyzer.placements;
        let mut before = vec![Vec::new(); function.statements().len()];
        for (variable, placement) in function.variables().iter().zip(&placements) {
            if let Some(statement) = placement
                && let Some(variables) = before.get_mut(statement.index())
            {
                variables.push(variable.id);
            }
        }
        Self { placements, before }
    }

    /// Returns the statement immediately preceded by `variable`'s
    /// declaration, or `None` when the variable has no occurrence.
    #[must_use]
    pub fn placement(&self, variable: VariableId) -> Option<StatementId> {
        self.placements.get(variable.index()).copied().flatten()
    }

    /// Returns variables whose declarations anchor immediately before
    /// `statement`, in dense variable-identity order.
    #[must_use]
    pub fn before(&self, statement: StatementId) -> &[VariableId] {
        self.before
            .get(statement.index())
            .map_or(&[], Vec::as_slice)
    }
}

struct Analyzer<'a, D: Dialect> {
    function: &'a Function<D>,
    placements: Vec<Option<StatementId>>,
}

impl<D: Dialect> Analyzer<'_, D> {
    fn analyze_body(&mut self, body: &[StatementId]) -> BTreeSet<VariableId> {
        let mut contained = BTreeSet::new();
        let mut first_statement = BTreeMap::new();
        for &statement in body {
            for variable in self.analyze_statement(statement) {
                if let Some(&first) = first_statement.get(&variable) {
                    self.place(variable, first);
                } else {
                    first_statement.insert(variable, statement);
                }
                contained.insert(variable);
            }
        }
        contained
    }

    fn analyze_statement(&mut self, statement: StatementId) -> BTreeSet<VariableId> {
        let Some(node) = self.function.statement(statement) else {
            return BTreeSet::new();
        };

        let mut owned = BTreeSet::new();
        expression_references(node.kind(), &mut |expression| {
            self.collect_expression(expression, &mut owned);
        });

        let mut components = BTreeMap::<VariableId, usize>::new();
        for &variable in &owned {
            components.insert(variable, 1);
        }
        for child in child_body_entries(node.kind()) {
            let variables = match child.kind {
                ChildBodyKind::Lexical => self.analyze_body(child.statements),
                ChildBodyKind::Clause => {
                    let variables = self.collect_body(child.statements);
                    owned.extend(variables.iter().copied());
                    variables
                }
            };
            for variable in variables {
                *components.entry(variable).or_default() += 1;
            }
        }

        for (&variable, &component_count) in &components {
            if owned.contains(&variable) || component_count > 1 {
                self.place(variable, statement);
            }
        }
        components.into_keys().collect()
    }

    fn collect_body(&self, body: &[StatementId]) -> BTreeSet<VariableId> {
        let mut variables = BTreeSet::new();
        for &statement in body {
            self.collect_statement(statement, &mut variables);
        }
        variables
    }

    fn collect_statement(&self, statement: StatementId, variables: &mut BTreeSet<VariableId>) {
        let Some(node) = self.function.statement(statement) else {
            return;
        };
        expression_references(node.kind(), &mut |expression| {
            self.collect_expression(expression, variables);
        });
        for child in child_body_entries(node.kind()) {
            for &nested in child.statements {
                self.collect_statement(nested, variables);
            }
        }
    }

    fn collect_expression(&self, expression: ExpressionId, variables: &mut BTreeSet<VariableId>) {
        let Some(node) = self.function.expression(expression) else {
            return;
        };
        match node.kind() {
            ExpressionKind::Variable(variable) => {
                variables.insert(*variable);
            }
            ExpressionKind::Constant(_) => {}
            ExpressionKind::Operation { operands, .. } => {
                for &operand in operands {
                    self.collect_expression(operand, variables);
                }
            }
        }
    }

    fn place(&mut self, variable: VariableId, statement: StatementId) {
        if let Some(placement) = self.placements.get_mut(variable.index()) {
            *placement = Some(statement);
        }
    }
}

#[cfg(test)]
mod tests;
