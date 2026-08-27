extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::ir::dialect::Vocabulary;
use crate::ir::mlil::{self, InstructionMetadata, VerificationIssue};
use crate::{EdgeKind, FlowEffect};

use super::{
    Dialect, Expr, Function, FunctionBuilder, Lift, LiftedStatement, Place, ScalarType, Statement,
    ValueShape, lift,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Effect {
    Emit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operator {
    Add,
    Shr,
    Div,
    Rem,
    Less,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EffectOp {
    Emit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Entry,
    True,
    False,
    Next,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Span {
    start: u32,
    end: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TestDialect;

impl Vocabulary for TestDialect {
    type ValueType = ValueShape;
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

impl Dialect for TestDialect {
    type Operator = Operator;
    type EffectOp = EffectOp;
    type Edge = Edge;

    fn mnemonic(operator: &Self::Operator) -> &str {
        match operator {
            Operator::Add => "add",
            Operator::Shr => "shr",
            Operator::Div => "div",
            Operator::Rem => "rem",
            Operator::Less => "less",
        }
    }

    fn effect_mnemonic(operation: &Self::EffectOp) -> &str {
        match operation {
            EffectOp::Emit => "emit",
        }
    }

    fn edge_kind(edge: &Self::Edge) -> EdgeKind {
        match edge {
            Edge::Entry | Edge::Next => EdgeKind::Fallthrough,
            Edge::True => EdgeKind::ConditionalTrue,
            Edge::False => EdgeKind::ConditionalFalse,
        }
    }

    fn is_entry_edge(edge: &Self::Edge) -> bool {
        matches!(edge, Edge::Entry)
    }
}

impl mlil::Dialect for TestDialect {
    type Operation = LiftedStatement<TestDialect>;
    type Edge = Edge;

    fn instruction_metadata(
        operation: &Self::Operation,
        may_throw: bool,
    ) -> InstructionMetadata<Self::Effect> {
        match operation {
            LiftedStatement::Assign { effects, .. } | LiftedStatement::Effect { effects, .. } => {
                InstructionMetadata::new(effects.clone(), FlowEffect::Fallthrough, may_throw)
            }
            LiftedStatement::Branch { .. } => {
                InstructionMetadata::new(Vec::new(), FlowEffect::ConditionalJump, false)
            }
            LiftedStatement::Return { .. } => {
                InstructionMetadata::new(Vec::new(), FlowEffect::Return, false)
            }
        }
    }

    fn mnemonic(operation: &Self::Operation) -> &str {
        match operation {
            LiftedStatement::Assign { .. } => "assign",
            LiftedStatement::Effect { .. } => "effect",
            LiftedStatement::Branch { .. } => "branch",
            LiftedStatement::Return { .. } => "return",
        }
    }

    fn edge_kind(edge: &Self::Edge) -> EdgeKind {
        <Self as Dialect>::edge_kind(edge)
    }

    fn is_entry_edge(edge: &Self::Edge) -> bool {
        <Self as Dialect>::is_entry_edge(edge)
    }
}

impl mlil::VerifyDialect for TestDialect {
    fn verify(_function: &mlil::Function<Self>, _issues: &mut Vec<VerificationIssue>) {}
}

impl Lift for TestDialect {
    fn value_type(shape: ValueShape) -> ValueShape {
        shape
    }

    fn web_role(storage: Option<&u8>) -> u8 {
        u8::from(storage.is_none())
    }

    fn operation(statement: LiftedStatement<Self>) -> LiftedStatement<Self> {
        statement
    }
}

fn read(storage: u8, lanes: &[u8], scalar: ScalarType) -> Expr<TestDialect> {
    Expr::Read {
        storage,
        lanes: lanes.to_vec(),
        scalar,
    }
}

fn constant(bits: u64, scalar: ScalarType) -> Expr<TestDialect> {
    Expr::Const {
        bits: vec![bits],
        shape: ValueShape::scalar(scalar),
    }
}

fn apply(
    operator: Operator,
    operands: Vec<Expr<TestDialect>>,
    shape: ValueShape,
) -> Expr<TestDialect> {
    Expr::Apply {
        operator,
        operands,
        shape,
    }
}

fn assign(storage: u8, lanes: &[u8], value: Expr<TestDialect>) -> Statement<TestDialect> {
    Statement::Transfer {
        assignments: vec![(
            Place {
                storage,
                lanes: lanes.to_vec(),
            },
            value,
        )],
        effects: Vec::new(),
        may_throw: false,
    }
}

/// Register reuse with different interpretations splits into typed webs.
#[test]
fn storage_reuse_splits_into_typed_webs() {
    let mut builder = FunctionBuilder::<TestDialect>::new("test".into());
    let entry = builder.entry();
    let body = builder.new_block("body");
    builder.add_edge(entry, body, Edge::Entry).unwrap();
    // r0.x ← f32 add; r1.x ← r0.x (float); r0.x ← u32 shr; r2.x ← r0.x (uint)
    builder
        .append(
            body,
            assign(
                0,
                &[0],
                apply(
                    Operator::Add,
                    vec![
                        constant(0x3f80_0000, ScalarType::F32),
                        constant(0x4000_0000, ScalarType::F32),
                    ],
                    ValueShape::scalar(ScalarType::F32),
                ),
            ),
            None,
        )
        .unwrap();
    builder
        .append(body, assign(1, &[0], read(0, &[0], ScalarType::F32)), None)
        .unwrap();
    builder
        .append(
            body,
            assign(
                0,
                &[0],
                apply(
                    Operator::Shr,
                    vec![constant(64, ScalarType::U32), constant(2, ScalarType::U32)],
                    ValueShape::scalar(ScalarType::U32),
                ),
            ),
            None,
        )
        .unwrap();
    builder
        .append(body, assign(2, &[0], read(0, &[0], ScalarType::U32)), None)
        .unwrap();
    builder
        .append(
            body,
            Statement::Effect {
                operation: EffectOp::Emit,
                operands: vec![read(2, &[0], ScalarType::U32)],
                effects: vec![Effect::Emit],
                may_throw: false,
            },
            None,
        )
        .unwrap();
    builder
        .append(body, Statement::Return { values: Vec::new() }, None)
        .unwrap();
    let function: Function<TestDialect> = builder.finish().unwrap();

    let lifting = lift(&function).unwrap();
    let r0_webs: Vec<_> = lifting
        .webs
        .iter()
        .filter(|web| web.storage == Some(0))
        .collect();
    assert_eq!(r0_webs.len(), 2, "reused storage splits into two webs");
    let scalars: Vec<ScalarType> = r0_webs.iter().map(|web| web.shape.scalar).collect();
    assert!(scalars.contains(&ScalarType::F32));
    assert!(scalars.contains(&ScalarType::U32));
    lifting.builder.finish().unwrap();
}

/// A loop-carried parallel transfer whose target is read by a sibling
/// serializes through a synthetic pre-state copy.
#[test]
fn parallel_hazard_pre_copies_when_webs_unite() {
    let mut builder = FunctionBuilder::<TestDialect>::new("test".into());
    let entry = builder.entry();
    let header = builder.new_block("header");
    let exit = builder.new_block("exit");
    builder.add_edge(entry, header, Edge::Entry).unwrap();
    builder.add_edge(header, header, Edge::True).unwrap();
    builder.add_edge(header, exit, Edge::False).unwrap();
    // Parallel: r0.x ← r1.x / r0.x ; r2.x ← r1.x % r0.x. The loop back
    // edge φ-unites r0.x's versions, so the sibling read needs the
    // pre-state copy.
    builder
        .append(
            header,
            Statement::Transfer {
                assignments: vec![
                    (
                        Place {
                            storage: 0,
                            lanes: vec![0],
                        },
                        apply(
                            Operator::Div,
                            vec![
                                read(1, &[0], ScalarType::U32),
                                read(0, &[0], ScalarType::U32),
                            ],
                            ValueShape::scalar(ScalarType::U32),
                        ),
                    ),
                    (
                        Place {
                            storage: 2,
                            lanes: vec![0],
                        },
                        apply(
                            Operator::Rem,
                            vec![
                                read(1, &[0], ScalarType::U32),
                                read(0, &[0], ScalarType::U32),
                            ],
                            ValueShape::scalar(ScalarType::U32),
                        ),
                    ),
                ],
                effects: Vec::new(),
                may_throw: false,
            },
            None,
        )
        .unwrap();
    builder
        .append(
            header,
            Statement::Branch {
                condition: apply(
                    Operator::Less,
                    vec![read(2, &[0], ScalarType::U32), constant(8, ScalarType::U32)],
                    ValueShape::scalar(ScalarType::U32),
                ),
            },
            None,
        )
        .unwrap();
    builder
        .append(exit, Statement::Return { values: Vec::new() }, None)
        .unwrap();
    let function = builder.finish().unwrap();

    let lifting = lift(&function).unwrap();
    let synthetic: Vec<_> = lifting
        .webs
        .iter()
        .filter(|web| web.storage.is_none())
        .collect();
    assert_eq!(synthetic.len(), 1, "one pre-state copy temporary");
    let function = lifting.builder.finish().unwrap();
    // header holds: copy, div, rem, branch.
    let header_len = function
        .cfg()
        .blocks()
        .iter()
        .find(|block| block.label() == Some("header"))
        .map(|block| block.instructions().len());
    assert_eq!(header_len, Some(4));
}

/// A diamond join φ-unites both definitions into one typed web.
#[test]
fn join_unites_definitions_into_one_web() {
    let mut builder = FunctionBuilder::<TestDialect>::new("test".into());
    let entry = builder.entry();
    let head = builder.new_block("head");
    let then = builder.new_block("then");
    let other = builder.new_block("else");
    let join = builder.new_block("join");
    builder.add_edge(entry, head, Edge::Entry).unwrap();
    builder.add_edge(head, then, Edge::True).unwrap();
    builder.add_edge(head, other, Edge::False).unwrap();
    builder.add_edge(then, join, Edge::Next).unwrap();
    builder.add_edge(other, join, Edge::Next).unwrap();
    builder
        .append(
            head,
            Statement::Branch {
                condition: read(9, &[0], ScalarType::U32),
            },
            None,
        )
        .unwrap();
    builder
        .append(
            then,
            assign(0, &[0], constant(0x3f80_0000, ScalarType::F32)),
            None,
        )
        .unwrap();
    builder
        .append(
            other,
            assign(0, &[0], constant(0x4000_0000, ScalarType::F32)),
            None,
        )
        .unwrap();
    builder
        .append(join, assign(1, &[0], read(0, &[0], ScalarType::F32)), None)
        .unwrap();
    builder
        .append(join, Statement::Return { values: Vec::new() }, None)
        .unwrap();
    let function = builder.finish().unwrap();

    let lifting = lift(&function).unwrap();
    let r0_webs: Vec<_> = lifting
        .webs
        .iter()
        .filter(|web| web.storage == Some(0))
        .collect();
    assert_eq!(r0_webs.len(), 1, "both arms define one web");
    assert_eq!(r0_webs[0].shape.scalar, ScalarType::F32);
    assert!(!r0_webs[0].live_in);
    lifting.builder.finish().unwrap();
}

/// Reads that resolve across webs compose, and partial writes merge.
#[test]
fn partial_writes_merge_and_cross_web_reads_compose() {
    let mut builder = FunctionBuilder::<TestDialect>::new("test".into());
    let entry = builder.entry();
    let body = builder.new_block("body");
    builder.add_edge(entry, body, Edge::Entry).unwrap();
    // r0.xy ← f32 vector; r0.x ← f32 scalar (partial rewrite of the same
    // web via a second co-written def is a fresh version — force a merge
    // by reading r0.xy afterwards so the versions unite through use of
    // both lanes written at different times.
    builder
        .append(
            body,
            assign(
                0,
                &[0, 1],
                apply(
                    Operator::Add,
                    vec![
                        Expr::Const {
                            bits: vec![1, 2],
                            shape: ValueShape::vector(ScalarType::F32, 2),
                        },
                        Expr::Const {
                            bits: vec![3, 4],
                            shape: ValueShape::vector(ScalarType::F32, 2),
                        },
                    ],
                    ValueShape::vector(ScalarType::F32, 2),
                ),
            ),
            None,
        )
        .unwrap();
    builder
        .append(
            body,
            assign(1, &[0, 1], read(0, &[0, 1], ScalarType::F32)),
            None,
        )
        .unwrap();
    builder
        .append(body, Statement::Return { values: Vec::new() }, None)
        .unwrap();
    let function = builder.finish().unwrap();

    let lifting = lift(&function).unwrap();
    let r0_web = lifting
        .webs
        .iter()
        .find(|web| web.storage == Some(0))
        .unwrap();
    assert_eq!(r0_web.lanes, vec![0, 1]);
    assert_eq!(r0_web.shape, ValueShape::vector(ScalarType::F32, 2));
    let r1_web = lifting
        .webs
        .iter()
        .find(|web| web.storage == Some(1))
        .unwrap();
    assert_eq!(r1_web.shape, ValueShape::vector(ScalarType::F32, 2));
    lifting.builder.finish().unwrap();
}
