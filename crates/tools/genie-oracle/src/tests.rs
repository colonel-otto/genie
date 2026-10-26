use super::*;

/// Word 9 reads a building's production action's state over its top action's (records
/// MarvinBlue/426048706's town centre 7150 after update 7499: its Attack in state 7 and its
/// make action in state 6 count 9 + 6); its type stays the top action's, −1 without one.
#[test]
fn a_building_s_production_state_comes_before_its_top_action_s() {
    let artifact = genie_core::ownership::artifact::Artifact {
        state: 10,
        timer: 0.0,
        gaia: false,
        move_to: None,
        goal: None,
        target: -1,
        walk_to: None,
        enter: None,
    };
    let making = genie_core::production::Production {
        action: Some(genie_core::production::Making::Unit(
            genie_core::production::MakeObj {
                state: 6,
                paid: true,
                master: 83,
                progress: 0.5,
                unique_id: -1,
                button: 0,
                train_time: 25,
            },
        )),
        ..genie_core::production::Production::default()
    };
    let kind = ARTIFACT_TYPE;
    let both = Unit {
        artifact: Some(artifact),
        production: Some(making.clone()),
        ..Unit::default()
    };
    assert_eq!(checkpoint_action(&both), Some(TopAction { kind, state: 6 }));
    let top = Unit {
        artifact: Some(artifact),
        ..Unit::default()
    };
    assert_eq!(checkpoint_action(&top), Some(TopAction { kind, state: 10 }));
    let training = Unit {
        production: Some(making),
        ..Unit::default()
    };
    assert_eq!(
        checkpoint_action(&training),
        Some(TopAction { kind: -1, state: 6 })
    );
    assert_eq!(checkpoint_action(&Unit::default()), None);
}

/// Each differing field of a matched object is one difference with the game's master; an
/// object in one list alone is one too; the lines are the differences' texts.
#[test]
fn differences_name_the_object_its_master_and_the_field() {
    let row = |id, master, hp| ObjectRow {
        id,
        master,
        hp,
        ..ObjectRow::default()
    };
    let slice = [row(1, 83, 25.0), row(2, 83, 25.0), row(4, 4, 1.0)];
    let game = [row(1, 83, 25.0), row(2, 293, 24.0), row(3, 59, 1.0)];
    let d = object_field_differences(&slice, &game);
    let fields: Vec<(i32, i16, &str)> = d
        .iter()
        .map(|d| (d.id, d.master, d.field.as_str()))
        .collect();
    assert_eq!(
        fields,
        [
            (2, 293, "master"),
            (2, 293, "hp"),
            (3, 59, ONLY_IN_THE_GAME),
            (4, 4, ONLY_IN_THE_SLICE)
        ]
    );
    assert_eq!(d[1].to_string(), "2: hp slice 25.0 game 24.0");
    assert!(
        d[2].to_string()
            .starts_with("3: only in the game (ObjectRow {")
    );
    assert_eq!(
        object_differences(&slice, &game),
        d.iter().map(ToString::to_string).collect::<Vec<_>>()
    );
}

/// A PATROL packet of `num` counted units and `count` waypoints (x 10.0 + i, y 20.0 + i).
fn patrol(num: u16, ids: Vec<i32>, count: u8, flag: u8) -> Vec<u8> {
    let x = |i: u32| (10.0 + i as f32).to_bits();
    let y = |i: u32| 20.0 + i as f32;
    de::command::Patrol {
        commp_id: 2,
        num,
        gap_0x06: [0; 2],
        field_0x08: flag,
        gap_0x09: [0; 2],
        waypoints_count: count,
        field_0x0c: x(0),
        field_0x10: x(1),
        field_0x14: x(2),
        field_0x18: x(3),
        field_0x1c: x(4),
        field_0x20: x(5),
        field_0x24: x(6),
        field_0x28: x(7),
        field_0x2c: x(8),
        field_0x30: x(9),
        field_0x34: y(0),
        field_0x38: y(1),
        field_0x3c: y(2),
        field_0x40: y(3),
        field_0x44: y(4),
        field_0x48: y(5),
        field_0x4c: y(6),
        field_0x50: y(7),
        field_0x54: y(8),
        field_0x58: y(9),
        ids,
    }
    .encode()
}

/// PATROL (0x15): the counted units, the counted waypoints (x as an f32's bits) and `+0x8`;
/// no command for a count past the packet's ids, nor without 1 to 10 waypoints.
#[test]
fn a_patrol_packet_decodes_its_units_and_waypoints() {
    let mut d = Decoder::default();
    let c = d.command(&patrol(2, vec![9397, 9398, 9399], 2, 1), &[]);
    assert_eq!(
        c,
        Some(Command::Patrol {
            player: 2,
            ids: vec![9397, 9398],
            waypoints: vec![[10.0, 20.0], [11.0, 21.0]],
            reset: true,
        })
    );
    assert_eq!(d.command(&patrol(4, vec![9397, 9398], 1, 0), &[]), None);
    assert_eq!(d.command(&patrol(1, vec![9397], 0, 0), &[]), None);
    assert_eq!(d.command(&patrol(1, vec![9397], 11, 0), &[]), None);
}

/// `ATTACK_MOVE` (0x21): the counted units, the counted waypoints (the first x as an f32's
/// bits, nine more from `+0x10`; the first y at `+0x34`, nine more from `+0x38`) and the flags
/// `+0x8`, `+0x9`, `+0xa`; no command for a count past the packet's ids, nor without 1 to 10
/// waypoints.
#[test]
fn an_attack_move_packet_decodes_its_units_waypoints_and_flags() {
    let packet = |num: u16, ids: Vec<i32>, count: u8| {
        let mut xs = [0; 36];
        let mut ys = [0; 36];
        xs[..4].copy_from_slice(&11.0f32.to_le_bytes());
        ys[..4].copy_from_slice(&21.0f32.to_le_bytes());
        de::command::AttackMove {
            commp_id: 6,
            num,
            gap_0x06: [0; 2],
            field_0x08: 1,
            field_0x09: 0,
            field_0x0a: 1,
            waypoints_count: count,
            field_0x0c: 10.0f32.to_bits(),
            gap_0x10: xs,
            field_0x34: 20.0,
            gap_0x38: ys,
            ids,
        }
        .encode()
    };
    let mut d = Decoder::default();
    assert_eq!(
        d.command(&packet(1, vec![30065, 1], 2), &[]),
        Some(Command::AttackMove {
            player: 6,
            ids: vec![30065],
            waypoints: vec![[10.0, 20.0], [11.0, 21.0]],
            reset: true,
            queued: false,
            ignore_pending: true,
        })
    );
    assert_eq!(d.command(&packet(3, vec![30065], 1), &[]), None);
    assert_eq!(d.command(&packet(1, vec![30065], 0), &[]), None);
    assert_eq!(d.command(&packet(1, vec![30065], 11), &[]), None);
}

/// `SELL_COMMODITY` and `BUY_COMMODITY`: the commodity is `+0x4`'s low byte, food, wood or
/// stone; another trades nothing.
#[test]
fn a_trade_packet_decodes_food_wood_or_stone_only() {
    assert_eq!(
        trade(2, 0x0101, 3, true),
        Some(Command::Trade {
            player: 2,
            attribute: 1,
            times: 3,
            buy: true,
        })
    );
    assert_eq!(trade(2, 3, 1, false), None);
    let packet = de::command::SellCommodity {
        commp_id: 1,
        attr: 2,
        amt: 5,
        u_id: 77,
    }
    .encode();
    assert_eq!(
        Decoder::default().command(&packet, &[]),
        Some(Command::Trade {
            player: 1,
            attribute: 2,
            times: 5,
            buy: false,
        })
    );
}

/// `ALL_BACK_TO_WORK` (0x8c): the issuer and the object at `+4`.
#[test]
fn an_all_back_to_work_packet_decodes_its_object() {
    let packet = de::command::AllBackToWork {
        commp_id: 3,
        u_id: 4321,
    }
    .encode();
    assert_eq!(
        Decoder::default().command(&packet, &[]),
        Some(Command::AllBackToWork {
            player: 3,
            id: 4321,
        })
    );
}

/// QUEUE (0x81): `num` buildings of the ids that follow (a packet may hold more), the unit
/// at `+0xc`, its count at `+0xe` (below 0 none) and the button at `+0x10`.
#[test]
fn a_queue_packet_decodes_its_buildings() {
    let packet = de::command::MultiQueue {
        commp_id: 1,
        num: 2,
        gap_0x06: [0; 4],
        b_id: 0,
        t_id: 83,
        count: 5,
        button_location_id: 1,
        ids: vec![9, 10, 11],
    }
    .encode();
    let decoded = de::command::MultiQueue::decode(&packet).expect("the layout");
    assert_eq!(queue_buildings(&decoded), [9, 10]);
}

/// GAME (0x67): the issuer (byte 1), the sub-command at `+0x4`, its player at `+0x8` and
/// its arguments at `+0xa`, `+0xc` and `+0x10`, whatever the sub-command.
#[test]
fn a_game_packet_decodes_its_fields() {
    let packet = de::command::Game {
        commp_id: 8,
        field_0x04: 0,
        gap_0x06: [0; 2],
        field_0x08: 8,
        field_0x0a: 3,
        field_0x0c: 3.0,
        field_0x10: 3,
    }
    .encode();
    assert_eq!(
        Decoder::default().command(&packet, &[]),
        Some(Command::Game(genie_core::turn::GameCommand {
            issuer: 8,
            sub: 0,
            player: 8,
            a: 3,
            speed: 3.0,
            value: 3,
        }))
    );
}

/// GIVE ATTRIBUTES (0xc4): the four entries' attributes (`+0x24`..), amounts (`+0x04`..)
/// and fees (`+0x14`..), the receiver at `+0x2c`.
#[test]
fn a_tribute_packet_decodes_its_four_entries() {
    let packet = de::command::TribeGiveAttributes {
        commp_id: 3,
        amt: 100.0,
        field_0x08: 0.0,
        field_0x0c: 0.0,
        field_0x10: 200.0,
        cost: 0.3,
        field_0x18: 0.3,
        field_0x1c: 0.3,
        field_0x20: 0.25,
        attr: 1,
        field_0x26: 0,
        field_0x28: 2,
        field_0x2a: 3,
        to: 5,
    }
    .encode();
    let gift = |attribute, amount, fee| genie_core::players::tribute::Gift {
        attribute,
        amount,
        fee,
    };
    assert_eq!(
        Decoder::default().command(&packet, &[]),
        Some(Command::Tribute {
            player: 3,
            to: 5,
            gifts: [
                gift(1, 100.0, 0.3),
                gift(0, 0.0, 0.3),
                gift(2, 0.0, 0.3),
                gift(3, 200.0, 0.25),
            ],
        })
    );
}
