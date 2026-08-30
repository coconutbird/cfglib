//! HLIL → MLIL lowering: control-flow flattening plus expression
//! linearization.
//!
//! [`lower_function`] is the downward mirror of
//! [`lift_function`](super::lift_function): structured statements become
//! blocks and edges, expression trees flatten into typed temporaries, and
//! declared exception regions are registered on the flat graph. The
//! consumer's [`LowerDialect`] supplies the per-operation translation and
//! the edge vocabulary; the library owns block formation, joins, loop and
//! switch wiring, and provenance.
//!
//! # Contract
//!
//! Every completed control path must end in a
//! [`Return`](super::StatementKind::Return) statement or iterate forever —
//! a body whose control falls off the end has no flat representation and
//! is rejected. Non-local transfers (`break`, `return`) out of a `try` do
//! not route through its `finally` body: a frontend with such semantics
//! duplicates the finally statements at each exit before lowering (as
//! source compilers do), keeping [`finally_body`](super::StatementKind::Try)
//! for the exceptional path.

extern crate alloc;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::block::BlockId;
use crate::ir::mlil;

mod control;

use super::{
    Dialect, EntityId, Error, ExpressionId, ExpressionKind, Function, Result, StatementId,
};

/// The bridge between one consumer's HLIL and MLIL dialects for lowering.
///
/// Implemented on the same type as both level dialects; the shared
/// [`Vocabulary`](crate::ir::dialect::Vocabulary) supertrait already
/// equates value types, effects, variables, and source coordinates, and
/// this trait's supertrait bound equates the constant domains.
pub trait LowerDialect:
    mlil::AnalysisDialect + Dialect<Constant = <Self as mlil::AnalysisDialect>::Constant>
{
    /// The MLIL operation applying one HLIL operation to its flattened
    /// operands (uses in operand order, one definition or none).
    fn lower_operation(
        operation: &<Self as Dialect>::Operation,
    ) -> <Self as mlil::Dialect>::Operation;

    /// The MLIL operation materializing one constant into its definition.
    fn lower_constant(constant: &<Self as Dialect>::Constant)
    -> <Self as mlil::Dialect>::Operation;

    /// The MLIL copy operation (one use, one definition).
    fn copy_operation() -> <Self as mlil::Dialect>::Operation;

    /// The MLIL operation storing its **last** use into the place formed by
    /// `location` over the preceding uses — the mirror of
    /// [`Lifted::Store`](super::Lifted::Store).
    fn store_operation(
        location: &<Self as Dialect>::Operation,
    ) -> <Self as mlil::Dialect>::Operation;

    /// The conditional branch deciding a block's true/false edges over its
    /// single use.
    fn branch_operation() -> <Self as mlil::Dialect>::Operation;

    /// The multi-way dispatch over its single use.
    fn switch_operation() -> <Self as mlil::Dialect>::Operation;

    /// The return carrying its uses as returned values.
    fn return_operation() -> <Self as mlil::Dialect>::Operation;

    /// The role assigned to lowering-introduced temporaries.
    fn temporary_role() -> Self::VariableRole;

    /// Whether a lowered operation may transfer exceptionally; drives
    /// unwind-edge placement inside protected regions.
    #[must_use]
    fn operation_may_throw(_operation: &<Self as mlil::Dialect>::Operation) -> bool {
        false
    }

    /// The operation materializing the delivered exception into a handler
    /// binding at its entry. `None` rejects handlers with bindings.
    #[must_use]
    fn caught_exception_operation() -> Option<<Self as mlil::Dialect>::Operation> {
        None
    }

    /// The enter/exit operation pair lowering one dialect
    /// [`Region`](super::StatementKind::Region) statement (for example
    /// monitor enter/exit). `None` rejects region statements.
    #[must_use]
    fn region_operations(
        _operation: &<Self as Dialect>::Operation,
    ) -> Option<(
        <Self as mlil::Dialect>::Operation,
        <Self as mlil::Dialect>::Operation,
    )> {
        None
    }

    /// The synthetic root's unique entry edge.
    fn entry_edge() -> <Self as mlil::Dialect>::Edge;

    /// Sequential fallthrough between consecutively lowered blocks.
    fn fallthrough_edge() -> <Self as mlil::Dialect>::Edge;

    /// A synthesized unconditional transfer (breaks, continues, gotos,
    /// loop back edges).
    fn jump_edge() -> <Self as mlil::Dialect>::Edge;

    /// The taken arm of a lowered conditional.
    fn true_edge() -> <Self as mlil::Dialect>::Edge;

    /// The not-taken arm of a lowered conditional.
    fn false_edge() -> <Self as mlil::Dialect>::Edge;

    /// The dispatch edge selecting one switch case value.
    fn case_edge(value: &<Self as Dialect>::Constant) -> <Self as mlil::Dialect>::Edge;

    /// The dispatch edge taken when no case matches.
    fn default_edge() -> <Self as mlil::Dialect>::Edge;

    /// An exceptional edge from a may-throw block to a handler entry.
    fn unwind_edge() -> <Self as mlil::Dialect>::Edge;
}

/// The result of lowering one HLIL function.
#[derive(Debug, Clone)]
pub struct LoweredFunction<D: LowerDialect> {
    /// The lowered function. Variable identities correspond one-to-one by
    /// index with the HLIL variable table; lowering-introduced temporaries
    /// follow them.
    pub function: mlil::Function<D>,
    /// MLIL instruction → the HLIL entity it was lowered from.
    pub instructions: BTreeMap<mlil::InstructionId, EntityId>,
}

/// Lowers one structured HLIL function into flat, verified MLIL.
///
/// # Errors
///
/// Returns [`Error::UnsupportedLower`] when the statement shape has no flat
/// translation (control falling off the end of the function, a handler
/// binding or region statement without the matching dialect hook, an
/// undeclared binding type), and [`Error::Lowering`] when the assembled
/// MLIL function violates an invariant.
pub fn lower_function<D>(source: &Function<D>) -> Result<LoweredFunction<D>>
where
    D: LowerDialect + mlil::VerifyDialect,
{
    let mut lowerer = Lowerer::new(source)?;
    lowerer.lower_body(source.body())?;
    if lowerer.current.is_some() || !lowerer.pending.is_empty() {
        return Err(Error::UnsupportedLower(
            "control falls off the end of the function; every path must end \
             in a return or iterate forever"
                .into(),
        ));
    }
    let function = lowerer.builder.finish()?;
    Ok(LoweredFunction {
        function,
        instructions: lowerer.instructions,
    })
}

/// One enclosing construct receiving break/continue transfers.
pub(super) struct Frame<D: LowerDialect> {
    pub label: Option<String>,
    pub kind: FrameKind,
    /// Sources waiting to be wired to the construct's continuation.
    pub break_sources: Vec<(BlockId, <D as mlil::Dialect>::Edge)>,
}

pub(super) enum FrameKind {
    /// A loop; `continue` transfers to `continue_target`.
    Loop {
        continue_target: BlockId,
    },
    Switch,
    /// A labeled non-loop statement (labeled breaks only).
    Block,
}

/// One protected region being lowered; collects may-throw blocks for
/// unwind-edge placement.
pub(super) struct TryFrame {
    pub may_throw_blocks: BTreeSet<BlockId>,
}

pub(super) struct Lowerer<'a, D: LowerDialect> {
    pub(super) source: &'a Function<D>,
    pub(super) builder: mlil::FunctionBuilder<D>,
    /// The open block receiving instructions, when one exists.
    pub(super) current: Option<BlockId>,
    /// Edges into the next materialized block.
    pub(super) pending: Vec<(BlockId, <D as mlil::Dialect>::Edge)>,
    pub(super) frames: Vec<Frame<D>>,
    pub(super) try_frames: Vec<TryFrame>,
    /// Materialized label targets.
    pub(super) labels: BTreeMap<String, BlockId>,
    /// Forward-goto sources waiting for their label to materialize.
    pub(super) forward_gotos: BTreeMap<String, Vec<(BlockId, <D as mlil::Dialect>::Edge)>>,
    pub(super) spans: BTreeMap<EntityId, Vec<D::SourceSpan>>,
    pub(super) instructions: BTreeMap<mlil::InstructionId, EntityId>,
}

impl<'a, D: LowerDialect + mlil::VerifyDialect> Lowerer<'a, D> {
    fn new(source: &'a Function<D>) -> Result<Self> {
        let mut builder = mlil::FunctionBuilder::new(source.source().clone());
        for variable in source.variables() {
            builder.declare_variable(variable.role.clone(), variable.native.clone())?;
        }
        let signature = source.signature();
        builder.set_signature(mlil::Signature::<D>::new(
            signature
                .parameters
                .iter()
                .map(|parameter| mlil::VariableId::from_raw(parameter.raw()))
                .collect(),
            signature.returns.clone(),
        ))?;
        let mut spans: BTreeMap<EntityId, Vec<D::SourceSpan>> = BTreeMap::new();
        for entry in source.provenance().entries() {
            if let EntityId::Variable(variable) = entry.entity {
                builder.map_entity(
                    entry.source.clone(),
                    mlil::EntityId::Variable(mlil::VariableId::from_raw(variable.raw())),
                )?;
            } else {
                spans
                    .entry(entry.entity)
                    .or_default()
                    .push(entry.source.clone());
            }
        }
        let entry = builder.entry();
        Ok(Self {
            source,
            builder,
            current: None,
            pending: alloc::vec![(entry, D::entry_edge())],
            frames: Vec::new(),
            try_frames: Vec::new(),
            labels: BTreeMap::new(),
            forward_gotos: BTreeMap::new(),
            spans,
            instructions: BTreeMap::new(),
        })
    }

    /// The open block, materializing one (and wiring the pending edges)
    /// when none is open.
    pub(super) fn block(&mut self) -> Result<BlockId> {
        if let Some(block) = self.current {
            return Ok(block);
        }
        let block = self.builder.new_block("");
        for (from, edge) in core::mem::take(&mut self.pending) {
            self.builder.add_edge(from, block, edge, None)?;
        }
        self.current = Some(block);
        Ok(block)
    }

    /// Ends the open block, deferring its continuation into the next
    /// materialized block through `edge`.
    pub(super) fn seal(&mut self, edge: <D as mlil::Dialect>::Edge) {
        if let Some(block) = self.current.take() {
            self.pending.push((block, edge));
        }
    }

    /// Wires the open block and every pending edge into `target`.
    pub(super) fn connect_to(&mut self, target: BlockId) -> Result<()> {
        if let Some(block) = self.current.take() {
            self.builder.add_edge(block, target, D::jump_edge(), None)?;
        }
        for (from, edge) in core::mem::take(&mut self.pending) {
            self.builder.add_edge(from, target, edge, None)?;
        }
        Ok(())
    }

    /// Appends one instruction to the open block, recording provenance,
    /// the origin map, and may-throw facts for enclosing regions.
    pub(super) fn emit(
        &mut self,
        operation: <D as mlil::Dialect>::Operation,
        uses: Vec<mlil::TypedVariable<D>>,
        defs: Vec<mlil::TypedVariable<D>>,
        origin: EntityId,
    ) -> Result<mlil::InstructionId> {
        let may_throw = D::operation_may_throw(&operation);
        let block = self.block()?;
        let instruction = self
            .builder
            .append_instruction(block, operation, uses, defs, may_throw, None)?;
        if may_throw {
            for frame in &mut self.try_frames {
                frame.may_throw_blocks.insert(block);
            }
        }
        self.instructions.insert(instruction, origin);
        if let Some(spans) = self.spans.get(&origin) {
            for span in spans.clone() {
                self.builder
                    .map_entity(span, mlil::EntityId::Instruction(instruction))?;
            }
        }
        Ok(instruction)
    }

    /// One flattened value: the variable holding it and its occurrence type.
    pub(super) fn flatten(
        &mut self,
        expression: ExpressionId,
    ) -> Result<(mlil::VariableId, D::ValueType)> {
        let node = self.expression(expression)?;
        let value_type = node.value_type().clone();
        match node.kind().clone() {
            ExpressionKind::Variable(variable) => {
                Ok((mlil::VariableId::from_raw(variable.raw()), value_type))
            }
            ExpressionKind::Constant(constant) => {
                let temporary = self.builder.declare_variable(D::temporary_role(), None)?;
                self.emit(
                    D::lower_constant(&constant),
                    Vec::new(),
                    alloc::vec![mlil::TypedVariable::new(temporary, value_type.clone())],
                    EntityId::Expression(expression),
                )?;
                Ok((temporary, value_type))
            }
            ExpressionKind::Operation {
                operation,
                operands,
            } => {
                let uses = self.flatten_all(&operands)?;
                let temporary = self.builder.declare_variable(D::temporary_role(), None)?;
                self.emit(
                    D::lower_operation(&operation),
                    uses,
                    alloc::vec![mlil::TypedVariable::new(temporary, value_type.clone())],
                    EntityId::Expression(expression),
                )?;
                Ok((temporary, value_type))
            }
        }
    }

    /// Flattens `expressions` in order into typed operand occurrences.
    pub(super) fn flatten_all(
        &mut self,
        expressions: &[ExpressionId],
    ) -> Result<Vec<mlil::TypedVariable<D>>> {
        expressions
            .iter()
            .map(|&operand| {
                let (variable, value_type) = self.flatten(operand)?;
                Ok(mlil::TypedVariable::new(variable, value_type))
            })
            .collect()
    }

    /// Flattens a value directly into `target` (avoiding a temporary).
    pub(super) fn flatten_into(
        &mut self,
        target: mlil::VariableId,
        target_type: D::ValueType,
        value: ExpressionId,
        origin: EntityId,
    ) -> Result<()> {
        let node = self.expression(value)?;
        match node.kind().clone() {
            ExpressionKind::Variable(variable) => {
                let value_type = node.value_type().clone();
                self.emit(
                    D::copy_operation(),
                    alloc::vec![mlil::TypedVariable::new(
                        mlil::VariableId::from_raw(variable.raw()),
                        value_type,
                    )],
                    alloc::vec![mlil::TypedVariable::new(target, target_type)],
                    origin,
                )?;
            }
            ExpressionKind::Constant(constant) => {
                self.emit(
                    D::lower_constant(&constant),
                    Vec::new(),
                    alloc::vec![mlil::TypedVariable::new(target, target_type)],
                    origin,
                )?;
            }
            ExpressionKind::Operation {
                operation,
                operands,
            } => {
                let uses = self.flatten_all(&operands)?;
                self.emit(
                    D::lower_operation(&operation),
                    uses,
                    alloc::vec![mlil::TypedVariable::new(target, target_type)],
                    origin,
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn expression(&self, id: ExpressionId) -> Result<&'a super::Expression<D>> {
        self.source.expression(id).ok_or_else(|| {
            Error::UnsupportedLower(format!("statement references missing expression {id}"))
        })
    }

    pub(super) fn statement(&self, id: StatementId) -> Result<&'a super::Statement<D>> {
        self.source.statement(id).ok_or_else(|| {
            Error::UnsupportedLower(format!("body references missing statement {id}"))
        })
    }
}
