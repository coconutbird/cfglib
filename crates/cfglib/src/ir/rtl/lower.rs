//! Lowering of MLIL functions into target RTL.
//!
//! The downward counterpart of [`lift`](super::lift()): the dialect
//! chooses target storage for every MLIL variable ([`Lower::place`]) and
//! translates each instruction into RTL statements
//! ([`Lower::lower_instruction`]) — legalizing representable semantic
//! operations, staging parallel copies as [`Statement::Transfer`]s, and
//! refusing unsupported semantics with a typed
//! [`Error::Lowering`](super::Error::Lowering) rather than a silent
//! approximation. Lifetime splitting and coalescing happen at the MLIL
//! level ([`split_variables`](crate::ir::mlil::Function::split_variables),
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
    Function as MlilFunction, Instruction as MlilInstruction, VariableId,
};
use crate::{BlockId, EdgeId};

use super::dialect::Dialect;
use super::error::{Error, Result};
use super::expr::{Expr, Place};
use super::function::{Function, FunctionBuilder};
use super::statement::{Statement, StatementId};

/// The lowering contract from a dialect's MLIL onto target RTL.
///
/// Implemented on the same type as the MLIL dialect, so both levels
/// share one vocabulary and one edge type. Storage-assignment and
/// legalization policy live entirely in the implementation; cfglib
/// supplies the walk, the checked construction, and the rewrite maps.
pub trait Lower: Dialect + crate::ir::mlil::Dialect<Edge = <Self as Dialect>::Edge> {
    /// The target place of one MLIL variable.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Lowering`](super::Error::Lowering) when the
    /// variable has no legal storage assignment.
    fn place(function: &MlilFunction<Self>, variable: VariableId) -> Result<Place<Self>>;

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

    /// Metadata of one lowered edge; the default clones verbatim.
    #[must_use]
    fn lower_edge(edge: &<Self as Dialect>::Edge) -> <Self as Dialect>::Edge {
        edge.clone()
    }
}

/// The context one [`Lower::lower_instruction`] call appends RTL
/// statements through.
pub struct LowerContext<'a, D: Lower> {
    builder: &'a mut FunctionBuilder<D>,
    block: BlockId,
    places: &'a BTreeMap<u32, Place<D>>,
    statements: Vec<StatementId>,
}

impl<D: Lower> LowerContext<'_, D> {
    /// The chosen target place of one MLIL variable.
    ///
    /// # Errors
    ///
    /// Returns an error for a variable the placement pass never saw.
    pub fn place(&self, variable: VariableId) -> Result<&Place<D>> {
        self.places
            .get(&variable.raw())
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
    /// checked builder.
    ///
    /// # Errors
    ///
    /// Returns an error when the statement fails RTL validation.
    pub fn emit(&mut self, statement: Statement<D>) -> Result<StatementId> {
        let id = self.builder.append(self.block, statement, None)?;
        self.statements.push(id);
        Ok(id)
    }
}

/// The result of lowering: a checked RTL function plus the rewrite maps
/// from every MLIL entity to its RTL counterparts.
pub struct Lowered<D: Lower> {
    /// The lowered RTL function.
    pub function: Function<D>,
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
    pub fn statements(&self, instruction: crate::ir::mlil::InstructionId) -> &[StatementId] {
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

    // Deterministic storage assignment: every variable an instruction
    // touches gets its place once, in identity order.
    let mut places: BTreeMap<u32, Place<D>> = BTreeMap::new();
    for block in cfg.blocks() {
        for instruction in block.instructions() {
            for variable in instruction.uses().iter().chain(instruction.defs()) {
                if let alloc::collections::btree_map::Entry::Vacant(entry) =
                    places.entry(variable.raw())
                {
                    entry.insert(D::place(function, *variable)?);
                }
            }
        }
    }

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
            let mut context = LowerContext {
                builder: &mut builder,
                block: blocks[block.id().index()],
                places: &places,
                statements: Vec::new(),
            };
            D::lower_instruction(&mut context, instruction)?;
            statements[instruction.id().index()] = context.statements;
        }
    }

    let mut edges: Vec<EdgeId> = Vec::with_capacity(cfg.edges().count());
    for edge in cfg.edges() {
        let lowered = builder.add_edge(
            blocks[edge.source().index()],
            blocks[edge.target().index()],
            D::lower_edge(edge.payload()),
        )?;
        edges.push(lowered);
    }

    Ok(Lowered {
        function: builder.finish()?,
        blocks,
        statements,
        edges,
    })
}
