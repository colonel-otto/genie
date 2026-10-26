//! The unit lines (`resources/_common/dat/unitlines.json`): each line names the masters of one
//! upgrade chain, its first master first. The game builds two tables from the file once, at its
//! first world load (`FUN_140fde680`, from the load `FUN_141533cb0` with Gaia's master count):
//! one entry a master (`DAT_143cc2968`, 12 bytes: the next master of its chain, its line, its
//! line's `Building` flag) and one entry a line (`DAT_143cc2988`, 200 ints by line + 399: the
//! chain's first master). The object tracker files a unit under its line's first master
//! (`FUN_141658430`), and a garrison into the buildings near its target looks for those of the
//! target's line (`FUN_14161e370`).
//!
//! 158041: the builder `FUN_140fde680`, a master's line `FUN_140fdd6c0`, a line's first master
//! `FUN_140fdd6f0`.

/// The first line id (table 2's index 0): a line's id + 399 indexes table 2.
pub const FIRST_LINE: i32 = -399;
/// Table 2's entries (`DAT_143cc2990`, the builder's 200).
pub const LINES: usize = 200;

/// One entry of the file's `UnitLines` as the builder reads it. A key the entry leaves out is
/// `None` (`LineID`, `IDChain`) or false (`Building`) or empty (`Identifier`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitLine {
    /// `LineID`.
    pub line: Option<i32>,
    /// `IDChain`: the masters, the line's first first.
    pub chain: Option<Vec<i32>>,
    /// `Building`.
    pub building: bool,
    /// `Identifier`: one string, or each of an array's.
    pub identifiers: Vec<String>,
}

/// A master's entry of table 1 (12 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineEntry {
    /// `+0`: the master after it in its chain (−1 for the last).
    pub next: i32,
    /// `+4`: its line (−1 for none).
    pub line: i32,
    /// `+8`: its line's `Building` flag.
    pub building: bool,
}

impl LineEntry {
    /// The entry of a master in no line (the builder's fill: −1, −1, 0).
    pub const NONE: Self = Self {
        next: -1,
        line: -1,
        building: false,
    };
}

/// The two tables and the identifiers' list (`DAT_143f15808`: each line's identifiers with its
/// id). Empty, as the game's when the file is missing, until [`UnitLines::build`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitLines {
    /// Table 1, by master id.
    pub entries: Vec<LineEntry>,
    /// Table 2, by line + 399: the line's first master (−1 for none).
    pub firsts: Vec<i32>,
    /// The identifiers, with their line, in the file's order.
    pub identifiers: Vec<(i32, String)>,
}

impl UnitLines {
    /// `FUN_140fde680` with `master_count` (Gaia's, `player+0x20`) on the file's `lines`: table 1
    /// holds `master_count` entries of no line and table 2 200 lines of no master. Each entry
    /// with both `IDChain` and `LineID` whose line + 399 is within table 2 fills its chain from
    /// the last master to the first: a master id out of the masters' range is left out (and the
    /// link passes over it); each other takes its line, the `Building` flag and the master after
    /// it, and the first (index 0) becomes the line's first master. Its identifiers then join
    /// the list with its line. A later line's entry for a master replaces an earlier one's. The
    /// game logs the entries it leaves out.
    #[must_use]
    pub fn build(master_count: i32, lines: &[UnitLine]) -> Self {
        let count = usize::try_from(master_count).unwrap_or(0);
        let mut out = Self {
            entries: vec![LineEntry::NONE; count],
            firsts: vec![-1; LINES],
            identifiers: Vec::new(),
        };
        for l in lines {
            let (Some(line), Some(chain)) = (l.line, l.chain.as_ref()) else {
                // "Unit line is incorrectly defined and is missing a key variable"
                continue;
            };
            let Some(slot) = Self::slot(line) else {
                // "Unit line ID is out of bounds (%d)"
                continue;
            };
            let mut next = -1;
            for (i, &id) in chain.iter().enumerate().rev() {
                let Some(e) = usize::try_from(id)
                    .ok()
                    .filter(|_| id < master_count)
                    .and_then(|k| out.entries.get_mut(k))
                else {
                    // "Unit type ID is out of bounds (%d)"
                    continue;
                };
                if i == 0 {
                    out.firsts[slot] = id;
                }
                *e = LineEntry {
                    next,
                    line,
                    building: l.building,
                };
                next = id;
            }
            out.identifiers
                .extend(l.identifiers.iter().map(|s| (line, s.clone())));
        }
        out
    }

    /// Table 2's index of `line`, when within its 200 entries.
    fn slot(line: i32) -> Option<usize> {
        let slot = usize::try_from(line.checked_sub(FIRST_LINE)?).ok()?;
        (slot < LINES).then_some(slot)
    }

    /// `FUN_140fdd6c0`: the line of master `master` (−1 for an id out of table 1 or in no line).
    #[must_use]
    pub fn line_of(&self, master: i32) -> i32 {
        usize::try_from(master)
            .ok()
            .and_then(|k| self.entries.get(k))
            .map_or(-1, |e| e.line)
    }

    /// `FUN_140fdd6f0`: the first master of line `line` (−1 for a line out of table 2 or
    /// without one). Table 2 has its 200 entries once built; before, it holds none.
    #[must_use]
    pub fn first_of(&self, line: i32) -> i32 {
        Self::slot(line)
            .and_then(|s| self.firsts.get(s))
            .copied()
            .unwrap_or(-1)
    }

    /// The master a line stands for (`FUN_141658430`'s and `FUN_14161e370`'s reading): the first
    /// of `master`'s line, or `master` itself in none (a line without a first master gives −1).
    #[must_use]
    pub fn base(&self, master: i32) -> i32 {
        match self.line_of(master) {
            -1 => master,
            line => self.first_of(line),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: i32, chain: &[i32], building: bool, names: &[&str]) -> UnitLine {
        UnitLine {
            line: Some(id),
            chain: Some(chain.to_vec()),
            building,
            identifiers: names.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn a_chain_maps_each_master_to_its_line_its_first_and_the_next() {
        // the archer line (-299: 4, 24, 492) and the tower line (-398: 79, 234, 235)
        let lines = UnitLines::build(
            600,
            &[
                line(-299, &[4, 24, 492], false, &["archer-line"]),
                line(-398, &[79, 234, 235], true, &["watch-tower-line"]),
            ],
        );
        assert_eq!(lines.entries.len(), 600);
        assert_eq!(lines.firsts.len(), LINES);
        assert_eq!(
            lines.entries[24],
            LineEntry {
                next: 492,
                line: -299,
                building: false
            }
        );
        assert_eq!(lines.entries[492].next, -1);
        assert_eq!(lines.entries[4].next, 24);
        assert!(lines.entries[234].building);
        assert_eq!(
            (lines.line_of(492), lines.first_of(-299), lines.base(492)),
            (-299, 4, 4)
        );
        assert_eq!((lines.base(235), lines.base(83)), (79, 83));
        assert_eq!(
            lines.identifiers,
            vec![
                (-299, "archer-line".to_owned()),
                (-398, "watch-tower-line".to_owned())
            ]
        );
    }

    #[test]
    fn ids_out_of_the_masters_range_are_left_out_and_the_links_pass_over_them() {
        // chain 7, -3, 900, 6 with 600 masters: 6's next is -1, 7's next is 6; the first (7) is
        // in range, so it is the line's first
        let lines = UnitLines::build(600, &[line(-297, &[7, -3, 900, 6], false, &[])]);
        assert_eq!(lines.entries[7].next, 6);
        assert_eq!(lines.first_of(-297), 7);
        // a chain whose index 0 is out of range leaves the line without a first master
        let lines = UnitLines::build(600, &[line(-296, &[600, 75], false, &[])]);
        assert_eq!(lines.first_of(-296), -1);
        assert_eq!(lines.line_of(75), -296);
        assert_eq!(lines.base(75), -1);
        // an id at the master count is out (the builder's `local_208 <= id`)
        assert_eq!(lines.line_of(600), -1);
    }

    #[test]
    fn a_line_out_of_table_two_or_without_a_key_is_skipped_whole() {
        let lines = UnitLines::build(
            100,
            &[
                line(-400, &[1], false, &["low"]),
                line(-199, &[2], false, &["high"]),
                UnitLine {
                    line: Some(-299),
                    chain: None,
                    ..UnitLine::default()
                },
                UnitLine {
                    line: None,
                    chain: Some(vec![3]),
                    ..UnitLine::default()
                },
                line(-200, &[4], false, &["last"]),
            ],
        );
        assert_eq!([1, 2, 3, 4].map(|m| lines.line_of(m)), [-1, -1, -1, -200]);
        assert_eq!(lines.identifiers, vec![(-200, "last".to_owned())]);
        // an empty chain still lists its identifiers
        let lines = UnitLines::build(100, &[line(-250, &[], false, &["empty"])]);
        assert_eq!(lines.identifiers, vec![(-250, "empty".to_owned())]);
        assert_eq!(lines.first_of(-250), -1);
    }

    #[test]
    fn a_later_line_replaces_an_earlier_lines_entry_of_a_master() {
        let lines = UnitLines::build(
            100,
            &[
                line(-290, &[10, 11], false, &[]),
                line(-280, &[12, 11], true, &[]),
            ],
        );
        assert_eq!(
            lines.entries[11],
            LineEntry {
                next: -1,
                line: -280,
                building: true
            }
        );
        // the first line keeps its first master
        assert_eq!(lines.first_of(-290), 10);
    }

    #[test]
    fn unbuilt_tables_hold_no_line() {
        let lines = UnitLines::default();
        assert_eq!(
            (lines.line_of(4), lines.first_of(-299), lines.base(4)),
            (-1, -1, 4)
        );
    }
}
