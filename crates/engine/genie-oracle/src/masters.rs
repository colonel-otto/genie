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
}

/// How a moving master turns (the `.dat` unit's moving part; master `+0x76c`, `+0x770`, `+0x774`),
/// which the save does not hold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Turning {
    pub turn_radius: f32,
    pub turn_radius_speed: f32,
    pub max_yaw_per_second_moving: f32,
}

/// A `.dat` task as the task finder reads it (`genie_core::masters::Task`).
fn dat_task(t: &genie_dat::de::Task) -> work::Task {
    work::Task {
        action_type: t.action_type,
        object_group: t.class_id,
        object_id: t.unit_id,
        work_val1: t.work_value1,
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
        tasks: unit
            .bird
            .as_ref()
            .map_or_else(Vec::new, |b| b.tasks.iter().map(dat_task).collect()),
        cost,
    }
}

/// Every master of every civ of a data file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MasterTable {
    rows: HashMap<(u8, i16), Row>,
    /// Each sprite's speed factor (the `.dat` graphic's `speed_multiplier`, sprite `+0xc`).
    sprite_speed: HashMap<i16, f32>,
}

impl MasterTable {
    /// The rows of every unit every civ of `dat` has: decay (`resource_decay`), the undead flag
    /// (`undead_mode`) and the default action (the action part's default task).
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
                    turn_radius: d.turn_radius,
                    turn_radius_speed: d.turn_radius_speed,
                    max_yaw_per_second_moving: d.max_yaw_per_second_moving,
                });
                Some((
                    (c as u8, m as i16),
                    data,
                    action,
                    dat_work(m as i16, unit),
                    turning,
                ))
            })
        });
        let rows: Vec<_> = rows.collect();
        let mut table = Self::from_rows(
            rows.iter()
                .map(|(key, data, action, _, _)| (*key, *data, *action)),
        );
        for (key, _, _, w, turning) in rows {
            if let Some(row) = table.rows.get_mut(&key) {
                row.work = Some(w);
                row.turning = turning;
            }
        }
        table.sprite_speed = dat
            .graphics
            .iter()
            .enumerate()
            .filter_map(|(id, g)| Some((id as i16, g.as_ref()?.speed_multiplier)))
            .collect();
        table
    }

    /// A table of the given `((civ, master), data, default action)` rows.
    pub fn from_rows(
        rows: impl IntoIterator<Item = ((u8, i16), MasterData, DefaultAction)>,
    ) -> Self {
        MasterTable {
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
                        },
                    )
                })
                .collect(),
            sprite_speed: HashMap::new(),
        }
    }

    /// The dead-object values of `master` for `civ`; None if the data file has no such master.
    pub fn get(&self, civ: u8, master: i16) -> Option<MasterData> {
        self.rows.get(&(civ, master)).map(|r| r.data)
    }

    /// The default action of `master` for `civ`; None if the data file has no such master.
    pub fn default_action(&self, civ: u8, master: i16) -> Option<DefaultAction> {
        self.rows.get(&(civ, master)).map(|r| r.default_action)
    }

    /// The `.dat`'s work-command values of a civ's master: the task swap group the save lacks, and
    /// the tasks a master without saved ones has.
    pub fn work(&self, civ: u8, master: i16) -> Option<&work::Master> {
        self.rows.get(&(civ, master)).and_then(|r| r.work.as_ref())
    }

    /// How a civ's moving master turns; None for a master without a moving part.
    pub fn turning(&self, civ: u8, master: i16) -> Option<Turning> {
        self.rows.get(&(civ, master)).and_then(|r| r.turning)
    }

    /// A sprite's speed factor; None for a sprite the data file does not have.
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
