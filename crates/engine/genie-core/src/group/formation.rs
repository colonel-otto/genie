//! A group's formation (FD-036): `BFormation`, its slots and lines, and the layouts that fill
//! them (createColumnLine `FUN_141012280`, ...).

use crate::GenieError;
use crate::checkpoint::cvttss2si;
use crate::fmath::{Vec3, add, scale, sub};
use crate::group::layout::{clear_line, reset_line};
use crate::group::{Formation, FormationLine, Slot, UnitGroup, sub_formation};
use crate::image::group::{FARTHEST, SLOT_NUDGE_X, SLOT_NUDGE_Z};
use crate::movement::PathData;
use crate::turn::Turns;

/// The formation types (`mType`; the log names of `FUN_14103fd00`).
pub const HORDE: i32 = 0;
/// A column (`FUN_141012280` with a column ratio).
pub const COLUMN: i32 = 1;
/// A line (`FUN_141012280` with a line ratio).
pub const LINE: i32 = 2;
/// A phalanx (`FUN_1410147b0`).
pub const PHALANX: i32 = 3;
/// A box (`FUN_141011a00`).
pub const BOX: i32 = 4;
/// A user defined formation (`FUN_141014f60`).
pub const USER_DEFINED: i32 = 5;
/// A staggered line (`FUN_141012280` staggered).
pub const STAGGERED: i32 = 7;
/// A flanking formation (`FUN_141012280` flanked).
pub const FLANKED: i32 = 8;

/// The sub-formations a layout goes through (`FUN_141012280`'s `0..=5`).
const CATEGORIES: i32 = 6;
/// A flanked line's slots move apart by this less the line's share of the depth
/// (`DAT_142eb3634`).
const FLANK_SPACING: f32 = crate::image::group::FLANK_SPACING;
/// A staggered line moves by a quarter of the slot offset (`DAT_142eb0f48`).
const STAGGER_SHARE: f32 = crate::image::group::STAGGER_SHARE;
/// A staggered line's offset is twice the spacing, and an even line moves by half the offset
/// (`DAT_142e8c150`).
const TWO: f32 = crate::image::group::SPACING_FACTOR;
/// Below this a distance back counts as none (`DAT_142eb0c60`).
const EPSILON: f32 = crate::image::group::EPSILON;

impl Formation {
    /// `BFormation`'s constructor (the 0x58 bytes `FUN_14103fd00` allocates): no slots, no
    /// lines, horde (0), facing x (`+0x3c` 1.0).
    #[must_use]
    pub fn new() -> Self {
        Self {
            transform_orientation: [1.0, 0.0, 0.0],
            ..Self::default()
        }
    }

    /// The slots' array's capacity (`+0x8`): the slots in use and the spare ones.
    #[must_use]
    pub const fn slot_capacity(&self) -> usize {
        self.slots.len() + self.spare_slots.len()
    }

    /// `FUN_14102d580`: a larger slots' array of `n`, each new slot without a unit (-1), its
    /// desired category and sub-category -1, the rest 0; the slots in use are copied but for
    /// their transform mark, the spare ones not.
    pub(crate) fn resize_slots(&mut self, n: usize) {
        if n == 0 || n <= self.slot_capacity() {
            return;
        }
        let fresh = Slot {
            unit_id: -1,
            desired_category: -1,
            desired_sub_category: -1,
            ..Slot::default()
        };
        self.spare_slots = vec![fresh; n - self.slots.len()];
        // the copy moves `+0x00..+0x43` (0x14102d646..0x14102d6bf): the transform mark (`+0x44`)
        // is the new array's 0
        for s in &mut self.slots {
            s.transformed = 0;
        }
    }

    /// `FUN_14102d490`: a new lines' array of `n` (the old lines go), each line empty.
    pub(crate) fn resize_lines(&mut self, n: usize) {
        if n == 0 || n <= self.lines.len() {
            return;
        }
        self.lines = vec![FormationLine::default(); n];
    }

    /// `FUN_141041810`: every slot of the array (in use and spare) loses its unit and its filled
    /// mark.
    pub fn clear_units(&mut self) {
        for s in self.every_slot_mut() {
            s.unit_id = -1;
            s.position_filled = 0;
        }
    }

    /// The slot at `i` of the whole array (in use or spare).
    pub(super) fn slot_mut(&mut self, i: usize) -> &mut Slot {
        let used = self.slots.len();
        if i < used {
            &mut self.slots[i]
        } else {
            &mut self.spare_slots[i - used]
        }
    }

    /// Every slot of the array, in use and spare.
    pub(crate) fn every_slot_mut(&mut self) -> impl Iterator<Item = &mut Slot> {
        self.slots.iter_mut().chain(self.spare_slots.iter_mut())
    }

    /// `mNumberSlots` (`+0x4`) set to `n`: the array's first `n` slots are the ones in use.
    pub(super) fn use_slots(&mut self, n: usize) {
        let mut all: Vec<Slot> = std::mem::take(&mut self.slots);
        all.append(&mut self.spare_slots);
        let n = n.min(all.len());
        self.spare_slots = all.split_off(n);
        self.slots = all;
    }
}

/// What createColumnLine reads of one sub-formation's statistic.
#[derive(Clone, Debug)]
struct Category {
    count: i32,
    spacing: f32,
    types: i32,
    unit_types: Vec<u32>,
    unit_type_counts: Vec<u32>,
}

/// The line and column counts of a sub-formation of `count` units for a line (`by_line`, the
/// column ratio -1) or a column: the root of the count, rounded up, as the lines (a line's) or
/// the columns (a column's), then fewer lines until a line is at least twice as wide as the
/// formation is deep (a column the reverse). Returns (lines, columns).
fn sizes(count: i32, by_line: bool) -> (i32, i32) {
    let root = cvttss2si(crate::fmath::sqrt(count as f32).ceil());
    let root = root.max(1);
    let per = |a: i32| {
        let b = count / a;
        if 0 < count % a { b + 1 } else { b }
    };
    // `first` is the side the root sizes, `second` the other; `first` shrinks until it is at
    // most half of `second`
    let mut first = (count / root).max(1);
    let mut second = per(first);
    while second / 2 < first {
        first -= 1;
        if first < 1 {
            first = 1;
            second = count;
            break;
        }
        second = per(first);
    }
    if by_line {
        (first, second)
    } else {
        (second, first)
    }
}

/// The slots of createColumnLine (`FUN_141012280`, [`Turns::create_column_line`])
/// up to its transform: the group's sub-formations (its unit types, by category 0 to 5) laid
/// out in lines on `formation`, `world_time` the slots' time. `column` lays a column (the column
/// ratio set, type 1), else a line (type 2, staggered 7, flanked 8). Returns false for a group
/// without members (nothing laid out); its callers always pass a ratio.
///
/// Each sub-formation with units takes the lines and columns of [`sizes`]; the widest line
/// (columns times the sub-formation's spacing) sets the width of the others when two or more
/// have units: one that fits takes one line, else as many lines as its width needs, and a line
/// less while its columns are fewer than its empty places. The slots' array grows to the
/// widest line's columns times the lines (`FUN_14102d580`), the lines' to the lines
/// (`FUN_14102d490`); every slot turns inactive. Then, sub-formation by sub-formation (its unit
/// types' used counts start over, `DAT_143f20640`), line by line (the last of a sub-formation
/// takes the rest of its units): the line's slots start at half its columns of the single
/// offset (the spacing straight back, `(0, 0, -spacing)`, doubled when staggered) and step by
/// it; a staggered line moves by a quarter of it (back on even lines, forward on odd ones), an
/// even line of a plain one by half of it back; a flanked line's slots move apart by
/// [`FLANK_SPACING`] less the line's share of the depth. A slot takes no unit, the offset, the
/// time, the line as its depth, the sub-formation and, with several unit types, the next one
/// with units left (round robin; with two types the last slot of a line keeps the type for the
/// next line). The line takes its slots' indices and its distance back, which grows by each
/// line's spacing (doubled when staggered) and, when a wider sub-formation follows, by the
/// difference.
///
/// The roots and the line counts are the CRT's `ceilf` (the ucrtbase import at 0x142a8e728),
/// for which `f32::ceil` stands: both are exact (the result is the integer above, a NaN stays
/// NaN); `CVTTSS2SI` then makes a NaN or an infinity `i32::MIN`
/// ([`crate::checkpoint::cvttss2si`]), which the clamps to 1 take in.
///
/// # Panics
///
/// Never for a line or slot past its array: the lines' array holds the sum of the
/// sub-formations' lines (`FUN_14102d490`), and the slots' array their lines times the widest
/// line's columns (`FUN_14102d580`), while no line holds more than its sub-formation's
/// columns.
pub(crate) fn column_line_slots(
    formation: &mut Formation,
    group: &UnitGroup,
    column: bool,
    (staggered, flanked): (bool, bool),
    world_time: u32,
) -> bool {
    if group.members.is_empty() {
        return false;
    }
    let category = |c: i32| -> Option<Category> {
        let stat = group
            .unit_types
            .iter()
            .find(|s| s.formation_category == c)?;
        Some(Category {
            count: stat.units.len() as i32,
            spacing: stat.desired_spacing,
            types: stat.number_unit_types,
            unit_types: stat.unit_types.clone(),
            unit_type_counts: stat.unit_type_counts.clone(),
        })
    };
    let mut lines_of = [0i32; CATEGORIES as usize];
    let mut columns_of = [0i32; CATEGORIES as usize];
    let mut widest = 0.0f32;
    let mut widest_index = -1;
    let mut valid = 0;
    for c in 0..CATEGORIES {
        let Some(cat) = category(c).filter(|cat| 0 < cat.count) else {
            continue;
        };
        valid += 1;
        let (lines, columns) = sizes(cat.count, !column);
        lines_of[c as usize] = lines;
        columns_of[c as usize] = columns;
        let width = columns as f32 * cat.spacing;
        if widest_index == -1 || widest < width {
            widest = width;
            widest_index = c;
        }
    }
    if widest_index != -1 && 2 <= valid {
        for c in 0..CATEGORIES {
            if c == widest_index {
                continue;
            }
            let Some(cat) = category(c).filter(|cat| 0 < cat.count) else {
                continue;
            };
            let k = c as usize;
            if cat.count as f32 * cat.spacing <= widest {
                columns_of[k] = cat.count;
                lines_of[k] = 1;
                continue;
            }
            let mut lines = cvttss2si(((cat.count as f32 * cat.spacing) / widest).ceil());
            if lines < 1 {
                lines = 1;
            }
            lines_of[k] = lines;
            let mut columns = cat.count / lines;
            if 0 < cat.count % lines {
                columns += 1;
            }
            columns_of[k] = columns;
            let mut empty = columns * lines - cat.count;
            while columns < empty && 1 < lines {
                lines -= 1;
                lines_of[k] = lines;
                empty = columns * lines - cat.count;
            }
        }
    }
    let max_width = columns_of.iter().copied().fold(0, i32::max);
    let depth: i32 = lines_of.iter().sum();
    formation.resize_slots((depth * max_width).max(0) as usize);
    formation.resize_lines(depth.max(0) as usize);
    for s in formation.every_slot_mut() {
        s.active = 0;
    }
    let mut used_slots = 0usize;
    formation.kind = if column {
        COLUMN
    } else if flanked {
        FLANKED
    } else if staggered {
        STAGGERED
    } else {
        LINE
    };
    formation.transform_type = 0;
    formation.maximum_width = max_width;
    formation.maximum_depth = depth;
    formation.number_lines = depth;
    let mut line_index = 0i32;
    let mut slot_index = 0i32;
    let mut distance_back = 0.0f32;
    let mut last_spacing = 0.0f32;
    for c in 0..CATEGORIES {
        let Some(cat) = category(c).filter(|cat| 0 < cat.types) else {
            continue;
        };
        let k = c as usize;
        let mut sub_type: i32 = if cat.types == 1 { -1 } else { 0 };
        let mut used = [0u32; 50];
        let mut single: Vec3 = scale(cat.spacing, [0.0, 0.0, -1.0]);
        if staggered {
            single = [single[0] * TWO, single[1] * TWO, single[2] * TWO];
        }
        if EPSILON < distance_back && EPSILON < last_spacing && last_spacing < cat.spacing {
            distance_back += cat.spacing - last_spacing;
        }
        let lines = lines_of[k];
        let columns = columns_of[k];
        let mut in_category = 0;
        while in_category < lines {
            let mut actual = columns;
            if 1 < lines && in_category == lines - 1 {
                actual = cat.count - (lines - 1) * columns;
            }
            let half = scale((actual / 2) as f32, single);
            let mut total = add(half, [0.0; 3]);
            if staggered {
                let quarter = [
                    single[0] / STAGGER_SHARE,
                    single[1] / STAGGER_SHARE,
                    single[2] / STAGGER_SHARE,
                ];
                total = if line_index & 1 == 0 {
                    sub(total, quarter)
                } else {
                    [
                        total[0] + quarter[0],
                        total[1] + quarter[1],
                        total[2] + quarter[2],
                    ]
                };
            } else if actual & 1 == 0 {
                let half_step = [single[0] / TWO, single[1] / TWO, single[2] / TWO];
                total = sub(total, half_step);
            }
            let line = &mut formation.lines[line_index as usize];
            clear_line(line);
            let _ = reset_line(line, actual, line_index);
            line.distance_back = distance_back;
            for col in 0..actual.max(0) {
                let at = used_slots + col as usize;
                let mut offset = total;
                if flanked {
                    let flank = FLANK_SPACING - line_index as f32 / depth as f32;
                    if 0.0 <= offset[2] {
                        offset[2] += flank;
                    } else {
                        offset[2] -= flank;
                    }
                }
                let desired_sub_category = if sub_type < 0 {
                    -1
                } else {
                    cat.unit_types[sub_type as usize] as i32
                };
                *formation.slot_mut(at) = Slot {
                    unit_id: -1,
                    offset,
                    desired_transform_position: [0.0; 3],
                    time: world_time,
                    state: 0,
                    depth: line_index,
                    transform_position: [0.0; 3],
                    position_filled: 0,
                    desired_category: c,
                    active: 1,
                    desired_sub_category,
                    transformed: formation.slot_mut(at).transformed,
                };
                let line = &mut formation.lines[line_index as usize];
                if (col as usize) < line.indices.len() {
                    line.indices[col as usize] = slot_index as u32;
                }
                total = sub(total, single);
                slot_index += 1;
                if 0 <= sub_type {
                    used[sub_type as usize] += 1;
                    if cat.types != 2 || col < actual - 1 {
                        sub_type += 1;
                    }
                    let next = if sub_type < cat.types { sub_type } else { 0 };
                    sub_type = next;
                    if cat.unit_type_counts[next as usize] <= used[next as usize] {
                        // out of units for that sub type: the next with units left, else none
                        let mut j = if next + 1 < cat.types { next + 1 } else { 0 };
                        while j != next && cat.unit_type_counts[j as usize] <= used[j as usize] {
                            j = if j + 1 < cat.types { j + 1 } else { 0 };
                        }
                        sub_type = if j == next { -1 } else { j };
                    }
                }
            }
            used_slots += actual.max(0) as usize;
            line_index += 1;
            last_spacing = if staggered {
                cat.spacing + cat.spacing
            } else {
                cat.spacing
            };
            distance_back += last_spacing;
            in_category += 1;
        }
    }
    formation.use_slots(used_slots);
    true
}

impl Turns {
    /// createColumnLine (`FUN_141012280`) of `formation` for group `g`: a column (`column`, the
    /// column ratio 2 and line ratio -1) or a line (ratios -1 and 2), staggered or flanked;
    /// false without members. Its slots ([`column_line_slots`]), then the transform and the
    /// units ([`Turns::lay_units`]) with the column-to-line flag (0x141013d20..0x141013d3a): a
    /// line (2, 7 or 8) whose group's previous formation is a column (1). The game reads that
    /// type through the
    /// previous formation's pointer, which `UpdateDesiredPositions` sets whenever it lays a
    /// formation out anew; a group without one is not a column here.
    ///
    /// # Errors
    ///
    /// As the transform and the units' assignment.
    pub(crate) fn create_column_line(
        &self,
        formation: &mut Formation,
        g: &UnitGroup,
        column: bool,
        staggered_flanked: (bool, bool),
    ) -> Result<bool, GenieError> {
        if !column_line_slots(formation, g, column, staggered_flanked, self.world_time) {
            return Ok(false);
        }
        let after_column = matches!(formation.kind, LINE | STAGGERED | FLANKED)
            && g.previous_formation
                .as_ref()
                .is_some_and(|p| p.kind == COLUMN);
        self.lay_units(formation, g, after_column)?;
        Ok(true)
    }
}

/// `FUN_140f96030`: the walked level's top ancestor (the paths are walked first), none for a
/// level without a parent.
const fn top_level(paths: &[PathData]) -> Option<&PathData> {
    if 2 <= paths.len() { paths.last() } else { None }
}

/// `OnLastHighLevelWP` (`FUN_141048180`): the walked level's top ancestor holds a path whose last
/// waypoint (its points less one) the level has reached.
#[must_use]
pub fn on_last_high_level_waypoint(paths: &[PathData]) -> bool {
    top_level(paths)
        .filter(|t| t.has_path)
        .is_some_and(|t| t.points.len() as i32 - 1 <= t.waypoint)
}

/// `FUN_141046500`: the path of the walked level's top ancestor, or, without one, the walked
/// level's own.
pub(crate) fn top_path(paths: &[PathData]) -> Option<&PathData> {
    top_level(paths).map_or_else(
        || paths.first().filter(|w| w.has_path),
        |top| Some(top).filter(|t| t.has_path),
    )
}

/// The path `UpdateDesiredPositions` measures its way along: the top ancestor's (found by its id
/// `+0x20`, `FUN_1415419d0`), else the walked level's own (`+0x18`); a level whose path the
/// game holds by pointer holds its id too (`FUN_140f9b7c0` sets both), so
/// [`PathData::has_path`] stands for either.
fn measured_path(paths: &[PathData]) -> Option<&PathData> {
    top_level(paths)
        .filter(|t| t.has_path)
        .or_else(|| paths.first().filter(|w| w.has_path))
}

impl Turns {
    /// The type `UpdateDesiredPositions` (`FUN_14103fd00`) chooses itself for group
    /// `g` (given none and its lock -1 or 0): a line (2), unless its path (the top level's, else the
    /// walked level's, [`measured_path`]) makes it a column (1): off the last high waypoint
    /// ([`on_last_high_level_waypoint`]) or on a patrol (the top path's flags bit 0,
    /// `FUN_141046500`) always; on the last waypoint, beyond both the owner's minimum column
    /// distance (`+0x6b8`) and its column-to-line distance (`+0x6bc`, plus the target's radius
    /// and range when the group has a target), measured on the ground from the group's place to
    /// the path's last point (the origin for a path of fewer than two points); the game tests a
    /// current column against the one and any other type against the other first, which the
    /// outcome does not depend on. Flags bit 2 (`MinAutoDistanceLock`) make it a line, a group of horde units
    /// only ([`Turns::horde_units_only`]) a horde (0).
    ///
    /// # Errors
    ///
    /// A member of a master the slice does not have.
    pub fn own_formation_type(&self, g: &UnitGroup) -> Result<i32, GenieError> {
        let current = g.current_formation.as_ref().map_or(HORDE, |f| f.kind);
        let mut kind = LINE;
        if let Some(path) = measured_path(&g.motion.paths) {
            let end = if 1 < path.points.len() {
                path.points[path.points.len() - 1]
            } else {
                [0.0; 3]
            };
            let mut d = sub(end, g.motion.position[0]);
            d[1] = 0.0;
            let formations = usize::try_from(g.owner)
                .ok()
                .and_then(|p| self.players.get(p))
                .map(|p| p.formations)
                .unwrap_or_default();
            let mut column_to_line = formations.column_to_line_distance as f32;
            let min_column = formations.min_column_distance as f32;
            if g.current_target.target != -1 {
                column_to_line =
                    column_to_line + g.current_target.target_radius + g.current_target.range;
            }
            let remaining = crate::fmath::sqrt(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
            let patrol = top_path(&g.motion.paths).is_some_and(|p| p.path_flags & 1 != 0);
            if on_last_high_level_waypoint(&g.motion.paths) && !patrol {
                if (current != COLUMN || column_to_line <= remaining)
                    && (current == COLUMN || min_column <= remaining)
                    && min_column < remaining
                    && column_to_line < remaining
                {
                    kind = COLUMN;
                }
            } else {
                kind = COLUMN;
            }
        }
        if g.flags & 4 != 0 {
            kind = LINE;
        }
        if self.horde_units_only(g)? {
            kind = HORDE;
        }
        Ok(kind)
    }

    /// The type `UpdateDesiredPositions` (`FUN_14103fd00`) lays group `g` out in for the type
    /// `given`: the given one, else (-1) the group's lock (`mFormationLockedType`), or with a
    /// lock of -1 or 0 its own choice ([`Turns::own_formation_type`]).
    ///
    /// # Errors
    ///
    /// As [`Turns::own_formation_type`].
    pub fn formation_type(&self, g: &UnitGroup, given: i32) -> Result<i32, GenieError> {
        Ok(if given != -1 {
            given
        } else if matches!(g.formation_locked_type, -1 | 0) {
            self.own_formation_type(g)?
        } else {
            g.formation_locked_type
        })
    }

    /// `UpdateDesiredPositions` (`FUN_14103fd00`) of group `group` with the type `given` (-1 for
    /// none): nothing without members. A group without a formation gets an empty one
    /// ([`Formation::new`]); the type is [`Turns::formation_type`]'s. A type other
    /// than the formation's, or a given one, makes the formation the previous one and a new one
    /// the current. A formation not yet of the type with a slot per member is laid out: a line,
    /// staggered or flanked line or a column by createColumnLine
    /// ([`Turns::create_column_line`]), a phalanx and a box by their own
    /// ([`Turns::create_phalanx`], [`Turns::create_box`]), each with its transform and units; a
    /// horde and a user defined formation from the members' places ([`Turns::create_horde`],
    /// [`Turns::create_udf`]) and then transformed ([`Turns::formation_transform`], `param_4`
    /// 0); a formation already laid out, or of a type the switch does not know (6), is only
    /// transformed.
    ///
    /// The walked path the type's choice measures ([`measured_path`]) is the top level's only by
    /// its id and the walked level's only by its pointer in the game; the slice's
    /// [`PathData::has_path`] stands for either.
    ///
    /// # Errors
    ///
    /// A member of a master the slice does not have, and the
    /// layouts' and the transform's.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: the game's routine gets the group itself, from a
    /// caller that holds it.
    pub fn update_desired_positions(&mut self, group: i32, given: i32) -> Result<(), GenieError> {
        let g = self.groups.get(group).expect("the caller holds the group");
        if g.members.is_empty() {
            return Ok(());
        }
        let kind = self.formation_type(g, given)?;
        let current_kind = g.current_formation.as_ref().map_or(HORDE, |f| f.kind);
        let renew = current_kind != kind || given != -1;
        let laid_out = !renew
            && g.current_formation
                .as_ref()
                .is_some_and(|f| f.kind == kind && f.slots.len() == g.members.len());
        let g = self.groups.get_mut(group).expect("the group read above");
        if g.current_formation.is_none() {
            g.current_formation = Some(Formation::new());
        }
        if renew {
            g.previous_formation = g.current_formation.take();
            g.current_formation = Some(Formation::new());
        }
        let mut f = g.current_formation.take().expect("made above");
        let g = self.groups.get(group).expect("the group read above");
        let result = match kind {
            LINE | STAGGERED | FLANKED | COLUMN if !laid_out => self
                .create_column_line(
                    &mut f,
                    g,
                    kind == COLUMN,
                    (kind == STAGGERED, kind == FLANKED),
                )
                .map(|_| ()),
            PHALANX if !laid_out => self.create_phalanx(&mut f, g).map(|_| ()),
            BOX if !laid_out => self.create_box(&mut f, g).map(|_| ()),
            // true: the group has members; the transform after (`LAB_141040abf`)
            HORDE if !laid_out => self
                .create_horde(&mut f, g)
                .and_then(|_| self.formation_transform(&mut f, g, false))
                .map(|_| ()),
            USER_DEFINED if !laid_out => self
                .create_udf(&mut f, g)
                .and_then(|_| self.formation_transform(&mut f, g, false))
                .map(|_| ()),
            // laid out already, or a type the switch does not know: the transform alone
            _ => self.formation_transform(&mut f, g, false).map(|_| ()),
        };
        self.groups
            .get_mut(group)
            .expect("the group read above")
            .current_formation = Some(f);
        result
    }
}

/// A turn of more than this (`DAT_142eb0c64`, π/2) makes a formation of fewer than two
/// sub-formations take its units anew (`FUN_14103a1f0`).
const RIGHT_ANGLE: f32 = crate::image::group::RIGHT_ANGLE;
/// A slot's place kept this far inside the map's far edges (`FUN_14100b760`, `DAT_142eb5690`).
const MAP_EDGE: f32 = crate::image::group::MAP_EDGE;

/// `FUN_14100b760`: a place kept on the map: below 0 it is 0, from the map's width on it is the
/// width less [`MAP_EDGE`], on both axes (the y test against the height, its value the width's).
pub(super) fn on_map(mut p: Vec3, map: [i32; 2]) -> Vec3 {
    if p[0] < 0.0 {
        p[0] = 0.0;
    }
    if p[2] < 0.0 {
        p[2] = 0.0;
    }
    let (w, h) = (map[0] as f32, map[1] as f32);
    if w <= p[0] {
        p[0] = w - MAP_EDGE;
    }
    if h <= p[2] {
        p[2] = w - MAP_EDGE;
    }
    p
}

/// `FUN_14103cfe0`: slot `index` of `formation` placed at `point` plus its offset (turned by
/// `rotation` but for the transform types 2 and 3) as its transform and desired transform
/// positions, kept on the map, with the world's time; once per transform (`+0x44`). Nothing for
/// a slot outside the slots in use or a point at the origin (shorter than 1e-7).
fn transform_slot(
    formation: &mut Formation,
    rotation: &[Vec3; 3],
    point: Vec3,
    index: i32,
    transform_type: i32,
    (world_time, map): (u32, [i32; 2]),
) -> bool {
    if index < 0 || formation.slots.len() as i32 <= index {
        return false;
    }
    if crate::fmath::sqrt(crate::fmath::dot(point, point)).abs() < EPSILON {
        return false;
    }
    let slot = &mut formation.slots[index as usize];
    if slot.transformed != 1 {
        let offset = if matches!(transform_type, 2 | 3) {
            slot.offset
        } else {
            crate::fmath::rotate(rotation, slot.offset)
        };
        let at = add(point, offset);
        slot.desired_transform_position = on_map(at, map);
        slot.transform_position = on_map(at, map);
        slot.transformed = 1;
        slot.time = world_time;
    }
    true
}

/// `FindPointFromFront` (`FUN_141016990`): the point `distance` back along the stored
/// positions `line` from its front (its first point), on the ground between each point and the
/// next (their heights apart ignored, the height the earlier point's), none for a line of fewer
/// than two points or one shorter than the distance; a distance of 0 or less is the front point.
/// The forward and right it also gives (the two points' halfway, normalized, `FUN_141027d00`)
/// only reach the transform's log.
#[must_use]
pub fn find_point_from_front(line: &crate::group::PointLine, distance: f32) -> Option<Vec3> {
    if line.points.len() < 2 {
        return None;
    }
    let ground = |i: usize| {
        let mut d = sub(line.points[i + 1].point, line.points[i].point);
        d[1] = 0.0;
        d
    };
    let mut node = 0;
    let mut diff = ground(0);
    let mut length = crate::fmath::sqrt(diff[0] * diff[0] + diff[1] * diff[1] + diff[2] * diff[2]);
    let mut covered = 0.0f32;
    while 0.0 <= length.abs() {
        let total = covered + length;
        if distance <= total {
            let p = line.points[node].point;
            if distance <= 0.0 {
                return Some(p);
            }
            let k = (distance - covered) / length;
            let along = [k * diff[0], k * diff[1], k * diff[2]];
            return Some([along[0] + p[0], along[1] + p[1], p[2] + along[2]]);
        }
        node += 1;
        if line.points.len() <= node + 1 {
            return None;
        }
        diff = ground(node);
        length = crate::fmath::sqrt(diff[0] * diff[0] + 0.0 + diff[2] * diff[2]);
        covered = total;
    }
    None
}

/// The passability's box starts at this (`DAT_142eea788`) on its low sides, 0 on its high ones.
const PASS_FAR: f32 = crate::image::group::PASS_FAR;
/// The passability's grid holds the box plus this (`DAT_142eb0f50`) and one more cell
/// ([`PASS_ONE`]) a side.
const PASS_MARGIN: f32 = crate::image::group::PASS_MARGIN;
/// The grid starts this far below the box (`DAT_142ed6f20`).
const PASS_HALF_MARGIN: f32 = crate::image::group::PASS_HALF_MARGIN;
/// Below this share of the slots' array reachable (`DAT_142eae8d0`), only the nearest slot is.
const PASS_SHARE: f32 = crate::image::group::PASS_SHARE;
/// The grid's extra cell, and the spacing below which slots are nudged (`DAT_14330eb8c`).
const PASS_ONE: f32 = crate::image::group::PASS_ONE;
/// A slot lies in its cell's middle (`DAT_142e7f384`).
const PASS_HALF: f32 = crate::image::group::PASS_HALF;
/// The grid's sides are at most this many cells (the literal `0x64` at `0x14103bf01`).
const PASS_CELLS: i32 = 100;
/// The grid's bytes (`local_2948`, cleared by the literal `0x2710` at `0x14103bf36`).
const GRID_BYTES: usize = 10000;
/// The nudges of a slot by its cell's count (the tables `DAT_142f49470` and `DAT_142f49dd0`
/// with the literals 0 and 0.25 after them, `0x14103cd8c` and `0x14103cda4`), x and y.
const SLOT_NUDGE: [[f32; 2]; 5] = [
    [SLOT_NUDGE_X[0], SLOT_NUDGE_Z[0]],
    [SLOT_NUDGE_X[1], SLOT_NUDGE_Z[1]],
    [SLOT_NUDGE_X[2], SLOT_NUDGE_Z[2]],
    [SLOT_NUDGE_X[3], SLOT_NUDGE_Z[3]],
    [0.0, 0.25],
];
/// The capacity of `ClosestUnit`'s list of the units the slots took (`DAT_143f20710`, 0x118
/// bytes).
const USED_UNITS: usize = 70;
/// The feature's assignment's arrays (`FUN_14100a0c0`): 70 members or slots a side
/// (`DAT_143f1fb50`, `DAT_143f1fc70`, 0x118 bytes each) ...
const ASSIGNMENT_SIDE: usize = 70;
/// ... the costs and the preferences 70 × 70 (`DAT_143f161e0`, `DAT_143f1ae70`, 0x4c90 bytes
/// each) ...
const ASSIGNMENT_CELLS: usize = ASSIGNMENT_SIDE * ASSIGNMENT_SIDE;
/// ... and the taken marks 80 bytes (`DAT_143f1fb00`).
const ASSIGNMENT_MARKS: usize = 80;

impl Turns {
    /// `FUN_141011300` of `object` (`param_1`, as the world's table gives it,
    /// [`Turns::world_object`]): -1 for none (a null object), else its master's sub-formation
    /// ([`sub_formation`], -1 for none); a ghost's master (type 25) has none.
    ///
    /// The routine's target (`param_2`) enters only the rams' rule: a target of type 80 and a
    /// unit of class 13, of master 1258, 35, 422 or 548 and of a player not a computer
    /// (`FUN_141659cf0`), are 0 while the byte `DAT_143f01837` is 1. The image holds 0 there and
    /// no instruction writes it: its only references are the reads at 0x141011603 (here) and
    /// 0x14102b73d (`readdRams`, `FUN_14102b730`); `FUN_140ecabe0`, which sets the settings'
    /// bytes before it, stops at `DAT_143f01834`. So the rule never holds and the result does
    /// not depend on the target.
    ///
    /// # Errors
    ///
    /// An object of a master the slice does not have.
    pub(crate) fn unit_category(
        &self,
        object: Option<crate::group::WorldObject<'_>>,
    ) -> Result<i32, GenieError> {
        let Some(object) = object else {
            return Ok(-1);
        };
        let master = self.master_of(object.player(), object.master_id())?;
        Ok(sub_formation(master.kind, master.hero_flag, master.formation_category).unwrap_or(-1))
    }

    /// `ClosestUnit` (`FUN_14100e370`) of group `g` to `point`: of the members the world holds
    /// of sub-formation `category` and master `sub_category` (-1 for any) that `used` does not
    /// hold, the one nearest the point (3D, `FUN_14087e980`, where it last moved to), the first
    /// of equals; -1 for none.
    ///
    /// # Errors
    ///
    /// A member of a master the slice does not have.
    pub fn closest_unit(
        &self,
        g: &UnitGroup,
        point: Vec3,
        category: i32,
        sub_category: i32,
        used: &[i32],
    ) -> Result<i32, GenieError> {
        let mut best = FARTHEST;
        let mut closest = -1;
        for m in &g.members {
            let Some(object) = self.world_object(m.id) else {
                continue;
            };
            if category != -1 && self.unit_category(Some(object))? != category {
                continue;
            }
            if sub_category != -1
                && i32::from(self.master_of(object.player(), object.master_id())?.id)
                    != sub_category
            {
                continue;
            }
            if used.contains(&m.id) {
                continue;
            }
            let at = object.moved_vector();
            let d = (point[1] - at[1]) * (point[1] - at[1])
                + (point[0] - at[0]) * (point[0] - at[0])
                + (point[2] - at[2]) * (point[2] - at[2]);
            if d < best {
                closest = m.id;
                best = d;
            }
        }
        Ok(closest)
    }

    /// The ground distance from slot place `at` to unit `id`'s (`FUN_14087e560`, its height
    /// zeroed, `sqrt(x² + 0 + y²)`), None for a unit the world does not hold.
    fn slot_distance(&self, at: Vec3, id: i32) -> Option<f32> {
        let object = self.world_object(id)?;
        let mut d = sub(at, object.moved_vector());
        d[1] = 0.0;
        Some(crate::fmath::sqrt(d[0] * d[0] + 0.0 + d[2] * d[2]))
    }

    /// `FUN_141009760` of `formation` for group `g` without the settings' flag `DAT_143f01834`
    /// (the world's saved `mUseFeatureFlags` being 1, which `World::load_world` hands to
    /// `FUN_140ecabe0`; 0 in every recording of this build), with which the game takes the
    /// feature's assignment instead ([`Turns::assign_slots_by_variance`]; the slice holds
    /// neither the flag nor that routine's process-global costs yet). The transformed active
    /// slots in use take the closest units of their sub-formation and type
    /// ([`Turns::closest_unit`]), none taken twice. Two orders are measured first, as the sum of
    /// the ground distances (slot to unit) and the count found: the slots in order, and the same
    /// with the first slot's unit of that order barred from the first slot that finds one (whose
    /// unit then takes its place in the list). The second wins unless the first is shorter or
    /// finds more; then the slots take their units in the winner's order and are filled.
    ///
    /// # Errors
    ///
    /// A member of a master the slice does not have.
    pub fn populate_slots(
        &self,
        formation: &mut Formation,
        g: &UnitGroup,
    ) -> Result<(), GenieError> {
        // what the slots ask for (the assignments change only their units)
        let asks: Vec<(Vec3, i32, i32)> = formation
            .slots
            .iter()
            .map(|s| {
                (
                    s.transform_position,
                    s.desired_category,
                    s.desired_sub_category,
                )
            })
            .collect();
        let wanted: Vec<usize> = (0..formation.slots.len())
            .filter(|&i| formation.slots[i].transformed != 0 && formation.slots[i].active != 0)
            .collect();
        let ask = |i: usize, used: &[i32]| {
            let (at, category, sub_category) = asks[i];
            self.closest_unit(g, at, category, sub_category, used)
        };
        // the slots in order
        let mut used = vec![0i32; USED_UNITS];
        let mut count = 0usize;
        let (mut sum_first, mut found_first, mut first) = (0.0f32, 0, -1);
        for &i in &wanted {
            let id = ask(i, &used[..count])?;
            let Some(d) = (id != -1)
                .then(|| self.slot_distance(asks[i].0, id))
                .flatten()
            else {
                continue;
            };
            sum_first += d;
            used[count] = id;
            count += 1;
            if first == -1 {
                first = id;
            }
            found_first += 1;
        }
        // the first unit barred from the first slot that finds one
        let (mut sum_second, mut found_second) = (0.0f32, 0);
        let mut count = 1usize;
        let mut leading = true;
        for &i in &wanted {
            let id = ask(i, &used[..count])?;
            let Some(d) = (id != -1)
                .then(|| self.slot_distance(asks[i].0, id))
                .flatten()
            else {
                continue;
            };
            sum_second += d;
            if leading {
                leading = false;
                used[0] = id;
            } else {
                used[count] = id;
                count += 1;
            }
            found_second += 1;
        }
        if sum_first < sum_second || found_second < found_first {
            let mut count = 0usize;
            for &i in &wanted {
                let id = ask(i, &used[..count])?;
                formation.slots[i].unit_id = id;
                if id != -1 {
                    formation.slots[i].position_filled = 1;
                    used[count] = id;
                    count += 1;
                }
            }
        } else {
            used[0] = first;
            let mut count = 1usize;
            let mut leading = true;
            for &i in &wanted {
                let id = ask(i, &used[..count])?;
                formation.slots[i].unit_id = id;
                if id != -1 {
                    formation.slots[i].position_filled = 1;
                    if leading {
                        leading = false;
                        used[0] = id;
                    } else {
                        used[count] = id;
                        count += 1;
                    }
                }
            }
        }
        Ok(())
    }

    /// The feature's assignment (`FUN_14100a0c0`, [`Turns::populate_slots`] with the settings'
    /// flag) of `formation`'s slots in use to group `g`'s members, `n` of them:
    ///
    /// - The costs (`costs`, the process global `DAT_143f161e0` of 70 × 70 floats that only this
    ///   routine reads and writes, all 0 as the process starts; grown to that when shorter): for
    ///   each member the world holds ([`Turns::world_object`]), slot by slot, the squared distance from where it last
    ///   moved to (slot 0x518, as slot 0x520) to the slot's place (`+0x1c`), summed x, height,
    ///   y; [`FARTHEST`] for a slot not transformed or not active, of another sub-formation
    ///   ([`Turns::unit_category`]) or, before both, of another master. A member's row starts at
    ///   its index times `n`, so with more slots than members a row runs into the next one's,
    ///   which overwrites it; a member the world does not hold leaves its row as it was.
    /// - The preferences: each member's slots by cost, each the cheapest not yet taken (the
    ///   first untaken one, then any strictly cheaper, `COMISS`/`JBE` at 0x14100a47e: a NaN cost
    ///   neither wins nor loses), in rows laid like the costs.
    /// - Each start member in turn takes its first preferred slot not taken, then the next
    ///   member (wrapping), until every slot is taken or a member has none left (a start that
    ///   placed nobody counts for nothing). Its mean is the sum of the costs over `n`, its
    ///   variance the mean of the squared differences over the pairs placed; a variance below
    ///   the best so far (`COMISS`/`JBE` at 0x14100a9be, from [`FARTHEST`]) keeps the start's
    ///   pairs.
    /// - Each kept pair's slot takes its member, filled (`+0x34`); the other slots are left as
    ///   they are, and a member the world does not hold may take one.
    ///
    /// The routine's other arrays are the process's too but are filled before each read: the
    /// preferences for the rows read (`DAT_143f1ae70`), the taken marks (`DAT_143f1fb00`), the
    /// start's pairs (`DAT_143f1fd90`, `DAT_143f1feb0`, -1 each start) and the kept ones
    /// (`DAT_143f1fb50`, `DAT_143f1fc70`, -1 at the start).
    ///
    /// # Errors
    ///
    /// A held member of a master the slice does not have.
    ///
    /// # Panics
    ///
    /// For more than 70 members or slots, which the game's arrays do not hold either: a group
    /// takes fewer than 70 members ([`crate::group::create`]'s `AddUnit`) and its layouts a slot
    /// each.
    pub fn assign_slots_by_variance(
        &self,
        formation: &mut Formation,
        g: &UnitGroup,
        costs: &mut Vec<f32>,
    ) -> Result<(), GenieError> {
        let n = g.members.len();
        let s = formation.slots.len();
        if costs.len() < ASSIGNMENT_CELLS {
            costs.resize(ASSIGNMENT_CELLS, 0.0);
        }
        for (i, m) in g.members.iter().enumerate() {
            let Some(object) = self.world_object(m.id) else {
                continue;
            };
            if s == 0 {
                continue;
            }
            let category = self.unit_category(Some(object))?;
            let master = i32::from(object.master_id());
            let at = object.moved_vector();
            for (j, slot) in formation.slots.iter().enumerate() {
                let mut skip = slot.transformed == 0 || slot.active == 0;
                if slot.desired_category != -1 && category != slot.desired_category {
                    skip = true;
                }
                let other_master =
                    slot.desired_sub_category != -1 && master != slot.desired_sub_category;
                costs[i * n + j] = if other_master || skip {
                    FARTHEST
                } else {
                    let d = sub(at, slot.transform_position);
                    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
                };
            }
        }
        let mut preferences = vec![-1i32; ASSIGNMENT_CELLS];
        let mut taken = [false; ASSIGNMENT_MARKS];
        for i in 0..n {
            let row = i * n;
            taken[..s].fill(false);
            for k in 0..s {
                let first = taken[..s].iter().position(|&t| !t).unwrap_or(s);
                let mut best = first;
                for j in first + 1..s {
                    if !taken[j] && costs[row + j] < costs[row + best] {
                        best = j;
                    }
                }
                preferences[row + k] = best as i32;
                taken[best] = true;
            }
        }
        let mut kept = ([-1i32; ASSIGNMENT_SIDE], [-1i32; ASSIGNMENT_SIDE]);
        let mut best_variance = FARTHEST;
        for start in 0..n {
            if s == 0 {
                continue;
            }
            taken[..s].fill(false);
            let mut pairs = ([-1i32; ASSIGNMENT_SIDE], [-1i32; ASSIGNMENT_SIDE]);
            let (mut placed, mut sum, mut member) = (0usize, 0.0f32, start);
            while placed < s {
                let row = member * n;
                let mut l = 0;
                let mut slot = preferences[row];
                while slot != -1 && taken[slot as usize] {
                    l += 1;
                    if s <= l {
                        slot = -1;
                        break;
                    }
                    slot = preferences[row + l];
                }
                if slot == -1 {
                    break;
                }
                pairs.0[placed] = member as i32;
                pairs.1[placed] = slot;
                taken[slot as usize] = true;
                sum += costs[row + slot as usize];
                member = if member + 1 < n { member + 1 } else { 0 };
                placed += 1;
            }
            if placed == 0 {
                continue;
            }
            let mean = sum / n as f32;
            let (mut variance, mut count) = (0.0f32, 0i32);
            for (&r, &c) in pairs.0[..s].iter().zip(&pairs.1[..s]) {
                if r != -1 && c != -1 {
                    count += 1;
                    let d = costs[r as usize * n + c as usize] - mean;
                    variance += d * d;
                }
            }
            variance /= count as f32;
            if variance < best_variance {
                best_variance = variance;
                kept.0[..s].copy_from_slice(&pairs.0[..s]);
                kept.1[..s].copy_from_slice(&pairs.1[..s]);
            }
        }
        for (&r, &c) in kept.0[..n].iter().zip(&kept.1[..n]) {
            if r == -1 {
                continue;
            }
            let slot = &mut formation.slots[c as usize];
            slot.position_filled = 1;
            slot.unit_id = g.members[r as usize].id;
        }
        Ok(())
    }

    /// `populateSlotsWithUnits` (`FUN_14102b230`) of group `g` over the first `n` slots of
    /// `formation` (at most the members), `order` their places in the line order: the members
    /// whose index has a transformed active slot (and that the world holds) form a list in
    /// member order; then, for each `k` of the list, the slot whose place in `order` holds `k`
    /// takes the first unit of the list left that the member of the same index (as the game
    /// reads it) matches in sub-formation and master (-1 any), filled.
    ///
    /// # Errors
    ///
    /// A member of a master the slice does not have.
    ///
    /// # Panics
    ///
    /// Never for [`Turns::from_previous_column`]'s order, a permutation of the places (the game
    /// reads past one that lacks a place).
    fn populate_in_order(
        &self,
        formation: &mut Formation,
        g: &UnitGroup,
        n: usize,
        order: &[i32],
    ) -> Result<(), GenieError> {
        let n = n.min(g.members.len());
        let mut list: Vec<i32> = Vec::new();
        for i in 0..n {
            let s = formation.slot_mut(i);
            if s.transformed != 0 && s.active != 0 && self.world_object(g.members[i].id).is_some() {
                list.push(g.members[i].id);
            }
        }
        for k in 0..list.len() {
            let at = order
                .iter()
                .position(|&o| o == k as i32)
                .expect("the line order holds every place up to the list's length");
            let (category, sub_category) = {
                let s = formation.slot_mut(at);
                (s.desired_category, s.desired_sub_category)
            };
            for (u, taken) in list.iter_mut().enumerate() {
                if *taken == -1 {
                    continue;
                }
                let Some(object) = g.members.get(u).and_then(|m| self.world_object(m.id)) else {
                    continue;
                };
                if category != -1 && self.unit_category(Some(object))? != category {
                    continue;
                }
                if sub_category != -1
                    && i32::from(self.master_of(object.player(), object.master_id())?.id)
                        != sub_category
                {
                    continue;
                }
                let s = formation.slot_mut(at);
                s.unit_id = *taken;
                s.position_filled = 1;
                *taken = -1;
                break;
            }
        }
        Ok(())
    }

    /// `FUN_141009060`: a line (not a column or a box) after a column (`previous`) takes its
    /// units line by line from the middle out: with a whole number of lines transformed (the
    /// transformed slots a multiple of the width), each line's places run from its middle
    /// (left of it for an even width) alternately right and left ([`Turns::populate_in_order`]
    /// over width times depth); else the slots take the closest units
    /// ([`Turns::populate_slots`]). Nothing without a previous column.
    ///
    /// # Errors
    ///
    /// As the routines it runs.
    ///
    /// # Panics
    ///
    /// For a line of no width, which the game divides by too (createColumnLine lays a line of a
    /// column at least).
    pub fn from_previous_column(
        &self,
        formation: &mut Formation,
        g: &UnitGroup,
    ) -> Result<(), GenieError> {
        let previous_column = g
            .previous_formation
            .as_ref()
            .is_some_and(|p| p.kind == COLUMN);
        if matches!(formation.kind, COLUMN | BOX) || !previous_column {
            return Ok(());
        }
        let width = formation.maximum_width;
        if formation.number_transformed_slots % width != 0 {
            return self.populate_slots(formation, g);
        }
        let depth = formation.maximum_depth;
        let mut order = vec![0i32; (width * depth).max(0) as usize];
        for line in 0..depth {
            let base = (width * line) as usize;
            let mut start = width / 2 + width * line;
            let mut magnitude = -1;
            if width & 1 == 0 {
                magnitude = 1;
                start -= 1;
            }
            order[base] = start;
            let mut uses = 0;
            for k in 1..width {
                let v = order[base] + magnitude;
                order[base + k as usize] = v;
                if (line + 1) * width - 1 < v || v < width * line {
                    break;
                }
                magnitude = -magnitude;
                uses += 1;
                if 1 < uses {
                    uses = 0;
                    magnitude += if magnitude < 1 { -1 } else { 1 };
                }
            }
        }
        self.populate_in_order(formation, g, (depth * width).max(0) as usize, &order)
    }

    /// createColumnLine's end (`FUN_141012280` after its layout), and the phalanx's and the
    /// box's (`after_column` false): the transform ([`Turns::formation_transform`]); unless it
    /// took the units anew, every slot of the array loses its unit and its filled mark, and the
    /// units take their slots: after a column with slots transformed, the line keeps the
    /// column's order ([`Turns::from_previous_column`]), else the closest
    /// ([`Turns::populate_slots`]).
    ///
    /// # Errors
    ///
    /// As the routines it runs.
    pub fn lay_units(
        &self,
        formation: &mut Formation,
        g: &UnitGroup,
        after_column: bool,
    ) -> Result<(), GenieError> {
        if self.formation_transform(formation, g, after_column)? {
            return Ok(());
        }
        formation.clear_units();
        if formation.number_transformed_slots < 1 || g.previous_formation.is_none() || !after_column
        {
            self.populate_slots(formation, g)
        } else {
            self.from_previous_column(formation, g)
        }
    }

    /// `FormationTransform` (`FUN_14103a1f0`) of `formation` for group `g`, `column_to_line`
    /// (`param_4`) a line laid out after a column: nothing without slots in use. The group's yaw
    /// from the canon forward (x, or -x about faced) and the formation's last one
    /// (`calculateYawAngle`) give the rotation about the group's up; the formation takes the
    /// group's forward as its orientation, every slot's transform mark clears and none counts as
    /// transformed. The line transform (type 0) takes each line's point behind the group (its
    /// distance back, or its index times the group's spacing): the stored positions' point there
    /// ([`find_point_from_front`]), else the group's place less its forward times the distance;
    /// each of the line's slots is placed there ([`transform_slot`]) and the line counts as
    /// transformed (below 256 places), while fewer than the array's slots are. The group (1) and
    /// horde (3) transforms place every slot of the array from the group's place, the target
    /// transform (2) from the target's original place, all counting. A turn of more than
    /// [`RIGHT_ANGLE`] with fewer than two sub-formations takes the units anew: every slot of
    /// the array loses its unit and its filled mark and the slots take their units (from the
    /// previous column, [`Turns::from_previous_column`], for `column_to_line`, else the closest,
    /// [`Turns::populate_slots`]). Then the slots' passability ([`Turns::passable_slots`]).
    /// Returns whether the slots took their units anew (`*param_3`).
    ///
    /// # Errors
    ///
    /// A formation line past the lines' array, and the routines it runs.
    pub fn formation_transform(
        &self,
        formation: &mut Formation,
        g: &UnitGroup,
        column_to_line: bool,
    ) -> Result<bool, GenieError> {
        if formation.slots.is_empty() {
            return Ok(false);
        }
        let world = (self.world_time, self.spatial.map);
        let canon = [
            if formation.about_faced == 0 {
                1.0
            } else {
                -1.0
            },
            0.0,
            0.0,
        ];
        let [position, forward, up, _right] = g.motion.position;
        // calculateYawAngle without its clamp (`param_5` 0), with the world's update time
        let dt = self.dt_seconds;
        let yaw = g.motion.calculate_yaw_angle(canon, forward, dt, false);
        let old = g
            .motion
            .calculate_yaw_angle(canon, formation.transform_orientation, dt, false);
        let flips = RIGHT_ANGLE < (yaw - old).abs() && g.unit_types.len() < 2;
        let rotation = crate::fmath::rotation(yaw, up);
        formation.transform_orientation = forward;
        for s in formation.every_slot_mut() {
            s.transformed = 0;
        }
        formation.number_transformed_slots = 0;
        let capacity = formation.slot_capacity() as i32;
        match formation.transform_type {
            0 => {
                for i in 0..formation.number_lines.max(0) {
                    if capacity <= formation.number_transformed_slots {
                        break;
                    }
                    let line = formation
                        .lines
                        .get(i as usize)
                        .ok_or(GenieError::unsupported(
                            "a formation line past the lines' array",
                        ))?;
                    let mut back = 0.0;
                    if 0 < i {
                        back = line.distance_back;
                        // COMISS/JA at 0x14103a850: a NaN distance back takes the spacing too
                        if crate::fmath::at_most_or_unordered(back, 0.0) {
                            back = i as f32 * g.desired_spacing;
                        }
                    }
                    let point = find_point_from_front(&g.stored_positions, back)
                        .unwrap_or_else(|| sub(position, scale(back, forward)));
                    let indices = line.indices.clone();
                    let width = indices.len() as i32;
                    for k in indices {
                        let _ = transform_slot(formation, &rotation, point, k as i32, 0, world);
                    }
                    if width < 0x100 {
                        formation.number_transformed_slots += width;
                    }
                }
            }
            kind @ 1..=3 => {
                let point = if kind == 2 {
                    g.current_target.original_position
                } else {
                    position
                };
                let mut i = 0;
                loop {
                    let _ = transform_slot(formation, &rotation, point, i, kind, world);
                    i += 1;
                    formation.number_transformed_slots += 1;
                    if capacity <= formation.number_transformed_slots {
                        break;
                    }
                }
            }
            _ => {}
        }
        let mut took = false;
        if flips {
            formation.clear_units();
            if column_to_line {
                self.from_previous_column(formation, g)?;
            } else {
                self.populate_slots(formation, g)?;
            }
            took = true;
        }
        self.passable_slots(formation, g)?;
        Ok(took)
    }

    /// `FormationTransform`'s passability (`FUN_14103a1f0` after the transform) of `formation`
    /// for group `g`, tested with the object of the group's lead ([`Turns::lead_object`], the
    /// lead -1 the world's object -1), else that of the first active slot's unit the world's
    /// table holds ([`Turns::world_object`], an empty slot's -1 too; none: nothing). The slots' places are their transform positions
    /// (`+0x1c`, the save's `mTransformPosition`). At the first active slot whose place's tile
    /// the tester cannot stand on (`FUN_141185690`, [`crate::spatial::Spatial::standable`] with
    /// its master's terrain mask): the active slot nearest the group's place (3D) must be
    /// standable, else it takes the group's place on x and y if that is (else nothing more).
    /// The box around the tester's place and the active slots' places gets the
    /// temporary records of the tiles its mask lacks (`FUN_141052b90`), the members are ignored
    /// (`FUN_1410562e0`), and each active slot is reachable when the line from the group's place
    /// to it meets nothing standing ([`crate::spatial::Spatial::still_line_hit`]). With fewer
    /// than [`PASS_SHARE`] of the slots' array reachable, all are unreachable but the nearest,
    /// which takes the group's place. On a grid of the box plus [`PASS_HALF_MARGIN`] each way
    /// (at most [`PASS_CELLS`] cells a side) the reachable slots' cells count their slots (bits 1 to 5, bit
    /// 0 set) and point toward the nearest slot (bits 6 and 7: +y, +x, -y, -x). Each
    /// unreachable slot's unit then searches breadth first from its own place's cell, through
    /// cells it can stand on, x+1, x-1, y+1, y-1 in turn, for a reachable cell: one of fewer
    /// than five slots takes it (counted), a fuller one has the searching cell take its place
    /// (one slot, pointing to it). A cell found that is not the nearest slot's steps once more
    /// the way its cell points, when that cell holds at most five and the line from the unit
    /// there is clear. The slot then lies in the middle of its cell, below a spacing of
    /// [`PASS_ONE`] nudged by its count ([`SLOT_NUDGE_X`], [`SLOT_NUDGE_Z`]). The temporary
    /// records and the ignore list go after.
    ///
    /// # Errors
    ///
    /// A tester or slot unit of a master the slice does not have or of no terrain restriction,
    /// a map without its terrain, and a line query near a record whose place the slice does not
    /// know.
    ///
    /// # Panics
    ///
    /// Never: an active slot was found, so the nearest one is too (its distance is finite).
    #[allow(clippy::too_many_lines)]
    fn passable_slots(&self, formation: &mut Formation, g: &UnitGroup) -> Result<(), GenieError> {
        let tester = self.lead_object(&g.motion).or_else(|| {
            formation
                .slots
                .iter()
                .filter(|s| s.active != 0)
                .find_map(|s| self.world_object(s.unit_id))
        });
        let Some(tester) = tester else {
            return Ok(());
        };
        let mask = self.master_mask(tester.player(), tester.master_id())?;
        let stands = |at: Vec3| {
            let (x, z) = slot_tile(at);
            self.spatial.standable(x, z, mask)
        };
        let position = g.motion.position[0];
        let n = formation.slots.len();
        let Some(_) = (0..n).find(|&i| {
            let s = &formation.slots[i];
            s.active != 0 && !stands(s.transform_position)
        }) else {
            return Ok(());
        };
        // the active slot nearest the group's place, and the box around the tester and them
        let ([min_x, min_z, max_x, max_z], nearest) =
            passability_box(&formation.slots, tester.moved_vector(), position);
        let k = nearest.expect(
            "an active slot lies nearer than f32::MAX (the game reads the slot before the array \
             when none does)",
        );
        if !stands(formation.slots[k].transform_position) {
            if !stands(position) {
                return Ok(());
            }
            let s = &mut formation.slots[k];
            s.transform_position[0] = position[0];
            s.transform_position[2] = position[2];
        }
        let temporary = ([min_x, min_z, max_x, max_z], mask);
        let members: Vec<i32> = g.members.iter().map(|m| m.id).collect();
        // the members are the manager's ignore list while the passability runs
        // (FUN_1410562e0, 0x14103b916), which every way out of it clears (0x14103ce93,
        // 0x14103cf93): none after it
        self.spatial.set_manager_ignore(&[]);
        let mut reachable = Vec::new();
        let mut unreachable = Vec::new();
        for (j, t) in formation.slots.iter().enumerate() {
            if t.active == 0 {
                continue;
            }
            let p = t.transform_position;
            if self.spatial.still_line_hit(
                [position[0], position[2]],
                [p[0], p[2]],
                temporary,
                &members,
            )? {
                unreachable.push(j);
            } else {
                reachable.push(j);
            }
        }
        if (reachable.len() as f32) / (formation.slot_capacity() as f32) < PASS_SHARE {
            unreachable.append(&mut reachable);
            reachable.push(k);
            let s = &mut formation.slots[k];
            s.transform_position[0] = position[0];
            s.transform_position[2] = position[2];
        }
        // CVTTSS2SI at 0x14103bedd, 0x14103bef9, 0x14103bf1c and 0x14103bf2e
        let w = cvttss2si((max_x - min_x) + PASS_MARGIN + PASS_ONE);
        let h = cvttss2si((max_z - min_z) + PASS_MARGIN + PASS_ONE);
        if PASS_CELLS < w || PASS_CELLS < h {
            return Ok(());
        }
        let ox = cvttss2si(min_x - PASS_HALF_MARGIN);
        let oz = cvttss2si(min_z - PASS_HALF_MARGIN);
        let mut grid = vec![0u8; GRID_BYTES];
        let near = formation.slots[k].transform_position;
        for &r in &reachable {
            let p = formation.slots[r].transform_position;
            let gx = cvttss2si(p[0] - ox as f32);
            let gz = cvttss2si(p[2] - oz as f32);
            if !(0 <= gx && gx < w && 0 <= gz && gz < h) {
                continue;
            }
            let cell = (gz * w + gx) as usize;
            grid[cell] = cell_mark(grid[cell], near[0] - p[0], near[2] - p[2]);
        }
        let mut visited = vec![0u8; (w * h).max(0) as usize];
        for &u in &unreachable {
            visited.fill(0);
            let Some(unit) = self.world_object(formation.slots[u].unit_id) else {
                continue;
            };
            let unit_mask = self.master_mask(unit.player(), unit.master_id())?;
            let unit_at = unit.position();
            let gx = cvttss2si(unit_at[0] - ox as f32);
            let gy = cvttss2si(unit_at[1] - oz as f32);
            let mut found: Option<(i32, i32, usize)> = None;
            if 0 <= gx && 0 <= gy && gx < w && gy < h {
                let mut queue = vec![(gx, gy)];
                visited[(gy * w + gx) as usize] = 1;
                let mut qi = 0;
                'search: while qi < queue.len() {
                    let (cx, cy) = queue[qi];
                    qi += 1;
                    if !(0 <= cx
                        && cx < w
                        && 0 <= cy
                        && cy < h
                        && self.spatial.standable(ox + cx, oz + cy, unit_mask))
                    {
                        continue;
                    }
                    let steps = [
                        (cx + 1, cy, cx + 1 < w, 0x43u8),
                        (cx - 1, cy, 0 < cx, 0xc3),
                        (cx, cy + 1, cy + 1 < h, 3),
                        (cx, cy - 1, 0 < cy, 0x83),
                    ];
                    for (nx, ny, inside, mark) in steps {
                        if !inside {
                            continue;
                        }
                        let cell = (w * ny + nx) as usize;
                        if visited[cell] != 0 {
                            continue;
                        }
                        visited[cell] = 1;
                        let b = grid[cell];
                        if b & 1 != 0 {
                            if b & 0x3e < 10 {
                                found = Some((ox + nx, oz + ny, usize::from(b >> 1 & 0x1f)));
                                grid[cell] = count_up(b);
                            } else {
                                found = Some((ox + cx, oz + cy, 0));
                                grid[(w * cy + cx) as usize] = mark;
                            }
                            break 'search;
                        }
                        queue.push((nx, ny));
                    }
                }
            }
            let Some((fx, fy, nudge)) = found else {
                continue;
            };
            let (mut x, mut y) = (fx, fy);
            // CVTTSS2SI at 0x14103cb6a and 0x14103cb70
            if slot_tile(formation.slots[k].transform_position) != (fx, fy) {
                let (rx, ry) = (fx - ox, fy - oz);
                let (ddx, ddy) = match grid[(w * ry + rx) as usize] >> 6 {
                    0 => (0, 1),
                    1 => (1, 0),
                    2 => (0, -1),
                    _ => (-1, 0),
                };
                let (nx, ny) = (ddx + rx, ddy + ry);
                if ny < 0 || h <= ny || nx < 0 || w <= nx {
                    continue;
                }
                let cell = (w * ny + nx) as usize;
                let b = grid[cell];
                if b & 0x3e < 0xb
                    && !self.spatial.still_line_hit(
                        [unit_at[0], unit_at[1]],
                        [(ddx + fx) as f32 + PASS_HALF, (fy + ddy) as f32 + PASS_HALF],
                        temporary,
                        &members,
                    )?
                {
                    grid[cell] = count_up(b);
                    y = fy + ddy;
                    x = ddx + fx;
                }
            }
            let s = &mut formation.slots[u];
            // COMISS/JC at 0x14103cde3: a NaN spacing nudges too
            if crate::fmath::below_or_unordered(g.desired_spacing, PASS_ONE) {
                s.transform_position[0] = x as f32 + PASS_HALF + SLOT_NUDGE[nudge][0];
                s.transform_position[2] = y as f32 + PASS_HALF + SLOT_NUDGE[nudge][1];
            } else {
                s.transform_position[0] = x as f32 + PASS_HALF;
                s.transform_position[2] = y as f32 + PASS_HALF;
            }
        }
        Ok(())
    }
}

/// `FUN_14103a1f0`'s grid cell count up: bits 1 to 5 (the slots a cell holds) count on (mod 32),
/// the others kept.
const fn count_up(b: u8) -> u8 {
    (((b & 0xfe).wrapping_add(2) ^ b) & 0x3e) ^ b
}

/// The tile of a slot's place the passability tests (`FUN_141185690`'s arguments): x and y
/// by `CVTTSS2SI` (0x14103b5df, 0x14103b777), a NaN or out of range one `i32::MIN`.
fn slot_tile(at: Vec3) -> (i32, i32) {
    (cvttss2si(at[0]), cvttss2si(at[2]))
}

/// The passability's box and nearest slot (0x14103b650..0x14103b758): from the box
/// ([`PASS_FAR`], [`PASS_FAR`]) to (0, 0), each active slot of `slots` widens it first by the
/// tester's place `at` (slot 0x520) and then by its own, as `MINSS new, old` and `MAXSS new, old`
/// take them (the old bound kept for a NaN either side); the nearest is the first active slot
/// whose squared distance to the group's place `position` (summed x, height, y) lies below the
/// best so far, from [`FARTHEST`] (`COMISS`/`CMOVBE`: a NaN distance keeps the earlier one, none
/// when no slot lies below `f32::MAX`). Returns min x, min y, max x, max y and the nearest.
fn passability_box(slots: &[Slot], at: Vec3, position: Vec3) -> ([f32; 4], Option<usize>) {
    use crate::fmath::{maxss, minss};
    let (mut min_x, mut min_z, mut max_x, mut max_z) = (PASS_FAR, PASS_FAR, 0.0f32, 0.0f32);
    let mut best = FARTHEST;
    let mut nearest = None;
    for (j, t) in slots.iter().enumerate() {
        if t.active == 0 {
            continue;
        }
        let d3 = sub(t.transform_position, position);
        let d = d3[0] * d3[0] + d3[1] * d3[1] + d3[2] * d3[2];
        let earlier = best;
        if d < best {
            best = d;
        }
        let (a, b) = (minss(at[0], min_x), minss(at[2], min_z));
        let (c, e) = (maxss(at[0], max_x), maxss(at[2], max_z));
        let p = t.transform_position;
        min_x = minss(p[0], a);
        min_z = minss(p[2], b);
        max_x = maxss(p[0], c);
        max_z = maxss(p[2], e);
        if d < earlier {
            nearest = Some(j);
        }
    }
    ([min_x, min_z, max_x, max_z], nearest)
}

/// A reachable slot's grid cell mark (0x14103bfd4..0x14103c046): the cell's count up
/// ([`count_up`]) with bit 0, then its way toward the nearest slot `dx`, `dz` off: along y when
/// `|dx|` is at most `|dz|` (`COMISS`/`JBE`: also for a NaN), with bit 7 for a `dz` below 0
/// (`JNC`: also NaN); else along x, bits 6 and 7 for a `dx` below 0 (`JC`: also NaN), bit 6 alone
/// otherwise.
fn cell_mark(b: u8, dx: f32, dz: f32) -> u8 {
    use crate::fmath::{at_most_or_unordered, below_or_unordered};
    let b = count_up(b) | 1;
    if at_most_or_unordered(dx.abs(), dz.abs()) {
        if below_or_unordered(dz, 0.0) {
            b & 0x3f | 0x80
        } else {
            b & 0x3f
        }
    } else if below_or_unordered(dx, 0.0) {
        b | 0xc0
    } else {
        b & 0x3f | 0x40
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// `FUN_14102d580` copies a slot in use without its transform mark (`+0x44`).
    #[test]
    fn a_larger_slots_array_drops_the_transform_marks() {
        let mut f = Formation::new();
        f.resize_slots(1);
        f.use_slots(1);
        f.slots[0].unit_id = 7;
        f.slots[0].transformed = 1;
        f.resize_slots(4);
        assert_eq!((f.slots[0].unit_id, f.slots[0].transformed), (7, 0));
        assert_eq!(f.slot_capacity(), 4);
    }

    /// A 20 by 20 map of terrain class 1 but class 2 on `blocked`, masters 4 and 5 (category 1)
    /// and 6 (category 3), and player 1's `units` (id, master, x, y) on it and on the
    /// obstruction manager, all in group 1 at the origin facing x.
    pub(crate) fn slot_world(units: &[(i32, i16, f32, f32)], blocked: &[(usize, usize)]) -> Turns {
        use crate::spatial::{Obstruction, Placed, Spatial, Tiles};
        use crate::terrain::{Terrain, TerrainKind};
        let objects: Vec<crate::turn::Unit> = units
            .iter()
            .map(|&(id, master, x, y)| {
                let mut u = crate::turn::Unit {
                    id,
                    position: [x, y, 0.0],
                    ..crate::turn::Unit::default()
                };
                u.life.master_id = master;
                u
            })
            .collect();
        let player = crate::turn::Player {
            objects: objects.clone(),
            ..crate::turn::Player::default()
        };
        let mut t = Turns::new(
            0,
            0,
            0,
            vec![0, 0],
            vec![crate::turn::Player::default(), player],
        );
        let master = |id: i16, category: u8| crate::masters::Master {
            id,
            kind: 70,
            formation_category: category,
            terrain_restriction: 0,
            radius: [0.2, 0.2],
            ..crate::masters::Master::default()
        };
        let masters = crate::masters::Masters::new([master(4, 1), master(5, 1), master(6, 3)]);
        t.masters = vec![masters.clone(), masters];
        let mut classes = vec![1u8; 400];
        for &(x, y) in blocked {
            classes[y * 20 + x] = 2;
        }
        t.spatial = Spatial::with_tiles([20, 20], Tiles::new(20, 20, classes));
        let kinds = vec![
            TerrainKind {
                shown: true,
                flags: 1,
                forest: false,
            },
            TerrainKind {
                shown: true,
                flags: 8,
                forest: false,
            },
        ];
        t.spatial.terrain = Some(Terrain::new(
            20,
            20,
            vec![(0, 255, -1); 400],
            kinds,
            vec![vec![1.0, 0.0]],
        ));
        for u in &objects {
            t.spatial.insert(
                u.id,
                Placed {
                    position: u.position,
                    radius: [0.2, 0.2],
                    class: 6,
                    player: 1,
                    master: u.life.master_id,
                    obstruction_type: 0,
                    buildable_on: 0,
                    tall: true,
                    speed: 0.0,
                    unknown: None,
                    obstruction: Some(Obstruction { kind: 5, class: 2 }),
                },
            );
        }
        let mut g = UnitGroup::new(1);
        g.owner = 1;
        g.members = units
            .iter()
            .map(|&(id, ..)| crate::group::Member {
                id,
                ..crate::group::Member::default()
            })
            .collect();
        g.motion.position[1] = [1.0, 0.0, 0.0];
        g.motion.position[2] = [0.0, 1.0, 0.0];
        g.desired_spacing = 2.0;
        t.groups = crate::group::UnitGroups {
            groups: vec![g],
            ..crate::group::UnitGroups::default()
        };
        t
    }

    /// A slot at `at` (x, height, y) of category `category`, transformed and active.
    pub(crate) fn slot_at(at: Vec3, category: i32) -> Slot {
        Slot {
            unit_id: -1,
            transform_position: at,
            desired_category: category,
            desired_sub_category: -1,
            active: 1,
            transformed: 1,
            ..Slot::default()
        }
    }

    /// `FUN_141016990`: along the stored positions on the ground from the front (the first
    /// point): 0 is the front, 2.5 halfway along the first stretch (5 long), 8 halfway along the
    /// second (6 long) at the second point's height, 11 its end; 12 is past the line, and one
    /// point is no line.
    #[test]
    fn the_point_from_the_front_runs_along_the_stored_positions() {
        let point = |p: Vec3| crate::group::LinePoint {
            point: p,
            ..crate::group::LinePoint::default()
        };
        let line = crate::group::PointLine {
            points: vec![
                point([0.0, 7.0, 0.0]),
                point([3.0, 1.0, 4.0]),
                point([3.0, 0.0, 10.0]),
            ],
            ..crate::group::PointLine::default()
        };
        assert_eq!(find_point_from_front(&line, 0.0), Some([0.0, 7.0, 0.0]));
        assert_eq!(find_point_from_front(&line, -1.0), Some([0.0, 7.0, 0.0]));
        assert_eq!(find_point_from_front(&line, 2.5), Some([1.5, 7.0, 2.0]));
        assert_eq!(find_point_from_front(&line, 8.0), Some([3.0, 1.0, 7.0]));
        assert_eq!(find_point_from_front(&line, 11.0), Some([3.0, 1.0, 10.0]));
        assert_eq!(find_point_from_front(&line, 12.0), None);
        let one = crate::group::PointLine {
            points: vec![point([0.0; 3])],
            ..crate::group::PointLine::default()
        };
        assert_eq!(find_point_from_front(&one, 0.0), None);
    }

    /// `FUN_141011300`: a null object (none in the world's table) is -1; a ghost (its master 243
    /// of type 25) is below 70, -1; a ram (class 13, master 35, of a human player) keeps its own
    /// category (2), as the rams' rule never holds; a combat master with hero flag bit 4 is 5.
    #[test]
    fn a_unit_s_category_is_its_masters_and_none_for_a_null_object_or_a_ghost() {
        let mut t = slot_world(&[(10, 4, 1.0, 0.0), (35, 35, 2.0, 0.0)], &[]);
        for masters in &mut t.masters {
            *masters = crate::masters::Masters::new([
                crate::masters::Master {
                    id: 4,
                    kind: 70,
                    formation_category: 1,
                    hero_flag: 0x10,
                    ..crate::masters::Master::default()
                },
                crate::masters::Master {
                    id: 35,
                    kind: 70,
                    class: 13,
                    formation_category: 2,
                    ..crate::masters::Master::default()
                },
                crate::masters::Master {
                    id: crate::ghost::GHOST_MASTER,
                    kind: crate::ghost::GHOST,
                    formation_category: 1,
                    ..crate::masters::Master::default()
                },
            ]);
        }
        t.players[1].kind = 1;
        t.players[1].ghosts = vec![crate::ghost::Ghost {
            id: -1,
            original: None,
            original_id: -1,
            original_master: None,
            original_owner: 1,
            position: [3.0, 4.0, 0.0],
            tile: [3, 4],
            footprint: 0,
            bits: 0,
            cant_see: 0,
            seen: false,
            state: 2,
        }];
        assert_eq!(t.unit_category(None).unwrap(), -1);
        assert_eq!(t.unit_category(t.world_object(99)).unwrap(), -1);
        let ghost = t.world_object(-1);
        assert!(matches!(
            ghost,
            Some(crate::group::WorldObject::Ghost(1, _))
        ));
        assert_eq!(t.unit_category(ghost).unwrap(), -1);
        assert_eq!(t.unit_category(t.world_object(35)).unwrap(), 2);
        assert_eq!(t.unit_category(t.world_object(10)).unwrap(), 5);
        // the world's table holds ghosts only in its negative half, and none for i32::MIN
        assert!(t.world_object(i32::MIN).is_none());
    }

    /// `FUN_14100e370` from the origin: of all, 12 (category 3) is nearest; of category 1, 11;
    /// of master 4, 10; with 11 used, 10; of category 2, none.
    #[test]
    fn the_closest_unit_matches_its_slots_category_and_type() {
        let t = slot_world(
            &[(10, 4, 5.0, 0.0), (11, 5, 2.0, 0.0), (12, 6, 1.0, 0.0)],
            &[],
        );
        let g = t.groups.get(1).unwrap();
        let o = [0.0; 3];
        assert_eq!(t.closest_unit(g, o, -1, -1, &[]).unwrap(), 12);
        assert_eq!(t.closest_unit(g, o, 1, -1, &[]).unwrap(), 11);
        assert_eq!(t.closest_unit(g, o, 1, 4, &[]).unwrap(), 10);
        assert_eq!(t.closest_unit(g, o, 1, -1, &[11]).unwrap(), 10);
        assert_eq!(t.closest_unit(g, o, 2, -1, &[]).unwrap(), -1);
    }

    /// `FUN_141009760`: slots at 0 and 3 on x, units at 1 and -5. In order the first takes the
    /// unit at 1 and the second the one at -5 (1 + 8); with the first unit barred from the first
    /// slot, 5 + 2 is shorter, so that order wins. Units at 1 and 5 keep the plain order
    /// (1 + 2 against 5 + 2).
    #[test]
    fn the_slots_take_their_units_in_the_shorter_of_two_orders() {
        let run = |second: f32| {
            let t = slot_world(&[(10, 4, 1.0, 0.0), (11, 4, second, 0.0)], &[]);
            let g = t.groups.get(1).unwrap().clone();
            let mut f = Formation::new();
            f.slots = vec![slot_at([0.0; 3], 1), slot_at([3.0, 0.0, 0.0], 1)];
            t.populate_slots(&mut f, &g).unwrap();
            f.slots
                .iter()
                .map(|s| (s.unit_id, s.position_filled))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(-5.0), [(11, 1), (10, 1)]);
        assert_eq!(run(5.0), [(10, 1), (11, 1)]);
    }

    /// `FUN_14100a0c0`: slots at (0, 0) and (3, 0), member 10 at the first and 11 at
    /// (1/3, 2.98), 3 from the first slot and 4 from the second. The trials of `FUN_141009760`
    /// give 10 the first slot (4 in all against 6); the feature keeps the start whose squared
    /// distances differ least, 11 in the first (9) and 10 in the second (9). A member the world
    /// does not hold (12) keeps its row of costs (0 as the process starts) and takes a slot.
    #[test]
    fn the_feature_s_assignment_keeps_the_start_of_least_variance() {
        let y = (80.0f32 / 9.0).sqrt();
        let run = |feature: bool, gone: bool| {
            let mut t = slot_world(&[(10, 4, 0.0, 0.0), (11, 4, 1.0 / 3.0, y)], &[]);
            if gone {
                t.groups.groups[0].members.push(crate::group::Member {
                    id: 12,
                    ..crate::group::Member::default()
                });
            }
            let g = t.groups.get(1).unwrap().clone();
            let mut f = Formation::new();
            f.slots = vec![slot_at([0.0; 3], 1), slot_at([3.0, 0.0, 0.0], 1)];
            if feature {
                t.assign_slots_by_variance(&mut f, &g, &mut Vec::new())
                    .unwrap();
            } else {
                t.populate_slots(&mut f, &g).unwrap();
            }
            f.slots
                .iter()
                .map(|s| (s.unit_id, s.position_filled))
                .collect::<Vec<_>>()
        };
        assert_eq!(run(false, false), [(10, 1), (11, 1)]);
        assert_eq!(run(true, false), [(11, 1), (10, 1)]);
        assert_eq!(run(true, true), [(11, 1), (12, 1)]);
    }

    /// The feature's costs persist between calls (`DAT_143f161e0`): a member the world no
    /// longer holds is placed by the row its last call wrote, and a cost row runs into the next
    /// member's with more slots than members.
    #[test]
    fn the_feature_s_costs_of_a_gone_member_are_its_last() {
        let mut t = slot_world(&[(10, 4, 0.0, 0.0), (11, 4, 3.0, 0.0)], &[]);
        let g = t.groups.get(1).unwrap().clone();
        let mut costs = Vec::new();
        let two = || vec![slot_at([0.0; 3], 1), slot_at([3.0, 0.0, 0.0], 1)];
        let mut f = Formation::new();
        f.slots = two();
        t.assign_slots_by_variance(&mut f, &g, &mut costs).unwrap();
        assert_eq!(costs[..4], [0.0, 9.0, 9.0, 0.0]);
        // 11 leaves the world: its row (9, 0) stays and still gives it the second slot
        let _ = t.players[1].objects.remove(1);
        let mut f = Formation::new();
        f.slots = two();
        t.assign_slots_by_variance(&mut f, &g, &mut costs).unwrap();
        assert_eq!(f.slots[1].unit_id, 11);
        // three slots for two members: row 0 runs into row 1's first place, which row 1 writes
        let t = slot_world(&[(10, 4, 0.0, 0.0), (11, 4, 3.0, 0.0)], &[]);
        let mut f = Formation::new();
        f.slots = vec![
            slot_at([0.0; 3], 1),
            slot_at([3.0, 0.0, 0.0], 1),
            slot_at([1.0, 0.0, 0.0], 1),
        ];
        let mut costs = Vec::new();
        t.assign_slots_by_variance(&mut f, &g, &mut costs).unwrap();
        assert_eq!(costs[..5], [0.0, 9.0, 9.0, 0.0, 4.0]);
    }

    /// `FUN_141009760` measures from the slot's `+0x1c`, which the load fills from the save's
    /// `mTransformPosition` (0x141018637), not from `+0x10` (`mDesiredTransformPosition`).
    #[test]
    fn a_slot_takes_the_unit_nearest_its_transform_position() {
        let t = slot_world(&[(10, 4, 1.0, 0.0), (11, 4, 9.0, 0.0)], &[]);
        let g = t.groups.get(1).unwrap().clone();
        let mut f = Formation::new();
        let mut s = slot_at([10.0, 0.0, 0.0], 1);
        s.desired_transform_position = [0.0; 3];
        f.slots = vec![s];
        t.populate_slots(&mut f, &g).unwrap();
        assert_eq!(f.slots[0].unit_id, 11);
    }

    /// `FUN_141009060` after a column: a line of 3 takes its members (in order) into its places
    /// from the middle out (1, 0, 2), one of 4 into (2, 0, 1, 3); with a line part-transformed,
    /// or no column before, the slots take the closest units or none.
    #[test]
    fn a_line_after_a_column_fills_from_its_middle_out() {
        let run = |width: usize, transformed: i32, previous: i32| {
            let units: Vec<(i32, i16, f32, f32)> = (0..width)
                .map(|i| (10 + i as i32, 4, i as f32, 0.0))
                .collect();
            let mut t = slot_world(&units, &[]);
            let mut column = Formation::new();
            column.kind = previous;
            t.groups.groups[0].previous_formation = Some(column);
            let g = t.groups.get(1).unwrap().clone();
            let mut f = Formation::new();
            f.kind = LINE;
            f.maximum_width = width as i32;
            f.maximum_depth = 1;
            f.number_transformed_slots = transformed;
            f.slots = (0..width)
                .map(|i| slot_at([i as f32, 0.0, 0.0], -1))
                .collect();
            t.from_previous_column(&mut f, &g).unwrap();
            f.slots.iter().map(|s| s.unit_id).collect::<Vec<_>>()
        };
        assert_eq!(run(3, 3, COLUMN), [11, 10, 12]);
        assert_eq!(run(4, 4, COLUMN), [11, 12, 10, 13]);
        // part-transformed: each slot its closest
        assert_eq!(run(3, 2, COLUMN), [10, 11, 12]);
        assert_eq!(run(3, 3, LINE), [-1, -1, -1]);
    }

    /// `FUN_14103a1f0`'s group (1), target (2) and horde (3) transforms of every slot of the
    /// array: the group's place plus the offset turned to the group's forward (for 1), as it is
    /// (2 and 3),
    /// the target's original place for 2; a turn of more than a right angle takes the units anew.
    #[test]
    fn the_group_target_and_horde_transforms_place_every_slot() {
        let mut t = slot_world(&[(10, 4, 3.0, 3.0)], &[]);
        t.groups.groups[0].motion.position[0] = [5.0, 0.0, 5.0];
        t.groups.groups[0].motion.position[1] = [0.0, 0.0, 1.0];
        t.groups.groups[0].current_target.original_position = [9.0, 0.0, 9.0];
        let g = t.groups.get(1).unwrap().clone();
        let run = |kind: i32| {
            let mut f = Formation::new();
            f.transform_type = kind;
            f.transform_orientation = [0.0, 0.0, 1.0];
            f.slots = vec![Slot {
                offset: [1.0, 0.0, 0.0],
                ..slot_at([0.0; 3], 1)
            }];
            f.spare_slots = vec![Slot::default()];
            let took = t.formation_transform(&mut f, &g, false).unwrap();
            (
                took,
                f.slots[0].transform_position,
                f.number_transformed_slots,
            )
        };
        let (_, turned, n) = run(1);
        assert_eq!(n, 2);
        // the offset ahead (x) turned to the group's forward (z)
        assert!(
            (turned[0] - 5.0).abs() < 1e-6 && (turned[2] - 6.0).abs() < 1e-6,
            "{turned:?}"
        );
        assert_eq!(run(3), (false, [6.0, 0.0, 5.0], 2));
        assert_eq!(run(2), (false, [10.0, 0.0, 9.0], 2));
        // facing back: the slot takes its (only) unit anew
        let mut f = Formation::new();
        f.transform_type = 3;
        f.transform_orientation = [0.0, 0.0, -1.0];
        f.slots = vec![slot_at([0.0; 3], 1)];
        assert!(t.formation_transform(&mut f, &g, false).unwrap());
        assert_eq!((f.slots[0].unit_id, f.slots[0].position_filled), (10, 1));
    }

    /// `FUN_14103a1f0`'s passability: slot 1 lies on a blocked tile (8, 5), behind which the
    /// line from the group's place meets the tile's temporary record; its unit at (7.5, 5.5)
    /// searches from its cell (x+1 blocked, then x-1) and finds slot 0's cell, so the slot takes
    /// that cell's middle; below a spacing of 1 nudged by the cell's count (1: x + 0.25).
    #[test]
    fn an_unreachable_slot_moves_to_the_reachable_cell_its_unit_finds() {
        let run = |spacing: f32| {
            let mut t = slot_world(&[(10, 4, 5.5, 5.5), (11, 4, 7.5, 5.5)], &[(8, 5)]);
            t.groups.groups[0].motion.position[0] = [5.5, 0.0, 5.5];
            t.groups.groups[0].motion.lead = 10;
            t.groups.groups[0].desired_spacing = spacing;
            let g = t.groups.get(1).unwrap().clone();
            let mut f = Formation::new();
            f.slots = vec![
                Slot {
                    unit_id: 10,
                    ..slot_at([5.5, 0.0, 5.5], 1)
                },
                Slot {
                    unit_id: 11,
                    ..slot_at([8.5, 0.0, 5.5], 1)
                },
            ];
            t.passable_slots(&mut f, &g).unwrap();
            f.slots
                .iter()
                .map(|s| s.transform_position)
                .collect::<Vec<_>>()
        };
        assert_eq!(run(2.0), [[5.5, 0.0, 5.5], [5.5, 0.0, 5.5]]);
        assert_eq!(run(0.5), [[5.5, 0.0, 5.5], [5.75, 0.0, 5.5]]);
        // COMISS/JC at 0x14103cde3: a NaN spacing nudges as one below 1
        assert_eq!(run(f32::NAN), [[5.5, 0.0, 5.5], [5.75, 0.0, 5.5]]);
        // every slot standable: nothing moves
        let mut t = slot_world(&[(10, 4, 5.5, 5.5)], &[]);
        t.groups.groups[0].motion.lead = 10;
        let g = t.groups.get(1).unwrap().clone();
        let mut f = Formation::new();
        f.slots = vec![slot_at([12.5, 0.0, 5.5], 1)];
        t.passable_slots(&mut f, &g).unwrap();
        assert_eq!(f.slots[0].transform_position, [12.5, 0.0, 5.5]);
    }

    /// A ghost of id `id` at (`x`, `y`) in its player's doppelganger list.
    pub(crate) fn ghost_at(id: i32, x: f32, y: f32) -> crate::ghost::Ghost {
        crate::ghost::Ghost {
            id,
            original: None,
            original_id: -1,
            original_master: None,
            original_owner: 1,
            position: [x, y, 0.0],
            tile: [x as i32, y as i32],
            footprint: 0,
            bits: 0,
            cant_see: 0,
            seen: false,
            state: 2,
        }
    }

    /// `FUN_1410465d0` for the constructor's lead -1 finds the world's object -1: a ghost there
    /// (master 243, its restriction 1 standing on class 2 only) tests the slots, stands on none
    /// of them nor on the group's place, so nothing moves; without it the first active slot's
    /// unit tests them, and slot 1 moves as with that unit as the lead.
    #[test]
    fn the_passability_tests_with_the_world_s_object_of_the_lead_id() {
        use crate::terrain::{Terrain, TerrainKind};
        let run = |ghost: bool| {
            let mut t = slot_world(&[(10, 4, 5.5, 5.5), (11, 4, 7.5, 5.5)], &[(8, 5)]);
            let kind = |flags: u8| TerrainKind {
                shown: true,
                flags,
                forest: false,
            };
            t.spatial.terrain = Some(Terrain::new(
                20,
                20,
                vec![(0, 255, -1); 400],
                vec![kind(1), kind(8)],
                vec![vec![1.0, 0.0], vec![0.0, 1.0]],
            ));
            for masters in &mut t.masters {
                let ghost = crate::masters::Master {
                    id: crate::ghost::GHOST_MASTER,
                    kind: crate::ghost::GHOST,
                    terrain_restriction: 1,
                    ..crate::masters::Master::default()
                };
                *masters = crate::masters::Masters::new(masters.iter().cloned().chain([ghost]));
            }
            if ghost {
                t.players[1].ghosts = vec![ghost_at(-1, 1.0, 1.0)];
            }
            t.groups.groups[0].motion.position[0] = [5.5, 0.0, 5.5];
            assert_eq!(t.groups.groups[0].motion.lead, -1);
            let g = t.groups.get(1).unwrap().clone();
            let mut f = Formation::new();
            f.slots = vec![
                Slot {
                    unit_id: 10,
                    ..slot_at([5.5, 0.0, 5.5], 1)
                },
                Slot {
                    unit_id: 11,
                    ..slot_at([8.5, 0.0, 5.5], 1)
                },
            ];
            t.passable_slots(&mut f, &g).unwrap();
            f.slots
                .iter()
                .map(|s| s.transform_position)
                .collect::<Vec<_>>()
        };
        assert_eq!(run(false), [[5.5, 0.0, 5.5], [5.5, 0.0, 5.5]]);
        assert_eq!(run(true), [[5.5, 0.0, 5.5], [8.5, 0.0, 5.5]]);
    }

    /// The passability's extra step: slot 1's unit at (7.5, 7.5) finds slot 2's cell (6, 7)
    /// first, which points toward the nearest slot (0, at (5.5, 5.5): -y), and the line to the
    /// next cell's middle (6.5, 6.5) is clear, so the slot takes that cell.
    #[test]
    fn a_found_cell_steps_toward_the_nearest_slot_when_the_way_is_clear() {
        let mut t = slot_world(&[(10, 4, 5.5, 5.5), (11, 4, 7.5, 7.5)], &[(8, 5)]);
        t.groups.groups[0].motion.position[0] = [5.5, 0.0, 5.5];
        t.groups.groups[0].motion.lead = 10;
        let g = t.groups.get(1).unwrap().clone();
        let mut f = Formation::new();
        f.slots = vec![
            Slot {
                unit_id: 10,
                ..slot_at([5.5, 0.0, 5.5], 1)
            },
            Slot {
                unit_id: 11,
                ..slot_at([8.5, 0.0, 5.5], 1)
            },
            slot_at([6.5, 0.0, 7.5], 1),
        ];
        t.passable_slots(&mut f, &g).unwrap();
        assert_eq!(f.slots[1].transform_position, [6.5, 0.0, 6.5]);
        assert_eq!(f.slots[2].transform_position, [6.5, 0.0, 7.5]);
    }

    /// The passability's box and nearest slot as the assembly takes them: a NaN place keeps the
    /// box's old bounds (`MINSS`/`MAXSS` of the new value first) and the earlier nearest slot
    /// (`COMISS`/`CMOVBE`); an inactive slot counts for neither.
    #[test]
    fn the_passability_box_keeps_its_bounds_and_nearest_for_a_nan_place() {
        let nan = [f32::NAN, 0.0, f32::NAN];
        let mut idle = slot_at([50.0, 0.0, 50.0], 1);
        idle.active = 0;
        let slots = [
            slot_at([4.0, 0.0, 6.0], 1),
            slot_at(nan, 1),
            idle,
            slot_at([3.0, 0.0, 2.0], 1),
        ];
        let (bounds, nearest) = passability_box(&slots, [5.0, 0.0, 5.0], [3.0, 0.0, 3.0]);
        assert_eq!(bounds, [3.0, 2.0, 5.0, 6.0]);
        assert_eq!(nearest, Some(3));
        let (_, nearest) = passability_box(&slots[..2], [5.0, 0.0, 5.0], [3.0, 0.0, 3.0]);
        assert_eq!(nearest, Some(0));
        // a NaN tester place keeps the starting bounds for its part too
        let (bounds, _) = passability_box(&slots[..1], nan, [3.0, 0.0, 3.0]);
        assert_eq!(bounds, [4.0, 6.0, 4.0, 6.0]);
        assert_eq!(passability_box(&slots[1..2], nan, [0.0; 3]).1, None);
    }

    /// A reachable slot's cell mark: along y for `|dx| <= |dz|` and for a NaN (bit 7 for a `dz`
    /// below 0 or NaN), else along x (bits 6 and 7 for a `dx` below 0 or NaN, bit 6 else); the
    /// count goes up and bit 0 is set.
    #[test]
    fn a_cell_mark_points_toward_the_nearest_slot_and_a_nan_along_y() {
        assert_eq!(cell_mark(0, 0.0, 2.0), 3);
        assert_eq!(cell_mark(0, 0.0, -2.0), 0x83);
        assert_eq!(cell_mark(0, 3.0, 1.0), 0x43);
        assert_eq!(cell_mark(0, -3.0, 1.0), 0xc3);
        assert_eq!(cell_mark(0xc3, 0.0, 2.0), 5);
        assert_eq!(cell_mark(0, f32::NAN, 2.0), 3);
        assert_eq!(cell_mark(0, 3.0, f32::NAN), 0x83);
        assert_eq!(cell_mark(0, f32::NAN, f32::NAN), 0x83);
        // a slot's tile by CVTTSS2SI: a NaN place is off any map
        assert_eq!(slot_tile([f32::NAN, 0.0, 2.5]), (i32::MIN, 2));
    }

    /// The line transform's distance back (0x14103a850, `COMISS`/`JA`): a line's own when it is
    /// above 0, else, a NaN too, the line's index times the spacing.
    #[test]
    fn a_nan_distance_back_takes_the_index_times_the_spacing() {
        let t = slot_world(&[(10, 4, 5.0, 5.0)], &[]);
        let mut g = t.groups.get(1).unwrap().clone();
        g.motion.position[0] = [9.0, 0.0, 5.0];
        g.desired_spacing = 2.0;
        let mut f = Formation::new();
        f.transform_type = 0;
        f.number_lines = 2;
        f.slots = vec![slot_at([0.0; 3], 1), slot_at([0.0; 3], 1)];
        f.lines = vec![
            FormationLine {
                indices: vec![0],
                ..FormationLine::default()
            },
            FormationLine {
                indices: vec![1],
                distance_back: f32::NAN,
                ..FormationLine::default()
            },
        ];
        let _ = t.formation_transform(&mut f, &g, false).unwrap();
        assert_eq!(f.slots[1].transform_position, [7.0, 0.0, 5.0]);
    }

    /// `FUN_141041810`: every slot of the array, spare ones too, loses its unit and filled mark.
    #[test]
    fn clearing_the_units_empties_every_slot_of_the_array() {
        let mut f = Formation::new();
        let taken = Slot {
            unit_id: 7,
            position_filled: 1,
            ..Slot::default()
        };
        f.slots = vec![taken];
        f.spare_slots = vec![taken];
        f.clear_units();
        assert!(
            f.slots
                .iter()
                .chain(&f.spare_slots)
                .all(|s| (s.unit_id, s.position_filled) == (-1, 0))
        );
    }

    /// The grid's count (bits 1 to 5) counts on and wraps, the other bits stay.
    #[test]
    fn a_grid_cell_counts_its_slots_in_bits_1_to_5() {
        assert_eq!(count_up(0), 2);
        assert_eq!(count_up(3), 5);
        assert_eq!(count_up(0xc1 | 0x3e), 0xc1);
    }

    /// One statistic of category 1 holding `units`, all of master 4, spacing 1.
    fn one_statistic(t: &mut Turns, units: &[i32]) {
        let mut unit_types = vec![0u32; 50];
        unit_types[0] = 4;
        let mut counts = vec![0u32; 50];
        counts[0] = units.len() as u32;
        let g = &mut t.groups.groups[0];
        g.desired_spacing = 1.0;
        g.unit_types = vec![crate::group::UnitTypeStatistic {
            formation_category: 1,
            units: units.to_vec(),
            desired_spacing: 1.0,
            number_unit_types: 1,
            unit_types,
            unit_type_counts: counts,
            ..crate::group::UnitTypeStatistic::default()
        }];
    }

    /// `FUN_141012280` lays its slots, transforms them and takes the units, the column-to-line
    /// flag set for a line after a column (0x141013d20..0x141013d3a): a line of three after a
    /// column takes its members from the middle out (11, 10, 12), after a line the closest
    /// (10, 11, 12); each as its slots and `lay_units` with that flag.
    #[test]
    fn a_line_after_a_column_takes_its_units_in_the_column_s_order() {
        let run = |previous: i32| {
            let mut t = slot_world(
                &[(10, 4, 5.5, 5.5), (11, 4, 5.5, 6.5), (12, 4, 5.5, 7.5)],
                &[],
            );
            one_statistic(&mut t, &[10, 11, 12]);
            t.groups.groups[0].motion.position[0] = [6.5, 0.0, 6.5];
            let mut before = Formation::new();
            before.kind = previous;
            t.groups.groups[0].previous_formation = Some(before);
            let g = t.groups.get(1).unwrap().clone();
            let mut whole = Formation::new();
            assert!(
                t.create_column_line(&mut whole, &g, false, (false, false))
                    .unwrap()
            );
            let mut parts = Formation::new();
            assert!(column_line_slots(
                &mut parts,
                &g,
                false,
                (false, false),
                t.world_time
            ));
            t.lay_units(&mut parts, &g, previous == COLUMN).unwrap();
            assert_eq!(whole, parts);
            whole.slots.iter().map(|s| s.unit_id).collect::<Vec<_>>()
        };
        assert_eq!(run(COLUMN), [11, 10, 12]);
        assert_eq!(run(LINE), [10, 11, 12]);
    }

    /// A path level of `points` points at waypoint `waypoint` (none without points).
    fn level(points: &[Vec3], waypoint: i32, flags: u32) -> PathData {
        PathData {
            level: 0,
            linked_type: 0,
            points: points.to_vec(),
            has_path: !points.is_empty(),
            path_flags: flags,
            waypoint,
            state: 0,
            range: 0.0,
            target: -1,
            pause_time: 0.0,
            flags: 0,
            length: 0.0,
            created: 0,
            continue_counter: 10,
            repath_time: 0.0,
            ignore: Vec::new(),
            ignored: Vec::new(),
        }
    }

    /// Player 1 (minimum column distance 10, column-to-line 5) with unit 7 of a soldier master
    /// (type 70, category 1) in group 1 at the origin along `paths` (walked first).
    fn type_world(paths: Vec<PathData>) -> Turns {
        let mut u = crate::turn::Unit {
            id: 7,
            ..crate::turn::Unit::default()
        };
        u.life.master_id = 4;
        let player = crate::turn::Player {
            objects: vec![u],
            formations: crate::turn::Formations {
                min_column_distance: 10,
                column_to_line_distance: 5,
                ..crate::turn::Formations::default()
            },
            ..crate::turn::Player::default()
        };
        let mut t = Turns::new(
            0,
            0,
            0,
            vec![0, 0],
            vec![crate::turn::Player::default(), player],
        );
        let masters = crate::masters::Masters::new([crate::masters::Master {
            id: 4,
            kind: 70,
            formation_category: 1,
            ..crate::masters::Master::default()
        }]);
        t.masters = vec![masters.clone(), masters];
        let mut g = UnitGroup::new(1);
        g.owner = 1;
        g.members = vec![crate::group::Member {
            id: 7,
            ..crate::group::Member::default()
        }];
        g.motion.paths = paths;
        t.groups = crate::group::UnitGroups {
            groups: vec![g],
            ..crate::group::UnitGroups::default()
        };
        t
    }

    fn own(t: &Turns) -> i32 {
        t.own_formation_type(t.groups.get(1).unwrap()).unwrap()
    }

    /// `FUN_141048180`: the top ancestor (the last level) on its last waypoint; a lone level has
    /// no ancestor, a top without a path is not on it.
    #[test]
    fn the_last_high_level_waypoint_is_the_top_levels_last_point() {
        let top = level(&[[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]], 2, 0);
        let walked = level(&[[0.0; 3], [1.0, 0.0, 0.0]], 0, 0);
        assert!(on_last_high_level_waypoint(&[walked.clone(), top.clone()]));
        assert!(!on_last_high_level_waypoint(std::slice::from_ref(&top)));
        let mut early = top;
        early.waypoint = 1;
        assert!(!on_last_high_level_waypoint(&[walked.clone(), early]));
        assert!(!on_last_high_level_waypoint(&[walked, level(&[], 5, 0)]));
    }

    /// `UpdateDesiredPositions`'s own choice for a soldier: a line without a path, a column off
    /// the last high waypoint or on a patrol, and on it a column only beyond both distances
    /// (measured on the ground to the path's last point): 30 is beyond 10 and 5, 8 within the
    /// minimum column distance, whatever the current type; a target's radius and range lengthen
    /// the column-to-line distance.
    #[test]
    fn a_group_chooses_a_column_far_along_its_path_and_a_line_near_its_end() {
        assert_eq!(own(&type_world(Vec::new())), LINE);
        let walked = level(&[[0.0; 3], [1.0, 0.0, 0.0]], 0, 0);
        let path_to = |x: f32, waypoint: i32, flags: u32| {
            vec![
                walked.clone(),
                level(&[[0.0; 3], [x, 9.0, 0.0]], waypoint, flags),
            ]
        };
        assert_eq!(own(&type_world(path_to(30.0, 0, 0))), COLUMN);
        assert_eq!(own(&type_world(path_to(30.0, 1, 1))), COLUMN);
        assert_eq!(own(&type_world(path_to(30.0, 1, 0))), COLUMN);
        assert_eq!(own(&type_world(path_to(8.0, 1, 0))), LINE);
        // a column (whose type the game tests first) turns to a line within 10 too; at 12 it
        // stays one until a target's radius and range lengthen 5 past 12
        let mut t = type_world(path_to(8.0, 1, 0));
        let mut column = Formation::new();
        column.kind = COLUMN;
        t.groups.groups[0].current_formation = Some(column);
        assert_eq!(own(&t), LINE);
        t.groups.groups[0].motion.paths = path_to(12.0, 1, 0);
        assert_eq!(own(&t), COLUMN);
        let g = &mut t.groups.groups[0];
        g.current_target.target = 3;
        g.current_target.target_radius = 5.0;
        g.current_target.range = 2.5;
        assert_eq!(own(&t), LINE);
        // a lone level measures its own path; one point measures to the origin
        let mut t = type_world(vec![level(&[[30.0, 0.0, 0.0]], 0, 0)]);
        t.groups.groups[0].motion.position[0] = [40.0, 0.0, 0.0];
        assert_eq!(own(&t), COLUMN);
    }

    /// Flags bit 2 (`MinAutoDistanceLock`) choose a line, horde units only a horde; given a
    /// type, or locked to one (not -1 or 0), the group takes it.
    #[test]
    fn a_lock_or_a_given_type_overrides_the_groups_own_choice() {
        let walked = level(&[[0.0; 3], [1.0, 0.0, 0.0]], 0, 0);
        let far = vec![walked, level(&[[0.0; 3], [30.0, 0.0, 0.0]], 1, 0)];
        let mut t = type_world(far);
        t.groups.groups[0].flags |= 4;
        assert_eq!(own(&t), LINE);
        t.masters[1] = crate::masters::Masters::new([crate::masters::Master {
            id: 4,
            kind: 70,
            ..crate::masters::Master::default()
        }]);
        assert_eq!(own(&t), HORDE);
        let g = t.groups.get(1).unwrap().clone();
        assert_eq!(t.formation_type(&g, BOX).unwrap(), BOX);
        let mut locked = g;
        locked.formation_locked_type = STAGGERED;
        assert_eq!(t.formation_type(&locked, -1).unwrap(), STAGGERED);
        locked.formation_locked_type = 0;
        assert_eq!(t.formation_type(&locked, -1).unwrap(), HORDE);
    }

    /// createColumnLine's sizes: a line at least twice as wide as it is deep, a column the other
    /// way round.
    #[test]
    fn a_line_is_twice_as_wide_as_deep_and_a_column_the_reverse() {
        assert_eq!(sizes(1, true), (1, 1));
        // root 4: 10 / 4 = 2 lines of 5
        assert_eq!(sizes(10, true), (2, 5));
        // root 3: 9 / 3 = 3 lines of 3, too deep: 2 lines of 5
        assert_eq!(sizes(9, true), (2, 5));
        assert_eq!(sizes(16, true), (3, 6));
        assert_eq!(sizes(10, false), (5, 2));
    }

    fn group_of(category: i32, count: usize, spacing: f32, types: &[(u32, u32)]) -> UnitGroup {
        let mut unit_types = vec![0u32; 50];
        let mut counts = vec![0u32; 50];
        for (i, &(t, n)) in types.iter().enumerate() {
            unit_types[i] = t;
            counts[i] = n;
        }
        UnitGroup {
            members: (0..count)
                .map(|i| crate::group::Member {
                    id: i as i32,
                    ..crate::group::Member::default()
                })
                .collect(),
            unit_types: vec![crate::group::UnitTypeStatistic {
                formation_category: category,
                units: (0..count as i32).collect(),
                desired_spacing: spacing,
                number_unit_types: types.len() as i32,
                unit_types,
                unit_type_counts: counts,
                ..crate::group::UnitTypeStatistic::default()
            }],
            ..UnitGroup::default()
        }
    }

    /// Ten units of two types in a line: two lines of five, centred (an odd line starts two
    /// offsets forward), the types alternating with the second line starting on the type the
    /// first ended with.
    #[test]
    fn a_line_of_two_unit_types_alternates_its_slots() {
        let g = group_of(3, 10, 1.0, &[(4, 5), (24, 5)]);
        let mut f = Formation::new();
        assert!(column_line_slots(&mut f, &g, false, (false, false), 77));
        assert_eq!((f.kind, f.maximum_width, f.maximum_depth), (LINE, 5, 2));
        assert_eq!(f.slots.len(), 10);
        assert_eq!(f.slots[0].offset, [0.0, 0.0, -2.0]);
        assert_eq!(f.slots[4].offset, [0.0, 0.0, 2.0]);
        let subs: Vec<i32> = f.slots.iter().map(|s| s.desired_sub_category).collect();
        assert_eq!(subs, [4, 24, 4, 24, 4, 4, 24, 4, 24, 24]);
        assert_eq!(f.lines[1].distance_back, 1.0);
        assert_eq!(f.lines[1].indices, [5, 6, 7, 8, 9]);
    }
}
