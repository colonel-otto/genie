use crate::decoder::{Decoder, LAST_COMMANDED, MAX_UNITS};
use genie_core::commands::{Command, Order, OrderUnits};
use genie_rec::de::command as packets;

fn decode(packet: &[u8]) -> Option<Command> {
    Decoder::default().command(packet, &[])
}

fn interact(num: u16, ids: Vec<i32>) -> Vec<u8> {
    packets::Interact {
        commp_id: 2,
        t_id: 500,
        x: 1.0,
        y: 2.0,
        num,
        field_0x12: 1,
        field_0x13: 0,
        field_0x14: 1,
        field_0x15: 1,
        field_0x16: 3,
        ids,
    }
    .encode()
}

/// INTERACT orders the units its count takes and notes them, at most 60, for a primed run; a
/// later count of `0xffff` orders the last commanded units, which the world keeps.
#[test]
fn an_interact_orders_its_units_and_a_later_one_the_last_commanded() {
    let mut d = Decoder::default();
    let order = Order::Interact(genie_core::orders::Interact {
        target: 500,
        point: [1.0, 2.0],
        carry_test: true,
        queued: false,
        direct: true,
        human: true,
        filter: 3,
    });
    let ordered = |units| {
        Some(Command::Order {
            player: 2,
            units,
            order: Some(order),
        })
    };
    assert_eq!(
        d.command(&interact(2, vec![10, 11, 12]), &[]),
        ordered(OrderUnits::Listed(vec![10, 11]))
    );
    assert_eq!(d.last_commanded.get(&2), Some(&vec![10, 11]));
    assert_eq!(
        d.command(&interact(LAST_COMMANDED, vec![]), &[]),
        ordered(OrderUnits::LastCommanded)
    );
    assert_eq!(d.last_commanded.get(&2), Some(&vec![10, 11]));
}

/// WORK orders its units (a run the world does not model) and notes them.
#[test]
fn a_work_command_orders_its_units_and_notes_them() {
    let mut d = Decoder::default();
    let work = packets::Work {
        commp_id: 4,
        t_id: 600,
        x: 0.0,
        y: 0.0,
        num: 1,
        field_0x12: 0,
        field_0x13: 0,
        field_0x14: 0,
        field_0x15: 0,
        field_0x16: 0,
        ids: vec![20],
    }
    .encode();
    assert_eq!(
        d.command(&work, &[]),
        Some(Command::Order {
            player: 4,
            units: OrderUnits::Listed(vec![20]),
            order: Some(Order::Work),
        })
    );
    assert_eq!(d.last_commanded.get(&4), Some(&vec![20]));
}

/// MOVE, RETREAT, FOLLOW and GUARD order their units: MOVE with its point, RETREAT without an
/// order, FOLLOW and GUARD with the target at `+8`. The list keeps its count's units (the
/// world caps them); the decoder notes at most 60 of MOVE's and none of FOLLOW's or GUARD's.
#[test]
fn move_retreat_follow_and_guard_order_their_units() {
    let many: Vec<i32> = (0..70).collect();
    let moved = packets::Move {
        commp_id: 1,
        t_id: -1,
        x: 5.0,
        y: 6.0,
        num: 70,
        field_0x12: 0,
        field_0x13: 1,
        field_0x14: 0,
        field_0x15: 1,
        gap_0x16: [0; 2],
        ids: many.clone(),
    }
    .encode();
    let order = Order::Move(genie_core::orders::MoveOrder {
        target: -1,
        point: [5.0, 6.0],
        queued: true,
        direct: false,
        human: true,
        control: false,
    });
    let mut d = Decoder::default();
    assert_eq!(
        d.command(&moved, &[]),
        Some(Command::Order {
            player: 1,
            units: OrderUnits::Listed(many.clone()),
            order: Some(order),
        })
    );
    assert_eq!(d.last_commanded.get(&1), Some(&many[..MAX_UNITS].to_vec()));
    let retreat = packets::Retreat {
        commp_id: 1,
        t_id: 0,
        x: 0.0,
        y: 0.0,
        num: 1,
        field_0x12: 0,
        field_0x13: 0,
        field_0x14: 0,
        field_0x15: 0,
        field_0x16: 0,
        ids: vec![7],
    }
    .encode();
    assert_eq!(
        decode(&retreat),
        Some(Command::Order {
            player: 1,
            units: OrderUnits::Listed(vec![7]),
            order: None,
        })
    );
    let follow = packets::Follow {
        commp_id: 1,
        num: 1,
        gap_0x06: [0; 2],
        target: 900,
        ids: vec![8],
    }
    .encode();
    let guard = packets::Guard {
        commp_id: 1,
        num: 1,
        gap_0x06: [0; 2],
        target: 901,
        ids: vec![8],
    }
    .encode();
    let ordered = |order| Command::Order {
        player: 1,
        units: OrderUnits::Listed(vec![8]),
        order: Some(order),
    };
    let mut d = Decoder::default();
    assert_eq!(d.command(&follow, &[]), Some(ordered(Order::Follow(900))));
    assert_eq!(d.command(&guard, &[]), Some(ordered(Order::Guard(901))));
    assert!(d.last_commanded.is_empty());
}

/// The listed commands keep the counted ids, at most 60.
#[test]
fn the_listed_commands_keep_their_counted_ids() {
    let gather = packets::SetGatherPoint {
        commp_id: 3,
        num: 1,
        gap_0x06: [0; 2],
        x: 1.0,
        y: 2.0,
        t_id: 40,
        tm_id: 83,
        field_0x18: 1,
        ids: vec![30, 31],
    }
    .encode();
    assert_eq!(
        decode(&gather),
        Some(Command::GatherPoint {
            player: 3,
            ids: vec![30],
            point: [1.0, 2.0],
            target: 40,
            target_master: 83,
            own: true,
        })
    );
    let queue = packets::BuildingCommandQueue {
        commp_id: 3,
        t_id: 41,
        x: 3.0,
        y: 4.0,
        num: 2,
        gap_0x12: [0; 6],
        ids: vec![32, 33],
    }
    .encode();
    assert_eq!(
        decode(&queue),
        Some(Command::CommandQueue {
            player: 3,
            ids: vec![32, 33],
            target: 41,
            point: [3.0, 4.0],
        })
    );
    let state = packets::UnitAiState {
        commp_id: 3,
        num: 1,
        gap_0x06: [0; 2],
        state: 2,
        gap_0x09: [0; 3],
        ids: vec![34],
    }
    .encode();
    assert_eq!(
        decode(&state),
        Some(Command::UnitAiState {
            player: 3,
            ids: vec![34],
            state: 2,
        })
    );
    let scout = packets::AutoScout {
        commp_id: 3,
        field_0x04: 1,
        gap_0x06: [0; 2],
        ids: vec![35],
    }
    .encode();
    assert_eq!(
        decode(&scout),
        Some(Command::AutoScout {
            player: 3,
            ids: vec![35],
        })
    );
    let shelter = packets::SeekShelter {
        commp_id: 3,
        num: 1,
        gap_0x06: [0; 2],
        ids: vec![36],
    }
    .encode();
    assert_eq!(
        decode(&shelter),
        Some(Command::SeekShelter {
            player: 3,
            ids: vec![36],
        })
    );
}

/// STOP: no command for a count past the ids the packet holds.
#[test]
fn a_stop_with_too_few_ids_does_nothing() {
    let stop = |num| {
        packets::Stop {
            commp_id: 2,
            num,
            gap_0x06: [0; 2],
            ids: vec![50],
        }
        .encode()
    };
    assert_eq!(
        decode(&stop(1)),
        Some(Command::Stop {
            player: 2,
            ids: vec![50],
        })
    );
    assert_eq!(decode(&stop(2)), None);
}

/// UNLOAD and `UNIT_ORDER` with the count `0xffff` (the last commanded units) are not modelled.
#[test]
fn unload_and_unit_order_decode_their_lists_but_not_the_last_commanded() {
    let unload = |num| {
        packets::Unload {
            commp_id: 2,
            num,
            gap_0x06: [0; 2],
            x: 1.0,
            y: 1.0,
            unit_type: -1,
            flag: 1,
            gap_0x15: [0; 3],
            field_0x18: 1,
            ids: vec![60],
        }
        .encode()
    };
    assert_eq!(
        decode(&unload(1)),
        Some(Command::Unload {
            player: 2,
            ids: vec![60],
            point: [1.0, 1.0],
            flag: 1,
            unit_type: -1,
            sound: true,
        })
    );
    assert!(matches!(
        decode(&unload(LAST_COMMANDED)),
        Some(Command::Unmodelled { player: 2, .. })
    ));
    let order = |num| {
        packets::UnitOrder {
            commp_id: 2,
            num,
            gap_0x06: [0; 2],
            t_id: 70,
            x: 0.0,
            y: 0.0,
            q_id: 0,
            param: -1,
            action: 5,
            field_0x1d: 1,
            field_0x1e: 0,
            gap_0x1f: [0; 1],
            field_0x20: 1,
            ids: vec![61],
        }
        .encode()
    };
    assert_eq!(
        decode(&order(1)),
        Some(Command::UnitOrder {
            player: 2,
            ids: vec![61],
            target: 70,
            through_ai: true,
            nearby: false,
            action: 5,
            param: u32::MAX,
            human: true,
        })
    );
    assert!(matches!(
        decode(&order(LAST_COMMANDED)),
        Some(Command::Unmodelled { player: 2, .. })
    ));
}

/// `GO_BACK_TO_WORK2`: the count at `+4` and the ids from `+8`, at most 60.
#[test]
fn go_back_to_work_2_reads_its_counted_ids() {
    let mut rest = vec![2, 0, 0, 0];
    for id in [80_i32, 81, 82] {
        rest.extend(id.to_le_bytes());
    }
    let packet = packets::GoBackToWork2 { commp_id: 1, rest }.encode();
    assert_eq!(
        decode(&packet),
        Some(Command::GoBackToWork {
            player: 1,
            ids: vec![80, 81],
        })
    );
    let empty = packets::GoBackToWork2 {
        commp_id: 1,
        rest: vec![],
    }
    .encode();
    assert_eq!(
        decode(&empty),
        Some(Command::GoBackToWork {
            player: 1,
            ids: vec![],
        })
    );
}

/// The commands of one object or player.
#[test]
fn the_single_commands_decode_their_fields() {
    let research = packets::Research {
        commp_id: 1,
        u_id: 90,
        field_0x08: 1,
        t_id: 22,
        q_id: -1,
        field_0x10: 1,
        ids: vec![90],
    }
    .encode();
    assert_eq!(
        decode(&research),
        Some(Command::Research {
            player: 1,
            building: 90,
            count: 1,
            buildings: vec![90],
            tech: 22,
            target: -1,
            shift: true,
        })
    );
    let resign = packets::Resign {
        commp_id: 2,
        dropped: 1,
    }
    .encode();
    assert_eq!(
        decode(&resign),
        Some(Command::Resign {
            player: 2,
            dropped: true,
        })
    );
    let transform = packets::TransformObject {
        commp_id: 2,
        field_0x04: 83,
        field_0x08: 91,
    }
    .encode();
    assert_eq!(
        decode(&transform),
        Some(Command::TransformObject {
            player: 2,
            id: 91,
            master: 83,
        })
    );
    let cancel = packets::CancelBuild {
        commp_id: 2,
        u_id: 92,
    }
    .encode();
    assert_eq!(decode(&cancel), Some(Command::Delete { player: 2, id: 92 }));
    let back = packets::GoBackToWork {
        commp_id: 2,
        u_id: 93,
    }
    .encode();
    assert_eq!(
        decode(&back),
        Some(Command::GoBackToWork {
            player: 2,
            ids: vec![93],
        })
    );
    let bell = packets::TownBell {
        commp_id: 2,
        u_id: 94,
        activate: 1,
    }
    .encode();
    assert_eq!(
        decode(&bell),
        Some(Command::TownBell {
            player: 2,
            id: 94,
            activate: true,
        })
    );
    let gate = packets::Gate {
        commp_id: 2,
        u_id: 95,
    }
    .encode();
    assert_eq!(decode(&gate), Some(Command::Gate { player: 2, id: 95 }));
    let flare = packets::Flare {
        commp_id: 2,
        field_0x04: u32::MAX,
        field_0x08: 7.0,
        field_0x0c: 8.0,
        field_0x10: 0b110,
        gap_0x11: [0; 1],
        field_0x12: 0,
        field_0x13: 0,
        rest: vec![],
    }
    .encode();
    assert_eq!(
        decode(&flare),
        Some(Command::Flare {
            player: 2,
            target: -1,
            point: [7.0, 8.0],
            players: 0b110,
        })
    );
    let buy = packets::BuyCommodity {
        commp_id: 2,
        attr: 1,
        amt: 2,
        u_id: 96,
    }
    .encode();
    assert_eq!(
        decode(&buy),
        Some(Command::Trade {
            player: 2,
            attribute: 1,
            times: 2,
            buy: true,
        })
    );
}

/// A command the decoder does not model stops its player (byte 1); a packet without one gives
/// no command.
#[test]
fn an_unmodelled_command_stops_its_player() {
    let pause = packets::Pause {
        commp_id: 5,
        num: 0,
        gap_0x06: [0; 2],
        ids: vec![],
    }
    .encode();
    assert!(matches!(
        decode(&pause),
        Some(Command::Unmodelled { player: 5, .. })
    ));
    assert_eq!(decode(&[]), None);
}
