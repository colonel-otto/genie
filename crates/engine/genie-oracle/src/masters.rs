//! The master values the slice reads that the header save does not hold, per civ and master, read
//! from the game's data file with genie-dat: the dead-object values (genie-core's [`MasterData`])
//! and the default task's action. They are game data, which patches and DLCs change, so they come
//! from the `.dat` of the recording's game, never from constants.

use genie_core::lifecycle::MasterData;
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
#[derive(Clone, Copy, Debug, PartialEq)]
struct Row {
    data: MasterData,
    default_action: DefaultAction,
}

/// Every master of every civ of a data file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MasterTable {
    rows: HashMap<(u8, i16), Row>,
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
                Some(((c as u8, m as i16), data, action))
            })
        });
        Self::from_rows(rows)
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
                        },
                    )
                })
                .collect(),
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
