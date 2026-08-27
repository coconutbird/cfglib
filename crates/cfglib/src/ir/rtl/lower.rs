//! Lowering of MLIL functions into target RTL.
//!
//! The downward counterpart of [`lift`](super::lift()): the dialect
//! plans target storage for the whole function at once ([`Lower::plan`])
//! and translates each instruction into RTL statements
//! ([`Lower::lower_instruction`]) — legalizing representable semantic
//! operations, staging parallel copies as [`Statement::Transfer`]s, and
//! refusing unsupported semantics with a typed
//! [`Error::Lowering`](super::Error::Lowering) rather than a silent
//! approximation. Edges lower *after* every statement exists, so
//! [`Lower::lower_edge`] can remap instruction identities in edge
//! payloads (an exceptional edge's throw site) onto lowered
//! [`StatementId`]s. Lifetime splitting and coalescing happen at the
//! MLIL level
//! ([`split_variables`](crate::ir::mlil::Function::split_variables),
//! copy propagation) before lowering; instruction selection, layout, and
//! encoding stay frontend-owned below RTL.
//!
//! [`Lowered`] retains the full rewrite map — MLIL block, instruction,
//! and edge to their RTL counterparts — so provenance survives the
//! round trip.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;

use crate::ir::mlil::{
    Function as MlilFunction, Instruction as MlilInstruction, InstructionId, VariableId,
};
use crate::{BlockId, EdgeId};

use super::dialect::Dialect;
use super::error::{Error, Result};
use super::expr::{Expr, Place};
use super::function::{Function, FunctionBuilder};
use super::statement::{Statement, StatementId};

/// One whole-function storage assignment: every MLIL variable's target
/// place, planned in one coordinated pass.
#[derive(Debug, Clone, Default)]
pub struct Placement<D: Dialect> {
    places: BTreeMap<u32, Place<D>>,
}

impl<D: Dialect> Placement<D> {
    /// An empty plan.
    #[must_use]
    pub fn new() -> Self {
        Self {
            places: BTreeMap::new(),
        }
    }

    /// Assigns one variable's target place.
    pub fn assign(&mut self, variable: VariableId, place: Place<D>) {
        self.places.insert(variable.raw(), place);
    }

    /// The planned place of one variable.
    #[must_use]
    pub fn place(&self, variable: VariableId) -> Option<&Place<D>> {
        self.places.get(&variable.raw())
    }
}

/// The lowering contract from a dialect's MLIL onto target RTL.
///
/// Implemented on the same type as the MLIL dialect, so both levels
/// share one vocabulary; the edge types stay independent, translated by
/// [`lower_edge`](Self::lower_edge). Storage-assignment and legalization
/// policy live entirely in the implementation; cfglib supplies the walk,
/// the checked construction, and the rewrite maps.
pub trait Lower: Dialect + crate::ir::mlil::Dialect {
    /// Plans target storage for the whole function at once — coordinated
    /// allocation (JVM local numbering, Dalvik register pressure, wide
    /// pairs) rather than one-variable-at-a-time choices. Every variable
    /// an instruction touches must receive a place; a missing one
    /// surfaces as a typed error when the translation reads it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Lowering`](super::Error::Lowering) when some
    /// variable has no legal storage assignment.
    fn plan(function: &MlilFunction<Self>) -> Result<Placement<Self>>;

    /// Translates one MLIL instruction into RTL statements through the
    /// context.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Lowering`](super::Error::Lowering) when the
    /// instruction's semantics have no target representation.
    fn lower_instruction(
        context: &mut LowerContext<'_, Self>,
        instruction: &MlilInstruction<Self>,
    ) -> Result<()>;

    /// Translates one MLIL edge into the lowered function's RTL edge
    /// metadata.
    ///
    /// Runs after every statement is emitted, so the context resolves
    /// instructions to lowered [`StatementId`]s — an exceptional edge's
    /// throw site remaps from the MLIL instruction identity into the RTL
    /// statement domain. A dialect sharing one edge type across both
    /// levels clones the metadata.
    #[must_use]
    fn lower_edge(
        edge: &<Self as crate::ir::mlil::Dialect>::Edge,
        context: &LowerEdgeContext<'_>,
    ) -> <Self as Dialect>::Edge;
}

/// The context one [`Lower::lower_instruction`] call appends RTL
/// statements through.
pub struct LowerContext<'a, D: Lower> {
    builder: &'a mut FunctionBuilder<D>,
    block: BlockId,
    placement: &'a Placement<D>,
    span: Option<<D as crate::ir::dialect::Vocabulary>::SourceSpan>,
    statements: Vec<StatementId>,
}

impl<D: Lower> LowerContext<'_, D> {
    /// The planned target place of one MLIL variable.
    ///
    /// # Errors
    ///
    /// Returns an error for a variable the plan never placed.
    pub fn place(&self, variable: VariableId) -> Result<&Place<D>> {
        self.placement
            .place(variable)
            .ok_or_else(|| Error::Lowering(format!("variable {variable:?} has no target place")))
    }

    /// A whole-place read of one MLIL variable under a constraint.
    ///
    /// # Errors
    ///
    /// Returns an error for a variable without a target place.
    pub fn read(&self, variable: VariableId, scalar: D::Constraint) -> Result<Expr<D>> {
        let place = self.place(variable)?;
        Ok(Expr::Read {
            storage: place.storage.clone(),
            lanes: place.lanes.clone(),
            scalar,
        })
    }

    /// Appends one RTL statement for the instruction, validated by the
    /// checked builder and carrying the instruction's source span.
    ///
    /// # Errors
    ///
    /// Returns an error when the statement fails RTL validation.
    pub fn emit(&mut self, statement: Statement<D>) -> Result<StatementId> {
        let id = self
            .builder
            .append(self.block, statement, self.span.clone())?;
        self.statements.push(id);
        Ok(id)
    }
}

/// The finished mappings available while lowering one edge.
pub struct LowerEdgeContext<'a> {
    blocks: &'a [BlockId],
    statements: &'a [Vec<StatementId>],
    owner: Option<InstructionId>,
}

impl LowerEdgeContext<'_> {
    /// The MLIL instruction owning the edge: the unique throwing
    /// instruction of the source block for an exceptional edge, the
    /// block's final instruction otherwise, `None` for an empty block.
    #[must_use]
    pub const fn owner(&self) -> Option<InstructionId> {
        self.owner
    }

    /// The RTL statements one MLIL instruction became, in order.
    #[must_use]
    pub fn statements(&self, instruction: InstructionId) -> &[StatementId] {
        self.statements
            .get(instruction.index())
            .map_or(&[], Vec::as_slice)
    }

    /// The RTL block one MLIL block became.
    #[must_use]
    pub fn block(&self, block: BlockId) -> Option<BlockId> {
        self.blocks.get(block.index()).copied()
    }
}

/// The result of lowering: a checked RTL function plus the rewrite maps
/// from every MLIL entity to its RTL counterparts.
pub struct Lowered<D: Lower> {
    /// The lowered RTL function.
    pub function: Function<D>,
    /// The storage plan the lowering ran under — including places
    /// reserved for variables no instruction touched, such as unused
    /// parameters holding JVM locals or Dalvik registers.
    pub placement: Placement<D>,
    /// MLIL block index → RTL block.
    blocks: Vec<BlockId>,
    /// MLIL instruction index → RTL statements, in emission order.
    statements: Vec<Vec<StatementId>>,
    /// MLIL edge index → RTL edge.
    edges: Vec<EdgeId>,
}

impl<D: Lower> Lowered<D> {
    /// The RTL block one MLIL block became.
    #[must_use]
    pub fn block(&self, block: BlockId) -> Option<BlockId> {
        self.blocks.get(block.index()).copied()
    }

    /// The RTL statements one MLIL instruction became, in order.
    #[must_use]
    pub fn statements(&self, instruction: InstructionId) -> &[StatementId] {
        self.statements
            .get(instruction.index())
            .map_or(&[], Vec::as_slice)
    }

    /// The RTL edge one MLIL edge became.
    #[must_use]
    pub fn edge(&self, edge: EdgeId) -> Option<EdgeId> {
        self.edges.get(edge.index()).copied()
    }
}

/// Lowers one MLIL function onto target RTL.
///
/// # Errors
///
/// Returns [`Error::Lowering`](super::Error::Lowering) when a variable
/// has no legal storage or an instruction no target representation, and
/// construction errors when the emitted RTL is structurally invalid.
pub fn lower<D: Lower>(function: &MlilFunction<D>) -> Result<Lowered<D>> {
    let cfg = function.cfg();
    let placement = D::plan(function)?;

    let mut builder = FunctionBuilder::<D>::new(function.source().clone());
    let mut blocks: Vec<BlockId> = Vec::with_capacity(cfg.block_count());
    for block in cfg.blocks() {
        if block.id() == cfg.entry() {
            blocks.push(builder.entry());
        } else {
            let label = block.label().unwrap_or("b").to_string();
            blocks.push(builder.new_block(label));
        }
    }

    let mut statements: Vec<Vec<StatementId>> = vec![Vec::new(); function.instruction_count()];
    for block in cfg.blocks() {
        for instruction in block.instructions() {
            let span = function
                .provenance()
                .mappings_to(crate::ir::mlil::EntityId::Instruction(instruction.id()))
                .next()
                .map(|entry| entry.source.clone());
            let mut context = LowerContext {
                builder: &mut builder,
                block: blocks[block.id().index()],
                placement: &placement,
                span,
                statements: Vec::new(),
            };
            D::lower_instruction(&mut context, instruction)?;
            statements[instruction.id().index()] = context.statements;
        }
    }

    let mut edges: Vec<EdgeId> = Vec::new();
    for edge in cfg.edges() {
        let source = cfg.block(edge.source());
        let owner = if edge.kind().is_exceptional() {
            source
                .instructions()
                .iter()
                .find(|instruction| instruction.may_throw())
                .map(MlilInstruction::id)
        } else {
            source.instructions().last().map(MlilInstruction::id)
        };
        let context = LowerEdgeContext {
            blocks: &blocks,
            statements: &statements,
            owner,
        };
        let lowered = builder.add_edge(
            blocks[edge.source().index()],
            blocks[edge.target().index()],
            D::lower_edge(edge.payload(), &context),
        )?;
        edges.push(lowered);
    }

    Ok(Lowered {
        function: builder.finish()?,
        placement,
        blocks,
        statements,
        edges,
    })
}
