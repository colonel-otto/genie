//! A group's formation (FD-036): `BFormation`, its slots and lines, and the layouts that fill
//! them (createColumnLine `FUN_141012280`, ...).

use super::{Formation, FormationLine, Slot, UnitGroup};
use crate::fmath::{Vec3, add, scale, sub};
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
    fn slot_mut(&mut self, i: usize) -> &mut Slot {
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
    fn use_slots(&mut self, n: usize) {
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

impl crate::turn::Turns {
    /// `UpdateDesiredPositions` (`FUN_14103fd00`) of group `group` with the type `given` (-1 for
    /// the group's own choice): nothing without members. A group without a formation gets an
    /// empty one ([`Formation::new`]). Its own choice, while it is unlocked (`mFormationLocked
    /// Type` -1 or 0), is a line (2), a column (1) from the distances to its path's last high
    /// waypoint (not modelled: a group whose motion has a path), a line again with flags bit 2
    /// (`MinAutoDistanceLock`), and a horde (0) when no member is a soldier of a formation
    /// category (`FUN_141008e50`). A type other than the formation's (or a given one) makes the
    /// formation the previous one and a new one the current; then the type's layout: a line,
    /// staggered or flanked line or a column by createColumnLine ([`create_column_line`], its
    /// transform after it); a horde, a phalanx, a box and a user defined formation are not
    /// modelled. A formation already of its type with a slot per member, or of a type the switch
    /// does not know, only transforms ([`formation_transform`], `param_4` 0).
    ///
    /// # Panics
    ///
    /// Never: the group is the one found first.
    pub fn update_desired_positions(&mut self, group: i32, given: i32) -> Result<(), Unsupported> {
        let soldiers = {
            let g = self.groups.get(group).ok_or(Unsupported(
                "the formation of a group the world does not hold",
            ))?;
            if g.members.is_empty() {
                return Ok(());
            }
            let mut any = false;
            for m in &g.members {
                if let Some((p, u)) = self.object(m.id) {
                    let master = self.master_of(p, u.life.master_id)?;
                    if 69 < master.kind && master.formation_category != 0 {
                        any = true;
                        break;
                    }
                }
            }
            any
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
        let mut kind = given;
        if given == -1 && matches!(g.formation_locked_type, -1 | 0) {
            if g.motion.paths.first().is_some_and(|p| p.has_path) {
                return Err(Unsupported(
                    "a group's own formation type along its path (FUN_141048180)",
                ));
            }
            // a line, then flags bit 2 (MinAutoDistanceLock) a line too
            kind = if soldiers { LINE } else { HORDE };
        }
        let current_kind = g.current_formation.as_ref().map_or(HORDE, |f| f.kind);
        if current_kind != kind || given != -1 {
            g.previous_formation = g.current_formation.take();
            g.current_formation = Some(Formation::new());
        }
        let same = |k: i32| {
            let f = g.current_formation.as_ref().expect("made above");
            f.kind == k && f.slots.len() == g.members.len()
        };
        let keeps = same(kind);
        match kind {
            LINE | STAGGERED | FLANKED | COLUMN if !keeps => {
                let mut f = g.current_formation.take().expect("made above");
                let laid = create_column_line(
                    &mut f,
                    g,
                    kind == COLUMN,
                    (kind == STAGGERED, kind == FLANKED),
                    world_time,
                );
                let result = match laid {
                    Ok(true) => {
                        // a line after a column keeps the units' slots (`FUN_141009060`)
                        let after_column = kind != COLUMN
                            && g.previous_formation
                                .as_ref()
                                .is_some_and(|p| p.kind == COLUMN);
                        lay_units(&mut f, g, after_column, (world_time, map), &held)
                    }
                    Ok(false) => Ok(()),
                    Err(e) => Err(e),
                };
                g.current_formation = Some(f);
                result
            }
            HORDE | PHALANX | BOX | USER_DEFINED if !keeps => Err(Unsupported(
                "a horde, phalanx, box or user defined formation (FUN_14103fd00)",
            )),
            // already in its type, or a type the switch does not know: the transform alone
            _ => {
                let mut f = g.current_formation.take().expect("made above");
                let result = formation_transform(&mut f, g, false, (world_time, map), &held);
                g.current_formation = Some(f);
                result.map(|_| ())
            }
        }
    }
}

/// A turn of more than this (`DAT_142eb0c64`, π/2) makes a formation of fewer than two
/// sub-formations take its units anew (`FUN_14103a1f0`).
const RIGHT_ANGLE: f32 = crate::image::group::RIGHT_ANGLE;
/// A slot's place kept this far inside the map's far edges (`FUN_14100b760`, `DAT_142eb5690`).
const MAP_EDGE: f32 = crate::image::group::MAP_EDGE;

impl super::Motion {
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
    let yaw = super::Motion::yaw_angle(canon, forward);
    let old = super::Motion::yaw_angle(canon, formation.transform_orientation);
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
                .map(|i| super::super::Member {
                    id: i as i32,
                    ..super::super::Member::default()
                })
                .collect(),
            unit_types: vec![super::super::UnitTypeStatistic {
                formation_category: category,
                units: (0..count as i32).collect(),
                desired_spacing: spacing,
                number_unit_types: types.len() as i32,
                unit_types,
                unit_type_counts: counts,
                ..super::super::UnitTypeStatistic::default()
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
