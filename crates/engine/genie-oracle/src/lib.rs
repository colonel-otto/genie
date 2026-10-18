//! The replay oracle: state read from a recording's header save, checked against the checkpoints
//! the recording stores in its body.
//!
//! The first checkpoint comes 500 syncs into the game; a recording has none at tick 0. Ten of its
//! eleven words per player still equal the start-of-game state ([`SAVE_WORDS`]). Word 9 does not:
//! it sums each object's top action and unit-AI retarget timer, which the engine redraws while the
//! game runs, so reproducing it needs the simulation.

pub mod masters;
pub mod tick_trace;

pub use masters::{DefaultAction, MasterTable, TerrainTable};

use genie_core::artifact::ARTIFACT_TYPE;
use genie_core::checkpoint::{ObjectState, PlayerState, TopAction, player_block};
use genie_core::lifecycle::{Class, Lifecycle, MasterData};
use genie_core::turn::{Command, Player, Turns, Unit};
use genie_core::unit_ai::Unsupported;
use genie_core::visibility::{BuildingSight, Extent, Seer, Visibility, VisibleMap, capture_kind};
use genie_rec::de::{self, Op};
use genie_save::Header;
use genie_save::PlayerSection;
use genie_save::generated::world::{
    Master, Path, PathData, PlayerObjects, RgeMasterAnimatedObject, RgeStaticObject,
};

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
    /// A mesh dump without its header line.
    BadMeshDump,
    /// A registry dump without its header line, or a line out of shape.
    BadRegistryDump,
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
            Error::BadMeshDump => write!(f, "the mesh dump has no mesh line"),
            Error::BadRegistryDump => write!(f, "the registry dump is out of shape"),
        }
    }
}

impl std::error::Error for Error {}

/// The static part of each live object, with its object.
fn live<'a>(
    section: &PlayerSection<'a>,
) -> impl Iterator<Item = (&'a genie_save::Object, &'a RgeStaticObject)> {
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
    let header = chapter.header()?;
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

/// The player attribute of the construction rate modifier (195; the work reads it at player
/// attributes `+0x618`, eight bytes per attribute).
const CONSTRUCTION_RATE: usize = 195;

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

/// The slice's view of one saved object: its lifecycle, its unit AI and its actions. A save from
/// the start of a game holds no actions, so an object starts its master's default task in its
/// first update ([`default_task`]); a save from later in the game holds them, and the slice
/// imports the ones it models ([`saved_actions`]).
fn unit(
    object: &genie_save::Object,
    objects: &PlayerObjects,
    paths: &[Path],
    civ: u8,
    masters: Option<&MasterTable>,
) -> Unit {
    let Some(s) = object.static_part() else {
        return Unit {
            id: -1,
            unmodelled_action: Some(Unsupported("an object without a static part")),
            ..Unit::default()
        };
    };
    let saved = object
        .actions()
        .is_some_and(|a| a.entries().next().is_some());
    let mut unit = Unit {
        id: s.id,
        position: [s.t_x, s.t_y, s.t_z],
        life: lifecycle(object, s, objects, civ, masters),
        ai: unit_ai(object),
        construction: construction(object, s, objects),
        work_rate_powerup: object.combat().map_or(0.0, |c| c.work_rate_powerup),
        attackable: s.unit_attackable_flag != 0,
        pasture: s.parent_pasture_id,
        my_pasture: s.my_pasture_id,
        annex_of: object
            .tribe_building_object
            .as_ref()
            .map(|b| b.mupt_linked_object_references_linked_owner_id)
            .filter(|&owner| owner != -1),
        hp_powerups: object
            .combat()
            .map_or([0.0; 2], |c| [c.hitpoints_powerup, c.hitpoints_max_powerup]),
        workers: s.worker_num,
        last_human_order: s.last_human_order_time,
        combat: object.combat().is_some(),
        attack_timer: object
            .combat()
            .map_or(0.0, |c| c.rge_combat_object.attack_timer),
        reload_stinger: object.combat().map_or(0.0, |c| c.reload_stinger),
        reload_powerup: object.combat().map_or(0.0, |c| c.reload_powerup),
        attack_powerup: object.combat().map_or(0.0, |c| c.attack_powerup),
        armor_powerups: object.combat().map_or([0; 2], |c| {
            [c.melee_armor_powerup as i16, c.pierce_armor_powerup as i16]
        }),
        armor_removed: object
            .combat()
            .map_or([0; 2], |c| [c.melee_armor_removed, c.pierce_armor_removed]),
        charge: object.combat().map_or(0.0, |c| c.charge),
        anim_speed: object
            .moving_part()
            .map(|m| m.rge_animated_object.current_speed),
        charge_hit: object
            .combat()
            .is_some_and(|c| c.rge_combat_object.hit_something_valid_for_charge_attack != 0),
        gather_point: gather_point(object),
        gate: saved_gate(object),
        ..Unit::default()
    };
    let imported = if saved {
        saved_actions(
            object,
            s,
            |m| build_task(objects, civ, masters, m),
            |m| fly_task(objects, civ, masters, m),
            |m, i| task_at(objects, civ, masters, m, i),
            masters.map(|t| &t.graphics),
            &mut unit,
        )
    } else {
        default_task(object, s, civ, masters).map(|artifact| unit.artifact = artifact)
    };
    // what the default task starts once the action list empties: nothing, the Artifact (whose
    // restart the slice does not model) or something not modelled
    unit.default_task = match default_task(object, s, civ, masters) {
        Ok(None) => None,
        Ok(Some(_)) => Some(Unsupported(
            "the default task's restart (the Artifact action)",
        )),
        Err(e) => Some(e),
    };
    let production = production(object, objects).map(|p| unit.production = p);
    let moving = mover(object, s, objects, paths, civ, masters).map(|m| unit.mover = m);
    unit.unmodelled_action = imported.and(production).and(moving).err();
    unit
}

/// A saved building's gate lock ([`genie_core::turn::Gate`]).
fn saved_gate(object: &genie_save::Object) -> Option<genie_core::turn::Gate> {
    let b = object.tribe_building_object.as_ref()?;
    Some(genie_core::turn::Gate {
        locked: b.gate_locked != 0,
    })
}

/// A saved building's construction: its `build_pts`, `desolid_flag` and `num_builders`, its
/// master's hit points (the sixth `readTemp`), build time (the first train location's) and
/// obstruction kind and class. None for an object that is not a building.
fn construction(
    object: &genie_save::Object,
    s: &RgeStaticObject,
    objects: &PlayerObjects,
) -> Option<genie_core::build::Construction> {
    let b = object.tribe_building_object.as_ref()?;
    let m = objects.master(s.master_obj)?;
    let ms = m.static_part()?;
    let build_time = m
        .tribe_master_building_object
        .as_ref()
        .map(|b| &b.tribe_master_combat_object)
        .or(m.tribe_master_combat_object.as_ref())
        .and_then(|c| c.train_location.first())
        .map_or(0, |t| t.train_time);
    Some(genie_core::build::Construction {
        progress: b.build_pts,
        desolid: b.desolid_flag != 0,
        builders: b.tribe_combat_object.num_builders,
        hit_points: ms.read_temp_6,
        build_time,
        record: genie_core::spatial::Obstruction {
            kind: ms.obstruction_type,
            class: ms.obstruction_class,
        },
        snow: b.snow_flag != 0,
        sprite_override: s.sprite_override_id != -1,
    })
}

fn vec3(b: &[u8; 12]) -> genie_core::fmath::Vec3 {
    let f = |i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    [f(0), f(4), f(8)]
}

/// The animated part of a player's master, which holds its speed (`mMaxSpeed`, master `+0x740`).
fn animated_master(m: &Master) -> Option<&RgeMasterAnimatedObject> {
    let combat = m.tribe_master_combat_object.as_ref().or(m
        .tribe_master_building_object
        .as_ref()
        .map(|b| &b.tribe_master_combat_object));
    if let Some(c) = combat {
        let moving = &c
            .rge_master_combat_object
            .rge_master_action_object
            .rge_master_moving_object;
        return Some(&moving.rge_master_animated_object);
    }
    let r = m.rge_master.as_ref()?;
    None.or(r.rge_master_animated_object.as_ref())
        .or(r
            .rge_master_moving_object
            .as_ref()
            .map(|m| &m.rge_master_animated_object))
        .or(r
            .rge_master_action_object
            .as_ref()
            .map(|a| &a.rge_master_moving_object.rge_master_animated_object))
        .or(r.rge_master_combat_object.as_ref().map(|c| {
            &c.rge_master_action_object
                .rge_master_moving_object
                .rge_master_animated_object
        }))
}

/// The moving part of a saved master, when its class has one.
fn moving_master(m: &Master) -> Option<&genie_save::generated::world::RgeMasterMovingObject> {
    let combat = m.tribe_master_combat_object.as_ref().or(m
        .tribe_master_building_object
        .as_ref()
        .map(|b| &b.tribe_master_combat_object));
    if let Some(c) = combat {
        return Some(
            &c.rge_master_combat_object
                .rge_master_action_object
                .rge_master_moving_object,
        );
    }
    let r = m.rge_master.as_ref()?;
    None.or(r.rge_master_moving_object.as_ref())
        .or(r
            .rge_master_action_object
            .as_ref()
            .map(|a| &a.rge_master_moving_object))
        .or(r
            .rge_master_combat_object
            .as_ref()
            .map(|c| &c.rge_master_action_object.rge_master_moving_object))
}

/// One saved level of path data, with its path's waypoints from the world's path table.
fn path_level(d: &PathData, paths: &[Path]) -> Result<genie_core::movement::PathData, Unsupported> {
    // a level without a path (id -1) holds none: a bird's top level
    let path = match d.path_id {
        -1 => None,
        id => Some(paths.iter().find(|p| p.idvalue == id).ok_or(Unsupported(
            "path data whose path is not in the world's table",
        ))?),
    };
    Ok(genie_core::movement::PathData {
        level: d.waypoint_level,
        linked_type: d.linked_path_type,
        points: path.map_or_else(Vec::new, |p| p.m_waypoints.iter().map(vec3).collect()),
        path_flags: path.map_or(0, |p| p.flags_m_bits),
        waypoint: d.waypoint,
        state: d.state,
        range: d.range,
        target: d.target_id,
        pause_time: f32::from_bits(d.pause_time),
        flags: d.flags_m_bits,
        length: path.map_or(0.0, |p| p.path_length_value),
        created: path.map_or(0, |p| p.creation_time_value),
        continue_counter: d.continue_counter,
        repath_time: d.repath_time,
        ignore: path.map_or_else(Vec::new, |p| {
            p.data
                .iter()
                .take(p.number_ignore_units.max(0) as usize)
                .map(|&id| id as i32)
                .collect()
        }),
    })
}

/// A moving object's movement state from the save: orientation, velocity data and path data (the
/// save lists the levels top first; the game walks the last), and from its master and the `.dat`
/// its speed, turning and radii. `None` for an object that does not move (types below 30).
///
/// The top speed is the master's speed times the saved sprite's speed factor when that is above 0
/// (`FUN_14114d1d0`); slot 65's other terms (`+0xa08`, `+0xa2c`, `+0xb38`, a followed object) are
/// not read yet. moveForward's factor `+0x878` is not in the save; every moving object of
/// MarvinBlue/426048731 holds 1.0 at tick 680 (the tick oracle).
fn mover(
    object: &genie_save::Object,
    s: &RgeStaticObject,
    objects: &PlayerObjects,
    paths: &[Path],
    civ: u8,
    masters: Option<&MasterTable>,
) -> Result<Option<genie_core::movement::Mover>, Unsupported> {
    use genie_core::movement::{MoveStats, Mover, Velocity};
    let Some(m) = object.moving_part() else {
        return Ok(None);
    };
    if m.future_path_data.is_some() {
        return Err(Unsupported("saved future path data"));
    }
    // an object with its own master (a copy, say a captured animal's) moves by its own values
    let (radius, restriction, base, turn, flags) = match (s.own_master(), s.own_animated_master()) {
        (Some(own), Some(animated)) => (
            [own.radius_x, own.radius_y, own.radius_z],
            own.terrain_restriction_mask_id,
            animated.max_speed,
            s.own_moving_master()
                .map(|m| (m.turn_speed, m.temp_move_sprite)),
            own.object_flags,
        ),
        (Some(_), None) => {
            return Err(Unsupported(
                "a moving object's own master without an animated part",
            ));
        }
        (None, _) => {
            let master = objects
                .master(s.master_obj)
                .ok_or(Unsupported("a moving object whose master is not saved"))?;
            let placement = master
                .static_part()
                .ok_or(Unsupported("a moving master without a static part"))?;
            let base = animated_master(master)
                .ok_or(Unsupported("a moving master without an animated part"))?
                .max_speed;
            (
                [placement.radius_x, placement.radius_y, placement.radius_z],
                placement.terrain_restriction_mask_id,
                base,
                moving_master(master).map(|m| (m.turn_speed, m.move_sprite_id)),
                placement.object_flags,
            )
        }
    };
    let table = masters.ok_or(Unsupported("no master table: turning is unknown"))?;
    let factor = table
        .sprite_speed(s.sprite)
        .ok_or(Unsupported("a sprite the data file does not have"))?;
    let speed = if 0.0 < factor { base * factor } else { base };
    let turning = table
        .turning(civ, s.master_obj)
        .ok_or(Unsupported("a moving master the data file does not have"))?;
    let (turn_speed, walking) = turn.ok_or(Unsupported("a moving master without a moving part"))?;
    let facets = table.graphics.get(&walking).map(|g| g.facets);
    let velocity = (m.have_movement == 1).then(|| Velocity {
        velocity: m.velocity.as_ref().map_or([0.0; 3], vec3),
        acceleration: m.acceleration.as_ref().map_or([0.0; 3], vec3),
    });
    let levels = m
        .temp_path_data
        .iter()
        .rev()
        .map(|d| path_level(d, paths))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(Mover {
        forward: vec3(&m.orientation_forward_value),
        right: vec3(&m.orientation_right_value),
        velocity,
        paths: levels,
        factor: 1.0,
        stats: MoveStats {
            speed,
            turn_rate: genie_core::movement::turn_rate(turn_speed, facets),
            turn_radius: 0.0,
            turn_radius_speed: f32::MAX,
            radius,
            restriction: usize::try_from(restriction)
                .map_err(|_| Unsupported("a negative terrain restriction"))?,
            restriction_mask: usize::try_from(restriction)
                .ok()
                .and_then(|r| table.terrain.masks().get(r))
                .copied()
                .ok_or(Unsupported(
                    "a terrain restriction the data file does not have",
                ))?,
            turns_at_once: turn_speed == 0.0,
            move_algorithm: turning.move_algorithm,
            min_collision: turning.min_collision_size_multiplier,
            fast_turn: flags & 2 != 0,
        },
        waiting: m.waiting_to_move_value,
        wait_delays: m.number_wait_delays,
        attack_moving: m.attack_moving != 0,
        last_move: m.last_move_time_value,
        substitute_count: m.consecutive_substitute_count,
        ignore_units: s.temp_int.clone(),
        move_to_point: saved_move_to_point(object),
        speed_stopped: false,
        pauses: Vec::new(),
    }))
}

/// The point of the MoveTo a saved unit walks with to a target object: its top action's first
/// sub-action (or that Attack's own first one) when that is a MoveTo with a target (its
/// `target_x`..`target_z`).
fn saved_move_to_point(object: &genie_save::Object) -> Option<[f32; 3]> {
    let (_, top) = object.actions()?.entries().next()?;
    let (mut kind, mut sub) = top.base()?.sub_actions.entries().next()?;
    // a Hunt's Attack walks with a MoveTo of its own
    if kind == genie_core::attack::ATTACK {
        (kind, sub) = sub.base()?.sub_actions.entries().next()?;
    }
    let b = sub.base()?;
    (kind == genie_core::move_to::MOVE_TO && b.target_id != -1)
        .then_some([b.target_x, b.target_y, b.target_z])
}

/// The top action of a saved object as the slice models it: the Artifact action (107), the
/// Build action (101) with its MoveTo, a MoveTo of its own (type 1), a bird's Fly action (10)
/// with its MoveTo and the Hunt (110) with its Attack or MoveTo (and in state 8 its second
/// target, the drop site). Any other saved action stops the object's player.
fn saved_actions(
    object: &genie_save::Object,
    s: &RgeStaticObject,
    work: impl Fn(i16) -> Result<genie_core::build::BuildTask, Unsupported>,
    fly_task: impl Fn(i16) -> Result<genie_core::masters::Task, Unsupported>,
    task_at: impl Fn(i16, i16) -> Result<genie_core::masters::Task, Unsupported>,
    graphics: Option<&std::collections::BTreeMap<i16, genie_core::sprite::Graphic>>,
    unit: &mut Unit,
) -> Result<(), Unsupported> {
    use genie_core::build::{BUILD, BuildAction, MoveTo};
    use genie_core::fly::{FLY, Fly};
    use genie_core::move_to::{MOVE_TO, MoveToAction};
    let move_to_state = |state: u8| match state {
        1 => Ok(MoveTo::Arrived),
        2 => Ok(MoveTo::Waiting),
        3 => Ok(MoveTo::Start),
        4 => Ok(MoveTo::Moving),
        13 => Ok(MoveTo::Failed),
        _ => Err(Unsupported(
            "a saved MoveTo in a state the slice does not model",
        )),
    };
    let point = |b: &genie_save::generated::world::RgeAction| {
        (b.target_id == -1).then_some([b.target_x, b.target_y, b.target_z])
    };
    let (kind, action) = object
        .actions()
        .and_then(|a| a.entries().next())
        .expect("saved_actions is called for an object with actions");
    let base = action
        .base()
        .ok_or(Unsupported("a saved action without its base"))?;
    match kind {
        ARTIFACT_TYPE => {
            let move_to = match base.sub_actions.entries().next() {
                None => None,
                Some((MOVE_TO, sub)) => Some(move_to_state(sub.base().map_or(0, |b| b.state))?),
                Some(_) => {
                    return Err(Unsupported(
                        "a saved Artifact action with another sub-action",
                    ));
                }
            };
            unit.artifact = Some(genie_core::artifact::Artifact {
                state: base.state,
                timer: base.timer,
                gaia: s.owner == 0,
                move_to,
                goal: move_to.and(point(base)),
            });
            Ok(())
        }
        MOVE_TO => {
            unit.move_to = Some(MoveToAction {
                state: move_to_state(base.state)?,
                goal: point(base),
                range: action
                    .move_to_range()
                    .ok_or(Unsupported("a saved MoveTo without its range"))?,
            });
            Ok(())
        }
        FLY => {
            let sub = match base.sub_actions.entries().next() {
                None => None,
                Some((MOVE_TO, sub)) => sub.base(),
                Some(_) => return Err(Unsupported("a saved Fly action with another sub-action")),
            };
            let move_to = sub.map(|b| move_to_state(b.state)).transpose()?;
            unit.fly = Some(Fly {
                state: base.state,
                timer: base.timer,
                move_to,
                goal: sub.and_then(point),
                home: [base.target_x, base.target_y],
                task: fly_task(s.master_obj)?,
            });
            Ok(())
        }
        BUILD => {
            let move_to = match base.sub_actions.entries().next() {
                None => None,
                Some((MOVE_TO, sub)) => Some(move_to_state(sub.base().map_or(0, |b| b.state))?),
                Some(_) => return Err(Unsupported("a saved Build action with another sub-action")),
            };
            // a MoveTo to a point has no target; its point is the action's target point
            let goal = base
                .sub_actions
                .entries()
                .next()
                .and_then(|(_, sub)| sub.base())
                .filter(|b| b.target_id == -1)
                .map(|b| [b.target_x, b.target_y, b.target_z]);
            let mut task = work(s.master_obj)?;
            task.work_rate += unit.work_rate_powerup;
            unit.build = Some(BuildAction {
                target: base.target_id,
                state: base.state,
                move_to,
                goal,
                task,
                added: action
                    .tribe_action_build
                    .as_ref()
                    .is_some_and(|b| b.added_to_num_builders != 0),
            });
            Ok(())
        }
        genie_core::hunt::HUNT => {
            use genie_core::hunt::{ATTACK, DROP, FACE, GATHER, HuntAction, HuntTask, WALK};
            let part = action
                .tribe_action_hunt
                .as_ref()
                .ok_or(Unsupported("a saved Hunt without its own part"))?;
            if !matches!(base.state, WALK | FACE | GATHER | DROP | ATTACK) {
                return Err(Unsupported(
                    "a saved Hunt in a state the slice does not model",
                ));
            }
            let t = task_at(s.master_obj, base.short_val)?;
            let task = HuntTask {
                id: t.id,
                range: t.work_range,
                resource: t.attribute_type,
                sprites: t.sprites,
                work_value: t.work_val1,
                productivity: t.attribute_type2,
                kind4: t.attribute_type4,
            };
            let mut hunt = HuntAction {
                target: base.target_id,
                state: base.state,
                point: [base.target_x, base.target_y, base.target_z],
                target_class: part.target_type as i16,
                task,
                attack: None,
                move_to: None,
                walk_range: task.range,
                drop_site: part.sheep_drop_site_id as i32,
                carry_to: base.target2_id,
                timer: base.timer,
                move_wait: genie_core::move_to::MoveWait::default(),
            };
            match base.sub_actions.entries().next() {
                None => {}
                Some((genie_core::attack::ATTACK, sub)) => {
                    hunt.attack = Some(saved_attack(sub, graphics, &move_to_state)?);
                }
                Some((MOVE_TO, sub)) => {
                    hunt.move_to = Some(move_to_state(sub.base().map_or(0, |b| b.state))?);
                    hunt.walk_range = sub
                        .move_to_range()
                        .ok_or(Unsupported("a saved MoveTo without its range"))?;
                    // its wait (`+0x6c`, `+0x70`, `+0x74`)
                    if let Some(m) = sub
                        .rge_action_by_type
                        .as_ref()
                        .and_then(|b| b.rge_action_move_to.as_ref())
                    {
                        hunt.move_wait = genie_core::move_to::MoveWait {
                            state: m.movement_wait_state,
                            timer: m.timer,
                            delay: f32::from_bits(m.k_default_wait_time),
                        };
                    }
                }
                Some(_) => return Err(Unsupported("a saved Hunt with another sub-action")),
            }
            unit.hunt = Some(hunt);
            Ok(())
        }
        genie_core::explore::EXPLORE => {
            let move_to = match base.sub_actions.entries().next() {
                None => None,
                Some((MOVE_TO, sub)) => Some(move_to_state(sub.base().map_or(0, |b| b.state))?),
                Some(_) => {
                    return Err(Unsupported("a saved Explore with another sub-action"));
                }
            };
            if !matches!(
                base.state,
                genie_core::explore::WAIT | genie_core::explore::WALK
            ) {
                return Err(Unsupported(
                    "a saved Explore in a state the slice does not model",
                ));
            }
            unit.explore = Some(genie_core::explore::ExploreAction {
                state: base.state,
                point: [base.target_x, base.target_y, base.target_z],
                move_to,
            });
            Ok(())
        }
        _ => Err(Unsupported("a saved action the slice does not import")),
    }
}

/// A saved Attack sub-action (type 9, `FUN_140a7c880`): its target, state, timer, ranges, point,
/// the target's last place (`+0xa4`..), a hit still to land (`+0x9e`), its MoveTo, and its sprites
/// and frames. The load restores the sprites at `+0x70`, `+0x80` and `+0x88` from the second to
/// fourth saved ids (the first goes to `+0x68`); the second attack sprite (`+0x78`) and the
/// swing's times (`+0xb0`, `+0xb4`) are not saved, and a loaded Attack starts without them.
fn saved_attack(
    sub: &genie_save::generated::world::Action,
    graphics: Option<&std::collections::BTreeMap<i16, genie_core::sprite::Graphic>>,
    move_to_state: &dyn Fn(u8) -> Result<genie_core::move_to::MoveTo, Unsupported>,
) -> Result<genie_core::attack::Attack, Unsupported> {
    use genie_core::move_to::MOVE_TO;
    let b = sub
        .base()
        .ok_or(Unsupported("a saved Attack without its base"))?;
    let a = sub
        .rge_action_by_type
        .as_ref()
        .and_then(|t| t.rge_action_attack.as_ref())
        .ok_or(Unsupported("a saved Attack without its own part"))?;
    let graphics = graphics.ok_or(Unsupported("a saved Attack without the sprites' table"))?;
    let g = |id: u16| {
        let id = id as i16;
        (0 <= id).then(|| graphics.get(&id).copied()).flatten()
    };
    let move_to = match b.sub_actions.entries().next() {
        None => None,
        Some((MOVE_TO, s2)) => Some(move_to_state(s2.base().map_or(0, |b| b.state))?),
        Some(_) => return Err(Unsupported("a saved Attack with another sub-action")),
    };
    Ok(genie_core::attack::Attack {
        target: b.target_id,
        state: b.state,
        timer: b.timer,
        range: a.range,
        min_range: a.min_range,
        point: [b.target_x, b.target_y, b.target_z],
        seen: [
            a.last_target_position_x,
            a.last_target_position_y,
            a.last_target_position_z,
        ],
        move_to,
        pending: a.need_to_attack != 0,
        swing: 0.0,
        rest: 0.0,
        arms: genie_core::attack::Arms {
            sprite: g(a.short_val_2),
            second_sprite: None,
            idle_sprite: g(a.short_val_3),
            idle_attack_sprite: g(a.short_val_4),
            missile: a.missile_id,
            fire_frame: a.frame_delay,
            interrupt_frame: a.interrupt_animation_at_frame,
        },
    })
}

/// A builder's Build task (the game's Build action keeps a pointer to it, in its master's saved
/// task list or, for a master that saves none, in the `.dat`'s): its work range (task `+0x30`)
/// and work value (`+0x28`), with the master's work rate (its action part's `work_rate`).
fn build_task(
    objects: &PlayerObjects,
    civ: u8,
    masters: Option<&MasterTable>,
    master: i16,
) -> Result<genie_core::build::BuildTask, Unsupported> {
    let m = objects
        .master(master)
        .ok_or(Unsupported("a builder whose master is not saved"))?;
    let work_rate = combat_master(m)
        .map(|c| c.rge_master_action_object.work_rate)
        .ok_or(Unsupported("a builder master without an action part"))?;
    let (range, work_value) = build_range(objects, civ, masters, m, master)?;
    Ok(genie_core::build::BuildTask {
        range,
        work_value,
        work_rate,
    })
}

/// A bird master's Fly task: its saved task list's, else the `.dat`'s.
fn fly_task(
    objects: &PlayerObjects,
    civ: u8,
    masters: Option<&MasterTable>,
    master: i16,
) -> Result<genie_core::masters::Task, Unsupported> {
    let fly = genie_core::fly::FLY;
    let saved = objects
        .master(master)
        .and_then(|m| m.rge_master.as_ref())
        .and_then(|r| r.rge_master_action_object.as_ref())
        .and_then(|a| a.tasks.as_ref());
    if let Some(tasks) = saved {
        return tasks
            .list
            .iter()
            .find(|e| e.task.action_type == fly)
            .map(|e| core_task(&e.task))
            .ok_or(Unsupported("a bird whose master has no Fly task"));
    }
    masters
        .and_then(|t| t.work(civ, master))
        .and_then(|w| w.tasks.iter().find(|t| t.action_type == fly))
        .copied()
        .ok_or(Unsupported("a bird whose Fly task is not in the data file"))
}

/// The work range and value of a builder master's Build task.
fn build_range(
    _objects: &PlayerObjects,
    civ: u8,
    masters: Option<&MasterTable>,
    m: &Master,
    master: i16,
) -> Result<(f32, f32), Unsupported> {
    let saved = m.tribe_master_combat_object.as_ref().and_then(|c| {
        c.rge_master_combat_object
            .rge_master_action_object
            .tasks
            .as_ref()
    });
    let build = genie_core::build::BUILD;
    if let Some(tasks) = saved {
        return tasks
            .list
            .iter()
            .find(|e| e.task.action_type == build)
            .map(|e| (e.task.work_range, e.task.work_val1))
            .ok_or(Unsupported("a builder whose master has no Build task"));
    }
    masters
        .and_then(|t| t.work(civ, master))
        .and_then(|w| w.tasks.iter().find(|t| t.action_type == build))
        .map(|t| (t.work_range, t.work_val1))
        .ok_or(Unsupported(
            "a builder whose Build task is not in the data file",
        ))
}

/// A saved building's production (`TRIBE_Building_Object`'s queue and its make action): each queued
/// unit with its train time, the make action's work done, and the building's work rate (its
/// `mWorkRatePowerup` plus its master's `work_rate`). `Ok(None)`: the building produces nothing.
fn production(
    object: &genie_save::Object,
    objects: &PlayerObjects,
) -> Result<Option<genie_core::production::Production>, Unsupported> {
    let Some(building) = object.tribe_building_object.as_ref() else {
        return Ok(None);
    };
    let queue: std::collections::VecDeque<_> = building
        .production_queue
        .iter()
        .take(building.production_queue_count.max(0) as usize)
        .flat_map(|e| {
            let unit = genie_core::production::Queued {
                master: e.production_queue_master_unit_id,
                train_time: f32::from(e.production_queue_train_time),
            };
            std::iter::repeat_n(unit, e.production_queue_unit_count.max(0) as usize)
        })
        .collect();
    // the make action, if the building made it yet (it comes in the building's next update)
    let progress = match building.production_queue_actions.entries().next() {
        None if queue.is_empty() => return Ok(None),
        None => None,
        Some((102, action)) => Some(
            action
                .tribe_action_make_obj
                .as_ref()
                .ok_or(Unsupported("a saved make action without its part"))?
                .work_done,
        ),
        Some(_) => {
            return Err(Unsupported(
                "a saved production action other than making a unit",
            ));
        }
    };
    let master = building
        .tribe_combat_object
        .rge_combat_object
        .rge_action_object
        .rge_moving_object
        .rge_animated_object
        .rge_static_object
        .master_obj;
    let work_rate = objects
        .master(master)
        .and_then(combat_master)
        .map(|c| c.rge_master_action_object.work_rate)
        .ok_or(Unsupported("a building master without an action part"))?;
    Ok(Some(genie_core::production::Production {
        queue,
        action: progress.is_some(),
        progress: progress.unwrap_or(0.0),
        rate: building.tribe_combat_object.work_rate_powerup + work_rate,
    }))
}

/// A building's gather point, if it has one (`gather_point_exists`).
fn gather_point(object: &genie_save::Object) -> Option<genie_core::turn::GatherPoint> {
    let b = object
        .tribe_building_object
        .as_ref()
        .filter(|b| b.gather_point_exists != 0)?;
    Some(genie_core::turn::GatherPoint {
        point: [b.gather_point_x, b.gather_point_y, b.gather_point_z],
        target: b.gather_point_target,
        target_master: b.gather_point_target_master,
    })
}

/// What an object's first update starts of its master's default task (`RGE_Action_Object::update`,
/// 0x14106ac90): only action objects (types 40 and up) have one. The default task is game data:
/// the table's, from the `.dat`, or the object's own master's. The slice models the Artifact action
/// (type 107, the default task of livestock); any other default task is reported.
fn default_task(
    object: &genie_save::Object,
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
        // the action list's factory (`FUN_140aca4b0`, then the RGE one `FUN_140a97e00`) makes
        // no action of the other types: the default task starts nothing (wild animals' Explore,
        // type 6)
        DefaultAction::Type(t) if !FACTORY_TYPES.contains(&t) => Ok(None),
        DefaultAction::Type(_) => Err(Unsupported("a default task other than the Artifact action")),
        DefaultAction::NoSuchTask => Err(Unsupported("a default task id that names no task")),
    }
}

/// The task types the action list's factory makes an action of: the TRIBE factory's
/// (`FUN_140aca4b0`) and the RGE factory's it falls back on (`FUN_140a97e00`).
const FACTORY_TYPES: [i16; 28] = [
    3, 0xc, 0xe, 0x65, 0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6e, 0x6f, 0x78, 0x79, 0x7a, 0x7b, 0x7d,
    0x83, 0x84, 0x87, 0x88, 0x95, 0x9a, 4, 5, 7, 10, 0xd, 0x15,
];

/// Whether the object runs `RGE_Action_Object::update`: types 40 (action), 50 (combat), 60
/// (missile), 70 (tribe combat) and 80 (building).
fn is_action_object(object: &genie_save::Object) -> bool {
    object.combat().is_some()
        || object.rge_object.as_ref().is_some_and(|r| {
            r.rge_action_object.is_some()
                || r.rge_combat_object.is_some()
                || r.rge_missile_object.is_some()
        })
}

/// The default task id of an object's own action master, if it has one.
fn own_default_task(s: &RgeStaticObject) -> Option<i16> {
    let tribe = |c: &genie_save::generated::world::OwnTribeMasterCombatObject| {
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
    object: &genie_save::Object,
    s: &RgeStaticObject,
    objects: &PlayerObjects,
    civ: u8,
    masters: Option<&MasterTable>,
) -> Lifecycle {
    let pure_static = object
        .rge_object
        .as_ref()
        .is_some_and(|r| r.rge_static_object.is_some());
    let death_sprite = match s.own_master() {
        Some(own) => own.death_sprite_num,
        None => objects
            .master(s.master_obj)
            .and_then(|m| m.static_part())
            .map_or(-1, |m| m.death_sprite_id),
    };
    // the sprite after the death: the dying one, else the current one
    let after = if death_sprite >= 0 {
        death_sprite
    } else {
        s.sprite
    };
    let death_facets = masters
        .and_then(|t| t.graphics.get(&after))
        .filter(|g| pure_static && g.flags & 4 != 0 && 1 < g.facets)
        .map(|g| g.facets);
    let (dying_sprite, [death_spawn, remains], master) = match s.own_master() {
        Some(own) => (
            (own.death_sprite_num >= 0).then_some(own.death_sprite_num),
            [own.death_spawn_obj_id, own.death_spawn_obj_id2],
            Some(MasterData {
                decay: own.attribute_rot,
                undead: own.undead_flag != 0,
            }),
        ),
        None => {
            let player = objects.master(s.master_obj).and_then(|m| m.static_part());
            (
                player.map(|m| m.death_sprite_id).filter(|&d| d >= 0),
                player.map_or([-1, -1], |m| [m.death_spawn_obj_id, m.death_spawn_obj_id2]),
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
        death_spawn: death_spawn >= 0,
        remains,
        death_facets,
        inside: s.inside_obj != -1,
        goto_sleep: s.goto_sleep_flag != 0,
        master_id: s.master_obj,
        master,
    }
}

fn unit_ai(object: &genie_save::Object) -> Option<genie_core::unit_ai::UnitAi> {
    let combat = object.combat()?;
    let ai = combat.unit_ai()?;
    Some(genie_core::unit_ai::UnitAi {
        ai_type: combat.need_unit_ai as u32,
        current_order: ai.current_order_value,
        order_priority: ai.current_order_priority_value,
        current_action: ai.current_action_value,
        current_target: ai.current_target_value,
        target_type: ai.current_target_type_value,
        target_point: [
            ai.current_target_xvalue,
            ai.current_target_yvalue,
            ai.current_target_zvalue,
        ],
        last_order: ai.last_order_value,
        last_action: ai.last_action_value,
        last_target: ai.last_target_value,
        last_target_type: ai.last_target_type_value,
        idle_timer: ai.idle_timer_value,
        adjusted_idle_timeout: ai.adjusted_idle_timeout_value,
        idle_timeout: ai.idle_timeout_value,
        secondary_timer: ai.secondary_timer_value,
        look_around_timer: ai.look_around_timer_value,
        look_around_timeout: ai.look_around_timeout_value,
        last_retarget_time: ai.last_retarget_time,
        retarget_timer: ai.randomized_retarget_timer,
        stance: ai.state_value,
        state_position: [ai.state_position_xvalue, ai.state_position_yvalue],
        retarget_disabled: ai.disable_retargeting,
        alert_mode: ai.alert_mode_value != 0,
        patrol_path: ai.has_patrol_path == 1,
        stop_after_kill: ai.stop_after_target_killed_value != 0,
        home: combat
            .tribe_elephant_unit_ai_module
            .as_ref()
            .map(|e| genie_core::unit_ai::Home {
                origin: [e.origin_x, e.origin_y, e.origin_z],
                furious: e.furious != 0,
                go_home: false,
            }),
        hunted: None,
        last_update_time: ai.last_update_time_value,
        orders: ai
            .temp_order
            .iter()
            .map(genie_core::unit_ai::QueuedOrder::from_bytes)
            .collect(),
        notifies: ai
            .temp_notify
            .iter()
            .map(genie_core::unit_ai::Notify::from_bytes)
            .collect(),
    })
}

/// Why the slice stops on a command it does not decode, by the command's name.
fn unmodelled_command(p: &genie_rec::de::command::Command) -> &'static str {
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
        Follow,
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
        Research,
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
                    .map(|o| unit(o, p.objects(), &header.world.base_world.path, civ, masters))
                    .collect(),
                sleeping: p
                    .sleeping_objects()
                    .map(|o| unit(o, p.objects(), &header.world.base_world.path, civ, masters))
                    .collect(),
                computer: false,
                resources: [resource(0), resource(1), resource(2), resource(3)],
                construction_rate: resource(CONSTRUCTION_RATE),
                attributes: attributes.clone(),
                civ,
                relations: p.world_player().base.relation.clone(),
                auto_farm_queue: p.world_player().auto_farm_queue != 0,
                auto_fishtrap_queue: p.world_player().auto_fish_trap_queue != 0,
                unsupported: None,
                freezes: Vec::new(),
                stopped: false,
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
    let none = TerrainTable::default();
    let terrain = masters.map_or(&none, |t| &t.terrain);
    turns.spatial = spatial(header, terrain);
    let map = &world.map;
    turns.pathing = genie_core::pathing::Pathing::new(
        [map.map_width, map.map_height],
        map.rows
            .iter()
            .flat_map(|r| r.tiles.iter().map(|t| t.terrain_type))
            .collect(),
        terrain.passable().to_vec(),
        header.world.game_type,
    );
    // a statistics flag the tick oracle reads as 1 in MarvinBlue/426048731 (Turns::unit_cost_246)
    turns.unit_cost_246 = true;
    turns.game_mode = header.file_header.game_options_read.game_options.game_mode;
    if let Some(table) = masters {
        turns.drop_sites = table.drop_sites.clone();
        turns.graphics = table.graphics.clone();
        turns.masters = header
            .players()
            .iter()
            .map(|p| work_masters(p.objects(), p.world_player().base.civ_id, table))
            .collect();
        // the players' unit masters (type 70), Gaia's left out
        let all = || {
            turns
                .masters
                .iter()
                .skip(1)
                .flat_map(|m| m.iter())
                .filter(|m| m.kind == 70)
        };
        turns.spatial.fastest = all()
            .filter_map(|m| m.moving.as_ref().map(|s| s.speed))
            .fold(0.0, f32::max);
        turns.spatial.reach = all()
            .flat_map(|m| m.tasks.iter().map(|t| t.work_range).chain([m.weapon_range]))
            .fold(0.0, f32::max);
    }
    turns
}

/// A saved task as the engine reads it.
/// A master's action part, whatever class it is: what holds its work rate, search radius and
/// task list.
fn action_part(m: &Master) -> Option<&genie_save::generated::world::RgeMasterActionObject> {
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

/// The task at `index` in a master's task list (its saved list, else the `.dat`'s): the task a
/// saved action's base `short_val` names (the base load `FUN_140a7a2f0` resolves it to `+0x48`
/// at `0x140a7b5e8`).
fn task_at(
    objects: &PlayerObjects,
    civ: u8,
    masters: Option<&MasterTable>,
    master: i16,
    index: i16,
) -> Result<genie_core::masters::Task, Unsupported> {
    let i = usize::try_from(index).map_err(|_| Unsupported("a saved action without its task"))?;
    let saved = objects
        .master(master)
        .and_then(action_part)
        .and_then(|a| a.tasks.as_ref());
    if let Some(tasks) = saved {
        return tasks
            .list
            .get(i)
            .map(|e| core_task(&e.task))
            .ok_or(Unsupported(
                "a saved action's task beyond its master's list",
            ));
    }
    masters
        .and_then(|t| t.work(civ, master))
        .and_then(|w| w.tasks.get(i))
        .copied()
        .ok_or(Unsupported(
            "a saved action's task the data file does not have",
        ))
}

fn core_task(t: &genie_save::generated::world::Task) -> genie_core::masters::Task {
    genie_core::masters::Task {
        id: t.id,
        action_type: t.action_type,
        object_group: t.object_group,
        object_id: t.object_id,
        combat_level: t.combat_level,
        terrain: t.terrain_id,
        owner_type: t.owner_type,
        holding_attr: t.holding_attr,
        attribute_type: t.attribute_type,
        attribute_type2: t.attribute_type2,
        attribute_type3: t.attribute_type3,
        attribute_type4: t.attribute_type4,
        wait_time: t.search_wait_time,
        work_val1: t.work_val1,
        work_val2: t.work_val2,
        work_range: t.work_range,
        enablement: t.enablement_resource_id,
        sprites: [t.short_val, t.short_val_2, t.short_val_3, t.short_val_4],
    }
}

/// A player's masters as the work command reads them: each master the save holds, with its saved
/// class, radii, cost and (where saved) tasks, and the `.dat`'s task swap group and type, which
/// the save does not carry (the game copies them from the `.dat` master).
fn work_masters(
    objects: &PlayerObjects,
    civ: u8,
    table: &MasterTable,
) -> genie_core::masters::Masters {
    let task = core_task;
    let saved = objects.masters.iter().flatten().map(|m| &m.master);
    genie_core::masters::Masters::new(saved.filter_map(|m| {
        let s = m.static_part()?;
        let mut work = table.work(civ, s.id())?.clone();
        work.class = s.read_temp_5;
        work.radius = [s.radius_x, s.radius_y];
        work.tall = 0.0 < s.radius_z;
        work.obstruction_type = s.obstruction_type;
        work.buildable_on = s.can_be_built_on;
        work.obstruction_class = s.obstruction_class;
        work.object_flags = s.object_flags;
        work.hit_points = s.read_temp_6;
        work.line_of_sight = s.los;
        work.capacity = s.attribute_max_amount;
        work.blast_defense = s.area_effect_object_level;
        if let Some(a) = action_part(m) {
            work.work_rate = a.work_rate;
            work.search_radius = a.search_radius;
            if let Some(tasks) = a.tasks.as_ref() {
                work.tasks = tasks.list.iter().map(|e| task(&e.task)).collect();
            }
        }
        let combat = m.tribe_master_combat_object.as_ref().or(m
            .tribe_master_building_object
            .as_ref()
            .map(|b| &b.tribe_master_combat_object));
        if let Some(c) = combat {
            work.cost = build_cost(&c.build_inventory);
            work.build_time = c.train_location.first().map_or(0, |t| t.train_time);
            work.charge_type = c.charge_type;
            work.formation_category = c.temp_formation_category;
            let rc = &c.rge_master_combat_object;
            work.weapon_range = rc.weapon_range;
            work.min_range = rc.minimum_weapon_range;
            work.missile = rc.missile_id;
            work.attack_sprite = rc.fight_sprite_id;
            work.fire_frame = rc.fire_missile_at_frame;
            work.interrupt_frame = rc.interrupt_frame;
            work.hit_chance = rc.base_hit_chance;
            work.reload = rc.speed_of_attack;
            work.weapons = rc
                .weapon
                .iter()
                .map(|&w| ((w & 0xffff) as i16, (w >> 16) as i16))
                .collect();
            work.base_armor = rc.base_armor;
            work.armors = rc
                .armor
                .iter()
                .map(|&a| ((a & 0xffff) as i16, (a >> 16) as i16))
                .collect();
            work.resistance = rc.bonus_damage_resistance;
            work.combat_ability = rc.combat_ability;
            work.reflection = rc.damage_reflection;
            work.friendly_fire = rc.friendly_fire_damage;
            work.area_effect = rc.area_effect_range;
            work.blast_level = rc.area_effect_level;
            work.area_damage = rc.area_damage;
            work.invulnerability = c.invulnerability_level;
            work.attack_priority = c.attack_priority;
            work.special_attack = c.special_attack_flag;
            work.max_charge = c.max_charge;
            work.charge_target = c.charge_target;
        }
        work.storage = s
            .attribute
            .iter()
            .map(|e| {
                (
                    e.attribute_type_held,
                    e.attribute_amount_held,
                    e.attribute_flag,
                )
            })
            .collect();
        // the save holds the foundation terrain of its own building masters
        if let (Some(b), Some(saved)) = (
            work.building.as_mut(),
            m.tribe_master_building_object.as_ref(),
        ) {
            b.foundation_terrain = saved.on_build_make_tile;
        }
        work.damage_sprites = !s.damage_sprite.is_empty();
        work.standing_sprite = s.idle_sprite_id;
        work.death_sprite = s.death_sprite_id;
        work.garrison = s.obj_max;
        work.death_spawns = [s.death_spawn_obj_id, s.death_spawn_obj_id2];
        work.life = table.get(civ, s.id());
        work.moving = moving_stats(m, s, civ, table);
        Some(work)
    }))
}

/// What a unit of a saved master moves with ([`genie_core::movement::MoveStats`]): the master's
/// top speed times its walking sprite's speed factor when above 0 (the run sprite if it has one,
/// else the move sprite: the MoveTo's, slots 2000 and 0x7c8), the `.dat`'s turning, the radii and
/// the terrain restriction with its mask.
fn moving_stats(
    m: &Master,
    s: &genie_save::generated::world::RgeMasterStaticObject,
    civ: u8,
    table: &MasterTable,
) -> Option<genie_core::movement::MoveStats> {
    let moving = moving_master(m)?;
    let sprite = if moving.run_sprite_id >= 0 {
        moving.run_sprite_id
    } else {
        moving.move_sprite_id
    };
    let base = moving.rge_master_animated_object.max_speed;
    let factor = table.sprite_speed(sprite).unwrap_or(0.0);
    let turning = table.turning(civ, s.id())?;
    let restriction = usize::try_from(s.terrain_restriction_mask_id).ok()?;
    let facets = table.graphics.get(&moving.move_sprite_id).map(|g| g.facets);
    Some(genie_core::movement::MoveStats {
        speed: if 0.0 < factor { base * factor } else { base },
        turn_rate: genie_core::movement::turn_rate(moving.turn_speed, facets),
        turn_radius: 0.0,
        turn_radius_speed: f32::MAX,
        radius: [s.radius_x, s.radius_y, s.radius_z],
        restriction,
        restriction_mask: *table.terrain.masks().get(restriction)?,
        turns_at_once: moving.turn_speed == 0.0,
        move_algorithm: turning.move_algorithm,
        min_collision: turning.min_collision_size_multiplier,
        fast_turn: s.object_flags & 2 != 0,
    })
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

/// What movement reads of a saved object's master: radii, class and obstruction kind, from the
/// object's own master or its player's.
/// How fast an object may move, for the slice's bound on a frozen object's place: its own or its
/// player's master's speed, 0 for an object that does not move.
fn moving_speed(o: &genie_save::Object, s: &RgeStaticObject, objects: &PlayerObjects) -> f32 {
    if o.moving_part().is_none() {
        return 0.0;
    }
    if s.own_master().is_some() {
        return s
            .own_animated_master()
            .map_or(f32::INFINITY, |a| a.max_speed);
    }
    objects
        .master(s.master_obj)
        .and_then(animated_master)
        .map_or(f32::INFINITY, |a| a.max_speed)
}

/// What movement and sight read of an object's master (its own or its player's).
struct Placement {
    radius: [f32; 3],
    class: i16,
    obstruction_type: u8,
    obstruction_class: u8,
    buildable_on: u8,
    los: f32,
}

fn placement(s: &RgeStaticObject, objects: &PlayerObjects) -> Option<Placement> {
    if let Some(own) = s.own_master() {
        // an own master's second readTemp is its class (its first the id, its third the hit
        // points: as the shared masters' of the same id read)
        return Some(Placement {
            radius: [own.radius_x, own.radius_y, own.radius_z],
            class: own.read_temp_2,
            obstruction_type: own.obstruction_type,
            obstruction_class: own.obstruction_class,
            buildable_on: own.can_be_built_on,
            los: own.los,
        });
    }
    let m = objects.master(s.master_obj)?.static_part()?;
    Some(Placement {
        radius: [m.radius_x, m.radius_y, m.radius_z],
        class: m.read_temp_5,
        obstruction_type: m.obstruction_type,
        obstruction_class: m.obstruction_class,
        buildable_on: m.can_be_built_on,
        los: m.los,
    })
}

/// The world's objects as movement sees them: every player's live and sleeping objects with their
/// positions and their masters' radii and class; the players' doppelgangers count on their tiles. An object has an obstruction record when its
/// master has an obstruction kind and all three radii, unless it is a building marked desolid (a
/// foundation no builder has started; the first builder's Build state 7 adds its record). Checked
/// against the game's obstruction manager in `tests/obstructions.rs`.
/// The map's terrain per tile from the save, with the `.dat`'s terrain records.
fn map_terrain(header: &Header, terrain: &TerrainTable) -> genie_core::terrain::Terrain {
    let map = &header.world.base_world.map;
    let tiles = map
        .rows
        .iter()
        .flat_map(|r| {
            r.tiles
                .iter()
                .map(|t| (t.terrain_type, t.restore_terrain_type, t.layer))
        })
        .collect();
    genie_core::terrain::Terrain::new(
        map.map_width,
        map.map_height,
        tiles,
        terrain.kinds().to_vec(),
        terrain.passable().to_vec(),
    )
}

/// The differences between the slice's map terrain and a save's, tile by tile.
pub fn terrain_differences(
    slice: &genie_core::terrain::Terrain,
    save: &[u8],
) -> Result<Vec<String>, Error> {
    let chapter = de::chapters(save)?[0];
    Ok(header_terrain_differences(slice, &chapter.header()?))
}

/// [`terrain_differences`] of a read header.
fn header_terrain_differences(
    slice: &genie_core::terrain::Terrain,
    header: &Header,
) -> Vec<String> {
    let (width, tiles) = saved_terrain(header);
    tile_differences(slice, width, &tiles)
}

/// A save's terrain per tile, row by row: the terrain, the terrain to restore, the layer; and the
/// map's width.
fn saved_terrain(header: &Header) -> (usize, Vec<(u8, u8, i16)>) {
    let map = &header.world.base_world.map;
    let tiles = map
        .rows
        .iter()
        .flat_map(|r| {
            r.tiles
                .iter()
                .map(|t| (t.terrain_type, t.restore_terrain_type, t.layer))
        })
        .collect();
    (map.map_width as usize, tiles)
}

/// The tiles whose terrain differs between the slice and the game's.
fn tile_differences(
    slice: &genie_core::terrain::Terrain,
    width: usize,
    tiles: &[(u8, u8, i16)],
) -> Vec<String> {
    let mut out = Vec::new();
    for (i, &theirs) in tiles.iter().enumerate() {
        let ours = (slice.kinds[i], slice.restore[i], slice.layers[i]);
        if ours != theirs {
            let (x, y) = (i % width, i / width);
            out.push(format!(
                "terrain tile ({x}, {y}): slice {ours:?} game {theirs:?}"
            ));
        }
    }
    out
}

/// What the forced replay reads of the game's save after an update: every player's object rows,
/// the sight, the map's terrain and the state the forcing takes. Parsing a save takes most of
/// the replay's time, so [`forced_run`] builds these on worker threads ahead of the replay.
struct SavedStep {
    objects: Vec<Vec<ObjectRow>>,
    seen: Visibility,
    width: usize,
    terrain: Vec<(u8, u8, i16)>,
    game: genie_core::turn::GameState,
}

impl SavedStep {
    fn of(save: &[u8], masters: Option<&MasterTable>) -> Result<Self, Error> {
        let chapter = de::chapters(save)?[0];
        let header = chapter.header()?;
        let objects = (0..header.players().len() as u32)
            .map(|id| header_objects(&header, id))
            .collect::<Result<_, _>>()?;
        let (width, terrain) = saved_terrain(&header);
        Ok(SavedStep {
            objects,
            seen: saved_sight(&header),
            width,
            terrain,
            game: game_state(&header, masters),
        })
    }
}

/// `f(0..n)` on up to `threads` worker threads, in order.
fn parse_ahead<T: Send>(n: usize, threads: usize, f: &(dyn Fn(usize) -> T + Sync)) -> Vec<T> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let done: Vec<std::sync::Mutex<Option<T>>> =
        (0..n).map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..threads.min(n) {
            scope.spawn(|| {
                loop {
                    let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if n <= k {
                        break;
                    }
                    let v = f(k);
                    *done[k].lock().expect("a worker panicked") = Some(v);
                }
            });
        }
    });
    done.into_iter()
        .map(|m| {
            m.into_inner()
                .expect("a worker panicked")
                .expect("every index ran")
        })
        .collect()
}

fn spatial(header: &Header, terrain: &TerrainTable) -> genie_core::spatial::Spatial {
    use genie_core::spatial::{Obstruction, Placed, Spatial, Tiles};
    let map = &header.world.base_world.map;
    let classes = map
        .rows
        .iter()
        .flat_map(|r| r.tiles.iter().map(|t| terrain.class(t.terrain_type)))
        .collect();
    let tiles = Tiles::new(map.map_width, map.map_height, classes);
    let mut out = Spatial::with_tiles([map.map_width, map.map_height], tiles);
    out.terrain = Some(map_terrain(header, terrain));
    let mut sight = saved_sight(header);
    for (player, section) in header.players().iter().enumerate() {
        let all = section.live_objects().chain(section.sleeping_objects());
        for o in all {
            let Some(s) = o.static_part() else { continue };
            let Some(m) = placement(s, section.objects()) else {
                continue;
            };
            let building = o.tribe_building_object.as_ref();
            // a building's place links its tiles (FUN_1411e6780), an annex's does not, and a
            // foundation made desolid lost them (FUN_1411cf770); so do a combat unit's that does
            // not move and is more than a tile wide (FUN_14108c320). A building keeps its
            // footprint's UMV entry (+0x904)
            let place = [s.t_x, s.t_y, s.t_z];
            let footprint = [m.radius[0], m.radius[1]];
            let speed = moving_speed(o, s, section.objects());
            let links = match building {
                Some(b) => {
                    b.mupt_linked_object_references_linked_owner_id == -1 && b.desolid_flag == 0
                }
                None => {
                    o.combat().is_some() && speed == 0.0 && (0.5 < m.radius[0] || 0.5 < m.radius[1])
                }
            };
            let umv = match (links, building.is_some()) {
                (true, true) => sight.link_building(place, footprint),
                (true, false) => {
                    sight.link_building(place, footprint);
                    0
                }
                (false, true) => sight.building_entry(place, footprint),
                (false, false) => 0,
            };
            sight.add_seer(
                s.id,
                Seer {
                    player,
                    master: s.master_obj,
                    los: m.los,
                    state: s.object_state,
                    // a foundation made in the game explores first when its construction starts
                    explored: building.is_none_or(|b| b.built != 0 || b.build_pts != 0.0),
                    building: building.map(|b| BuildingSight {
                        built: b.built != 0,
                        started: b.build_pts != 0.0,
                    }),
                    umv: umv as u8,
                    capture: capture_kind(player, m.class, s.master_obj),
                    // the object's type: the save's factory types 70 and 80, any other below
                    kind: if o.tribe_combat_object.is_some() {
                        70
                    } else if building.is_some() {
                        80
                    } else {
                        0
                    },
                },
            );
            let desolid = o
                .tribe_building_object
                .as_ref()
                .is_some_and(|b| b.desolid_flag != 0);
            let solid = m.radius.iter().all(|&r| 0.0 < r) && !desolid;
            let obstruction = (m.obstruction_type != 0 && solid).then_some(Obstruction {
                kind: m.obstruction_type,
                class: m.obstruction_class,
            });
            out.insert(
                s.id,
                Placed {
                    position: [s.t_x, s.t_y, s.t_z],
                    radius: [m.radius[0], m.radius[1]],
                    class: m.class,
                    player,
                    master: s.master_obj,
                    obstruction_type: m.obstruction_type,
                    buildable_on: m.buildable_on,
                    tall: 0.0 < m.radius[2],
                    speed: moving_speed(o, s, section.objects()),
                    unknown: None,
                    obstruction,
                },
            );
        }
        for ghost in section.objects().doppleganger_objects.objects() {
            if let Some(s) = ghost.static_part() {
                out.insert_ghost([s.t_x, s.t_y, s.t_z]);
            }
        }
    }
    out.sight = Some(sight);
    out
}

/// What the players see in a save: each player's visible map, the map's unified map and UMV list
/// and which players are computers; no tile links and no objects' sight.
fn saved_sight(header: &Header) -> Visibility {
    let map = &header.world.base_world.map;
    let n = (map.map_width.max(0) * map.map_height.max(0)) as usize;
    let players = header.players();
    let maps = players
        .iter()
        .map(|p| {
            let vm = &p.objects().rge_visible_map;
            VisibleMap {
                counts: vm.visible_map.clone().unwrap_or_else(|| vec![-1; n]),
                explored: vm.number_tiles_explored_value,
            }
        })
        .collect();
    let umv = &map.umv_data_list;
    let extents = umv
        .umv_data_guid
        .iter()
        .zip(&umv.umv_data_list)
        .map(|(&guid, e)| Extent {
            guid,
            offsets: e
                .p
                .iter()
                .map(|&v| [v as u32 as i32, (v >> 32) as u32 as i32])
                .collect(),
            capacity: e.n + 1,
        })
        .collect();
    Visibility::new(
        map.map_width,
        map.map_height,
        maps,
        map.unified_vis_map
            .unified_visible_map
            .clone()
            .unwrap_or_else(|| vec![0; n]),
        extents,
        umv.umv_data_list_size,
        players
            .iter()
            .map(|p| p.world_player().type_ == 3)
            .collect(),
    )
}

/// The players' visible maps and the unified map of a save, as [`Visibility`] holds them.
pub fn saved_visibility(save: &[u8]) -> Result<Visibility, Error> {
    let chapter = de::chapters(save)?[0];
    let header = chapter.header()?;
    Ok(saved_sight(&header))
}

/// Where player `player`'s sight in `slice` differs from `game`'s: its explored count, each tile's
/// count and its two bits of the unified map.
pub fn visibility_differences(slice: &Visibility, game: &Visibility, player: usize) -> Vec<String> {
    let mut out = Vec::new();
    let (Some(a), Some(b)) = (slice.maps.get(player), game.maps.get(player)) else {
        return vec![format!("player {player} has no visible map")];
    };
    if a.explored != b.explored {
        out.push(format!(
            "player {player} explored {} tiles, the game {}",
            a.explored, b.explored
        ));
    }
    let w = slice.width.max(1) as usize;
    let bits = 1u64 << player | 1u64 << (32 + player);
    for (i, (&c, &g)) in a.counts.iter().zip(&b.counts).enumerate() {
        let (x, y) = (i % w, i / w);
        if c != g {
            out.push(format!(
                "player {player} tile ({x}, {y}) count {c}, the game {g}"
            ));
        }
        let (u, v) = (slice.unified[i] & bits, game.unified[i] & bits);
        if u != v {
            out.push(format!(
                "player {player} tile ({x}, {y}) unified bits {u:#x}, the game {v:#x}"
            ));
        }
    }
    out
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
/// QUEUE, SET GATHER POINT and the orders run; every other command is left out (see [`GROUP_COMMANDS`] for what
/// that may miss).
///
/// The player byte is the comm player, which the handlers map to a world player through the lobby
/// (`FUN_140e6b210`); the slice takes the two as equal.
#[derive(Default)]
struct Decoder {
    last_commanded: std::collections::HashMap<u8, Vec<i32>>,
}

impl Decoder {
    /// The commands of the frames a save already played: decoded for the unit lists they leave
    /// (a later command's 'last commanded units'), not run.
    fn prime(&mut self, frames: &[Frame<'_>], sections: &[PlayerSection<'_>]) {
        for frame in frames {
            for p in &frame.commands {
                self.command(p, sections);
            }
        }
    }

    fn command(&mut self, packet: &[u8], sections: &[PlayerSection<'_>]) -> Option<Command> {
        use genie_rec::de::command::Command as Packet;
        // INTERACT, MOVE and RETREAT reset their units' idle timeouts (`FUN_140e6cfc0`), WORK not
        let (player, num, ids, order, interact) = match Packet::decode(packet) {
            Packet::Build(build) => return Some(build_command(&build, sections)),
            Packet::MultiQueue(queue) => return Some(queue_command(&queue, sections)),
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
                let order = genie_core::order::Interact {
                    target: c.t_id,
                    point: [c.x, c.y],
                    carry_test: c.field_0x12 != 0,
                    queued: c.field_0x13 != 0,
                    direct: c.field_0x14 != 0,
                    human: c.field_0x15 == 1,
                    filter: c.field_0x16 as i16,
                };
                let order = genie_core::turn::Order::Interact(order);
                (c.commp_id, c.num, c.ids, true, Some(order))
            }
            Packet::Work(c) => (c.commp_id, c.num, c.ids, false, None),
            Packet::Move(c) => {
                let order = genie_core::order::MoveOrder {
                    target: c.t_id,
                    point: [c.x, c.y],
                    queued: c.field_0x13 != 0,
                    direct: c.field_0x14 != 0,
                    human: c.field_0x15 == 1,
                };
                let order = genie_core::turn::Order::Move(order);
                (c.commp_id, c.num, c.ids, true, Some(order))
            }
            Packet::Retreat(c) => (c.commp_id, c.num, c.ids, true, None),
            Packet::Gate(c) => {
                return Some(Command::Gate {
                    player: c.commp_id as usize,
                    id: c.u_id,
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
            // GAME's toggles of the automatic farm and fish trap queues; the player is at +8
            Packet::Game(g) if matches!(g.field_0x04, 0x10 | 0x13) => {
                return Some(Command::AutoQueue {
                    player: usize::try_from(g.field_0x08).ok()?,
                    fishtrap: g.field_0x04 == 0x13,
                });
            }
            other => {
                // a command the decoder does not turn into the slice's stops its player (byte 1
                // is the issuing comm player)
                let player = *packet.get(1)? as usize;
                return Some(Command::Unmodelled {
                    player,
                    why: Unsupported(unmodelled_command(&other)),
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
            self.last_commanded.insert(player, units.clone());
            units
        };
        order.then_some(Command::Order {
            player: player as usize,
            units,
            order: interact,
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
    let train_time = |c: &genie_save::generated::world::TribeMasterCombatObject| {
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

fn combat_master(m: &Master) -> Option<&genie_save::generated::world::RgeMasterCombatObject> {
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
    // GENIE_TRACE_COMMANDS lists each decoded command with the update it runs in
    let trace = std::env::var_os("GENIE_TRACE_COMMANDS").is_some();
    for frame in frames {
        let commands: Vec<Command> = frame
            .commands
            .iter()
            .filter_map(|p| decoder.command(p, sections))
            .collect();
        if trace {
            for c in &commands {
                eprintln!("update {}: {c:?}", turns.ticks + 1);
            }
        }
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
    let header = chapter.header()?;
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
    state.sleeping_count = simulated.sleeping.len() as u32;
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
            top_action: top_action(unit).or(unit.production.as_ref().map(|_| TopAction {
                kind: -1,
                state: genie_core::production::Production::MAKING,
            })),
        })
        .collect();
    player_block(&state)
}

/// A unit's top action as the slice models it, type and state.
fn top_action(unit: &Unit) -> Option<TopAction> {
    unit.top_action()
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
    let header = chapter.header()?;
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
    let header = chapter.header()?;
    let sections = header.players();
    if let Some(&id) = ids.iter().find(|&&id| sections.len() <= id as usize) {
        return Err(Error::NoSuchPlayer(id));
    }
    let mut turns = slice(&header, masters);
    // the header's load makes the registry empty, with every object's mask 0
    if let Some(sight) = turns.spatial.sight.as_mut() {
        sight.registry = Some(genie_core::registry::Registry::new(
            sections.len(),
            genie_core::registry::CLASSES,
        ));
    }
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

/// What the comparison with the game's saves checks of one live object: its master, state, hit
/// points, held amount and position in full f32, its top action (type, state) and its unit AI's
/// retarget timer.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectRow {
    pub id: i32,
    pub master: i16,
    pub state: u8,
    pub hp: f32,
    pub amount: f32,
    pub position: [f32; 3],
    pub action: Option<(i16, u8)>,
    pub retarget: Option<i32>,
    /// A moving object's orientation, velocity data and path data (the level it walks first).
    pub movement: Option<MoveRow>,
    /// A building's construction: desolid, progress, builders.
    pub construction: Option<(bool, f32, u8)>,
    /// A Build action's `added_to_num_builders`.
    pub build_added: Option<bool>,
    /// The unit AI's order, action and target with their last values, and its timers.
    pub ai: Option<AiRow>,
    /// `+0x650`, the last human order time.
    pub human_order: u32,
    /// A building's gather point: x, y and its target.
    pub gather: Option<([f32; 2], i32)>,
    /// A combat object's charge, `mHitSomethingValidForChargeAttack` and hit point powerups,
    /// which its hits and the hits on it change.
    pub combat: Option<(f32, bool, [f32; 2])>,
    /// A building's gate lock.
    pub gate: Option<genie_core::turn::Gate>,
}

/// What an object's row compares of its unit AI (named as in the save).
#[derive(Clone, Debug, PartialEq)]
pub struct AiRow {
    pub fields: Vec<(&'static str, String)>,
}

impl AiRow {
    fn of(ai: &genie_core::unit_ai::UnitAi) -> Self {
        let f = |v: &dyn std::fmt::Debug| format!("{v:?}");
        AiRow {
            fields: vec![
                ("current_order", f(&ai.current_order)),
                ("current_order_priority", f(&ai.order_priority)),
                ("current_action", f(&ai.current_action)),
                ("current_target", f(&ai.current_target)),
                ("current_target_type", f(&ai.target_type)),
                ("current_target_point", f(&ai.target_point)),
                ("last_order", f(&ai.last_order)),
                ("last_action", f(&ai.last_action)),
                ("last_target", f(&ai.last_target)),
                ("last_target_type", f(&ai.last_target_type)),
                ("idle_timer", f(&ai.idle_timer)),
                ("adjusted_idle_timeout", f(&ai.adjusted_idle_timeout)),
                ("secondary_timer", f(&ai.secondary_timer)),
                ("look_around_timer", f(&ai.look_around_timer)),
                ("look_around_timeout", f(&ai.look_around_timeout)),
                ("last_retarget_time", f(&ai.last_retarget_time)),
                ("state_position", f(&ai.state_position)),
                ("last_update_time", f(&ai.last_update_time)),
                ("orders", f(&ai.orders)),
                ("notifies", f(&ai.notifies)),
            ],
        }
    }

    fn saved(ai: &genie_save::generated::world::UnitAiModule) -> Self {
        let f = |v: &dyn std::fmt::Debug| format!("{v:?}");
        AiRow {
            fields: vec![
                ("current_order", f(&ai.current_order_value)),
                (
                    "current_order_priority",
                    f(&ai.current_order_priority_value),
                ),
                ("current_action", f(&ai.current_action_value)),
                ("current_target", f(&ai.current_target_value)),
                ("current_target_type", f(&ai.current_target_type_value)),
                (
                    "current_target_point",
                    f(&[
                        ai.current_target_xvalue,
                        ai.current_target_yvalue,
                        ai.current_target_zvalue,
                    ]),
                ),
                ("last_order", f(&ai.last_order_value)),
                ("last_action", f(&ai.last_action_value)),
                ("last_target", f(&ai.last_target_value)),
                ("last_target_type", f(&ai.last_target_type_value)),
                ("idle_timer", f(&ai.idle_timer_value)),
                ("adjusted_idle_timeout", f(&ai.adjusted_idle_timeout_value)),
                ("secondary_timer", f(&ai.secondary_timer_value)),
                ("look_around_timer", f(&ai.look_around_timer_value)),
                ("look_around_timeout", f(&ai.look_around_timeout_value)),
                ("last_retarget_time", f(&ai.last_retarget_time)),
                (
                    "state_position",
                    f(&[ai.state_position_xvalue, ai.state_position_yvalue]),
                ),
                ("last_update_time", f(&ai.last_update_time_value)),
                (
                    "orders",
                    f(&ai
                        .temp_order
                        .iter()
                        .map(genie_core::unit_ai::QueuedOrder::from_bytes)
                        .collect::<Vec<_>>()),
                ),
                (
                    "notifies",
                    f(&ai
                        .temp_notify
                        .iter()
                        .map(genie_core::unit_ai::Notify::from_bytes)
                        .collect::<Vec<_>>()),
                ),
            ],
        }
    }
}

/// What a moving object's row compares of its movement.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveRow {
    pub forward: genie_core::fmath::Vec3,
    pub right: genie_core::fmath::Vec3,
    /// Velocity and acceleration, when the object has velocity data.
    pub velocity: Option<(genie_core::fmath::Vec3, genie_core::fmath::Vec3)>,
    pub paths: Vec<genie_core::movement::PathData>,
    /// `+0x7f9`, `+0x7fa`, `+0x7fb`, `+0x870`, `+0x874`.
    pub waiting: u8,
    pub wait_delays: u8,
    pub attack_moving: bool,
    pub last_move: u32,
    pub substitute_count: i32,
    /// The unit's ignore list (`+0x670`).
    pub ignore_units: Vec<i32>,
}

impl ObjectRow {
    fn of_unit(unit: &Unit) -> Self {
        ObjectRow {
            id: unit.id,
            master: unit.life.master_id,
            state: unit.life.state,
            hp: unit.life.hp,
            amount: unit.life.amount,
            position: unit.position,
            action: top_action(unit).map(|a| (a.kind, a.state)),
            retarget: unit.ai.as_ref().map(|ai| ai.retarget_timer as i32),
            movement: unit.mover.as_ref().map(|m| MoveRow {
                forward: m.forward,
                right: m.right,
                velocity: m.velocity.as_ref().map(|v| (v.velocity, v.acceleration)),
                paths: m.paths.clone(),
                waiting: m.waiting,
                wait_delays: m.wait_delays,
                attack_moving: m.attack_moving,
                last_move: m.last_move,
                substitute_count: m.substitute_count,
                ignore_units: m.ignore_units.clone(),
            }),
            construction: unit
                .construction
                .map(|c| (c.desolid, c.progress, c.builders)),
            build_added: unit.build.map(|b| b.added),
            ai: unit.ai.as_ref().map(AiRow::of),
            human_order: unit.last_human_order,
            gather: unit
                .gather_point
                .map(|g| ([g.point[0], g.point[1]], g.target)),
            combat: unit
                .combat
                .then_some((unit.charge, unit.charge_hit, unit.hp_powerups)),
            gate: unit.gate,
        }
    }
}

/// A saved moving object's [`MoveRow`]; a level whose path the world's table lacks is left out.
fn saved_movement(object: &genie_save::Object, paths: &[Path]) -> Option<MoveRow> {
    let m = object.moving_part()?;
    let s = object.static_part()?;
    Some(MoveRow {
        forward: vec3(&m.orientation_forward_value),
        right: vec3(&m.orientation_right_value),
        velocity: (m.have_movement == 1).then(|| {
            (
                m.velocity.as_ref().map_or([0.0; 3], vec3),
                m.acceleration.as_ref().map_or([0.0; 3], vec3),
            )
        }),
        paths: m
            .temp_path_data
            .iter()
            .rev()
            .filter_map(|d| path_level(d, paths).ok())
            .collect(),
        waiting: m.waiting_to_move_value,
        wait_delays: m.number_wait_delays,
        attack_moving: m.attack_moving != 0,
        last_move: m.last_move_time_value,
        substitute_count: m.consecutive_substitute_count,
        ignore_units: s.temp_int.clone(),
    })
}

/// A slice run from a save ([`objects_from_save`]): one player's objects after it, and the first
/// update the slice could not model, if any.
#[derive(Clone, Debug)]
pub struct SliceRun {
    pub objects: Vec<ObjectRow>,
    pub unsupported: Option<(u32, usize, Unsupported)>,
    /// The id of the object whose update was not modelled.
    pub unsupported_id: Option<i32>,
    /// Every player's first unsupported update, by player.
    pub stopped: Vec<Option<(u32, usize, Unsupported)>>,
    /// What the players see after the run.
    pub visibility: Option<Visibility>,
    /// The map's terrain after the run.
    pub terrain: Option<genie_core::terrain::Terrain>,
    /// The objects the slice stopped updating, of every player.
    pub frozen: Vec<i32>,
    /// Every object's freeze, by player: the tick, the object and why.
    pub freezes: Vec<Vec<(u32, i32, Unsupported)>>,
}

/// The live objects of player `id` in a save (a recording's header, or the tick oracle's
/// `--snapshot`), as [`ObjectRow`]s.
pub fn saved_objects(save: &[u8], id: u32) -> Result<Vec<ObjectRow>, Error> {
    let chapter = de::chapters(save)?[0];
    let header = chapter.header()?;
    header_objects(&header, id)
}

/// [`saved_objects`] of a read header.
fn header_objects(header: &Header, id: u32) -> Result<Vec<ObjectRow>, Error> {
    let sections = header.players();
    let section = sections.get(id as usize).ok_or(Error::NoSuchPlayer(id))?;
    Ok(live(section)
        .map(|(object, s)| ObjectRow {
            id: s.id,
            master: s.master_obj,
            state: s.object_state,
            hp: s.hp,
            amount: s.attribute_amount_held,
            position: [s.t_x, s.t_y, s.t_z],
            action: object
                .actions()
                .and_then(|a| a.entries().next())
                .map(|(kind, action)| (kind, action.base().map_or(0, |b| b.state))),
            retarget: object
                .combat()
                .and_then(|c| c.unit_ai())
                .map(|ai| ai.randomized_retarget_timer as i32),
            movement: saved_movement(object, &header.world.base_world.path),
            construction: object.tribe_building_object.as_ref().map(|b| {
                (
                    b.desolid_flag != 0,
                    b.build_pts,
                    b.tribe_combat_object.num_builders,
                )
            }),
            build_added: object
                .actions()
                .and_then(|a| a.entries().next())
                .filter(|&(kind, _)| kind == genie_core::build::BUILD)
                .and_then(|(_, a)| a.tribe_action_build.as_ref())
                .map(|b| b.added_to_num_builders != 0),
            ai: object.combat().and_then(|c| c.unit_ai()).map(AiRow::saved),
            human_order: s.last_human_order_time,
            gather: gather_point(object).map(|g| ([g.point[0], g.point[1]], g.target)),
            combat: object.combat().map(|c| {
                (
                    c.charge,
                    c.rge_combat_object.hit_something_valid_for_charge_attack != 0,
                    [c.hitpoints_powerup, c.hitpoints_max_powerup],
                )
            }),
            gate: saved_gate(object),
        })
        .collect())
}

/// What the tick oracle dumps of the game at a save's tick that the save does not hold.
#[derive(Clone, Copy, Debug, Default)]
pub struct Dumps<'a> {
    /// The shape mesh with each object's shape (`load_oracle/probes/mesh.py`); without it a path
    /// request stops the slice where it needs the short pather.
    pub mesh: Option<&'a str>,
    /// The objects-in-sight registry and the objects' masks (`load_oracle/probes/registry.py`);
    /// without it the slice keeps no registry, and a unit AI's search stops it.
    pub registry: Option<&'a str>,
}

/// The objects-in-sight registry as the tick oracle dumps it (`load_oracle/probes/registry.py`):
/// a `registry PLAYERS CLASSES` line, a `list PLAYER CLASS COUNT CAPACITY` line before each list's
/// `rec ID DIST OWNER X Y RX RY` lines (the floats as hex bits), then `mask ID HEX SLOTS...` per
/// combat object (its record's index in each player's list of its class, -1 for none).
pub fn registry_from_dump(text: &str) -> Result<genie_core::registry::Registry, Error> {
    use genie_core::registry::{Entry, Record, Registry};
    let bad = || Error::BadRegistryDump;
    let mut lines = text.lines();
    let head: Vec<&str> = lines.next().ok_or_else(bad)?.split_whitespace().collect();
    let [_, players, classes] = head[..] else {
        return Err(bad());
    };
    let number = |w: &str| w.parse::<i64>().map_err(|_| bad());
    let bits = |w: &str| {
        u32::from_str_radix(w, 16)
            .map(f32::from_bits)
            .map_err(|_| bad())
    };
    let mut out = Registry::new(number(players)? as usize, number(classes)? as usize);
    let mut list: Option<(usize, usize)> = None;
    for line in lines {
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.first().copied() {
            Some("list") if w.len() == 5 => {
                let (p, c) = (number(w[1])? as usize, number(w[2])? as usize);
                if out.players <= p || out.classes <= c {
                    return Err(bad());
                }
                list = Some((p, c));
            }
            Some("rec") if w.len() == 8 => {
                let (p, c) = list.ok_or_else(bad)?;
                out.lists[p][c].push(Record {
                    id: number(w[1])? as i32,
                    distance: bits(w[2])?,
                    owner: number(w[3])? as u8,
                    x: bits(w[4])?,
                    y: bits(w[5])?,
                    rx: bits(w[6])?,
                    ry: bits(w[7])?,
                });
            }
            Some("mask") if w.len() == 3 + genie_core::registry::SLOTS => {
                let mut entry = Entry {
                    mask: u64::from_str_radix(w[2], 16).map_err(|_| bad())?,
                    ..Entry::default()
                };
                for (k, slot) in entry.slots.iter_mut().enumerate() {
                    let v = number(w[3 + k])?;
                    *slot = usize::try_from(v).ok();
                }
                out.entries.insert(number(w[1])? as i32, entry);
            }
            _ => return Err(bad()),
        }
    }
    Ok(out)
}

/// The slice started from the tick oracle's save at tick `start_tick` (`--snapshot`), run through
/// the `recording`'s next `ticks` ticks: player `id`'s objects after it. The save gives the state,
/// the recording the ticks' lengths and commands, `dumps` what the save does not hold. The tick
/// oracle counts ticks from 0, as its trace does: its tick N is the game's update N + 1, and the
/// save at tick N holds the state after it.
pub fn objects_from_save(
    save: &[u8],
    recording: &[u8],
    start_tick: usize,
    ticks: usize,
    id: u32,
    masters: Option<&MasterTable>,
    dumps: Dumps<'_>,
) -> Result<SliceRun, Error> {
    let mut turns = run_from_save(save, recording, start_tick, ticks, masters, dumps)?;
    if turns.players.len() <= id as usize {
        return Err(Error::NoSuchPlayer(id));
    }
    let player = &turns.players[id as usize];
    Ok(SliceRun {
        objects: player
            .objects
            .iter()
            .filter(|u| u.id >= 0)
            .map(ObjectRow::of_unit)
            .collect(),
        unsupported: player.unsupported,
        unsupported_id: player
            .unsupported
            .and_then(|(_, i, _)| player.objects.get(i).map(|u| u.id)),
        stopped: turns.players.iter().map(|p| p.unsupported).collect(),
        visibility: turns.spatial.sight.take(),
        terrain: turns.spatial.terrain.take(),
        frozen: turns
            .players
            .iter()
            .flat_map(|p| p.objects.iter().filter(|u| p.stopped || u.stopped))
            .map(|u| u.id)
            .collect(),
        freezes: turns.players.iter().map(|p| p.freezes.clone()).collect(),
    })
}

/// What one forced replay step found ([`forced_ticks`]): the slice started from the game's save
/// after `start`, run `ticks` updates, against the game's save after `start + ticks`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TickReport {
    pub start: usize,
    pub ticks: usize,
    /// What differs, by player: the objects that ran modelled (an object that froze, or every
    /// object of a player that stopped, is left out: its reason is in `unsupported`) and the
    /// player's sight.
    pub differences: Vec<(u32, String)>,
    /// What the slice could not model: the player, the object (None for the player's stop), the
    /// update and why.
    pub unsupported: Vec<(u32, Option<i32>, u32, Unsupported)>,
}

/// One step of the forced replay (FD-008, CONTINUE.md): genie-core from the game's own save after
/// update `start` (`save`, with that save's mesh and registry dumps), `ticks` updates, compared
/// with the game's save after `start + ticks` (`after`) for every player. Every step starts from
/// the game's state, so the frame seed and every object, a frozen one too, are the game's again:
/// nothing the slice gets wrong carries into the next step. Dev only: the saves and dumps come
/// from the tick oracle, which genie-core never reads.
pub fn forced_ticks(
    save: &[u8],
    after: &[u8],
    recording: &[u8],
    start: usize,
    ticks: usize,
    masters: Option<&MasterTable>,
    dumps: Dumps<'_>,
) -> Result<TickReport, Error> {
    let turns = run_from_save(save, recording, start, ticks, masters, dumps)?;
    let mut report = TickReport {
        start,
        ticks,
        ..TickReport::default()
    };
    compare_players(&turns, &SavedStep::of(after, masters)?, None, &mut report)?;
    Ok(report)
}

/// The forced replay's continuous mode (FD-008, CONTINUE.md): genie-core from the game's save
/// after `start` (with its dumps), then `ticks` updates, each compared with the game's save after
/// it (`save_after(update)`, which may end the run early with None) and then forced to the game's
/// state where the slice did not model it or got it wrong ([`genie_core::turn::Turns::force`]: the
/// frame seed, the frozen and the differing objects, a stopped player, the sight). The objects the
/// slice gets right keep their own state, the saved and the unsaved, from tick to tick. One
/// report per update, each with that update's own differences. Dev only.
pub fn forced_run(
    start_save: &[u8],
    recording: &[u8],
    start: usize,
    ticks: usize,
    masters: Option<&MasterTable>,
    dumps: Dumps<'_>,
    save_after: &(dyn Fn(usize) -> Option<Vec<u8>> + Sync),
) -> Result<Vec<TickReport>, Error> {
    let chapter = de::chapters(start_save)?[0];
    let header = chapter.header()?;
    let sections = header.players();
    let mut turns = slice(&header, masters);
    if let Some(text) = dumps.mesh {
        let m = mesh_from_dump(text).ok_or(Error::BadMeshDump)?;
        turns.pathing.set_mesh(m, mesh_owners_from_dump(text));
    }
    if let (Some(text), Some(sight)) = (dumps.registry, turns.spatial.sight.as_mut()) {
        sight.registry = Some(registry_from_dump(text)?);
    }
    let played = de::chapters(recording)?[0];
    let start_header = played.header()?;
    hunted_origins(&mut turns, &start_header);
    let updates = start + 1;
    turns.ticks = updates as u32;
    let all = frames(&played, updates + ticks)?;
    let mut decoder = Decoder::default();
    decoder.prime(&all[..updates.min(all.len())], &sections);
    let mut reports = Vec::new();
    // the game's saves after each update, parsed ahead on every core a batch at a time
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut ahead = std::collections::VecDeque::new();
    for k in 0..ticks {
        let Some(frame) = all.get(updates + k) else {
            break;
        };
        if ahead.is_empty() {
            let n = (2 * threads).min(ticks - k);
            let parse =
                |i: usize| save_after(start + k + i + 1).map(|save| SavedStep::of(&save, masters));
            ahead.extend(parse_ahead(n, threads, &parse));
        }
        let Some(after) = ahead.pop_front().flatten() else {
            break;
        };
        let after = after?;
        run(
            &mut turns,
            &mut decoder,
            std::slice::from_ref(frame),
            &sections,
        );
        let mut report = TickReport {
            start: start + k,
            ticks: 1,
            ..TickReport::default()
        };
        let tick = turns.ticks;
        compare_players(&turns, &after, Some(tick), &mut report)?;
        let mut game = after.game;
        // what the slice got wrong takes the game's state too: each update's differences are
        // its own, not an earlier update's carried on
        game.resync = report
            .differences
            .iter()
            .filter_map(|(_, d)| d.split(':').next().and_then(|o| o.parse::<i32>().ok()))
            .collect();
        if report
            .differences
            .iter()
            .any(|(_, d)| d.starts_with("player "))
        {
            game.sight = Some((after.seen.maps, after.seen.unified));
        }
        for (p, id, why) in turns.force(&game) {
            report.unsupported.push((p as u32, Some(id), tick, why));
        }
        reports.push(report);
    }
    Ok(reports)
}

/// The game's state in a save as the slice's units ([`genie_core::turn::GameState`]).
fn game_state(header: &Header, masters: Option<&MasterTable>) -> genie_core::turn::GameState {
    let world = &header.world.base_world;
    let mut game = genie_core::turn::GameState {
        frame_seed: world.current_frame_random_seed,
        ..Default::default()
    };
    for p in header.players() {
        let civ = p.world_player().base.civ_id;
        game.players.push(
            p.live_objects()
                .map(|o| unit(o, p.objects(), &world.path, civ, masters))
                .collect(),
        );
        let attributes = &p.world_player().base.attributes;
        let r = |i: usize| attributes.get(i).copied().unwrap_or_default();
        game.resources.push([r(0), r(1), r(2), r(3)]);
    }
    game
}

/// Every player's objects and sight after a run against the game's save `after`; with `tick`,
/// only the freezes of that update count, else all. A frozen object's differences are left out.
fn compare_players(
    turns: &Turns,
    after: &SavedStep,
    tick: Option<u32>,
    report: &mut TickReport,
) -> Result<(), Error> {
    let seen = &after.seen;
    // the map's terrain is the world's: its differences go with Gaia's
    if let Some(t) = turns.spatial.terrain.as_ref() {
        for d in tile_differences(t, after.width, &after.terrain) {
            report.differences.push((0, d));
        }
    }
    for (p, player) in turns.players.iter().enumerate() {
        let id = p as u32;
        let now = |t: u32| tick.is_none_or(|n| n == t);
        if let Some((t, _, why)) = player.unsupported.filter(|u| player.stopped && now(u.0)) {
            report.unsupported.push((id, None, t, why));
        }
        let frozen: Vec<i32> = player
            .objects
            .iter()
            .filter(|u| u.stopped)
            .map(|u| u.id)
            .chain(player.freezes.iter().map(|&(_, o, _)| o))
            .collect();
        for &(t, o, why) in player.freezes.iter().filter(|f| now(f.0)) {
            report.unsupported.push((id, Some(o), t, why));
        }
        let rows: Vec<ObjectRow> = player
            .objects
            .iter()
            .filter(|u| u.id >= 0)
            .map(ObjectRow::of_unit)
            .collect();
        let game = after.objects.get(p).ok_or(Error::NoSuchPlayer(id))?;
        if !player.stopped {
            for d in object_differences(&rows, game) {
                let object = d.split(':').next().and_then(|o| o.parse::<i32>().ok());
                if object.is_some_and(|o| frozen.contains(&o)) {
                    continue;
                }
                report.differences.push((id, d));
            }
        }
        if let Some(sight) = turns.spatial.sight.as_ref() {
            for d in visibility_differences(sight, seen, p) {
                report.differences.push((id, d));
            }
        }
    }
    Ok(())
}

/// The slice from a save: [`objects_from_save`]'s run, every player's state after it.
fn run_from_save(
    save: &[u8],
    recording: &[u8],
    start_tick: usize,
    ticks: usize,
    masters: Option<&MasterTable>,
    dumps: Dumps<'_>,
) -> Result<Turns, Error> {
    let chapter = de::chapters(save)?[0];
    let header = chapter.header()?;
    let sections = header.players();
    let mut turns = slice(&header, masters);
    if let Some(text) = dumps.mesh {
        let m = mesh_from_dump(text).ok_or(Error::BadMeshDump)?;
        turns.pathing.set_mesh(m, mesh_owners_from_dump(text));
    }
    if let (Some(text), Some(sight)) = (dumps.registry, turns.spatial.sight.as_mut()) {
        sight.registry = Some(registry_from_dump(text)?);
    }
    let played = de::chapters(recording)?[0];
    hunted_origins(&mut turns, &played.header()?);
    let updates = start_tick + 1;
    turns.ticks = updates as u32;
    let all = frames(&played, updates + ticks)?;
    let next = all.get(updates..).unwrap_or_default();
    let mut decoder = Decoder::default();
    decoder.prime(&all[..updates.min(all.len())], &sections);
    run(&mut turns, &mut decoder, next, &sections);
    Ok(turns)
}

/// The hunted animals' unit AI fields the save does not hold (`TribeHuntedAnimalUnitAIModule`,
/// made with the unit: `FUN_140c11920`): the origin is the animal's place at the recording's
/// start (`start`, the recording's header save, whose load made the AI), the leash its master's
/// last task of action type 6's work range (the unit's `+0xb74`, `FUN_141206d50`).
fn hunted_origins(turns: &mut Turns, start: &Header) {
    let places: std::collections::BTreeMap<i32, [f32; 3]> = start
        .players()
        .iter()
        .flat_map(|p| {
            p.live_objects()
                .chain(p.sleeping_objects())
                .collect::<Vec<_>>()
        })
        .filter_map(|o| o.static_part().map(|s| (s.id, [s.t_x, s.t_y, s.t_z])))
        .collect();
    for (p, player) in turns.players.iter_mut().enumerate() {
        let masters = turns.masters.get(p);
        for unit in player.objects.iter_mut().chain(player.sleeping.iter_mut()) {
            let Some(ai) = unit.ai.as_mut().filter(|ai| ai.ai_type == 2) else {
                continue;
            };
            let Some(&origin) = places.get(&unit.id) else {
                continue;
            };
            let leash = masters
                .and_then(|m| m.get(unit.life.master_id))
                .and_then(|m| m.tasks.iter().rev().find(|t| t.action_type == 6))
                .map_or(0.0, |t| t.work_range);
            ai.hunted = Some(genie_core::unit_ai::Hunted { origin, leash });
        }
    }
}

/// The world's objects as movement sees them, from a save ([`spatial`]), on a map without terrain.
pub fn spatial_from_save(save: &[u8]) -> Result<genie_core::spatial::Spatial, Error> {
    spatial_with_tiles(save, &TerrainTable::default())
}

/// The world's objects as movement sees them, from a save, with each tile's terrain class.
pub fn spatial_with_tiles(
    save: &[u8],
    terrain: &TerrainTable,
) -> Result<genie_core::spatial::Spatial, Error> {
    let chapter = de::chapters(save)?[0];
    let header = chapter.header()?;
    Ok(spatial(&header, terrain))
}

/// The obstruction manager's shape mesh as the tick oracle dumps it (`load_oracle/probes/mesh.py`):
/// a header line `mesh width W shapes N free F`, a `shape ID X0 Y0 X1 Y1` line per slot with its
/// sides' contacts (`  side S` and seven hex words), then `tile X Y IDS...` for every tile that
/// holds shapes. The save does not hold the mesh: its ids and orders follow the game's history.
pub fn mesh_from_dump(text: &str) -> Option<genie_core::mesh::Mesh> {
    use genie_core::mesh::{Contact, Mesh};
    let mut mesh: Option<Mesh> = None;
    for line in text.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.first().copied() {
            Some("mesh") => {
                let width = words.get(2)?.parse().ok()?;
                let free = u32::from_str_radix(words.get(6)?.trim_start_matches("0x"), 16).ok()?;
                let mut m = Mesh::new(width);
                m.free = free;
                mesh = Some(m);
            }
            Some("shape") => {
                let m = mesh.as_mut()?;
                let v: Vec<i32> = words[2..6].iter().map(|w| w.parse().unwrap()).collect();
                m.shapes.push([v[0], v[1], v[2], v[3]]);
                m.sides.push(Default::default());
            }
            Some("side") => {
                let m = mesh.as_mut()?;
                let side: usize = words.get(1)?.parse().ok()?;
                let w: Vec<u32> = words[2..9]
                    .iter()
                    .map(|w| u32::from_str_radix(w, 16).unwrap())
                    .collect();
                m.sides.last_mut()?[side].push(Contact {
                    other: w[0],
                    side: w[1],
                    at: [w[2] as i32, w[3] as i32],
                    along: w[4] as i32,
                    flag: w[5] as u8,
                    half: w[6] as i32,
                });
            }
            Some("tile") => {
                let m = mesh.as_mut()?;
                let x: u32 = words.get(1)?.parse().ok()?;
                let y: u32 = words.get(2)?.parse().ok()?;
                m.tiles[(y * m.width + x) as usize] =
                    words[3..].iter().map(|w| w.parse().unwrap()).collect();
            }
            _ => {}
        }
    }
    mesh
}

/// What the task finder (`genie_core::tasks`) reads of a save: every player's masters as the work
/// command reads them, every object's state, holding and flags, the players' relations and their
/// attribute 95.
pub struct FinderWorld {
    pub masters: Vec<genie_core::masters::Masters>,
    pub objects: std::collections::HashMap<i32, FinderObject>,
    /// `relation[i]` of each player (`player+0x260`).
    pub relations: Vec<Vec<u8>>,
    pub attribute_95: Vec<f32>,
    pub civs: Vec<u8>,
    pub drop_sites: Vec<genie_core::tasks::DropSite>,
}

/// One object as the task finder reads it.
#[derive(Clone, Copy, Debug)]
pub struct FinderObject {
    pub player: usize,
    pub master: i16,
    pub state: u8,
    pub held: (i16, f32),
    pub hp: f32,
    pub max_hp: f32,
    pub attackable: bool,
    pub pasture: i32,
    /// The main building of an annex (`+0xbb8`), if it is one.
    pub annex_of: Option<i32>,
    pub top_action: Option<i16>,
    pub position: [f32; 3],
}

impl genie_core::tasks::Diplomacy for FinderWorld {
    /// `FUN_141663dc0`: one's own player counts; Gaia never does; else the relation 0 does, and
    /// 4 with the ally flag. Relation 1 counts only under allied victory (`FUN_1415e8e10`), which
    /// the slice does not model.
    fn allied(&self, from: usize, to: usize, with_ally: bool) -> bool {
        if from == to {
            return true;
        }
        if to == 0 {
            return false;
        }
        match self.relations.get(from).and_then(|r| r.get(to)) {
            Some(0) => true,
            Some(4) => with_ally,
            _ => false,
        }
    }

    fn drops_at(&self, building: i16, actor: &genie_core::tasks::Actor<'_>) -> bool {
        genie_core::tasks::drops_at(&self.drop_sites, building, actor)
    }
}

impl FinderWorld {
    /// The finder's view of object `id`, with its master from its player's masters; an annex
    /// stands for its main building (the finder's step, `+0xbb8`), `steps` times.
    pub fn target_up(&self, id: i32, steps: usize) -> Option<genie_core::tasks::TaskTarget<'_>> {
        let mut id = id;
        for _ in 0..steps {
            match self.objects.get(&id)?.annex_of {
                Some(main) => id = main,
                None => break,
            }
        }
        let o = self.objects.get(&id)?;
        Some(genie_core::tasks::TaskTarget {
            id,
            master: self.masters.get(o.player)?.get(o.master)?,
            player: o.player,
            state: o.state,
            held: o.held,
            hp: o.hp,
            max_hp: o.max_hp,
            attackable: o.attackable,
            pasture: o.pasture,
            top_action: o.top_action,
        })
    }

    /// The finder's target: an annex stands for its main building.
    pub fn target(&self, id: i32) -> Option<genie_core::tasks::TaskTarget<'_>> {
        self.target_up(id, 1)
    }

    /// The finder's view of unit `id` as the actor.
    pub fn actor(&self, id: i32) -> Option<genie_core::tasks::Actor<'_>> {
        let o = self.objects.get(&id)?;
        Some(genie_core::tasks::Actor {
            id,
            master: self.masters.get(o.player)?.get(o.master)?,
            player: o.player,
            held: o.held,
            attribute_95: self.attribute_95.get(o.player).copied().unwrap_or(0.0),
            civ: self.civs.get(o.player).copied().unwrap_or(0),
        })
    }
}

/// The task finder's inputs from a save, with the `.dat`'s values for what the save does not hold
/// and the table's drop sites (`dropsites.json`, [`drop_sites`]).
pub fn finder_world(save: &[u8], table: &MasterTable) -> Result<FinderWorld, Error> {
    let chapter = de::chapters(save)?[0];
    let header = chapter.header()?;
    let sections = header.players();
    let masters = sections
        .iter()
        .map(|p| work_masters(p.objects(), p.world_player().base.civ_id, table))
        .collect();
    let mut objects = std::collections::HashMap::new();
    for (player, section) in sections.iter().enumerate() {
        for o in section.live_objects().chain(section.sleeping_objects()) {
            let Some(s) = o.static_part() else { continue };
            let combat = o.combat();
            let powerup = |f: fn(&genie_save::generated::world::TribeCombatObject) -> f32| {
                combat.map_or(0.0, f)
            };
            let master_hp = section
                .objects()
                .master(s.master_obj)
                .and_then(|m| m.static_part())
                .map_or(0.0, |m| f32::from(m.read_temp_6));
            objects.insert(
                s.id,
                FinderObject {
                    player,
                    master: s.master_obj,
                    state: s.object_state,
                    held: (s.attribute_type_held, s.attribute_amount_held),
                    hp: s.hp
                        + (powerup(|c| c.hitpoints_max_powerup) - powerup(|c| c.hitpoints_powerup)),
                    max_hp: master_hp + powerup(|c| c.hitpoints_max_powerup),
                    attackable: s.unit_attackable_flag != 0,
                    pasture: s.parent_pasture_id,
                    annex_of: o
                        .tribe_building_object
                        .as_ref()
                        .map(|b| b.mupt_linked_object_references_linked_owner_id)
                        .filter(|&owner| owner != -1),
                    top_action: o
                        .actions()
                        .and_then(|a| a.entries().next())
                        .map(|(kind, _)| kind),
                    position: [s.t_x, s.t_y, s.t_z],
                },
            );
        }
    }
    Ok(FinderWorld {
        masters,
        objects,
        relations: sections
            .iter()
            .map(|p| p.world_player().base.relation.clone())
            .collect(),
        attribute_95: sections
            .iter()
            .map(|p| {
                p.world_player()
                    .base
                    .attributes
                    .get(95)
                    .copied()
                    .unwrap_or(0.0)
            })
            .collect(),
        civs: sections
            .iter()
            .map(|p| p.world_player().base.civ_id)
            .collect(),
        drop_sites: table.drop_sites.clone(),
    })
}

/// The game's drop-site table (`resources/_common/dat/dropsites.json`): each entry's
/// `building_id`, `worker_object_group` (−1 when absent), `worker_object_list`, `target_civs`
/// and its targets' `attribute_type`s, in file order.
pub fn drop_sites(json: &str) -> Result<Vec<genie_core::tasks::DropSite>, serde_json::Error> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let num = |v: &serde_json::Value| v.as_i64().unwrap_or(-1);
    let list = |e: &serde_json::Value, key: &str| -> Vec<i64> {
        e.get(key)
            .and_then(|l| l.as_array())
            .map_or_else(Vec::new, |l| l.iter().map(num).collect())
    };
    Ok(v["drop_site_list"]
        .as_array()
        .map_or_else(Vec::new, |entries| {
            entries
                .iter()
                .map(|e| genie_core::tasks::DropSite {
                    building: num(&e["building_id"]) as i16,
                    worker_class: e.get("worker_object_group").map_or(-1, num) as i16,
                    workers: list(e, "worker_object_list")
                        .into_iter()
                        .map(|w| w as i16)
                        .collect(),
                    civs: list(e, "target_civs")
                        .into_iter()
                        .map(|c| c as u8)
                        .collect(),
                    resources: e["target_list"].as_array().map_or_else(Vec::new, |ts| {
                        ts.iter()
                            .map(|t| t.get("attribute_type").map_or(-1, num) as i16)
                            .collect()
                    }),
                    livestock: e.get("accepts_livestock").is_some(),
                })
                .collect()
        }))
}

/// The owner of each shape in a mesh dump (its `owner ID SHAPE` lines): the objects' `+0x64c`.
pub fn mesh_owners_from_dump(text: &str) -> Vec<(i32, u32)> {
    text.lines()
        .filter_map(|line| {
            let mut w = line.split_whitespace();
            (w.next()? == "owner").then_some(())?;
            Some((w.next()?.parse().ok()?, w.next()?.parse().ok()?))
        })
        .collect()
}

/// What the tile pathfinder reads of object `id` in a save: its master's x radius and terrain
/// restriction (master `+0x5d8`, `+0x606`). None for an object the save does not hold.
pub fn unit_terrain(save: &[u8], id: i32) -> Result<Option<(f32, usize)>, Error> {
    let chapter = de::chapters(save)?[0];
    let header = chapter.header()?;
    for section in header.players() {
        for (_, s) in live(&section) {
            if s.id != id {
                continue;
            }
            let Some(m) = section
                .objects()
                .master(s.master_obj)
                .and_then(|m| m.static_part())
            else {
                return Ok(None);
            };
            return Ok(usize::try_from(m.terrain_restriction_mask_id)
                .ok()
                .map(|r| (m.radius_x, r)));
        }
    }
    Ok(None)
}

/// The tile pathfinder's planner for one terrain restriction (`passable`, its value per terrain),
/// built from a save's map and the tiles its static obstructions block, as the game builds it at
/// load (`FUN_140f7d310`). Checked against the game's grid in `tests/tile_path.rs`.
pub fn planner_from_save(
    save: &[u8],
    terrain: &TerrainTable,
    passable: Vec<f32>,
) -> Result<genie_core::tile_path::Planner, Error> {
    use genie_core::spatial::BLOCKED;
    let chapter = de::chapters(save)?[0];
    let header = chapter.header()?;
    let map = &header.world.base_world.map;
    let terrain_ids: Vec<u8> = map
        .rows
        .iter()
        .flat_map(|r| r.tiles.iter().map(|t| t.terrain_type))
        .collect();
    let spatial = spatial(&header, terrain);
    let tiles = spatial.tiles();
    let blocked = |x: i32, y: i32| tiles.word(x, y).is_some_and(|w| w & BLOCKED != 0);
    Ok(genie_core::tile_path::Planner::new(
        [map.map_width, map.map_height],
        terrain_ids,
        passable,
        &blocked,
    ))
}

/// Where two object lists differ, one line per object and field: `slice` from the slice, `game`
/// from the game's save, matched by id. Equal lists give none.
pub fn object_differences(slice: &[ObjectRow], game: &[ObjectRow]) -> Vec<String> {
    let mut out = Vec::new();
    for g in game {
        let Some(s) = slice.iter().find(|s| s.id == g.id) else {
            out.push(format!("{}: only in the game ({g:?})", g.id));
            continue;
        };
        let mut field = |name: &str, a: String, b: String| {
            if a != b {
                out.push(format!("{}: {name} slice {a} game {b}", g.id));
            }
        };
        field("master", s.master.to_string(), g.master.to_string());
        field("state", s.state.to_string(), g.state.to_string());
        field("hp", format!("{:?}", s.hp), format!("{:?}", g.hp));
        field(
            "amount",
            format!("{:?}", s.amount),
            format!("{:?}", g.amount),
        );
        field(
            "position",
            format!("{:?}", s.position),
            format!("{:?}", g.position),
        );
        field(
            "action",
            format!("{:?}", s.action),
            format!("{:?}", g.action),
        );
        field(
            "retarget",
            format!("{:?}", s.retarget),
            format!("{:?}", g.retarget),
        );
        field(
            "construction",
            format!("{:?}", s.construction),
            format!("{:?}", g.construction),
        );
        field(
            "build added",
            format!("{:?}", s.build_added),
            format!("{:?}", g.build_added),
        );
        field(
            "last human order time",
            s.human_order.to_string(),
            g.human_order.to_string(),
        );
        field(
            "gather point",
            format!("{:?}", s.gather),
            format!("{:?}", g.gather),
        );
        field(
            "charge, charge hit, hp powerups",
            format!("{:?}", s.combat),
            format!("{:?}", g.combat),
        );
        field("gate", format!("{:?}", s.gate), format!("{:?}", g.gate));
        match (&s.ai, &g.ai) {
            (Some(a), Some(b)) => {
                for ((name, x), (_, y)) in a.fields.iter().zip(&b.fields) {
                    field(name, x.clone(), y.clone());
                }
            }
            (a, b) => field(
                "unit AI",
                format!("{}", a.is_some()),
                format!("{}", b.is_some()),
            ),
        }
        match (&s.movement, &g.movement) {
            (Some(a), Some(b)) => {
                field(
                    "forward",
                    format!("{:?}", a.forward),
                    format!("{:?}", b.forward),
                );
                field("right", format!("{:?}", a.right), format!("{:?}", b.right));
                field(
                    "velocity",
                    format!("{:?}", a.velocity),
                    format!("{:?}", b.velocity),
                );
                field(
                    "path levels",
                    a.paths.len().to_string(),
                    b.paths.len().to_string(),
                );
                for (i, (pa, pb)) in a.paths.iter().zip(&b.paths).enumerate() {
                    field(
                        &format!("path level {i}"),
                        format!("{pa:?}"),
                        format!("{pb:?}"),
                    );
                }
                let bytes = |m: &MoveRow| {
                    format!(
                        "{:?}",
                        (
                            m.waiting,
                            m.wait_delays,
                            m.attack_moving,
                            m.substitute_count
                        )
                    )
                };
                field("move bytes", bytes(a), bytes(b));
                field(
                    "last move",
                    a.last_move.to_string(),
                    b.last_move.to_string(),
                );
                field(
                    "ignore units",
                    format!("{:?}", a.ignore_units),
                    format!("{:?}", b.ignore_units),
                );
            }
            // a building's movement part without velocity data or paths is not modelled
            (None, Some(b)) if b.velocity.is_none() && b.paths.is_empty() => {}
            (a, b) => field(
                "movement",
                format!("{:?}", a.is_some()),
                format!("{:?}", b.is_some()),
            ),
        }
    }
    for s in slice.iter().filter(|s| !game.iter().any(|g| g.id == s.id)) {
        out.push(format!("{}: only in the slice ({s:?})", s.id));
    }
    out
}
