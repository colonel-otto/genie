//! The master values a dead object's steps read that the header save does not hold (genie-core's
//! [`MasterData`]), per civ and master, as `scripts/dat_masters.py` writes them from a `.dat`. A
//! stand-in until the format crates read the current `.dat`: the table is the only input here not
//! taken from the recording.

use genie_core::lifecycle::MasterData;
use std::collections::HashMap;

/// Every master of every civ the table's `.dat` has.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MasterTable {
    rows: HashMap<(u8, i16), MasterData>,
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
    /// Tab-separated `civ master decay undead` rows; lines starting with `#` are comments.
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
            let [civ, master, decay, undead] = cells[..] else {
                return Err(error("not four columns"));
            };
            let civ = civ.parse().map_err(|_| error("civ is not a u8"))?;
            let master = master.parse().map_err(|_| error("master is not an i16"))?;
            let decay = decay.parse().map_err(|_| error("decay is not an f32"))?;
            let undead = match undead {
                "0" => false,
                "1" => true,
                _ => return Err(error("undead is not 0 or 1")),
            };
            rows.insert((civ, master), MasterData { decay, undead });
        }
        Ok(MasterTable { rows })
    }

    /// The values of `master` for `civ`; None if the `.dat` has no such master.
    pub fn get(&self, civ: u8, master: i16) -> Option<MasterData> {
        self.rows.get(&(civ, master)).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_found_by_civ_and_master() {
        let t = MasterTable::parse(
            "# source\n# columns\n1\t332\t1.0\t0\n2\t9\t0.10000000149011612\t1\n",
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
    }

    #[test]
    fn a_malformed_line_names_its_number() {
        for (text, reason) in [
            ("1\t332\t1.0\n", "not four columns"),
            ("x\t332\t1.0\t0\n", "civ is not a u8"),
            ("1\t332\tfast\t0\n", "decay is not an f32"),
            ("1\t332\t1.0\t2\n", "undead is not 0 or 1"),
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
