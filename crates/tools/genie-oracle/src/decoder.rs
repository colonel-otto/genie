//! The recorded commands the slice runs ([`Decoder`]), and why it stops on one it does not
//! decode.

use crate::{
    Frame, attack_move_command, build_command, build_wall_command, patrol_command, queue_command,
    trade,
};
use genie_core::GenieError;
use genie_core::commands::Command;
use genie_rec::de;
use genie_rec::de::command::Command as Packet;
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
        let decoded = Packet::decode(packet);
        if let Some(list) = unit_list_order(&decoded) {
            return self.order(list);
        }
        listed_command(&decoded)
            .unwrap_or_else(|NotListed| single_command(&decoded, packet, sections))
    }

    /// An order to the units listed, or to the comm player's last commanded units (a count of
    /// `0xffff`); a list of its own becomes the player's last commanded units. WORK and RETREAT
    /// only leave their list.
    fn order(&mut self, list: UnitList) -> Option<Command> {
        let UnitList {
            player,
            num,
            ids,
            order,
            interact,
        } = list;
        let units = if num == LAST_COMMANDED {
            self.last_commanded
                .get(&player)
                .cloned()
                .unwrap_or_default()
        } else {
            let units = capped(&ids, usize::from(num));
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

/// A command to the units of the game's unit-list reader: its comm player, the count, the ids,
/// whether it orders them and the order.
struct UnitList {
    player: u8,
    num: u16,
    ids: Vec<i32>,
    order: bool,
    interact: Option<genie_core::commands::Order>,
}

/// The first `num` of `ids`, at most a human player's 60.
fn capped(ids: &[i32], num: usize) -> Vec<i32> {
    ids.iter().copied().take(num.min(MAX_UNITS)).collect()
}

/// INTERACT, WORK, MOVE, RETREAT, FOLLOW and GUARD, whose units the unit-list reader
/// (`FUN_140e6b290`) reads; None for any other packet.
fn unit_list_order(packet: &Packet) -> Option<UnitList> {
    // INTERACT, MOVE and RETREAT reset their units' idle timeouts (`FUN_140e6cfc0`), WORK not
    let (player, num, ids, order, interact) = match packet {
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
            (c.commp_id, c.num, c.ids.clone(), true, Some(order))
        }
        Packet::Work(c) => (c.commp_id, c.num, c.ids.clone(), false, None),
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
            (c.commp_id, c.num, c.ids.clone(), true, Some(order))
        }
        Packet::Retreat(c) => (c.commp_id, c.num, c.ids.clone(), true, None),
        // FOLLOW: its units as MOVE's, the target at +8
        Packet::Follow(c) => {
            let order = genie_core::commands::Order::Follow(c.target);
            (c.commp_id, c.num, c.ids.clone(), true, Some(order))
        }
        // GUARD: its units as FOLLOW's, the target at +8
        Packet::Guard(c) => {
            let order = genie_core::commands::Order::Guard(c.target);
            (c.commp_id, c.num, c.ids.clone(), true, Some(order))
        }
        _ => return None,
    };
    Some(UnitList {
        player,
        num,
        ids,
        order,
        interact,
    })
}

/// A packet that is not one of [`listed_command`]'s.
struct NotListed;

/// The commands to the objects listed (at most a human player's 60) that do not go through the
/// unit-list reader; `Ok(None)` where the command's check refuses it.
fn listed_command(packet: &Packet) -> Result<Option<Command>, NotListed> {
    Ok(match packet {
        Packet::SetGatherPoint(c) => Some(Command::GatherPoint {
            player: c.commp_id as usize,
            ids: capped(&c.ids, c.num as usize),
            point: [c.x, c.y],
            target: c.t_id,
            target_master: c.tm_id as i16,
            own: c.field_0x18 != 0,
        }),
        // BUILDING_COMMAND_QUEUE: the count at +0x10
        Packet::BuildingCommandQueue(c) => Some(Command::CommandQueue {
            player: c.commp_id as usize,
            ids: capped(&c.ids, c.num as usize),
            target: c.t_id,
            point: [c.x, c.y],
        }),
        // GO_BACK_TO_WORK2: a count (+4, a short) and the ids from +8
        Packet::GoBackToWork2(c) => Some(Command::GoBackToWork {
            player: c.commp_id as usize,
            ids: go_back_to_work_ids(&c.rest),
        }),
        // UNLOAD: the count at +4
        Packet::Unload(c) if c.num != LAST_COMMANDED => Some(Command::Unload {
            player: c.commp_id as usize,
            ids: capped(&c.ids, c.num as usize),
            point: [c.x, c.y],
            flag: c.flag,
            unit_type: c.unit_type,
            sound: c.field_0x18 != 0,
        }),
        // UNIT_ORDER: the count at +4
        Packet::UnitOrder(c) if c.num != LAST_COMMANDED => Some(Command::UnitOrder {
            player: c.commp_id as usize,
            ids: capped(&c.ids, c.num as usize),
            target: c.t_id,
            through_ai: c.field_0x1d != 0,
            nearby: c.field_0x1e != 0,
            action: c.action,
            param: c.param.cast_unsigned(),
            human: c.field_0x20 == 1,
        }),
        // UNIT_AI_STATE: the count at +4
        Packet::UnitAiState(c) => Some(Command::UnitAiState {
            player: c.commp_id as usize,
            ids: capped(&c.ids, usize::from(c.num)),
            state: c.state,
        }),
        // AUTO_SCOUT: the count at +4
        Packet::AutoScout(c) => Some(Command::AutoScout {
            player: c.commp_id as usize,
            ids: capped(&c.ids, usize::from(c.field_0x04)),
        }),
        // STOP: the count at +4; a count past the ids the packet holds fails the command's
        // check (`FUN_140e55a30`)
        Packet::Stop(c) => (usize::from(c.num) <= c.ids.len()).then(|| Command::Stop {
            player: c.commp_id as usize,
            ids: capped(&c.ids, usize::from(c.num)),
        }),
        // PATROL: the units listed, the count at +4 (the core caps them at the player's 60
        // or 500); the command's check refuses a count past the ids the packet holds
        // (`FUN_140e55590`), and the handler does nothing without 1 to 10 waypoints (`+0xb`)
        Packet::Patrol(c) => patrol_command(c),
        // ATTACK_MOVE: the same layout and check (`FUN_140e54490`); the handler does nothing
        // without 1 to 10 waypoints (`+0xb`)
        Packet::AttackMove(c) => attack_move_command(c),
        // SEEK_SHELTER: the count at +4
        Packet::SeekShelter(c) => Some(Command::SeekShelter {
            player: c.commp_id as usize,
            ids: capped(&c.ids, usize::from(c.num)),
        }),
        _ => return Err(NotListed),
    })
}

/// `GO_BACK_TO_WORK2`'s ids: the count is the short at the start of `rest` (+4), the ids follow
/// from +8.
fn go_back_to_work_ids(rest: &[u8]) -> Vec<i32> {
    let count = rest
        .get(..2)
        .map_or(0, |b| usize::from(u16::from_le_bytes([b[0], b[1]])));
    rest.get(4..)
        .unwrap_or_default()
        .as_chunks::<4>()
        .0
        .iter()
        .take(count.min(MAX_UNITS))
        .map(|b| i32::from_le_bytes(*b))
        .collect()
}

/// The commands of one object, one player or the game; a command the decoder does not turn
/// into the slice's stops its player (byte 1 of the `raw` packet is the issuing comm player).
fn single_command(packet: &Packet, raw: &[u8], sections: &[PlayerSection<'_>]) -> Option<Command> {
    Some(match packet {
        Packet::Build(build) => build_command(build, sections),
        Packet::MultiQueue(queue) => queue_command(queue, sections),
        Packet::BuildWall(c) => build_wall_command(c, sections),
        // RESEARCH (0x65): the handler reads the building count at +0x8 as a short
        Packet::Research(c) => Command::Research {
            player: c.commp_id as usize,
            building: c.u_id,
            count: c.field_0x08 as i16,
            buildings: c.ids.clone(),
            tech: c.t_id,
            target: c.q_id,
            shift: c.field_0x10 != 0,
        },
        // RESIGN: the comm player, the world player of the same number in these games
        Packet::Resign(c) => Command::Resign {
            player: c.commp_id as usize,
            dropped: c.dropped != 0,
        },
        // TRANSFORM_OBJECT: the object at +8 takes the master at +4
        Packet::TransformObject(c) => Command::TransformObject {
            player: c.commp_id as usize,
            id: c.field_0x08 as i32,
            master: c.field_0x04 as i16,
        },
        Packet::CancelBuild(c) => Command::Delete {
            player: c.commp_id as usize,
            id: c.u_id,
        },
        // GO_BACK_TO_WORK: one building (+4)
        Packet::GoBackToWork(c) => Command::GoBackToWork {
            player: c.commp_id as usize,
            ids: vec![c.u_id],
        },
        // ALL_BACK_TO_WORK: the object (+4)
        Packet::AllBackToWork(c) => Command::AllBackToWork {
            player: c.commp_id as usize,
            id: c.u_id,
        },
        // TOWN_BELL: the town centre (+4) and the ring or its end (+8, a byte)
        Packet::TownBell(c) => Command::TownBell {
            player: c.commp_id as usize,
            id: c.u_id,
            activate: c.activate != 0,
        },
        Packet::TribeGiveAttributes(c) => tribute_command(c),
        // SELL_COMMODITY and BUY_COMMODITY: food, wood or stone (the low byte), else nothing
        Packet::SellCommodity(c) => return trade(c.commp_id, c.attr, c.amt, false),
        Packet::BuyCommodity(c) => return trade(c.commp_id, c.attr, c.amt, true),
        Packet::Flare(c) => Command::Flare {
            player: c.commp_id as usize,
            target: c.field_0x04.cast_signed(),
            point: [c.field_0x08, c.field_0x0c],
            players: c.field_0x10,
        },
        Packet::Gate(c) => Command::Gate {
            player: c.commp_id as usize,
            id: c.u_id,
        },
        // GAME: the sub-command at +4, its player at +8 and arguments at +0xa, +0xc, +0x10
        // (do_command's gate and the switch are the core's)
        Packet::Game(g) => Command::Game(genie_core::commands::game::GameCommand {
            issuer: usize::from(g.commp_id),
            sub: g.field_0x04,
            player: g.field_0x08,
            a: g.field_0x0a,
            speed: g.field_0x0c,
            value: g.field_0x10,
        }),
        other => Command::Unmodelled {
            player: *raw.get(1)? as usize,
            why: GenieError::unsupported(unmodelled_command(other)),
        },
    })
}

/// GIVE ATTRIBUTES (0xc4): the four entries' attributes (`+0x24`..), amounts (`+0x04`..) and
/// fees (`+0x14`..), the receiver at `+0x2c`.
fn tribute_command(c: &de::command::TribeGiveAttributes) -> Command {
    let gift = |attribute, amount, fee| genie_core::players::tribute::Gift {
        attribute,
        amount,
        fee,
    };
    Command::Tribute {
        player: c.commp_id as usize,
        to: usize::from(c.to),
        gifts: [
            gift(c.attr, c.amt, c.cost),
            gift(c.field_0x26, c.field_0x08, c.field_0x18),
            gift(c.field_0x28, c.field_0x0c, c.field_0x1c),
            gift(c.field_0x2a, c.field_0x10, c.field_0x20),
        ],
    }
}

#[cfg(test)]
mod tests;
