extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;

use crate::{
    Cfg, DominatorTree, EdgeKind, ExactMemoryAlias, InstrInfo, MemoryAccess, MemoryAccessKind,
    MemoryDefinition, MemoryEvent, MemoryEventInfo, MemoryEventSite, MemorySSA, MemorySSAEvent,
    MemoryUse, ProgramPoint, SsaForm,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Location {
    A,
    B,
    C,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fence {
    Group,
}

#[derive(Debug, Clone)]
struct Instruction {
    uses: Vec<u8>,
    defs: Vec<u8>,
    events: Vec<MemoryEvent<Location, u8, Fence>>,
}

impl Instruction {
    fn plain(uses: Vec<u8>, defs: Vec<u8>) -> Self {
        Self {
            uses,
            defs,
            events: Vec::new(),
        }
    }

    fn with_events(events: Vec<MemoryEvent<Location, u8, Fence>>) -> Self {
        Self {
            uses: Vec::new(),
            defs: Vec::new(),
            events,
        }
    }

    fn access(location: Location, kind: MemoryAccessKind) -> Self {
        Self::with_events(vec![memory_access(location, Vec::new(), kind)])
    }
}

impl InstrInfo for Instruction {
    type Variable = u8;

    fn uses(&self) -> &[Self::Variable] {
        &self.uses
    }

    fn defs(&self) -> &[Self::Variable] {
        &self.defs
    }
}

impl MemoryEventInfo for Instruction {
    type Location = Location;
    type Fence = Fence;

    fn memory_events(
        &self,
    ) -> impl Iterator<Item = MemoryEvent<Self::Location, Self::Variable, Self::Fence>> {
        self.events.iter().cloned()
    }
}

fn memory_access(
    location: Location,
    address_uses: Vec<u8>,
    kind: MemoryAccessKind,
) -> MemoryEvent<Location, u8, Fence> {
    let access = match kind {
        MemoryAccessKind::Read => MemoryAccess::read(location, Vec::new()),
        MemoryAccessKind::Write => MemoryAccess::write(location, Vec::new()),
        MemoryAccessKind::ReadModifyWrite => {
            MemoryAccess::read_modify_write(location, Vec::new(), Vec::new())
        }
    };
    MemoryEvent::Access(access.with_address_uses(address_uses))
}

fn site(block: crate::BlockId, inst_idx: usize) -> MemoryEventSite {
    MemoryEventSite::new(ProgramPoint { block, inst_idx }, 0)
}

fn linear_cfg(instructions: impl IntoIterator<Item = Instruction>) -> Cfg<Instruction> {
    let mut cfg = Cfg::new();
    cfg.block_mut(cfg.entry())
        .instructions_mut()
        .extend(instructions);
    cfg
}

#[test]
fn blind_writes_clobber_without_becoming_semantic_readers() {
    let cfg = linear_cfg([
        Instruction::access(Location::A, MemoryAccessKind::Write),
        Instruction::access(Location::A, MemoryAccessKind::Read),
        Instruction::access(Location::A, MemoryAccessKind::Write),
        Instruction::access(Location::A, MemoryAccessKind::Read),
    ]);
    let memory = MemorySSA::compute(&cfg, &ExactMemoryAlias);
    let block = cfg.entry();
    let first_write = site(block, 0);
    let first_read = site(block, 1);
    let second_write = site(block, 2);
    let second_read = site(block, 3);

    assert_eq!(
        memory.reaching_definition(first_read),
        Some(&MemoryDefinition::Event { site: first_write })
    );
    assert_eq!(
        memory.clobbered_definition(second_write),
        Some(&MemoryDefinition::Event { site: first_write })
    );
    assert_eq!(
        memory.reaching_definition(second_read),
        Some(&MemoryDefinition::Event { site: second_write })
    );

    let first_version = memory
        .event(first_write)
        .and_then(MemorySSAEvent::written_version)
        .unwrap();
    assert_eq!(
        memory.users(first_version),
        &[MemoryUse::Event { site: first_read }]
    );
    assert_eq!(memory.transitive_readers(first_write), vec![first_read]);
}

#[test]
fn exact_locations_have_independent_live_in_states() {
    let cfg = linear_cfg([
        Instruction::access(Location::A, MemoryAccessKind::Write),
        Instruction::access(Location::B, MemoryAccessKind::Read),
    ]);
    let memory = MemorySSA::compute(&cfg, &ExactMemoryAlias);
    let read = site(cfg.entry(), 1);

    let Some(MemoryDefinition::LiveIn { class }) = memory.reaching_definition(read) else {
        panic!("an unrelated read must consume its own live-in state");
    };
    assert_eq!(Some(*class), memory.class_of(&Location::B));
    assert_ne!(memory.class_of(&Location::A), memory.class_of(&Location::B));
}

#[test]
fn caller_alias_relation_is_symmetrized_and_closed_transitively() {
    let cfg = linear_cfg([
        Instruction::access(Location::A, MemoryAccessKind::Write),
        Instruction::access(Location::B, MemoryAccessKind::Read),
        Instruction::access(Location::C, MemoryAccessKind::Read),
    ]);
    let alias = |left: &Location, right: &Location| {
        matches!(
            (left, right),
            (Location::A, Location::B) | (Location::B, Location::C)
        )
    };
    let memory = MemorySSA::compute(&cfg, &alias);

    assert_eq!(memory.classes().len(), 1);
    assert!(memory.may_alias(&Location::A, &Location::C));
    assert_eq!(memory.events_for(&Location::B).count(), 3);
    assert_eq!(
        memory.reaching_definition(site(cfg.entry(), 2)),
        Some(&MemoryDefinition::Event {
            site: site(cfg.entry(), 0),
        })
    );
}

#[test]
fn branch_merge_expands_memory_phi_to_both_writes() {
    let mut cfg = Cfg::<Instruction>::new();
    let left = cfg.new_block();
    let right = cfg.new_block();
    let merge = cfg.new_block();
    cfg.add_edge(cfg.entry(), left, EdgeKind::ConditionalTrue);
    cfg.add_edge(cfg.entry(), right, EdgeKind::ConditionalFalse);
    cfg.add_edge(left, merge, EdgeKind::Fallthrough);
    cfg.add_edge(right, merge, EdgeKind::Fallthrough);
    cfg.block_mut(left)
        .push(Instruction::access(Location::A, MemoryAccessKind::Write));
    cfg.block_mut(right)
        .push(Instruction::access(Location::A, MemoryAccessKind::Write));
    cfg.block_mut(merge)
        .push(Instruction::access(Location::A, MemoryAccessKind::Read));

    let memory = MemorySSA::compute(&cfg, &ExactMemoryAlias);
    let read = site(merge, 0);
    assert!(matches!(
        memory.reaching_definition(read),
        Some(MemoryDefinition::Phi { block, .. }) if *block == merge
    ));

    let mut definitions: Vec<_> = memory
        .reaching_definitions(read)
        .into_iter()
        .filter_map(|definition| match definition {
            MemoryDefinition::Event { site } => Some(*site),
            MemoryDefinition::LiveIn { .. } | MemoryDefinition::Phi { .. } => None,
        })
        .collect();
    definitions.sort_unstable();
    assert_eq!(definitions, [site(left, 0), site(right, 0)]);
}

#[test]
fn loop_header_phi_retains_live_in_and_backedge_write() {
    let mut cfg = Cfg::<Instruction>::new();
    let header = cfg.new_block();
    let body = cfg.new_block();
    let exit = cfg.new_block();
    cfg.add_edge(cfg.entry(), header, EdgeKind::Fallthrough);
    cfg.add_edge(header, body, EdgeKind::ConditionalTrue);
    cfg.add_edge(header, exit, EdgeKind::ConditionalFalse);
    cfg.add_edge(body, header, EdgeKind::Back);
    cfg.block_mut(header)
        .push(Instruction::access(Location::A, MemoryAccessKind::Read));
    cfg.block_mut(body)
        .push(Instruction::access(Location::A, MemoryAccessKind::Write));

    let memory = MemorySSA::compute(&cfg, &ExactMemoryAlias);
    let definitions = memory.reaching_definitions(site(header, 0));
    assert_eq!(definitions.len(), 2);
    assert!(definitions.iter().any(|definition| matches!(
        definition,
        MemoryDefinition::LiveIn { class }
            if Some(*class) == memory.class_of(&Location::A)
    )));
    assert!(definitions.iter().any(|definition| matches!(
        definition,
        MemoryDefinition::Event { site: definition_site }
            if *definition_site == site(body, 0)
    )));
}

#[test]
fn read_modify_write_links_both_sides_of_the_data_flow() {
    let cfg = linear_cfg([
        Instruction::access(Location::A, MemoryAccessKind::Write),
        Instruction::access(Location::A, MemoryAccessKind::ReadModifyWrite),
        Instruction::access(Location::A, MemoryAccessKind::Read),
    ]);
    let memory = MemorySSA::compute(&cfg, &ExactMemoryAlias);
    let block = cfg.entry();
    let write = site(block, 0);
    let modify = site(block, 1);
    let read = site(block, 2);

    let event = memory.event(modify).unwrap();
    assert!(event.reads() && event.modifies() && event.writes());
    assert_eq!(
        memory.reaching_definition(modify),
        Some(&MemoryDefinition::Event { site: write })
    );
    assert_eq!(
        memory.reaching_definition(read),
        Some(&MemoryDefinition::Event { site: modify })
    );
    assert_eq!(memory.transitive_readers(write), [modify, read]);
}

#[test]
fn event_order_and_fences_remain_visible_without_changing_memory_state() {
    let instruction = Instruction::with_events(vec![
        memory_access(Location::A, Vec::new(), MemoryAccessKind::Write),
        MemoryEvent::Fence(Fence::Group),
        memory_access(Location::A, Vec::new(), MemoryAccessKind::Read),
    ]);
    let cfg = linear_cfg([instruction]);
    let memory = MemorySSA::compute(&cfg, &ExactMemoryAlias);
    let point = ProgramPoint {
        block: cfg.entry(),
        inst_idx: 0,
    };
    let events = memory.events_at(point);

    assert_eq!(events.len(), 3);
    assert_eq!(events[0].site().event_index(), 0);
    assert!(events[1].is_fence());
    assert_eq!(events[2].site().event_index(), 2);
    assert_eq!(memory.fences().count(), 1);
    assert_eq!(
        memory.operations_at(point),
        crate::MemoryOperations::READ_WRITE
    );
    assert_eq!(
        memory.reaching_definition(events[2].site()),
        Some(&MemoryDefinition::Event {
            site: events[0].site(),
        })
    );
}

#[test]
fn address_dependencies_resolve_to_ordinary_ssa_values() {
    let mut read = Instruction::with_events(vec![memory_access(
        Location::A,
        vec![7],
        MemoryAccessKind::Read,
    )]);
    read.uses.push(7);
    let cfg = linear_cfg([Instruction::plain(Vec::new(), vec![7]), read]);
    let dominators = DominatorTree::compute(&cfg);
    let values = SsaForm::compute(&cfg, &dominators);
    let memory = MemorySSA::compute(&cfg, &ExactMemoryAlias);
    let event = memory.event(site(cfg.entry(), 1)).unwrap();

    assert_eq!(event.location(), Some(&Location::A));
    assert_eq!(event.address_uses(), Some(&[7][..]));
    assert_eq!(
        event.ssa_address_uses(&values).unwrap(),
        values
            .instruction(ProgramPoint {
                block: cfg.entry(),
                inst_idx: 1,
            })
            .unwrap()
            .uses
    );
}
