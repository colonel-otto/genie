//! The replay oracle: state read from a recording's header save, checked against the checkpoints
//! the recording stores in its body.
//!
//! The first checkpoint comes 500 syncs into the game; a recording has none at tick 0. Ten of its
//! eleven words per player still equal the start-of-game state ([`SAVE_WORDS`]). Word 9 does not:
//! it sums each object's top action and unit-AI retarget timer, which the engine redraws while the
//! game runs, so reproducing it needs the simulation.

// The port's exceptions to the workspace's lints: the engine reproduces build 158041 bit for bit,
// so it keeps the game's arithmetic and conversions as they are.
#![allow(
    // the game's f32 sums and products, each rounded on its own (no fused multiply-add)
    clippy::suboptimal_flops,
    clippy::imprecise_flops,
    // the game compares floats exactly, in conditions and loops
    clippy::float_cmp,
    clippy::while_float,
    // the game's conversions (cvttss2si, movsx, movzx, truncating stores), written as `as`
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    // the decompiled routines' math names (dx, dy, x0, y1 …), formulas, flag fields and
    // parameters, assignment order, and member functions whose `this` goes unused
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::suspicious_operation_groupings,
    clippy::struct_excessive_bools,
    clippy::fn_params_excessive_bools,
    clippy::useless_let_if_seq,
    clippy::unused_self,
    // the contexts are bundles of references, passed by value
    clippy::needless_pass_by_value,
    // a routine is ported whole, its documentation opens with the routine and its addresses
    clippy::too_many_lines,
    clippy::too_long_first_doc_paragraph,
    // every error is `Unsupported`, which names the part the slice does not model
    clippy::missing_errors_doc
)]

pub mod census;
pub mod groups;
pub mod masters;
pub mod tick_trace;
pub mod views;

pub use masters::{DefaultAction, MasterTable, ReadError, TerrainTable};

use genie_core::artifact::ARTIFACT_TYPE;
use genie_core::checkpoint::{ObjectState, PlayerState, TopAction, player_block};
use genie_core::lifecycle::{Class, Lifecycle, MasterData};
use genie_core::turn::{Command, Player, Turns, Unit};
use genie_core::unit_ai::Unsupported;
use genie_core::visibility::{
    BuildingSight, Extent, Seer, Visibility, VisibleMap, VisibleResource, VisibleResources,
    capture_kind,
};
use genie_rec::de::{self, Op};
use genie_save::Header;
use genie_save::PlayerSection;
use genie_save::generated::world::{
    Master, Path, PathData, PlayerObjects, RgeMasterAnimatedObject, RgeStaticObject,
};

/// The words of the first checkpoint that the start-of-game save determines: all but word 9.
pub const SAVE_WORDS: [usize; 10] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 10];

/// One player's block, computed from the save and as the recording stores it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerComparison {
    /// The player's id.
    pub id: u32,
    /// The block computed from the header save.
    pub from_save: [u32; 11],
    /// The block the checkpoint stores.
    pub checkpoint: [u32; 11],
}

/// Why a recording could not be compared.
#[derive(Debug)]
pub enum Error {
    /// The recording could not be read.
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
    /// A tile list dump's line out of shape.
    BadTileDump,
    /// A recycled pools dump's line out of shape.
    BadPoolDump,
    /// A recording without a chapter.
    NoChapter,
    /// A player the save does not have.
    NoPlayer(usize),
    /// What the slice does not model of the save.
    Unmodelled(Unsupported),
}

impl From<de::Error> for Error {
    fn from(e: de::Error) -> Self {
        Self::Rec(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rec(e) => write!(f, "{e}"),
            Self::NoCheckpoint => write!(f, "the recording has no checkpoint"),
            Self::NoSuchPlayer(id) => write!(f, "a checkpoint names player {id}, not in the save"),
            Self::CommandBeforeFirstSync => write!(f, "a command comes before the first sync"),
            Self::BadMeshDump => write!(f, "the mesh dump has no mesh line"),
            Self::BadRegistryDump => write!(f, "the registry dump is out of shape"),
            Self::BadTileDump => write!(f, "the tile list dump is out of shape"),
            Self::BadPoolDump => write!(f, "the recycled pools dump is out of shape"),
            Self::NoChapter => write!(f, "the recording has no chapter"),
            Self::Unmodelled(u) => write!(f, "not modelled: {}", u.0),
            Self::NoPlayer(p) => write!(f, "no player {p} in the save"),
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

/// An object of a player's live list: not the stand-in for a saved object without a static part
/// ([`unit`]). A missile's id is below 0 (the world's reusable ids, `next_reusable_object_id`).
const fn listed(u: &Unit) -> bool {
    u.id >= 0 || u.life.master_id >= 0
}

/// An object saved alive without hit points: its first update kills it, and it leaves the live
/// list within a few ticks (genie-core's lifecycle; a flare's 1 s corpse timer takes 14). The
/// save's PLACEHOLDER2s, FLARE3s and empty town-centre annexes are such objects.
fn dies_at_start(s: &RgeStaticObject) -> bool {
    matches!(s.object_state, 0 | 2) && s.hp < 1.0
}

/// What the block reads of a saved object, before the slice changes any of it.
const fn object_state(s: &RgeStaticObject) -> ObjectState {
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
/// MOVE, `FORM_FORMATION`, `GROUP_MULTI_WAYPOINTS`, BUILD, REPAIR. More player-side callers exist and
/// are not mapped yet. A group draws from the World stream every tick while active and every 3 s
/// while idle (`FUN_14103d910` → `FUN_14102d370`), which the slice does not model. Whether a command
/// creates one depends on its units: the splitter (`FUN_140fd76d0`) groups only clusters of two or
/// more formation-capable units, and villagers never join a formation (as openage's
/// `doc/reverse_engineering/game_mechanics/formations.md` describes for `AoC`).
pub const GROUP_COMMANDS: [u8; 7] = [0x00, 0x02, 0x03, 0x17, 0x1f, 0x66, 0x6e];

/// The player attribute of the construction rate modifier (195; the work reads it at player
/// attributes `+0x618`, eight bytes per attribute).
const CONSTRUCTION_RATE: usize = 195;

/// A command that may create a unit group and so take the frame-seed chain out of the slice's model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupCommand {
    /// The tick (1-based) the command executes in: the frame of the sync before it.
    pub tick: u32,
    /// The commanding player.
    pub player: u8,
    /// The command's byte.
    pub command: u8,
}

/// Word 9 of one player's first checkpoint, from the unit-AI slice and as recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Word9 {
    /// The player's id.
    pub id: u32,
    /// Word 9 from the slice.
    pub simulated: u32,
    /// Word 9 as the checkpoint stores it.
    pub checkpoint: u32,
    /// The first command before the checkpoint, by any player, that may create a unit group.
    pub group_command: Option<GroupCommand>,
    /// The first unit update of this player the slice could not model.
    pub unsupported: Option<(u32, usize, Unsupported)>,
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
        untargetable: s.unit_targeting_flag == 0,
        under_attack: s.under_attack_value != 0,
        pasture: s.parent_pasture_id,
        my_pasture: s.my_pasture_id,
        undeletable: s.deletion_enabled == 0,
        capture_flag: object
            .combat()
            .map_or(0, |c| c.rge_combat_object.capture_flag),
        annex_of: object
            .tribe_building_object
            .as_deref()
            .map(|b| b.mupt_linked_object_references_linked_owner_id)
            .filter(|&owner| owner != -1),
        group: group_link(s),
        hp_powerups: object
            .combat()
            .map_or([0.0; 2], |c| [c.hitpoints_powerup, c.hitpoints_max_powerup]),
        workers: s.worker_num,
        last_human_order: s.last_human_order_time,
        town_bell: object.combat().map(town_bell).unwrap_or_default(),
        combat: object.combat().is_some(),
        next_volley: object.combat().map_or(0, |c| c.next_volley),
        raider_countdown: object.combat().map_or(0, |c| c.raider_build_countdown),
        garrison_time: object
            .tribe_building_object
            .as_deref()
            .map_or(0, |b| b.last_garrison_time),
        relics: object
            .tribe_building_object
            .as_deref()
            .map_or([0; 2], |b| [b.relic_count, b.specific_relic_count]),
        reload_worldtime: object
            .combat()
            .and_then(|c| c.rge_combat_object.reload_worldtime)
            .unwrap_or(0),
        // the load's (FUN_14121ec90): the master's volley fire; the shares of the units inside
        // follow in `garrison_firepower`
        firepower: objects
            .master(s.master_obj)
            .and_then(|m| {
                m.tribe_master_combat_object.as_deref().or_else(|| {
                    m.tribe_master_building_object
                        .as_deref()
                        .map(|b| &b.tribe_master_combat_object)
                })
            })
            .map_or(0.0, |c| c.volley_fire_amount),
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
        // every animated object's current sprite, which slot 0x68 and the sprite list read
        sprite: masters.and_then(|t| saved_sprite(s, &t.graphics)),
        charge_hit: object
            .combat()
            .is_some_and(|c| c.rge_combat_object.hit_something_valid_for_charge_attack != 0),
        gather_point: gather_point(object),
        gate: saved_gate(object),
        attack_count: object
            .combat()
            .map(|c| &c.rge_combat_object)
            .or_else(|| {
                object
                    .rge_object
                    .as_deref()
                    .and_then(|r| r.rge_missile_object.as_deref())
                    .map(|m| &m.rge_combat_object)
            })
            .map_or(0, |c| c.attack_count),
        roo_called: s.rooalready_called != 0,
        // mIsOnMangroveShallows (+0xcd0); build 158041's saves all hold it (its version gate
        // 0x421999 lies before them)
        rice: object
            .tribe_building_object
            .as_deref()
            .and_then(|b| b.is_on_mangrove_shallows)
            .is_some_and(|v| v != 0),
        // the combat object's localResourceStore (+0xae0), a map by attribute saved in its order
        store: object
            .combat()
            .map(|c| {
                c.local_resource_store
                    .iter()
                    .map(|e| (e.attribute_idx as i16, e.value))
                    .collect()
            })
            .unwrap_or_default(),
        ..Unit::default()
    };
    let imported = if saved {
        saved_actions(
            object,
            s,
            objects,
            |m| build_task(objects, civ, masters, m),
            |m| fly_task(objects, civ, masters, m),
            |m, i| task_at(objects, civ, masters, m, i),
            masters.map(|t| &t.graphics),
            &mut unit,
        )
    } else {
        // an object without actions in a save: its default task's action (an Artifact exists
        // from the load on; a Fly starts in its first update)
        default_task(object, s, objects, civ, masters).map(|d| match d {
            Some(DefaultStart::Artifact(a)) => unit.artifact = Some(a),
            Some(DefaultStart::Fly(task)) => unit.default_fly = Some(task),
            Some(DefaultStart::Make) => unit.default_make = true,
            None => {}
        })
    };
    // what the default task starts once the action list empties: nothing, a Fly, the Artifact
    // (whose restart the slice does not model) or something not modelled
    unit.default_task = match default_task(object, s, objects, civ, masters) {
        Ok(None | Some(DefaultStart::Fly(_) | DefaultStart::Make)) => None,
        Ok(Some(DefaultStart::Artifact(_))) => Some(Unsupported(
            "the default task's restart (the Artifact action)",
        )),
        Err(e) => Some(e),
    };
    if saved {
        // a saved object's Fly is its saved action's: none starts again (it never ends)
        unit.default_fly = None;
    }
    // a missile without its action (one that met its target, dying): the object's own fields, and
    // the flying flag the load constructor sets (`+0x994` 1, not saved)
    if unit.missile.is_none()
        && let Some(mo) = object
            .rge_object
            .as_deref()
            .and_then(|r| r.rge_missile_object.as_deref())
    {
        let step = object
            .moving_part()
            .map_or([0.0; 3], |m| [m.velocity_x, m.velocity_y, m.velocity_z]);
        unit.missile = Some(genie_core::missile::Missile {
            max_range: mo.max_range,
            fired_from: mo.fired_from_id,
            charge: mo.is_charge_projectile != 0,
            flying: true,
            step,
            attack_count: mo.rge_combat_object.attack_count,
            action: None,
        });
    }
    let production = production(object).map(|p| unit.production = p);
    let moving = mover(object, s, objects, paths, civ, masters).map(|m| unit.mover = m);
    unit.unmodelled_action = imported.and(production).and(moving).err();
    unit
}

/// A combat object's town-bell flag and back-to-work records.
const fn town_bell(
    c: &genie_save::generated::world::TribeCombatObject,
) -> genie_core::town_bell::TownBell {
    use genie_core::town_bell::{BackToWork, TownBell};
    TownBell {
        flag: c.town_bell_flag != 0,
        work: BackToWork {
            target: c.back_to_work1_town_bell_target_id,
            target_type: c.back_to_work1_town_bell_target_type,
            action: c.back_to_work1_town_bell_action,
            target2: c.back_to_work1_town_bell_target_id2,
            point: [
                c.back_to_work1_town_bell_target_x,
                c.back_to_work1_town_bell_target_y,
            ],
        },
        before_building: BackToWork {
            target: c.back_to_work2_town_bell_target_id,
            target_type: c.back_to_work2_town_bell_target_type,
            action: c.back_to_work2_town_bell_action,
            target2: c.back_to_work2_town_bell_target_id2,
            point: [
                c.back_to_work2_town_bell_target_x,
                c.back_to_work2_town_bell_target_y,
            ],
        },
    }
}

/// A saved building's gate state ([`genie_core::turn::Gate`]): its lock, first update and close
/// timer, and whether a saved open or shut master or its original owner's would take part in a
/// toggle.
fn saved_gate(object: &genie_save::Object) -> Option<genie_core::turn::Gate> {
    let b = object.tribe_building_object.as_deref()?;
    Some(genie_core::turn::Gate {
        locked: b.gate_locked != 0,
        first_update: b.first_update != 0,
        close_timer: b.close_timer,
        other_masters: b.has_open_closed_master_object != 0 || b.original_owner != -1,
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
    let b = object.tribe_building_object.as_deref()?;
    let m = objects.master(s.master_obj)?;
    let ms = m.static_part()?;
    let build_time = m
        .tribe_master_building_object
        .as_deref()
        .map(|b| &b.tribe_master_combat_object)
        .or(m.tribe_master_combat_object.as_deref())
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
        started: false,
    })
}

fn vec3(b: &[u8; 12]) -> genie_core::fmath::Vec3 {
    let f = |i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    [f(0), f(4), f(8)]
}

/// The animated part of a player's master, which holds its speed (`mMaxSpeed`, master `+0x740`).
fn animated_master(m: &Master) -> Option<&RgeMasterAnimatedObject> {
    let combat = m.tribe_master_combat_object.as_deref().or_else(|| {
        m.tribe_master_building_object
            .as_deref()
            .map(|b| &b.tribe_master_combat_object)
    });
    if let Some(c) = combat {
        let moving = &c
            .rge_master_combat_object
            .rge_master_action_object
            .rge_master_moving_object;
        return Some(&moving.rge_master_animated_object);
    }
    let r = m.rge_master.as_deref()?;
    None.or(r.rge_master_animated_object.as_deref())
        .or_else(|| {
            r.rge_master_moving_object
                .as_deref()
                .map(|m| &m.rge_master_animated_object)
        })
        .or_else(|| {
            r.rge_master_action_object
                .as_deref()
                .map(|a| &a.rge_master_moving_object.rge_master_animated_object)
        })
        .or_else(|| {
            r.rge_master_combat_object.as_deref().map(|c| {
                &c.rge_master_action_object
                    .rge_master_moving_object
                    .rge_master_animated_object
            })
        })
        .or_else(|| {
            r.rge_master_missile_object.as_deref().map(|m| {
                &m.rge_master_combat_object
                    .rge_master_action_object
                    .rge_master_moving_object
                    .rge_master_animated_object
            })
        })
}

/// The moving part of a saved master, when its class has one.
fn moving_master(m: &Master) -> Option<&genie_save::generated::world::RgeMasterMovingObject> {
    let combat = m.tribe_master_combat_object.as_deref().or_else(|| {
        m.tribe_master_building_object
            .as_deref()
            .map(|b| &b.tribe_master_combat_object)
    });
    if let Some(c) = combat {
        return Some(
            &c.rge_master_combat_object
                .rge_master_action_object
                .rge_master_moving_object,
        );
    }
    let r = m.rge_master.as_deref()?;
    None.or(r.rge_master_moving_object.as_deref())
        .or_else(|| {
            r.rge_master_action_object
                .as_deref()
                .map(|a| &a.rge_master_moving_object)
        })
        .or_else(|| {
            r.rge_master_combat_object
                .as_deref()
                .map(|c| &c.rge_master_action_object.rge_master_moving_object)
        })
        .or_else(|| {
            r.rge_master_missile_object.as_deref().map(|m| {
                &m.rge_master_combat_object
                    .rge_master_action_object
                    .rge_master_moving_object
            })
        })
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
        has_path: path.is_some(),
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
        ignored: Vec::new(),
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
    let (radius, restriction, base, turn, flags, standing, sprite_facets) =
        match (s.own_master(), s.own_animated_master()) {
            (Some(own), Some(animated)) => (
                [own.radius_x, own.radius_y, own.radius_z],
                own.terrain_restriction_mask_id,
                animated.max_speed,
                s.own_moving_master()
                    .map(|m| (m.turn_speed, m.temp_move_sprite, m.temp_run_sprite)),
                own.object_flags,
                own.sprite_num,
                // an own master copy (a captured animal's) shows its master's sprites
                masters.and_then(|t| {
                    let master = objects.master(s.master_obj)?;
                    master_facets(master, master.static_part()?, t)
                }),
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
                    moving_master(master)
                        .map(|m| (m.turn_speed, m.move_sprite_id, m.run_sprite_id)),
                    placement.object_flags,
                    placement.idle_sprite_id,
                    masters.and_then(|t| master_facets(master, placement, t)),
                )
            }
        };
    let table = masters.ok_or(Unsupported("no master table: turning is unknown"))?;
    // without a sprite (−1: the empty town-centre annex 890, say) no factor
    let factor = if s.sprite == -1 {
        0.0
    } else {
        table
            .sprite_speed(s.sprite)
            .ok_or(Unsupported("a sprite the data file does not have"))?
    };
    let speed = if 0.0 < factor { base * factor } else { base };
    let turning = table
        .turning(civ, s.master_obj)
        .ok_or(Unsupported("a moving master the data file does not have"))?;
    let (turn_speed, walking, running) =
        turn.ok_or(Unsupported("a moving master without a moving part"))?;
    // the top speed with a sprite: the master's speed times the sprite's factor above 0
    let with = |sprite: i16| {
        let f = table.sprite_speed(sprite).unwrap_or(0.0);
        if 0.0 < f { base * f } else { base }
    };
    let walk_speed = with(walking);
    let run_speed = if 0 <= running {
        with(running)
    } else {
        walk_speed
    };
    let facets = table.graphics.get(&walking).map(|g| g.facets);
    let velocity = (m.have_movement == 1).then(|| Velocity {
        velocity: m.velocity.as_ref().map_or([0.0; 3], vec3),
        acceleration: m.acceleration.as_ref().map_or([0.0; 3], vec3),
    });
    let mut levels = m
        .temp_path_data
        .iter()
        .rev()
        .map(|d| path_level(d, paths))
        .collect::<Result<Vec<_>, _>>()?;
    loaded_ignored(&mut levels, &s.temp_int);
    Ok(Some(Mover {
        forward: vec3(&m.orientation_forward_value),
        right: vec3(&m.orientation_right_value),
        velocity,
        paths: levels,
        factor: 1.0,
        stats: MoveStats {
            fly_mode: table.fly_mode(civ, s.master_obj).unwrap_or(0),
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
            turn_time: turn_speed,
            move_algorithm: turning.move_algorithm,
            min_collision: turning.min_collision_size_multiplier,
            fast_turn: flags & 2 != 0,
            walk_speed,
            run_speed,
            stand_speed: with(standing),
            facets: sprite_facets,
            weapon_range: objects
                .master(s.master_obj)
                .and_then(combat_master)
                .map_or(0.0, |c| c.weapon_range),
        },
        waiting: m.waiting_to_move_value,
        wait_delays: m.number_wait_delays,
        attack_moving: m.attack_moving != 0,
        last_move: m.last_move_time_value,
        // the save's x, height, y
        moved_at: {
            let f = |i: usize| {
                let b = &m.position_value;
                f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
            };
            [f(0), f(8), f(4)]
        },
        substitute_count: m.consecutive_substitute_count,
        ignore_units: s.temp_int.clone(),
        // the save's userDefinedWaypoints, in the game's layout (x, height, y)
        user_waypoints: m.user_defined_waypoints.iter().map(vec3).collect(),
        last_turn: m.turn_towards_time,
        angle: m.angle,
        move_to_point: saved_move_to_point(object),
        speed_stopped: false,
        pauses: Vec::new(),
        in_the_way: Vec::new(),
        // the current sprite's, which the unit's update takes from its sprite
        sprite_facets: None,
    }))
}

/// A loaded unit's paths' ignore units (`+0x450`), which the save does not hold: a level below
/// the top whose path was made in the top path's tick took the unit's own (`+0x670`,
/// `FUN_140f9b7a0` after the short pather).
fn loaded_ignored(levels: &mut [genie_core::movement::PathData], units: &[i32]) {
    let Some((top, below)) = levels.split_last_mut() else {
        return;
    };
    if top.points.is_empty() {
        return;
    }
    for level in below.iter_mut().filter(|l| l.created == top.created) {
        level.ignored = units.to_vec();
    }
}

/// The point of the `MoveTo` a saved unit walks with to a target object: its top action when that
/// is a `MoveTo`, else the top action's first sub-action (or that Attack's own first one), when
/// that `MoveTo` has a target (its `target_x`..`target_z`).
fn saved_move_to_point(object: &genie_save::Object) -> Option<[f32; 3]> {
    let (top_kind, top) = object.actions()?.entries().next()?;
    if top_kind == genie_core::move_to::MOVE_TO {
        let b = top.base()?;
        return (b.target_id != -1).then_some([b.target_x, b.target_y, b.target_z]);
    }
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
/// Build action (101) with its `MoveTo`, a `MoveTo` of its own (type 1), a bird's Fly action (10)
/// with its `MoveTo`, the Hunt (110) with its Attack or `MoveTo` (and in state 8 its second
/// target, the drop site) and a farm's Make (21). Any other saved action stops the object's
/// player.
#[allow(clippy::too_many_arguments)]
fn saved_actions(
    object: &genie_save::Object,
    s: &RgeStaticObject,
    objects: &PlayerObjects,
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
    // the master's running sprite, which an Attack's MoveTo may show
    let run_sprite = objects
        .master(s.master_obj)
        .and_then(moving_master)
        .map_or(-1, |m| m.run_sprite_id);
    let base = action
        .base()
        .ok_or(Unsupported("a saved action without its base"))?;
    // the second target (+0x34): only the Hunt and the Gather (their drop site), the missile (its
    // shooter) and the Attack hold one in the slice
    if base.target2_id != -1
        && !matches!(
            kind,
            genie_core::hunt::HUNT
                | genie_core::gather::GATHER
                | genie_core::missile::MISSILE
                | genie_core::attack::ATTACK
        )
    {
        return Err(Unsupported("a saved action with a second target"));
    }
    match kind {
        ARTIFACT_TYPE => {
            let sub = match base.sub_actions.entries().next() {
                None => None,
                Some((MOVE_TO, sub)) => Some(sub),
                Some(_) => {
                    return Err(Unsupported(
                        "a saved Artifact action with another sub-action",
                    ));
                }
            };
            let move_to = sub
                .map(|sub| move_to_state(sub.base().map_or(0, |b| b.state)))
                .transpose()?;
            // a move to an object (set_state 5): the target, and its quiet MoveTo's goal, a
            // town centre's point or the target, with its range
            let walk_to = match sub {
                Some(sub) if base.target_id != -1 => {
                    let b = sub
                        .base()
                        .ok_or(Unsupported("a saved MoveTo without its base"))?;
                    let goal = if b.target_id == -1 {
                        genie_core::move_to::Goal::Point(
                            point(b).ok_or(Unsupported("a saved MoveTo without its point"))?,
                        )
                    } else {
                        genie_core::move_to::Goal::Target(b.target_id)
                    };
                    let range = sub
                        .move_to_range()
                        .ok_or(Unsupported("a saved MoveTo without its range"))?;
                    Some((goal, range))
                }
                _ => None,
            };
            unit.artifact = Some(genie_core::artifact::Artifact {
                state: base.state,
                timer: base.timer,
                gaia: s.owner == 0,
                move_to,
                goal: if base.target_id == -1 {
                    move_to.and_then(|_| point(base))
                } else {
                    // the target's place, `+0x38`
                    (base.target_x != -1.0).then_some([base.target_x, base.target_y, base.target_z])
                },
                target: base.target_id,
                walk_to,
            });
            Ok(())
        }
        MOVE_TO => {
            unit.move_to = Some(MoveToAction {
                state: move_to_state(base.state)?,
                goal: point(base),
                target: base.target_id,
                range: action
                    .move_to_range()
                    .ok_or(Unsupported("a saved MoveTo without its range"))?,
                // its sprite (`+0x48`'s id) is the master's running one
                run: objects
                    .master(s.master_obj)
                    .and_then(moving_master)
                    .is_some_and(|m| {
                        0 <= m.run_sprite_id
                            && m.run_sprite_id != m.move_sprite_id
                            && base.short_val_2 == m.run_sprite_id
                    }),
                // its wait (`+0x6c`, `+0x70`, `+0x74`)
                wait: action
                    .rge_action_by_type
                    .as_deref()
                    .and_then(|b| b.rge_action_move_to.as_deref())
                    .map_or_else(genie_core::move_to::MoveWait::default, |m| {
                        genie_core::move_to::MoveWait {
                            state: m.movement_wait_state,
                            timer: m.timer,
                            delay: f32::from_bits(m.k_default_wait_time),
                        }
                    }),
            });
            Ok(())
        }
        genie_core::missile::MISSILE => {
            let ma = action
                .rge_action_by_type
                .as_deref()
                .and_then(|t| t.rge_action_missile.as_deref())
                .ok_or(Unsupported("a saved missile action without its part"))?;
            let mo = object
                .rge_object
                .as_deref()
                .and_then(|r| r.rge_missile_object.as_deref())
                .ok_or(Unsupported(
                    "a missile action of an object that is not a missile",
                ))?;
            let moving = object
                .moving_part()
                .ok_or(Unsupported("a missile without its moving part"))?;
            // the hit mode copies the master's (`+0x80`, a miss's 1 is not saved)
            let hit_mode = objects
                .master(s.master_obj)
                .and_then(|m| m.rge_master.as_deref())
                .and_then(|r| r.rge_master_missile_object.as_deref())
                .map_or(0, |m| m.missile_hit_info);
            unit.missile = Some(genie_core::missile::Missile {
                max_range: mo.max_range,
                fired_from: mo.fired_from_id,
                charge: mo.is_charge_projectile != 0,
                flying: genie_core::missile::LANDING < ma.distance,
                step: [moving.velocity_x, moving.velocity_y, moving.velocity_z],
                attack_count: mo.rge_combat_object.attack_count,
                action: Some(genie_core::missile::MissileAction {
                    state: base.state,
                    target: base.target_id,
                    fired_from: base.target2_id,
                    point: [base.target_x, base.target_y, base.target_z],
                    timer: base.timer,
                    direction: [ma.velocity_x, ma.velocity_y, ma.velocity_z],
                    ballistic_velocity: ma.ballistic_velocity,
                    ballistic_acceleration: ma.ballistic_acceleration,
                    distance: ma.distance,
                    hit_mode,
                    result: ma.interacted_with_agility,
                    origin: genie_core::missile::Origin::from_bytes(&ma.projectile_data),
                    met: Vec::new(),
                }),
            });
            Ok(())
        }
        genie_core::make::MAKE => {
            let make = action
                .rge_action_by_type
                .as_deref()
                .and_then(|t| t.rge_action_make.as_deref())
                .ok_or(Unsupported("a saved Make without its part"))?;
            if base.sub_actions.entries().next().is_some() {
                return Err(Unsupported("a saved Make with a sub-action"));
            }
            unit.make = Some(genie_core::make::MakeAction {
                state: base.state,
                remaining: base.timer,
                total: make.work_timer,
                task: task_at(s.master_obj, base.short_val)?,
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
                    .as_deref()
                    .is_some_and(|b| b.added_to_num_builders != 0),
                timer: base.timer,
            });
            Ok(())
        }
        genie_core::hunt::HUNT => {
            use genie_core::hunt::{
                ATTACK, DROP, FACE, GATHER, HuntAction, HuntTask, IDLE, LEAD, SEARCH, WALK,
                WALK_POINT, WALK_SEARCH,
            };
            let part = action
                .tribe_action_hunt
                .as_deref()
                .ok_or(Unsupported("a saved Hunt without its own part"))?;
            if !matches!(
                base.state,
                IDLE | SEARCH
                    | WALK
                    | FACE
                    | GATHER
                    | DROP
                    | LEAD
                    | ATTACK
                    | WALK_SEARCH
                    | WALK_POINT
            ) {
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
                lead: genie_core::hunt::Lead {
                    moved: part.moved_sheep != 0,
                    still_time: part.sheep_still_time as i16,
                    start: [
                        f32::from_bits(part.sheep_start_x),
                        f32::from_bits(part.sheep_start_y),
                    ],
                },
            };
            match base.sub_actions.entries().next() {
                None => {}
                Some((genie_core::attack::ATTACK, sub)) => {
                    let mut attack = saved_attack(sub, graphics, &move_to_state, run_sprite)?;
                    // a swing past its hit: the swing's times (`+0xb0`, `+0xb4`, unsaved) are what
                    // the hit left, the reload less the attack sprite's length
                    if attack.state == genie_core::attack::SWING && !attack.pending {
                        let reload = saved_reload(object, s, objects)
                            .ok_or(Unsupported("a saved swing without its unit's reload"))?;
                        let g = attack
                            .arms
                            .sprite
                            .ok_or(Unsupported("a saved swing without its attack sprite"))?;
                        let rest = reload - f32::from(g.frames) * g.frame_duration;
                        attack.swing = rest;
                        attack.rest = rest.max(0.0);
                    }
                    hunt.attack = Some(attack);
                    // the attack reads and changes the unit's sprite from here on
                    unit.sprite = graphics.and_then(|g| saved_sprite(s, g));
                }
                Some((MOVE_TO, sub)) => {
                    hunt.move_to = Some(move_to_state(sub.base().map_or(0, |b| b.state))?);
                    hunt.walk_range = sub
                        .move_to_range()
                        .ok_or(Unsupported("a saved MoveTo without its range"))?;
                    // its wait (`+0x6c`, `+0x70`, `+0x74`)
                    if let Some(m) = sub
                        .rge_action_by_type
                        .as_deref()
                        .and_then(|b| b.rge_action_move_to.as_deref())
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
        genie_core::gather::GATHER => {
            use genie_core::gather::{
                DONE, DROP, FACE, GatherAction, IDLE, MOVED, SEARCH, WALK, WALK_BACK, WORK,
            };
            let part = action
                .rge_action_by_type
                .as_deref()
                .and_then(|b| b.rge_action_gather.as_deref())
                .ok_or(Unsupported("a saved Gather without its own part"))?;
            if !matches!(
                base.state,
                WALK | FACE | WORK | DROP | DONE | IDLE | MOVED | SEARCH | WALK_BACK
            ) {
                return Err(Unsupported(
                    "a saved Gather in a state the slice does not model",
                ));
            }
            let t = task_at(s.master_obj, base.short_val)?;
            let task = genie_core::hunt::HuntTask {
                id: t.id,
                range: t.work_range,
                resource: t.attribute_type,
                sprites: t.sprites,
                work_value: t.work_val1,
                productivity: t.attribute_type2,
                kind4: t.attribute_type4,
            };
            let mut gather = GatherAction {
                target: base.target_id,
                carry_to: base.target2_id,
                state: base.state,
                point: [base.target_x, base.target_y, base.target_z],
                timer: base.timer,
                task,
                target_type: part.target_type,
                stops: part.number_stops,
                current_stop: part.current_stop,
                time_per_stop: part.time_per_stop,
                time_at_stop: part.time_at_current_stop,
                last_stop: [part.last_stop_x, part.last_stop_y],
                // `+0x80` is not saved: set_state(7) set it
                worked: base.state == WORK,
                farm: [part.farm_x, part.farm_y],
                gather_at: [part.gather_x, part.gather_y],
                move_to: None,
                walk_to: None,
                walk_range: task.range,
                move_wait: genie_core::move_to::MoveWait::default(),
            };
            match base.sub_actions.entries().next() {
                None => {}
                Some((MOVE_TO, sub)) => {
                    gather.move_to = Some(move_to_state(sub.base().map_or(0, |b| b.state))?);
                    // a MoveTo without a target walks to its point (a farm's stop)
                    gather.walk_to = sub
                        .base()
                        .filter(|b| b.target_id == -1)
                        .map(|b| [b.target_x, b.target_y, b.target_z]);
                    gather.walk_range = sub
                        .move_to_range()
                        .ok_or(Unsupported("a saved MoveTo without its range"))?;
                    if let Some(m) = sub
                        .rge_action_by_type
                        .as_deref()
                        .and_then(|b| b.rge_action_move_to.as_deref())
                    {
                        gather.move_wait = genie_core::move_to::MoveWait {
                            state: m.movement_wait_state,
                            timer: m.timer,
                            delay: f32::from_bits(m.k_default_wait_time),
                        };
                    }
                }
                Some(_) => return Err(Unsupported("a saved Gather with another sub-action")),
            }
            unit.gather = Some(gather);
            Ok(())
        }
        genie_core::garrison::ENTER => {
            use genie_core::garrison::{ENTERED, EnterAction};
            let first_time = action
                .rge_action_enter
                .as_deref()
                .ok_or(Unsupported("a saved Enter without its own part"))?
                .first_time_enter;
            let move_to = match base.sub_actions.entries().next() {
                None => None,
                Some((MOVE_TO, sub)) => Some(move_to_state(sub.base().map_or(0, |b| b.state))?),
                Some(_) => return Err(Unsupported("a saved Enter with another sub-action")),
            };
            if !matches!(base.state, ENTERED | 2 | 4) {
                return Err(Unsupported(
                    "a saved Enter in a state the slice does not model",
                ));
            }
            let task = task_at(s.master_obj, base.short_val)?;
            unit.enter = Some(EnterAction {
                target: base.target_id,
                state: base.state,
                move_to,
                point: [base.target_x, base.target_y, base.target_z],
                range: task.work_range,
                timer: base.timer,
                retry: first_time != 0,
                task_class: task.object_group,
            });
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
            let move_wait = base
                .sub_actions
                .entries()
                .next()
                .and_then(|(_, sub)| sub.rge_action_by_type.as_deref())
                .and_then(|b| b.rge_action_move_to.as_deref())
                .map(|m| genie_core::move_to::MoveWait {
                    state: m.movement_wait_state,
                    timer: m.timer,
                    delay: f32::from_bits(m.k_default_wait_time),
                })
                .unwrap_or_default();
            unit.explore = Some(genie_core::explore::ExploreAction {
                state: base.state,
                point: [base.target_x, base.target_y, base.target_z],
                move_to,
                move_wait,
            });
            Ok(())
        }
        genie_core::attack::ATTACK => {
            unit.attack = Some(saved_attack(action, graphics, &move_to_state, run_sprite)?);
            // the attack reads and changes the unit's sprite from here on
            unit.sprite = graphics.and_then(|g| saved_sprite(s, g));
            Ok(())
        }
        genie_core::wait_solidify::WAIT_SOLIDIFY => {
            // TRIBE_Action_Wait_Solidify's load (FUN_140ae8220): the base action's state and
            // task, and the foundation it waits for (`mTargetBuildingId`, +0x68); it makes no
            // sub-action
            if base.sub_actions.entries().next().is_some() {
                return Err(Unsupported("a saved Wait_Solidify with a sub-action"));
            }
            let target = action
                .tribe_action_wait_solidify
                .as_deref()
                .ok_or(Unsupported("a saved Wait_Solidify without its own part"))?
                .target_building_id;
            let mut wait = genie_core::wait_solidify::WaitSolidify::new(
                task_at(s.master_obj, base.short_val)?,
                target,
            );
            wait.set_state(base.state);
            unit.wait_solidify = Some(wait);
            Ok(())
        }
        _ => Err(Unsupported("a saved action the slice does not import")),
    }
}

/// A saved unit's reload (the Strike's): its `mReloadStinger` plus its master's
/// `speed_of_attack` less its `mReloadPowerup`; None without a combat master.
fn saved_reload(
    object: &genie_save::Object,
    s: &RgeStaticObject,
    objects: &PlayerObjects,
) -> Option<f32> {
    let c = object.combat()?;
    let master = objects
        .masters
        .iter()
        .flatten()
        .map(|m| &m.master)
        .find(|m| m.static_part().is_some_and(|ms| ms.id() == s.master_obj))?;
    let rc = &master
        .tribe_master_combat_object
        .as_deref()?
        .rge_master_combat_object;
    Some(c.reload_stinger + (rc.speed_of_attack - c.reload_powerup))
}

/// The unit's current sprite as the save holds it: the node of its sprite list
/// (`temp_load_sprite_list`, `FUN_1413b87c0`) that shows its sprite (`+0x18`), an animated one
/// (type 2) with its frame and clock, a static one (type 1) with its frame; None without one or
/// for a graphic the table lacks.
fn saved_sprite(
    s: &RgeStaticObject,
    graphics: &std::collections::BTreeMap<i16, genie_core::sprite::Graphic>,
) -> Option<genie_core::sprite::ActiveSprite> {
    let list = s.active_sprite_list.as_deref()?;
    let graphic = graphics.get(&s.sprite).copied()?;
    let nodes = || {
        list.sprite_type_list
            .iter()
            .filter_map(|e| e.type_.as_deref())
    };
    let Some(node) = nodes()
        .filter_map(|n| n.rge_active_animated_sprite.as_deref())
        .find(|a| a.rge_active_sprite.sprite_id as i16 == s.sprite)
    else {
        let node = nodes()
            .filter_map(|n| n.rge_active_sprite.as_deref())
            .find(|a| a.sprite_id as i16 == s.sprite)?;
        return Some(genie_core::sprite::ActiveSprite::at_rest(
            graphic, node.frame,
        ));
    };
    Some(genie_core::sprite::ActiveSprite {
        graphic,
        frame: node.rge_active_sprite.frame,
        interval: node.animate_interval,
        last: node.animate_last,
        last_frame: node.last_frame as i16,
        changed: node.frame_changed != 0,
        looped: node.frame_looped != 0,
        animate: node.animate_flag,
        speed: node.last_speed_value,
    })
}

/// A saved Attack sub-action (type 9, `FUN_140a7c880`): its target, state, timer, ranges, point,
/// the target's last place (`+0xa4`..), a hit still to land (`+0x9e`), its `MoveTo`, and its sprites
/// and frames. The load restores the sprites at `+0x70`, `+0x80` and `+0x88` from the second to
/// fourth saved ids (the first goes to `+0x68`); the second attack sprite (`+0x78`) and the
/// swing's times (`+0xb0`, `+0xb4`) are not saved, and a loaded Attack starts without them.
fn saved_attack(
    sub: &genie_save::generated::world::Action,
    graphics: Option<&std::collections::BTreeMap<i16, genie_core::sprite::Graphic>>,
    move_to_state: &dyn Fn(u8) -> Result<genie_core::move_to::MoveTo, Unsupported>,
    run_sprite: i16,
) -> Result<genie_core::attack::Attack, Unsupported> {
    use genie_core::move_to::MOVE_TO;
    let b = sub
        .base()
        .ok_or(Unsupported("a saved Attack without its base"))?;
    let a = sub
        .rge_action_by_type
        .as_deref()
        .and_then(|t| t.rge_action_attack.as_deref())
        .ok_or(Unsupported("a saved Attack without its own part"))?;
    let graphics = graphics.ok_or(Unsupported("a saved Attack without the sprites' table"))?;
    let g = |id: u16| {
        let id = id as i16;
        (0 <= id).then(|| graphics.get(&id).copied()).flatten()
    };
    let (move_to, move_wait) = match b.sub_actions.entries().next() {
        None => (None, genie_core::move_to::MoveWait::default()),
        Some((MOVE_TO, s2)) => (
            Some(move_to_state(s2.base().map_or(0, |b| b.state))?),
            // its wait (`+0x6c`, `+0x70`, `+0x74`)
            s2.rge_action_by_type
                .as_deref()
                .and_then(|t| t.rge_action_move_to.as_deref())
                .map_or_else(genie_core::move_to::MoveWait::default, |m| {
                    genie_core::move_to::MoveWait {
                        state: m.movement_wait_state,
                        timer: m.timer,
                        delay: f32::from_bits(m.k_default_wait_time),
                    }
                }),
        ),
        Some(_) => return Err(Unsupported("a saved Attack with another sub-action")),
    };
    Ok(genie_core::attack::Attack {
        target: b.target_id,
        target2: b.target2_id,
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
        move_wait,
        pending: a.need_to_attack != 0,
        swing: 0.0,
        rest: 0.0,
        arms: genie_core::attack::Arms {
            move_sprite: g(a.short_val),
            sprite: g(a.short_val_2),
            second_sprite: None,
            idle_sprite: g(a.short_val_3),
            idle_attack_sprite: g(a.short_val_4),
            missile: a.missile_id,
            fire_frame: a.frame_delay,
            interrupt_frame: a.interrupt_animation_at_frame,
        },
        sub_action: b.sub_action_value != 0,
        repaths: b.target_moved_state != 0,
        // `+0x68`, its MoveTo's sprite
        runs: 0 <= run_sprite && a.short_val as i16 == run_sprite,
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
        .and_then(|m| m.rge_master.as_deref())
        .and_then(|r| r.rge_master_action_object.as_deref())
        .and_then(|a| a.tasks.as_deref());
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
    let saved = m.tribe_master_combat_object.as_deref().and_then(|c| {
        c.rge_master_combat_object
            .rge_master_action_object
            .tasks
            .as_deref()
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

/// A saved building's production (`TRIBE_Building_Object`'s queue, its production list's action
/// and its local lists, `FUN_1411df160`): every slot of the queue (`mProductionQueueSize` of them,
/// the stale ones past the count too) with its count, total and enabled flag; the load numbers the
/// slots' serials from 0 and leaves the building's next serial (`+0xcd8`) at the slot count; the
/// change counter (`+0xc00`) and `+0xc01` start at 0. The action: a unit's make action or a
/// research, each with its state and `+0x60` (the base's `subActionValue`). `Ok(None)`: a building
/// without slots, action or local lists.
fn production(
    object: &genie_save::Object,
) -> Result<Option<genie_core::production::Production>, Unsupported> {
    use genie_core::production::{Entry, MakeObj, MakeTech, Making, Production, Queue};
    let Some(building) = object.tribe_building_object.as_deref() else {
        return Ok(None);
    };
    let slots: Vec<Entry> = building
        .production_queue
        .iter()
        .enumerate()
        .map(|(k, e)| {
            let c = &e.production_queue_costs;
            let word = |k: usize| [c[k], c[k + 1], c[k + 2], c[k + 3]];
            let cost = |j: usize| {
                (
                    i32::from_le_bytes(word(8 * j)),
                    f32::from_le_bytes(word(8 * j + 4)),
                )
            };
            Entry {
                master: e.production_queue_master_unit_id,
                count: e.production_queue_unit_count,
                tech: e.production_queue_tech_id,
                costs: [cost(0), cost(1), cost(2)],
                serial: k as i64,
                button: e.production_queue_button_location_id,
                train_time: e.production_queue_train_time,
            }
        })
        .collect();
    let action = match building.production_queue_actions.entries().next() {
        None => None,
        Some((102, action)) => {
            let make = action
                .tribe_action_make_obj
                .as_deref()
                .ok_or(Unsupported("a saved make action without its part"))?;
            let base = action
                .base()
                .ok_or(Unsupported("a saved make action without its base"))?;
            Some(Making::Unit(MakeObj {
                state: base.state,
                paid: base.sub_action_value != 0,
                master: make.obj_id,
                progress: make.work_done,
                unique_id: make.unique_id,
                button: make.button_location_id,
                train_time: make.train_time,
            }))
        }
        Some((103, action)) => {
            let tech = action
                .tribe_action_make_tech
                .as_deref()
                .ok_or(Unsupported("a saved research action without its part"))?;
            let base = action
                .base()
                .ok_or(Unsupported("a saved research action without its base"))?;
            Some(Making::Research(MakeTech {
                state: base.state,
                paid: base.sub_action_value != 0,
                tech: tech.tech_id,
                master: tech.unique_id,
                progress: tech.m_research_progress,
            }))
        }
        Some(_) => {
            return Err(Unsupported(
                "a saved production action other than making a unit",
            ));
        }
    };
    let (disabled, available, researched) = (&building.value, &building.value_2, &building.value_3);
    if slots.is_empty()
        && action.is_none()
        && disabled.is_empty()
        && available.is_empty()
        && researched.is_empty()
    {
        return Ok(None);
    }
    let next_serial = slots.len() as i64;
    Ok(Some(Production {
        queue: Queue {
            slots,
            count: building.production_queue_count,
            total: building.production_queue_total_units,
            enabled: building.production_queue_enabled != 0,
            changes: 0,
            waiting: false,
            next_serial,
        },
        action,
        disabled_techs: disabled.clone(),
        researched_techs: researched.clone(),
        available_units: available.clone(),
    }))
}

/// A building's gather point, if it has one (`gather_point_exists`).
fn gather_point(object: &genie_save::Object) -> Option<genie_core::turn::GatherPoint> {
    let b = object
        .tribe_building_object
        .as_deref()
        .filter(|b| b.gather_point_exists != 0)?;
    Some(genie_core::turn::GatherPoint {
        point: [b.gather_point_x, b.gather_point_y, b.gather_point_z],
        target: b.gather_point_target,
        target_master: b.gather_point_target_master,
    })
}

/// A default task's action the slice models.
enum DefaultStart {
    /// The Artifact action (type 107, livestock's).
    Artifact(genie_core::artifact::Artifact),
    /// A bird's Fly (type 10) with its task.
    Fly(genie_core::masters::Task),
    /// A farm's or fish trap's Make (type 21).
    Make,
}

/// What an object's first update starts of its master's default task (`RGE_Action_Object::update`,
/// 0x14106ac90): only action objects (types 40 and up) have one. The default task is game data:
/// the table's, from the `.dat`, or the object's own master's. The slice models the Artifact action
/// (type 107, the default task of livestock) and a bird's Fly (type 10); any other default task is
/// reported.
fn default_task(
    object: &genie_save::Object,
    s: &RgeStaticObject,
    objects: &PlayerObjects,
    civ: u8,
    masters: Option<&MasterTable>,
) -> Result<Option<DefaultStart>, Unsupported> {
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
        DefaultAction::Type(ARTIFACT_TYPE) => Ok(Some(DefaultStart::Artifact(
            genie_core::artifact::Artifact::new(s.owner == 0),
        ))),
        DefaultAction::Type(genie_core::fly::FLY) => {
            fly_task(objects, civ, masters, s.master_obj).map(|t| Some(DefaultStart::Fly(t)))
        }
        DefaultAction::Type(genie_core::make::MAKE) => Ok(Some(DefaultStart::Make)),
        // the action list's factory (`FUN_140aca4b0`, then the RGE one `FUN_140a97e00`) makes
        // no action of the other types: the default task starts nothing (wild animals' Explore,
        // type 6)
        DefaultAction::Type(t) if !ACTION_FACTORY_TYPES.contains(&t) => Ok(None),
        DefaultAction::Type(_) => Err(Unsupported("a default task other than the Artifact action")),
        DefaultAction::NoSuchTask => Err(Unsupported("a default task id that names no task")),
    }
}

use genie_core::masters::ACTION_FACTORY_TYPES;

/// Whether the object runs `RGE_Action_Object::update`: types 40 (action), 50 (combat), 60
/// (missile), 70 (tribe combat) and 80 (building).
fn is_action_object(object: &genie_save::Object) -> bool {
    object.combat().is_some()
        || object.rge_object.as_deref().is_some_and(|r| {
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
    None.or_else(|| {
        s.rge_master_action_object
            .as_deref()
            .map(|a| a.default_task)
    })
    .or_else(|| {
        s.rge_master_missile_object.as_deref().map(|m| {
            m.own_rge_master_combat_object
                .own_rge_master_action_object
                .default_task
        })
    })
    .or_else(|| s.tribe_master_combat_object.as_deref().map(tribe))
    .or_else(|| {
        s.tribe_master_building_object
            .as_deref()
            .map(|b| tribe(&b.own_tribe_master_combat_object))
    })
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
        .as_deref()
        .is_some_and(|r| r.rge_static_object.is_some());
    let death_sprite = s.own_master().map_or_else(
        || {
            objects
                .master(s.master_obj)
                .and_then(|m| m.static_part())
                .map_or(-1, |m| m.death_sprite_id)
        },
        |own| own.death_sprite_num,
    );
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
    let (dying_sprite, [death_spawn, remains], master) = s.own_master().map_or_else(
        || {
            let player = objects.master(s.master_obj).and_then(|m| m.static_part());
            (
                player.map(|m| m.death_sprite_id).filter(|&d| d >= 0),
                player.map_or([-1, -1], |m| [m.death_spawn_obj_id, m.death_spawn_obj_id2]),
                masters.and_then(|t| t.get(civ, s.master_obj)),
            )
        },
        |own| {
            (
                (own.death_sprite_num >= 0).then_some(own.death_sprite_num),
                [own.death_spawn_obj_id, own.death_spawn_obj_id2],
                Some(MasterData {
                    decay: own.attribute_rot,
                    undead: own.undead_flag != 0,
                }),
            )
        },
    );
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
        inside: (s.inside_obj != -1).then_some(s.inside_obj),
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
        desired_distance: ai.desired_target_distance_value,
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
        formation_type: ai.formation_type,
        state_position: [ai.state_position_xvalue, ai.state_position_yvalue],
        retarget_disabled: ai.disable_retargeting,
        alert_mode: ai.alert_mode_value != 0,
        time_since_enemy: ai.time_since_enemy_sighting,
        // the saved patrol (its path's points in the game's layout, length and flags)
        patrol_path: ai
            .patrol_path
            .as_deref()
            .map(|p| genie_core::unit_ai::PatrolPath {
                path: genie_core::tile_path::Path {
                    points: p.m_waypoints.iter().map(vec3).collect(),
                    length: p.path_length_value,
                },
                flags: p.flags_m_bits,
            }),
        patrol_waypoint: ai.patrol_current_waypoint,
        stop_after_kill: ai.stop_after_target_killed_value != 0,
        home: combat
            .tribe_elephant_unit_ai_module
            .as_deref()
            .map(|e| genie_core::unit_ai::Home {
                origin: [e.origin_x, e.origin_y, e.origin_z],
                furious: e.furious != 0,
                go_home: false,
            }),
        hunted: None,
        last_update_time: ai.last_update_time_value,
        attackers: ai.temp_int.clone(),
        orders: ai
            .temp_order
            .iter()
            .map(genie_core::unit_ai::QueuedOrder::from_bytes)
            .collect(),
        building_orders: combat
            .tribe_tower_unit_ai_module
            .as_deref()
            .map(|t| &t.saved_order)
            .or_else(|| {
                combat
                    .tribe_building_unit_ai_module
                    .as_deref()
                    .map(|b| &b.saved_order)
            })
            .map(|orders| {
                orders
                    .iter()
                    .map(genie_core::unit_ai::QueuedOrder::from_bytes)
                    .collect()
            })
            .unwrap_or_default(),
        notifies: ai
            .temp_notify
            .iter()
            .map(genie_core::unit_ai::Notify::from_bytes)
            .collect(),
        // the load's factory capacity, doubled for what it reads
        notify_capacity: genie_core::unit_ai::notify_capacity(
            combat.need_unit_ai as u32,
            ai.temp_notify.len(),
        ),
        processing: false,
        buffered: Vec::new(),
        commanded: false,
    })
}

/// A player's saved state of one technology (`Tech_Player_Tree`).
fn tech_state(t: &genie_save::generated::world::TechPlayerTree) -> genie_core::tech::TechState {
    let o = |j: usize| {
        t.cost_override.get(j).map_or((-1, -1, -2), |e| {
            (
                e.tech_player_tree_cost_amount_override as i16,
                e.tech_player_tree_cost_attribute_type_override as i16,
                e.tech_player_tree_cost_attribute_used_override as i16,
            )
        })
    };
    genie_core::tech::TechState {
        research_done: t.research_done,
        state: t.state,
        cost_adj: [0, 1, 2].map(|j| t.cost_adj.get(j).copied().unwrap_or(0)),
        times_researched: t.tech_player_tree_times_researched as i16,
        allow_multiple: t.tech_player_tree_allow_multiple_research != 0,
        stack: t.tech_player_tree_stack_tech != 0,
        overrides: [o(0), o(1), o(2)],
        research_cap: t.tech_player_tree_research_cap as i16,
        pending: t.tech_player_tree_pending_count as i16,
        enabled: t.tech_player_tree_enabled_tech != 0,
        effect_override: t.tech_player_tree_effect_override as i16,
        building_master: t.tech_player_tree_research_building_master_id as i16,
        locations: t
            .location
            .iter()
            .flatten()
            .map(|l| genie_core::tech::Location {
                building: l.location_research_location_id as i16,
                time: l.location_research_time,
                button: l.location_button_location_id,
                adjustment: f32::from_bits(l.location_research_adjustment),
            })
            .collect(),
    }
}

/// Why the slice stops on a command it does not decode, by the command's name.
const fn unmodelled_command(p: &de::command::Command) -> &'static str {
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

/// The slice set up from the header save: every player's live objects in their saved order, and
/// `BaseWorld`'s fields before the map, the frame seed (`mCurrentFrameRandomSeed`, world+0x7d0) and
/// the turn order (`player_turn`, `player_time_delta[i]`).
///
/// Only for a save whose run-time state then comes from the tick oracle's dumps (a per-tick save:
/// the registry, tiles, mesh owners …). A run from a recording's header takes [`header_slice`]:
/// it adds what the game's own load builds, which the save does not hold (an empty
/// objects-in-sight registry, FD-008-D72; the hunted animals' origins; the watched lists).
/// Without it every unit-AI search stops as Unsupported: [`simulate_word9`] once used this and
/// lost MarvinBlue/426048731 player 2's first checkpoint that way.
fn slice(header: &Header, masters: Option<&MasterTable>) -> Turns {
    let victory = &header.world.base_world.victory_conditions;
    let players = header
        .players()
        .iter()
        .enumerate()
        .map(|(i, p)| {
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
                kind: p.player.type_,
                resources: [resource(0), resource(1), resource(2), resource(3)],
                construction_rate: resource(CONSTRUCTION_RATE),
                attributes: attributes.clone(),
                civ,
                relations: p.world_player().base.relation.clone(),
                auto_farm_queue: p.world_player().auto_farm_queue != 0,
                auto_fishtrap_queue: p.world_player().auto_fish_trap_queue != 0,
                formations: genie_core::turn::Formations {
                    auto: p.world_player().base.auto_formations != 0,
                    influence: p.world_player().base.formation_influence_distance,
                    by_speed: p.world_player().base.break_auto_formations_by_speed == 1,
                    min_column_distance: p.world_player().base.min_column_distance,
                    column_to_line_distance: p.world_player().base.column_to_line_distance,
                },
                pending_debit: p.world_player().base.pending_debit,
                techs: p
                    .world_player()
                    .tech_tree
                    .tech_player_tree
                    .iter()
                    .map(tech_state)
                    .collect(),
                unsupported: None,
                freezes: Vec::new(),
                stopped: false,
                game_status: p.world_player().base.game_status,
                // `+0x8b` holds the drop in its bit 1 (the load moves it to `+0x8c`)
                resigned: p.world_player().base.resigned & 2 == 0
                    && p.world_player().base.resigned != 0,
                dropped: p.world_player().base.resigned & 2 != 0,
                allied_victory: p.world_player().base.allied_victory != 0,
                allied_vision: p.world_player().base.allied_los_enable != 0,
                // the reveal of the enemies' town centres (attribute 209) is done once any
                // mTCFlared bit is set; the saved map then already holds the revealed tiles
                tc_flared: p.world_player().tcflared.iter().map(|&b| b != 0).collect(),
                sharing: p.world_player().sharing_is_caring != 0,
                victory: victory.get(i).map_or(0, |v| v.victory),
                fealty: p.world_player().fealty as i16,
                // the load sets it 0 (`FUN_14163e970`)
                posture: 0,
                // the load builds the watched lists (Turns::build_watched)
                watched: Vec::new(),
                ghosts: saved_ghosts(p),
                history: history(&p.world_player().history),
                // the load files the objects (Turns::load_trackers, once the masters are in)
                tracker: genie_core::tracker::Tracker::default(),
                // +0x1378 is not saved
                one_military_queued: false,
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
    turns.recycling.next_id = world.next_reusable_object_id;
    let none = TerrainTable::default();
    let terrain = masters.map_or(&none, |t| &t.terrain);
    turns.spatial = spatial(header, terrain);
    // the players' scout maps (`mIsScoutMapEnabled` and its InfluenceMap, player+0x1370)
    for (i, p) in header.players().iter().enumerate() {
        let wp = p.world_player();
        if wp.is_scout_map_enabled.unwrap_or_default() == 0 {
            continue;
        }
        if let Some(m) = wp.tribe_information_aimodule.as_deref() {
            let width = m.x_size_value.max(0) as usize;
            let cells = m
                .row_value
                .iter()
                .flat_map(|r| {
                    let row = r.row_value.as_deref().unwrap_or(&[]);
                    (0..width).map(move |x| row.get(x).copied().unwrap_or(0))
                })
                .collect();
            turns.spatial.set_scout_map(
                i,
                genie_core::explore::InfluenceMap {
                    size: [m.x_size_value, m.y_size_value],
                    origin: [m.x_reference_point_value, m.y_reference_point_value],
                    limit: m.unchangeable_limit_value,
                    cells,
                },
            );
        }
    }
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
    // the zone maps (`map+0x5e28`) as the save holds them, their adjacency built by the load
    turns.zones = genie_core::zones::Zones::from_saved(
        map.map_zones
            .zone_map
            .iter()
            .map(|z| {
                genie_core::zones::ZoneMap::from_saved(
                    map.map_width,
                    map.map_height,
                    &z.number_tiles_in_zone_value,
                    &z.zone_map,
                    &z.float_value,
                    z.number_zones,
                )
            })
            .collect(),
    );
    // the players' path caps (saved); their plans start over at each player's update
    turns.pathing.budgets = header
        .players()
        .iter()
        .map(|p| {
            let b = &p.world_player().base;
            genie_core::pathing::PlanBudget {
                attempt_cap: b.pathing_attempt_cap_value,
                delay_cap: b.pathing_delay_cap_value,
                plans: 0,
            }
        })
        .collect();
    turns.settings = settings(&header.file_header.game_options_read.game_options);
    // the Empire Wars start's pass, once in the game's first update (the world's +0x6c6)
    turns.start_work_due =
        turns.settings.game_mode == genie_core::start_work::EMPIRE_WARS && turns.world_time == 0;
    turns.game_state = u32::from(world.temp_game_state);
    turns.victory_conquest = world.tribe_scenario.base_scenario.victory_conquest != 0;
    match groups::unit_groups(world) {
        Ok(g) => turns.groups = g,
        Err(e) => {
            for p in &mut turns.players {
                let _ = p.unsupported.get_or_insert((0, 0, e));
                p.stopped = true;
            }
        }
    }
    if let Some(table) = masters {
        turns.drop_sites.clone_from(&table.drop_sites);
        turns.linked_techs.clone_from(&table.linked_techs);
        turns.graphics = table.graphics.clone();
        turns.techs.clone_from(&table.techs);
        turns.history_period = table.time_slice;
        // the unit lines' tables, which the game builds at its first load with Gaia's master
        // count (`FUN_141533cb0` -> `FUN_140fde680`)
        let gaia_masters = header
            .players()
            .first()
            .map_or(0, |p| p.objects().master_object_num);
        turns.lines = genie_core::lines::UnitLines::build(gaia_masters, &table.unit_lines);
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
                .flat_map(genie_core::masters::Masters::iter)
                .filter(|m| m.kind == 70)
        };
        turns.spatial.fastest = all()
            .filter_map(|m| m.moving.as_ref().map(|s| s.speed))
            .fold(0.0, f32::max);
        turns.spatial.reach = all()
            .flat_map(|m| m.tasks.iter().map(|t| t.work_range).chain([m.weapon_range]))
            .fold(0.0, f32::max);
        garrison_firepower(&mut turns);
        // the planners the load makes and the shape mesh it builds (a dump of the game's mesh
        // replaces it)
        turns.register_planners();
        turns.build_mesh();
    }
    // the players' object trackers, which the objects' loads fill
    turns.load_trackers();
    // the players' vision masks, which the world's load rebuilds (its slot 0x78)
    if let Err(e) = turns.rebuild_vision() {
        for p in &mut turns.players {
            let _ = p.unsupported.get_or_insert((0, 0, e));
            p.stopped = true;
        }
    }
    turns
}

/// A player's ghosts (its doppelganger list) as saved: each one's id, place, state, the original's
/// id (`long_val`), owner (`doppled_player`) and UMV entry (`Multi_Unified_Points`), and the
/// players who cannot see it (`CantSeeBits`). Its last bits (`+0x7e8`) and seen flag (`+0x7f7`)
/// are not saved: a load starts them at 0; its original's link waits for the original's update
/// (the slice links it by id).
/// The game options the simulation reads ([`genie_core::settings::Settings`]) from the header's
/// `GameOptions`, as `FUN_141305310` loads them.
#[must_use]
pub const fn settings(
    o: &genie_save::generated::world::GameOptions,
) -> genie_core::settings::Settings {
    genie_core::settings::Settings {
        game_mode: o.game_mode,
        full_tech_tree: o.full_tech_tree_enabled != 0,
        walkable_farms: o.walkable_farms != 0,
        selected_content_mode: o.selected_content_mode,
        villager_force_drop: o.villager_force_drop != 0,
        cheats_enabled: o.cheats_enabled != 0,
    }
}

/// A player's history as saved (`TRIBE_History_Info::load`, `FUN_141560f20`): the entries, the
/// events in the list's order (the load links each after the last), and the running totals'
/// low 16 bits into the zeroed ints.
#[must_use]
pub fn history(h: &genie_save::generated::world::History) -> genie_core::history::History {
    genie_core::history::History {
        entries: h
            .history
            .iter()
            .map(|e| [e.history_civilian_pop, e.history_military_pop])
            .collect(),
        max_entries: h.max_number_of_entries,
        events: h
            .events
            .iter()
            .map(|e| genie_core::history::Event {
                kind: e.temp_event,
                time_slice: e.temp_time_slice,
                world_time: e.temp_world_time,
                data1: e.temp_data1,
                data2: e.temp_data2,
                data3: e.temp_data3,
            })
            .collect(),
        number_of_events: h.number_of_events,
        update_count: h.update_history_count,
        old_total_razings: h.old_total_razings,
        old_build_value_razings: h.old_build_value_razings,
        old_total_razings_other: h.old_total_razings_other,
        old_build_value_razings_other: h.old_build_value_razings_other,
        old_total_kills: h.old_total_kills,
        old_build_value_kills: h.old_build_value_kills,
        old_total_kills_other: h.old_total_kills_other,
        old_build_value_kills_other: h.old_build_value_kills_other,
        razings_weight: h.razings_weight,
        kills_weight: h.kills_weight,
        razing_percent: h.razing_percent,
        kills_percent: h.kills_perecent,
        razings_flag: h.razings_flag,
        battle_flag: h.battle_flag,
        old_current_build_value_units: h.old_current_build_value_units as i32,
        old_current_build_value_buildings: h.old_current_build_value_buildings as i32,
        old_player_kill_bv: h.old_player_kill_bv.iter().map(|&v| v as i32).collect(),
        old_player_razing_bv: h.old_player_razing_bv.iter().map(|&v| v as i32).collect(),
        running_ave_bv_percent: h.running_ave_bv_percent,
        running_total_bv_kills: h.running_total_bv_kills as i32,
        running_total_bv_razings: h.running_total_bv_razings as i32,
        running_total_kills: i32::from(h.running_total_kills),
        running_total_razings: i32::from(h.running_total_razings),
    }
}

fn saved_ghosts(section: &PlayerSection<'_>) -> Vec<genie_core::ghost::Ghost> {
    section
        .objects()
        .doppleganger_objects
        .objects()
        .filter_map(|o| {
            let s = o.static_part()?;
            let d = o
                .rge_object
                .as_deref()?
                .rge_doppleganger_object
                .as_deref()?;
            let original = d.long_val as i32;
            Some(genie_core::ghost::Ghost {
                id: s.id,
                original: (original != -1).then_some(original),
                original_id: original,
                original_master: (d.long_val_2 as i32 != -1).then_some(d.long_val_2 as i16),
                original_owner: usize::try_from(d.doppled_player).unwrap_or(0),
                position: [s.t_x, s.t_y, s.t_z],
                tile: [s.t_x as i32, s.t_y as i32],
                footprint: d.multi_unified_points,
                bits: 0,
                cant_see: d.cant_see_bits,
                seen: false,
                state: s.object_state,
            })
        })
        .collect()
}

/// The garrisons of the running game: each object inside another of its player is in that one's
/// list (`+0x2f8`), in the order the objects are listed (the game's is the order they came in,
/// which no save holds); and each unit (type 70) added its share to a combat object's volleys at
/// its enter (`FUN_14120b790`, [`genie_core::garrison::share`]), which the load does not restore
/// (`FUN_14121ec90` sets the master's volley fire). A share the slice does not model leaves the
/// building unmodelled.
fn garrison_firepower(turns: &mut Turns) {
    for (p, player) in turns.players.iter_mut().enumerate() {
        let Some(masters) = turns.masters.get(p) else {
            continue;
        };
        let units: Vec<(i32, i32, i16, f32)> = player
            .objects
            .iter()
            .chain(&player.sleeping)
            .filter_map(|u| Some((u.id, u.life.inside?, u.life.master_id, u.reload_powerup)))
            .collect();
        for (id, building, master, reload_powerup) in units {
            let Some(b) = player
                .objects
                .iter_mut()
                .chain(&mut player.sleeping)
                .find(|u| u.id == building)
            else {
                continue;
            };
            b.garrison.push(id);
            let Some(me) = masters.get(master).filter(|m| m.kind == 70) else {
                continue;
            };
            let Some(bm) = masters.get(b.life.master_id) else {
                continue;
            };
            let rate = genie_core::garrison::attack_rate(bm, masters, b.reload_powerup);
            match genie_core::garrison::share(me, masters, reload_powerup, rate) {
                Ok(share) => b.firepower += share,
                Err(e) => b.unmodelled_action = Some(e),
            }
        }
    }
}

/// The facets every sprite a live object of a saved master shows ([`genie_core::facing::SpriteFacets`]):
/// its standing, second standing, walking, running and attack sprites and its tasks' four, those
/// the data file has, all of one count and bit 1 of the sequence flags; None when they differ or
/// it has none. The dying sprite shows once the object is dead, where nothing turns it.
fn master_facets(
    m: &Master,
    s: &genie_save::generated::world::RgeMasterStaticObject,
    table: &MasterTable,
) -> Option<genie_core::facing::SpriteFacets> {
    let mut sprites = vec![s.idle_sprite_id, s.idle2_sprite_id];
    if let Some(mv) = moving_master(m) {
        sprites.extend([mv.move_sprite_id, mv.run_sprite_id]);
    }
    if let Some(c) = combat_master(m) {
        sprites.push(c.fight_sprite_id);
    }
    if let Some(tasks) = action_part(m).and_then(|a| a.tasks.as_deref()) {
        for e in &tasks.list {
            let t = &e.task;
            sprites.extend([t.short_val, t.short_val_2, t.short_val_3, t.short_val_4]);
        }
    }
    let mut seen = sprites
        .into_iter()
        .filter(|&id| 0 <= id)
        .filter_map(|id| table.graphics.get(&id))
        .map(|g| genie_core::facing::SpriteFacets {
            count: g.facets,
            flag_1: g.flags & 2 != 0,
        });
    let first = seen.next()?;
    seen.all(|f| f == first).then_some(first)
}

/// A master's action part, whatever class it is: what holds its work rate, search radius and
/// task list.
fn action_part(m: &Master) -> Option<&genie_save::generated::world::RgeMasterActionObject> {
    None.or_else(|| {
        m.tribe_master_combat_object
            .as_deref()
            .map(|c| &c.rge_master_combat_object.rge_master_action_object)
    })
    .or_else(|| {
        m.tribe_master_building_object.as_deref().map(|b| {
            &b.tribe_master_combat_object
                .rge_master_combat_object
                .rge_master_action_object
        })
    })
    .or_else(|| {
        m.rge_master.as_deref().and_then(|r| {
            r.rge_master_action_object.as_deref().or_else(|| {
                r.rge_master_combat_object
                    .as_deref()
                    .map(|c| &c.rge_master_action_object)
            })
        })
    })
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
        .and_then(|a| a.tasks.as_deref());
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

const fn core_task(t: &genie_save::generated::world::Task) -> genie_core::masters::Task {
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
        work_flag: t.work_flag,
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
        work.height = s.radius_z;
        work.obstruction_type = s.obstruction_type;
        work.buildable_on = s.can_be_built_on;
        work.obstruction_class = s.obstruction_class;
        work.object_flags = s.object_flags;
        work.hit_points = s.read_temp_6;
        work.available = s.available != 0;
        work.disabled = s.disabled != 0;
        work.line_of_sight = s.los;
        work.capacity = s.attribute_max_amount;
        work.blast_defense = s.area_effect_object_level;
        if let Some(a) = action_part(m) {
            work.work_rate = a.work_rate;
            work.search_radius = a.search_radius;
            if let Some(tasks) = a.tasks.as_deref() {
                work.tasks = tasks.list.iter().map(|e| task(&e.task)).collect();
            }
        }
        let combat = m.tribe_master_combat_object.as_deref().or_else(|| {
            m.tribe_master_building_object
                .as_deref()
                .map(|b| &b.tribe_master_combat_object)
        });
        if let Some(c) = combat {
            work.cost = build_cost(&c.build_inventory);
            work.build_time = c.train_location.first().map_or(0, |t| t.train_time);
            work.trained_at = c
                .train_location
                .iter()
                .map(|t| t.train_location_id)
                .collect();
            work.charge_type = c.charge_type;
            work.formation_category = c.temp_formation_category;
            work.hero_flag = c.hero_flag;
            work.volley_fire = c.volley_fire_amount;
            work.max_volley = c.max_attacks_in_volley;
            work.volley_spread = [
                c.volley_x_spread,
                c.volley_y_spread,
                c.volley_start_spread_adjustment,
            ];
            let id = |id: i32| i16::try_from(id).ok().filter(|&id| 0 <= id);
            work.volley_missile = id(c.volley_missile_id);
            work.charge_projectile = id(c.charge_projectile_unit);
            work.invulnerability = c.invulnerability_level;
            work.attack_priority = c.attack_priority;
            work.special_attack = c.special_attack_flag;
            work.max_charge = c.max_charge;
            work.charge_target = c.charge_target;
        }
        // the combat part a missile master has too (`RGE_Master_Missile_Object`'s base)
        let rge_combat = combat.map(|c| &c.rge_master_combat_object).or_else(|| {
            m.rge_master
                .as_deref()
                .and_then(|r| r.rge_master_missile_object.as_deref())
                .map(|mm| &mm.rge_master_combat_object)
        });
        if let Some(rc) = rge_combat {
            work.weapon_range = rc.weapon_range;
            work.min_range = rc.minimum_weapon_range;
            work.missile = rc.missile_id;
            work.weapon_offset = [rc.weapon_offset_x, rc.weapon_offset_y, rc.weapon_offset_z];
            work.missed_spread = rc.missed_missile_spread;
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
            work.garrison_firepower = rc.garrison_firepower;
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
            m.tribe_master_building_object.as_deref(),
        ) {
            b.foundation_terrain = saved.on_build_make_tile;
        }
        // a missile master's flight: the save's three bytes and arc, the data file's projectile
        // type
        if let Some(mm) = m
            .rge_master
            .as_deref()
            .and_then(|r| r.rge_master_missile_object.as_deref())
        {
            work.missile_stats = Some(genie_core::missile::MissileStats {
                projectile_type: table.projectile_type(civ, s.id()).unwrap_or(0),
                smart_mode: mm.targetting_type,
                hit_mode: mm.missile_hit_info,
                vanish_mode: mm.missile_die_info,
                arc: mm.ballistics_ratio,
                speed: animated_master(m).map_or(0.0, |a| a.max_speed),
                radius: [s.radius_x, s.radius_y, s.radius_z],
                fly_mode: table.fly_mode(civ, s.id()).unwrap_or(0),
            });
        }
        work.damage_sprites = !s.damage_sprite.is_empty();
        // what the masters' attribute setters change: the top and turn speeds (+0x740,
        // +0x758), the damage sprites (+0x638) and the train locations (+0x898)
        work.max_speed = animated_master(m).map_or(0.0, |a| a.max_speed);
        work.turn_speed = moving_master(m).map_or(0.0, |mv| mv.turn_speed);
        work.damage_graphics = s
            .damage_sprite
            .iter()
            .map(|d| genie_core::masters::DamageGraphic {
                sprite: d.temp_id,
                percent: d.sprite_info_damage_percent,
                apply_mode: d.sprite_info_flag,
            })
            .collect();
        work.train_locations = m
            .tribe_master_building_object
            .as_deref()
            .map(|b| &b.tribe_master_combat_object)
            .or(m.tribe_master_combat_object.as_deref())
            .map_or_else(Vec::new, |c| {
                c.train_location
                    .iter()
                    .map(|t| genie_core::masters::TrainLocation {
                        location: t.train_location_id,
                        time: t.train_time,
                        button: t.button_location_id,
                        hotkey: t.hotkey_id,
                    })
                    .collect()
            });
        work.fly_mode = table.fly_mode(civ, s.id()).unwrap_or(0);
        work.standing_sprite = s.idle_sprite_id;
        work.standing_sprite_2 = (0 <= s.idle2_sprite_id).then_some(s.idle2_sprite_id);
        (work.move_sprite, work.run_sprite) =
            moving_master(m).map_or((-1, -1), |mv| (mv.move_sprite_id, mv.run_sprite_id));
        work.death_sprite = s.death_sprite_id;
        work.garrison = s.obj_max;
        work.death_spawns = [s.death_spawn_obj_id, s.death_spawn_obj_id2];
        work.terrain_restriction = s.terrain_restriction_mask_id;
        work.life = table.get(civ, s.id());
        work.moving = moving_stats(m, s, civ, table);
        Some(work)
    }))
}

/// What a unit of a saved master moves with ([`genie_core::movement::MoveStats`]): the master's
/// top speed times its walking sprite's speed factor when above 0 (the run sprite if it has one,
/// else the move sprite: the `MoveTo`'s, slots 2000 and 0x7c8), the `.dat`'s turning, the radii and
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
        fly_mode: table.fly_mode(civ, s.id()).unwrap_or(0),
        speed: if 0.0 < factor { base * factor } else { base },
        turn_rate: genie_core::movement::turn_rate(moving.turn_speed, facets),
        turn_radius: 0.0,
        turn_radius_speed: f32::MAX,
        radius: [s.radius_x, s.radius_y, s.radius_z],
        restriction,
        restriction_mask: *table.terrain.masks().get(restriction)?,
        turn_time: moving.turn_speed,
        move_algorithm: turning.move_algorithm,
        min_collision: turning.min_collision_size_multiplier,
        fast_turn: s.object_flags & 2 != 0,
        walk_speed: {
            let f = table.sprite_speed(moving.move_sprite_id).unwrap_or(0.0);
            if 0.0 < f { base * f } else { base }
        },
        run_speed: {
            let id = if 0 <= moving.run_sprite_id {
                moving.run_sprite_id
            } else {
                moving.move_sprite_id
            };
            let f = table.sprite_speed(id).unwrap_or(0.0);
            if 0.0 < f { base * f } else { base }
        },
        stand_speed: {
            let f = table.sprite_speed(s.idle_sprite_id).unwrap_or(0.0);
            if 0.0 < f { base * f } else { base }
        },
        facets: master_facets(m, s, table),
        weapon_range: combat_master(m).map_or(0.0, |c| c.weapon_range),
    })
}

/// A combat master's `build_inventory` entries `(resource, amount, flag)`, three shorts each; the
/// flag set means paid. Entries of resource -1 are empty.
fn build_cost(inventory: &[[u8; 6]]) -> Vec<(i16, i16, bool)> {
    build_entries(inventory)
        .into_iter()
        .filter(|&(kind, ..)| kind >= 0)
        .collect()
}

/// A combat master's `build_inventory` entries `(resource, amount, paid)`, the empty ones
/// (resource −1) kept in their places.
fn build_entries(inventory: &[[u8; 6]]) -> Vec<(i16, i16, bool)> {
    inventory
        .iter()
        .map(|e| {
            let kind = i16::from_le_bytes([e[0], e[1]]);
            let amount = i16::from_le_bytes([e[2], e[3]]);
            let flag = i16::from_le_bytes([e[4], e[5]]);
            (kind, amount, flag != 0)
        })
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
/// against the game's obstruction manager, dumped by the tick oracle.
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
    let mut t = genie_core::terrain::Terrain::new(
        map.map_width,
        map.map_height,
        tiles,
        terrain.kinds().to_vec(),
        terrain.passable().to_vec(),
    );
    t.set_elevations(
        map.rows
            .iter()
            .flat_map(|r| r.tiles.iter().map(|t| t.t_m_elevation))
            .collect(),
    );
    t
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
    /// Each player's ghosts (type 25) in its list's order.
    ghosts: Vec<Vec<genie_core::ghost::Ghost>>,
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
        Ok(Self {
            objects,
            ghosts: header.players().iter().map(saved_ghosts).collect(),
            seen: saved_sight(&header),
            width,
            terrain,
            game: game_state(&header, masters),
        })
    }
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
    // what each player's queries make of the others' objects (player +0x278, the load's
    // unit_diplomacy)
    out.unit_diplomacy = header
        .players()
        .iter()
        .map(|p| p.world_player().base.unit_diplomacy.clone())
        .collect();
    let mut sight = saved_sight(header);
    for (player, section) in header.players().iter().enumerate() {
        let all = section.live_objects().chain(section.sleeping_objects());
        for o in all {
            let Some(s) = o.static_part() else { continue };
            let Some(m) = placement(s, section.objects()) else {
                continue;
            };
            let building = o.tribe_building_object.as_deref();
            // a building's place links its tiles (FUN_1411e6780), an annex's does not, and a
            // foundation made desolid lost them (FUN_1411cf770); so do a combat unit's that does
            // not move and is more than a tile wide (FUN_14108c320). A building keeps its
            // footprint's UMV entry (+0x904)
            let place = [s.t_x, s.t_y, s.t_z];
            let footprint = [m.radius[0], m.radius[1]];
            let speed = moving_speed(o, s, section.objects());
            let links = building.map_or_else(
                || o.combat().is_some() && speed == 0.0 && (0.5 < m.radius[0] || 0.5 < m.radius[1]),
                |b| b.mupt_linked_object_references_linked_owner_id == -1 && b.desolid_flag == 0,
            );
            let umv = match (links, building.is_some()) {
                (true, true) => sight.link_building(place, footprint),
                (true, false) => {
                    let _ = sight.link_building(place, footprint);
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
                .as_deref()
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
            // an object inside another left its tile's list and the obstruction manager
            // (FUN_1411921d0)
            if s.inside_obj != -1 {
                out.go_inside(s.id, [s.t_x, s.t_y, s.t_z]);
            }
        }
        for ghost in section.objects().doppleganger_objects.objects() {
            if let Some(s) = ghost.static_part() {
                out.insert_ghost(s.id, [s.t_x, s.t_y, s.t_z]);
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
    let mut sight = Visibility::new(
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
    );
    sight.resources = players
        .iter()
        .map(|p| saved_resources(&p.objects().visible_resource_manager))
        .collect();
    sight
}

/// A player's saved `Visible_Resource_Manager` as [`VisibleResources`] holds it.
fn saved_resources(m: &genie_save::generated::world::VisibleResourceManager) -> VisibleResources {
    VisibleResources {
        capacities: m
            .list_sizes
            .iter()
            .flatten()
            .map(|s| s.vr_list_size)
            .collect(),
        lists: m
            .lists
            .iter()
            .flatten()
            .map(|l| {
                l.visible_resource_rec
                    .iter()
                    .map(|r| VisibleResource {
                        id: r.object_id,
                        distance: r.distance,
                        zone: r.zone,
                        x: r.pos_x,
                        y: r.pos_y,
                    })
                    .collect()
            })
            .collect(),
    }
}

/// The players' visible maps and the unified map of a save, as [`Visibility`] holds them.
pub fn saved_visibility(save: &[u8]) -> Result<Visibility, Error> {
    let chapter = de::chapters(save)?[0];
    let header = chapter.header()?;
    Ok(saved_sight(&header))
}

/// Where player `player`'s sight in `slice` differs from `game`'s: its explored count, each tile's
/// count and its two bits of the unified map.
#[must_use]
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
    out.extend(resource_differences(slice, game, player));
    let w = slice.width.max(1) as usize;
    let bits = 1u64 << player | 1u64 << (32 + player);
    let unified_same = || {
        slice
            .unified
            .iter()
            .zip(&game.unified)
            .all(|(u, v)| u & bits == v & bits)
    };
    // the usual case, every tile the same, without a look at each tile's place
    if a.counts == b.counts && unified_same() {
        return out;
    }
    for (i, (&c, &g)) in a.counts.iter().zip(&b.counts).enumerate() {
        let (u, v) = (slice.unified[i] & bits, game.unified[i] & bits);
        if c == g && u == v {
            continue;
        }
        let (x, y) = (i % w, i / w);
        if c != g {
            out.push(format!(
                "player {player} tile ({x}, {y}) count {c}, the game {g}"
            ));
        }
        if u != v {
            out.push(format!(
                "player {player} tile ({x}, {y}) unified bits {u:#x}, the game {v:#x}"
            ));
        }
    }
    out
}

/// Where player `player`'s visible-resource lists in `slice` differ from `game`'s: their count,
/// each list's capacity and its first record that differs (every field, the floats by their
/// bits), or its length.
fn resource_differences(slice: &Visibility, game: &Visibility, player: usize) -> Vec<String> {
    let (Some(a), Some(b)) = (slice.resources.get(player), game.resources.get(player)) else {
        return vec![format!("player {player} has no visible-resource lists")];
    };
    let mut out = Vec::new();
    if a.lists.len() != b.lists.len() {
        out.push(format!(
            "player {player} holds {} visible-resource lists, the game {}",
            a.lists.len(),
            b.lists.len()
        ));
    }
    if a.capacities != b.capacities {
        out.push(format!(
            "player {player} visible-resource capacities {:?}, the game {:?}",
            a.capacities, b.capacities
        ));
    }
    let bits = |r: &VisibleResource| {
        (
            r.id,
            r.distance.to_bits(),
            r.zone,
            r.x.to_bits(),
            r.y.to_bits(),
        )
    };
    for (g, (x, y)) in a.lists.iter().zip(&b.lists).enumerate() {
        if let Some(k) = x.iter().zip(y).position(|(r, s)| bits(r) != bits(s)) {
            out.push(format!(
                "player {player} visible resources {g}: record {k} {:?}, the game {:?}",
                x[k], y[k]
            ));
        } else if x.len() != y.len() {
            out.push(format!(
                "player {player} visible resources {g}: {} records, the game {}",
                x.len(),
                y.len()
            ));
        }
    }
    out
}

/// One frame of the body (FD-007's per-frame record order): the op-2 record that starts a tick, its
/// dt and, every interval, a checkpoint, then the op-1 commands the tick runs after its `SeedAll`.
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
                let _ = self.command(p, sections);
            }
        }
    }

    fn command(&mut self, packet: &[u8], sections: &[PlayerSection<'_>]) -> Option<Command> {
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
                    control: c.field_0x12 != 0,
                };
                let order = genie_core::turn::Order::Move(order);
                (c.commp_id, c.num, c.ids, true, Some(order))
            }
            Packet::Retreat(c) => (c.commp_id, c.num, c.ids, true, None),
            // FOLLOW: its units as MOVE's, the target at +8
            Packet::Follow(c) => {
                let order = genie_core::turn::Order::Follow(c.target);
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
            // GAME's toggles of the automatic farm and fish trap queues; the player is at +8
            Packet::Game(g) if matches!(g.field_0x04, 0x10 | 0x13) => {
                return Some(Command::AutoQueue {
                    player: usize::try_from(g.field_0x08).ok()?,
                    fishtrap: g.field_0x04 == 0x13,
                });
            }
            // GAME's toggle of the offensive posture; the player is at +8
            Packet::Game(g) if g.field_0x04 == 0x14 => {
                return Some(Command::Posture {
                    player: usize::try_from(g.field_0x08).ok()?,
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

/// PATROL (0x15): the units of the count at `+0x4` from `+0x5c`, the waypoints of the count
/// at `+0xb` (x from `+0xc`, y from `+0x34`, each an f32's bits), `+0x8`. None where the
/// command does nothing: a count past the ids the packet holds (the check `FUN_140e55590`),
/// no waypoints or more than 10 (`FUN_140e69cc0`).
fn patrol_command(c: &de::command::Patrol) -> Option<Command> {
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
fn queue_command(packet: &de::command::MultiQueue, sections: &[PlayerSection<'_>]) -> Command {
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
    if sections.get(player).is_none() {
        return unmodelled("QUEUE by a player the save does not have");
    }
    Command::Queue {
        player,
        building,
        unit: packet.t_id,
        amount: packet.count.max(0) as u16,
        button: packet.button_location_id,
    }
}

/// BUILD (`FUN_140e7a400`): `num` builders put a foundation of master `o_id` at (`x`, `y`); the
/// builder ids follow. The handler reads the master as an i16 (`+0x10`). The slice takes z = 0:
/// the game sets it from the terrain, so a foundation on raised terrain shows in word 7.
fn build_command(packet: &de::command::Build, sections: &[PlayerSection<'_>]) -> Command {
    let player = packet.commp_id as usize;
    let master = packet.o_id as i16;
    let unit_ai = sections.get(player).map_or(
        Err(Unsupported("BUILD names a player the save does not have")),
        |section| new_unit_ai(section.objects(), master),
    );
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
        queued: packet.field_0x1e != 0,
        commits: packet.field_0x1f == 0,
    }
}

/// `BUILD_WALL` (0x69): the wall's master at `+0x10`, the line's tiles at `+0x8`..`+0xe`, the
/// target at `+0x14`, the queue flags at `+0x1a` and `+0x1b`, the builders (at most a human
/// player's 60, `FUN_140e6b250`).
fn build_wall_command(packet: &de::command::BuildWall, sections: &[PlayerSection<'_>]) -> Command {
    let player = packet.commp_id as usize;
    let master = packet.o_id as i16;
    if packet.num == LAST_COMMANDED {
        return Command::Unmodelled {
            player,
            why: Unsupported("BUILD_WALL by the last commanded units"),
        };
    }
    let unit_ai = sections.get(player).map_or(
        Err(Unsupported(
            "BUILD_WALL names a player the save does not have",
        )),
        |section| new_unit_ai(section.objects(), master),
    );
    Command::BuildWall(genie_core::wall::Wall {
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

/// Whether the runs list each decoded command with the update it runs in ([`trace_commands`]).
static TRACE_COMMANDS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Every run from now on lists each decoded command with the update it runs in, on standard
/// error (`on`), or stops listing them.
pub fn trace_commands(on: bool) {
    TRACE_COMMANDS.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Run the slice over `frames`, each tick with its commands.
fn run(
    turns: &mut Turns,
    decoder: &mut Decoder,
    frames: &[Frame<'_>],
    sections: &[PlayerSection<'_>],
) {
    let trace = TRACE_COMMANDS.load(std::sync::atomic::Ordering::Relaxed);
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
///
/// # Panics
///
/// Never: the first checkpoint it reads is the one it found.
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
    // from the recording's header: the load's run-time state too, not the bare `slice`
    let mut turns = header_slice(&header, masters);
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
        .filter(|unit| listed(unit))
        .map(|unit| ObjectState {
            id: unit.id,
            main_id: unit.life.master_id,
            state: unit.life.state,
            amount_held: unit.life.amount,
            position: unit.position,
            retarget_timer: unit.ai.as_ref().map(|ai| ai.retarget_timer as i32),
            // a producing building's action state is its make action's while it has one; its
            // type stays -1
            top_action: top_action(unit).or_else(|| {
                unit.production
                    .as_ref()
                    .and_then(|p| p.action)
                    .map(|a| TopAction {
                        kind: -1,
                        state: a.state(),
                    })
            }),
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
    /// The object's id.
    pub id: u32,
    /// Its top action's type, −1 for none.
    pub action: i32,
    /// Its top action's state.
    pub state: u8,
    /// Its unit AI's retarget timer, 0 without a unit AI.
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
    // from the recording's header: the load's run-time state too, not the bare `slice`
    let mut turns = header_slice(&header, masters);
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
            action: unit.artifact.map_or(-1, |_| i32::from(ARTIFACT_TYPE)),
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
    let mut turns = header_slice(&header, masters);
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

/// The slice as the recording's header load leaves it: [`slice`] with the objects-in-sight
/// registry empty, every object's mask 0, as the game's load makes it, and the hunted animals'
/// unit AI fields their load made.
fn header_slice(header: &Header, masters: Option<&MasterTable>) -> Turns {
    let mut turns = slice(header, masters);
    if let Some(sight) = turns.spatial.sight.as_mut() {
        sight.registry = Some(genie_core::registry::Registry::new(
            header.players().len(),
            genie_core::registry::CLASSES,
        ));
    }
    // the hunted animals' origins are their places in this very save
    hunted_origins(&mut turns, header);
    watch_at_load(&mut turns);
    turns
}

/// The ghosts' watched lists as the game's load builds them ([`Turns::build_watched`]), once the
/// slice holds its registry; what it cannot build stops every player.
fn watch_at_load(turns: &mut Turns) {
    if let Err(e) = turns.build_watched() {
        for p in &mut turns.players {
            let _ = p.unsupported.get_or_insert((0, 0, e));
            p.stopped = true;
        }
    }
}

/// The slice from the recording's header ([`header_slice`]) after its first `ticks` updates,
/// nothing forced: for comparing a run from the start with the tick oracle's saves. Dev only.
pub fn header_run(
    recording: &[u8],
    ticks: usize,
    masters: Option<&MasterTable>,
) -> Result<Turns, Error> {
    let chapter = de::chapters(recording)?[0];
    let header = chapter.header()?;
    let sections = header.players();
    let mut turns = header_slice(&header, masters);
    let mut decoder = Decoder::default();
    run(
        &mut turns,
        &mut decoder,
        &frames(&chapter, ticks)?,
        &sections,
    );
    Ok(turns)
}

/// The rows the comparison with the game's saves reads of a player's live objects in a slice.
#[must_use]
pub fn object_rows(turns: &Turns, player: usize) -> Vec<ObjectRow> {
    turns.players.get(player).map_or_else(Vec::new, |p| {
        p.objects
            .iter()
            .filter(|u| listed(u))
            .map(ObjectRow::of_unit)
            .collect()
    })
}

/// What the comparison with the game's saves checks of one live object: its master, state, hit
/// points, held amount and position in full f32, its top action (type, state) and its unit AI's
/// retarget timer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectRow {
    /// The object's id.
    pub id: i32,
    /// Its master.
    pub master: i16,
    /// Its state.
    pub state: u8,
    /// Its hit points.
    pub hp: f32,
    /// What it holds.
    pub amount: f32,
    /// Its place.
    pub position: [f32; 3],
    /// Its top action's type and state.
    pub action: Option<(i16, u8)>,
    /// The top action's target (`+0x30`), where the slice holds the action.
    pub action_target: Option<i32>,
    /// Its unit AI's retarget timer.
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
    /// A combat object's town-bell flag and back-to-work records.
    pub town_bell: Option<genie_core::town_bell::TownBell>,
    /// A combat object's `next_volley` (the missiles of its volley under way already fired) and
    /// `mReloadWorldtime` (when it may attack again).
    pub volley: Option<(u8, u32)>,
    /// Its unit group (`mGroupID`, `mGroupFlag`).
    pub group: genie_core::group::GroupLink,
}

/// What an object's row compares of its unit AI (named as in the save).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiRow {
    /// The fields, named as in the save, with their values.
    pub fields: Vec<(&'static str, String)>,
}

impl AiRow {
    fn of(ai: &genie_core::unit_ai::UnitAi, under_attack: bool) -> Self {
        let f = |v: &dyn std::fmt::Debug| format!("{v:?}");
        Self {
            fields: vec![
                ("current_order", f(&ai.current_order)),
                ("current_order_priority", f(&ai.order_priority)),
                ("current_action", f(&ai.current_action)),
                ("current_target", f(&ai.current_target)),
                ("current_target_type", f(&ai.target_type)),
                ("current_target_point", f(&ai.target_point)),
                ("desired_target_distance", f(&ai.desired_distance)),
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
                ("alert_mode", f(&ai.alert_mode)),
                ("time_since_enemy_sighting", f(&ai.time_since_enemy)),
                ("orders", f(&ai.orders)),
                ("notifies", f(&ai.notifies)),
                ("attackers", f(&ai.attackers)),
                ("under_attack", f(&under_attack)),
                ("furious", f(&ai.home.map(|h| h.furious))),
                ("formation_type", f(&ai.formation_type)),
            ],
        }
    }

    fn saved(
        ai: &genie_save::generated::world::UnitAiModule,
        elephant: Option<&genie_save::generated::world::TribeElephantUnitAiModule>,
        under_attack: bool,
    ) -> Self {
        let f = |v: &dyn std::fmt::Debug| format!("{v:?}");
        Self {
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
                (
                    "desired_target_distance",
                    f(&ai.desired_target_distance_value),
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
                ("alert_mode", f(&(ai.alert_mode_value != 0))),
                (
                    "time_since_enemy_sighting",
                    f(&ai.time_since_enemy_sighting),
                ),
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
                ("attackers", f(&ai.temp_int)),
                ("under_attack", f(&under_attack)),
                ("furious", f(&elephant.map(|e| e.furious != 0))),
                ("formation_type", f(&ai.formation_type)),
            ],
        }
    }
}

/// Equality to the bit (an `f32` by its bits): two values the same this way print the same, so a
/// row the same this way has no difference ([`object_differences`] compares the fields' texts).
trait Same {
    fn same(&self, other: &Self) -> bool;
}

macro_rules! same_by_eq {
    ($($t:ty),*) => {
        $(impl Same for $t {
            fn same(&self, other: &Self) -> bool {
                self == other
            }
        })*
    };
}

same_by_eq!(
    i16,
    i32,
    u8,
    u32,
    bool,
    &'static str,
    String,
    genie_core::turn::Gate
);

impl Same for f32 {
    fn same(&self, other: &Self) -> bool {
        self.to_bits() == other.to_bits()
    }
}

impl<T: Same> Same for Option<T> {
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Some(a), Some(b)) => a.same(b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl<T: Same, const N: usize> Same for [T; N] {
    fn same(&self, other: &Self) -> bool {
        self.iter().zip(other).all(|(a, b)| a.same(b))
    }
}

impl<T: Same> Same for Vec<T> {
    fn same(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().zip(other).all(|(a, b)| a.same(b))
    }
}

impl<A: Same, B: Same> Same for (A, B) {
    fn same(&self, other: &Self) -> bool {
        self.0.same(&other.0) && self.1.same(&other.1)
    }
}

impl<A: Same, B: Same, C: Same> Same for (A, B, C) {
    fn same(&self, other: &Self) -> bool {
        self.0.same(&other.0) && self.1.same(&other.1) && self.2.same(&other.2)
    }
}

impl Same for genie_core::town_bell::BackToWork {
    fn same(&self, other: &Self) -> bool {
        self.target == other.target
            && self.target_type == other.target_type
            && self.action == other.action
            && self.target2 == other.target2
            && self.point.same(&other.point)
    }
}

impl Same for genie_core::town_bell::TownBell {
    fn same(&self, other: &Self) -> bool {
        self.flag == other.flag
            && self.work.same(&other.work)
            && self.before_building.same(&other.before_building)
    }
}

impl Same for genie_core::movement::PathData {
    fn same(&self, other: &Self) -> bool {
        self.level == other.level
            && self.linked_type == other.linked_type
            && self.points.same(&other.points)
            && self.has_path == other.has_path
            && self.path_flags == other.path_flags
            && self.waypoint == other.waypoint
            && self.state == other.state
            && self.range.same(&other.range)
            && self.target == other.target
            && self.pause_time.same(&other.pause_time)
            && self.flags == other.flags
            && self.length.same(&other.length)
            && self.created == other.created
            && self.continue_counter == other.continue_counter
            && self.repath_time.same(&other.repath_time)
            && self.ignore == other.ignore
            && self.ignored == other.ignored
    }
}

impl Same for MoveRow {
    fn same(&self, other: &Self) -> bool {
        self.forward.same(&other.forward)
            && self.right.same(&other.right)
            && self.velocity.same(&other.velocity)
            && self.paths.same(&other.paths)
            && self.waiting == other.waiting
            && self.wait_delays == other.wait_delays
            && self.attack_moving == other.attack_moving
            && self.last_move == other.last_move
            && self.substitute_count == other.substitute_count
            && self.ignore_units == other.ignore_units
            && self.angle.same(&other.angle)
            && self.last_turn.same(&other.last_turn)
    }
}

impl Same for AiRow {
    fn same(&self, other: &Self) -> bool {
        self.fields == other.fields
    }
}

impl Same for ObjectRow {
    fn same(&self, other: &Self) -> bool {
        self.id == other.id
            && self.master == other.master
            && self.state == other.state
            && self.hp.same(&other.hp)
            && self.amount.same(&other.amount)
            && self.position.same(&other.position)
            && self.action == other.action
            && self.action_target == other.action_target
            && self.retarget == other.retarget
            && self.movement.same(&other.movement)
            && self.construction.same(&other.construction)
            && self.build_added == other.build_added
            && self.ai.same(&other.ai)
            && self.human_order == other.human_order
            && self.gather.same(&other.gather)
            && self.combat.same(&other.combat)
            && self.gate == other.gate
            && self.town_bell.same(&other.town_bell)
            && self.volley == other.volley
            && self.group == other.group
    }
}

/// What a moving object's row compares of its movement.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveRow {
    /// `+0x830`: where it heads.
    pub forward: genie_core::fmath::Vec3,
    /// `+0x83c`: its right.
    pub right: genie_core::fmath::Vec3,
    /// Velocity and acceleration, when the object has velocity data.
    pub velocity: Option<(genie_core::fmath::Vec3, genie_core::fmath::Vec3)>,
    /// Its path data, the walked level first.
    pub paths: Vec<genie_core::movement::PathData>,
    /// `+0x7f9`, `+0x7fa`, `+0x7fb`, `+0x870`, `+0x874`.
    pub waiting: u8,
    /// `+0x7fa`: the walk's waits.
    pub wait_delays: u8,
    /// `+0x7fb`: it moves to attack.
    pub attack_moving: bool,
    /// `+0x870`: its last move time.
    pub last_move: u32,
    /// `+0x874`: the substitutes counted.
    pub substitute_count: i32,
    /// The unit's ignore list (`+0x670`).
    pub ignore_units: Vec<i32>,
    /// `+0x7e8`, where it faces.
    pub angle: f32,
    /// `+0x7ec`, `turn_towards_time`: its last facet step toward a target.
    pub last_turn: f32,
}

impl ObjectRow {
    fn of_unit(unit: &Unit) -> Self {
        Self {
            id: unit.id,
            master: unit.life.master_id,
            state: unit.life.state,
            hp: unit.life.hp,
            amount: unit.life.amount,
            position: unit.position,
            action: top_action(unit).map(|a| (a.kind, a.state)),
            action_target: unit.top_target(),
            retarget: unit.ai.as_ref().map(|ai| ai.retarget_timer as i32),
            movement: unit.mover.as_ref().map(|m| MoveRow {
                forward: m.forward,
                right: m.right,
                velocity: m.velocity.as_ref().map(|v| (v.velocity, v.acceleration)),
                // the paths' ignore units, which the save does not hold, are left out
                paths: m
                    .paths
                    .iter()
                    .map(|l| genie_core::movement::PathData {
                        ignored: Vec::new(),
                        ..l.clone()
                    })
                    .collect(),
                waiting: m.waiting,
                wait_delays: m.wait_delays,
                attack_moving: m.attack_moving,
                last_move: m.last_move,
                substitute_count: m.substitute_count,
                ignore_units: m.ignore_units.clone(),
                angle: m.angle,
                last_turn: m.last_turn,
            }),
            construction: unit
                .construction
                .map(|c| (c.desolid, c.progress, c.builders)),
            build_added: unit.build.map(|b| b.added),
            ai: unit.ai.as_ref().map(|ai| AiRow::of(ai, unit.under_attack)),
            human_order: unit.last_human_order,
            gather: unit
                .gather_point
                .map(|g| ([g.point[0], g.point[1]], g.target)),
            combat: unit
                .combat
                .then_some((unit.charge, unit.charge_hit, unit.hp_powerups)),
            gate: unit.gate,
            town_bell: unit.combat.then_some(unit.town_bell),
            volley: unit
                .combat
                .then_some((unit.next_volley, unit.reload_worldtime)),
            group: unit.group,
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
        angle: m.angle,
        last_turn: m.turn_towards_time,
    })
}

/// A slice run from a save ([`objects_from_save`]): one player's objects after it, and the first
/// update the slice could not model, if any.
#[derive(Clone, Debug)]
pub struct SliceRun {
    /// Every object the player holds after the run.
    pub objects: Vec<ObjectRow>,
    /// The first update the slice could not model: the tick, the object's index and why.
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
pub(crate) fn header_objects(header: &Header, id: u32) -> Result<Vec<ObjectRow>, Error> {
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
            action_target: object
                .actions()
                .and_then(|a| a.entries().next())
                .map(|(_, action)| action.base().map_or(-1, |b| b.target_id)),
            retarget: object
                .combat()
                .and_then(|c| c.unit_ai())
                .map(|ai| ai.randomized_retarget_timer as i32),
            movement: saved_movement(object, &header.world.base_world.path),
            construction: object.tribe_building_object.as_deref().map(|b| {
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
                .and_then(|(_, a)| a.tribe_action_build.as_deref())
                .map(|b| b.added_to_num_builders != 0),
            ai: object.combat().and_then(|c| {
                let elephant = c.tribe_elephant_unit_ai_module.as_deref();
                c.unit_ai()
                    .map(|ai| AiRow::saved(ai, elephant, s.under_attack_value != 0))
            }),
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
            town_bell: object.combat().map(town_bell),
            volley: object.combat().map(|c| {
                (
                    c.next_volley,
                    c.rge_combat_object.reload_worldtime.unwrap_or(0),
                )
            }),
            group: group_link(s),
        })
        .collect())
}

/// An object's unit group link as saved (`mGroupID`, `mGroupFlag`).
const fn group_link(s: &RgeStaticObject) -> genie_core::group::GroupLink {
    genie_core::group::GroupLink {
        id: s.group_id,
        flag: s.group_flag,
    }
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
    /// The map tiles' object lists in the game's order ([`tile_lists_from_dump`]); without it
    /// the slice keeps the order of its load.
    pub tiles: Option<&'a str>,
    /// The world's pools of recycled objects ([`recycle_pools_from_dump`]); without it the slice
    /// starts them empty, as a game loaded from the save does.
    pub pools: Option<&'a str>,
    /// The objects' inside lists in the game's order ([`inside_lists_from_dump`]); without it
    /// the slice keeps the order of the objects, as a game loaded from the save does.
    pub inside: Option<&'a str>,
    /// The process globals the game reads and no save holds ([`groups::globals_from_dump`]);
    /// without it the slice takes what a load gives (the group command counter past the saved
    /// groups', the scratch array zero).
    pub globals: Option<&'a str>,
}

/// The objects' inside lists as the tick oracle dumps them (`state_dumps.inside_lists_text`):
/// `container id...` per object with objects inside, in the list's order.
pub fn inside_lists_from_dump(text: &str) -> Result<Vec<(i32, Vec<i32>)>, Error> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let words = l
                .split_whitespace()
                .map(|w| w.parse::<i32>().map_err(|_| Error::BadTileDump))
                .collect::<Result<Vec<_>, _>>()?;
            match words[..] {
                [c, ..] => Ok((c, words[1..].to_vec())),
                [] => Err(Error::BadTileDump),
            }
        })
        .collect()
}

/// The pool of recycled missiles (type 60, `world+0x1d8`) as the tick oracle dumps it
/// (`state_dumps.recycle_pools_text`: per type a `pool TYPE COUNT` line, then `ID FLYING` per
/// object from the list's first, the next to come back), in the slice's order
/// ([`genie_core::turn::Recycling::missiles`]: the next to come back last). The slice recycles no
/// object of another type; their pools are not read.
pub fn recycle_pools_from_dump(text: &str) -> Result<Vec<(i32, bool)>, Error> {
    let mut missiles = Vec::new();
    let mut kind = None;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words[..] {
            ["pool", k, _] => kind = Some(k.parse::<i32>().map_err(|_| Error::BadPoolDump)?),
            [id, flying] if kind == Some(60) => missiles.push((
                id.parse::<i32>().map_err(|_| Error::BadPoolDump)?,
                flying.parse::<u8>().map_err(|_| Error::BadPoolDump)? != 0,
            )),
            [_, _] if kind.is_some() => {}
            _ => return Err(Error::BadPoolDump),
        }
    }
    missiles.reverse();
    Ok(missiles)
}

/// The world's pool of recycled ghosts (type 25) as the tick oracle dumps it
/// (`state_dumps.recycle_pools_text`): their ids, the last to come back first in the slice's
/// order ([`genie_core::turn::Recycling::ghosts`]).
pub fn ghost_pool_from_dump(text: &str) -> Result<Vec<i32>, Error> {
    let mut ghosts = Vec::new();
    let mut kind = None;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words[..] {
            ["pool", k, _] => kind = Some(k.parse::<i32>().map_err(|_| Error::BadPoolDump)?),
            [id, _] if kind == Some(i32::from(genie_core::ghost::GHOST)) => {
                ghosts.push(id.parse::<i32>().map_err(|_| Error::BadPoolDump)?);
            }
            [_, _] if kind.is_some() => {}
            _ => return Err(Error::BadPoolDump),
        }
    }
    ghosts.reverse();
    Ok(ghosts)
}

/// The map tiles' object lists as the tick oracle dumps them (`state_dumps.tile_lists_text`):
/// `x y id...` per tile of two objects or more, in the tile's order (an id signed, or its 32
/// bits unsigned as the first dumps wrote a missile's).
pub fn tile_lists_from_dump(text: &str) -> Result<Vec<(i32, i32, Vec<i32>)>, Error> {
    let word = |v: &str| {
        v.parse::<i64>()
            .ok()
            .filter(|&n| i64::from(i32::MIN) <= n && n <= i64::from(u32::MAX))
            .map(|n| n as u32 as i32)
            .ok_or(Error::BadTileDump)
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let words = l
                .split_whitespace()
                .map(word)
                .collect::<Result<Vec<_>, _>>()?;
            match words[..] {
                [x, y, ..] => Ok((x, y, words[2..].to_vec())),
                _ => Err(Error::BadTileDump),
            }
        })
        .collect()
}

/// The tiles whose objects the slice holds as the game does (the dump's `game`) but in another
/// order, as differences: `tile (x, y) order slice [...], the game [...]`. Tiles whose objects
/// differ are left out: their objects' places differ.
fn tile_order_differences(
    spatial: &genie_core::spatial::Spatial,
    game: &[(i32, i32, Vec<i32>)],
) -> Vec<String> {
    game.iter()
        .filter_map(|(x, y, ids)| {
            let mine = spatial.tiles().list(*x, *y);
            let (mut a, mut b) = (mine.to_vec(), ids.clone());
            a.sort_unstable();
            b.sort_unstable();
            (a == b && mine != &ids[..])
                .then(|| format!("tile ({x}, {y}) order slice {mine:?}, the game {ids:?}"))
        })
        .collect()
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
                let _ = out.entries.insert(number(w[1])? as i32, entry);
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
            .filter(|u| listed(u))
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

/// The slice as it starts from the tick oracle's save at tick `start_tick` with `dumps`, before
/// any update, its world told what it reads of the units (as an update starts). Dev only.
pub fn turns_from_save(
    save: &[u8],
    recording: &[u8],
    start_tick: usize,
    masters: Option<&MasterTable>,
    dumps: Dumps<'_>,
) -> Result<Turns, Error> {
    let mut turns = run_from_save(save, recording, start_tick, 0, masters, dumps)?;
    turns.sync_world();
    Ok(turns)
}

/// The old short pather's result and path for one call ([`old_pather_paths`]).
pub type PatherAnswer = (u8, Vec<[f32; 2]>);

/// The old short pather's answers (slot 0x3e8, [`genie_core::hull_path::HullPather::find`]) in
/// the world of the slice started from the tick oracle's save at tick `start_tick` with `dumps`,
/// for each call (the unit, its place then, the target, the range): the pather's result and path.
/// Each unit stands at its place for its call. Dev only: a check of the pather against the game's
/// own calls (`load_oracle/probes/_obspath.py`), whose world is the save's but for the units the
/// update moved before the call.
pub fn old_pather_paths(
    save: &[u8],
    recording: &[u8],
    start_tick: usize,
    masters: Option<&MasterTable>,
    dumps: Dumps<'_>,
    calls: &[(i32, [f32; 2], i32, f32)],
) -> Result<Vec<Result<PatherAnswer, Unsupported>>, Error> {
    let mut turns = turns_from_save(save, recording, start_tick, masters, dumps)?;
    let mut out = Vec::new();
    for &(id, from, target, range) in calls {
        let Some(unit) = turns
            .players
            .iter()
            .flat_map(|p| &p.objects)
            .find(|u| u.id == id)
        else {
            out.push(Err(Unsupported("a caller the save does not hold")));
            continue;
        };
        let Some(mover) = unit.mover.as_ref() else {
            out.push(Err(Unsupported("a caller that does not move")));
            continue;
        };
        let (mask, radius) = (mover.stats.restriction_mask, mover.stats.radius[0]);
        let z = turns.spatial.get(id).map_or(0.0, |p| p.position[2]);
        turns.spatial.set_position(id, [from[0], from[1], z]);
        let Some(to) = turns
            .spatial
            .get(target)
            .map(|p| [p.position[0], p.position[1]])
        else {
            out.push(Err(Unsupported("a target the save does not hold")));
            continue;
        };
        let mut pather =
            genie_core::hull_path::HullPather::new(&turns.spatial, id, target, mask, radius);
        out.push(
            pather
                .find(from, to, target, range)
                .map(|(r, p)| (r, p.points)),
        );
    }
    Ok(out)
}

/// What one forced replay step found ([`forced_ticks`]): the slice started from the game's save
/// after `start`, run `ticks` updates, against the game's save after `start + ticks`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    /// The save the step starts from.
    pub start: usize,
    /// How many updates it ran.
    pub ticks: usize,
    /// What differs, by player: the objects that ran modelled (an object that froze, or every
    /// object of a player that stopped, is left out: its reason is in `unsupported`) and the
    /// player's sight.
    pub differences: Vec<(u32, String)>,
    /// What the slice could not model: the player, the object (None for the player's stop), the
    /// update and why.
    pub unsupported: Vec<(u32, Option<i32>, u32, Unsupported)>,
    /// The players' attributes past the resources that differ, apart from the differences (the
    /// slice does not keep every statistic yet): the player, the attribute, the slice's value
    /// and the game's (their f32 bits).
    pub attributes: Vec<(u32, usize, u32, u32)>,
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
    let saved = SavedStep::of(after, masters)?;
    compare_players(&turns, &saved, None, &mut report)?;
    if let Some(g) = saved.game.groups.as_ref() {
        for d in groups::group_differences(&turns.groups, g, false) {
            report
                .differences
                .push((group_owner(&turns, &saved.game, &d), d));
        }
    }
    Ok(report)
}

/// The command phase of the update after save `start` against the game's state at its end
/// (`after`, the tick oracle's `snapshot_at` at `0x141552990`, with the process globals dumped
/// there when given): every player's objects and the world's unit groups, as [`forced_ticks`]
/// reports them. Dev only.
///
/// # Errors
///
/// When a save or the recording does not parse.
pub fn command_phase(
    save: &[u8],
    (after, after_globals): (&[u8], Option<&str>),
    recording: &[u8],
    start: usize,
    masters: Option<&MasterTable>,
    dumps: Dumps<'_>,
) -> Result<TickReport, Error> {
    let turns = command_phase_from_save(save, recording, start, masters, dumps)?;
    let mut report = TickReport {
        start,
        ticks: 1,
        ..TickReport::default()
    };
    let mut saved = SavedStep::of(after, masters)?;
    compare_players(&turns, &saved, None, &mut report)?;
    if let (Some(text), Some(g)) = (after_globals, saved.game.groups.as_mut()) {
        groups::globals_from_dump(text, g);
    }
    if let Some(g) = saved.game.groups.as_ref() {
        for d in groups::group_differences(&turns.groups, g, after_globals.is_some()) {
            report
                .differences
                .push((group_owner(&turns, &saved.game, &d), d));
        }
    }
    Ok(report)
}

/// The forced replay's continuous mode (FD-008, CONTINUE.md): genie-core from the game's save
/// after `start` (with its dumps), then `ticks` updates, each compared with the game's save after
/// it (`save_after(update)`, which may end the run early with None) and then forced to the game's
/// state where the slice did not model it or got it wrong ([`genie_core::turn::Turns::force`]: the
/// frame seed, the frozen and the differing objects, a stopped player, the sight, and what the
/// game's dumps after the update hold, `dump_after(update, extension)`: the tiles' object lists
/// (`tiles`) and the recycled missiles (`pools`), whose differences are Gaia's). The objects the
/// slice gets right keep their own state, the saved and the unsaved, from tick to tick. One report
/// per update, each with that update's own differences, and where the run's time went. The saves
/// are parsed on rayon's pool while the run goes on, at most `window` ahead of it. Dev only.
#[allow(clippy::too_many_arguments)]
pub fn forced_run(
    start_save: &[u8],
    recording: &[u8],
    start: usize,
    ticks: usize,
    masters: Option<&MasterTable>,
    dumps: Dumps<'_>,
    save_after: &(dyn Fn(usize) -> Option<Vec<u8>> + Sync),
    dump_after: &dyn Fn(usize, &str) -> Option<String>,
    window: usize,
) -> Result<(Vec<TickReport>, Times), Error> {
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
    if let Some(text) = dumps.tiles {
        let _ = turns.spatial.force_tile_lists(&tile_lists_from_dump(text)?);
    }
    if let Some(text) = dumps.pools {
        turns.recycling.missiles = recycle_pools_from_dump(text)?;
        turns.recycling.ghosts = ghost_pool_from_dump(text)?;
    }
    if let Some(text) = dumps.inside {
        let _ = turns.force_garrisons(&inside_lists_from_dump(text)?);
    }
    if let Some(text) = dumps.globals {
        groups::globals_from_dump(text, &mut turns.groups);
    }
    watch_at_load(&mut turns);
    let played = de::chapters(recording)?[0];
    let start_header = played.header()?;
    hunted_origins(&mut turns, &start_header);
    let updates = start + 1;
    turns.ticks = updates as u32;
    let all = frames(&played, updates + ticks)?;
    let mut decoder = Decoder::default();
    decoder.prime(&all[..updates.min(all.len())], &sections);
    let mut reports = Vec::new();
    // the game's saves after each update, parsed on rayon's pool while the run goes on, at most
    // `window` ahead of it; the pool is shared by every run going on at once
    let window = window.max(1);
    let ahead = Ahead::new();
    let parse = |i: usize| save_after(start + i + 1).map(|save| SavedStep::of(&save, masters));
    let mut times = Times::default();
    rayon::in_place_scope(|tasks| {
        let (ahead, parse) = (&ahead, &parse);
        let ask = |i: usize| {
            if i < ticks {
                tasks.spawn(move |_| ahead.put(i, parse(i)));
            }
        };
        (0..window).for_each(ask);
        let mut run_all = || -> Result<(), Error> {
            for k in 0..ticks {
                let Some(frame) = all.get(updates + k) else {
                    break;
                };
                let clock = std::time::Instant::now();
                let taken = ahead.take(k);
                ask(k + window);
                let Some(after) = taken else {
                    break;
                };
                let after = after?;
                times.parse += clock.elapsed();
                let clock = std::time::Instant::now();
                run(
                    &mut turns,
                    &mut decoder,
                    std::slice::from_ref(frame),
                    &sections,
                );
                times.run += clock.elapsed();
                let clock = std::time::Instant::now();
                let mut report = TickReport {
                    start: start + k,
                    ticks: 1,
                    ..TickReport::default()
                };
                let tick = turns.ticks;
                compare_players(&turns, &after, Some(tick), &mut report)?;
                let tiles = dump_after(start + k + 1, "tiles")
                    .map(|text| tile_lists_from_dump(&text))
                    .transpose()?;
                if let Some(t) = tiles.as_ref() {
                    for d in tile_order_differences(&turns.spatial, t) {
                        report.differences.push((0, d));
                    }
                }
                // each player's ghosts, in the list's order, by what the save holds of them
                for (p, game) in after.ghosts.iter().enumerate() {
                    let slice = turns.players.get(p).map_or(&[][..], |q| &q.ghosts[..]);
                    let key = |g: &genie_core::ghost::Ghost| {
                        (
                            g.id,
                            g.original_id,
                            g.state,
                            g.cant_see,
                            g.position.map(f32::to_bits),
                        )
                    };
                    let (a, b): (Vec<_>, Vec<_>) = (
                        slice.iter().map(key).collect(),
                        game.iter().map(key).collect(),
                    );
                    if a != b {
                        let only = |x: &Vec<_>, y: &Vec<_>| -> Vec<_> {
                            x.iter().filter(|e| !y.contains(*e)).copied().collect()
                        };
                        let (sl, gm) = (only(&a, &b), only(&b, &a));
                        let what = if sl.is_empty() && gm.is_empty() {
                            format!("ghosts in another order: slice {a:?}, the game {b:?}")
                        } else {
                            format!("ghosts the slice's alone {sl:?}, the game's alone {gm:?}")
                        };
                        report.differences.push((p as u32, what));
                    }
                }
                let pools_text = dump_after(start + k + 1, "pools");
                let pool = pools_text
                    .as_deref()
                    .map(recycle_pools_from_dump)
                    .transpose()?;
                let ghost_pool = pools_text
                    .as_deref()
                    .map(ghost_pool_from_dump)
                    .transpose()?;
                if let Some(game_pool) = pool.as_ref()
                    && *game_pool != turns.recycling.missiles
                {
                    report.differences.push((
                        0,
                        format!(
                            "recycled missiles slice {:?}, the game {game_pool:?}",
                            turns.recycling.missiles
                        ),
                    ));
                }
                if let Some(game_pool) = ghost_pool.as_ref()
                    && *game_pool != turns.recycling.ghosts
                {
                    report.differences.push((
                        0,
                        format!(
                            "recycled ghosts slice {:?}, the game {game_pool:?}",
                            turns.recycling.ghosts
                        ),
                    ));
                }
                // the world's next reusable id, which ghosts and missiles take
                if turns.recycling.next_id != after.game.next_reusable_id {
                    report.differences.push((
                        0,
                        format!(
                            "next reusable id slice {} game {}",
                            turns.recycling.next_id, after.game.next_reusable_id
                        ),
                    ));
                }
                // the world's unit groups, by their owners; the process globals the tick
                // oracle dumped beside the save are the game's
                let mut after = after;
                let globals = dump_after(start + k + 1, "globals");
                if let (Some(text), Some(g)) = (globals.as_deref(), after.game.groups.as_mut()) {
                    groups::globals_from_dump(text, g);
                }
                let group_differences = after
                    .game
                    .groups
                    .as_ref()
                    .map(|g| groups::group_differences(&turns.groups, g, globals.is_some()))
                    .unwrap_or_default();
                let groups_differ = !group_differences.is_empty();
                for d in group_differences {
                    report
                        .differences
                        .push((group_owner(&turns, &after.game, &d), d));
                }
                // the next frame seed: a free run draws the game's
                if turns.frame_seed != after.game.frame_seed {
                    report.differences.push((
                        0,
                        format!(
                            "frame seed slice {} game {}",
                            turns.frame_seed, after.game.frame_seed
                        ),
                    ));
                }
                times.compare += clock.elapsed();
                let clock = std::time::Instant::now();
                let SavedStep {
                    objects,
                    mut seen,
                    terrain,
                    mut game,
                    ..
                } = after;
                game.tile_lists = tiles;
                game.missile_pool = pool;
                game.ghost_pool = ghost_pool;
                // the groups take the game's state only where they differ
                if !groups_differ {
                    game.groups = None;
                }
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
                    game.sight = Some((
                        std::mem::take(&mut seen.maps),
                        std::mem::take(&mut seen.unified),
                    ));
                }
                for (p, id, why) in turns.force(&game) {
                    report.unsupported.push((p as u32, Some(id), tick, why));
                }
                // a parsed save's state is dropped on the pool: its drop costs the run as much as
                // its forcing
                let gone = (game, objects, seen, terrain);
                tasks.spawn(move |_| drop(gone));
                times.force += clock.elapsed();
                reports.push(report);
            }
            Ok(())
        };
        run_all()
    })?;
    Ok((reports, times))
}

/// The player a group difference (`group ID: …`) goes with: the group's owner in the game or the
/// slice, Gaia for the world's list and counter.
fn group_owner(turns: &Turns, game: &genie_core::turn::GameState, difference: &str) -> u32 {
    let id = difference
        .strip_prefix("group ")
        .and_then(|d| d.split(':').next())
        .and_then(|d| d.parse::<i32>().ok());
    let owner = id.and_then(|id| {
        game.groups
            .as_ref()
            .and_then(|g| g.get(id))
            .or_else(|| turns.groups.get(id))
            .map(|g| g.owner)
    });
    owner
        .filter(|&o| (o as usize) < turns.players.len())
        .unwrap_or(0)
}

/// The game's saves parsed ahead of a forced run ([`forced_run`]): the pool's tasks put them by
/// index, the run takes them in order.
struct Ahead<T> {
    parsed: std::sync::Mutex<std::collections::BTreeMap<usize, T>>,
    ready: std::sync::Condvar,
}

impl<T> Ahead<T> {
    const fn new() -> Self {
        Self {
            parsed: std::sync::Mutex::new(std::collections::BTreeMap::new()),
            ready: std::sync::Condvar::new(),
        }
    }

    /// A task's parse of index `k`.
    fn put(&self, k: usize, v: T) {
        let _ = self.parsed.lock().expect("a parse panicked").insert(k, v);
        self.ready.notify_all();
    }

    /// The run takes index `k`, waiting until it is parsed.
    fn take(&self, k: usize) -> T {
        let mut parsed = self.parsed.lock().expect("a parse panicked");
        loop {
            if let Some(v) = parsed.remove(&k) {
                return v;
            }
            parsed = self.ready.wait(parsed).expect("a parse panicked");
        }
    }
}

/// Where a forced run's time went ([`forced_run`]): waiting for the game's saves to be parsed
/// (the parse of a batch runs on every core while the run waits), the slice's updates, the
/// comparisons (with the dumps' reading) and the forcing.
#[derive(Clone, Copy, Debug, Default)]
pub struct Times {
    /// Waiting for the parsed saves.
    pub parse: std::time::Duration,
    /// The slice's updates.
    pub run: std::time::Duration,
    /// The comparisons with the game's saves and dumps.
    pub compare: std::time::Duration,
    /// The forcing.
    pub force: std::time::Duration,
}

/// The game's state in a save as the slice's units ([`genie_core::turn::GameState`]).
fn game_state(header: &Header, masters: Option<&MasterTable>) -> genie_core::turn::GameState {
    let world = &header.world.base_world;
    let mut game = genie_core::turn::GameState {
        frame_seed: world.current_frame_random_seed,
        next_reusable_id: world.next_reusable_object_id,
        game_state: u32::from(world.temp_game_state),
        groups: groups::unit_groups(world).ok(),
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
        game.attributes.push(attributes.clone());
        game.pending_debit.push(p.world_player().base.pending_debit);
        let base = &p.world_player().base;
        game.statuses.push((
            base.game_status,
            base.resigned & 2 == 0 && base.resigned != 0,
        ));
        let techs = &p.world_player().tech_tree.tech_player_tree;
        game.techs.push(techs.iter().map(tech_state).collect());
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
    // the world's game state and the map's terrain go with Gaia's differences
    if turns.game_state != after.game.game_state {
        let d = format!(
            "game state slice {} game {}",
            turns.game_state, after.game.game_state
        );
        report.differences.push((0, d));
    }
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
            .filter(|u| listed(u))
            .map(ObjectRow::of_unit)
            .collect();
        let game = after.objects.get(p).ok_or(Error::NoSuchPlayer(id))?;
        if !player.stopped
            && let Some(r) = after.game.resources.get(p)
            && *r != player.resources
        {
            let d = format!("resources slice {:?} game {:?}", player.resources, r);
            report.differences.push((id, d));
        }
        // the other attributes, each one the slice holds that differs (reported apart)
        if !player.stopped
            && let Some(saved) = after.game.attributes.get(p)
        {
            report.attributes.extend(
                player
                    .attributes
                    .iter()
                    .zip(saved)
                    .enumerate()
                    .skip(4)
                    .filter(|(_, (a, b))| a.to_bits() != b.to_bits())
                    .map(|(i, (a, b))| (id, i, a.to_bits(), b.to_bits())),
            );
        }
        if let Some(&(status, resigned)) = after.game.statuses.get(p)
            && (status, resigned) != (player.game_status, player.resigned)
        {
            let d = format!(
                "game status slice {:?} game {:?} (status, resigned)",
                (player.game_status, player.resigned),
                (status, resigned)
            );
            report.differences.push((id, d));
        }
        if !player.stopped {
            for d in tech_differences(player, &after.game, p) {
                report.differences.push((id, d));
            }
        }
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

/// A player's pending debit and technology states against the game's: per technology its state,
/// researches queued and done, work done and building.
fn tech_differences(player: &Player, game: &genie_core::turn::GameState, p: usize) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(d) = game
        .pending_debit
        .get(p)
        .filter(|&&d| d != player.pending_debit)
    {
        out.push(format!(
            "pending debit slice {:?} game {:?}",
            player.pending_debit, d
        ));
    }
    let Some(theirs) = game.techs.get(p) else {
        return out;
    };
    let key = |t: &genie_core::tech::TechState| {
        (
            t.state,
            t.pending,
            t.times_researched,
            t.research_done,
            t.building_master,
        )
    };
    for (i, (a, b)) in player.techs.iter().zip(theirs).enumerate() {
        if key(a) != key(b) {
            out.push(format!(
                "tech {i}: slice {:?} game {:?} (state, queued, done, work, building)",
                key(a),
                key(b)
            ));
        }
    }
    out
}

/// The slice from a save: [`objects_from_save`]'s run, every player's state after it. Dev only.
///
/// # Errors
///
/// When a save or the recording does not parse.
pub fn run_from_save(
    save: &[u8],
    recording: &[u8],
    start_tick: usize,
    ticks: usize,
    masters: Option<&MasterTable>,
    dumps: Dumps<'_>,
) -> Result<Turns, Error> {
    from_save(save, recording, start_tick, ticks, masters, dumps, false)
}

/// The world's unit groups of a save, with the process globals dumped beside it when given.
///
/// # Errors
///
/// When the save does not parse or its groups do not import.
pub fn saved_groups(
    save: &[u8],
    globals: Option<&str>,
) -> Result<genie_core::group::UnitGroups, Error> {
    let chapter = de::chapters(save)?[0];
    let header = chapter.header()?;
    let mut g = groups::unit_groups(&header.world.base_world).map_err(Error::Unmodelled)?;
    if let Some(text) = globals {
        groups::globals_from_dump(text, &mut g);
    }
    Ok(g)
}

/// The slice from a save after the command phase of the next update alone
/// ([`genie_core::turn::Turns::command_phase`]), for a check against the game's save taken
/// there (the tick oracle's `snapshot_at` at `0x141552990`). Dev only.
///
/// # Errors
///
/// When a save or the recording does not parse.
pub fn command_phase_from_save(
    save: &[u8],
    recording: &[u8],
    start_tick: usize,
    masters: Option<&MasterTable>,
    dumps: Dumps<'_>,
) -> Result<Turns, Error> {
    from_save(save, recording, start_tick, 1, masters, dumps, true)
}

/// [`run_from_save`]'s run; with `commands_only` the first update stops after its commands.
fn from_save(
    save: &[u8],
    recording: &[u8],
    start_tick: usize,
    ticks: usize,
    masters: Option<&MasterTable>,
    dumps: Dumps<'_>,
    commands_only: bool,
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
    if let Some(text) = dumps.tiles {
        let _ = turns.spatial.force_tile_lists(&tile_lists_from_dump(text)?);
    }
    if let Some(text) = dumps.pools {
        turns.recycling.missiles = recycle_pools_from_dump(text)?;
        turns.recycling.ghosts = ghost_pool_from_dump(text)?;
    }
    if let Some(text) = dumps.inside {
        let _ = turns.force_garrisons(&inside_lists_from_dump(text)?);
    }
    if let Some(text) = dumps.globals {
        groups::globals_from_dump(text, &mut turns.groups);
    }
    watch_at_load(&mut turns);
    let played = de::chapters(recording)?[0];
    hunted_origins(&mut turns, &played.header()?);
    let updates = start_tick + 1;
    turns.ticks = updates as u32;
    let all = frames(&played, updates + ticks)?;
    let next = all.get(updates..).unwrap_or_default();
    let mut decoder = Decoder::default();
    decoder.prime(&all[..updates.min(all.len())], &sections);
    if commands_only {
        if let Some(frame) = next.first() {
            let commands: Vec<Command> = frame
                .commands
                .iter()
                .filter_map(|p| decoder.command(p, &sections))
                .collect();
            turns.command_phase(frame.dt, &commands);
        }
        return Ok(turns);
    }
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
///
/// # Panics
///
/// When a contact line of the dump does not hold numbers (the dump's own format).
#[must_use]
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
#[derive(Debug)]
pub struct FinderWorld {
    /// Every player's masters.
    pub masters: Vec<genie_core::masters::Masters>,
    /// The objects by id.
    pub objects: std::collections::HashMap<i32, FinderObject>,
    /// `relation[i]` of each player (`player+0x260`).
    pub relations: Vec<Vec<u8>>,
    /// Every player's attribute 95.
    pub attribute_95: Vec<f32>,
    /// Every player's civ.
    pub civs: Vec<u8>,
    /// The drop-site table.
    pub drop_sites: Vec<genie_core::tasks::DropSite>,
}

/// One object as the task finder reads it.
#[derive(Clone, Copy, Debug)]
pub struct FinderObject {
    /// Its player.
    pub player: usize,
    /// Its master.
    pub master: i16,
    /// Its state.
    pub state: u8,
    /// What it holds: the attribute and the amount.
    pub held: (i16, f32),
    /// Its hit points.
    pub hp: f32,
    /// Its most hit points.
    pub max_hp: f32,
    /// It may be attacked.
    pub attackable: bool,
    /// The pasture it is in, −1 for none.
    pub pasture: i32,
    /// The main building of an annex (`+0xbb8`), if it is one.
    pub annex_of: Option<i32>,
    /// Its top action's type, if it has one.
    pub top_action: Option<i16>,
    /// Its place.
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
    #[must_use]
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
    #[must_use]
    pub fn target(&self, id: i32) -> Option<genie_core::tasks::TaskTarget<'_>> {
        self.target_up(id, 1)
    }

    /// The finder's view of unit `id` as the actor.
    #[must_use]
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
            let _ = objects.insert(
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
                        .as_deref()
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

/// The game's linked technologies (`resources/_common/dat/linkedTechs.json`): each group's
/// technologies with the group's kind. A technology of a kind the game does not know, or out of
/// the ids' range, is left out.
pub fn linked_techs(
    json: &str,
) -> Result<std::collections::BTreeMap<i16, genie_core::tech::Linked>, serde_json::Error> {
    let file: genie_json::LinkedTechs = serde_json::from_str(json)?;
    Ok(file
        .linked_techs
        .iter()
        .filter_map(|g| genie_core::tech::Linked::of(&g.kind).map(|k| (g, k)))
        .flat_map(|(g, k)| {
            g.techs
                .iter()
                .filter_map(move |&t| i16::try_from(t).ok().map(|t| (t, k)))
        })
        .collect())
}

/// What makes `unitlines.json` out of shape for [`unit_lines`].
#[derive(Debug)]
pub enum UnitLinesError {
    /// The file is not JSON.
    Json(serde_json::Error),
    /// A value of a kind the builder does not read: the entry's index and the key.
    Value(usize, &'static str),
}

impl std::fmt::Display for UnitLinesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(e) => write!(f, "{e}"),
            Self::Value(i, key) => write!(f, "UnitLines[{i}]: {key} is not what the game reads"),
        }
    }
}

impl std::error::Error for UnitLinesError {}

/// The game's unit lines (`resources/_common/dat/unitlines.json`) as `FUN_140fde680` reads each
/// entry of `UnitLines`: `LineID` (an int), `IDChain` (an array of ints), `Building` (a bool,
/// false when absent) and `Identifier` (a string or an array of strings). A key an entry leaves
/// out stays out ([`genie_core::lines::UnitLines::build`] skips the entry); a file without
/// `UnitLines` holds none.
///
/// # Errors
///
/// A file that is not JSON, or a value of another kind than the game reads.
pub fn unit_lines(json: &str) -> Result<Vec<genie_core::lines::UnitLine>, UnitLinesError> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(UnitLinesError::Json)?;
    let Some(entries) = v.get("UnitLines").and_then(serde_json::Value::as_array) else {
        return Ok(Vec::new());
    };
    let int = |i: usize, key: &'static str, v: &serde_json::Value| {
        v.as_i64()
            .and_then(|n| i32::try_from(n).ok())
            .ok_or(UnitLinesError::Value(i, key))
    };
    let text = |i: usize, v: &serde_json::Value| {
        v.as_str()
            .map(str::to_owned)
            .ok_or(UnitLinesError::Value(i, "Identifier"))
    };
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let line = e.get("LineID").map(|v| int(i, "LineID", v)).transpose()?;
            let chain = e
                .get("IDChain")
                .map(|v| {
                    v.as_array()
                        .ok_or(UnitLinesError::Value(i, "IDChain"))?
                        .iter()
                        .map(|id| int(i, "IDChain", id))
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?;
            let building = match e.get("Building") {
                None => false,
                Some(b) => b.as_bool().ok_or(UnitLinesError::Value(i, "Building"))?,
            };
            let identifiers = match e.get("Identifier") {
                None => Vec::new(),
                Some(serde_json::Value::Array(names)) => names
                    .iter()
                    .map(|n| text(i, n))
                    .collect::<Result<Vec<_>, _>>()?,
                Some(name) => vec![text(i, name)?],
            };
            Ok(genie_core::lines::UnitLine {
                line,
                chain,
                building,
                identifiers,
            })
        })
        .collect()
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
                    auto_work: e.get("auto_work").and_then(serde_json::Value::as_bool)
                        != Some(false),
                    targets: e["target_list"].as_array().map_or_else(Vec::new, |ts| {
                        ts.iter()
                            .map(|t| {
                                let field = |key: &str| t.get(key).map_or(-1, num);
                                genie_core::tasks::DropTarget {
                                    class: field("object_group") as i16,
                                    workers: field("worker_num") as i32,
                                    state: field("object_state") as i32,
                                    attribute: field("attribute_type") as i16,
                                }
                            })
                            .collect()
                    }),
                    livestock: e.get("accepts_livestock").is_some(),
                })
                .collect()
        }))
}

/// The owner of each shape in a mesh dump (its `owner ID SHAPE` lines): the objects' `+0x64c`.
#[must_use]
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
/// load (`FUN_140f7d310`). Checked against the game's grid, dumped by the tick oracle.
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

/// One field of one object where the slice's row and the game's save differ
/// ([`object_field_differences`]); its text is [`object_differences`]' line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Difference {
    /// The object's id.
    pub id: i32,
    /// Its master: the game's, or the slice's for an object only the slice holds.
    pub master: i16,
    /// The field's name, or [`ONLY_IN_THE_GAME`], [`ONLY_IN_THE_SLICE`].
    pub field: String,
    /// The slice's value (its whole row for an object only the slice holds).
    pub slice: String,
    /// The game's value (its whole row for an object only the game holds).
    pub game: String,
}

/// [`Difference::field`] of an object the game's save holds and the slice does not.
pub const ONLY_IN_THE_GAME: &str = "only in the game";
/// [`Difference::field`] of an object the slice holds and the game's save does not.
pub const ONLY_IN_THE_SLICE: &str = "only in the slice";

impl std::fmt::Display for Difference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.field.as_str() {
            ONLY_IN_THE_GAME => write!(f, "{}: {} ({})", self.id, self.field, self.game),
            ONLY_IN_THE_SLICE => write!(f, "{}: {} ({})", self.id, self.field, self.slice),
            _ => write!(
                f,
                "{}: {} slice {} game {}",
                self.id, self.field, self.slice, self.game
            ),
        }
    }
}

/// Where two object lists differ, one line per object and field ([`object_field_differences`]).
#[must_use]
pub fn object_differences(slice: &[ObjectRow], game: &[ObjectRow]) -> Vec<String> {
    object_field_differences(slice, game)
        .iter()
        .map(ToString::to_string)
        .collect()
}

/// Where two object lists differ, one [`Difference`] per object and field: `slice` from the
/// slice, `game` from the game's save, matched by id, in the game's order, then the objects only
/// the slice holds. Equal lists give none.
#[must_use]
pub fn object_field_differences(slice: &[ObjectRow], game: &[ObjectRow]) -> Vec<Difference> {
    let mut out = Vec::new();
    // the first slice row of each id, as a search from the front finds it
    let mut by_id = std::collections::HashMap::with_capacity(slice.len());
    for s in slice {
        let _ = by_id.entry(s.id).or_insert(s);
    }
    for g in game {
        let Some(&s) = by_id.get(&g.id) else {
            out.push(Difference {
                id: g.id,
                master: g.master,
                field: ONLY_IN_THE_GAME.to_owned(),
                slice: String::new(),
                game: format!("{g:?}"),
            });
            continue;
        };
        // a row the same to the bit has no field that differs: skip the fields' texts
        if s.same(g) {
            continue;
        }
        let mut field = |name: &str, a: String, b: String| {
            if a != b {
                out.push(Difference {
                    id: g.id,
                    master: g.master,
                    field: name.to_owned(),
                    slice: a,
                    game: b,
                });
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
        if s.action_target.is_some() && s.action == g.action {
            field(
                "action target",
                format!("{:?}", s.action_target),
                format!("{:?}", g.action_target),
            );
        }
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
        field(
            "town bell",
            format!("{:?}", s.town_bell),
            format!("{:?}", g.town_bell),
        );
        field(
            "next volley, reload world time",
            format!("{:?}", s.volley),
            format!("{:?}", g.volley),
        );
        field(
            "unit group",
            format!("{:?}", s.group),
            format!("{:?}", g.group),
        );
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
                field("angle", format!("{:?}", a.angle), format!("{:?}", b.angle));
                field(
                    "last turn",
                    format!("{:?}", a.last_turn),
                    format!("{:?}", b.last_turn),
                );
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
        out.push(Difference {
            id: s.id,
            master: s.master,
            field: ONLY_IN_THE_SLICE.to_owned(),
            slice: format!("{s:?}"),
            game: String::new(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
