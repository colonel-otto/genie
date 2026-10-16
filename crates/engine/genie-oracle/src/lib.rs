//! The replay oracle: state read from a recording's header save, checked against the checkpoints
//! the recording stores in its body.
//!
//! The first checkpoint comes 500 syncs into the game; a recording has none at tick 0. Ten of its
//! eleven words per player still equal the start-of-game state ([`SAVE_WORDS`]). Word 9 does not:
//! it sums each object's top action and unit-AI retarget timer, which the engine redraws while the
//! game runs, so reproducing it needs the simulation.

pub mod masters;
pub mod tick_trace;

pub use masters::MasterTable;

use genie_core::artifact::ARTIFACT_TYPE;
use genie_core::checkpoint::{ObjectState, PlayerState, TopAction, player_block};
use genie_core::lifecycle::{Class, Lifecycle, MasterData};
use genie_core::turn::{Player, Turns, Unit};
use genie_rec::de::header::Header;
use genie_rec::de::header::generated::world::{PlayerObjects, RgeStaticObject};
use genie_rec::de::{self, Op, header::PlayerSection};

/// The words of the first checkpoint that the start-of-game save determines: all but word 9.
pub const SAVE_WORDS: [usize; 10] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 10];

/// One player's block, computed from the save and as the recording stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerComparison {
    pub id: u32,
    pub from_save: [u32; 11],
    pub checkpoint: [u32; 11],
}

/// Why a recording could not be compared.
#[derive(Debug)]
pub enum Error {
    Rec(de::Error),
    /// The recording has no checkpoint (it ends within the first interval).
    NoCheckpoint,
    /// A checkpoint lane names a player the header save does not have.
    NoSuchPlayer(u32),
}

impl From<de::Error> for Error {
    fn from(e: de::Error) -> Self {
        Error::Rec(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Rec(e) => write!(f, "{e}"),
            Error::NoCheckpoint => write!(f, "the recording has no checkpoint"),
            Error::NoSuchPlayer(id) => write!(f, "a checkpoint names player {id}, not in the save"),
        }
    }
}

impl std::error::Error for Error {}

/// The static part of each live object, with its object.
fn live<'a>(
    section: &PlayerSection<'a>,
) -> impl Iterator<Item = (&'a de::header::Object, &'a RgeStaticObject)> {
    section
        .live_objects()
        .filter_map(|o| o.static_part().map(|s| (o, s)))
}

/// An object saved alive without hit points: its first update kills it, and it leaves the live
/// list within a few ticks (genie-core's lifecycle; a flare's 1 s corpse timer takes 14). The
/// save's PLACEHOLDER2s, FLARE3s and empty town-centre annexes are such objects.
fn dies_at_start(s: &RgeStaticObject) -> bool {
    matches!(s.object_state, 0 | 2) && s.hp < 1.0
}

/// What the block reads of a saved object, before the slice changes any of it.
fn object_state(s: &RgeStaticObject) -> ObjectState {
    ObjectState {
        id: s.id,
        main_id: s.master_obj,
        state: s.object_state,
        amount_held: s.attribute_amount_held,
        position: [s.t_x, s.t_y, s.t_z],
        top_action: None,
        retarget_timer: None,
    }
}

/// The checkpoint inputs of one player section at the first checkpoint, from the save alone: the
/// objects that die at the start ([`dies_at_start`]) are gone by then. Word 9's inputs stay empty
/// (see the crate docs).
pub fn player_state(section: &PlayerSection<'_>, id: u32) -> PlayerState {
    let attributes = &section.world_player().base.attributes;
    let resource = |i: usize| attributes.get(i).copied().unwrap_or_default();
    PlayerState {
        id,
        resources: [resource(0), resource(1), resource(2), resource(3)],
        sleeping_count: section.sleeping_objects().count() as u32,
        live: live(section)
            .map(|(_, s)| s)
            .filter(|s| !dies_at_start(s))
            .map(object_state)
            .collect(),
    }
}

/// Compare every player of the first chapter's save with the recording's first checkpoint, in the
/// order of the checkpoint's lanes. Lanes that are all zero hold no player.
pub fn compare_first_checkpoint(recording: &[u8]) -> Result<Vec<PlayerComparison>, Error> {
    let chapter = de::chapters(recording)?[0];
    let header = de::header::parse(&chapter.inflate_header()?)?;
    let checkpoint = chapter
        .ops()
        .find_map(|op| match op {
            Ok(Op::Checkpoint { checkpoint, .. }) => Some(Ok(checkpoint)),
            Ok(_) => None,
            Err(e) => Some(Err(e)),
        })
        .ok_or(Error::NoCheckpoint)??;
    let players = header.players();
    checkpoint
        .lanes
        .iter()
        .filter(|lane| lane.iter().any(|&w| w != 0))
        .map(|lane| {
            let id = lane[8];
            let section = players.get(id as usize).ok_or(Error::NoSuchPlayer(id))?;
            Ok(PlayerComparison {
                id,
                from_save: player_block(&player_state(section, id)),
                checkpoint: *lane,
            })
        })
        .collect()
}

/// Commands whose handlers can create a unit group (158041 `FUN_14152efc0`): INTERACT, WORK,
/// MOVE, FORM_FORMATION, GROUP_MULTI_WAYPOINTS, BUILD, REPAIR. More player-side callers exist and
/// are not mapped yet. A group draws from the World stream every tick while active and every 3 s
/// while idle (`FUN_14103d910` → `FUN_14102d370`), which the slice does not model. Whether a command
/// creates one depends on its units: the splitter (`FUN_140fd76d0`) groups only clusters of two or
/// more formation-capable units, and villagers never join a formation (as openage's
/// `doc/reverse_engineering/game_mechanics/formations.md` describes for AoC).
pub const GROUP_COMMANDS: [u8; 7] = [0x00, 0x02, 0x03, 0x17, 0x1f, 0x66, 0x6e];

/// A command that may create a unit group and so take the frame-seed chain out of the slice's model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupCommand {
    /// The tick (1-based) the command executes in.
    pub tick: u32,
    pub player: u8,
    pub command: u8,
}

/// Word 9 of one player's first checkpoint, from the unit-AI slice and as recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word9 {
    pub id: u32,
    pub simulated: u32,
    pub checkpoint: u32,
    /// The first command before the checkpoint, by any player, that may create a unit group.
    pub group_command: Option<GroupCommand>,
    /// The first unit update of this player the slice could not model.
    pub unsupported: Option<(u32, usize, genie_core::unit_ai::Unsupported)>,
}

/// Unit-AI type 19, livestock: their default task is the Artifact action (the .dat's livestock task
/// lists), which an idle animal holds as its top action.
const LIVESTOCK_AI: u32 = 19;

/// The slice's view of one saved object: its lifecycle, its unit AI and, for livestock, the Artifact
/// action.
fn unit(
    object: &de::header::Object,
    objects: &PlayerObjects,
    civ: u8,
    masters: Option<&MasterTable>,
) -> Unit {
    let ai = unit_ai(object);
    let gaia = object.static_part().is_some_and(|s| s.owner == 0);
    let artifact = ai
        .as_ref()
        .filter(|ai| ai.ai_type == LIVESTOCK_AI)
        .map(|_| genie_core::artifact::Artifact::new(gaia));
    let (id, life) = object
        .static_part()
        .map_or((-1, Lifecycle::default()), |s| {
            (s.id, lifecycle(object, s, objects, civ, masters))
        });
    Unit {
        id,
        life,
        ai,
        artifact,
    }
}

/// The object's lifecycle fields. An object with its own master takes every master value from it;
/// otherwise the death sprite and spawns come from its player's master in the save, and the decay
/// and undead flag from the table, which the save lacks.
fn lifecycle(
    object: &de::header::Object,
    s: &RgeStaticObject,
    objects: &PlayerObjects,
    civ: u8,
    masters: Option<&MasterTable>,
) -> Lifecycle {
    let pure_static = object
        .rge_object
        .as_ref()
        .is_some_and(|r| r.rge_static_object.is_some());
    let (dying_sprite, death_spawn, master) = match s.own_master() {
        Some(own) => (
            own.death_sprite_num >= 0,
            own.death_spawn_obj_id >= 0 || own.death_spawn_obj_id2 >= 0,
            Some(MasterData {
                decay: own.attribute_rot,
                undead: own.undead_flag != 0,
            }),
        ),
        None => {
            let player = objects.master(s.master_obj).and_then(|m| m.static_part());
            (
                player.is_some_and(|m| m.death_sprite_id >= 0),
                player.is_some_and(|m| m.death_spawn_obj_id >= 0 || m.death_spawn_obj_id2 >= 0),
                masters.and_then(|t| t.get(civ, s.master_obj)),
            )
        }
    };
    Lifecycle {
        class: if pure_static {
            Class::Static
        } else {
            Class::Animated
        },
        state: s.object_state,
        hp: s.hp,
        attribute_type: s.attribute_type_held,
        amount: s.attribute_amount_held,
        dying_sprite,
        death_spawn,
        inside: s.inside_obj != -1,
        goto_sleep: s.goto_sleep_flag != 0,
        master_id: s.master_obj,
        master,
    }
}

fn unit_ai(object: &de::header::Object) -> Option<genie_core::unit_ai::UnitAi> {
    let combat = object.combat()?;
    let ai = combat.unit_ai()?;
    Some(genie_core::unit_ai::UnitAi {
        ai_type: combat.need_unit_ai as u32,
        current_order: ai.current_order_value,
        current_action: ai.current_action_value,
        current_target: ai.current_target_value,
        idle_timer: ai.idle_timer_value,
        adjusted_idle_timeout: ai.adjusted_idle_timeout_value,
        secondary_timer: ai.secondary_timer_value,
        look_around_timer: ai.look_around_timer_value,
        look_around_timeout: ai.look_around_timeout_value,
        last_retarget_time: ai.last_retarget_time,
        retarget_timer: ai.randomized_retarget_timer,
        stance: ai.state_value,
        retarget_disabled: ai.disable_retargeting,
    })
}

/// The slice set up from the header save: every player's live objects in their saved order, and
/// BaseWorld's fields before the map, the frame seed (`mCurrentFrameRandomSeed`, world+0x7d0) and
/// the turn order (`player_turn`, `player_time_delta[i]`).
fn slice(header: &Header, masters: Option<&MasterTable>) -> Turns {
    let players = header
        .players()
        .iter()
        .map(|p| {
            let civ = p.world_player().base.civ_id;
            Player {
                objects: p
                    .live_objects()
                    .map(|o| unit(o, p.objects(), civ, masters))
                    .collect(),
                computer: false,
                unsupported: None,
            }
        })
        .collect();
    let world = &header.world.base_world;
    Turns::new(
        world.world_time,
        world.current_frame_random_seed,
        world.player_turn.max(0) as usize,
        world.player_time_delta.clone(),
        players,
    )
}

/// The tick lengths of the chapter's first `limit` ticks (a sync or a checkpoint each).
fn dts(chapter: &de::Chapter<'_>, limit: usize) -> Result<Vec<u32>, Error> {
    let mut out = Vec::new();
    for op in chapter.ops() {
        if out.len() == limit {
            break;
        }
        match op? {
            Op::Sync { dt } | Op::Checkpoint { dt, .. } => out.push(dt),
            _ => {}
        }
    }
    Ok(out)
}

/// Run the unit-AI slice from the header save up to the first checkpoint and compare word 9.
/// `masters` gives the dead-object values the save lacks; without it, an object that dies stops
/// its player's slice.
///
/// The checkpoint block is built before the player loop of the tick that records it, so it shows
/// the state after one tick fewer than the syncs before it.
pub fn simulate_word9(
    recording: &[u8],
    masters: Option<&MasterTable>,
) -> Result<Vec<Word9>, Error> {
    let chapter = de::chapters(recording)?[0];
    let header = de::header::parse(&chapter.inflate_header()?)?;
    let mut dts = Vec::new();
    let mut group_command = None;
    let mut checkpoint = None;
    for op in chapter.ops() {
        match op? {
            Op::Sync { dt } => dts.push(dt),
            Op::Command { packet, .. } => {
                if group_command.is_none() && GROUP_COMMANDS.contains(&packet[0]) {
                    group_command = Some(GroupCommand {
                        tick: dts.len() as u32 + 1,
                        player: packet.get(1).copied().unwrap_or_default(),
                        command: packet[0],
                    });
                }
            }
            Op::Checkpoint { checkpoint: c, .. } => {
                checkpoint = Some(c);
                break;
            }
            _ => {}
        }
    }
    let checkpoint = checkpoint.ok_or(Error::NoCheckpoint)?;
    let sections = header.players();
    let mut turns = slice(&header, masters);
    for &dt in &dts {
        turns.tick(dt);
    }
    checkpoint
        .lanes
        .iter()
        .filter(|lane| lane.iter().any(|&w| w != 0))
        .map(|lane| {
            let id = lane[8];
            let section = sections.get(id as usize).ok_or(Error::NoSuchPlayer(id))?;
            let simulated = &turns.players[id as usize];
            Ok(Word9 {
                id,
                simulated: lane_block(section, id, simulated)[9],
                checkpoint: lane[9],
                group_command,
                unsupported: simulated.unsupported,
            })
        })
        .collect()
}

/// A player's block with the slice's objects in place of the save's: the save gives what the slice
/// leaves alone (resources, sleeping objects, each object's main id and position), the slice its
/// live list and each object's state, held amount, top action and retarget timer. An object the
/// save does not have is left out; the slice creates none.
fn lane_block(section: &PlayerSection<'_>, id: u32, simulated: &Player) -> [u32; 11] {
    let mut state = player_state(section, id);
    let saved: std::collections::HashMap<i32, ObjectState> = live(section)
        .map(|(_, s)| (s.id, object_state(s)))
        .collect();
    state.live = simulated
        .objects
        .iter()
        .filter_map(|unit| {
            let mut object = saved.get(&unit.id)?.clone();
            object.state = unit.life.state;
            object.amount_held = unit.life.amount;
            object.retarget_timer = unit.ai.as_ref().map(|ai| ai.retarget_timer as i32);
            object.top_action = unit.artifact.map(|a| TopAction {
                kind: ARTIFACT_TYPE,
                state: a.state,
            });
            Some(object)
        })
        .collect();
    player_block(&state)
}

/// One live object's word-9 terms in the slice after a number of ticks: its id, the top action's
/// type (or -1) and state, and the unit-AI retarget timer (0 without a unit AI).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Word9Terms {
    pub id: u32,
    pub action: i32,
    pub state: u8,
    pub retarget: i32,
}

/// The slice's [`Word9Terms`] of player `id`'s live objects after `ticks` ticks.
pub fn word9_terms(
    recording: &[u8],
    id: u32,
    ticks: usize,
    masters: Option<&MasterTable>,
) -> Result<Vec<Word9Terms>, Error> {
    let chapter = de::chapters(recording)?[0];
    let header = de::header::parse(&chapter.inflate_header()?)?;
    if header.players().len() <= id as usize {
        return Err(Error::NoSuchPlayer(id));
    }
    let mut turns = slice(&header, masters);
    for dt in dts(&chapter, ticks)? {
        turns.tick(dt);
    }
    Ok(turns.players[id as usize]
        .objects
        .iter()
        .map(|unit| Word9Terms {
            id: unit.id as u32,
            action: unit.artifact.map_or(-1, |_| ARTIFACT_TYPE as i32),
            state: unit.artifact.map_or(0, |a| a.state),
            retarget: unit.ai.as_ref().map_or(0, |ai| ai.retarget_timer as i32),
        })
        .collect())
}

/// The slice's blocks of the players `ids` after each of the first `ticks` ticks, from the header
/// save on: the per-tick counterpart of [`simulate_word9`], for the tick oracle's trace
/// ([`tick_trace::TickTrace`]) to name the first tick and word where the slice and the game part.
pub fn blocks_per_tick(
    recording: &[u8],
    ids: &[u32],
    ticks: usize,
    masters: Option<&MasterTable>,
) -> Result<Vec<Vec<[u32; 11]>>, Error> {
    let chapter = de::chapters(recording)?[0];
    let header = de::header::parse(&chapter.inflate_header()?)?;
    let sections = header.players();
    if let Some(&id) = ids.iter().find(|&&id| sections.len() <= id as usize) {
        return Err(Error::NoSuchPlayer(id));
    }
    let mut turns = slice(&header, masters);
    let mut out = Vec::with_capacity(ticks);
    for dt in dts(&chapter, ticks)? {
        turns.tick(dt);
        out.push(
            ids.iter()
                .map(|&id| lane_block(&sections[id as usize], id, &turns.players[id as usize]))
                .collect(),
        );
    }
    Ok(out)
}
