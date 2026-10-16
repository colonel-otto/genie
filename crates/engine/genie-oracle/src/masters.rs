//! The master values the slice reads that the header save does not hold, per civ and master, as
//! `scripts/dat_masters.py` writes them from a `.dat`: the dead-object values (genie-core's
//! [`MasterData`]) and the default task's action. They are game data, which patches and DLCs
//! change, so they come from the `.dat` of the recording's game. A stand-in until the format crates
//! read the current `.dat` (repos/genieutils, the C++ library the game ships with AGE, is the
//! reference); the table is the only input here not taken from the recording.

use genie_core::lifecycle::MasterData;
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

/// One master's row.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Row {
    data: MasterData,
    default_action: DefaultAction,
}

/// Every master of every civ the table's `.dat` has.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MasterTable {
    rows: HashMap<(u8, i16), Row>,
}

/// A line of the table that does not read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based.
    pub line: usize,
    pub reason: &'static str,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "master table line {}: {}", self.line, self.reason)
    }
}

impl std::error::Error for ParseError {}

impl MasterTable {
    /// Tab-separated `civ master decay undead default_action` rows; lines starting with `#` are
    /// comments.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let mut rows = HashMap::new();
        for (i, line) in text.lines().enumerate() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let error = |reason| ParseError {
                line: i + 1,
                reason,
            };
            let cells: Vec<&str> = line.split('\t').collect();
            let [civ, master, decay, undead, default_action] = cells[..] else {
                return Err(error("not five columns"));
            };
            let civ = civ.parse().map_err(|_| error("civ is not a u8"))?;
            let master = master.parse().map_err(|_| error("master is not an i16"))?;
            let decay = decay.parse().map_err(|_| error("decay is not an f32"))?;
            let undead = match undead {
                "0" => false,
                "1" => true,
                _ => return Err(error("undead is not 0 or 1")),
            };
            let default_action = match default_action.parse::<i16>() {
                Ok(-1) => DefaultAction::None,
                Ok(-2) => DefaultAction::NoSuchTask,
                Ok(t) if t >= 0 => DefaultAction::Type(t),
                _ => return Err(error("default action is not -2, -1 or an action type")),
            };
            let row = Row {
                data: MasterData { decay, undead },
                default_action,
            };
            rows.insert((civ, master), row);
        }
        Ok(MasterTable { rows })
    }

    /// The dead-object values of `master` for `civ`; None if the `.dat` has no such master.
    pub fn get(&self, civ: u8, master: i16) -> Option<MasterData> {
        self.rows.get(&(civ, master)).map(|r| r.data)
    }

    /// The default action of `master` for `civ`; None if the `.dat` has no such master.
    pub fn default_action(&self, civ: u8, master: i16) -> Option<DefaultAction> {
        self.rows.get(&(civ, master)).map(|r| r.default_action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_found_by_civ_and_master() {
        let t = MasterTable::parse(
            "# source\n# columns\n1\t332\t1.0\t0\t-1\n2\t9\t0.10000000149011612\t1\t107\n",
        )
        .unwrap();
        assert_eq!(
            t.get(1, 332),
            Some(MasterData {
                decay: 1.0,
                undead: false
            })
        );
        assert_eq!(
            t.get(2, 9),
            Some(MasterData {
                decay: 0.1,
                undead: true
            })
        );
        assert_eq!(t.get(2, 332), None);
        assert_eq!(
            (
                t.default_action(1, 332),
                t.default_action(2, 9),
                t.default_action(2, 332)
            ),
            (
                Some(DefaultAction::None),
                Some(DefaultAction::Type(107)),
                None
            )
        );
    }

    #[test]
    fn a_default_task_that_names_no_task_is_kept_apart() {
        let t = MasterTable::parse("1\t1193\t0.0\t0\t-2\n").unwrap();
        assert_eq!(t.default_action(1, 1193), Some(DefaultAction::NoSuchTask));
    }

    #[test]
    fn a_malformed_line_names_its_number() {
        for (text, reason) in [
            ("1\t332\t1.0\t0\n", "not five columns"),
            ("x\t332\t1.0\t0\t-1\n", "civ is not a u8"),
            ("1\t332\tfast\t0\t-1\n", "decay is not an f32"),
            ("1\t332\t1.0\t2\t-1\n", "undead is not 0 or 1"),
            (
                "1\t332\t1.0\t0\t-3\n",
                "default action is not -2, -1 or an action type",
            ),
        ] {
            let text = format!("# header\n{text}");
            assert_eq!(
                MasterTable::parse(&text),
                Err(ParseError { line: 2, reason }),
                "{text}"
            );
        }
    }
}
