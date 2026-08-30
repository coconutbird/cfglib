extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use crate::ir::mlil::{MemoryDialect, TypedVariable};
use crate::{
    MemoryAccess, MemoryAccessKind, MemoryEvent, MemoryEventInfo, MemoryOperations, MemoryTrace,
};

use super::{Edge, Operation, ToyDialect, Type};

impl MemoryDialect for ToyDialect {
    type MemoryLocation = u8;
    type MemoryFence = ();

    fn memory_events(
        instruction: &super::Instruction<Self>,
    ) -> impl Iterator<Item = MemoryEvent<Self::MemoryLocation, super::VariableId, Self::MemoryFence>>
    {
        let access = match instruction.operation() {
            Operation::Load(location) => Some(MemoryAccess::read(
                *location,
                instruction.defs().iter().copied(),
            )),
            Operation::Store(location) => Some(MemoryAccess::write(
                *location,
                instruction.uses().iter().copied(),
            )),
            Operation::Constant(_)
            | Operation::Copy
            | Operation::AddressOf(_)
            | Operation::Branch
            | Operation::Return => None,
        };
        access.map(MemoryEvent::Access).into_iter()
    }
}

#[test]
fn mlil_dialect_automatically_exposes_memory_events() {
    let mut builder = super::FunctionBuilder::<ToyDialect>::new("toy::memory".into());
    let body = builder.new_block("body");
    let value = builder.declare_variable(0, None).unwrap();
    let typed = || TypedVariable::new(value, Type::Integer);
    builder
        .append_instruction(
            body,
            Operation::Store(7),
            vec![typed()],
            Vec::new(),
            false,
            None,
        )
        .unwrap();
    builder
        .append_instruction(
            body,
            Operation::Load(7),
            Vec::new(),
            vec![typed()],
            false,
            None,
        )
        .unwrap();
    builder
        .append_instruction(
            body,
            Operation::Return,
            vec![typed()],
            Vec::new(),
            false,
            None,
        )
        .unwrap();
    builder
        .add_edge(builder.entry(), body, Edge::Entry, None)
        .unwrap();
    let function = builder.finish().unwrap();

    assert_eq!(
        function.cfg().block(body).instructions()[0].memory_operations(),
        MemoryOperations::WRITE
    );
    assert_eq!(
        function.cfg().block(body).instructions()[1].memory_operations(),
        MemoryOperations::READ
    );

    let trace = MemoryTrace::compute(function.cfg());
    let kinds: Vec<_> = trace
        .entries_in(body)
        .filter_map(|entry| match entry.event() {
            MemoryEvent::Access(access) => Some(access.kind()),
            MemoryEvent::Fence(()) => None,
        })
        .collect();

    assert_eq!(kinds, [MemoryAccessKind::Write, MemoryAccessKind::Read]);
}
