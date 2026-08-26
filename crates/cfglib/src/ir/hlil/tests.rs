extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crate::ir::dialect::Vocabulary;
use crate::ir::mlil;
use crate::{EdgeKind, FlowEffect};

use super::{
    Dialect, ExpressionKind, FunctionBuilder, LiftDialect, Lifted, LowerDialect, Signature,
    StatementKind, VerificationIssue, VerifyDialect, lift_function,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Type {
    Integer,
    Boolean,
    Void,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Effect {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Span {
    start: u32,
    end: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Toy;

impl Vocabulary for Toy {
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

/// High-level operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operation {
    Add,
    LessThan,
    Below,
    AtLeast,
    Not,
    Load,
    Deref,
    Call,
    Select,
    Acquire,
    Release,
    Caught,
    Throw,
}

impl Dialect for Toy {
    type Operation = Operation;
    type Constant = i64;

    fn mnemonic(operation: &Self::Operation) -> &str {
        match operation {
            Operation::Add => "add",
            Operation::LessThan => "lt",
            Operation::Below => "below",
            Operation::AtLeast => "at-least",
            Operation::Not => "not",
            Operation::Load => "load",
            Operation::Deref => "deref",
            Operation::Call => "call",
            Operation::Select => "select",
            Operation::Acquire => "acquire",
            Operation::Release => "release",
            Operation::Caught => "caught",
            Operation::Throw => "throw",
        }
    }
}

impl VerifyDialect for Toy {
    fn verify(_function: &super::Function<Self>, _issues: &mut Vec<VerificationIssue>) {}
}

/// Medium-level operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MediumOperation {
    Constant(i64),
    Copy,
    Add,
    LessThan,
    Not,
    Load,
    Call,
    Store,
    Exchange,
    Branch,
    CompareBranch,
    Switch,
    Jump,
    Return,
    /// A read-modify-write merge: operand 1 reads the destination's
    /// previous value.
    Merge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Entry,
    True,
    False,
    Fall,
    Jump,
    Case(i64),
    Except,
}

impl mlil::Dialect for Toy {
    type Operation = MediumOperation;
    type Edge = Edge;

    fn instruction_metadata(
        operation: &Self::Operation,
        may_throw: bool,
    ) -> mlil::InstructionMetadata<Self::Effect> {
        let (effects, flow) = match operation {
            MediumOperation::Call | MediumOperation::Store => {
                (vec![Effect::Write], FlowEffect::Fallthrough)
            }
            MediumOperation::Load => (vec![Effect::Read], FlowEffect::Fallthrough),
            MediumOperation::Branch | MediumOperation::CompareBranch => {
                (Vec::new(), FlowEffect::ConditionalJump)
            }
            MediumOperation::Switch => (Vec::new(), FlowEffect::IndirectJump),
            MediumOperation::Jump => (Vec::new(), FlowEffect::Jump),
            MediumOperation::Return => (Vec::new(), FlowEffect::Return),
            _ => (Vec::new(), FlowEffect::Fallthrough),
        };
        mlil::InstructionMetadata::new(effects, flow, may_throw)
    }

    fn mnemonic(operation: &Self::Operation) -> &str {
        match operation {
            MediumOperation::Constant(_) => "const",
            MediumOperation::Copy => "copy",
            MediumOperation::Add => "add",
            MediumOperation::LessThan => "lt",
            MediumOperation::Not => "not",
            MediumOperation::Load => "load",
            MediumOperation::Call => "call",
            MediumOperation::Store => "store",
            MediumOperation::Exchange => "exchange",
            MediumOperation::Branch => "branch",
            MediumOperation::CompareBranch => "compare_branch",
            MediumOperation::Switch => "switch",
            MediumOperation::Jump => "jump",
            MediumOperation::Return => "return",
            MediumOperation::Merge => "merge",
        }
    }

    fn edge_kind(edge: &Self::Edge) -> EdgeKind {
        match edge {
            Edge::Entry | Edge::Fall => EdgeKind::Fallthrough,
            Edge::True => EdgeKind::ConditionalTrue,
            Edge::False => EdgeKind::ConditionalFalse,
            Edge::Jump => EdgeKind::Jump,
            Edge::Case(_) => EdgeKind::SwitchCase,
            Edge::Except => EdgeKind::ExceptionHandler,
        }
    }

    fn is_entry_edge(edge: &Self::Edge) -> bool {
        *edge == Edge::Entry
    }
}

impl mlil::AnalysisDialect for Toy {
    type Constant = i64;
    type ExpressionOperator = MediumOperation;
    type Callee = u32;

    fn is_copy(operation: &Self::Operation) -> bool {
        *operation == MediumOperation::Copy
    }

    fn expression_operator(operation: &Self::Operation) -> Option<Self::ExpressionOperator> {
        matches!(
            operation,
            MediumOperation::Add | MediumOperation::LessThan | MediumOperation::Copy
        )
        .then_some(*operation)
    }

    fn constant(operation: &Self::Operation) -> Option<Self::Constant> {
        match operation {
            MediumOperation::Constant(value) => Some(*value),
            _ => None,
        }
    }

    fn fold_constant(
        _instruction: &mlil::Instruction<Self>,
        _known: &alloc::collections::BTreeMap<mlil::VariableId, Self::Constant>,
    ) -> Option<(mlil::VariableId, Self::Constant)> {
        None
    }

    fn callee(_operation: &Self::Operation) -> Option<Self::Callee> {
        None
    }
}

impl mlil::VerifyDialect for Toy {
    fn verify(_function: &mlil::Function<Self>, _issues: &mut Vec<mlil::VerificationIssue>) {}
}

impl LiftDialect for Toy {
    fn negate_operation(operation: &Operation) -> Option<Operation> {
        match operation {
            Operation::Below => Some(Operation::AtLeast),
            Operation::AtLeast => Some(Operation::Below),
            _ => None,
        }
    }

    fn previous_value_operand(operation: &MediumOperation) -> Option<usize> {
        matches!(operation, MediumOperation::Merge).then_some(1)
    }

    fn lift_operation(operation: &MediumOperation) -> Lifted<Operation> {
        match operation {
            MediumOperation::Add => Lifted::Operation(Operation::Add),
            MediumOperation::LessThan => Lifted::Operation(Operation::LessThan),
            MediumOperation::Not => Lifted::Operation(Operation::Not),
            MediumOperation::Load => Lifted::Operation(Operation::Load),
            MediumOperation::Call | MediumOperation::Merge => Lifted::Operation(Operation::Call),
            MediumOperation::Store => Lifted::Store {
                location: Operation::Deref,
            },
            MediumOperation::Exchange => Lifted::ParallelCopy,
            MediumOperation::Branch => Lifted::Branch,
            MediumOperation::CompareBranch => Lifted::BranchOperation(Operation::Below),
            MediumOperation::Switch => Lifted::Switch,
            MediumOperation::Return => Lifted::Return,
            MediumOperation::Jump | MediumOperation::Constant(_) | MediumOperation::Copy => {
                Lifted::ControlFlow
            }
        }
    }

    fn case_values(edge: &Edge) -> Vec<i64> {
        match edge {
            Edge::Case(value) => vec![*value],
            _ => Vec::new(),
        }
    }

    fn void_type() -> Type {
        Type::Void
    }

    fn logical_not() -> Option<Operation> {
        Some(Operation::Not)
    }

    fn temporary_role() -> Option<u8> {
        Some(1)
    }

    fn evaluation_commutes(
        moved_effects: &[Effect],
        moved_may_throw: bool,
        crossed_effects: &[Effect],
        crossed_may_throw: bool,
    ) -> bool {
        // Reads pass reads; nothing passes a write or a potential throw.
        !moved_may_throw
            && !crossed_may_throw
            && moved_effects.iter().all(|effect| *effect == Effect::Read)
            && crossed_effects.iter().all(|effect| *effect == Effect::Read)
    }
}

impl super::RecoverDialect for Toy {
    fn select() -> Option<Operation> {
        Some(Operation::Select)
    }

    fn region_enter(operation: &Operation) -> Option<Operation> {
        matches!(operation, Operation::Acquire).then_some(Operation::Acquire)
    }

    fn releases(enter: &Operation, exit: &Operation) -> bool {
        matches!(enter, Operation::Acquire) && matches!(exit, Operation::Release)
    }

    fn is_exception_materialization(operation: &Operation) -> bool {
        matches!(operation, Operation::Caught)
    }

    fn is_throw(operation: &Operation) -> bool {
        matches!(operation, Operation::Throw)
    }
}

impl LowerDialect for Toy {
    fn lower_operation(operation: &Operation) -> MediumOperation {
        match operation {
            Operation::Add | Operation::Select => MediumOperation::Add,
            Operation::LessThan
            | Operation::Below
            | Operation::AtLeast
            | Operation::Acquire
            | Operation::Release
            | Operation::Caught
            | Operation::Throw => MediumOperation::LessThan,
            Operation::Not => MediumOperation::Not,
            Operation::Load | Operation::Deref => MediumOperation::Load,
            Operation::Call => MediumOperation::Call,
        }
    }

    fn lower_constant(constant: &i64) -> MediumOperation {
        MediumOperation::Constant(*constant)
    }

    fn copy_operation() -> MediumOperation {
        MediumOperation::Copy
    }

    fn store_operation(_location: &Operation) -> MediumOperation {
        MediumOperation::Store
    }

    fn branch_operation() -> MediumOperation {
        MediumOperation::Branch
    }

    fn switch_operation() -> MediumOperation {
        MediumOperation::Switch
    }

    fn return_operation() -> MediumOperation {
        MediumOperation::Return
    }

    fn temporary_role() -> u8 {
        1
    }

    fn operation_may_throw(operation: &MediumOperation) -> bool {
        *operation == MediumOperation::Call
    }

    fn entry_edge() -> Edge {
        Edge::Entry
    }

    fn fallthrough_edge() -> Edge {
        Edge::Fall
    }

    fn jump_edge() -> Edge {
        Edge::Jump
    }

    fn true_edge() -> Edge {
        Edge::True
    }

    fn false_edge() -> Edge {
        Edge::False
    }

    fn case_edge(value: &i64) -> Edge {
        Edge::Case(*value)
    }

    fn default_edge() -> Edge {
        Edge::Fall
    }

    fn unwind_edge() -> Edge {
        Edge::Except
    }
}

#[test]
fn builder_constructs_verifies_and_renders() {
    let mut builder = FunctionBuilder::<Toy>::new("toy::bump".into());
    let counter = builder
        .declare_variable(0, Some(7), Some(Type::Integer))
        .unwrap();
    let read = builder
        .add_expression(ExpressionKind::Variable(counter), Type::Integer)
        .unwrap();
    let ten = builder
        .add_expression(ExpressionKind::Constant(10), Type::Integer)
        .unwrap();
    let compare = builder
        .add_expression(
            ExpressionKind::Operation {
                operation: Operation::LessThan,
                operands: vec![read, ten],
            },
            Type::Boolean,
        )
        .unwrap();
    let read_again = builder
        .add_expression(ExpressionKind::Variable(counter), Type::Integer)
        .unwrap();
    let one = builder
        .add_expression(ExpressionKind::Constant(1), Type::Integer)
        .unwrap();
    let sum = builder
        .add_expression(
            ExpressionKind::Operation {
                operation: Operation::Add,
                operands: vec![read_again, one],
            },
            Type::Integer,
        )
        .unwrap();
    let target = builder
        .add_expression(ExpressionKind::Variable(counter), Type::Integer)
        .unwrap();
    let assign = builder
        .add_statement(
            StatementKind::Assign { target, value: sum },
            Some(Span { start: 4, end: 9 }),
        )
        .unwrap();
    let conditional = builder
        .add_statement(
            StatementKind::If {
                condition: compare,
                then_body: vec![assign],
                else_body: Vec::new(),
            },
            None,
        )
        .unwrap();
    let result = builder
        .add_expression(ExpressionKind::Variable(counter), Type::Integer)
        .unwrap();
    let return_statement = builder
        .add_statement(
            StatementKind::Return {
                values: vec![result],
            },
            None,
        )
        .unwrap();
    builder
        .set_signature(Signature::<Toy>::new(vec![counter], vec![Type::Integer]))
        .unwrap();
    builder
        .set_body(vec![conditional, return_statement])
        .unwrap();

    let function = builder.finish().unwrap();
    assert!(function.verify().is_ok());
    assert_eq!(function.source(), "toy::bump");
    assert_eq!(function.signature().parameters, vec![counter]);

    let pseudo = function.to_pseudocode();
    assert!(pseudo.contains("if (lt(v0, 10)) {"), "{pseudo}");
    assert!(pseudo.contains("v0 = add(v0, 1);"), "{pseudo}");
    assert!(pseudo.contains("return v0;"), "{pseudo}");

    assert_eq!(function.provenance().mappings_from(5).count(), 1);
}

#[test]
fn shared_expressions_and_orphans_are_rejected() {
    let mut builder = FunctionBuilder::<Toy>::new("toy::shared".into());
    let variable = builder.declare_variable(0, None, None).unwrap();
    let read = builder
        .add_expression(ExpressionKind::Variable(variable), Type::Integer)
        .unwrap();
    builder
        .add_statement(StatementKind::Return { values: vec![read] }, None)
        .unwrap();
    let second = builder
        .add_statement(StatementKind::Return { values: vec![read] }, None)
        .unwrap();
    builder.set_body(vec![second]).unwrap();

    let error = builder.finish().unwrap_err().to_string();
    assert!(error.contains("referenced 2 times"), "{error}");
    assert!(error.contains("referenced 0 times"), "{error}");
}

#[test]
fn transfers_need_matching_context() {
    let mut builder = FunctionBuilder::<Toy>::new("toy::transfers".into());
    let stray_break = builder
        .add_statement(StatementKind::Break { label: None }, None)
        .unwrap();
    let stray_goto = builder
        .add_statement(
            StatementKind::Goto {
                label: "missing".into(),
            },
            None,
        )
        .unwrap();
    builder.set_body(vec![stray_break, stray_goto]).unwrap();

    let error = builder.finish().unwrap_err().to_string();
    assert!(
        error.contains("breaks outside any loop or switch"),
        "{error}"
    );
    assert!(error.contains("targets undefined label missing"), "{error}");
}

/// A machine-shaped counting loop:
/// `while (i < n) { i = i + 1 }; return i;` in flat MLIL.
fn counting_loop() -> mlil::Function<Toy> {
    let mut builder = mlil::FunctionBuilder::<Toy>::new("toy::count".into());
    let header = builder.new_block("header");
    let body = builder.new_block("body");
    let exit = builder.new_block("exit");
    let i = builder.declare_variable(0, None).unwrap();
    let n = builder.declare_variable(0, None).unwrap();
    let t_cond = builder.declare_variable(1, None).unwrap();
    let t_sum = builder.declare_variable(1, None).unwrap();
    let typed = |variable, value_type| mlil::TypedVariable::<Toy>::new(variable, value_type);

    builder
        .append_instruction(
            header,
            MediumOperation::LessThan,
            vec![typed(i, Type::Integer), typed(n, Type::Integer)],
            vec![typed(t_cond, Type::Boolean)],
            false,
            None,
        )
        .unwrap();
    builder
        .append_instruction(
            header,
            MediumOperation::Branch,
            vec![typed(t_cond, Type::Boolean)],
            Vec::new(),
            false,
            None,
        )
        .unwrap();
    builder
        .append_instruction(
            body,
            MediumOperation::Constant(1),
            Vec::new(),
            vec![typed(t_sum, Type::Integer)],
            false,
            None,
        )
        .unwrap();
    let add = builder
        .append_instruction(
            body,
            MediumOperation::Add,
            vec![typed(i, Type::Integer), typed(t_sum, Type::Integer)],
            vec![typed(i, Type::Integer)],
            false,
            Some(Span { start: 10, end: 20 }),
        )
        .unwrap();
    builder
        .append_instruction(
            body,
            MediumOperation::Jump,
            Vec::new(),
            Vec::new(),
            false,
            None,
        )
        .unwrap();
    builder
        .append_instruction(
            exit,
            MediumOperation::Return,
            vec![typed(i, Type::Integer)],
            Vec::new(),
            false,
            None,
        )
        .unwrap();
    builder
        .add_edge(builder.entry(), header, Edge::Entry, None)
        .unwrap();
    builder.add_edge(header, body, Edge::True, None).unwrap();
    builder.add_edge(header, exit, Edge::False, None).unwrap();
    builder.add_edge(body, header, Edge::Fall, None).unwrap();
    builder
        .set_signature(mlil::Signature::<Toy>::new(vec![i, n], vec![Type::Integer]))
        .unwrap();
    let function = builder.finish().unwrap();
    // Sanity: the lifted instruction map below keys off this id.
    assert_eq!(add.index(), 3);
    function
}

#[test]
fn lift_recovers_a_while_loop_with_inlined_expressions() {
    let source = counting_loop();
    let lifted = lift_function(&source).unwrap();
    assert!(lifted.report.is_fully_structured(), "{:?}", lifted.report);
    assert!(lifted.function.verify().is_ok());

    let pseudo = lifted.function.to_pseudocode();
    assert!(pseudo.contains("while (lt(v0, v1)) {"), "{pseudo}");
    assert!(pseudo.contains("v0 = add(v0, 1);"), "{pseudo}");
    assert!(pseudo.contains("return v0;"), "{pseudo}");
    // The comparison and the constant were inlined: no temporaries survive.
    assert!(!pseudo.contains("v2 ="), "{pseudo}");
    assert!(!pseudo.contains("v3 ="), "{pseudo}");

    // Signature and variables carried over one-to-one.
    assert_eq!(lifted.function.signature().parameters.len(), 2);
    assert_eq!(lifted.function.variables().len(), source.variables().len());

    // The add instruction's provenance span survived onto its statement.
    assert_eq!(lifted.function.provenance().mappings_from(12).count(), 1);
    assert!(
        lifted
            .instructions
            .contains_key(&mlil::InstructionId::from_raw(3)),
        "{:?}",
        lifted.instructions
    );
}

#[test]
fn lift_recovers_a_switch_with_case_values_and_default() {
    let mut builder = mlil::FunctionBuilder::<Toy>::new("toy::dispatch".into());
    let dispatch = builder.new_block("dispatch");
    let case_one = builder.new_block("one");
    let case_two = builder.new_block("two");
    let fallback = builder.new_block("fallback");
    let merge = builder.new_block("merge");
    let selector = builder.declare_variable(0, None).unwrap();
    let result = builder.declare_variable(0, None).unwrap();
    let typed = |variable| mlil::TypedVariable::<Toy>::new(variable, Type::Integer);

    builder
        .append_instruction(
            dispatch,
            MediumOperation::Switch,
            vec![typed(selector)],
            Vec::new(),
            false,
            None,
        )
        .unwrap();
    for (block, value) in [(case_one, 10), (case_two, 20), (fallback, 30)] {
        builder
            .append_instruction(
                block,
                MediumOperation::Constant(value),
                Vec::new(),
                vec![typed(result)],
                false,
                None,
            )
            .unwrap();
    }
    builder
        .append_instruction(
            merge,
            MediumOperation::Return,
            vec![typed(result)],
            Vec::new(),
            false,
            None,
        )
        .unwrap();
    builder
        .add_edge(builder.entry(), dispatch, Edge::Entry, None)
        .unwrap();
    builder
        .add_edge(dispatch, case_one, Edge::Case(1), None)
        .unwrap();
    builder
        .add_edge(dispatch, case_one, Edge::Case(2), None)
        .unwrap();
    builder
        .add_edge(dispatch, case_two, Edge::Case(3), None)
        .unwrap();
    builder
        .add_edge(dispatch, fallback, Edge::Fall, None)
        .unwrap();
    builder.add_edge(case_one, merge, Edge::Fall, None).unwrap();
    builder.add_edge(case_two, merge, Edge::Fall, None).unwrap();
    builder.add_edge(fallback, merge, Edge::Fall, None).unwrap();

    let lifted = lift_function(&builder.finish().unwrap()).unwrap();
    assert!(lifted.report.is_fully_structured(), "{:?}", lifted.report);
    let pseudo = lifted.function.to_pseudocode();
    assert!(pseudo.contains("switch (v0) {"), "{pseudo}");
    assert!(pseudo.contains("case 1, 2: {"), "{pseudo}");
    assert!(pseudo.contains("case 3: {"), "{pseudo}");
    assert!(pseudo.contains("default: {"), "{pseudo}");
    assert!(pseudo.contains("v1 = 30;"), "{pseudo}");
    assert!(pseudo.contains("return v1;"), "{pseudo}");
}

#[test]
fn lift_structures_declared_exception_regions() {
    let mut builder = mlil::FunctionBuilder::<Toy>::new("toy::guarded".into());
    let protected = builder.new_block("protected");
    let pad = builder.new_block("pad");
    let after = builder.new_block("after");
    let x = builder.declare_variable(0, None).unwrap();
    let fallback = builder.declare_variable(0, None).unwrap();
    let typed = |variable| mlil::TypedVariable::<Toy>::new(variable, Type::Integer);

    builder
        .append_instruction(
            protected,
            MediumOperation::Call,
            Vec::new(),
            vec![typed(x)],
            true,
            None,
        )
        .unwrap();
    builder
        .append_instruction(
            pad,
            MediumOperation::Constant(7),
            Vec::new(),
            vec![typed(fallback)],
            false,
            None,
        )
        .unwrap();
    builder
        .append_instruction(
            pad,
            MediumOperation::Return,
            vec![typed(fallback)],
            Vec::new(),
            false,
            None,
        )
        .unwrap();
    builder
        .append_instruction(
            after,
            MediumOperation::Return,
            vec![typed(x)],
            Vec::new(),
            false,
            None,
        )
        .unwrap();
    builder
        .add_edge(builder.entry(), protected, Edge::Entry, None)
        .unwrap();
    builder
        .add_edge(protected, after, Edge::Fall, None)
        .unwrap();
    builder
        .add_edge(protected, pad, Edge::Except, None)
        .unwrap();
    builder
        .add_region(crate::Region {
            id: crate::RegionId::from_raw(0),
            protected_blocks: [protected].into_iter().collect(),
            handlers: vec![crate::Handler {
                entry: pad,
                body: crate::HandlerBody::known([pad]),
                kind: crate::HandlerKind::CatchAll,
            }],
            parent: None,
        })
        .unwrap();

    let lifted = lift_function(&builder.finish().unwrap()).unwrap();
    let pseudo = lifted.function.to_pseudocode();
    assert!(pseudo.contains("try {"), "{pseudo}");
    assert!(pseudo.contains("} catch (...)"), "{pseudo}");
    assert!(pseudo.contains("return 7;"), "{pseudo}");
    assert!(pseudo.contains("v0 = call();"), "{pseudo}");
    assert!(pseudo.contains("return v0;"), "{pseudo}");
}
#[test]
fn variable_splitting_composes_with_lifting() {
    // One storage slot reused for two lifetimes, each read twice so neither
    // definition inlines away — the decompiler shape variable splitting
    // exists for.
    let mut builder = mlil::FunctionBuilder::<Toy>::new("toy::slots".into());
    let block = builder.new_block("body");
    let slot = builder.declare_variable(0, Some(7)).unwrap();
    let reads: Vec<_> = (0..4)
        .map(|_| builder.declare_variable(0, None).unwrap())
        .collect();
    let typed = |variable| mlil::TypedVariable::<Toy>::new(variable, Type::Integer);
    for (value, pair) in [(1, &reads[0..2]), (2, &reads[2..4])] {
        builder
            .append_instruction(
                block,
                MediumOperation::Constant(value),
                Vec::new(),
                vec![typed(slot)],
                false,
                None,
            )
            .unwrap();
        for &read in pair {
            builder
                .append_instruction(
                    block,
                    MediumOperation::Copy,
                    vec![typed(slot)],
                    vec![typed(read)],
                    false,
                    None,
                )
                .unwrap();
        }
    }
    builder
        .append_instruction(
            block,
            MediumOperation::Return,
            vec![typed(reads[3])],
            Vec::new(),
            false,
            None,
        )
        .unwrap();
    builder
        .add_edge(builder.entry(), block, Edge::Entry, None)
        .unwrap();
    let function = builder.finish().unwrap();

    let split = function.split_variables().unwrap();
    assert_eq!(split.splits[&mlil::VariableId::from_raw(0)].len(), 2);
    let lifted = lift_function(&split.function).unwrap();
    let pseudo = lifted.function.to_pseudocode();

    let target_of = |value: &str| {
        let position = pseudo.find(value).expect(&pseudo);
        let line_start = pseudo[..position].rfind('\n').map_or(0, |at| at + 1);
        pseudo[line_start..position].trim().to_string()
    };
    let first_lifetime = target_of(" = 1;");
    let second_lifetime = target_of(" = 2;");
    assert_ne!(
        first_lifetime, second_lifetime,
        "each lifetime gets its own local: {pseudo}"
    );
}

/// HLIL → MLIL lowering tests, split out to respect the source-size policy.
mod lowering;

/// Effect-ordered inlining tests, split out to respect the source-size
/// policy.
mod effects;

/// Parallel-copy and fused-branch translation tests, split out to respect
/// the source-size policy.
mod fused;

/// Structural recovery tests, split out to respect the source-size policy.
mod recover;

/// Pure-transfer trampoline tests, split out to respect the source-size
/// policy.
mod trampoline;
