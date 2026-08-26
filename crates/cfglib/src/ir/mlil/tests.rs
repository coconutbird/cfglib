extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crate::ir::dialect::Vocabulary;
use crate::{ConstValue, EdgeKind, FlowEffect};

use super::{
    AnalysisDialect, Dialect, FunctionBuilder, Instruction, InstructionMetadata, TypedVariable,
    VariableId, VerificationIssue, VerifyDialect,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Type {
    Integer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Effect {
    Control,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operation {
    Constant(i64),
    Copy,
    Return,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Entry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Span {
    start: u32,
    end: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ToyDialect;

impl Vocabulary for ToyDialect {
    type ValueType = Type;
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

impl Dialect for ToyDialect {
    type Operation = Operation;
    type Edge = Edge;

    fn instruction_metadata(
        operation: &Self::Operation,
        _may_throw: bool,
    ) -> InstructionMetadata<Self::Effect> {
        match operation {
            Operation::Return => {
                InstructionMetadata::new(vec![Effect::Control], FlowEffect::Return, false)
            }
            Operation::Constant(_) | Operation::Copy => {
                InstructionMetadata::new(Vec::new(), FlowEffect::Fallthrough, false)
            }
        }
    }

    fn mnemonic(operation: &Self::Operation) -> &str {
        match operation {
            Operation::Constant(_) => "const",
            Operation::Copy => "copy",
            Operation::Return => "return",
        }
    }

    fn edge_kind(_edge: &Self::Edge) -> EdgeKind {
        EdgeKind::Fallthrough
    }

    fn is_entry_edge(edge: &Self::Edge) -> bool {
        *edge == Edge::Entry
    }
}

impl AnalysisDialect for ToyDialect {
    type Constant = i64;
    type ExpressionOperator = Operation;
    type Callee = u32;

    fn is_copy(operation: &Self::Operation) -> bool {
        *operation == Operation::Copy
    }

    fn expression_operator(operation: &Self::Operation) -> Option<Self::ExpressionOperator> {
        matches!(operation, Operation::Copy).then_some(*operation)
    }

    fn constant(operation: &Self::Operation) -> Option<Self::Constant> {
        let Operation::Constant(value) = operation else {
            return None;
        };
        Some(*value)
    }

    fn fold_constant(
        instruction: &Instruction<Self>,
        known: &BTreeMap<VariableId, Self::Constant>,
    ) -> Option<(VariableId, Self::Constant)> {
        let destination = *instruction.defs().first()?;
        let value = match instruction.operation() {
            Operation::Constant(value) => *value,
            Operation::Copy => *known.get(instruction.uses().first()?)?,
            Operation::Return => return None,
        };
        Some((destination, value))
    }

    fn callee(_operation: &Self::Operation) -> Option<Self::Callee> {
        None
    }
}

impl VerifyDialect for ToyDialect {
    fn verify(function: &super::Function<Self>, issues: &mut Vec<VerificationIssue>) {
        for block in function.cfg().blocks().iter().skip(1) {
            if !matches!(
                block.instructions().last().map(Instruction::operation),
                Some(Operation::Return)
            ) {
                issues.push(VerificationIssue::new(format!(
                    "block {} does not end in return",
                    block.id()
                )));
            }
        }
    }
}

#[test]
fn generic_dialect_builds_verifies_and_uses_analyses() {
    let mut builder = FunctionBuilder::<ToyDialect>::new("toy::answer".into());
    let body = builder.new_block("body");
    let value = builder.declare_variable(0, Some(7)).unwrap();
    builder
        .append_instruction(
            body,
            Operation::Constant(42),
            Vec::new(),
            vec![TypedVariable::new(value, Type::Integer)],
            false,
            Some(Span { start: 4, end: 5 }),
        )
        .unwrap();
    builder
        .append_instruction(
            body,
            Operation::Return,
            vec![TypedVariable::new(value, Type::Integer)],
            Vec::new(),
            false,
            Some(Span { start: 5, end: 6 }),
        )
        .unwrap();
    builder
        .add_edge(builder.entry(), body, Edge::Entry, None)
        .unwrap();

    let function = builder.finish().unwrap();
    assert!(function.verify().is_ok());
    assert_eq!(function.source(), "toy::answer");
    assert_eq!(function.ssa().unwrap().blocks().len(), 2);
    assert_eq!(
        function.constants().fact_out(body).get(&value),
        Some(&ConstValue::Const(42))
    );
    assert_eq!(function.provenance().mappings_from(4).count(), 1);
}

#[test]
fn signature_and_region_survive_construction() {
    let mut builder = FunctionBuilder::<ToyDialect>::new("toy::signature".into());
    let body = builder.new_block("body");
    let pad = builder.new_block("pad");
    let argument = builder.declare_variable(0, None).unwrap();
    builder
        .append_instruction(
            body,
            Operation::Return,
            vec![TypedVariable::new(argument, Type::Integer)],
            Vec::new(),
            false,
            None,
        )
        .unwrap();
    builder
        .append_instruction(pad, Operation::Return, Vec::new(), Vec::new(), false, None)
        .unwrap();
    builder
        .add_edge(builder.entry(), body, Edge::Entry, None)
        .unwrap();
    builder
        .set_signature(super::Signature::<ToyDialect>::new(
            vec![argument],
            vec![Type::Integer],
        ))
        .unwrap();
    let region = builder
        .add_region(crate::Region {
            id: crate::RegionId::from_raw(0),
            protected_blocks: [body].into_iter().collect(),
            handlers: vec![crate::Handler {
                entry: pad,
                body: crate::HandlerBody::known([pad]),
                kind: crate::HandlerKind::CatchAll,
            }],
            parent: None,
        })
        .unwrap();

    let function = builder.finish().unwrap();
    assert_eq!(function.signature().parameters, vec![argument]);
    assert_eq!(function.signature().returns, vec![Type::Integer]);
    assert_eq!(function.cfg().regions().len(), 1);
    assert_eq!(function.cfg().regions()[0].id, region);
    assert_eq!(function.instructions().count(), 2);
}

#[test]
fn invalid_signatures_and_regions_are_rejected() {
    let mut builder = FunctionBuilder::<ToyDialect>::new("toy::invalid-metadata".into());
    let body = builder.new_block("body");
    let undeclared = VariableId::from_raw(7);
    let error = builder
        .set_signature(super::Signature::<ToyDialect>::new(
            vec![undeclared],
            Vec::new(),
        ))
        .unwrap_err()
        .to_string();
    assert!(error.contains("undeclared parameter"));

    let declared = builder.declare_variable(0, None).unwrap();
    let error = builder
        .set_signature(super::Signature::<ToyDialect>::new(
            vec![declared, declared],
            Vec::new(),
        ))
        .unwrap_err()
        .to_string();
    assert!(error.contains("repeats parameter"));

    let error = builder
        .add_region(crate::Region {
            id: crate::RegionId::from_raw(0),
            protected_blocks: [crate::BlockId::from_raw(9)].into_iter().collect(),
            handlers: Vec::new(),
            parent: None,
        })
        .unwrap_err()
        .to_string();
    assert!(error.contains("outside"));

    let error = builder
        .add_region(crate::Region {
            id: crate::RegionId::from_raw(0),
            protected_blocks: [body].into_iter().collect(),
            handlers: vec![crate::Handler {
                entry: body,
                body: crate::HandlerBody::known([builder.entry()]),
                kind: crate::HandlerKind::CatchAll,
            }],
            parent: None,
        })
        .unwrap_err()
        .to_string();
    assert!(error.contains("synthetic root"));
}

#[test]
fn generic_verifier_requires_one_root_entry() {
    let mut builder = FunctionBuilder::<ToyDialect>::new("toy::invalid".into());
    let body = builder.new_block("body");
    builder
        .append_instruction(body, Operation::Return, Vec::new(), Vec::new(), false, None)
        .unwrap();

    let error = builder.finish().unwrap_err().to_string();
    assert!(error.contains("synthetic root has 0 outgoing edges instead of one"));
}

#[test]
fn rejected_source_spans_do_not_partially_mutate_the_builder() {
    let mut builder = FunctionBuilder::<ToyDialect>::new("toy::atomic".into());
    let body = builder.new_block("body");
    let value = builder.declare_variable(0, None).unwrap();
    let invalid = Span { start: 3, end: 3 };

    assert!(
        builder
            .append_instruction(
                body,
                Operation::Constant(7),
                Vec::new(),
                vec![TypedVariable::new(value, Type::Integer)],
                false,
                Some(invalid),
            )
            .is_err()
    );
    assert!(
        builder
            .add_edge(builder.entry(), body, Edge::Entry, Some(invalid))
            .is_err()
    );

    let instruction = builder
        .append_instruction(
            body,
            Operation::Return,
            Vec::new(),
            Vec::new(),
            false,
            Some(Span { start: 3, end: 4 }),
        )
        .unwrap();
    builder
        .add_edge(builder.entry(), body, Edge::Entry, None)
        .unwrap();

    let function = builder.finish().unwrap();
    assert_eq!(instruction.index(), 0);
    assert_eq!(function.cfg().edge_count(), 1);
}
