//! Lifting of RTL functions into typed MLIL through def-use webs.
//!
//! Storage reuse dissolves here: per-lane SSA versions unite into webs —
//! φ operands with their results, lanes co-written by one assignment with
//! each other — and every web becomes one typed MLIL variable. A register
//! reused for a float and later a counter becomes two variables with
//! honest types, so the consumer renders reinterpretations only where a
//! web genuinely mixes representations. Parallel transfers serialize
//! safely because reads reference prior versions; when a target web is
//! also read by a sibling assignment, the pre-state is copied into a
//! synthetic temporary first.

extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::ToString;
use alloc::vec::Vec;

use crate::ir::dialect::Vocabulary;
use crate::ir::mlil::{FunctionBuilder as MlilBuilder, TypedVariable, VariableId};
use crate::{BlockId, DominatorTree, SsaForm, SsaValue};

use super::dialect::{Dialect, Lift};
use super::error::{Error, Result};
use super::expr::Expr;
use super::function::Function;
use super::statement::{Lane, Statement};
use super::types::{ScalarType, ValueShape};

/// One typed expression over lifted web variables.
///
/// Consumers embed these in their MLIL operations: the tree mirrors the
/// RTL expression, with storage reads resolved to web variables and
/// their lane positions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VarExpr<D: Dialect> {
    /// A read of web-variable positions, with the interpretation the
    /// consuming operation imposes.
    Read {
        /// The web variable read.
        variable: VariableId,
        /// Positions within the web variable, in consumption order.
        positions: Vec<u8>,
        /// The interpretation the consumer imposes on each position.
        scalar: ScalarType,
    },
    /// An immediate value, one bit pattern per lane.
    Const {
        /// Raw lane bit patterns.
        bits: Vec<u64>,
        /// The shape of the constant.
        shape: ValueShape,
    },
    /// A pure operator application.
    Apply {
        /// The dialect operator.
        operator: D::Operator,
        /// Operand values in operator order.
        operands: Vec<VarExpr<D>>,
        /// The result shape.
        shape: ValueShape,
    },
    /// A bit reinterpretation to a same-width shape.
    Reinterpret {
        /// The reinterpreted value.
        operand: Box<VarExpr<D>>,
        /// The target shape.
        shape: ValueShape,
    },
    /// A vector composed from reads of several webs — one storage read
    /// whose lanes resolved to different variables.
    Compose {
        /// The composed parts in lane order.
        parts: Vec<VarExpr<D>>,
        /// The composed shape.
        shape: ValueShape,
    },
}

impl<D: Dialect> VarExpr<D> {
    /// The shape of this expression's value.
    #[must_use]
    pub fn shape(&self) -> ValueShape {
        match self {
            Self::Read {
                positions, scalar, ..
            } => ValueShape {
                scalar: *scalar,
                lanes: u8::try_from(positions.len()).unwrap_or(u8::MAX),
            },
            Self::Const { shape, .. }
            | Self::Apply { shape, .. }
            | Self::Reinterpret { shape, .. }
            | Self::Compose { shape, .. } => *shape,
        }
    }

    /// Visits every web-variable read in deterministic pre-order.
    pub fn for_each_read(&self, visit: &mut impl FnMut(VariableId)) {
        match self {
            Self::Read { variable, .. } => visit(*variable),
            Self::Const { .. } => {}
            Self::Apply { operands, .. } => {
                for operand in operands {
                    operand.for_each_read(visit);
                }
            }
            Self::Reinterpret { operand, .. } => operand.for_each_read(visit),
            Self::Compose { parts, .. } => {
                for part in parts {
                    part.for_each_read(visit);
                }
            }
        }
    }

    fn substitute(&mut self, from: VariableId, to: VariableId) {
        match self {
            Self::Read { variable, .. } => {
                if *variable == from {
                    *variable = to;
                }
            }
            Self::Const { .. } => {}
            Self::Apply { operands, .. } => {
                for operand in operands {
                    operand.substitute(from, to);
                }
            }
            Self::Reinterpret { operand, .. } => operand.substitute(from, to),
            Self::Compose { parts, .. } => {
                for part in parts {
                    part.substitute(from, to);
                }
            }
        }
    }
}

/// One lifted statement handed to the consumer's MLIL operation hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiftedStatement<D: Dialect> {
    /// One serialized assignment of a value to web-variable positions.
    Assign {
        /// The written web variable.
        target: VariableId,
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
    /// A function return carrying result values.
    Return {
        /// Returned values in signature order.
        values: Vec<VarExpr<D>>,
    },
}

/// One lifted web: a typed MLIL variable recovered from storage lanes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebInfo<D: Dialect> {
    /// The declared MLIL variable.
    pub variable: VariableId,
    /// The native storage the web lives in, or `None` for a synthetic
    /// serialization temporary.
    pub storage: Option<<D as Vocabulary>::NativeVariable>,
    /// Ascending storage lanes the web spans.
    pub lanes: Vec<u8>,
    /// The web's inferred shape.
    pub shape: ValueShape,
    /// Whether the web contains the version-zero live-in value.
    pub live_in: bool,
}

/// The result of lifting: an MLIL builder ready for signature assignment
/// and [`finish`](MlilBuilder::finish), plus the recovered webs in
/// variable-identity order.
pub struct Lifting<D: Lift> {
    /// The populated MLIL builder.
    pub builder: MlilBuilder<D>,
    /// Recovered webs, indexed by declared variable order.
    pub webs: Vec<WebInfo<D>>,
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
    scalar: ScalarType,
    live_in: bool,
}

/// Lifts one RTL function into a typed MLIL builder.
///
/// # Errors
///
/// Returns an error when SSA annotations disagree with statement
/// structure or MLIL construction fails.
#[expect(clippy::too_many_lines, reason = "one linear pass per phase")]
pub fn lift<D: Lift>(function: &Function<D>) -> Result<Lifting<D>> {
    let cfg = &function.cfg;
    let dominators = DominatorTree::compute(cfg);
    let ssa: SsaForm<Lane<D>> = SsaForm::compute(cfg, &dominators);

    // Phase 1: dense ids for every SSA value, then unite φ families and
    // co-written lanes into webs.
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
    for block in cfg.blocks() {
        let annotations = ssa.block(block.id());
        for phi in &annotations.phis {
            let result = id_of(&phi.result, &mut union);
            for (_, operand) in &phi.operands {
                let operand = id_of(operand, &mut union);
                union.unite(result, operand);
            }
        }
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
            scalar: ScalarType::Bits,
            live_in: false,
        });
        fact.lanes.insert(lane.1);
        fact.live_in |= *version == 0;
    }

    // Phase 3: unify each web's scalar type from read wants and
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
                        fact.scalar = fact.scalar.unify(scalar);
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
                        fact.scalar = fact.scalar.unify(value.shape().scalar);
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
        let shape = ValueShape {
            scalar: fact.scalar,
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

    // Phase 5: mirror blocks and edges.
    let mut block_map: Vec<BlockId> = Vec::with_capacity(cfg.block_count());
    for block in cfg.blocks() {
        if block.id() == cfg.entry() {
            block_map.push(builder.entry());
        } else {
            let label = block.label().unwrap_or("b").to_string();
            block_map.push(builder.new_block(label));
        }
    }
    for edge in cfg.edges() {
        builder
            .add_edge(
                block_map[edge.source().index()],
                block_map[edge.target().index()],
                edge.payload().clone(),
                None,
            )
            .map_err(|error| Error::Lifting(error.to_string()))?;
    }

    // Phase 6: emit one MLIL instruction per serialized statement.
    let resolver = Resolver {
        ids: &ids,
        roots: &roots,
        web_of_root: &web_of_root,
    };
    let mut emitter = Emitter {
        builder,
        webs,
        resolver,
    };
    for block in cfg.blocks() {
        let annotations = ssa.block(block.id());
        for (index, node) in block.instructions().iter().enumerate() {
            let annotation = &annotations.instructions[index];
            emitter.statement(
                block_map[block.id().index()],
                node.statement(),
                node.span().cloned(),
                annotation,
            )?;
        }
    }

    Ok(Lifting {
        builder: emitter.builder,
        webs: emitter.webs,
    })
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
    resolver: Resolver<'a, D>,
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
        TypedVariable::new(self.webs[web].variable, D::value_type(self.webs[web].shape))
    }

    /// Rebuilds one RTL expression over web variables, consuming SSA
    /// uses in the shared deterministic read order.
    fn rebuild(
        &self,
        expr: &Expr<D>,
        uses: &[SsaValue<Lane<D>>],
        cursor: &mut usize,
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
                for (web, position) in mapped {
                    match parts.last_mut() {
                        Some(VarExpr::Read {
                            variable,
                            positions,
                            ..
                        }) if *variable == self.webs[web].variable => positions.push(position),
                        _ => parts.push(VarExpr::Read {
                            variable: self.webs[web].variable,
                            positions: alloc::vec![position],
                            scalar: *scalar,
                        }),
                    }
                }
                if parts.len() == 1 {
                    Ok(parts.remove(0))
                } else {
                    let shape = ValueShape {
                        scalar: *scalar,
                        lanes: u8::try_from(lanes.len()).unwrap_or(u8::MAX),
                    };
                    Ok(VarExpr::Compose { parts, shape })
                }
            }
            Expr::Const { bits, shape } => Ok(VarExpr::Const {
                bits: bits.clone(),
                shape: *shape,
            }),
            Expr::Apply {
                operator,
                operands,
                shape,
            } => {
                let operands = operands
                    .iter()
                    .map(|operand| self.rebuild(operand, uses, cursor))
                    .collect::<Result<Vec<_>>>()?;
                Ok(VarExpr::Apply {
                    operator: operator.clone(),
                    operands,
                    shape: *shape,
                })
            }
            Expr::Reinterpret { operand, shape } => Ok(VarExpr::Reinterpret {
                operand: Box::new(self.rebuild(operand, uses, cursor)?),
                shape: *shape,
            }),
        }
    }

    /// Emits one MLIL instruction for a lifted statement.
    fn emit(
        &mut self,
        block: BlockId,
        statement: LiftedStatement<D>,
        may_throw: bool,
        span: Option<<D as Vocabulary>::SourceSpan>,
    ) -> Result<()> {
        let mut uses: Vec<TypedVariable<D>> = Vec::new();
        let mut defs: Vec<TypedVariable<D>> = Vec::new();
        let collect = |expr: &VarExpr<D>, uses: &mut Vec<TypedVariable<D>>, webs: &[WebInfo<D>]| {
            expr.for_each_read(&mut |variable| {
                let web = webs
                    .iter()
                    .position(|info| info.variable == variable)
                    .unwrap_or_default();
                uses.push(TypedVariable::new(variable, D::value_type(webs[web].shape)));
            });
        };
        match &statement {
            LiftedStatement::Assign {
                target,
                merges,
                value,
                ..
            } => {
                collect(value, &mut uses, &self.webs);
                let target_web = self
                    .webs
                    .iter()
                    .position(|info| info.variable == *target)
                    .ok_or_else(|| Error::Lifting("assignment target lost its web".into()))?;
                if *merges {
                    uses.push(self.typed(target_web));
                }
                defs.push(self.typed(target_web));
            }
            LiftedStatement::Effect { operands, .. } => {
                for operand in operands {
                    collect(operand, &mut uses, &self.webs);
                }
            }
            LiftedStatement::Branch { condition } => collect(condition, &mut uses, &self.webs),
            LiftedStatement::Return { values } => {
                for value in values {
                    collect(value, &mut uses, &self.webs);
                }
            }
        }
        let operation = D::operation(statement);
        self.builder
            .append_instruction(block, operation, uses, defs, may_throw, span)
            .map_err(|error| Error::Lifting(error.to_string()))?;
        Ok(())
    }

    #[expect(clippy::too_many_lines, reason = "one arm per statement form")]
    fn statement(
        &mut self,
        block: BlockId,
        statement: &Statement<D>,
        span: Option<<D as Vocabulary>::SourceSpan>,
        annotation: &crate::SsaInstruction<Lane<D>>,
    ) -> Result<()> {
        match statement {
            Statement::Transfer {
                assignments,
                effects,
                may_throw,
            } => {
                // Rebuild every assignment against pre-statement state.
                let mut use_cursor = 0usize;
                let mut def_cursor = 0usize;
                let mut lifted: Vec<(usize, Vec<u8>, VarExpr<D>)> = Vec::new();
                for (place, value) in assignments {
                    let rebuilt = self.rebuild(value, &annotation.uses, &mut use_cursor)?;
                    let target = annotation
                        .defs
                        .get(def_cursor)
                        .ok_or_else(|| Error::Lifting("SSA lost a definition".into()))?;
                    let target_web = self.resolver.web(target)?;
                    let mut positions = Vec::with_capacity(place.lanes.len());
                    for offset in 0..place.lanes.len() {
                        let def = &annotation.defs[def_cursor + offset];
                        positions.push(self.position(target_web, def.variable.1)?);
                    }
                    def_cursor += place.lanes.len();
                    lifted.push((target_web, positions, rebuilt));
                }
                // Serialize: a target web read by any sibling assignment
                // keeps its pre-state through a synthetic copy.
                if lifted.len() > 1 {
                    let targets: Vec<usize> = lifted.iter().map(|(target, ..)| *target).collect();
                    let mut hazards: BTreeSet<usize> = BTreeSet::new();
                    for &target in &targets {
                        let variable = self.webs[target].variable;
                        for (_, _, value) in &lifted {
                            let mut read = false;
                            value.for_each_read(&mut |used| read |= used == variable);
                            if read {
                                hazards.insert(target);
                            }
                        }
                    }
                    for hazard in hazards {
                        let shape = self.webs[hazard].shape;
                        let original = self.webs[hazard].variable;
                        let temp = self
                            .builder
                            .declare_variable(D::web_role(None), None)
                            .map_err(|error| Error::Lifting(error.to_string()))?;
                        let width = shape.lanes;
                        self.webs.push(WebInfo {
                            variable: temp,
                            storage: None,
                            lanes: (0..width).collect(),
                            shape,
                            live_in: false,
                        });
                        let all: Vec<u8> = (0..width).collect();
                        self.emit(
                            block,
                            LiftedStatement::Assign {
                                target: temp,
                                positions: all.clone(),
                                width,
                                merges: false,
                                value: VarExpr::Read {
                                    variable: original,
                                    positions: all,
                                    scalar: shape.scalar,
                                },
                                effects: Vec::new(),
                            },
                            false,
                            span.clone(),
                        )?;
                        for (_, _, value) in &mut lifted {
                            value.substitute(original, temp);
                        }
                    }
                }
                let mut first = true;
                for (target_web, positions, value) in lifted {
                    let width = self.webs[target_web].shape.lanes;
                    let merges = positions.len() < usize::from(width);
                    let statement_effects = if first { effects.clone() } else { Vec::new() };
                    let throws = if first { *may_throw } else { false };
                    first = false;
                    self.emit(
                        block,
                        LiftedStatement::Assign {
                            target: self.webs[target_web].variable,
                            positions,
                            width,
                            merges,
                            value,
                            effects: statement_effects,
                        },
                        throws,
                        span.clone(),
                    )?;
                }
                Ok(())
            }
            Statement::Effect {
                operation,
                operands,
                effects,
                may_throw,
            } => {
                let mut cursor = 0usize;
                let operands = operands
                    .iter()
                    .map(|operand| self.rebuild(operand, &annotation.uses, &mut cursor))
                    .collect::<Result<Vec<_>>>()?;
                self.emit(
                    block,
                    LiftedStatement::Effect {
                        operation: operation.clone(),
                        operands,
                        effects: effects.clone(),
                    },
                    *may_throw,
                    span,
                )
            }
            Statement::Branch { condition } => {
                let mut cursor = 0usize;
                let condition = self.rebuild(condition, &annotation.uses, &mut cursor)?;
                self.emit(block, LiftedStatement::Branch { condition }, false, span)
            }
            Statement::Return { values } => {
                let mut cursor = 0usize;
                let values = values
                    .iter()
                    .map(|value| self.rebuild(value, &annotation.uses, &mut cursor))
                    .collect::<Result<Vec<_>>>()?;
                self.emit(block, LiftedStatement::Return { values }, false, span)
            }
        }
    }
}
