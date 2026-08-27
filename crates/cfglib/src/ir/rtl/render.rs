//! Consumer-side helpers for rendering lifted functions.
//!
//! [`lift`](super::lift()) establishes conventions its consumers must
//! honor: reads align positionally with HLIL operand lists, a merging
//! assignment carries its target as the trailing use, and webs are
//! declared in dense variable order. The helpers here consume those
//! conventions in one place — [`ReadResolver`] pairs each
//! [`VarExpr::Read`] with its operand (following inlined producers and
//! remapping positions), [`Webs`] resolves variables to their webs, and
//! the [`LiftedStatement`] methods answer the MLIL and HLIL dialect
//! hooks whose shape the lift fixed.

extern crate alloc;

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::FlowEffect;
use crate::ir::hlil::{self, ExpressionId, ExpressionKind, Function as HlilFunction, Lifted};
use crate::ir::mlil::VariableId;

use super::dialect::Dialect;
use super::error::{Error, Result};
use super::template::{LiftedStatement, VarExpr, WebInfo};

impl<D: Dialect> VarExpr<D> {
    /// The number of [`Read`](VarExpr::Read) nodes in pre-order — the
    /// number of HLIL operands the expression consumes.
    #[must_use]
    pub fn read_count(&self) -> usize {
        let mut count = 0usize;
        self.for_each_expression(&mut |expression| {
            if matches!(expression, Self::Read { .. }) {
                count += 1;
            }
        });
        count
    }

    /// Visits this expression and every descendant in pre-order.
    pub fn for_each_expression(&self, visit: &mut impl FnMut(&Self)) {
        visit(self);
        match self {
            Self::Read { .. } | Self::Const { .. } => {}
            Self::Apply { operands, .. } => {
                for operand in operands {
                    operand.for_each_expression(visit);
                }
            }
            Self::Reinterpret { operand, .. } => operand.for_each_expression(visit),
            Self::Compose { parts, .. } => {
                for part in parts {
                    part.for_each_expression(visit);
                }
            }
        }
    }

    /// A compact stable mnemonic for the expression's producing form.
    #[must_use]
    pub fn mnemonic(&self) -> &str {
        match self {
            Self::Read { .. } => "mov",
            Self::Const { .. } => "const",
            Self::Apply { operator, .. } => D::mnemonic(operator),
            Self::Reinterpret { .. } => "bitcast",
            Self::Compose { .. } => "compose",
        }
    }
}

impl<D: Dialect> LiftedStatement<D> {
    /// The operand position holding the target's previous value, when
    /// unwritten positions merge.
    ///
    /// The lift appends a merging assignment's target as its trailing
    /// use, after every value read. A dialect's
    /// [`previous_value_operand`](hlil::LiftDialect::previous_value_operand)
    /// delegates here so the HLIL lift neither inlines a producer into
    /// the merge slot nor inlines the merge-def forward.
    #[must_use]
    pub fn merge_operand(&self) -> Option<usize> {
        match self {
            Self::Assign {
                merges: true,
                value,
                ..
            } => Some(value.read_count()),
            _ => None,
        }
    }

    /// A compact stable mnemonic for the statement.
    #[must_use]
    pub fn mnemonic(&self) -> &str {
        match self {
            Self::Assign { value, .. } => value.mnemonic(),
            Self::Effect { operation, .. } | Self::Raise { operation, .. } => {
                D::effect_mnemonic(operation)
            }
            Self::Branch { .. } => "branch",
            Self::Dispatch { .. } => "switch",
            Self::Return { .. } => "ret",
        }
    }

    /// The control-flow classification of the statement, for a dialect's
    /// [`instruction_metadata`](crate::ir::mlil::Dialect::instruction_metadata).
    #[must_use]
    pub fn flow_effect(&self) -> FlowEffect {
        match self {
            Self::Assign { .. } | Self::Effect { .. } => FlowEffect::Fallthrough,
            Self::Branch { .. } => FlowEffect::ConditionalJump,
            Self::Dispatch { .. } => FlowEffect::IndirectJump,
            Self::Return { .. } => FlowEffect::Return,
            Self::Raise { .. } => FlowEffect::Terminate,
        }
    }

    /// The HLIL translation of the statement, for a dialect's
    /// [`lift_operation`](hlil::LiftDialect::lift_operation): branches
    /// carry their condition inside the operation, dispatches lift to
    /// the structural switch, returns lift to the structural return, and
    /// everything else — raises included — stays an operation.
    #[must_use]
    pub fn lifted(&self) -> Lifted<Self> {
        match self {
            Self::Branch { .. } => Lifted::BranchOperation(self.clone()),
            Self::Dispatch { .. } => Lifted::Switch,
            Self::Return { .. } => Lifted::Return,
            other => Lifted::Operation(other.clone()),
        }
    }
}

/// The webs recovered by one lift, resolvable by variable identity.
///
/// Resolution keys on the variable's raw identity, and the HLIL lift
/// preserves MLIL variable identities before appending any temporaries —
/// so both levels' variables resolve here. A dialect temporary declared
/// during emission, or an HLIL temporary, has no web.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Webs<D: Dialect> {
    webs: Vec<WebInfo<D>>,
    by_variable: alloc::collections::BTreeMap<u32, usize>,
}

impl<D: Dialect> Webs<D> {
    pub(super) fn new(webs: Vec<WebInfo<D>>) -> Self {
        let by_variable = webs
            .iter()
            .enumerate()
            .map(|(index, web)| (web.variable.raw(), index))
            .collect();
        Self { webs, by_variable }
    }

    /// The web behind one MLIL variable.
    #[must_use]
    pub fn of(&self, variable: VariableId) -> Option<&WebInfo<D>> {
        self.by_variable
            .get(&variable.raw())
            .map(|&index| &self.webs[index])
    }

    /// The web behind one lifted (HLIL) variable.
    #[must_use]
    pub fn of_lifted(&self, variable: hlil::VariableId) -> Option<&WebInfo<D>> {
        self.by_variable
            .get(&variable.raw())
            .map(|&index| &self.webs[index])
    }

    /// The webs in declared variable order.
    pub fn iter(&self) -> core::slice::Iter<'_, WebInfo<D>> {
        self.webs.iter()
    }

    /// The number of webs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.webs.len()
    }

    /// Whether the lift recovered no webs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.webs.is_empty()
    }
}

impl<'a, D: Dialect> IntoIterator for &'a Webs<D> {
    type Item = &'a WebInfo<D>;
    type IntoIter = core::slice::Iter<'a, WebInfo<D>>;

    fn into_iter(self) -> Self::IntoIter {
        self.webs.iter()
    }
}

/// One resolved [`VarExpr::Read`]: what its HLIL operand turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedRead<'a, D>
where
    D: Dialect + hlil::Dialect<Operation = LiftedStatement<D>>,
{
    /// The read is an occurrence of a declared variable; the read's
    /// positions select within that variable's web.
    Variable(hlil::VariableId),
    /// The read consumed a single-use producer the HLIL lift inlined.
    Inlined {
        /// The producer's assigned value.
        value: &'a VarExpr<D>,
        /// The producer's own operand list, for a nested resolver.
        operands: &'a [ExpressionId],
        /// How the read's positions select from the producer's value
        /// lanes: `None` when the read consumes the written positions in
        /// order, otherwise the value-lane index per read position.
        remap: Option<Vec<u8>>,
    },
    /// The read consumed a producer the HLIL lift folded to a dialect
    /// constant via
    /// [`AnalysisDialect::constant`](crate::ir::mlil::AnalysisDialect::constant).
    Constant(&'a <D as hlil::Dialect>::Constant),
}

/// Pairs the [`VarExpr::Read`] nodes of one lifted operation with the
/// HLIL operand list they align with.
///
/// The lift guarantees reads and operands correspond one-to-one in
/// pre-order; each [`resolve`](Self::resolve) consumes the next operand.
/// The resolver is cheap to clone: probe a speculative resolution on a
/// clone and commit by assignment when it matches.
#[derive(Debug)]
pub struct ReadResolver<'a, D>
where
    D: Dialect + hlil::Dialect<Operation = LiftedStatement<D>>,
{
    function: &'a HlilFunction<D>,
    operands: &'a [ExpressionId],
    cursor: usize,
}

impl<D> Clone for ReadResolver<'_, D>
where
    D: Dialect + hlil::Dialect<Operation = LiftedStatement<D>>,
{
    fn clone(&self) -> Self {
        Self {
            function: self.function,
            operands: self.operands,
            cursor: self.cursor,
        }
    }
}

impl<'a, D> ReadResolver<'a, D>
where
    D: Dialect + hlil::Dialect<Operation = LiftedStatement<D>>,
{
    /// Creates a resolver over one operation's operand list.
    #[must_use]
    pub const fn new(function: &'a HlilFunction<D>, operands: &'a [ExpressionId]) -> Self {
        Self {
            function,
            operands,
            cursor: 0,
        }
    }

    /// A nested resolver over an inlined producer's operand list.
    #[must_use]
    pub const fn nested(&self, operands: &'a [ExpressionId]) -> Self {
        Self {
            function: self.function,
            operands,
            cursor: 0,
        }
    }

    /// Resolves the next read, consuming one operand. `positions` are
    /// the read's positions, used to compute the inlined-producer remap.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Resolution`] when the operand list is exhausted,
    /// the operand has no producer form a read can consume, or a read
    /// position falls outside an inlined producer's written positions —
    /// each a broken read/operand alignment.
    pub fn resolve(&mut self, positions: &[u8]) -> Result<ResolvedRead<'a, D>> {
        let id = *self
            .operands
            .get(self.cursor)
            .ok_or_else(|| Error::Resolution("read without an operand".into()))?;
        self.cursor += 1;
        let expression = self
            .function
            .expression(id)
            .ok_or_else(|| Error::Resolution("read of an unresolvable operand".into()))?;
        match expression.kind() {
            ExpressionKind::Variable(variable) => Ok(ResolvedRead::Variable(*variable)),
            ExpressionKind::Constant(constant) => Ok(ResolvedRead::Constant(constant)),
            ExpressionKind::Operation {
                operation:
                    LiftedStatement::Assign {
                        positions: written,
                        value,
                        ..
                    },
                operands,
            } => {
                let remap = if positions == written.as_slice() {
                    None
                } else {
                    let mapped = positions
                        .iter()
                        .map(|position| {
                            written
                                .iter()
                                .position(|candidate| candidate == position)
                                .and_then(|index| u8::try_from(index).ok())
                                .ok_or_else(|| {
                                    Error::Resolution(
                                        "read position outside the inlined producer".into(),
                                    )
                                })
                        })
                        .collect::<Result<Vec<u8>>>()?;
                    Some(mapped)
                };
                Ok(ResolvedRead::Inlined {
                    value,
                    operands,
                    remap,
                })
            }
            ExpressionKind::Operation { .. } => {
                Err(Error::Resolution("read of a non-assign operation".into()))
            }
        }
    }
}

/// The variables a structured function still references: every variable
/// occurrence, including statement-level assignment targets (which are
/// variable expressions themselves).
///
/// An inlined producer's target is deliberately absent — its definition
/// was absorbed into its consumer, no occurrence of the variable exists,
/// and it never renders.
#[must_use]
pub fn referenced_webs<D>(function: &HlilFunction<D>) -> BTreeSet<VariableId>
where
    D: Dialect + hlil::Dialect<Operation = LiftedStatement<D>>,
{
    let mut referenced = BTreeSet::new();
    for expression in function.expressions() {
        if let ExpressionKind::Variable(variable) = expression.kind() {
            referenced.insert(VariableId::from_raw(variable.raw()));
        }
    }
    referenced
}
