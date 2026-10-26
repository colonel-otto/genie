//! The body's frames and the commands' arguments: patrol, attack move, queue, trade and the
//! market, build and build wall; the unit AI a new object gets.

use crate::{Error, LAST_COMMANDED, MAX_UNITS};
use genie_core::GenieError;
use genie_core::commands::Command;
use genie_rec::de;
use genie_rec::de::Op;
use genie_save::PlayerSection;
use genie_save::generated::world::{Master, PlayerObjects};

/// One frame of the body (FD-007's per-frame record order): the op-2 record that starts a tick, its
/// dt and, every interval, a checkpoint, then the op-1 commands the tick runs after its `SeedAll`.
pub(super) struct Frame<'a> {
    pub(super) dt: u32,
    pub(super) checkpoint: Option<Box<de::Checkpoint>>,
    pub(super) commands: Vec<&'a [u8]>,
}

/// The chapter's first `limit` frames.
pub(super) fn frames<'a>(chapter: &de::Chapter<'a>, limit: usize) -> Result<Vec<Frame<'a>>, Error> {
    let mut out: Vec<Frame<'a>> = Vec::new();
    for op in chapter.ops() {
        let (dt, checkpoint) = match op? {
            Op::Sync { dt } => (dt, None),
            Op::Checkpoint { dt, checkpoint } => (dt, Some(checkpoint)),
            Op::Command { packet, .. } => {
                out.last_mut()
                    .ok_or(Error::CommandBeforeFirstSync)?
                    .commands
                    .push(packet);
                continue;
            }
            _ => continue,
        };
        if out.len() == limit {
            break;
        }
        out.push(Frame {
            dt,
            checkpoint,
            commands: Vec::new(),
        });
    }
    Ok(out)
}

/// PATROL (0x15): the units of the count at `+0x4` from `+0x5c`, the waypoints of the count
/// at `+0xb` (x from `+0xc`, y from `+0x34`, each an f32's bits), `+0x8`. None where the
/// command does nothing: a count past the ids the packet holds (the check `FUN_140e55590`),
/// no waypoints or more than 10 (`FUN_140e69cc0`).
/// A saved `MoveTo`'s wait (`+0x6c`, `+0x70`, `+0x74`); none for another sub-action.
pub(super) fn saved_move_wait(
    sub: &genie_save::generated::world::Action,
) -> genie_core::movement::move_to::MoveWait {
    sub.rge_action_by_type
        .as_deref()
        .and_then(|t| t.rge_action_move_to.as_deref())
        .map_or_else(genie_core::movement::move_to::MoveWait::default, |m| {
            genie_core::movement::move_to::MoveWait {
                state: m.movement_wait_state,
                timer: m.timer,
                delay: f32::from_bits(m.k_default_wait_time),
            }
        })
}

pub(super) fn attack_move_command(c: &de::command::AttackMove) -> Option<Command> {
    let count = usize::from(c.num);
    let waypoints = usize::from(c.waypoints_count);
    if c.ids.len() < count || !(1..=10).contains(&waypoints) {
        return None;
    }
    // x from +0xc (the first as bits, nine more after it), y from +0x34 (likewise)
    let word = |b: &[u8], i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let xs = std::iter::once(f32::from_bits(c.field_0x0c))
        .chain((0..9).map(|i| word(&c.gap_0x10, 4 * i)));
    let ys = std::iter::once(c.field_0x34).chain((0..9).map(|i| word(&c.gap_0x38, 4 * i)));
    Some(Command::AttackMove {
        player: c.commp_id as usize,
        ids: c.ids[..count].to_vec(),
        waypoints: xs.zip(ys).take(waypoints).map(<[f32; 2]>::from).collect(),
        reset: c.field_0x08 != 0,
        queued: c.field_0x09 != 0,
        ignore_pending: c.field_0x0a != 0,
    })
}

pub(super) fn patrol_command(c: &de::command::Patrol) -> Option<Command> {
    let count = usize::from(c.num);
    let waypoints = usize::from(c.waypoints_count);
    if c.ids.len() < count || !(1..=10).contains(&waypoints) {
        return None;
    }
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
    Some(Command::Patrol {
        player: c.commp_id as usize,
        ids: c.ids[..count].to_vec(),
        waypoints: xs
            .iter()
            .zip(ys)
            .take(waypoints)
            .map(|(&x, y)| [f32::from_bits(x), y])
            .collect(),
        reset: c.field_0x08 != 0,
    })
}

/// QUEUE (`do_command` case 0x81, `MULTI_QUEUE)`: `num` buildings, the unit `t_id`, `count` of it and
/// the train button (`button_location_id`, -1 for none); the building ids follow. genie-core reads
/// the costs, the train time and the building's work rate from its own state.
pub(super) fn queue_command(
    packet: &de::command::MultiQueue,
    sections: &[PlayerSection<'_>],
) -> Command {
    let player = packet.commp_id as usize;
    let unmodelled = |why| Command::Unmodelled {
        player,
        why: GenieError::unsupported(why),
    };
    if sections.get(player).is_none() {
        return unmodelled("QUEUE by a player the save does not have");
    }
    Command::Queue {
        player,
        buildings: queue_buildings(packet),
        unit: packet.t_id,
        amount: packet.count.max(0) as u16,
        button: packet.button_location_id,
    }
}

/// `SELL_COMMODITY`'s and `BUY_COMMODITY`'s command: the dispatcher trades food, wood and stone
/// only (`+0x4`'s low byte).
pub(super) fn trade(player: u8, attribute: u16, times: u16, buy: bool) -> Option<Command> {
    let attribute = attribute as u8;
    (attribute <= 2).then_some(Command::Trade {
        player: usize::from(player),
        attribute,
        times,
        buy,
    })
}

/// The world's market as the save holds it: the save names the int net trades `*_price_delta`
/// (their bits) and the swings `old_*_price_adj`.
pub(super) fn market(
    w: &genie_save::generated::world::World,
) -> genie_core::players::market::Market {
    let commodity = |price: f32, trades: f32, swing: f32| genie_core::players::market::Commodity {
        price,
        trades: trades.to_bits().cast_signed(),
        swing,
    };
    genie_core::players::market::Market {
        min_price: w.world_min_price,
        max_price: w.world_max_price,
        food: commodity(w.world_food_price, w.food_price_delta, w.old_food_price_adj),
        wood: commodity(w.world_wood_price, w.wood_price_delta, w.old_wood_price_adj),
        stone: commodity(
            w.world_stone_price,
            w.stone_price_delta,
            w.old_stone_price_adj,
        ),
        ore: commodity(w.world_ore_price, w.ore_price_delta, w.old_ore_price_adj),
    }
}

/// QUEUE's buildings: the first `num` of the ids that follow (the dispatcher's check reads that
/// many, `FUN_140e6b250` with `num`).
pub(super) fn queue_buildings(packet: &de::command::MultiQueue) -> Vec<i32> {
    packet
        .ids
        .iter()
        .take(usize::from(packet.num))
        .copied()
        .collect()
}

/// BUILD (`FUN_140e7a400`): `num` builders put a foundation of master `o_id` at (`x`, `y`); the
/// builder ids follow. The handler reads the master as an i16 (`+0x10`). The slice takes z = 0:
/// the game sets it from the terrain, so a foundation on raised terrain shows in word 7.
pub(super) fn build_command(
    packet: &de::command::Build,
    sections: &[PlayerSection<'_>],
) -> Command {
    let player = packet.commp_id as usize;
    let master = packet.o_id as i16;
    let unit_ai = sections.get(player).map_or(
        Err(GenieError::unsupported(
            "BUILD names a player the save does not have",
        )),
        |section| new_unit_ai(section.objects(), master),
    );
    if packet.num == LAST_COMMANDED {
        return Command::Unmodelled {
            player,
            why: GenieError::unsupported("BUILD by the last commanded units"),
        };
    }
    Command::Build {
        player,
        master,
        unit_ai,
        position: [packet.x, packet.y, 0.0],
        builders: packet
            .ids
            .iter()
            .copied()
            .take(packet.num as usize)
            .collect(),
        queued: packet.field_0x1e != 0,
        commits: packet.field_0x1f == 0,
        frame: packet.frame,
    }
}

/// `BUILD_WALL` (0x69): the wall's master at `+0x10`, the line's tiles at `+0x8`..`+0xe`, the
/// target at `+0x14`, the queue flags at `+0x1a` and `+0x1b`, the builders (at most a human
/// player's 60, `FUN_140e6b250`).
pub(super) fn build_wall_command(
    packet: &de::command::BuildWall,
    sections: &[PlayerSection<'_>],
) -> Command {
    let player = packet.commp_id as usize;
    let master = packet.o_id as i16;
    if packet.num == LAST_COMMANDED {
        return Command::Unmodelled {
            player,
            why: GenieError::unsupported("BUILD_WALL by the last commanded units"),
        };
    }
    let unit_ai = sections.get(player).map_or(
        Err(GenieError::unsupported(
            "BUILD_WALL names a player the save does not have",
        )),
        |section| new_unit_ai(section.objects(), master),
    );
    Command::BuildWall(genie_core::commands::wall::Wall {
        player,
        master,
        unit_ai,
        from: [packet.x1, packet.y1],
        to: [packet.x2, packet.y2],
        target: packet.q_id,
        builders: packet
            .ids
            .iter()
            .copied()
            .take((packet.num as usize).min(MAX_UNITS))
            .collect(),
        queued: packet.field_0x1a != 0,
        commits: packet.field_0x1b == 0,
    })
}

/// The unit AI a new object of `master` gets (`FUN_141215a90`, genie-core's
/// [`genie_core::unit_ai::factory_type`]): by its class (the master's fifth saved short), its id,
/// its volley missile (the tribe combat master's `volley_missile_id`, the game's `+0x8d4`: a
/// building with one gets the tower's unit AI and its fourth draw) and its combat level.
pub(super) fn new_unit_ai(objects: &PlayerObjects, master: i16) -> Result<Option<u32>, GenieError> {
    let m = objects.master(master).ok_or(GenieError::unsupported(
        "BUILD of a master the player does not have",
    ))?;
    let static_part = m.static_part().ok_or(GenieError::unsupported(
        "BUILD of a master without a static part",
    ))?;
    let class = static_part.read_temp_5;
    // the factory (`FUN_141215a90`) gives a building with a volley missile (the master's
    // `+0x8d4`, `volley_missile_id`) the tower's unit AI, as genie-core's `factory_type` does
    let volley = m
        .tribe_master_combat_object
        .as_deref()
        .or_else(|| {
            m.tribe_master_building_object
                .as_deref()
                .map(|b| &b.tribe_master_combat_object)
        })
        .and_then(|c| i16::try_from(c.volley_missile_id).ok())
        .unwrap_or(-1);
    // the class-10 choice reads the master's combat level (`+0x62c`, FUN_140fde4f0)
    Ok(genie_core::unit_ai::factory_type(
        class,
        master,
        volley,
        static_part.combat_level,
    ))
}

pub(super) fn combat_master(
    m: &Master,
) -> Option<&genie_save::generated::world::RgeMasterCombatObject> {
    None.or_else(|| {
        m.tribe_master_combat_object
            .as_deref()
            .map(|c| &c.rge_master_combat_object)
    })
    .or_else(|| {
        m.tribe_master_building_object
            .as_deref()
            .map(|b| &b.tribe_master_combat_object.rge_master_combat_object)
    })
    .or_else(|| {
        m.rge_master
            .as_deref()
            .and_then(|r| r.rge_master_combat_object.as_deref())
    })
    .or_else(|| {
        m.rge_master.as_deref().and_then(|r| {
            r.rge_master_missile_object
                .as_deref()
                .map(|m| &m.rge_master_combat_object)
        })
    })
}
