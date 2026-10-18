//! The master values the slice reads that the header save does not hold, per civ and master, read
//! from the game's data file with genie-dat: the dead-object values (genie-core's [`MasterData`])
//! and the default task's action. They are game data, which patches and DLCs change, so they come
//! from the `.dat` of the recording's game, never from constants.

use genie_core::lifecycle::MasterData;
use genie_core::masters as work;
use genie_dat::de::DatFile;
use std::collections::HashMap;

/// The action an action object's first update starts from its master's default task
/// (`RGE_Action_Object::update`, 0x14106ac90).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultAction {
    /// The master has no default task.
    None,
    /// The default task's action type (107 is the Artifact action).
    Type(i16),
    /// The default task id names no task in the master's list; what the game starts then is not
    /// read yet.
    NoSuchTask,
}

/// The default action of a master whose default task id is `default_task_id` and whose own tasks
/// are `(id, action type)` pairs (DE keeps the tasks in each unit's own list).
pub fn default_action(
    default_task_id: i16,
    mut tasks: impl Iterator<Item = (i16, i16)>,
) -> DefaultAction {
    if default_task_id < 0 {
        return DefaultAction::None;
    }
    tasks
        .find(|&(id, _)| id == default_task_id)
        .map_or(DefaultAction::NoSuchTask, |(_, action)| {
            DefaultAction::Type(action)
        })
}

/// One master's row.
#[derive(Clone, Debug, PartialEq)]
struct Row {
    data: MasterData,
    default_action: DefaultAction,
    /// What the work command reads of the master, as the `.dat` has it ([`MasterTable::work`]).
    work: Option<work::Master>,
    turning: Option<Turning>,
    /// A missile master's `projectile_type` (`+0x880`) and every master's `fly_mode` (`+0x608`),
    /// which the save does not hold.
    projectile_type: Option<u8>,
    fly_mode: u8,
}

/// What the save does not hold of a moving master's movement (the `.dat` unit's moving part).
/// Its turning is not here: every copy of a master derives it from its turn speed and walking
/// sprite ([`genie_core::movement::turn_rate`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Turning {
    /// Master `+0x768`, `old_move_algorithm`: bit 0 has a stop reset the unit's sprite.
    pub move_algorithm: u8,
    /// Master `+0x780`.
    pub min_collision_size_multiplier: f32,
}

/// A `.dat` task as the task finder reads it (`genie_core::masters::Task`).
fn dat_task(t: &genie_dat::de::Task) -> work::Task {
    work::Task {
        id: t.id,
        action_type: t.action_type,
        object_group: t.class_id,
        object_id: t.unit_id,
        combat_level: t.enable_targeting,
        terrain: t.terrain_id,
        owner_type: t.target_diplomacy,
        holding_attr: t.carry_check,
        attribute_type: t.resource_in,
        attribute_type2: t.resource_multiplier,
        attribute_type3: t.resource_out,
        attribute_type4: t.unused_resource,
        wait_time: t.search_wait_time,
        work_val1: t.work_value1,
        work_val2: t.work_value2,
        work_range: t.work_range,
        enablement: t.enabled.unwrap_or(-1),
        sprites: [
            t.moving_graphic_id,
            t.proceeding_graphic_id,
            t.working_graphic_id,
            t.carrying_graphic_id,
        ],
    }
}

/// The work-command values of a `.dat` unit.
fn dat_work(id: i16, unit: &genie_dat::de::Unit) -> work::Master {
    let cost = unit.creatable.as_ref().map_or_else(Vec::new, |c| {
        c.resource_costs
            .iter()
            .filter(|r| r.kind >= 0)
            .map(|r| (r.kind, r.amount, r.flag != 0))
            .collect()
    });
    work::Master {
        id,
        kind: unit.unit_type,
        class: unit.class,
        task_by_group: unit.bird.as_ref().map_or(0, |b| b.task_swap_group),
        radius: [unit.collision_size[0], unit.collision_size[1]],
        tall: 0.0 < unit.collision_size[2],
        height: unit.collision_size[2],
        obstruction_type: unit.obstruction_type,
        buildable_on: unit.can_be_built_on,
        obstruction_class: unit.obstruction_class,
        object_flags: 0,
        combat_level: unit.combat_level,
        tasks: unit
            .bird
            .as_ref()
            .map_or_else(Vec::new, |b| b.tasks.iter().map(dat_task).collect()),
        cost,
        work_rate: unit.bird.as_ref().map_or(0.0, |b| b.work_rate),
        drop_sites: unit
            .bird
            .as_ref()
            .map_or_else(Vec::new, |b| b.drop_sites.clone()),
        hit_points: unit.hit_points,
        capacity: unit.resource_capacity,
        build_time: unit
            .creatable
            .as_ref()
            .and_then(|c| c.train_locations.first())
            .map_or(0, |t| t.queue_time),
        attack_sprite2: unit
            .type50
            .as_ref()
            .and_then(|t| t.attack_graphic2)
            .unwrap_or(-1),
        idle_attack_sprite: unit
            .creatable
            .as_ref()
            .and_then(|c| c.idle_attack_graphic)
            .unwrap_or(-1),
        clearance: unit.clearance_size,
        outline: [unit.outline_size[0], unit.outline_size[1]],
        terrain_bonus: unit.type50.as_ref().map_or(-1, |t| t.defense_terrain_bonus),
        building: unit.building.as_ref().map(|b| work::BuildingMaster {
            adjacent_mode: b.adjacent_mode,
            disappears_when_built: b.disappears_when_built != 0,
            stack_unit: b.stack_unit_id,
            foundation_terrain: b.foundation_terrain_id,
            tech: b.tech_id,
        }),
        garrison: unit.garrison_capacity,
        recyclable: unit.recyclable != 0,
        auto_gather: unit.enable_auto_gather != 0,
        hill_mode: unit.hill_mode,
        placement_terrains: unit
            .placement_side_terrain
            .iter()
            .chain(&unit.placement_terrain)
            .any(|&t| 0 <= t),
        special_graphic: unit
            .creatable
            .as_ref()
            .is_some_and(|c| 0 <= c.special_graphic),
        death_sprite: unit.dying_graphic,
        death_spawns: [unit.dead_unit_id, unit.blood_unit_id],
        ..work::Master::default()
    }
}

/// What the `.dat` says of terrain for passability: each terrain's class (`FUN_1413fbff0` of its
/// `is_water` flags: 1 water, 2 shallows and beach, 4 land, 0 none) and each terrain restriction's
/// mask, the classes it can pass (the obstruction manager's table, `FUN_141054630`). Checked
/// against the game's in `tests/obstructions.rs`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerrainTable {
    classes: Vec<u8>,
    /// Each terrain's record as the map's terrain reads see it.
    kinds: Vec<genie_core::terrain::TerrainKind>,
    masks: Vec<u8>,
    /// Each terrain restriction's value per terrain (`passable_buildable_dmg_multiplier`).
    passable: Vec<Vec<f32>>,
}

/// The class bit of a terrain's flags.
const fn terrain_class(flags: u8) -> u8 {
    if flags == 0 {
        0
    } else if flags & 7 != 0 {
        1
    } else if flags & 0x18 != 0 {
        2
    } else {
        (flags >> 3) & 4
    }
}

impl TerrainTable {
    /// The terrain table of `dat`: each terrain's restrictions and the restrictions' passability.
    #[must_use]
    pub fn from_dat(dat: &DatFile) -> Self {
        let classes: Vec<u8> = dat
            .terrain_block
            .terrains
            .iter()
            .map(|t| terrain_class(t.is_water))
            .collect();
        let masks = dat
            .terrain_restrictions
            .iter()
            .map(|r| {
                let passable = r.passable_buildable_dmg_multiplier.iter().enumerate();
                passable
                    .filter(|&(_, &v)| 0.0 < v)
                    .fold(0, |m, (t, _)| m | classes.get(t).copied().unwrap_or(0))
            })
            .collect();
        let passable = dat
            .terrain_restrictions
            .iter()
            .map(|r| r.passable_buildable_dmg_multiplier.clone())
            .collect();
        let kinds = dat
            .terrain_block
            .terrains
            .iter()
            .map(|t| genie_core::terrain::TerrainKind {
                shown: t.enabled != 0 || 0 <= t.terrain_to_draw,
                flags: t.is_water,
            })
            .collect();
        Self {
            classes,
            kinds,
            masks,
            passable,
        }
    }

    /// The class bit of terrain `id`; 0 for a terrain the data file does not have.
    #[must_use]
    pub fn class(&self, id: u8) -> u8 {
        self.classes.get(usize::from(id)).copied().unwrap_or(0)
    }

    /// Each terrain restriction's mask, by restriction id.
    #[must_use]
    pub fn masks(&self) -> &[u8] {
        &self.masks
    }

    /// Each terrain restriction's value per terrain, by restriction id.
    #[must_use]
    pub fn passable(&self) -> &[Vec<f32>] {
        &self.passable
    }

    /// Each terrain's record as the map's terrain reads see it.
    #[must_use]
    pub fn kinds(&self) -> &[genie_core::terrain::TerrainKind] {
        &self.kinds
    }
}

/// Every master of every civ of a data file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MasterTable {
    rows: HashMap<(u8, i16), Row>,
    /// Each sprite's speed factor (the `.dat` graphic's `speed_multiplier`, sprite `+0xc`).
    sprite_speed: HashMap<i16, f32>,
    /// What the sprite clock reads of each graphic.
    pub graphics: std::collections::BTreeMap<i16, genie_core::sprite::Graphic>,
    /// The terrain table.
    pub terrain: TerrainTable,
    /// The drop-site table (`dropsites.json` beside the `.dat`, [`crate::drop_sites`]); empty
    /// when not loaded.
    pub drop_sites: Vec<genie_core::tasks::DropSite>,
    /// The technologies, by id.
    pub techs: Vec<genie_core::tech::TechDef>,
}

/// A technology of the `.dat` as the tech tree holds it: its costs, its type and its effect's
/// commands.
fn tech_def(dat: &DatFile, t: &genie_dat::de::Tech) -> genie_core::tech::TechDef {
    let effect = usize::try_from(t.effect_id)
        .ok()
        .and_then(|e| dat.effects.get(e))
        .map_or_else(Vec::new, |e| {
            e.commands
                .iter()
                .map(|c| genie_core::tech::EffectCommand {
                    kind: c.kind,
                    a: c.a,
                    b: c.b,
                    c: c.c,
                    d: c.d,
                })
                .collect()
        });
    genie_core::tech::TechDef {
        costs: t.resource_costs.each_ref().map(|c| genie_core::tech::Cost {
            attribute: c.kind,
            amount: c.amount,
            paid: c.flag,
        }),
        kind: t.kind,
        effect_id: t.effect_id,
        effect,
        required: t.required_techs,
        required_count: t.required_tech_count,
        civ: t.civ,
        mode: t.full_tech_mode,
        locations: t
            .research_locations
            .iter()
            .map(|l| (l.location_id, l.queue_time))
            .collect(),
    }
}

impl MasterTable {
    /// The rows of every unit every civ of `dat` has: decay (`resource_decay`), the undead flag
    /// (`undead_mode`) and the default action (the action part's default task).
    #[must_use]
    pub fn from_dat(dat: &DatFile) -> Self {
        let rows = dat.civs.iter().enumerate().flat_map(|(c, civ)| {
            civ.units.iter().enumerate().filter_map(move |(m, unit)| {
                let unit = unit.as_ref()?;
                let data = MasterData {
                    decay: unit.resource_decay,
                    undead: unit.undead_mode != 0,
                };
                let action = unit.bird.as_ref().map_or(DefaultAction::None, |b| {
                    default_action(
                        b.default_task_id,
                        b.tasks.iter().map(|t| (t.id, t.action_type)),
                    )
                });
                let turning = unit.dead_fish.as_ref().map(|d| Turning {
                    move_algorithm: d.old_move_algorithm,
                    min_collision_size_multiplier: d.min_collision_size_multiplier,
                });
                let flight = (
                    unit.projectile.as_ref().map(|p| p.projectile_type),
                    unit.fly_mode,
                );
                Some((
                    (c as u8, m as i16),
                    data,
                    action,
                    dat_work(m as i16, unit),
                    turning,
                    flight,
                ))
            })
        });
        let rows: Vec<_> = rows.collect();
        let mut table = Self::from_rows(
            rows.iter()
                .map(|(key, data, action, _, _, _)| (*key, *data, *action)),
        );
        for (key, _, _, w, turning, (projectile_type, fly_mode)) in rows {
            if let Some(row) = table.rows.get_mut(&key) {
                row.work = Some(w);
                row.turning = turning;
                row.projectile_type = projectile_type;
                row.fly_mode = fly_mode;
            }
        }
        table.terrain = TerrainTable::from_dat(dat);
        table.sprite_speed = dat
            .graphics
            .iter()
            .enumerate()
            .filter_map(|(id, g)| Some((id as i16, g.as_ref()?.speed_multiplier)))
            .collect();
        table.graphics = dat
            .graphics
            .iter()
            .enumerate()
            .filter_map(|(id, g)| {
                let g = g.as_ref()?;
                Some((
                    id as i16,
                    genie_core::sprite::Graphic {
                        id: id as i16,
                        frames: g.frame_count,
                        facets: g.angle_count,
                        speed_multiplier: g.speed_multiplier,
                        frame_duration: g.frame_duration,
                        replay_delay: g.replay_delay,
                        flags: g.sequence_type,
                    },
                ))
            })
            .collect();
        table.techs = dat.techs.iter().map(|t| tech_def(dat, t)).collect();
        table
    }

    /// A table of the given `((civ, master), data, default action)` rows.
    pub fn from_rows(
        rows: impl IntoIterator<Item = ((u8, i16), MasterData, DefaultAction)>,
    ) -> Self {
        Self {
            rows: rows
                .into_iter()
                .map(|(key, data, default_action)| {
                    (
                        key,
                        Row {
                            data,
                            default_action,
                            work: None,
                            turning: None,
                            projectile_type: None,
                            fly_mode: 0,
                        },
                    )
                })
                .collect(),
            sprite_speed: HashMap::new(),
            graphics: std::collections::BTreeMap::default(),
            terrain: TerrainTable::default(),
            drop_sites: Vec::new(),
            techs: Vec::new(),
        }
    }

    /// The dead-object values of `master` for `civ`; None if the data file has no such master.
    #[must_use]
    pub fn get(&self, civ: u8, master: i16) -> Option<MasterData> {
        self.rows.get(&(civ, master)).map(|r| r.data)
    }

    /// The default action of `master` for `civ`; None if the data file has no such master.
    #[must_use]
    pub fn default_action(&self, civ: u8, master: i16) -> Option<DefaultAction> {
        self.rows.get(&(civ, master)).map(|r| r.default_action)
    }

    /// The `.dat`'s work-command values of a civ's master: the task swap group the save lacks, and
    /// the tasks a master without saved ones has.
    #[must_use]
    pub fn work(&self, civ: u8, master: i16) -> Option<&work::Master> {
        self.rows.get(&(civ, master)).and_then(|r| r.work.as_ref())
    }

    /// How a civ's moving master turns; None for a master without a moving part.
    #[must_use]
    pub fn turning(&self, civ: u8, master: i16) -> Option<Turning> {
        self.rows.get(&(civ, master)).and_then(|r| r.turning)
    }

    /// A missile master's `projectile_type` (`+0x880`).
    #[must_use]
    pub fn projectile_type(&self, civ: u8, master: i16) -> Option<u8> {
        self.rows
            .get(&(civ, master))
            .and_then(|r| r.projectile_type)
    }

    /// A master's `fly_mode` (`+0x608`): 0 keeps its objects on the ground, 1 above it.
    #[must_use]
    pub fn fly_mode(&self, civ: u8, master: i16) -> Option<u8> {
        self.rows.get(&(civ, master)).map(|r| r.fly_mode)
    }

    /// A sprite's speed factor; None for a sprite the data file does not have.
    #[must_use]
    pub fn sprite_speed(&self, sprite: i16) -> Option<f32> {
        self.sprite_speed.get(&sprite).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_action_is_the_action_type_of_the_default_task() {
        // the cow: default task 0, whose action type is 107 (the Artifact action)
        assert_eq!(
            default_action(0, [(0, 107), (1, 6)].into_iter()),
            DefaultAction::Type(107)
        );
        assert_eq!(
            default_action(-1, [(0, 107)].into_iter()),
            DefaultAction::None
        );
        // FARMDROP and two others name task 0 but list no task
        assert_eq!(
            default_action(0, std::iter::empty()),
            DefaultAction::NoSuchTask
        );
    }

    #[test]
    fn rows_are_found_by_civ_and_master() {
        let t = MasterTable::from_rows([
            (
                (1, 332),
                MasterData {
                    decay: 1.0,
                    undead: false,
                },
                DefaultAction::None,
            ),
            (
                (2, 705),
                MasterData {
                    decay: 0.25,
                    undead: false,
                },
                DefaultAction::Type(107),
            ),
        ]);
        assert_eq!(t.get(1, 332).map(|d| d.decay), Some(1.0));
        assert_eq!(t.default_action(2, 705), Some(DefaultAction::Type(107)));
        assert_eq!((t.get(2, 332), t.default_action(1, 705)), (None, None));
    }
}
