//! The codec's round trips: every packet decodes into its command and that command writes back
//! the packet's bytes; a packet whose command would not stays raw.

use crate::{decode, encode};
use genie_core::commands::{
    AllBackToWorkCommand, AttackMoveCommand, AutoScoutCommand, BuildCommand, BuildWallCommand,
    BuildingQueueCommand, Command, DeleteCommand, FlareCommand, GameCommand, GateCommand,
    GatherPointCommand, GoBackToWork2Command, GoBackToWorkCommand, Order, OrderCommand,
    PatrolCommand, QueueCommand, ResearchCommand, ResignCommand, SeekShelterCommand, StopCommand,
    TownBellCommand, TradeCommand, TransformObjectCommand, TributeCommand, UnitAiStateCommand,
    UnitList, UnitOrderAction, UnitOrderCommand, UnloadCommand, UnmodelledCommand, Waypoints,
};
use genie_core::orders::{Interact, MoveOrder};
use genie_core::players::tribute::Gift;
use genie_core::{ErrorKind, GenieError};
use genie_rec::de::command as packets;

/// The command of a packet, which writes the packet back.
fn round_trip(packet: &[u8]) -> Command {
    let command = decode(packet).expect("a packet with its player byte");
    assert!(
        !matches!(command, Command::Unmodelled(_)),
        "{command:?} stays raw"
    );
    assert_eq!(encode(&command).expect("a modelled command"), packet);
    command
}

/// Why a packet stays raw, which it writes back as it is.
fn raw(packet: &[u8]) -> GenieError {
    let command = decode(packet).expect("a packet with its player byte");
    assert_eq!(encode(&command).expect("its own bytes"), packet);
    match command {
        Command::Unmodelled(UnmodelledCommand { player, why, .. }) => {
            assert_eq!(player, usize::from(packet[1]));
            why
        }
        other => panic!("{other:?} is modelled"),
    }
}

/// INTERACT's packet of player 2 with `num` units listed and the human byte `human`.
fn interact(num: u16, human: u8, ids: Vec<i32>) -> Vec<u8> {
    packets::Interact {
        commp_id: 2,
        t_id: 500,
        x: 1.0,
        y: 2.0,
        num,
        field_0x12: 1,
        field_0x13: 0,
        field_0x14: 1,
        field_0x15: human,
        field_0x16: 3,
        ids,
    }
    .encode()
}

/// INTERACT keeps its target, point, flags and filter, and its count and every id the packet
/// holds, the last commanded units' `0xffff` too; WORK and RETREAT share its layout.
#[test]
fn an_interact_keeps_its_order_and_units() {
    let order = Interact {
        target: 500,
        point: [1.0, 2.0],
        carry_test: true,
        queued: false,
        direct: true,
        human: true,
        filter: 3,
    };
    assert_eq!(
        round_trip(&interact(2, 1, vec![10, 11, 12])),
        Command::Order(OrderCommand {
            player: 2,
            units: UnitList {
                count: 2,
                ids: vec![10, 11, 12],
            },
            order: Order::Interact(order),
        })
    );
    assert_eq!(
        round_trip(&interact(UnitList::LAST_COMMANDED, 1, vec![])),
        Command::Order(OrderCommand {
            player: 2,
            units: UnitList::last_commanded(),
            order: Order::Interact(order),
        })
    );
    let work = packets::Work {
        commp_id: 4,
        t_id: 600,
        x: 0.0,
        y: 0.0,
        num: 1,
        field_0x12: 0,
        field_0x13: 1,
        field_0x14: 0,
        field_0x15: 0,
        field_0x16: 0,
        ids: vec![20],
    }
    .encode();
    let worked = Interact {
        target: 600,
        queued: true,
        ..Interact::default()
    };
    assert_eq!(
        round_trip(&work),
        Command::Order(OrderCommand {
            player: 4,
            units: UnitList::of(vec![20]),
            order: Order::Work(worked),
        })
    );
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
        field_0x16: 0xffff,
        ids: vec![7],
    }
    .encode();
    assert_eq!(
        round_trip(&retreat),
        Command::Order(OrderCommand {
            player: 1,
            units: UnitList::of(vec![7]),
            order: Order::Retreat(Interact {
                filter: -1,
                ..Interact::default()
            }),
        })
    );
}

/// MOVE keeps its point and flags; FOLLOW and GUARD their target at `+8`.
#[test]
fn move_follow_and_guard_keep_their_orders() {
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
    assert_eq!(
        round_trip(&moved),
        Command::Order(OrderCommand {
            player: 1,
            units: UnitList::of(many),
            order: Order::Move(MoveOrder {
                target: -1,
                point: [5.0, 6.0],
                queued: true,
                direct: false,
                human: true,
                control: false,
            }),
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
    let ordered = |order| {
        Command::Order(OrderCommand {
            player: 1,
            units: UnitList::of(vec![8]),
            order,
        })
    };
    assert_eq!(round_trip(&follow), ordered(Order::Follow(900)));
    assert_eq!(round_trip(&guard), ordered(Order::Guard(901)));
}

/// SET GATHER POINT and `BUILDING_COMMAND_QUEUE` keep their count and every id the packet
/// holds (the runs read them).
#[test]
fn the_buildings_lists_keep_their_counted_ids() {
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
        round_trip(&gather),
        Command::GatherPoint(GatherPointCommand {
            player: 3,
            units: UnitList {
                count: 1,
                ids: vec![30, 31],
            },
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
        gap_0x12: [0, 1, 1, 0, 0, 0],
        ids: vec![32, 33],
    }
    .encode();
    assert_eq!(
        round_trip(&queue),
        Command::CommandQueue(BuildingQueueCommand {
            player: 3,
            units: UnitList::of(vec![32, 33]),
            target: 41,
            point: [3.0, 4.0],
        })
    );
}

/// The units' listed commands keep their count and every id the packet holds, a count past
/// the ids too.
#[test]
fn the_unit_lists_keep_their_counted_ids() {
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
        round_trip(&state),
        Command::UnitAiState(UnitAiStateCommand {
            player: 3,
            units: UnitList::of(vec![34]),
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
        round_trip(&scout),
        Command::AutoScout(AutoScoutCommand {
            player: 3,
            units: UnitList::of(vec![35]),
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
        round_trip(&shelter),
        Command::SeekShelter(SeekShelterCommand {
            player: 3,
            units: UnitList::of(vec![36]),
        })
    );
    let stop = packets::Stop {
        commp_id: 2,
        num: 2,
        gap_0x06: [0; 2],
        ids: vec![50],
    }
    .encode();
    // a count past the ids: STOP's check refuses it in the run
    assert_eq!(
        round_trip(&stop),
        Command::Stop(StopCommand {
            player: 2,
            units: UnitList {
                count: 2,
                ids: vec![50],
            },
        })
    );
}

/// PATROL keeps all ten waypoint slots, the unused ones too; `ATTACK_MOVE` its first slot from
/// its fields and the other nine from its gaps.
#[test]
fn patrol_and_attack_move_keep_every_waypoint_slot() {
    let patrol = packets::Patrol {
        commp_id: 1,
        num: 1,
        gap_0x06: [0; 2],
        field_0x08: 1,
        gap_0x09: [0; 2],
        waypoints_count: 2,
        field_0x0c: 1.5_f32.to_bits(),
        field_0x10: 3.5_f32.to_bits(),
        field_0x14: 9.0_f32.to_bits(),
        field_0x18: 0,
        field_0x1c: 0,
        field_0x20: 0,
        field_0x24: 0,
        field_0x28: 0,
        field_0x2c: 0,
        field_0x30: 0,
        field_0x34: 2.5,
        field_0x38: 4.5,
        field_0x3c: 0.0,
        field_0x40: 0.0,
        field_0x44: 0.0,
        field_0x48: 0.0,
        field_0x4c: 0.0,
        field_0x50: 0.0,
        field_0x54: 0.0,
        field_0x58: -0.0,
        ids: vec![5],
    }
    .encode();
    let Command::Patrol(PatrolCommand {
        waypoints, reset, ..
    }) = round_trip(&patrol)
    else {
        panic!("a patrol")
    };
    assert!(reset);
    assert_eq!(waypoints.used(), Some(&[[1.5, 2.5], [3.5, 4.5]][..]));
    assert_eq!(waypoints.points[2], [9.0, 0.0]);

    let mut gap_0x10 = [0; 36];
    gap_0x10[..4].copy_from_slice(&7.0_f32.to_le_bytes());
    let mut gap_0x38 = [0; 36];
    gap_0x38[32..].copy_from_slice(&8.0_f32.to_le_bytes());
    let attack = packets::AttackMove {
        commp_id: 1,
        num: 1,
        gap_0x06: [0; 2],
        field_0x08: 0,
        field_0x09: 1,
        field_0x0a: 1,
        waypoints_count: 2,
        field_0x0c: 1.0_f32.to_bits(),
        gap_0x10,
        field_0x34: 2.0,
        gap_0x38,
        ids: vec![5],
    }
    .encode();
    assert_eq!(
        round_trip(&attack),
        Command::AttackMove(AttackMoveCommand {
            player: 1,
            units: UnitList::of(vec![5]),
            waypoints: Waypoints {
                count: 2,
                points: std::array::from_fn(|i| match i {
                    0 => [1.0, 2.0],
                    1 => [7.0, 0.0],
                    9 => [0.0, 8.0],
                    _ => [0.0, 0.0],
                }),
            },
            reset: false,
            queued: true,
            ignore_pending: true,
        })
    );
}

/// UNLOAD and `UNIT_ORDER` keep their count, `0xffff` too (their checks refuse it in the run),
/// and `UNIT_ORDER` its point and `+0x14`.
#[test]
fn unload_and_unit_order_keep_their_lists_and_counts() {
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
        round_trip(&unload(1)),
        Command::Unload(UnloadCommand {
            player: 2,
            units: UnitList::of(vec![60]),
            point: [1.0, 1.0],
            flag: 1,
            unit_type: -1,
            sound: true,
        })
    );
    let _ = round_trip(&unload(UnitList::LAST_COMMANDED));
    let order = |num| {
        packets::UnitOrder {
            commp_id: 2,
            num,
            gap_0x06: [0; 2],
            t_id: 70,
            x: 3.0,
            y: 4.0,
            q_id: 71,
            param: -1,
            action: 5,
            field_0x1d: 1,
            field_0x1e: 0,
            gap_0x1f: [1],
            field_0x20: 1,
            ids: vec![61],
        }
        .encode()
    };
    assert_eq!(
        round_trip(&order(1)),
        Command::UnitOrder(UnitOrderCommand {
            player: 2,
            units: UnitList::of(vec![61]),
            target: 70,
            point: [3.0, 4.0],
            q_id: 71,
            through_ai: true,
            nearby: false,
            action: UnitOrderAction::Garrison,
            param: u32::MAX,
            human: true,
        })
    );
    let _ = round_trip(&order(UnitList::LAST_COMMANDED));
}

/// `GO_BACK_TO_WORK2`: the count at `+4` and the ids from `+8`; a tail not of whole ids, or too
/// short for the count, stays raw.
#[test]
fn go_back_to_work_2_keeps_its_counted_ids() {
    let mut rest = vec![2, 0, 0, 0];
    for id in [80_i32, 81, 82] {
        rest.extend(id.to_le_bytes());
    }
    let packet = packets::GoBackToWork2 { commp_id: 1, rest }.encode();
    assert_eq!(
        round_trip(&packet),
        Command::GoBackToWork2(GoBackToWork2Command {
            player: 1,
            units: UnitList {
                count: 2,
                ids: vec![80, 81, 82],
            },
        })
    );
    let ragged = packets::GoBackToWork2 {
        commp_id: 1,
        rest: vec![1, 0, 0, 0, 9],
    }
    .encode();
    assert_eq!(raw(&ragged).kind(), ErrorKind::InvalidInput);
    let short = packets::GoBackToWork2 {
        commp_id: 1,
        rest: vec![1, 0, 0],
    }
    .encode();
    assert_eq!(raw(&short).kind(), ErrorKind::Unsupported);
}

/// BUILD and `BUILD_WALL` keep every field their packet holds.
#[test]
fn build_and_build_wall_keep_their_fields() {
    let build = packets::Build {
        commp_id: 1,
        num: 1,
        gap_0x06: [0; 2],
        x: 10.5,
        y: 11.5,
        o_id: 70,
        q_id: -1,
        field_0x18: 0x2c9,
        frame: 3,
        gap_0x1d: [0],
        field_0x1e: 1,
        field_0x1f: 1,
        ids: vec![9],
    }
    .encode();
    assert_eq!(
        round_trip(&build),
        Command::Build(BuildCommand {
            player: 1,
            master: 70,
            position: [10.5, 11.5],
            target: -1,
            sprite_override: 0x2c9,
            builders: UnitList::of(vec![9]),
            queued: true,
            commits: false,
            frame: 3,
        })
    );
    let wall = packets::BuildWall {
        commp_id: 1,
        num: 1,
        gap_0x06: [0; 2],
        x1: 1,
        y1: 2,
        x2: 3,
        y2: 4,
        o_id: 72,
        q_id: -1,
        gap_0x18: [0; 2],
        field_0x1a: 0,
        field_0x1b: 0,
        ids: vec![9],
    }
    .encode();
    assert_eq!(
        round_trip(&wall),
        Command::BuildWall(BuildWallCommand {
            player: 1,
            master: 72,
            from: [1, 2],
            to: [3, 4],
            target: -1,
            builders: UnitList::of(vec![9]),
            queued: false,
            commits: true,
        })
    );
}

/// QUEUE and RESEARCH keep every field their packet holds.
#[test]
fn queue_and_research_keep_their_fields() {
    let queue = packets::MultiQueue {
        commp_id: 1,
        num: 1,
        gap_0x06: [0; 4],
        b_id: 109,
        t_id: 83,
        count: 5,
        button_location_id: 2,
        ids: vec![9],
    }
    .encode();
    assert_eq!(
        round_trip(&queue),
        Command::Queue(QueueCommand {
            player: 1,
            buildings: UnitList::of(vec![9]),
            building_master: 109,
            unit: 83,
            amount: 5,
            button: 2,
        })
    );
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
        round_trip(&research),
        Command::Research(ResearchCommand {
            player: 1,
            building: 90,
            count: 1,
            buildings: vec![90],
            tech: 22,
            target: -1,
            shift: true,
        })
    );
}

/// The commands of one object or player keep their fields.
#[test]
fn the_object_commands_keep_their_fields() {
    let commands = [
        (
            packets::Resign {
                commp_id: 2,
                dropped: 1,
            }
            .encode(),
            Command::Resign(ResignCommand {
                player: 2,
                dropped: true,
            }),
        ),
        (
            packets::TransformObject {
                commp_id: 2,
                field_0x04: 83,
                field_0x08: 91,
            }
            .encode(),
            Command::TransformObject(TransformObjectCommand {
                player: 2,
                id: 91,
                master: 83,
            }),
        ),
        (
            packets::CancelBuild {
                commp_id: 2,
                u_id: 92,
            }
            .encode(),
            Command::Delete(DeleteCommand { player: 2, id: 92 }),
        ),
        (
            packets::GoBackToWork {
                commp_id: 2,
                u_id: 93,
            }
            .encode(),
            Command::GoBackToWork(GoBackToWorkCommand { player: 2, id: 93 }),
        ),
        (
            packets::AllBackToWork {
                commp_id: 2,
                u_id: 93,
            }
            .encode(),
            Command::AllBackToWork(AllBackToWorkCommand { player: 2, id: 93 }),
        ),
        (
            packets::TownBell {
                commp_id: 2,
                u_id: 94,
                activate: 1,
            }
            .encode(),
            Command::TownBell(TownBellCommand {
                player: 2,
                id: 94,
                activate: true,
            }),
        ),
        (
            packets::Gate {
                commp_id: 2,
                u_id: 95,
            }
            .encode(),
            Command::Gate(GateCommand { player: 2, id: 95 }),
        ),
    ];
    for (packet, command) in commands {
        assert_eq!(round_trip(&packet), command);
    }
}

/// FLARE, the trades and GAME keep their fields.
#[test]
fn flare_trade_and_game_keep_their_fields() {
    let commands = [
        (
            packets::Flare {
                commp_id: 2,
                field_0x04: u32::MAX,
                field_0x08: 7.0,
                field_0x0c: 8.0,
                field_0x10: 4,
                gap_0x11: [1],
                field_0x12: 0,
                field_0x13: 1,
                rest: vec![0, 1],
            }
            .encode(),
            Command::Flare(FlareCommand {
                player: 2,
                target: -1,
                point: [7.0, 8.0],
                players: 4,
                shown: vec![true, false, true, false, true],
            }),
        ),
        (
            packets::BuyCommodity {
                commp_id: 2,
                attr: 1,
                amt: 2,
                u_id: 96,
            }
            .encode(),
            Command::Trade(TradeCommand {
                player: 2,
                attribute: 1,
                times: 2,
                market: 96,
                buy: true,
            }),
        ),
        (
            packets::SellCommodity {
                commp_id: 2,
                attr: 0x0102,
                amt: 1,
                u_id: 97,
            }
            .encode(),
            Command::Trade(TradeCommand {
                player: 2,
                attribute: 0x0102,
                times: 1,
                market: 97,
                buy: false,
            }),
        ),
        (
            packets::Game {
                commp_id: 2,
                field_0x04: 1,
                gap_0x06: [0; 2],
                field_0x08: 3,
                field_0x0a: -1,
                field_0x0c: 1.7,
                field_0x10: 9,
            }
            .encode(),
            Command::Game(GameCommand {
                issuer: 2,
                sub: 1,
                player: 3,
                a: -1,
                speed: 1.7,
                value: 9,
            }),
        ),
    ];
    for (packet, command) in commands {
        assert_eq!(round_trip(&packet), command);
    }
}

/// GIVE ATTRIBUTES keeps its four entries in order and the receiver.
#[test]
fn a_tribute_keeps_its_four_gifts() {
    let tribute = packets::TribeGiveAttributes {
        commp_id: 1,
        amt: 100.0,
        field_0x08: 0.0,
        field_0x0c: 0.0,
        field_0x10: 50.0,
        cost: 30.0,
        field_0x18: 0.0,
        field_0x1c: 0.0,
        field_0x20: 15.0,
        attr: 3,
        field_0x26: 0,
        field_0x28: 0,
        field_0x2a: 1,
        to: 2,
    }
    .encode();
    let gift = |attribute, amount, fee| Gift {
        attribute,
        amount,
        fee,
    };
    assert_eq!(
        round_trip(&tribute),
        Command::Tribute(TributeCommand {
            player: 1,
            to: 2,
            gifts: [
                gift(3, 100.0, 30.0),
                gift(0, 0.0, 0.0),
                gift(0, 0.0, 0.0),
                gift(1, 50.0, 15.0),
            ],
        })
    );
}

/// A packet whose command would not write back its bytes stays raw, with the reason the run
/// stops on: a byte the handler reads as a flag but holds 2, a gap that is not the client's.
#[test]
fn a_packet_its_command_does_not_write_back_stays_raw() {
    let why = raw(&interact(1, 2, vec![10]));
    assert_eq!(why.kind(), ErrorKind::InvalidInput);
    assert_eq!(
        why.reason(),
        "a command packet its command does not write back"
    );
    let queue = packets::BuildingCommandQueue {
        commp_id: 3,
        t_id: 41,
        x: 3.0,
        y: 4.0,
        num: 1,
        gap_0x12: [0; 6],
        ids: vec![32],
    }
    .encode();
    assert_eq!(raw(&queue).kind(), ErrorKind::InvalidInput);
}

/// A kind the engine does not run, and a packet that fits no layout, stay raw; a packet without
/// its player byte is an error.
#[test]
fn an_unmodelled_packet_stays_raw() {
    let pause = packets::Pause {
        commp_id: 5,
        num: 0,
        gap_0x06: [0; 2],
        ids: vec![],
    }
    .encode();
    assert_eq!(raw(&pause).kind(), ErrorKind::Unsupported);
    // an INTERACT one byte short of its fixed part
    let mut short = interact(0, 1, vec![]);
    short.truncate(short.len() - 1);
    short[2] -= 1;
    assert_eq!(raw(&short).kind(), ErrorKind::Unsupported);
    assert_eq!(
        decode(&[0]).map_err(|e| e.kind()),
        Err(ErrorKind::InvalidInput)
    );
}

/// A command whose values its packet cannot hold is an error: a player past a byte, a FLARE of
/// fewer than three players' bytes, more ids than a packet's size holds.
#[test]
fn a_command_its_packet_cannot_hold_is_an_error() {
    let stop = |player, ids: Vec<i32>| {
        Command::Stop(StopCommand {
            player,
            units: UnitList::of(ids),
        })
    };
    let refused = |c: &Command| encode(c).map_err(|e| e.kind());
    assert_eq!(refused(&stop(256, vec![1])), Err(ErrorKind::InvalidInput));
    let most = (usize::from(u16::MAX) + 4 - packets::Stop::FIXED) / 4;
    assert_eq!(encode(&stop(1, vec![0; most])).map(|p| p.len()), Ok(65536));
    assert_eq!(
        refused(&stop(1, vec![0; most + 1])),
        Err(ErrorKind::InvalidInput)
    );
    let flare = Command::Flare(FlareCommand {
        player: 1,
        target: -1,
        point: [0.0; 2],
        players: 2,
        shown: vec![true, false],
    });
    assert_eq!(refused(&flare), Err(ErrorKind::InvalidInput));
}
