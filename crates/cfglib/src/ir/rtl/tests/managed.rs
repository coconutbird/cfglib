//! A managed-language test dialect: verifier-style reference
//! constraints over a class hierarchy, exceptional edges carrying exact
//! throw sites, dispatch, semantic multi-instruction emission, and the
//! MLIL → RTL lowering direction.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::EdgeKind;
use crate::ir::dialect::Vocabulary;
use crate::ir::hlil::{self, Lifted, StatementKind, lift_function as lift_hlil};
use crate::ir::mlil::{self, InstructionId, InstructionMetadata, TypedVariable};

use super::super::{
    Constraint, Dialect, EdgeContext, Emission, Expr, FunctionBuilder, Lift, LiftedStatement,
    Lower, LowerContext, Place, Result, ScalarType, Shape, Statement, VarExpr, lift, lower,
};

/// Verifier-style lane constraints over a two-level class hierarchy:
/// class 0 is the root, classes 1 and 2 extend it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum JvmConstraint {
    /// A numeric lane, delegating to the provided scalar domain.
    Word(ScalarType),
    /// An exact reference type.
    Reference(u8),
    /// The null literal — merges into any reference.
    Null,
    /// No constraint yet.
    Unknown,
    /// Irreconcilable observations.
    Conflict,
}

fn superclass(a: u8, b: u8) -> u8 {
    if a == b { a } else { 0 }
}

impl Constraint for JvmConstraint {
    fn free() -> Self {
        Self::Unknown
    }

    fn conflicted() -> Self {
        Self::Conflict
    }

    fn merge(&self, other: &Self) -> Option<Self> {
        match (self, other) {
            (a, b) if a == b => Some(*a),
            (Self::Word(a), Self::Word(b)) => Constraint::merge(a, b).map(Self::Word),
            (Self::Reference(a), Self::Reference(b)) => {
                Some(Self::Reference(superclass(*a, *b)))
            }
            (Self::Reference(class), Self::Null) | (Self::Null, Self::Reference(class)) => {
                Some(Self::Reference(*class))
            }
            _ => None,
        }
    }

    fn width(&self) -> Option<u32> {
        match self {
            Self::Word(scalar) => Constraint::width(scalar),
            _ => None,
        }
    }
}

type JvmShape = Shape<JvmConstraint>;

/// Exact caller-owned edge metadata, including exceptional edges that
/// carry their emitted throw site after the lift.
#[derive(Debug, Clone, PartialEq, Eq)]
enum JvmEdge {
    Entry,
    Fall,
    True,
    False,
    Case(i64),
    Except { site: Option<InstructionId> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operator {
    /// Numeric addition — the managed dialect *expands* assignments of
    /// this operator into two MLIL instructions to exercise semantic
    /// emission.
    Add,
    /// An allocation of one exact class.
    New(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EffectOp {
    Invoke,
    Throw,
    /// Deliberately drops the statement's exceptional behavior — the
    /// emission validation must reject it.
    DropThrow,
    /// Deliberately references a variable outside the statement's reads.
    Smuggle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Effect {
    Call,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Span {
    start: u32,
    end: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Managed;

impl Vocabulary for Managed {
    type ValueType = JvmShape;
    type Effect = Effect;
    type Source = String;
    type SourceSpan = Span;
    type SourcePoint = u32;
    type VariableRole = u8;
    type NativeVariable = u8;

    fn span_is_empty(span: &Self::SourceSpan) -> bool {
        span.start >= span.end
    }

    fn span_contains(span: &Self::SourceSpan, point: &Self::SourcePoint) -> bool {
        span.start <= *point && *point < span.end
    }
}

impl Dialect for Managed {
    type Constraint = JvmConstraint;
    type Operator = Operator;
    type EffectOp = EffectOp;
    type Edge = JvmEdge;

    fn mnemonic(operator: &Self::Operator) -> &str {
        match operator {
            Operator::Add => "add",
            Operator::New(_) => "new",
        }
    }

    fn effect_mnemonic(operation: &Self::EffectOp) -> &str {
        match operation {
            EffectOp::Invoke => "invoke",
            EffectOp::Throw => "throw",
            EffectOp::DropThrow => "drop-throw",
            EffectOp::Smuggle => "smuggle",
        }
    }

    fn edge_kind(edge: &Self::Edge) -> EdgeKind {
        match edge {
            JvmEdge::Entry | JvmEdge::Fall => EdgeKind::Fallthrough,
            JvmEdge::True => EdgeKind::ConditionalTrue,
            JvmEdge::False => EdgeKind::ConditionalFalse,
            JvmEdge::Case(_) => EdgeKind::SwitchCase,
            JvmEdge::Except { .. } => EdgeKind::ExceptionHandler,
        }
    }

    fn is_entry_edge(edge: &Self::Edge) -> bool {
        *edge == JvmEdge::Entry
    }
}

impl mlil::Dialect for Managed {
    type Operation = LiftedStatement<Managed>;
    type Edge = JvmEdge;

    fn instruction_metadata(
        operation: &Self::Operation,
        may_throw: bool,
    ) -> InstructionMetadata<Self::Effect> {
        let effects = match operation {
            LiftedStatement::Assign { effects, .. }
            | LiftedStatement::Effect { effects, .. }
            | LiftedStatement::Raise { effects, .. } => effects.clone(),
            LiftedStatement::Branch { .. }
            | LiftedStatement::Dispatch { .. }
            | LiftedStatement::Return { .. } => Vec::new(),
        };
        InstructionMetadata::new(effects, operation.flow_effect(), may_throw)
    }

    fn mnemonic(operation: &Self::Operation) -> &str {
        operation.mnemonic()
    }

    fn edge_kind(edge: &Self::Edge) -> EdgeKind {
        <Managed as Dialect>::edge_kind(edge)
    }

    fn is_entry_edge(edge: &Self::Edge) -> bool {
        <Managed as Dialect>::is_entry_edge(edge)
    }
}

impl mlil::AnalysisDialect for Managed {
    type Constant = Vec<u64>;
    type ExpressionOperator = Operator;
    type Callee = u32;

    fn is_copy(_operation: &Self::Operation) -> bool {
        false
    }

    fn expression_operator(_operation: &Self::Operation) -> Option<Self::ExpressionOperator> {
        None
    }

    fn constant(_operation: &Self::Operation) -> Option<Self::Constant> {
        None
    }

    fn fold_constant(
        _instruction: &mlil::Instruction<Self>,
        _known: &BTreeMap<mlil::VariableId, Self::Constant>,
    ) -> Option<(mlil::VariableId, Self::Constant)> {
        None
    }

    fn callee(_operation: &Self::Operation) -> Option<Self::Callee> {
        None
    }
}

impl mlil::VerifyDialect for Managed {
    fn verify(_function: &mlil::Function<Self>, _issues: &mut Vec<mlil::VerificationIssue>) {}
}

impl hlil::Dialect for Managed {
    type Operation = LiftedStatement<Managed>;
    type Constant = Vec<u64>;

    fn mnemonic(operation: &Self::Operation) -> &str {
        operation.mnemonic()
    }
}

impl hlil::VerifyDialect for Managed {
    fn verify(_function: &hlil::Function<Self>, _issues: &mut Vec<hlil::VerificationIssue>) {}
}

impl hlil::LiftDialect for Managed {
    fn lift_operation(operation: &LiftedStatement<Self>) -> Lifted<LiftedStatement<Self>> {
        operation.lifted()
    }

    fn case_values(edge: &JvmEdge) -> Vec<Vec<u64>> {
        match edge {
            #[expect(clippy::cast_sign_loss, reason = "case value as raw pattern")]
            JvmEdge::Case(value) => vec![vec![*value as u64]],
            _ => Vec::new(),
        }
    }

    fn void_type() -> JvmShape {
        JvmShape::vector(JvmConstraint::Unknown, 0)
    }

    fn previous_value_operand(operation: &LiftedStatement<Self>) -> Option<usize> {
        operation.merge_operand()
    }
}

impl Lift for Managed {
    fn value_type(shape: JvmShape) -> JvmShape {
        shape
    }

    fn web_role(storage: Option<&u8>) -> u8 {
        u8::from(storage.is_none())
    }

    fn emit(context: &mut Emission<'_, '_, Self>, statement: LiftedStatement<Self>) -> Result<()> {
        match &statement {
            // Deliberate misbehavior markers for validation tests.
            LiftedStatement::Effect {
                operation: EffectOp::DropThrow,
                ..
            } => {
                let reads = context.reads().to_vec();
                context.append(statement.clone(), reads, Vec::new(), false)?;
                Ok(())
            }
            LiftedStatement::Effect {
                operation: EffectOp::Smuggle,
                ..
            } => {
                let foreign = TypedVariable::new(
                    mlil::VariableId::from_raw(999),
                    JvmShape::scalar(JvmConstraint::Unknown),
                );
                context.append(statement.clone(), vec![foreign], Vec::new(), false)?;
                Ok(())
            }
            // Semantic expansion: an addition computes into a dialect
            // temporary, then commits to the native target — two MLIL
            // instructions from one statement, throw site on the first.
            LiftedStatement::Assign {
                positions,
                width,
                value: VarExpr::Apply {
                    operator: Operator::Add,
                    ..
                },
                ..
            } => {
                let target = context
                    .target()
                    .expect("assignments define a target")
                    .clone();
                let temporary = context.temporary(9, target.value_type)?;
                let compute = statement.clone();
                let reads = context.reads().to_vec();
                let may_throw = context.may_throw();
                context.append(compute, reads, vec![temporary.clone()], may_throw)?;
                let commit = LiftedStatement::Assign {
                    positions: positions.clone(),
                    width: *width,
                    merges: false,
                    value: VarExpr::Read {
                        positions: (0..*width).collect(),
                        scalar: JvmConstraint::Unknown,
                    },
                    effects: Vec::new(),
                };
                context.append(commit, vec![temporary], vec![target], false)?;
                Ok(())
            }
            _ => {
                context.single(statement)?;
                Ok(())
            }
        }
    }

    fn lift_edge(edge: &JvmEdge, context: &EdgeContext<'_>) -> JvmEdge {
        match edge {
            JvmEdge::Except { .. } => JvmEdge::Except {
                site: context
                    .owner()
                    .and_then(|statement| context.throw_site(statement)),
            },
            other => other.clone(),
        }
    }
}

impl Lower for Managed {
    fn place(
        _function: &mlil::Function<Self>,
        variable: mlil::VariableId,
    ) -> Result<Place<Self>> {
        Ok(Place {
            storage: u8::try_from(variable.raw()).unwrap_or(u8::MAX),
            lanes: vec![0],
        })
    }

    fn lower_instruction(
        context: &mut LowerContext<'_, Self>,
        instruction: &mlil::Instruction<Self>,
    ) -> Result<()> {
        let reads = |context: &LowerContext<'_, Self>| -> Result<Vec<Expr<Self>>> {
            instruction
                .uses()
                .iter()
                .map(|&variable| context.read(variable, JvmConstraint::Unknown))
                .collect()
        };
        match instruction.operation() {
            LiftedStatement::Assign { .. } => {
                let target = context.place(instruction.defs()[0])?.clone();
                let operands = reads(context)?;
                let value = if operands.is_empty() {
                    Expr::Const {
                        bits: vec![0],
                        shape: JvmShape::scalar(JvmConstraint::Word(ScalarType::I32)),
                    }
                } else {
                    Expr::Apply {
                        operator: Operator::Add,
                        operands,
                        shape: JvmShape::scalar(JvmConstraint::Unknown),
                    }
                };
                context.emit(Statement::Transfer {
                    assignments: vec![(target, value)],
                    effects: Vec::new(),
                    may_throw: instruction.may_throw(),
                })?;
                Ok(())
            }
            LiftedStatement::Effect { operation, .. } => {
                let operands = reads(context)?;
                context.emit(Statement::Effect {
                    operation: *operation,
                    operands,
                    effects: instruction.effects().to_vec(),
                    may_throw: instruction.may_throw(),
                })?;
                Ok(())
            }
            LiftedStatement::Branch { .. } => {
                let mut operands = reads(context)?;
                let condition = operands
                    .pop()
                    .ok_or_else(|| super::super::Error::Lowering("branch without a condition".into()))?;
                context.emit(Statement::Branch { condition })?;
                Ok(())
            }
            LiftedStatement::Dispatch { .. } => {
                let mut operands = reads(context)?;
                let scrutinee = operands.pop().ok_or_else(|| {
                    super::super::Error::Lowering("dispatch without a scrutinee".into())
                })?;
                context.emit(Statement::Dispatch { scrutinee })?;
                Ok(())
            }
            LiftedStatement::Return { .. } => {
                let values = reads(context)?;
                context.emit(Statement::Return { values })?;
                Ok(())
            }
            LiftedStatement::Raise { .. } => {
                let operands = reads(context)?;
                context.emit(Statement::Raise {
                    operation: EffectOp::Throw,
                    operands,
                    effects: instruction.effects().to_vec(),
                })?;
                Ok(())
            }
        }
    }
}

fn slot_read(storage: u8, constraint: JvmConstraint) -> Expr<Managed> {
    Expr::Read {
        storage,
        lanes: vec![0],
        scalar: constraint,
    }
}

fn slot_write(storage: u8, value: Expr<Managed>) -> Statement<Managed> {
    Statement::Transfer {
        assignments: vec![(
            Place {
                storage,
                lanes: vec![0],
            },
            value,
        )],
        effects: Vec::new(),
        may_throw: false,
    }
}

fn allocate(class: u8) -> Expr<Managed> {
    Expr::Apply {
        operator: Operator::New(class),
        operands: Vec::new(),
        shape: JvmShape::scalar(JvmConstraint::Reference(class)),
    }
}

fn word_const(value: u64) -> Expr<Managed> {
    Expr::Const {
        bits: vec![value],
        shape: JvmShape::scalar(JvmConstraint::Word(ScalarType::I32)),
    }
}

/// Reference lifetimes merge through the class hierarchy, null yields to
/// references, and a word/reference mix is a conflict.
#[test]
fn reference_constraints_merge_through_the_hierarchy() {
    let mut builder = FunctionBuilder::<Managed>::new("test".into());
    let entry = builder.entry();
    let head = builder.new_block("head");
    let then = builder.new_block("then");
    let other = builder.new_block("else");
    let join = builder.new_block("join");
    builder.add_edge(entry, head, JvmEdge::Entry).unwrap();
    builder.add_edge(head, then, JvmEdge::True).unwrap();
    builder.add_edge(head, other, JvmEdge::False).unwrap();
    builder.add_edge(then, join, JvmEdge::Fall).unwrap();
    builder.add_edge(other, join, JvmEdge::Fall).unwrap();
    builder
        .append(
            head,
            Statement::Branch {
                condition: slot_read(9, JvmConstraint::Word(ScalarType::I32)),
            },
            None,
        )
        .unwrap();
    // Slot 0: new A(1) vs new B(2) — merges to the common superclass 0.
    builder.append(then, slot_write(0, allocate(1)), None).unwrap();
    builder.append(other, slot_write(0, allocate(2)), None).unwrap();
    // Slot 1: new A(1) vs null — null yields, the reference survives.
    builder.append(then, slot_write(1, allocate(1)), None).unwrap();
    builder
        .append(
            other,
            slot_write(
                1,
                Expr::Const {
                    bits: vec![0],
                    shape: JvmShape::scalar(JvmConstraint::Null),
                },
            ),
            None,
        )
        .unwrap();
    // Slot 2: an integer vs a reference — a genuine conflict.
    builder.append(then, slot_write(2, word_const(7)), None).unwrap();
    builder.append(other, slot_write(2, allocate(1)), None).unwrap();
    for slot in 0..3u8 {
        builder
            .append(
                join,
                slot_write(10 + slot, slot_read(slot, JvmConstraint::Unknown)),
                None,
            )
            .unwrap();
    }
    builder
        .append(join, Statement::Return { values: Vec::new() }, None)
        .unwrap();
    let function = builder.finish().unwrap();

    let lifting = lift(&function).unwrap();
    let constraint = |storage: u8| {
        lifting
            .webs
            .iter()
            .find(|web| web.storage == Some(storage))
            .expect("slot web")
            .shape
            .scalar
    };
    assert_eq!(constraint(0), JvmConstraint::Reference(0));
    assert_eq!(constraint(1), JvmConstraint::Reference(1));
    assert_eq!(constraint(2), JvmConstraint::Conflict);
    lifting.builder.finish().unwrap();
}

/// A dispatch statement lifts through MLIL into a structured switch.
#[test]
fn dispatch_lifts_to_a_structured_switch() {
    let mut builder = FunctionBuilder::<Managed>::new("test".into());
    let entry = builder.entry();
    let head = builder.new_block("head");
    let one = builder.new_block("one");
    let two = builder.new_block("two");
    let fallback = builder.new_block("fallback");
    let merge = builder.new_block("merge");
    builder.add_edge(entry, head, JvmEdge::Entry).unwrap();
    builder.add_edge(head, one, JvmEdge::Case(1)).unwrap();
    builder.add_edge(head, two, JvmEdge::Case(2)).unwrap();
    builder.add_edge(head, fallback, JvmEdge::Fall).unwrap();
    builder.add_edge(one, merge, JvmEdge::Fall).unwrap();
    builder.add_edge(two, merge, JvmEdge::Fall).unwrap();
    builder.add_edge(fallback, merge, JvmEdge::Fall).unwrap();
    builder
        .append(
            head,
            Statement::Dispatch {
                scrutinee: slot_read(9, JvmConstraint::Word(ScalarType::I32)),
            },
            None,
        )
        .unwrap();
    for block in [one, two, fallback] {
        builder
            .append(
                block,
                Statement::Effect {
                    operation: EffectOp::Invoke,
                    operands: Vec::new(),
                    effects: vec![Effect::Call],
                    may_throw: false,
                },
                None,
            )
            .unwrap();
    }
    builder
        .append(merge, Statement::Return { values: Vec::new() }, None)
        .unwrap();
    let function = builder.finish().unwrap();

    let lifting = lift(&function).unwrap();
    let function = lifting.builder.finish().unwrap();
    let lifted = lift_hlil(&function).unwrap();
    assert!(lifted.report.is_fully_structured(), "{:?}", lifted.report);
    assert!(
        lifted
            .function
            .statements()
            .iter()
            .any(|statement| matches!(statement.kind(), StatementKind::Switch { .. })),
        "the dispatch structures as a switch"
    );
}

/// A throwing invoke expands into two MLIL instructions, the first owns
/// the exceptional behavior, and the lifted exceptional edge's payload
/// carries that exact emitted throw site.
#[test]
fn exceptional_edge_carries_the_emitted_throw_site() {
    let mut builder = FunctionBuilder::<Managed>::new("test".into());
    let entry = builder.entry();
    let body = builder.new_block("body");
    let exit = builder.new_block("exit");
    let handler = builder.new_block("handler");
    builder.add_edge(entry, body, JvmEdge::Entry).unwrap();
    builder.add_edge(body, exit, JvmEdge::Fall).unwrap();
    builder
        .add_edge(body, handler, JvmEdge::Except { site: None })
        .unwrap();
    // An invoke-shaped throwing assignment whose Add value triggers the
    // dialect's two-instruction expansion.
    let invoke = builder
        .append(
            body,
            Statement::Transfer {
                assignments: vec![(
                    Place {
                        storage: 0,
                        lanes: vec![0],
                    },
                    Expr::Apply {
                        operator: Operator::Add,
                        operands: vec![word_const(1), word_const(2)],
                        shape: JvmShape::scalar(JvmConstraint::Word(ScalarType::I32)),
                    },
                )],
                effects: vec![Effect::Call],
                may_throw: true,
            },
            None,
        )
        .unwrap();
    builder
        .append(exit, Statement::Return { values: Vec::new() }, None)
        .unwrap();
    builder
        .append(
            handler,
            Statement::Raise {
                operation: EffectOp::Throw,
                operands: Vec::new(),
                effects: vec![Effect::Call],
            },
            None,
        )
        .unwrap();
    let function = builder.finish().unwrap();

    let lifting = lift(&function).unwrap();
    let emitted = lifting.maps.instructions(invoke);
    assert_eq!(emitted.len(), 2, "the addition expands into two instructions");
    let site = lifting.maps.throw_site(invoke).expect("a designated throw site");
    assert_eq!(site, emitted[0], "the first instruction owns the throw");

    let function = lifting.builder.finish().unwrap();
    let payload = function
        .cfg()
        .edges()
        .find_map(|edge| match edge.payload() {
            JvmEdge::Except { site } => Some(*site),
            _ => None,
        })
        .expect("the exceptional edge survives the lift");
    assert_eq!(payload, Some(site), "the edge payload names the throw site");
    let instruction = function.instruction(site).expect("the throw site exists");
    assert!(instruction.may_throw());
}

/// An emission that ignores the statement's exceptional behavior is
/// rejected instead of silently dropping it.
#[test]
fn dropped_exceptional_behavior_is_rejected() {
    let mut builder = FunctionBuilder::<Managed>::new("test".into());
    let entry = builder.entry();
    let body = builder.new_block("body");
    builder.add_edge(entry, body, JvmEdge::Entry).unwrap();
    builder
        .append(
            body,
            Statement::Effect {
                operation: EffectOp::DropThrow,
                operands: Vec::new(),
                effects: vec![Effect::Call],
                may_throw: true,
            },
            None,
        )
        .unwrap();
    builder
        .append(body, Statement::Return { values: Vec::new() }, None)
        .unwrap();
    let function = builder.finish().unwrap();
    assert!(lift(&function).is_err(), "the dropped throw must be caught");
}

/// An emission that references a variable outside the statement's reads
/// is rejected — read/definition alignment survives consumer expansion.
#[test]
fn foreign_operand_is_rejected() {
    let mut builder = FunctionBuilder::<Managed>::new("test".into());
    let entry = builder.entry();
    let body = builder.new_block("body");
    builder.add_edge(entry, body, JvmEdge::Entry).unwrap();
    builder
        .append(
            body,
            Statement::Effect {
                operation: EffectOp::Smuggle,
                operands: Vec::new(),
                effects: Vec::new(),
                may_throw: false,
            },
            None,
        )
        .unwrap();
    builder
        .append(body, Statement::Return { values: Vec::new() }, None)
        .unwrap();
    let function = builder.finish().unwrap();
    assert!(lift(&function).is_err(), "the foreign operand must be caught");
}

/// A block with exceptional edges must contain exactly one throwing
/// statement to own them.
#[test]
fn exceptional_edges_need_one_owning_statement() {
    let mut builder = FunctionBuilder::<Managed>::new("test".into());
    let entry = builder.entry();
    let body = builder.new_block("body");
    let exit = builder.new_block("exit");
    let handler = builder.new_block("handler");
    builder.add_edge(entry, body, JvmEdge::Entry).unwrap();
    builder.add_edge(body, exit, JvmEdge::Fall).unwrap();
    builder
        .add_edge(body, handler, JvmEdge::Except { site: None })
        .unwrap();
    for _ in 0..2 {
        builder
            .append(
                body,
                Statement::Effect {
                    operation: EffectOp::Invoke,
                    operands: Vec::new(),
                    effects: vec![Effect::Call],
                    may_throw: true,
                },
                None,
            )
            .unwrap();
    }
    builder
        .append(exit, Statement::Return { values: Vec::new() }, None)
        .unwrap();
    builder
        .append(
            handler,
            Statement::Raise {
                operation: EffectOp::Throw,
                operands: Vec::new(),
                effects: Vec::new(),
            },
            None,
        )
        .unwrap();
    assert!(
        builder.finish().is_err(),
        "two throwing statements cannot share the block's edges"
    );
}

/// A raise block must not continue normally.
#[test]
fn raise_with_normal_edge_is_rejected() {
    let mut builder = FunctionBuilder::<Managed>::new("test".into());
    let entry = builder.entry();
    let body = builder.new_block("body");
    let after = builder.new_block("after");
    builder.add_edge(entry, body, JvmEdge::Entry).unwrap();
    builder.add_edge(body, after, JvmEdge::Fall).unwrap();
    builder
        .append(
            body,
            Statement::Raise {
                operation: EffectOp::Throw,
                operands: Vec::new(),
                effects: Vec::new(),
            },
            None,
        )
        .unwrap();
    builder
        .append(after, Statement::Return { values: Vec::new() }, None)
        .unwrap();
    assert!(builder.finish().is_err());
}

/// Lowering walks an MLIL function back onto target RTL with complete
/// rewrite maps, refusing nothing on a representable function.
#[test]
fn lowering_round_trips_with_rewrite_maps() {
    // Build RTL, lift it, then lower the MLIL back down.
    let mut builder = FunctionBuilder::<Managed>::new("test".into());
    let entry = builder.entry();
    let head = builder.new_block("head");
    let then = builder.new_block("then");
    let other = builder.new_block("else");
    let join = builder.new_block("join");
    builder.add_edge(entry, head, JvmEdge::Entry).unwrap();
    builder.add_edge(head, then, JvmEdge::True).unwrap();
    builder.add_edge(head, other, JvmEdge::False).unwrap();
    builder.add_edge(then, join, JvmEdge::Fall).unwrap();
    builder.add_edge(other, join, JvmEdge::Fall).unwrap();
    builder
        .append(
            head,
            Statement::Branch {
                condition: slot_read(9, JvmConstraint::Word(ScalarType::I32)),
            },
            None,
        )
        .unwrap();
    builder.append(then, slot_write(0, word_const(1)), None).unwrap();
    builder.append(other, slot_write(0, word_const(2)), None).unwrap();
    builder
        .append(
            join,
            Statement::Return {
                values: vec![slot_read(0, JvmConstraint::Word(ScalarType::I32))],
            },
            None,
        )
        .unwrap();
    let function = builder.finish().unwrap();
    let lifting = lift(&function).unwrap();
    let mlil_function = lifting.builder.finish().unwrap();

    let lowered = lower(&mlil_function).unwrap();
    assert_eq!(
        lowered.function.cfg().block_count(),
        mlil_function.cfg().block_count(),
        "blocks mirror one-to-one"
    );
    for block in mlil_function.cfg().blocks() {
        assert!(
            lowered.block(block.id()).is_some(),
            "every block maps: {}",
            block.id()
        );
        for instruction in block.instructions() {
            assert!(
                !lowered.statements(instruction.id()).is_empty(),
                "every instruction lowers to at least one statement"
            );
        }
    }
    for edge in mlil_function.cfg().edges() {
        assert!(lowered.edge(edge.id()).is_some(), "every edge maps");
    }
}
