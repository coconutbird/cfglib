//! Return-value lowering tests: materialization into temporaries and
//! the one-use-per-value contract.

extern crate alloc;

use alloc::vec;

use crate::ir::hlil::{ExpressionKind, StatementKind, lift_function as lift_hlil};

use super::{
    Edge, FunctionBuilder, LiftedStatement, Operator, ScalarType, Statement, TestDialect,
    ValueShape, VarExpr, apply, constant, instructions, lift, read,
};

/// A returned expression materializes into a temporary, so the return's
/// MLIL uses pair one-to-one with its values and HLIL sees exactly one
/// returned value.
#[test]
fn return_expression_materializes_a_temporary() {
    let mut builder = FunctionBuilder::<TestDialect>::new("test".into());
    let entry = builder.entry();
    let body = builder.new_block("body");
    builder.add_edge(entry, body, Edge::Entry).unwrap();
    builder
        .append(
            body,
            Statement::Return {
                values: vec![apply(
                    Operator::Add,
                    vec![
                        constant(0x3f80_0000, ScalarType::F32),
                        constant(0x4000_0000, ScalarType::F32),
                    ],
                    ValueShape::scalar(ScalarType::F32),
                )],
            },
            None,
        )
        .unwrap();
    let function = builder.finish().unwrap();

    let lifting = lift(&function, &()).unwrap();
    assert_eq!(
        lifting
            .webs
            .iter()
            .filter(|web| web.storage.is_none())
            .count(),
        1,
        "the returned expression lives in a temporary"
    );
    let function = lifting.builder.finish().unwrap();
    let return_uses = instructions(&function)
        .into_iter()
        .find_map(|instruction| {
            matches!(instruction.operation(), LiftedStatement::Return { .. })
                .then(|| instruction.uses().len())
        })
        .expect("one return instruction");
    assert_eq!(return_uses, 1, "one use per returned value");

    let lifted = lift_hlil(&function).unwrap();
    assert!(lifted.report.is_fully_structured());
    let mut returns = 0usize;
    for statement in lifted.function.statements() {
        if let StatementKind::Return { values } = statement.kind() {
            returns += 1;
            assert_eq!(values.len(), 1, "one returned value survives to HLIL");
            let value = lifted.function.expression(values[0]).unwrap();
            assert!(
                matches!(
                    value.kind(),
                    ExpressionKind::Operation {
                        operation: LiftedStatement::Assign {
                            value: VarExpr::Apply { .. },
                            ..
                        },
                        ..
                    }
                ),
                "the temporary inlines back into the return"
            );
        }
    }
    assert_eq!(returns, 1);
}

/// A returned constant is still one returned value, never a void return.
#[test]
fn returned_constant_stays_a_returned_value() {
    let mut builder = FunctionBuilder::<TestDialect>::new("test".into());
    let entry = builder.entry();
    let body = builder.new_block("body");
    builder.add_edge(entry, body, Edge::Entry).unwrap();
    builder
        .append(
            body,
            Statement::Return {
                values: vec![constant(42, ScalarType::U32)],
            },
            None,
        )
        .unwrap();
    let function = builder.finish().unwrap();

    let lifting = lift(&function, &()).unwrap();
    let function = lifting.builder.finish().unwrap();
    let lifted = lift_hlil(&function).unwrap();
    let mut returns = 0usize;
    for statement in lifted.function.statements() {
        if let StatementKind::Return { values } = statement.kind() {
            returns += 1;
            assert_eq!(values.len(), 1, "a constant return is not void");
        }
    }
    assert_eq!(returns, 1);
}

/// A whole identity read of one web returns as-is, without a temporary.
#[test]
fn whole_web_return_passes_through() {
    let mut builder = FunctionBuilder::<TestDialect>::new("test".into());
    let entry = builder.entry();
    let body = builder.new_block("body");
    builder.add_edge(entry, body, Edge::Entry).unwrap();
    builder
        .append(
            body,
            Statement::Return {
                values: vec![read(0, &[0, 1], ScalarType::F32)],
            },
            None,
        )
        .unwrap();
    let function = builder.finish().unwrap();

    let lifting = lift(&function, &()).unwrap();
    assert!(
        lifting.webs.iter().all(|web| web.storage.is_some()),
        "no temporary for a whole-web read"
    );
    let function = lifting.builder.finish().unwrap();
    let return_uses = instructions(&function)
        .into_iter()
        .find_map(|instruction| {
            matches!(instruction.operation(), LiftedStatement::Return { .. })
                .then(|| instruction.uses().len())
        })
        .expect("one return instruction");
    assert_eq!(return_uses, 1);
}
