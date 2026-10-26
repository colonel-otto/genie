//! The recorded commands the slice runs ([`Decoder`]), and why it stops on one it does not
//! decode.

use crate::{
    Frame, attack_move_command, build_command, build_wall_command, patrol_command, queue_command,
    trade,
};
use genie_core::GenieError;
use genie_core::commands::Command;
use genie_rec::de;
use genie_save::PlayerSection;

/// Why the slice stops on a command it does not decode, by the command's name.
pub(super) const fn unmodelled_command(p: &de::command::Command) -> &'static str {
    use genie_rec::de::command::Command as Packet;
    macro_rules! named {
        ($($v:ident),* $(,)?) => {
            match p {
                $(Packet::$v(_) => concat!("a command the slice does not model: ", stringify!($v)),)*
                _ => "a command the slice does not model",
            }
        };
    }
    named!(
        Stop,
        Create,
        AddAttribute,
        RgeGiveAttribute,
        AiOrder,
        Resign,
        AddWaypoint,
        Pause,
        GroupWaypoint,
        GroupAiOrder,
        UnitAiState,
        Guard,
        Patrol,
        Scout,
        FormFormation,
        BreakFormation,
        WheelFormation,
        AboutFaceFormation,
        Save,
        FormationParameters,
        AutoFormations,
        LockFormation,
        GroupMultiWaypoints,
        Chapter,
        AttackMove,
        AttackMoveTarget,
        UnitTypeAiState,
        AutoScout,
        StopAll,
        TransformObject,
        Trigger,
        TransformObjects,
        SeekShelter,
        BuildingCommandQueue,
        FireActiveAbility,
        RgeGiveAttributes,
        Make,
        Game,
        Explore,
        BuildWall,
        CancelBuild,
        AttackGround,
        TribeGiveAttribute,
        TradeAttribute,
        Repair,
        Unload,
        Gate,
        Flare,
        Special,
        UnitOrder,
        Diplomacy,
        Queue,
        SetRetreatPoint,
        SellCommodity,
        BuyCommodity,
        OffboardTrade,
        UnitTransform,
        DropRelic,
        TownBell,
        GoBackToWork,
        SetGatherState,
        DeleteObjects,
        ResetBuildings,
        MultiGate,
        GoBackToWork2,
        ChangePlayerName,
        SendAiSignal,
        AiEffectAmount,
        AiEffectPercent,
        AllBackToWork,
        TribeGiveAttributes,
        Unknown,
    )
}

/// A count of `0xffff` names the comm player's last commanded units.
pub(super) const LAST_COMMANDED: u16 = 0xffff;

/// The unit-list reader keeps at most this many units of a human player's command (500 for the
/// players `FUN_141659cf0` accepts).
pub(super) const MAX_UNITS: usize = 60;

/// Decodes what the slice runs of the recorded commands, in their order (the packets through
/// genie-rec's generated layouts). It remembers each comm player's last commanded units, as the
/// game's unit-list reader (`FUN_140e6b290`) does for INTERACT, WORK, MOVE and RETREAT. BUILD,
/// QUEUE, SET GATHER POINT and the orders run; every other command is left out (see [`GROUP_COMMANDS`] for what
/// that may miss).
///
/// The player byte is the comm player, which the handlers map to a world player through the lobby
/// (`FUN_140e6b210`); the slice takes the two as equal.
///
/// [`GROUP_COMMANDS`]: crate::header_runs::GROUP_COMMANDS
#[derive(Default)]
pub(super) struct Decoder {
    pub(super) last_commanded: std::collections::HashMap<u8, Vec<i32>>,
}

impl Decoder {
    /// The commands of the frames a save already played: decoded for the unit lists they leave
    /// (a later command's 'last commanded units'), not run.
    pub(super) fn prime(&mut self, frames: &[Frame<'_>], sections: &[PlayerSection<'_>]) {
        for frame in frames {
            for p in &frame.commands {
                let _ = self.command(p, sections);
            }
        }
    }

    pub(super) fn command(
        &mut self,
        packet: &[u8],
        sections: &[PlayerSection<'_>],
    ) -> Option<Command> {
        use genie_rec::de::command::Command as Packet;
        // INTERACT, MOVE and RETREAT reset their units' idle timeouts (`FUN_140e6cfc0`), WORK not
        let (player, num, ids, order, interact) = match Packet::decode(packet) {
            Packet::Build(build) => return Some(build_command(&build, sections)),
            Packet::MultiQueue(queue) => return Some(queue_command(&queue, sections)),
            // RESEARCH (0x65): the handler reads the building count at +0x8 as a short
            Packet::Research(c) => {
                return Some(Command::Research {
                    player: c.commp_id as usize,
                    building: c.u_id,
                    count: c.field_0x08 as i16,
                    buildings: c.ids.clone(),
                    tech: c.t_id,
                    target: c.q_id,
                    shift: c.field_0x10 != 0,
                });
            }
            Packet::SetGatherPoint(c) => {
                return Some(Command::GatherPoint {
                    player: c.commp_id as usize,
                    ids: c
                        .ids
                        .iter()
                        .copied()
                        .take((c.num as usize).min(MAX_UNITS))
                        .collect(),
                    point: [c.x, c.y],
                    target: c.t_id,
                    target_master: c.tm_id as i16,
                    own: c.field_0x18 != 0,
                });
            }
            Packet::Interact(c) => {
                let order = genie_core::orders::Interact {
                    target: c.t_id,
                    point: [c.x, c.y],
                    carry_test: c.field_0x12 != 0,
                    queued: c.field_0x13 != 0,
                    direct: c.field_0x14 != 0,
                    human: c.field_0x15 == 1,
                    filter: c.field_0x16 as i16,
                };
                let order = genie_core::commands::Order::Interact(order);
                (c.commp_id, c.num, c.ids, true, Some(order))
            }
            Packet::Work(c) => (c.commp_id, c.num, c.ids, false, None),
            Packet::Move(c) => {
                let order = genie_core::orders::MoveOrder {
                    target: c.t_id,
                    point: [c.x, c.y],
                    queued: c.field_0x13 != 0,
                    direct: c.field_0x14 != 0,
                    human: c.field_0x15 == 1,
                    control: c.field_0x12 != 0,
                };
                let order = genie_core::commands::Order::Move(order);
                (c.commp_id, c.num, c.ids, true, Some(order))
            }
            Packet::Retreat(c) => (c.commp_id, c.num, c.ids, true, None),
            // FOLLOW: its units as MOVE's, the target at +8
            Packet::Follow(c) => {
                let order = genie_core::commands::Order::Follow(c.target);
                (c.commp_id, c.num, c.ids, true, Some(order))
            }
            // GUARD: its units as FOLLOW's, the target at +8
            Packet::Guard(c) => {
                let order = genie_core::commands::Order::Guard(c.target);
                (c.commp_id, c.num, c.ids, true, Some(order))
            }
            // RESIGN: the comm player, the world player of the same number in these games
            Packet::Resign(c) => {
                return Some(Command::Resign {
                    player: c.commp_id as usize,
                    dropped: c.dropped != 0,
                });
            }
            // TRANSFORM_OBJECT: the object at +8 takes the master at +4
            Packet::TransformObject(c) => {
                return Some(Command::TransformObject {
                    player: c.commp_id as usize,
                    id: c.field_0x08 as i32,
                    master: c.field_0x04 as i16,
                });
            }
            // BUILDING_COMMAND_QUEUE: the objects listed (at most a human player's 60), the
            // count at +0x10
            Packet::BuildingCommandQueue(c) => {
                return Some(Command::CommandQueue {
                    player: c.commp_id as usize,
                    ids: c
                        .ids
                        .iter()
                        .copied()
                        .take((c.num as usize).min(MAX_UNITS))
                        .collect(),
                    target: c.t_id,
                    point: [c.x, c.y],
                });
            }
            Packet::BuildWall(c) => return Some(build_wall_command(&c, sections)),
            Packet::CancelBuild(c) => {
                return Some(Command::Delete {
                    player: c.commp_id as usize,
                    id: c.u_id,
                });
            }
            // GO_BACK_TO_WORK: one building (+4); GO_BACK_TO_WORK2: a count (+4, a short) and
            // the ids from +8
            Packet::GoBackToWork(c) => {
                return Some(Command::GoBackToWork {
                    player: c.commp_id as usize,
                    ids: vec![c.u_id],
                });
            }
            // ALL_BACK_TO_WORK: the object (+4)
            Packet::AllBackToWork(c) => {
                return Some(Command::AllBackToWork {
                    player: c.commp_id as usize,
                    id: c.u_id,
                });
            }
            // TOWN_BELL: the town centre (+4) and the ring or its end (+8, a byte)
            Packet::TownBell(c) => {
                return Some(Command::TownBell {
                    player: c.commp_id as usize,
                    id: c.u_id,
                    activate: c.activate != 0,
                });
            }
            Packet::GoBackToWork2(c) => {
                let count = c
                    .rest
                    .get(..2)
                    .map_or(0, |b| usize::from(u16::from_le_bytes([b[0], b[1]])));
                let ids = c
                    .rest
                    .get(4..)
                    .unwrap_or_default()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .take(count.min(MAX_UNITS))
                    .map(|b| i32::from_le_bytes(*b))
                    .collect();
                return Some(Command::GoBackToWork {
                    player: c.commp_id as usize,
                    ids,
                });
            }
            Packet::TribeGiveAttributes(c) => {
                let gift = |attribute, amount, fee| genie_core::players::tribute::Gift {
                    attribute,
                    amount,
                    fee,
                };
                return Some(Command::Tribute {
                    player: c.commp_id as usize,
                    to: usize::from(c.to),
                    gifts: [
                        gift(c.attr, c.amt, c.cost),
                        gift(c.field_0x26, c.field_0x08, c.field_0x18),
                        gift(c.field_0x28, c.field_0x0c, c.field_0x1c),
                        gift(c.field_0x2a, c.field_0x10, c.field_0x20),
                    ],
                });
            }
            // SELL_COMMODITY and BUY_COMMODITY: food, wood or stone (the low byte), else nothing
            Packet::SellCommodity(c) => return trade(c.commp_id, c.attr, c.amt, false),
            Packet::BuyCommodity(c) => return trade(c.commp_id, c.attr, c.amt, true),
            Packet::Flare(c) => {
                return Some(Command::Flare {
                    player: c.commp_id as usize,
                    target: c.field_0x04.cast_signed(),
                    point: [c.field_0x08, c.field_0x0c],
                    players: c.field_0x10,
                });
            }
            Packet::Gate(c) => {
                return Some(Command::Gate {
                    player: c.commp_id as usize,
                    id: c.u_id,
                });
            }
            // UNLOAD: the objects listed (at most a human player's 60), the count at +4
            Packet::Unload(c) if c.num != LAST_COMMANDED => {
                return Some(Command::Unload {
                    player: c.commp_id as usize,
                    ids: c
                        .ids
                        .iter()
                        .copied()
                        .take((c.num as usize).min(MAX_UNITS))
                        .collect(),
                    point: [c.x, c.y],
                    flag: c.flag,
                    unit_type: c.unit_type,
                    sound: c.field_0x18 != 0,
                });
            }
            // UNIT_ORDER: the units listed (at most a human player's 60), the count at +4
            Packet::UnitOrder(c) if c.num != LAST_COMMANDED => {
                return Some(Command::UnitOrder {
                    player: c.commp_id as usize,
                    ids: c
                        .ids
                        .iter()
                        .copied()
                        .take((c.num as usize).min(MAX_UNITS))
                        .collect(),
                    target: c.t_id,
                    through_ai: c.field_0x1d != 0,
                    nearby: c.field_0x1e != 0,
                    action: c.action,
                    param: c.param.cast_unsigned(),
                    human: c.field_0x20 == 1,
                });
            }
            // UNIT_AI_STATE: the units listed (at most a human player's 60), the count at +4
            Packet::UnitAiState(c) => {
                return Some(Command::UnitAiState {
                    player: c.commp_id as usize,
                    ids: c
                        .ids
                        .iter()
                        .copied()
                        .take(usize::from(c.num).min(MAX_UNITS))
                        .collect(),
                    state: c.state,
                });
            }
            // AUTO_SCOUT: the units listed (at most a human player's 60), the count at +4
            Packet::AutoScout(c) => {
                return Some(Command::AutoScout {
                    player: c.commp_id as usize,
                    ids: c
                        .ids
                        .iter()
                        .copied()
                        .take(usize::from(c.field_0x04).min(MAX_UNITS))
                        .collect(),
                });
            }
            // STOP: the units listed (at most a human player's 60), the count at +4; a count past
            // the ids the packet holds fails the command's check (`FUN_140e55a30`)
            Packet::Stop(c) => {
                if c.ids.len() < usize::from(c.num) {
                    return None;
                }
                return Some(Command::Stop {
                    player: c.commp_id as usize,
                    ids: c
                        .ids
                        .iter()
                        .copied()
                        .take(usize::from(c.num).min(MAX_UNITS))
                        .collect(),
                });
            }
            // PATROL: the units listed, the count at +4 (the core caps them at the player's 60
            // or 500); the command's check refuses a count past the ids the packet holds
            // (`FUN_140e55590`), and the handler does nothing without 1 to 10 waypoints (`+0xb`)
            Packet::Patrol(c) => return patrol_command(&c),
            // ATTACK_MOVE: the same layout and check (`FUN_140e54490`); the handler does nothing
            // without 1 to 10 waypoints (`+0xb`)
            Packet::AttackMove(c) => return attack_move_command(&c),
            // SEEK_SHELTER: the units listed (at most a human player's 60), the count at +4
            Packet::SeekShelter(c) => {
                return Some(Command::SeekShelter {
                    player: c.commp_id as usize,
                    ids: c
                        .ids
                        .iter()
                        .copied()
                        .take(usize::from(c.num).min(MAX_UNITS))
                        .collect(),
                });
            }
            // GAME: the sub-command at +4, its player at +8 and arguments at +0xa, +0xc, +0x10
            // (do_command's gate and the switch are the core's)
            Packet::Game(g) => {
                return Some(Command::Game(genie_core::commands::game::GameCommand {
                    issuer: usize::from(g.commp_id),
                    sub: g.field_0x04,
                    player: g.field_0x08,
                    a: g.field_0x0a,
                    speed: g.field_0x0c,
                    value: g.field_0x10,
                }));
            }
            other => {
                // a command the decoder does not turn into the slice's stops its player (byte 1
                // is the issuing comm player)
                let player = *packet.get(1)? as usize;
                return Some(Command::Unmodelled {
                    player,
                    why: GenieError::unsupported(unmodelled_command(&other)),
                });
            }
        };
        let units = if num == LAST_COMMANDED {
            self.last_commanded
                .get(&player)
                .cloned()
                .unwrap_or_default()
        } else {
            let units: Vec<i32> = ids
                .into_iter()
                .take((num as usize).min(MAX_UNITS))
                .collect();
            let _ = self.last_commanded.insert(player, units.clone());
            units
        };
        order.then_some(Command::Order {
            player: player as usize,
            units,
            order: interact,
        })
    }
}
