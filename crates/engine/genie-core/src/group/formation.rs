//! A group's formation (FD-036): `BFormation`, its slots and lines, and the layouts that fill
//! them (createColumnLine `FUN_141012280`, ...).

use crate::fmath::{Vec3, add, scale, sub};
use crate::group::create::FormationUnit;
use crate::group::layout::{MemberPlace, create_box, create_by_place, create_phalanx};
use crate::group::motion::moved_vector;
use crate::group::{Formation, FormationLine, Slot, UnitGroup, sub_formation};
use crate::movement::PathData;
use crate::unit_ai::Unsupported;

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
    /// desired category and sub-category -1, the rest 0; the slots in use are copied, the spare
    /// ones not.
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
    }

    /// `FUN_14102d490`: a new lines' array of `n` (the old lines go), each line empty.
    pub(crate) fn resize_lines(&mut self, n: usize) {
        if n == 0 || n <= self.lines.len() {
            return;
        }
        self.lines = vec![FormationLine::default(); n];
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
    let root = crate::fmath::sqrt(count as f32).ceil() as i32;
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

/// createColumnLine (`FUN_141012280`) up to its transform: the group's sub-formations (its unit
/// types, by category 0 to 5) laid out in lines on `formation`, `world_time` the slots' time.
/// `column` lays a column (the column ratio set, type 1), else a line (type 2, staggered 7,
/// flanked 8). Returns false for a group without members or ratios (nothing laid out).
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
pub(crate) fn create_column_line(
    formation: &mut Formation,
    group: &UnitGroup,
    column: bool,
    (staggered, flanked): (bool, bool),
    world_time: u32,
) -> Result<bool, Unsupported> {
    if group.members.is_empty() {
        return Ok(false);
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
            let mut lines = ((cat.count as f32 * cat.spacing) / widest).ceil() as i32;
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
            let line = formation
                .lines
                .get_mut(line_index as usize)
                .ok_or(Unsupported("a formation line past the lines' array"))?;
            // FUN_141008f50: a line of `actual` places (-1 each) at depth `line_index`; none for
            // no places
            *line = if 0 < actual {
                FormationLine {
                    depth: line_index,
                    indices: vec![u32::MAX; actual as usize],
                    ..FormationLine::default()
                }
            } else {
                FormationLine::default()
            };
            line.distance_back = distance_back;
            for col in 0..actual.max(0) {
                let at = used_slots + col as usize;
                if formation.slot_capacity() <= at {
                    return Err(Unsupported("a formation slot past the slots' array"));
                }
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
                    transform_position: [0.0; 3],
                    time: world_time,
                    state: 0,
                    depth: line_index,
                    desired_transform_position: [0.0; 3],
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
    Ok(true)
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
fn top_path(paths: &[PathData]) -> Option<&PathData> {
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

impl crate::turn::Turns {
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
    pub fn own_formation_type(&self, g: &UnitGroup) -> Result<i32, Unsupported> {
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
    pub fn formation_type(&self, g: &UnitGroup, given: i32) -> Result<i32, Unsupported> {
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
    /// staggered or flanked line or a column by createColumnLine ([`create_column_line`], its
    /// transform and units after it, [`lay_units`]), a phalanx and a box by their own
    /// ([`crate::group::layout::create_phalanx`], [`crate::group::layout::create_box`], then
    /// [`lay_units`]), a horde and a user defined formation from the members' places
    /// ([`crate::group::layout::create_by_place`]) and then transformed
    /// ([`formation_transform`], `param_4` 0); a formation already laid out, or of a type the
    /// switch does not know (6), is only transformed.
    ///
    /// # Errors
    ///
    /// A group the world does not hold, a member of a master the slice does not have, and the
    /// layouts' and the transform's.
    ///
    /// # Panics
    ///
    /// Never: the group is the one found first.
    pub fn update_desired_positions(&mut self, group: i32, given: i32) -> Result<(), Unsupported> {
        let g = self.groups.get(group).ok_or(Unsupported(
            "the formation of a group the world does not hold",
        ))?;
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
        let places = if !laid_out && matches!(kind, HORDE | USER_DEFINED) {
            let mut places = Vec::with_capacity(g.members.len());
            for m in &g.members {
                let place = match self.object(m.id) {
                    Some((p, u)) => {
                        let unit = FormationUnit::of(self.master_of(p, u.life.master_id)?);
                        Some(MemberPlace {
                            id: m.id,
                            at: moved_vector(u),
                            category: sub_formation(
                                unit.kind,
                                unit.hero_flag,
                                unit.formation_category,
                            )
                            .unwrap_or(-1),
                        })
                    }
                    None => None,
                };
                places.push(place);
            }
            places
        } else {
            Vec::new()
        };
        let world_time = self.world_time;
        let map = self.spatial.map;
        let held: Vec<i32> = self
            .players
            .iter()
            .flat_map(|p| p.objects.iter().chain(&p.sleeping))
            .map(|u| u.id)
            .collect();
        let held = move |id: i32| id != -1 && held.contains(&id);
        let g = self.groups.get_mut(group).expect("the group read above");
        if g.current_formation.is_none() {
            g.current_formation = Some(Formation::new());
        }
        if renew {
            g.previous_formation = g.current_formation.take();
            g.current_formation = Some(Formation::new());
        }
        let mut f = g.current_formation.take().expect("made above");
        let world = (world_time, map);
        let result = match kind {
            LINE | STAGGERED | FLANKED | COLUMN if !laid_out => {
                match create_column_line(
                    &mut f,
                    g,
                    kind == COLUMN,
                    (kind == STAGGERED, kind == FLANKED),
                    world_time,
                ) {
                    Ok(true) => {
                        // a line after a column keeps the units' slots (`FUN_141009060`)
                        let after_column = kind != COLUMN
                            && g.previous_formation
                                .as_ref()
                                .is_some_and(|p| p.kind == COLUMN);
                        lay_units(&mut f, g, after_column, world, &held)
                    }
                    Ok(false) => Ok(()),
                    Err(e) => Err(e),
                }
            }
            PHALANX if !laid_out => {
                if create_phalanx(&mut f, g, world_time) {
                    lay_units(&mut f, g, false, world, &held)
                } else {
                    Ok(())
                }
            }
            BOX if !laid_out => match create_box(&mut f, g, world_time) {
                Ok(true) => lay_units(&mut f, g, false, world, &held),
                Ok(false) => Ok(()),
                Err(e) => Err(e),
            },
            HORDE | USER_DEFINED if !laid_out => {
                // true: the group has members
                let _ = create_by_place(&mut f, g, &places, kind == USER_DEFINED, world_time);
                formation_transform(&mut f, g, false, world, &held).map(|_| ())
            }
            // laid out already, or a type the switch does not know: the transform alone
            _ => formation_transform(&mut f, g, false, world, &held).map(|_| ()),
        };
        g.current_formation = Some(f);
        result
    }
}

/// A turn of more than this (`DAT_142eb0c64`, π/2) makes a formation of fewer than two
/// sub-formations take its units anew (`FUN_14103a1f0`).
const RIGHT_ANGLE: f32 = crate::image::group::RIGHT_ANGLE;
/// A slot's place kept this far inside the map's far edges (`FUN_14100b760`, `DAT_142eb5690`).
const MAP_EDGE: f32 = crate::image::group::MAP_EDGE;

impl crate::group::Motion {
    /// `BMotion::calculateYawAngle` (`FUN_141046270`) without its clamp (`param_5` 0): the angle
    /// from `canon` to `dir` (`FUN_14087e650`), negative when their cross product `dir × canon`
    /// points up.
    #[must_use]
    pub fn yaw_angle(canon: Vec3, dir: Vec3) -> f32 {
        let angle = crate::fmath::angle_between(dir, canon);
        if 0.0 < crate::fmath::cross(dir, canon)[1] {
            -angle
        } else {
            angle
        }
    }
}

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
        slot.transform_position = on_map(at, map);
        slot.desired_transform_position = on_map(at, map);
        slot.transformed = 1;
        slot.time = world_time;
    }
    true
}

/// `FormationTransform` (`FUN_14103a1f0`) of `formation` for `group`, `flag` (`param_4`) the
/// column-to-line change: nothing without slots. The group's yaw from the canon forward (x, or
/// -x about faced) and the formation's last one (`calculateYawAngle`) give the rotation about
/// the group's up; the formation takes the group's forward as its orientation, every slot's
/// transform mark clears and none counts as transformed. The line transform (type 0) takes each
/// line's point behind the group (its distance back, or its index times the group's spacing):
/// the stored positions' point there (`FUN_141016990`, none below two points), else the group's
/// place less its forward times the distance; each of the line's slots is placed there
/// ([`transform_slot`]) and the line counts as transformed (below 256 places). A turn of more
/// than [`RIGHT_ANGLE`] with fewer than two sub-formations takes the units anew (not modelled),
/// as are the other transform types, a group whose stored positions hold two points or more,
/// and the slots' passability against the group's lead unit (`+0xc4`, else the first slot's
/// unit). Returns whether the slots took their units anew (`*param_3`).
pub(crate) fn formation_transform(
    formation: &mut Formation,
    group: &UnitGroup,
    _flag: bool,
    (world_time, map): (u32, [i32; 2]),
    unit_held: &dyn Fn(i32) -> bool,
) -> Result<bool, Unsupported> {
    if formation.slots.is_empty() {
        return Ok(false);
    }
    let canon = [
        if formation.about_faced == 0 {
            1.0
        } else {
            -1.0
        },
        0.0,
        0.0,
    ];
    let [position, forward, up, _right] = group.motion.position;
    let yaw = crate::group::Motion::yaw_angle(canon, forward);
    let old = crate::group::Motion::yaw_angle(canon, formation.transform_orientation);
    let flips = RIGHT_ANGLE < (yaw - old).abs() && group.unit_types.len() < 2;
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
                    .ok_or(Unsupported("a formation line past the lines' array"))?;
                let mut back = 0.0;
                if 0 < i {
                    back = line.distance_back;
                    if back <= 0.0 {
                        back = i as f32 * group.desired_spacing;
                    }
                }
                if 2 <= group.stored_positions.points.len() {
                    return Err(Unsupported(
                        "a formation line's point on the stored positions (FUN_141016990)",
                    ));
                }
                let point = sub(position, scale(back, forward));
                let indices = line.indices.clone();
                let width = indices.len() as i32;
                for k in indices {
                    let _ =
                        transform_slot(formation, &rotation, point, k as i32, 0, (world_time, map));
                }
                if width < 0x100 {
                    formation.number_transformed_slots += width;
                }
            }
        }
        _ => {
            return Err(Unsupported(
                "a formation transform of the group, target or horde type (FUN_14103a1f0)",
            ));
        }
    }
    if flips {
        return Err(Unsupported(
            "a formation turned past a right angle that takes its units anew",
        ));
    }
    let lead = group.motion.lead;
    let has_unit = unit_held(lead)
        || formation
            .slots
            .iter()
            .any(|s| s.active != 0 && unit_held(s.unit_id));
    if has_unit {
        return Err(Unsupported(
            "a formation's slots tested against its lead unit (FUN_141185690)",
        ));
    }
    Ok(false)
}

/// createColumnLine's end (`FUN_141012280` after its layout): the transform
/// ([`formation_transform`]); unless it took the units anew, every slot of the array loses its
/// unit and its filled mark, and the units take their slots ([`populate_slots`]; after a column,
/// with slots transformed, from the previous formation's: `FUN_141009060`, not modelled).
fn lay_units(
    formation: &mut Formation,
    group: &UnitGroup,
    after_column: bool,
    world: (u32, [i32; 2]),
    held: &dyn Fn(i32) -> bool,
) -> Result<(), Unsupported> {
    if formation_transform(formation, group, after_column, world, held)? {
        return Ok(());
    }
    for s in formation.every_slot_mut() {
        s.unit_id = -1;
        s.position_filled = 0;
    }
    if formation.number_transformed_slots < 1 || group.previous_formation.is_none() || !after_column
    {
        populate_slots(formation)
    } else {
        Err(Unsupported(
            "a line's slots taken from the previous column's (FUN_141009060)",
        ))
    }
}

/// `FUN_141009760` (the settings' flag `DAT_143f01834` 0, as every load of the recordings sets
/// it): the transformed and active slots take the closest units of their sub-formation and type
/// (`FUN_14100e370`, two orders tried, the shorter sum kept), not modelled; with no such slot
/// nothing changes.
fn populate_slots(formation: &Formation) -> Result<(), Unsupported> {
    if formation
        .slots
        .iter()
        .any(|s| s.transformed != 0 && s.active != 0)
    {
        return Err(Unsupported(
            "the slots' closest units (ClosestUnit, FUN_14100e370)",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn type_world(paths: Vec<PathData>) -> crate::turn::Turns {
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
        let mut t = crate::turn::Turns::new(
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

    fn own(t: &crate::turn::Turns) -> i32 {
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
        assert!(create_column_line(&mut f, &g, false, (false, false), 77).unwrap());
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
