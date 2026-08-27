//! Lifting of RTL functions into typed MLIL through def-use webs.
//!
//! Storage reuse dissolves here: per-lane SSA versions unite into webs —
//! *live* φ families (a header φ nothing reads must not fuse the
//! lifetimes on either side of it), lanes co-written by one assignment
//! with each other — and every web becomes one typed MLIL variable. A
//! register reused for a float and later a counter becomes two variables
//! with honest constraints, so the consumer renders reinterpretations
//! only where a web genuinely mixes representations. Parallel transfers
//! serialize safely because reads reference prior versions; when a
//! target web is also read by a sibling assignment, the pre-state is
//! copied into a synthetic temporary first.
//!
//! Emission is dialect-driven: each serialized statement reaches
//! [`Lift::emit`] through an [`Emission`] context that can append one or
//! more MLIL instructions — the storage-flavored one-operation form is
//! [`Emission::single`], while semantic dialects expand or fuse freely
//! under read-alignment and exceptional-placement validation. Edges lift
//! *after* every instruction exists, so [`Lift::lift_edge`] can bake
//! exact emitted throw-site identities into exceptional edge payloads,
//! and [`LiftMaps`] returns the complete block, statement, instruction,
//! and edge correspondences for regions, signatures, and diagnostics.
//!
//! The emitted operation templates are identity-free: a
//! [`VarExpr::Read`] names positions and a constraint, never a variable,
//! and the instruction's positional `uses`/`defs` lists carry the web
//! variables (one use per read in pre-order, the definition as the sole
//! def). Generic MLIL transformations that rewrite instruction operands
//! therefore stay sound over lifted functions.

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::ToString;
use alloc::vec::Vec;

use crate::ir::dialect::Vocabulary;
use crate::ir::mlil::{
    FunctionBuilder as MlilBuilder, InstructionId, TypedVariable, VariableId,
};
use crate::{BlockId, DominatorTree, EdgeId, PhiWebs, SsaForm, SsaValue};

use super::dialect::{Dialect, Lift};
use super::error::{Error, Result};
use super::expr::{Expr, Place};
use super::function::Function;
use super::render::Webs;
use super::statement::{Lane, Statement, StatementId};
use super::types::{Inference, Shape};

/// One typed expression over lifted web variables.
///
/// Consumers embed these in their MLIL operations: the tree mirrors the
/// RTL expression, with storage reads resolved to positions within web
/// variables. The web variables themselves live in the instruction's
/// `uses` list — one entry per [`Read`](VarExpr::Read) in pre-order —
/// so the template survives operand rewriting; pair reads with operands
/// through [`ReadResolver`](super::ReadResolver).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VarExpr<D: Dialect> {
    /// A read of one web variable's positions, with the constraint the
    /// consuming operation imposes. The variable read is the matching
    /// pre-order entry of the instruction's `uses`.
    Read {
        /// Positions within the web variable, in consumption order.
        positions: Vec<u8>,
        /// The constraint the consumer imposes on each position.
        scalar: D::Constraint,
    },
    /// An immediate value, one bit pattern per lane.
    Const {
        /// Raw lane bit patterns:
        /// [`Constraint::words`](super::Constraint::words) little-endian
        /// 64-bit words per lane, lanes in order.
        bits: Vec<u64>,
        /// The shape of the constant.
        shape: Shape<D::Constraint>,
    },
    /// A pure operator application.
    Apply {
        /// The dialect operator.
        operator: D::Operator,
        /// Operand values in operator order.
        operands: Vec<VarExpr<D>>,
        /// The result shape.
        shape: Shape<D::Constraint>,
    },
    /// A bit reinterpretation to a same-width shape.
    Reinterpret {
        /// The reinterpreted value.
        operand: Box<VarExpr<D>>,
        /// The target shape.
        shape: Shape<D::Constraint>,
    },
    /// A vector composed from reads of several webs — one storage read
    /// whose lanes resolved to different variables.
    Compose {
        /// The composed parts in lane order.
        parts: Vec<VarExpr<D>>,
        /// The composed shape.
        shape: Shape<D::Constraint>,
    },
}

impl<D: Dialect> VarExpr<D> {
    /// The shape of this expression's value.
    #[must_use]
    pub fn shape(&self) -> Shape<D::Constraint> {
        match self {
            Self::Read {
                positions, scalar, ..
            } => Shape {
                scalar: scalar.clone(),
                lanes: u8::try_from(positions.len()).unwrap_or(u8::MAX),
            },
            Self::Const { shape, .. }
            | Self::Apply { shape, .. }
            | Self::Reinterpret { shape, .. }
            | Self::Compose { shape, .. } => shape.clone(),
        }
    }
}

/// One lifted statement handed to the consumer's [`Lift::emit`] hook.
///
/// The statement is a template over the instruction's positional
/// variable lists: reads align with `uses` in pre-order, and an
/// assignment's written variable is the instruction's sole definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiftedStatement<D: Dialect> {
    /// One serialized assignment of a value to positions of the
    /// instruction's defined web variable.
    Assign {
        /// Written positions within the target, in value-lane order.
        positions: Vec<u8>,
        /// The target web's full width.
        width: u8,
        /// Whether unwritten positions keep their prior value — the
        /// instruction then also uses the target as its trailing operand.
        merges: bool,
        /// The assigned value.
        value: VarExpr<D>,
        /// Observable effects attached to this instruction.
        effects: Vec<<D as Vocabulary>::Effect>,
    },
    /// An effect-bearing operation.
    Effect {
        /// The dialect effect operation.
        operation: D::EffectOp,
        /// Operand values in operation order.
        operands: Vec<VarExpr<D>>,
        /// Observable effects of the operation.
        effects: Vec<<D as Vocabulary>::Effect>,
    },
    /// A conditional transfer on one scalar condition.
    Branch {
        /// The scalar branch condition.
        condition: VarExpr<D>,
    },
    /// A multi-way dispatch on one scalar scrutinee.
    Dispatch {
        /// The scalar dispatch scrutinee.
        scrutinee: VarExpr<D>,
    },
    /// A function return carrying result values.
    ///
    /// The lift materializes every non-trivial value into a temporary,
    /// so each value is a whole single-variable [`VarExpr::Read`] and
    /// the instruction's `uses` pair one-to-one with the returned
    /// values — the shape [`Lifted::Return`](crate::ir::hlil::Lifted)
    /// requires.
    Return {
        /// Returned values in signature order.
        values: Vec<VarExpr<D>>,
    },
    /// A terminating exceptional raise.
    Raise {
        /// The dialect effect operation performing the raise.
        operation: D::EffectOp,
        /// Operand values in operation order.
        operands: Vec<VarExpr<D>>,
        /// Observable effects beyond the exceptional transfer itself.
        effects: Vec<<D as Vocabulary>::Effect>,
    },
}

/// One lifted web: a typed MLIL variable recovered from storage lanes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebInfo<D: Dialect> {
    /// The declared MLIL variable.
    pub variable: VariableId,
    /// The native storage the web lives in, or `None` for a synthetic
    /// temporary.
    pub storage: Option<<D as Vocabulary>::NativeVariable>,
    /// Ascending storage lanes the web spans.
    pub lanes: Vec<u8>,
    /// The web's inferred shape.
    pub shape: Shape<D::Constraint>,
    /// Whether the web contains the version-zero live-in value.
    pub live_in: bool,
}

/// The stable correspondences one lift established.
///
/// Signatures, exception regions, native provenance, and diagnostics
/// attach through these maps rather than through coincidentally equal
/// dense identities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiftMaps {
    blocks: Vec<BlockId>,
    instructions: Vec<Vec<InstructionId>>,
    throw_sites: Vec<Option<InstructionId>>,
    edges: Vec<EdgeId>,
}

impl LiftMaps {
    /// The MLIL block one RTL block became.
    #[must_use]
    pub fn block(&self, block: BlockId) -> Option<BlockId> {
        self.blocks.get(block.index()).copied()
    }

    /// The MLIL instructions one RTL statement emitted, in order.
    #[must_use]
    pub fn instructions(&self, statement: StatementId) -> &[InstructionId] {
        self.instructions
            .get(statement.index())
            .map_or(&[], Vec::as_slice)
    }

    /// The emitted instruction designated as one statement's throw site.
    #[must_use]
    pub fn throw_site(&self, statement: StatementId) -> Option<InstructionId> {
        self.throw_sites.get(statement.index()).copied().flatten()
    }

    /// The MLIL edge one RTL edge became.
    #[must_use]
    pub fn edge(&self, edge: EdgeId) -> Option<EdgeId> {
        self.edges.get(edge.index()).copied()
    }
}

/// The result of lifting: an MLIL builder ready for signature and region
/// assignment and [`finish`](MlilBuilder::finish), the recovered webs,
/// and the full provenance maps.
pub struct Lifting<D: Lift> {
    /// The populated MLIL builder.
    pub builder: MlilBuilder<D>,
    /// Recovered webs, resolvable by variable identity. Webs flagged
    /// [`live_in`](WebInfo::live_in) are the function's parameters and
    /// implicit input channels, in declaration order.
    pub webs: Webs<D>,
    /// Block, statement, instruction, and edge correspondences.
    pub maps: LiftMaps,
}

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn find(&mut self, mut node: usize) -> usize {
        while self.parent[node] != node {
            self.parent[node] = self.parent[self.parent[node]];
            node = self.parent[node];
        }
        node
    }

    fn unite(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            // Uniting toward the smaller id keeps ordering deterministic.
            let (keep, fold) = if ra < rb { (ra, rb) } else { (rb, ra) };
            self.parent[fold] = keep;
        }
    }
}

struct WebFact<D: Dialect> {
    storage: <D as Vocabulary>::NativeVariable,
    lanes: BTreeSet<u8>,
    scalar: Inference<D::Constraint>,
    live_in: bool,
}

/// Lifts one RTL function into a typed MLIL builder.
///
/// # Errors
///
/// Returns an error when SSA annotations disagree with statement
/// structure, the dialect's emission violates read alignment or
/// exceptional placement, or MLIL construction fails.
#[expect(clippy::too_many_lines, reason = "one linear pass per phase")]
pub fn lift<D: Lift>(function: &Function<D>) -> Result<Lifting<D>> {
    let cfg = &function.cfg;
    let dominators = DominatorTree::compute(cfg);
    let ssa: SsaForm<Lane<D>> = SsaForm::compute(cfg, &dominators);

    // Phase 1: dense ids for every SSA value, then unite φ families and
    // co-written lanes into webs. Only *live* φ families unite: SSA
    // placement is not liveness-pruned, and a dead header φ would fuse
    // the unrelated lifetimes on either side of a full overwrite.
    let mut ids: BTreeMap<(Lane<D>, usize), usize> = BTreeMap::new();
    let mut union = UnionFind { parent: Vec::new() };
    let mut id_of = |value: &SsaValue<Lane<D>>, union: &mut UnionFind| -> usize {
        let key = (value.variable.clone(), value.version);
        if let Some(&id) = ids.get(&key) {
            return id;
        }
        let id = union.parent.len();
        union.parent.push(id);
        ids.insert(key, id);
        id
    };
    for web in &PhiWebs::compute_live(&ssa).webs {
        let mut anchor: Option<usize> = None;
        for value in &web.values {
            let id = id_of(value, &mut union);
            match anchor {
                Some(anchor) => union.unite(anchor, id),
                None => anchor = Some(id),
            }
        }
    }
    for block in cfg.blocks() {
        let annotations = ssa.block(block.id());
        for (index, node) in block.instructions().iter().enumerate() {
            let annotation = annotations
                .instructions
                .get(index)
                .ok_or_else(|| Error::Lifting("SSA lost an instruction".into()))?;
            for value in &annotation.uses {
                id_of(value, &mut union);
            }
            if let Statement::Transfer { assignments, .. } = node.statement() {
                let mut cursor = 0usize;
                for (place, _) in assignments {
                    let defs = annotation
                        .defs
                        .get(cursor..cursor + place.lanes.len())
                        .ok_or_else(|| Error::Lifting("SSA lost a definition".into()))?;
                    let first = id_of(&defs[0], &mut union);
                    for value in &defs[1..] {
                        let other = id_of(value, &mut union);
                        union.unite(first, other);
                    }
                    cursor += place.lanes.len();
                }
            }
        }
    }

    // The version-zero values of one storage are all the same live-in
    // value: unite them so an input register read lane-by-lane stays one
    // variable.
    let mut previous: Option<(
        &<D as crate::ir::dialect::Vocabulary>::NativeVariable,
        usize,
    )> = None;
    for ((lane, version), &id) in &ids {
        if *version != 0 {
            continue;
        }
        if let Some((storage, first)) = previous
            && *storage == lane.0
        {
            union.unite(first, id);
        } else {
            previous = Some((&lane.0, id));
        }
    }

    // Phase 2: resolve roots and collect per-web facts.
    let roots: Vec<usize> = (0..union.parent.len()).map(|id| union.find(id)).collect();
    let mut facts: BTreeMap<usize, WebFact<D>> = BTreeMap::new();
    for ((lane, version), &id) in &ids {
        let root = roots[id];
        let fact = facts.entry(root).or_insert_with(|| WebFact {
            storage: lane.0.clone(),
            lanes: BTreeSet::new(),
            scalar: Inference::Unseen,
            live_in: false,
        });
        fact.lanes.insert(lane.1);
        fact.live_in |= *version == 0;
    }

    // Phase 3: infer each web's constraint from read wants and
    // assignment value shapes.
    for block in cfg.blocks() {
        let annotations = ssa.block(block.id());
        for (index, node) in block.instructions().iter().enumerate() {
            let annotation = &annotations.instructions[index];
            let mut cursor = 0usize;
            let mut missing = false;
            node.statement().for_each_read(&mut |_, lanes, scalar| {
                for _ in lanes {
                    let Some(value) = annotation.uses.get(cursor) else {
                        missing = true;
                        return;
                    };
                    cursor += 1;
                    let key = (value.variable.clone(), value.version);
                    if let Some(&id) = ids.get(&key)
                        && let Some(fact) = facts.get_mut(&roots[id])
                    {
                        fact.scalar.observe(scalar);
                    }
                }
            });
            if missing {
                return Err(Error::Lifting("SSA lost a use".into()));
            }
            if let Statement::Transfer { assignments, .. } = node.statement() {
                let mut def_cursor = 0usize;
                for (place, value) in assignments {
                    let target = &annotation.defs[def_cursor];
                    def_cursor += place.lanes.len();
                    let key = (target.variable.clone(), target.version);
                    if let Some(&id) = ids.get(&key)
                        && let Some(fact) = facts.get_mut(&roots[id])
                    {
                        fact.scalar.observe(&value.shape().scalar);
                    }
                }
            }
        }
    }

    // Phase 4: declare one typed MLIL variable per web, in deterministic
    // first-appearance order over sorted SSA values.
    let mut builder = MlilBuilder::<D>::new(function.source.clone());
    let mut web_of_root: BTreeMap<usize, usize> = BTreeMap::new();
    let mut webs: Vec<WebInfo<D>> = Vec::new();
    for &id in ids.values() {
        let root = roots[id];
        if web_of_root.contains_key(&root) {
            continue;
        }
        let fact = &facts[&root];
        let lanes: Vec<u8> = fact.lanes.iter().copied().collect();
        let shape = Shape {
            scalar: fact.scalar.resolve(),
            lanes: u8::try_from(lanes.len()).unwrap_or(u8::MAX),
        };
        let variable = builder
            .declare_variable(D::web_role(Some(&fact.storage)), Some(fact.storage.clone()))
            .map_err(|error| Error::Lifting(error.to_string()))?;
        web_of_root.insert(root, webs.len());
        webs.push(WebInfo {
            variable,
            storage: Some(fact.storage.clone()),
            lanes,
            shape,
            live_in: fact.live_in,
        });
    }

    // Phase 5: mirror blocks. Edges wait until every instruction exists,
    // so edge lifting can reference emitted instruction identities.
    let mut block_map: Vec<BlockId> = Vec::with_capacity(cfg.block_count());
    for block in cfg.blocks() {
        if block.id() == cfg.entry() {
            block_map.push(builder.entry());
        } else {
            let label = block.label().unwrap_or("b").to_string();
            block_map.push(builder.new_block(label));
        }
    }

    // Phase 6: emit MLIL instructions through the dialect's hook, one
    // serialized statement at a time.
    let resolver = Resolver {
        ids: &ids,
        roots: &roots,
        web_of_root: &web_of_root,
    };
    let statement_count = function.statement_count();
    let mut emitter = Emitter {
        builder,
        web_index: webs
            .iter()
            .enumerate()
            .map(|(index, web)| (web.variable, index))
            .collect(),
        webs,
        resolver,
        maps: LiftMaps {
            blocks: block_map.clone(),
            instructions: alloc::vec![Vec::new(); statement_count],
            throw_sites: alloc::vec![None; statement_count],
            edges: Vec::new(),
        },
        current: None,
        native_defined: false,
    };
    for block in cfg.blocks() {
        let annotations = ssa.block(block.id());
        for (index, node) in block.instructions().iter().enumerate() {
            let annotation = &annotations.instructions[index];
            emitter.statement(
                block_map[block.id().index()],
                node.id(),
                node.statement(),
                node.span().cloned(),
                annotation,
            )?;
        }
    }

    // Phase 7: lift edges with the finished statement-to-instruction
    // mappings in hand.
    for edge in cfg.edges() {
        let source = cfg.block(edge.source());
        let owner = if edge.kind().is_exceptional() {
            source
                .instructions()
                .iter()
                .find(|node| node.statement().may_throw())
                .map(super::statement::StatementNode::id)
        } else {
            source
                .instructions()
                .last()
                .filter(|node| node.statement().is_terminator())
                .map(super::statement::StatementNode::id)
        };
        let context = EdgeContext {
            maps: &emitter.maps,
            owner,
        };
        let metadata = D::lift_edge(edge.payload(), &context);
        let lifted = emitter
            .builder
            .add_edge(
                block_map[edge.source().index()],
                block_map[edge.target().index()],
                metadata,
                None,
            )
            .map_err(|error| Error::Lifting(error.to_string()))?;
        emitter.maps.edges.push(lifted);
    }

    Ok(Lifting {
        builder: emitter.builder,
        webs: Webs::new(emitter.webs),
        maps: emitter.maps,
    })
}

/// The finished mappings available while lifting one edge.
pub struct EdgeContext<'a> {
    maps: &'a LiftMaps,
    owner: Option<StatementId>,
}

impl EdgeContext<'_> {
    /// The RTL statement owning the edge: the unique throwing statement
    /// for an exceptional edge, the block's terminator otherwise, `None`
    /// for a plain fallthrough out of an unterminated block.
    #[must_use]
    pub const fn owner(&self) -> Option<StatementId> {
        self.owner
    }

    /// The MLIL instructions one statement emitted, in order.
    #[must_use]
    pub fn instructions(&self, statement: StatementId) -> &[InstructionId] {
        self.maps.instructions(statement)
    }

    /// The emitted throw site of one statement.
    #[must_use]
    pub fn throw_site(&self, statement: StatementId) -> Option<InstructionId> {
        self.maps.throw_site(statement)
    }

    /// The MLIL block one RTL block became.
    #[must_use]
    pub fn block(&self, block: BlockId) -> Option<BlockId> {
        self.maps.block(block)
    }
}

struct Resolver<'a, D: Dialect> {
    ids: &'a BTreeMap<(Lane<D>, usize), usize>,
    roots: &'a [usize],
    web_of_root: &'a BTreeMap<usize, usize>,
}

impl<D: Dialect> Resolver<'_, D> {
    /// Resolves one SSA value to its web index.
    fn web(&self, value: &SsaValue<Lane<D>>) -> Result<usize> {
        let key = (value.variable.clone(), value.version);
        let id = self
            .ids
            .get(&key)
            .ok_or_else(|| Error::Lifting("unregistered SSA value".into()))?;
        self.web_of_root
            .get(&self.roots[*id])
            .copied()
            .ok_or_else(|| Error::Lifting("web lost its variable".into()))
    }
}

struct Emitter<'a, D: Lift> {
    builder: MlilBuilder<D>,
    webs: Vec<WebInfo<D>>,
    web_index: BTreeMap<VariableId, usize>,
    resolver: Resolver<'a, D>,
    maps: LiftMaps,
    current: Option<StatementId>,
    native_defined: bool,
}

/// One rebuilt assignment awaiting serialization.
struct PendingAssign<D: Dialect> {
    target: usize,
    positions: Vec<u8>,
    value: VarExpr<D>,
    reads: Vec<usize>,
}

/// The context one [`Lift::emit`] call appends MLIL instructions
/// through.
///
/// It validates the dialect's expansion: every use must be a variable
/// the statement made available (its reads, its assignment target, a
/// context temporary, or an earlier definition of the same statement),
/// exactly one appended instruction may carry the statement's exceptional
/// behavior, and no native web may be defined before that throw site.
pub struct Emission<'a, 'b, D: Lift> {
    emitter: &'a mut Emitter<'b, D>,
    block: BlockId,
    statement: StatementId,
    reads: Vec<TypedVariable<D>>,
    target: Option<TypedVariable<D>>,
    merge: bool,
    may_throw: bool,
    span: Option<<D as Vocabulary>::SourceSpan>,
    allowed: BTreeSet<VariableId>,
    appended_throwing: bool,
}

impl<D: Lift> Emission<'_, '_, D> {
    /// The RTL statement being emitted.
    #[must_use]
    pub const fn statement(&self) -> StatementId {
        self.statement
    }

    /// The typed web variables the statement's reads consume, in
    /// pre-order — the instruction `uses` of the one-operation form.
    #[must_use]
    pub fn reads(&self) -> &[TypedVariable<D>] {
        &self.reads
    }

    /// The typed web variable an assignment defines.
    #[must_use]
    pub const fn target(&self) -> Option<&TypedVariable<D>> {
        self.target.as_ref()
    }

    /// Whether the statement can transfer exceptionally — exactly one
    /// appended instruction must then carry `may_throw`.
    #[must_use]
    pub const fn may_throw(&self) -> bool {
        self.may_throw
    }

    /// Declares one dialect temporary for a multi-instruction expansion.
    ///
    /// # Errors
    ///
    /// Returns an error when MLIL declaration fails.
    pub fn temporary(
        &mut self,
        role: <D as Vocabulary>::VariableRole,
        value_type: <D as Vocabulary>::ValueType,
    ) -> Result<TypedVariable<D>> {
        let variable = self
            .emitter
            .builder
            .declare_variable(role, None)
            .map_err(|error| Error::Lifting(error.to_string()))?;
        self.allowed.insert(variable);
        Ok(TypedVariable::new(variable, value_type))
    }

    /// Appends one MLIL instruction for the statement.
    ///
    /// # Errors
    ///
    /// Returns an error when a use references a variable the statement
    /// did not make available, when a second instruction claims the
    /// statement's throw site, or when a native web was already defined
    /// before a throwing append.
    pub fn append(
        &mut self,
        operation: <D as crate::ir::mlil::Dialect>::Operation,
        uses: Vec<TypedVariable<D>>,
        defs: Vec<TypedVariable<D>>,
        may_throw: bool,
    ) -> Result<InstructionId> {
        for used in &uses {
            if !self.allowed.contains(&used.variable) {
                return Err(Error::Lifting(format!(
                    "emission uses a variable outside statement {}'s reads",
                    self.statement.raw()
                )));
            }
        }
        if may_throw {
            if self.emitter.maps.throw_sites[self.statement.index()].is_some() {
                return Err(Error::Lifting(format!(
                    "statement {} designated two throw sites",
                    self.statement.raw()
                )));
            }
            if self.emitter.native_defined {
                return Err(Error::Lifting(format!(
                    "statement {} committed native state before its throw site",
                    self.statement.raw()
                )));
            }
        }
        for defined in &defs {
            self.allowed.insert(defined.variable);
            if self
                .emitter
                .web_index
                .get(&defined.variable)
                .is_some_and(|&web| self.emitter.webs[web].storage.is_some())
            {
                self.emitter.native_defined = true;
            }
        }
        let id = self
            .emitter
            .builder
            .append_instruction(
                self.block,
                operation,
                uses,
                defs,
                may_throw,
                self.span.clone(),
            )
            .map_err(|error| Error::Lifting(error.to_string()))?;
        self.emitter.maps.instructions[self.statement.index()].push(id);
        if may_throw {
            self.emitter.maps.throw_sites[self.statement.index()] = Some(id);
            self.appended_throwing = true;
        }
        Ok(id)
    }

    /// Appends the one-operation form: uses are the statement's reads
    /// (plus the merging assignment's trailing target), the definition
    /// is the assignment target, and the statement's exceptional
    /// behavior lands on this instruction.
    ///
    /// # Errors
    ///
    /// Returns an error when MLIL construction fails.
    pub fn single(
        &mut self,
        operation: <D as crate::ir::mlil::Dialect>::Operation,
    ) -> Result<InstructionId> {
        let mut uses = self.reads.clone();
        if self.merge
            && let Some(target) = &self.target
        {
            uses.push(target.clone());
        }
        let defs: Vec<TypedVariable<D>> = self.target.iter().cloned().collect();
        let may_throw = self.may_throw;
        self.append(operation, uses, defs, may_throw)
    }
}

impl<D: Lift> Emitter<'_, D> {
    fn position(&self, web: usize, lane: u8) -> Result<u8> {
        let index = self.webs[web]
            .lanes
            .binary_search(&lane)
            .map_err(|_| Error::Lifting(format!("lane {lane} escaped its web")))?;
        Ok(u8::try_from(index).unwrap_or(u8::MAX))
    }

    fn typed(&self, web: usize) -> TypedVariable<D> {
        TypedVariable::new(
            self.webs[web].variable,
            D::value_type(self.webs[web].shape.clone()),
        )
    }

    /// Declares one synthetic temporary web of the given shape.
    fn declare_temporary(&mut self, shape: Shape<D::Constraint>) -> Result<usize> {
        let variable = self
            .builder
            .declare_variable(D::web_role(None), None)
            .map_err(|error| Error::Lifting(error.to_string()))?;
        let index = self.webs.len();
        self.webs.push(WebInfo {
            variable,
            storage: None,
            lanes: (0..shape.lanes).collect(),
            shape,
            live_in: false,
        });
        self.web_index.insert(variable, index);
        Ok(index)
    }

    /// Rebuilds one RTL expression over web positions, consuming SSA
    /// uses in the shared deterministic read order and appending each
    /// read's web (in pre-order) to `reads` — the instruction's use
    /// list in the making.
    fn rebuild(
        &self,
        expr: &Expr<D>,
        uses: &[SsaValue<Lane<D>>],
        cursor: &mut usize,
        reads: &mut Vec<usize>,
    ) -> Result<VarExpr<D>> {
        match expr {
            Expr::Read { lanes, scalar, .. } => {
                let mut mapped: Vec<(usize, u8)> = Vec::with_capacity(lanes.len());
                for _ in lanes {
                    let value = uses
                        .get(*cursor)
                        .ok_or_else(|| Error::Lifting("SSA lost a use".into()))?;
                    *cursor += 1;
                    let web = self.resolver.web(value)?;
                    mapped.push((web, self.position(web, value.variable.1)?));
                }
                let mut parts: Vec<VarExpr<D>> = Vec::new();
                let mut part_webs: Vec<usize> = Vec::new();
                for (web, position) in mapped {
                    match (parts.last_mut(), part_webs.last()) {
                        (Some(VarExpr::Read { positions, .. }), Some(&last)) if last == web => {
                            positions.push(position);
                        }
                        _ => {
                            parts.push(VarExpr::Read {
                                positions: alloc::vec![position],
                                scalar: scalar.clone(),
                            });
                            part_webs.push(web);
                        }
                    }
                }
                reads.extend(part_webs);
                if parts.len() == 1 {
                    Ok(parts.remove(0))
                } else {
                    let shape = Shape {
                        scalar: scalar.clone(),
                        lanes: u8::try_from(lanes.len()).unwrap_or(u8::MAX),
                    };
                    Ok(VarExpr::Compose { parts, shape })
                }
            }
            Expr::Const { bits, shape } => Ok(VarExpr::Const {
                bits: bits.clone(),
                shape: shape.clone(),
            }),
            Expr::Apply {
                operator,
                operands,
                shape,
            } => {
                let operands = operands
                    .iter()
                    .map(|operand| self.rebuild(operand, uses, cursor, reads))
                    .collect::<Result<Vec<_>>>()?;
                Ok(VarExpr::Apply {
                    operator: operator.clone(),
                    operands,
                    shape: shape.clone(),
                })
            }
            Expr::Reinterpret { operand, shape } => Ok(VarExpr::Reinterpret {
                operand: Box::new(self.rebuild(operand, uses, cursor, reads)?),
                shape: shape.clone(),
            }),
        }
    }

    /// Hands one serialized statement to the dialect's emission hook.
    #[expect(clippy::too_many_arguments, reason = "one slot per statement facet")]
    fn hand_off(
        &mut self,
        block: BlockId,
        id: StatementId,
        statement: LiftedStatement<D>,
        reads: &[usize],
        target: Option<usize>,
        merge: bool,
        may_throw: bool,
        span: Option<<D as Vocabulary>::SourceSpan>,
    ) -> Result<()> {
        if self.current != Some(id) {
            self.current = Some(id);
            self.native_defined = false;
        }
        let reads: Vec<TypedVariable<D>> = reads.iter().map(|&web| self.typed(web)).collect();
        let target = target.map(|web| self.typed(web));
        let mut allowed: BTreeSet<VariableId> =
            reads.iter().map(|typed| typed.variable).collect();
        if let Some(target) = &target {
            allowed.insert(target.variable);
        }
        let mut emission = Emission {
            emitter: self,
            block,
            statement: id,
            reads,
            target,
            merge,
            may_throw,
            span,
            allowed,
            appended_throwing: false,
        };
        D::emit(&mut emission, statement)?;
        if may_throw && !emission.appended_throwing {
            return Err(Error::Lifting(format!(
                "the emission dropped statement {}'s exceptional behavior",
                id.raw()
            )));
        }
        Ok(())
    }

    /// Lowers one return value: a whole identity read of a web passes
    /// through, anything else materializes into a temporary so every
    /// returned value is exactly one instruction use.
    fn returnable(
        &mut self,
        block: BlockId,
        id: StatementId,
        value: VarExpr<D>,
        reads: &[usize],
        span: Option<<D as Vocabulary>::SourceSpan>,
    ) -> Result<(VarExpr<D>, usize)> {
        if let VarExpr::Read { positions, scalar } = &value
            && let [web] = reads
        {
            let info = &self.webs[*web];
            let whole = positions.len() == usize::from(info.shape.lanes)
                && positions
                    .iter()
                    .enumerate()
                    .all(|(index, &position)| usize::from(position) == index);
            if whole && scalar == &info.shape.scalar {
                return Ok((value, *web));
            }
        }
        let shape = value.shape();
        let temporary = self.declare_temporary(shape.clone())?;
        let all: Vec<u8> = (0..shape.lanes).collect();
        self.hand_off(
            block,
            id,
            LiftedStatement::Assign {
                positions: all.clone(),
                width: shape.lanes,
                merges: false,
                value,
                effects: Vec::new(),
            },
            reads,
            Some(temporary),
            false,
            false,
            span,
        )?;
        Ok((
            VarExpr::Read {
                positions: all,
                scalar: shape.scalar,
            },
            temporary,
        ))
    }

    /// Serializes one parallel transfer: rebuild every assignment
    /// against pre-statement state, pre-copy hazarded targets, then
    /// emit one MLIL assignment per destination.
    #[expect(clippy::too_many_arguments, reason = "one slot per statement facet")]
    fn transfer(
        &mut self,
        block: BlockId,
        id: StatementId,
        assignments: &[(Place<D>, Expr<D>)],
        effects: &[<D as Vocabulary>::Effect],
        may_throw: bool,
        span: Option<&<D as Vocabulary>::SourceSpan>,
        annotation: &crate::SsaInstruction<Lane<D>>,
    ) -> Result<()> {
        let mut use_cursor = 0usize;
        let mut def_cursor = 0usize;
        let mut lifted: Vec<PendingAssign<D>> = Vec::new();
        for (place, value) in assignments {
            let mut reads = Vec::new();
            let value = self.rebuild(value, &annotation.uses, &mut use_cursor, &mut reads)?;
            let target = annotation
                .defs
                .get(def_cursor)
                .ok_or_else(|| Error::Lifting("SSA lost a definition".into()))?;
            let target = self.resolver.web(target)?;
            let mut positions = Vec::with_capacity(place.lanes.len());
            for offset in 0..place.lanes.len() {
                let def = &annotation.defs[def_cursor + offset];
                positions.push(self.position(target, def.variable.1)?);
            }
            def_cursor += place.lanes.len();
            lifted.push(PendingAssign {
                target,
                positions,
                value,
                reads,
            });
        }
        // Serialize: a target web read by any sibling assignment keeps
        // its pre-state through a synthetic copy — the sibling reads
        // then reference the copy instead.
        if lifted.len() > 1 {
            let targets: Vec<usize> = lifted.iter().map(|pending| pending.target).collect();
            let mut hazards: BTreeSet<usize> = BTreeSet::new();
            for &target in &targets {
                if lifted.iter().any(|pending| pending.reads.contains(&target)) {
                    hazards.insert(target);
                }
            }
            for hazard in hazards {
                let shape = self.webs[hazard].shape.clone();
                let temporary = self.declare_temporary(shape.clone())?;
                let all: Vec<u8> = (0..shape.lanes).collect();
                self.hand_off(
                    block,
                    id,
                    LiftedStatement::Assign {
                        positions: all.clone(),
                        width: shape.lanes,
                        merges: false,
                        value: VarExpr::Read {
                            positions: all,
                            scalar: shape.scalar,
                        },
                        effects: Vec::new(),
                    },
                    &[hazard],
                    Some(temporary),
                    false,
                    false,
                    span.cloned(),
                )?;
                for pending in &mut lifted {
                    for web in &mut pending.reads {
                        if *web == hazard {
                            *web = temporary;
                        }
                    }
                }
            }
        }
        let mut first = true;
        for pending in lifted {
            let width = self.webs[pending.target].shape.lanes;
            let merges = pending.positions.len() < usize::from(width);
            let statement_effects = if first { effects.to_vec() } else { Vec::new() };
            let throws = first && may_throw;
            first = false;
            self.hand_off(
                block,
                id,
                LiftedStatement::Assign {
                    positions: pending.positions,
                    width,
                    merges,
                    value: pending.value,
                    effects: statement_effects,
                },
                &pending.reads,
                Some(pending.target),
                merges,
                throws,
                span.cloned(),
            )?;
        }
        Ok(())
    }

    #[expect(clippy::too_many_lines, reason = "one arm per statement form")]
    fn statement(
        &mut self,
        block: BlockId,
        id: StatementId,
        statement: &Statement<D>,
        span: Option<<D as Vocabulary>::SourceSpan>,
        annotation: &crate::SsaInstruction<Lane<D>>,
    ) -> Result<()> {
        match statement {
            Statement::Transfer {
                assignments,
                effects,
                may_throw,
            } => self.transfer(
                block,
                id,
                assignments,
                effects,
                *may_throw,
                span.as_ref(),
                annotation,
            ),
            Statement::Effect {
                operation,
                operands,
                effects,
                may_throw,
            } => {
                let mut cursor = 0usize;
                let mut reads = Vec::new();
                let operands = operands
                    .iter()
                    .map(|operand| self.rebuild(operand, &annotation.uses, &mut cursor, &mut reads))
                    .collect::<Result<Vec<_>>>()?;
                self.hand_off(
                    block,
                    id,
                    LiftedStatement::Effect {
                        operation: operation.clone(),
                        operands,
                        effects: effects.clone(),
                    },
                    &reads,
                    None,
                    false,
                    *may_throw,
                    span,
                )
            }
            Statement::Branch { condition } => {
                let mut cursor = 0usize;
                let mut reads = Vec::new();
                let condition =
                    self.rebuild(condition, &annotation.uses, &mut cursor, &mut reads)?;
                self.hand_off(
                    block,
                    id,
                    LiftedStatement::Branch { condition },
                    &reads,
                    None,
                    false,
                    false,
                    span,
                )
            }
            Statement::Dispatch { scrutinee } => {
                let mut cursor = 0usize;
                let mut reads = Vec::new();
                let scrutinee =
                    self.rebuild(scrutinee, &annotation.uses, &mut cursor, &mut reads)?;
                self.hand_off(
                    block,
                    id,
                    LiftedStatement::Dispatch { scrutinee },
                    &reads,
                    None,
                    false,
                    false,
                    span,
                )
            }
            Statement::Return { values } => {
                let mut cursor = 0usize;
                let mut lowered = Vec::with_capacity(values.len());
                let mut return_reads = Vec::with_capacity(values.len());
                for value in values {
                    let mut reads = Vec::new();
                    let rebuilt =
                        self.rebuild(value, &annotation.uses, &mut cursor, &mut reads)?;
                    let (value, web) =
                        self.returnable(block, id, rebuilt, &reads, span.clone())?;
                    lowered.push(value);
                    return_reads.push(web);
                }
                self.hand_off(
                    block,
                    id,
                    LiftedStatement::Return { values: lowered },
                    &return_reads,
                    None,
                    false,
                    false,
                    span,
                )
            }
            Statement::Raise {
                operation,
                operands,
                effects,
            } => {
                let mut cursor = 0usize;
                let mut reads = Vec::new();
                let operands = operands
                    .iter()
                    .map(|operand| self.rebuild(operand, &annotation.uses, &mut cursor, &mut reads))
                    .collect::<Result<Vec<_>>>()?;
                self.hand_off(
                    block,
                    id,
                    LiftedStatement::Raise {
                        operation: operation.clone(),
                        operands,
                        effects: effects.clone(),
                    },
                    &reads,
                    None,
                    false,
                    true,
                    span,
                )
            }
        }
    }
}
