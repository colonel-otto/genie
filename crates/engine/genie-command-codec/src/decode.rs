//! A packet into its command: every field the packet holds, in the command's terms.

use crate::encode::encode;
use crate::names::unmodelled_command;
use genie_core::GenieError;
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
use genie_rec::de::command::{self as packets, Command as Packet};

/// The command a packet holds.
///
/// A packet of a kind the engine does not run, one that fits no layout, and one whose command
/// would not write back its bytes ([`encode`]) stay raw: an [`UnmodelledCommand`] whose run
/// stops the issuing player.
///
/// # Errors
///
/// A packet too short for its player byte (byte 1).
pub fn decode(packet: &[u8]) -> Result<Command, GenieError> {
    let player = usize::from(*packet.get(1).ok_or(GenieError::invalid_input(
        "a command packet without its player byte",
    ))?);
    let decoded = Packet::decode(packet);
    let Some(command) = typed(&decoded) else {
        return Ok(raw(
            player,
            packet,
            GenieError::unsupported(unmodelled_command(&decoded)),
        ));
    };
    if encode(&command).is_ok_and(|bytes| bytes == packet) {
        return Ok(command);
    }
    Ok(raw(
        player,
        packet,
        GenieError::invalid_input("a command packet its command does not write back")
            .with("command", unmodelled_command(&decoded)),
    ))
}

/// The packet kept as it is, with why it does not run.
fn raw(player: usize, packet: &[u8], why: GenieError) -> Command {
    Command::Unmodelled(UnmodelledCommand {
        player,
        packet: packet.to_vec(),
        why,
    })
}

/// A word's low short, which a handler reads where the packet holds a word (the masters).
const fn short(word: i32) -> i16 {
    let [low, high, ..] = word.to_le_bytes();
    i16::from_le_bytes([low, high])
}

/// A packet's unit list: its count and every id it holds (the runs read the first `count`).
fn listed(count: u16, ids: &[i32]) -> UnitList {
    UnitList {
        count,
        ids: ids.to_vec(),
    }
}

/// The command of a packet the engine runs, None for any other.
fn typed(packet: &Packet) -> Option<Command> {
    orders(packet)
        .or_else(|| listed_command(packet))
        .or_else(|| building_command(packet))
        .or_else(|| single_command(packet))
}

/// INTERACT's layout, which WORK and RETREAT share: the target, the point and the flags.
macro_rules! interact {
    ($c:expr) => {
        Interact {
            target: $c.t_id,
            point: [$c.x, $c.y],
            carry_test: $c.field_0x12 != 0,
            queued: $c.field_0x13 != 0,
            direct: $c.field_0x14 != 0,
            human: $c.field_0x15 == 1,
            filter: $c.field_0x16.cast_signed(),
        }
    };
}

/// INTERACT, WORK, MOVE and RETREAT, whose units the orders' reader reads (`FUN_140e6b290`),
/// and FOLLOW and GUARD, which list theirs.
fn orders(packet: &Packet) -> Option<Command> {
    let (player, units, order) = match packet {
        Packet::Interact(c) => (
            c.commp_id,
            listed(c.num, &c.ids),
            Order::Interact(interact!(c)),
        ),
        Packet::Work(c) => (c.commp_id, listed(c.num, &c.ids), Order::Work(interact!(c))),
        Packet::Retreat(c) => (
            c.commp_id,
            listed(c.num, &c.ids),
            Order::Retreat(interact!(c)),
        ),
        Packet::Move(c) => {
            let order = MoveOrder {
                target: c.t_id,
                point: [c.x, c.y],
                queued: c.field_0x13 != 0,
                direct: c.field_0x14 != 0,
                human: c.field_0x15 == 1,
                control: c.field_0x12 != 0,
            };
            (c.commp_id, listed(c.num, &c.ids), Order::Move(order))
        }
        Packet::Follow(c) => (c.commp_id, listed(c.num, &c.ids), Order::Follow(c.target)),
        Packet::Guard(c) => (c.commp_id, listed(c.num, &c.ids), Order::Guard(c.target)),
        _ => return None,
    };
    Some(Command::Order(OrderCommand {
        player: usize::from(player),
        units,
        order,
    }))
}

/// The commands to the objects listed that do not go through the orders' reader.
fn listed_command(packet: &Packet) -> Option<Command> {
    Some(match packet {
        Packet::Stop(c) => Command::Stop(StopCommand {
            player: usize::from(c.commp_id),
            units: listed(c.num, &c.ids),
        }),
        Packet::UnitAiState(c) => Command::UnitAiState(UnitAiStateCommand {
            player: usize::from(c.commp_id),
            units: listed(c.num, &c.ids),
            state: c.state,
        }),
        Packet::AutoScout(c) => Command::AutoScout(AutoScoutCommand {
            player: usize::from(c.commp_id),
            units: listed(c.field_0x04, &c.ids),
        }),
        Packet::SeekShelter(c) => Command::SeekShelter(SeekShelterCommand {
            player: usize::from(c.commp_id),
            units: listed(c.num, &c.ids),
        }),
        Packet::Patrol(c) => patrol(c),
        Packet::AttackMove(c) => attack_move(c),
        Packet::Unload(c) => Command::Unload(UnloadCommand {
            player: usize::from(c.commp_id),
            units: listed(c.num, &c.ids),
            point: [c.x, c.y],
            flag: c.flag,
            unit_type: c.unit_type,
            sound: c.field_0x18 != 0,
        }),
        Packet::UnitOrder(c) => Command::UnitOrder(UnitOrderCommand {
            player: usize::from(c.commp_id),
            units: listed(c.num, &c.ids),
            target: c.t_id,
            point: [c.x, c.y],
            q_id: c.q_id,
            through_ai: c.field_0x1d != 0,
            nearby: c.field_0x1e != 0,
            action: UnitOrderAction::of(c.action),
            param: c.param.cast_unsigned(),
            human: c.field_0x20 == 1,
        }),
        Packet::GoBackToWork2(c) => Command::GoBackToWork2(GoBackToWork2Command {
            player: usize::from(c.commp_id),
            units: go_back_to_work_units(&c.rest)?,
        }),
        _ => return None,
    })
}

/// PATROL (0x15): the units, the waypoints' count (`+0xb`) and slots (x from `+0xc`, each an
/// f32's bits; y from `+0x34`) and `+0x8`.
fn patrol(c: &packets::Patrol) -> Command {
    let xs = [
        c.field_0x0c,
        c.field_0x10,
        c.field_0x14,
        c.field_0x18,
        c.field_0x1c,
        c.field_0x20,
        c.field_0x24,
        c.field_0x28,
        c.field_0x2c,
        c.field_0x30,
    ];
    let ys = [
        c.field_0x34,
        c.field_0x38,
        c.field_0x3c,
        c.field_0x40,
        c.field_0x44,
        c.field_0x48,
        c.field_0x4c,
        c.field_0x50,
        c.field_0x54,
        c.field_0x58,
    ];
    Command::Patrol(PatrolCommand {
        player: usize::from(c.commp_id),
        units: listed(c.num, &c.ids),
        waypoints: Waypoints {
            count: c.waypoints_count,
            points: std::array::from_fn(|i| [f32::from_bits(xs[i]), ys[i]]),
        },
        reset: c.field_0x08 != 0,
    })
}

/// `ATTACK_MOVE` (0x21): PATROL's layout, the first x as an f32's bits and nine more from
/// `+0x10`, the first y at `+0x34` and nine more from `+0x38`; its flags at `+0x8`..`+0xa`.
fn attack_move(c: &packets::AttackMove) -> Command {
    let word = |b: &[u8; 36], i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let points = std::array::from_fn(|i| match i {
        0 => [f32::from_bits(c.field_0x0c), c.field_0x34],
        _ => [
            word(&c.gap_0x10, 4 * (i - 1)),
            word(&c.gap_0x38, 4 * (i - 1)),
        ],
    });
    Command::AttackMove(AttackMoveCommand {
        player: usize::from(c.commp_id),
        units: listed(c.num, &c.ids),
        waypoints: Waypoints {
            count: c.waypoints_count,
            points,
        },
        reset: c.field_0x08 != 0,
        queued: c.field_0x09 != 0,
        ignore_pending: c.field_0x0a != 0,
    })
}

/// `GO_BACK_TO_WORK2`'s units: the count is the short at the start of `rest` (+4), the ids
/// follow from +8 (the last bytes of a tail not of whole ids are none, so its command does not
/// write them back); None for a tail too short for the count and the two bytes after it.
fn go_back_to_work_units(rest: &[u8]) -> Option<UnitList> {
    let count = u16::from_le_bytes([*rest.first()?, *rest.get(1)?]);
    let (ids, _) = rest.get(4..)?.as_chunks::<4>();
    Some(UnitList {
        count,
        ids: ids.iter().map(|b| i32::from_le_bytes(*b)).collect(),
    })
}

/// The commands that place or queue at buildings: BUILD, `BUILD_WALL`, QUEUE, RESEARCH, SET
/// GATHER POINT and `BUILDING_COMMAND_QUEUE`.
fn building_command(packet: &Packet) -> Option<Command> {
    Some(match packet {
        Packet::Build(c) => Command::Build(BuildCommand {
            player: usize::from(c.commp_id),
            master: short(c.o_id),
            position: [c.x, c.y],
            target: c.q_id,
            sprite_override: c.field_0x18.cast_signed(),
            builders: listed(c.num, &c.ids),
            queued: c.field_0x1e != 0,
            commits: c.field_0x1f == 0,
            frame: c.frame,
        }),
        Packet::BuildWall(c) => Command::BuildWall(BuildWallCommand {
            player: usize::from(c.commp_id),
            master: short(c.o_id),
            from: [c.x1, c.y1],
            to: [c.x2, c.y2],
            target: c.q_id,
            builders: listed(c.num, &c.ids),
            queued: c.field_0x1a != 0,
            commits: c.field_0x1b == 0,
        }),
        Packet::MultiQueue(c) => Command::Queue(QueueCommand {
            player: usize::from(c.commp_id),
            buildings: listed(c.num, &c.ids),
            building_master: c.b_id,
            unit: c.t_id,
            amount: c.count,
            button: c.button_location_id,
        }),
        Packet::Research(c) => Command::Research(ResearchCommand {
            player: usize::from(c.commp_id),
            building: c.u_id,
            count: c.field_0x08.cast_signed(),
            buildings: c.ids.clone(),
            tech: c.t_id,
            target: c.q_id,
            shift: c.field_0x10 != 0,
        }),
        Packet::SetGatherPoint(c) => Command::GatherPoint(GatherPointCommand {
            player: usize::from(c.commp_id),
            units: listed(c.num, &c.ids),
            point: [c.x, c.y],
            target: c.t_id,
            target_master: short(c.tm_id),
            own: c.field_0x18 != 0,
        }),
        Packet::BuildingCommandQueue(c) => Command::CommandQueue(BuildingQueueCommand {
            player: usize::from(c.commp_id),
            units: listed(c.num, &c.ids),
            target: c.t_id,
            point: [c.x, c.y],
        }),
        _ => return None,
    })
}

/// The commands of one object, one player or the game.
fn single_command(packet: &Packet) -> Option<Command> {
    Some(match packet {
        Packet::Resign(c) => Command::Resign(ResignCommand {
            player: usize::from(c.commp_id),
            dropped: c.dropped != 0,
        }),
        Packet::TransformObject(c) => Command::TransformObject(TransformObjectCommand {
            player: usize::from(c.commp_id),
            id: c.field_0x08.cast_signed(),
            master: short(c.field_0x04),
        }),
        Packet::CancelBuild(c) => Command::Delete(DeleteCommand {
            player: usize::from(c.commp_id),
            id: c.u_id,
        }),
        Packet::GoBackToWork(c) => Command::GoBackToWork(GoBackToWorkCommand {
            player: usize::from(c.commp_id),
            id: c.u_id,
        }),
        Packet::AllBackToWork(c) => Command::AllBackToWork(AllBackToWorkCommand {
            player: usize::from(c.commp_id),
            id: c.u_id,
        }),
        Packet::TownBell(c) => Command::TownBell(TownBellCommand {
            player: usize::from(c.commp_id),
            id: c.u_id,
            activate: c.activate != 0,
        }),
        Packet::Gate(c) => Command::Gate(GateCommand {
            player: usize::from(c.commp_id),
            id: c.u_id,
        }),
        Packet::TribeGiveAttributes(c) => tribute(c),
        Packet::SellCommodity(c) => trade(c.commp_id, c.attr, c.amt, c.u_id, false),
        Packet::BuyCommodity(c) => trade(c.commp_id, c.attr, c.amt, c.u_id, true),
        Packet::Flare(c) => Command::Flare(FlareCommand {
            player: usize::from(c.commp_id),
            target: c.field_0x04.cast_signed(),
            point: [c.field_0x08, c.field_0x0c],
            players: c.field_0x10,
            shown: [c.gap_0x11[0], c.field_0x12, c.field_0x13]
                .iter()
                .chain(&c.rest)
                .map(|&b| b != 0)
                .collect(),
        }),
        Packet::Game(g) => Command::Game(GameCommand {
            issuer: usize::from(g.commp_id),
            sub: g.field_0x04,
            player: g.field_0x08,
            a: g.field_0x0a,
            speed: g.field_0x0c,
            value: g.field_0x10,
        }),
        _ => return None,
    })
}

/// `SELL_COMMODITY` (0x7a) or `BUY_COMMODITY` (0x7b): the attribute (`+0x4`), the lots
/// (`+0x6`) and the market (`+0x8`).
fn trade(player: u8, attribute: u16, times: u16, market: i32, buy: bool) -> Command {
    Command::Trade(TradeCommand {
        player: usize::from(player),
        attribute,
        times,
        market,
        buy,
    })
}

/// GIVE ATTRIBUTES (0xc4): the four entries' attributes (`+0x24`..), amounts (`+0x04`..) and
/// fees (`+0x14`..), the receiver at `+0x2c`.
fn tribute(c: &packets::TribeGiveAttributes) -> Command {
    let gift = |attribute, amount, fee| Gift {
        attribute,
        amount,
        fee,
    };
    Command::Tribute(TributeCommand {
        player: usize::from(c.commp_id),
        to: usize::from(c.to),
        gifts: [
            gift(c.attr, c.amt, c.cost),
            gift(c.field_0x26, c.field_0x08, c.field_0x18),
            gift(c.field_0x28, c.field_0x0c, c.field_0x1c),
            gift(c.field_0x2a, c.field_0x10, c.field_0x20),
        ],
    })
}
