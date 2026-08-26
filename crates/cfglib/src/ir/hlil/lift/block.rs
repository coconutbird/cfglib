//! Flat-instruction-list translation with effect-ordered inlining.
//!
//! A definition inlines into its consumer only when doing so provably
//! preserves evaluation order: the definition has exactly one local use
//! (and is dead beyond it), pure computations may cross anything that does
//! not redefine their reads, and effectful or throwing computations may
//! move only into the immediately following instruction. Everything else
//! materializes as an assignment at its original position.

extern crate alloc;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::ir::mlil;

use super::super::{
    Dialect, EntityId, Error, ExpressionId, ExpressionKind, Result, StatementId, StatementKind,
    VariableId, VerifyDialect,
};
use super::{LiftDialect, Lifted, Lifter};

/// What a translated instruction list is allowed to end with.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Expect {
    /// Ordinary statements; a branch or dispatch terminator is an error.
    Statements,
    /// The list must end in a conditional branch.
    Branch,
    /// The list must end in a switch dispatch.
    Switch,
}

/// The branch or dispatch value a condition list ends with.
pub(super) struct ListEnd {
    /// The condition or scrutinee expression.
    pub value: ExpressionId,
    /// The terminator instruction, mapped by the caller to the structural
    /// statement it becomes.
    pub instruction: mlil::InstructionId,
}

/// The emission shape of one MLIL instruction.
enum Shape<D: LiftDialect> {
    Constant(<D as Dialect>::Constant),
    Copy,
    Operation(<D as Dialect>::Operation),
    Store(<D as Dialect>::Operation),
    Branch,
    Dispatch,
    Return,
    ControlFlow,
}

fn classify<D: LiftDialect>(instruction: &mlil::Instruction<D>) -> Shape<D> {
    let operation = instruction.operation();
    if let Some(constant) = <D as mlil::AnalysisDialect>::constant(operation) {
        return Shape::Constant(constant);
    }
    if <D as mlil::AnalysisDialect>::is_copy(operation) {
        return Shape::Copy;
    }
    match D::lift_operation(operation) {
        Lifted::Operation(operation) => Shape::Operation(operation),
        Lifted::Store { location } => Shape::Store(location),
        Lifted::Branch => Shape::Branch,
        Lifted::Switch => Shape::Dispatch,
        Lifted::Return => Shape::Return,
        Lifted::ControlFlow => Shape::ControlFlow,
    }
}

/// Whether emission builds an operand expression for use position `k`.
///
/// Branch and dispatch terminators evaluate only their first use (the
/// condition or scrutinee); their remaining uses never become expressions,
/// so definitions feeding them must stay materialized.
fn use_is_consumed<D: LiftDialect>(shape: &Shape<D>, position: usize) -> bool {
    match shape {
        Shape::Copy | Shape::Operation(_) | Shape::Store(_) | Shape::Return => true,
        Shape::Branch | Shape::Dispatch => position == 0,
        Shape::Constant(_) | Shape::ControlFlow => false,
    }
}

/// Whether the instruction can inline into a consumer as a value.
fn is_value_shape<D: LiftDialect>(shape: &Shape<D>) -> bool {
    matches!(
        shape,
        Shape::Constant(_) | Shape::Copy | Shape::Operation(_)
    )
}

struct CandidateFacts {
    /// Every variable the candidate's expression tree reads.
    reads: BTreeSet<mlil::VariableId>,
    /// Whether the tree contains effects or may throw.
    impure: bool,
}

/// Decide, per definition, the consumer it inlines into (if any).
fn plan_inlining<D: LiftDialect>(
    instructions: &[mlil::Instruction<D>],
    shapes: &[Shape<D>],
    live_out: &BTreeSet<mlil::VariableId>,
) -> Vec<Option<usize>> {
    let length = instructions.len();
    // Exact local single-use positions: one use between the definition and
    // the variable's next redefinition, and dead beyond the list unless
    // redefined inside it.
    let mut viable: Vec<Option<usize>> = vec![None; length];
    for (position, instruction) in instructions.iter().enumerate() {
        if !is_value_shape(&shapes[position]) || instruction.defs().len() != 1 {
            continue;
        }
        let variable = instruction.defs()[0];
        let mut occurrences = 0usize;
        let mut use_position = None;
        let mut redefined = false;
        for (later, candidate_use) in instructions.iter().enumerate().skip(position + 1) {
            let here = candidate_use
                .uses()
                .iter()
                .filter(|&&used| used == variable)
                .count();
            if here > 0 {
                occurrences += here;
                use_position.get_or_insert(later);
            }
            if candidate_use.defs().contains(&variable) {
                redefined = true;
                break;
            }
        }
        if occurrences == 1 && (redefined || !live_out.contains(&variable)) {
            viable[position] = use_position;
        }
    }

    // Order-safety walk over the movable candidates.
    let mut inline_at: Vec<Option<usize>> = vec![None; length];
    let mut facts: Vec<Option<CandidateFacts>> = (0..length).map(|_| None).collect();
    let mut active: BTreeMap<mlil::VariableId, usize> = BTreeMap::new();
    for (position, instruction) in instructions.iter().enumerate() {
        // Impure candidates survive only into the immediately next
        // instruction.
        active.retain(|_, &mut candidate| {
            facts[candidate].as_ref().is_some_and(|f| !f.impure) || candidate + 1 == position
        });
        let mut consumed: Vec<usize> = Vec::new();
        for (operand, &variable) in instruction.uses().iter().enumerate() {
            if !use_is_consumed(&shapes[position], operand) {
                continue;
            }
            if let Some(&candidate) = active.get(&variable) {
                if viable[candidate] == Some(position) {
                    inline_at[candidate] = Some(position);
                    consumed.push(candidate);
                    active.remove(&variable);
                }
            }
        }
        let impure_here = !instruction.effects().is_empty() || instruction.may_throw();
        if impure_here {
            // Nothing impure moves across another impure instruction.
            active.retain(|_, &mut candidate| facts[candidate].as_ref().is_some_and(|f| !f.impure));
        }
        for &defined in instruction.defs() {
            active.remove(&defined);
            // A candidate reading the redefined variable can no longer move
            // past this point.
            active.retain(|_, &mut candidate| {
                facts[candidate]
                    .as_ref()
                    .is_some_and(|f| !f.reads.contains(&defined))
            });
        }
        if viable[position].is_some() {
            let mut reads = BTreeSet::new();
            let mut impure = impure_here;
            for &variable in instruction.uses() {
                let inlined = consumed
                    .iter()
                    .copied()
                    .find(|&candidate| instructions[candidate].defs()[0] == variable);
                if let Some(candidate) = inlined {
                    if let Some(f) = facts[candidate].as_ref() {
                        reads.extend(f.reads.iter().copied());
                        impure |= f.impure;
                    }
                } else {
                    reads.insert(variable);
                }
            }
            facts[position] = Some(CandidateFacts { reads, impure });
            active.insert(instruction.defs()[0], position);
        }
    }
    inline_at
}

/// The mutable per-list emission state shared by the shape emitters.
struct ListState {
    /// Per definition, the consumer position it inlines into.
    inline_at: Vec<Option<usize>>,
    /// Consumer position → (consumed variable → its inlined definition).
    by_consumer: BTreeMap<usize, BTreeMap<mlil::VariableId, usize>>,
    /// Expressions parked for their inlined consumer.
    built: Vec<Option<ExpressionId>>,
    /// Statements emitted so far, in program order.
    statements: Vec<StatementId>,
}

impl<D: LiftDialect + VerifyDialect> Lifter<'_, D> {
    /// Translates one block-shaped instruction list into statements, with
    /// its terminator value when the caller expects one.
    pub(super) fn translate_list(
        &mut self,
        block: BlockId,
        instructions: &[mlil::Instruction<D>],
        expect: Expect,
    ) -> Result<(Vec<StatementId>, Option<ListEnd>)> {
        let shapes: Vec<Shape<D>> = instructions.iter().map(classify).collect();
        let inline_at = plan_inlining(instructions, &shapes, self.liveness.live_out(block));
        let mut by_consumer: BTreeMap<usize, BTreeMap<mlil::VariableId, usize>> = BTreeMap::new();
        for (candidate, consumer) in inline_at.iter().enumerate() {
            if let Some(consumer) = consumer {
                by_consumer
                    .entry(*consumer)
                    .or_default()
                    .insert(instructions[candidate].defs()[0], candidate);
            }
        }
        let mut state = ListState {
            inline_at,
            by_consumer,
            built: vec![None; instructions.len()],
            statements: Vec::new(),
        };

        let mut end = None;
        let mut finished = false;
        for (position, instruction) in instructions.iter().enumerate() {
            if finished || end.is_some() {
                return Err(Error::UnsupportedLift(format!(
                    "instruction {} follows its block's terminator",
                    instruction.id()
                )));
            }
            match &shapes[position] {
                Shape::ControlFlow => {}
                Shape::Constant(constant) => {
                    if instruction.defs().is_empty() {
                        continue;
                    }
                    require_single_definition(instruction)?;
                    let value_type = instruction.def_types()[0].clone();
                    let expression = self
                        .builder
                        .add_expression(ExpressionKind::Constant(constant.clone()), value_type)?;
                    self.finish_value(position, instruction, expression, &mut state)?;
                }
                Shape::Copy => {
                    require_single_definition(instruction)?;
                    if instruction.uses().len() != 1 {
                        return Err(Error::UnsupportedLift(format!(
                            "copy {} does not have exactly one use",
                            instruction.id()
                        )));
                    }
                    let value = self.operand(position, 0, instruction, &mut state)?;
                    self.finish_value(position, instruction, value, &mut state)?;
                }
                Shape::Operation(operation) => {
                    self.emit_operation(position, instruction, operation.clone(), &mut state)?;
                }
                Shape::Store(location) => {
                    self.emit_store(position, instruction, location.clone(), &mut state)?;
                }
                Shape::Branch | Shape::Dispatch => {
                    let expected = matches!(
                        (&shapes[position], expect),
                        (Shape::Branch, Expect::Branch) | (Shape::Dispatch, Expect::Switch)
                    );
                    if !expected {
                        return Err(Error::UnsupportedLift(format!(
                            "unexpected branch or dispatch instruction {}",
                            instruction.id()
                        )));
                    }
                    if instruction.uses().is_empty() {
                        return Err(Error::UnsupportedLift(format!(
                            "branch {} has no condition operand",
                            instruction.id()
                        )));
                    }
                    let value = self.operand(position, 0, instruction, &mut state)?;
                    end = Some(ListEnd {
                        value,
                        instruction: instruction.id(),
                    });
                }
                Shape::Return => {
                    let values = self.operands(position, instruction, &mut state)?;
                    let statement = self
                        .builder
                        .add_statement(StatementKind::Return { values }, None)?;
                    self.map_instruction(instruction.id(), EntityId::Statement(statement))?;
                    state.statements.push(statement);
                    finished = true;
                }
            }
        }
        if end.is_none() && !matches!(expect, Expect::Statements) {
            return Err(Error::UnsupportedLift(format!(
                "block {block} does not end in its expected branch"
            )));
        }
        Ok((state.statements, end))
    }

    /// Emits one dialect operation: a value (inlined or assigned) with one
    /// definition, an effect statement with none.
    fn emit_operation(
        &mut self,
        position: usize,
        instruction: &mlil::Instruction<D>,
        operation: <D as Dialect>::Operation,
        state: &mut ListState,
    ) -> Result<()> {
        let operands = self.operands(position, instruction, state)?;
        match instruction.defs().len() {
            0 => {
                let expression = self.builder.add_expression(
                    ExpressionKind::Operation {
                        operation,
                        operands,
                    },
                    D::void_type(),
                )?;
                let statement = self
                    .builder
                    .add_statement(StatementKind::Expression(expression), None)?;
                self.map_instruction(instruction.id(), EntityId::Statement(statement))?;
                state.statements.push(statement);
                Ok(())
            }
            1 => {
                let value_type = instruction.def_types()[0].clone();
                let expression = self.builder.add_expression(
                    ExpressionKind::Operation {
                        operation,
                        operands,
                    },
                    value_type,
                )?;
                self.finish_value(position, instruction, expression, state)
            }
            _ => Err(Error::UnsupportedLift(format!(
                "instruction {} defines more than one result",
                instruction.id()
            ))),
        }
    }

    /// Emits one store: the last use assigned into the place formed by the
    /// location operation over the preceding uses.
    fn emit_store(
        &mut self,
        position: usize,
        instruction: &mlil::Instruction<D>,
        location: <D as Dialect>::Operation,
        state: &mut ListState,
    ) -> Result<()> {
        let id = instruction.id();
        if !instruction.defs().is_empty() {
            return Err(Error::UnsupportedLift(format!(
                "store {id} also defines variables"
            )));
        }
        if instruction.uses().is_empty() {
            return Err(Error::UnsupportedLift(format!(
                "store {id} has no stored value"
            )));
        }
        let mut operands = self.operands(position, instruction, state)?;
        let value = operands.pop().expect("a store carries its value last");
        let value_type = self
            .builder
            .expression(value)
            .map_or_else(D::void_type, |expression| expression.value_type().clone());
        let target = self.builder.add_expression(
            ExpressionKind::Operation {
                operation: location,
                operands,
            },
            value_type,
        )?;
        let statement = self
            .builder
            .add_statement(StatementKind::Assign { target, value }, None)?;
        self.map_instruction(id, EntityId::Statement(statement))?;
        state.statements.push(statement);
        Ok(())
    }

    /// The expression for use position `operand` of `instruction`: the
    /// inlined definition's tree, or a fresh typed variable read.
    fn operand(
        &mut self,
        position: usize,
        operand: usize,
        instruction: &mlil::Instruction<D>,
        state: &mut ListState,
    ) -> Result<ExpressionId> {
        let variable = instruction.uses()[operand];
        if let Some(&candidate) = state
            .by_consumer
            .get(&position)
            .and_then(|consumed| consumed.get(&variable))
        {
            if let Some(expression) = state.built[candidate].take() {
                return Ok(expression);
            }
        }
        let value_type = instruction
            .use_types()
            .get(operand)
            .cloned()
            .unwrap_or_else(D::void_type);
        self.builder.add_expression(
            ExpressionKind::Variable(VariableId::from_raw(variable.raw())),
            value_type,
        )
    }

    fn operands(
        &mut self,
        position: usize,
        instruction: &mlil::Instruction<D>,
        state: &mut ListState,
    ) -> Result<Vec<ExpressionId>> {
        (0..instruction.uses().len())
            .map(|operand| self.operand(position, operand, instruction, state))
            .collect()
    }

    /// Completes a value-producing instruction: park the expression for its
    /// inlined consumer, or materialize the assignment here.
    fn finish_value(
        &mut self,
        position: usize,
        instruction: &mlil::Instruction<D>,
        expression: ExpressionId,
        state: &mut ListState,
    ) -> Result<()> {
        let id = instruction.id();
        if state.inline_at[position].is_some() {
            state.built[position] = Some(expression);
            self.map_instruction(id, EntityId::Expression(expression))?;
            return Ok(());
        }
        let variable = instruction.defs()[0];
        let value_type = instruction.def_types()[0].clone();
        let target = self.builder.add_expression(
            ExpressionKind::Variable(VariableId::from_raw(variable.raw())),
            value_type,
        )?;
        let statement = self.builder.add_statement(
            StatementKind::Assign {
                target,
                value: expression,
            },
            None,
        )?;
        self.map_instruction(id, EntityId::Statement(statement))?;
        state.statements.push(statement);
        Ok(())
    }
}

fn require_single_definition<D: LiftDialect>(instruction: &mlil::Instruction<D>) -> Result<()> {
    if instruction.defs().len() == 1 {
        Ok(())
    } else {
        Err(Error::UnsupportedLift(format!(
            "instruction {} defines more than one result",
            instruction.id()
        )))
    }
}
