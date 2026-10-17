//! The replay oracle: state read from a recording's header save, checked against the checkpoints
//! the recording stores in its body.
//!
//! The first checkpoint comes 500 syncs into the game; a recording has none at tick 0. Ten of its
//! eleven words per player still equal the start-of-game state ([`SAVE_WORDS`]). Word 9 does not:
//! it sums each object's top action and unit-AI retarget timer, which the engine redraws while the
//! game runs, so reproducing it needs the simulation.

pub mod masters;
pub mod tick_trace;

pub use masters::{DefaultAction, MasterTable};

use genie_core::artifact::ARTIFACT_TYPE;
use genie_core::checkpoint::{ObjectState, PlayerState, TopAction, player_block};
use genie_core::lifecycle::{Class, Lifecycle, MasterData};
use genie_core::turn::{Command, Player, Turns, Unit};
use genie_core::unit_ai::Unsupported;
use genie_rec::de::header::Header;
use genie_rec::de::header::generated::world::{Master, PlayerObjects, RgeStaticObject};
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
    /// A command comes before the body's first sync, so no tick runs it.
    CommandBeforeFirstSync,
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
            Error::CommandBeforeFirstSync => write!(f, "a command comes before the first sync"),
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
    /// The tick (1-based) the command executes in: the frame of the sync before it.
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

/// The slice's view of one saved object: its lifecycle, its unit AI and its default task.
fn unit(
    object: &de::header::Object,
    objects: &PlayerObjects,
    civ: u8,
    masters: Option<&MasterTable>,
) -> Unit {
    let Some(s) = object.static_part() else {
        return Unit {
            id: -1,
            unmodelled_default_task: Some(Unsupported("an object without a static part")),
            ..Unit::default()
        };
    };
    let (artifact, unmodelled_default_task) = match default_task(object, s, civ, masters) {
        Ok(artifact) => (artifact, None),
        Err(e) => (None, Some(e)),
    };
    Unit {
        id: s.id,
        position: [s.t_x, s.t_y, s.t_z],
        life: lifecycle(object, s, objects, civ, masters),
        ai: unit_ai(object),
        artifact,
        unmodelled_default_task,
        production: None,
        build: None,
    }
}

/// What an object's first update starts of its master's default task (`RGE_Action_Object::update`,
/// 0x14106ac90): only action objects (types 40 and up) have one. The default task is game data:
/// the table's, from the `.dat`, or the object's own master's. The slice models the Artifact action
/// (type 107, the default task of livestock); any other default task is reported.
fn default_task(
    object: &de::header::Object,
    s: &RgeStaticObject,
    civ: u8,
    masters: Option<&MasterTable>,
) -> Result<Option<genie_core::artifact::Artifact>, Unsupported> {
    if !is_action_object(object) {
        return Ok(None);
    }
    let action = match s.own_master() {
        Some(_) => match own_default_task(s) {
            Some(-1) | None => DefaultAction::None,
            Some(_) => return Err(Unsupported("the default task of an object's own master")),
        },
        None => masters
            .ok_or(Unsupported("no master table: the default task is unknown"))?
            .default_action(civ, s.master_obj)
            .ok_or(Unsupported("a master the table does not have"))?,
    };
    match action {
        DefaultAction::None => Ok(None),
        DefaultAction::Type(ARTIFACT_TYPE) => {
            Ok(Some(genie_core::artifact::Artifact::new(s.owner == 0)))
        }
        DefaultAction::Type(_) => Err(Unsupported("a default task other than the Artifact action")),
        DefaultAction::NoSuchTask => Err(Unsupported("a default task id that names no task")),
    }
}

/// Whether the object runs `RGE_Action_Object::update`: types 40 (action), 50 (combat), 60
/// (missile), 70 (tribe combat) and 80 (building).
fn is_action_object(object: &de::header::Object) -> bool {
    object.combat().is_some()
        || object.rge_object.as_ref().is_some_and(|r| {
            r.rge_action_object.is_some()
                || r.rge_combat_object.is_some()
                || r.rge_missile_object.is_some()
        })
}

/// The default task id of an object's own action master, if it has one.
fn own_default_task(s: &RgeStaticObject) -> Option<i16> {
    let tribe = |c: &genie_rec::de::header::generated::world::OwnTribeMasterCombatObject| {
        c.own_rge_master_combat_object
            .own_rge_master_action_object
            .default_task
    };
    None.or(s.rge_master_action_object.as_ref().map(|a| a.default_task))
        .or(s.rge_master_missile_object.as_ref().map(|m| {
            m.own_rge_master_combat_object
                .own_rge_master_action_object
                .default_task
        }))
        .or(s.tribe_master_combat_object.as_ref().map(tribe))
        .or(s
            .tribe_master_building_object
            .as_ref()
            .map(|b| tribe(&b.own_tribe_master_combat_object)))
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
        idle_timeout: ai.idle_timeout_value,
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
            let attributes = &p.world_player().base.attributes;
            let resource = |i: usize| attributes.get(i).copied().unwrap_or_default();
            Player {
                objects: p
                    .live_objects()
                    .map(|o| unit(o, p.objects(), civ, masters))
                    .collect(),
                computer: false,
                resources: [resource(0), resource(1), resource(2), resource(3)],
                unsupported: None,
            }
        })
        .collect();
    let world = &header.world.base_world;
    let mut turns = Turns::new(
        world.world_time,
        world.current_frame_random_seed,
        world.player_turn.max(0) as usize,
        world.player_time_delta.clone(),
        players,
    );
    turns.next_object_id = world.next_object_id;
    if let Some(table) = masters {
        turns.masters = header
            .players()
            .iter()
            .map(|p| work_masters(p.objects(), p.world_player().base.civ_id, table))
            .collect();
    }
    turns
}

/// A player's masters as the work command reads them: each master the save holds, with its saved
/// class, radii, cost and (where saved) tasks, and the `.dat`'s task swap group and type, which
/// the save does not carry (the game copies them from the `.dat` master).
fn work_masters(
    objects: &PlayerObjects,
    civ: u8,
    table: &MasterTable,
) -> genie_core::masters::Masters {
    use genie_rec::de::header::generated::world::{RgeMasterActionObject, Task};
    let task = |t: &Task| genie_core::masters::Task {
        action_type: t.action_type,
        object_group: t.object_group,
        object_id: t.object_id,
        work_val1: t.work_val1,
    };
    fn action_part(m: &Master) -> Option<&RgeMasterActionObject> {
        None.or(m
            .tribe_master_combat_object
            .as_ref()
            .map(|c| &c.rge_master_combat_object.rge_master_action_object))
            .or(m.tribe_master_building_object.as_ref().map(|b| {
                &b.tribe_master_combat_object
                    .rge_master_combat_object
                    .rge_master_action_object
            }))
            .or(m.rge_master.as_ref().and_then(|r| {
                r.rge_master_action_object.as_ref().or(r
                    .rge_master_combat_object
                    .as_ref()
                    .map(|c| &c.rge_master_action_object))
            }))
    }
    let saved = objects.masters.iter().flatten().map(|m| &m.master);
    genie_core::masters::Masters::new(saved.filter_map(|m| {
        let s = m.static_part()?;
        let mut work = table.work(civ, s.id())?.clone();
        work.class = s.read_temp_5;
        work.radius = [s.radius_x, s.radius_y];
        if let Some(tasks) = action_part(m).and_then(|a| a.tasks.as_ref()) {
            work.tasks = tasks.list.iter().map(|e| task(&e.task)).collect();
        }
        let combat = m.tribe_master_combat_object.as_ref().or(m
            .tribe_master_building_object
            .as_ref()
            .map(|b| &b.tribe_master_combat_object));
        if let Some(c) = combat {
            work.cost = build_cost(&c.build_inventory);
        }
        Some(work)
    }))
}

/// A combat master's `build_inventory` entries `(resource, amount, flag)`, three shorts each; the
/// flag set means paid. Entries of resource -1 are empty.
fn build_cost(inventory: &[[u8; 6]]) -> Vec<(i16, i16, bool)> {
    inventory
        .iter()
        .map(|e| {
            let kind = i16::from_le_bytes([e[0], e[1]]);
            let amount = i16::from_le_bytes([e[2], e[3]]);
            let flag = i16::from_le_bytes([e[4], e[5]]);
            (kind, amount, flag != 0)
        })
        .filter(|&(kind, ..)| kind >= 0)
        .collect()
}

/// One frame of the body (FD-007's per-frame record order): the op-2 record that starts a tick, its
/// dt and, every interval, a checkpoint, then the op-1 commands the tick runs after its SeedAll.
struct Frame<'a> {
    dt: u32,
    checkpoint: Option<Box<de::Checkpoint>>,
    commands: Vec<&'a [u8]>,
}

/// The chapter's first `limit` frames.
fn frames<'a>(chapter: &de::Chapter<'a>, limit: usize) -> Result<Vec<Frame<'a>>, Error> {
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

/// A count of `0xffff` names the comm player's last commanded units.
const LAST_COMMANDED: u16 = 0xffff;

/// The unit-list reader keeps at most this many units of a human player's command (500 for the
/// players `FUN_141659cf0` accepts).
const MAX_UNITS: usize = 60;

/// Decodes what the slice runs of the recorded commands, in their order (the packets through
/// genie-rec's generated layouts). It remembers each comm player's last commanded units, as the
/// game's unit-list reader (`FUN_140e6b290`) does for INTERACT, WORK, MOVE and RETREAT. BUILD,
/// QUEUE and the orders run; every other command is left out (see [`GROUP_COMMANDS`] for what
/// that may miss).
///
/// The player byte is the comm player, which the handlers map to a world player through the lobby
/// (`FUN_140e6b210`); the slice takes the two as equal.
#[derive(Default)]
struct Decoder {
    last_commanded: std::collections::HashMap<u8, Vec<i32>>,
}

impl Decoder {
    fn command(&mut self, packet: &[u8], sections: &[PlayerSection<'_>]) -> Option<Command> {
        use genie_rec::de::command::Command as Packet;
        // INTERACT, MOVE and RETREAT reset their units' idle timeouts (`FUN_140e6cfc0`), WORK not
        let (player, num, ids, order) = match Packet::decode(packet) {
            Packet::Build(build) => return Some(build_command(&build, sections)),
            Packet::MultiQueue(queue) => return Some(queue_command(&queue, sections)),
            Packet::Interact(c) => (c.commp_id, c.num, c.ids, true),
            Packet::Work(c) => (c.commp_id, c.num, c.ids, false),
            Packet::Move(c) => (c.commp_id, c.num, c.ids, true),
            Packet::Retreat(c) => (c.commp_id, c.num, c.ids, true),
            _ => return None,
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
            self.last_commanded.insert(player, units.clone());
            units
        };
        order.then_some(Command::Order {
            player: player as usize,
            units,
        })
    }
}

/// QUEUE (`do_command` case 0x81, MULTI_QUEUE): `num` buildings, the unit `t_id`, `count` of it and
/// the train button (`button_location_id`, -1 for none); the building ids follow. The cost, the
/// train time and the building's work rate come from the save (the player's masters and the
/// building).
fn queue_command(
    packet: &genie_rec::de::command::MultiQueue,
    sections: &[PlayerSection<'_>],
) -> Command {
    let player = packet.commp_id as usize;
    let unmodelled = |why| Command::Unmodelled {
        player,
        why: Unsupported(why),
    };
    let Some(&building) = packet.ids.first() else {
        return unmodelled("a QUEUE packet without a building");
    };
    if packet.num != 1 {
        return unmodelled("QUEUE at several buildings");
    }
    let (unit, amount, button) = (packet.t_id, packet.count, packet.button_location_id);
    let Some(section) = sections.get(player) else {
        return unmodelled("QUEUE by a player the save does not have");
    };
    // the generic villager becomes the villager before anything is read (FUN_1411cbc40)
    let master_id = if unit == 0x712 { 0x53 } else { unit };
    match queue_data(section, master_id, button, building) {
        Ok(QueueData {
            train_time,
            rate,
            cost,
        }) => Command::Queue {
            player,
            building,
            unit,
            amount: amount.max(0) as u16,
            train_time,
            rate,
            cost,
        },
        Err(why) => Command::Unmodelled { player, why },
    }
}

/// What QUEUE needs from the save: the unit's train time at the building, the building's work rate,
/// and the unit's cost entries `(resource, amount, paid)`.
struct QueueData {
    train_time: f32,
    rate: f32,
    cost: Vec<(i16, i16, bool)>,
}

/// QUEUE's train time, the building's work rate and the unit's cost, from the save.
fn queue_data(
    section: &PlayerSection<'_>,
    master_id: i16,
    button: i32,
    building: i32,
) -> Result<QueueData, Unsupported> {
    let objects = section.objects();
    let combat = |id| {
        objects
            .master(id)
            .and_then(|m| m.tribe_master_combat_object.as_ref())
    };
    let unit = combat(master_id).ok_or(Unsupported("QUEUE of a master without a combat part"))?;
    let building_object = section
        .live_objects()
        .find(|o| o.static_part().is_some_and(|s| s.id == building))
        .ok_or(Unsupported("QUEUE at a building not in the save"))?;
    let building_master = building_object
        .static_part()
        .expect("found by it")
        .master_obj;
    let powerup = building_object
        .combat()
        .map_or(0.0, |c| c.work_rate_powerup);
    let work_rate = objects
        .master(building_master)
        .and_then(|m| combat_master(m))
        .map(|c| c.rge_master_action_object.work_rate)
        .ok_or(Unsupported("a building master without an action part"))?;
    // FUN_140acf280: the train location of this building and button, else the first
    let train_time = |c: &genie_rec::de::header::generated::world::TribeMasterCombatObject| {
        c.train_location
            .iter()
            .find(|t| {
                button != -1
                    && t.train_location_id == building_master
                    && t.button_location_id == button as u8
            })
            .or(c.train_location.first())
            .map(|t| t.train_time)
    };
    let time = train_time(unit).ok_or(Unsupported("a master without a train location"))?;
    // a villager is drawn male or female, and each pays and trains by its own master: the slice
    // takes one cost and one time, so both must agree
    if master_id == 0x53 {
        let female = combat(0x125).ok_or(Unsupported("no female villager master"))?;
        if female.build_inventory != unit.build_inventory || train_time(female) != Some(time) {
            return Err(Unsupported("villager sexes that cost or train differently"));
        }
    }
    let cost = build_cost(&unit.build_inventory);
    Ok(QueueData {
        train_time: f32::from(time),
        rate: powerup + work_rate,
        cost,
    })
}

/// BUILD (`FUN_140e7a400`): `num` builders put a foundation of master `o_id` at (`x`, `y`); the
/// builder ids follow. The handler reads the master as an i16 (`+0x10`). The slice takes z = 0:
/// the game sets it from the terrain, so a foundation on raised terrain shows in word 7.
fn build_command(
    packet: &genie_rec::de::command::Build,
    sections: &[PlayerSection<'_>],
) -> Command {
    let player = packet.commp_id as usize;
    let master = packet.o_id as i16;
    let unit_ai = match sections.get(player) {
        Some(section) => new_unit_ai(section.objects(), master),
        None => Err(Unsupported("BUILD names a player the save does not have")),
    };
    if packet.num == LAST_COMMANDED {
        return Command::Unmodelled {
            player,
            why: Unsupported("BUILD by the last commanded units"),
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
    }
}

/// The unit AI a new object of `master` gets, by its class (`FUN_141215a90`): the building AI
/// (17) for a building class without a missile, none for the classes without one, and for the
/// rest a class the slice has no constructor for. The class is the master's fifth saved short,
/// the missile the combat master's `missile_id` (`+0x8d4` in the game).
fn new_unit_ai(objects: &PlayerObjects, master: i16) -> Result<Option<u32>, Unsupported> {
    let m = objects
        .master(master)
        .ok_or(Unsupported("BUILD of a master the player does not have"))?;
    let class = m
        .static_part()
        .ok_or(Unsupported("BUILD of a master without a static part"))?
        .read_temp_5;
    let missile = combat_master(m).map_or(-1, |c| c.missile_id);
    match class {
        5 | 7 | 8 | 11 | 14 | 15 | 16 | 30..=34 | 38 | 42 | 48 | 63 => Ok(None),
        3 | 27 | 39 | 49 | 60 if missile < 0 => Ok(Some(17)),
        _ => Err(Unsupported(
            "BUILD of a master whose unit-AI class is not modelled",
        )),
    }
}

fn combat_master(
    m: &Master,
) -> Option<&genie_rec::de::header::generated::world::RgeMasterCombatObject> {
    None.or(m
        .tribe_master_combat_object
        .as_ref()
        .map(|c| &c.rge_master_combat_object))
        .or(m
            .tribe_master_building_object
            .as_ref()
            .map(|b| &b.tribe_master_combat_object.rge_master_combat_object))
        .or(m
            .rge_master
            .as_ref()
            .and_then(|r| r.rge_master_combat_object.as_ref()))
}

/// Run the slice over `frames`, each tick with its commands.
fn run(
    turns: &mut Turns,
    decoder: &mut Decoder,
    frames: &[Frame<'_>],
    sections: &[PlayerSection<'_>],
) {
    for frame in frames {
        let commands: Vec<Command> = frame
            .commands
            .iter()
            .filter_map(|p| decoder.command(p, sections))
            .collect();
        turns.tick_with(frame.dt, &commands);
    }
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
    let all = frames(&chapter, usize::MAX)?;
    let first = all
        .iter()
        .position(|f| f.checkpoint.is_some())
        .ok_or(Error::NoCheckpoint)?;
    let checkpoint = all[first].checkpoint.as_ref().expect("found above");
    let group_command = all[..first].iter().enumerate().find_map(|(i, f)| {
        let packet = f.commands.iter().find(|p| GROUP_COMMANDS.contains(&p[0]))?;
        Some(GroupCommand {
            tick: i as u32 + 1,
            player: packet.get(1).copied().unwrap_or_default(),
            command: packet[0],
        })
    });
    let sections = header.players();
    let mut turns = slice(&header, masters);
    run(
        &mut turns,
        &mut Decoder::default(),
        &all[..first],
        &sections,
    );
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
/// leaves alone (the sleeping objects), the slice its resources, its live list (the objects it
/// created included) and each object's master, position, state, held amount, top action and
/// retarget timer.
fn lane_block(section: &PlayerSection<'_>, id: u32, simulated: &Player) -> [u32; 11] {
    let mut state = player_state(section, id);
    state.resources = simulated.resources;
    state.live = simulated
        .objects
        .iter()
        .filter(|unit| unit.id >= 0)
        .map(|unit| ObjectState {
            id: unit.id,
            main_id: unit.life.master_id,
            state: unit.life.state,
            amount_held: unit.life.amount,
            position: unit.position,
            retarget_timer: unit.ai.as_ref().map(|ai| ai.retarget_timer as i32),
            // a producing building's action state is its make action's; its type stays -1
            top_action: unit
                .artifact
                .map(|a| TopAction {
                    kind: ARTIFACT_TYPE,
                    state: a.state,
                })
                .or(unit.build.map(|b| TopAction {
                    kind: genie_core::build::BUILD,
                    state: b.state,
                }))
                .or(unit.production.as_ref().map(|_| TopAction {
                    kind: -1,
                    state: genie_core::production::Production::MAKING,
                })),
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
    let sections = header.players();
    if sections.len() <= id as usize {
        return Err(Error::NoSuchPlayer(id));
    }
    let mut turns = slice(&header, masters);
    run(
        &mut turns,
        &mut Decoder::default(),
        &frames(&chapter, ticks)?,
        &sections,
    );
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
    let mut decoder = Decoder::default();
    for frame in frames(&chapter, ticks)? {
        run(
            &mut turns,
            &mut decoder,
            std::slice::from_ref(&frame),
            &sections,
        );
        out.push(
            ids.iter()
                .map(|&id| lane_block(&sections[id as usize], id, &turns.players[id as usize]))
                .collect(),
        );
    }
    Ok(out)
}
