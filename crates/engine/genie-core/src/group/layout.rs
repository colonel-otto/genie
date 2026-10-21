//! The formations' layouts besides createColumnLine (FD-036): a horde (`FUN_141013eb0`), a user
//! defined formation (`FUN_141014f60`), a phalanx (`FUN_1410147b0`) and a box (`FUN_141011a00`
//! with `FUN_14102ab30` and `CreateLines` `FUN_1410143a0`), as `UpdateDesiredPositions`
//! (`FUN_14103fd00`) makes them.

use crate::fmath::{self, Vec3};
use crate::group::formation::{BOX, HORDE, PHALANX, USER_DEFINED};
use crate::group::{Formation, FormationLine, Slot, UnitGroup};
use crate::unit_ai::Unsupported;

/// A horde's and a user defined formation's slots lie at most this far from the group's place
/// (`DAT_14330ebb8`).
const HORDE_REACH: f32 = crate::image::group::HORDE_REACH;
/// A user defined formation's slots move back by the farthest one ahead plus this
/// (`DAT_142e7f384`).
const USER_DEFINED_BACK: f32 = crate::image::group::USER_DEFINED_BACK;
/// Half: a phalanx's even line's step, a box's first side and its middles (`DAT_142e7f384`, and
/// the phalanx's `DAT_142e8c150` as a divisor).
const BOX_HALF: f32 = crate::image::group::BOX_HALF;
/// A phalanx's even line moves back by its step over this (`DAT_142e8c150`).
const TWO: f32 = crate::image::group::SPACING_FACTOR;
/// A box's perimeter is its side times this (`DAT_142eb0f48`).
const BOX_SIDES: f32 = crate::image::group::BOX_SIDES;
/// A box too small for its units grows to this share of their length (`DAT_142eb880c`).
const BOX_SIDE_SHARE: f32 = crate::image::group::BOX_SIDE_SHARE;
/// Two slots whose offsets on x differ by less than this share a box's line (`DAT_142eb0c60`).
const LINE_EPSILON: f32 = crate::image::group::EPSILON;
/// The places of the box's positions buffer (`DAT_143f15f00`, `FUN_14102ab30`'s `0x3c`).
pub const BOX_POSITIONS: usize = 60;

/// What a horde and a user defined formation read of a member the world holds: its id, its place
/// (slot 0x520) and its sub-formation (`FUN_141011300` with 0, -1 for none).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MemberPlace {
    /// The unit.
    pub id: i32,
    /// Slot 0x520.
    pub at: Vec3,
    /// `FUN_141011300(unit, 0)`.
    pub category: i32,
}

/// A slot's 3D length as the layouts take it (`x² + y² + z²` in that order).
fn length(v: Vec3) -> f32 {
    fmath::sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
}

/// `createHorde` (`FUN_141013eb0`), or `createUDF` (`FUN_141014f60`) for `user_defined`, of
/// `formation` for `group`, `members` its members (None for one the world does not hold):
/// false without members. The slots' array grows to the members (`FUN_14102d580`) and every
/// slot turns inactive; the type is a horde (0) or user defined (5), the width and depth -1, no
/// lines, the transform type 3 (a horde) or 1. The slot of each held member's index takes the
/// unit, its offset from the group's place, its place as the transform and desired transform
/// positions (transformed, `+0x44`), the world's time, state and depth 0, filled, active, its
/// sub-formation and no sub-category; the slots in use are the members (an unheld member's slot
/// keeps what it had, inactive). A user defined formation's offsets on x move back by the
/// largest of them (0 at least) plus [`USER_DEFINED_BACK`]; then any offset longer than
/// [`HORDE_REACH`] is scaled down to it.
pub fn create_by_place(
    formation: &mut Formation,
    group: &UnitGroup,
    members: &[Option<MemberPlace>],
    user_defined: bool,
    world_time: u32,
) -> bool {
    let n = group.members.len();
    if n == 0 {
        return false;
    }
    formation.resize_slots(n);
    for s in formation.every_slot_mut() {
        s.active = 0;
    }
    formation.kind = if user_defined { USER_DEFINED } else { HORDE };
    formation.maximum_width = -1;
    formation.maximum_depth = -1;
    formation.number_lines = 0;
    formation.transform_type = if user_defined { 1 } else { 3 };
    let position = group.motion.position[0];
    for (i, m) in members.iter().enumerate().take(n) {
        let Some(m) = m else {
            continue;
        };
        let s = formation.slot_mut(i);
        s.unit_id = m.id;
        s.offset = fmath::sub(m.at, position);
        s.transform_position = m.at;
        s.desired_transform_position = m.at;
        s.transformed = 1;
        s.time = world_time;
        s.state = 0;
        s.depth = 0;
        s.position_filled = 1;
        s.active = 1;
        s.desired_category = m.category;
        s.desired_sub_category = -1;
    }
    formation.use_slots(n);
    if user_defined {
        let mut ahead = 0.0f32;
        for s in &formation.slots {
            let x = s.offset[0];
            ahead = if x <= ahead { ahead } else { x };
        }
        let back = ahead + USER_DEFINED_BACK;
        for s in &mut formation.slots {
            s.offset[0] -= back;
        }
    }
    for s in &mut formation.slots {
        let l = length(s.offset);
        if HORDE_REACH < l {
            let k = HORDE_REACH / l;
            s.offset = [k * s.offset[0], k * s.offset[1], k * s.offset[2]];
        }
    }
    true
}

/// `FUN_141008f50` on a line reset as the phalanx and `CreateLines` reset it (its indices, width,
/// depth and attack range gone, its distance back kept): `width` places of -1 at depth `depth`,
/// none for no places.
fn reset_line(line: &mut FormationLine, width: i32, depth: i32) {
    line.indices = Vec::new();
    line.depth = 0;
    line.attack_range = 0.0;
    if 0 < width && 0 <= depth {
        line.indices = vec![u32::MAX; width as usize];
        line.depth = depth;
    }
}

/// The unit type statistic a phalanx's next slot takes (`FUN_1410147b0`'s search): the current
/// one while it has units left, else the first of the next category with units (from 0 after
/// none, else from the current's category plus 1, up to 5); `placed` counts its slots.
fn next_statistic(group: &UnitGroup, current: Option<usize>, placed: &mut usize) -> Option<usize> {
    if let Some(i) = current
        && *placed < group.unit_types[i].units.len()
    {
        return current;
    }
    *placed = 0;
    let mut c = current.map_or(0, |i| group.unit_types[i].formation_category + 1);
    while c <= 5 {
        if let Some(i) = group
            .unit_types
            .iter()
            .position(|s| s.formation_category == c)
            && !group.unit_types[i].units.is_empty()
        {
            return Some(i);
        }
        c += 1;
    }
    None
}

/// `createPhalanx` (`FUN_1410147b0`) up to its transform: false without members. The lines are
/// the fewest whose triangle (1 + 2 + ...) holds the members; the slots' array grows to their
/// square, the lines' to them, every slot turns inactive; a phalanx (3) of no slots yet,
/// transform type 0, the lines as its width, depth and lines. Line `i` holds `i + 1` slots, the
/// last (of two lines or more) the members left; it starts at half its slots (integer) times the
/// spacing straight back (`(0, 0, -spacing)`), less half a step for an even count, and steps by
/// it. Each slot takes no unit, the offset, the world's time, the line as its depth, active, and
/// the category of the unit type statistic in turn ([`next_statistic`], -1 when none is left);
/// the slots in use are the members.
pub fn create_phalanx(formation: &mut Formation, group: &UnitGroup, world_time: u32) -> bool {
    let n = group.members.len() as i32;
    if n < 1 {
        return false;
    }
    let mut lines = 0;
    let mut left = n;
    let mut step = 1;
    while 0 < left {
        left -= step;
        lines += 1;
        step += 1;
    }
    formation.resize_slots((lines * lines) as usize);
    formation.resize_lines(lines as usize);
    for s in formation.every_slot_mut() {
        s.active = 0;
    }
    formation.kind = PHALANX;
    formation.use_slots(0);
    formation.transform_type = 0;
    formation.maximum_width = lines;
    formation.maximum_depth = lines;
    formation.number_lines = lines;
    let single = fmath::scale(group.desired_spacing, [0.0, 0.0, -1.0]);
    let mut slot = 0usize;
    let mut statistic = None;
    let mut placed = 0usize;
    for line in 0..lines {
        let mut width = line + 1;
        if 1 < lines && line == lines - 1 {
            width = n - slot as i32;
        }
        reset_line(&mut formation.lines[line as usize], width, line);
        let mut at = fmath::add(fmath::scale((width / 2) as f32, single), [0.0; 3]);
        if width & 1 == 0 {
            let half = [single[0] / TWO, single[1] / TWO, single[2] / TWO];
            at = [at[0] - half[0], at[1] - half[1], at[2] - half[2]];
        }
        for k in 0..width.max(0) as usize {
            statistic = next_statistic(group, statistic, &mut placed);
            *formation.slot_mut(slot) = Slot {
                unit_id: -1,
                offset: at,
                transform_position: [0.0; 3],
                desired_transform_position: [0.0; 3],
                time: world_time,
                state: 0,
                depth: line,
                position_filled: 0,
                active: 1,
                desired_category: statistic.map_or(-1, |i| group.unit_types[i].formation_category),
                desired_sub_category: -1,
                transformed: formation.slot_mut(slot).transformed,
            };
            let l = &mut formation.lines[line as usize];
            if k < l.indices.len() {
                l.indices[k] = slot as u32;
            }
            placed += 1;
            at = fmath::sub(at, single);
            slot += 1;
        }
    }
    formation.use_slots(n as usize);
    true
}

/// `placeBoxPerimeter` (`FUN_14102ab30`): `count` places on the box from `min` to `max` (x and z;
/// both grow by half of what a perimeter of `count` units of `spacing` lacks), into `out`'s x
/// and z; false (nothing written) for none or more than [`BOX_POSITIONS`]. Up to 8 units take
/// fixed places on the corners and middles of the sides at x `max` and `min`; more walk the
/// perimeter from (`max.x`, `min.z`) by the perimeter over the count, as the game walks it
/// (its first corner's x is `max.z` less the step's rest, which its third corner reuses).
pub fn place_box_perimeter(
    count: usize,
    out: &mut [Vec3; BOX_POSITIONS],
    min: &mut Vec3,
    max: &mut Vec3,
    spacing: f32,
) -> bool {
    if !(1..=BOX_POSITIONS).contains(&count) {
        return false;
    }
    let side = max[0] - min[0];
    let total = side * BOX_SIDES;
    let n = count as f32;
    let units = n * spacing;
    let mut step = total / n;
    if total < units {
        step = units / n;
        let grow = (units * BOX_SIDE_SHARE - side) * BOX_HALF;
        min[0] -= grow;
        min[2] -= grow;
        max[0] += grow;
        max[2] += grow;
    }
    let mid_z = (min[2] + max[2]) * BOX_HALF;
    let mid_x = (max[0] + min[0]) * BOX_HALF;
    let fixed: &[(f32, f32)] = match count {
        1 => &[(max[0], mid_z)],
        2 => &[(max[0], mid_z), (min[0], mid_z)],
        3 => &[(max[0], min[2]), (max[0], max[2]), (min[0], mid_z)],
        4 => &[
            (max[0], min[2]),
            (max[0], max[2]),
            (min[0], min[2]),
            (min[0], max[2]),
        ],
        5 => &[
            (max[0], min[2]),
            (max[0], mid_z),
            (max[0], max[2]),
            (min[0], min[2]),
            (min[0], max[2]),
        ],
        6 => &[
            (max[0], min[2]),
            (max[0], mid_z),
            (max[0], max[2]),
            (min[0], min[2]),
            (min[0], mid_z),
            (min[0], max[2]),
        ],
        7 => &[
            (max[0], min[2]),
            (max[0], mid_z),
            (max[0], max[2]),
            (mid_x, min[2]),
            (mid_x, max[2]),
            (min[0], min[2]),
            (min[0], max[2]),
        ],
        8 => &[
            (max[0], min[2]),
            (max[0], mid_z),
            (max[0], max[2]),
            (mid_x, min[2]),
            (mid_x, max[2]),
            (min[0], min[2]),
            (min[0], mid_z),
            (min[0], max[2]),
        ],
        _ => &[],
    };
    if !fixed.is_empty() {
        for (p, &(x, z)) in out.iter_mut().zip(fixed) {
            p[0] = x;
            p[2] = z;
        }
        return true;
    }
    out[0][0] = max[0];
    out[0][2] = min[2];
    let mut corner = 0.0f32;
    let mut state = 0;
    for i in 1..count {
        let prev = out[i - 1];
        let (x, z) = match state {
            0 => {
                let z = prev[2] + step;
                if z < max[2] {
                    (max[0], z)
                } else {
                    state = 1;
                    corner = max[2] - (step - (max[2] - prev[2]));
                    (corner, max[2])
                }
            }
            1 => {
                let x = prev[0] - step;
                if min[0] < x {
                    (x, max[2])
                } else {
                    state = 2;
                    (min[0], max[2] - (step - (prev[0] - min[0])))
                }
            }
            2 => {
                let z = prev[2] - step;
                if min[2] < z {
                    (min[0], z)
                } else {
                    state = 3;
                    (corner, min[2])
                }
            }
            3 => {
                let mut x = prev[0] - step;
                if x <= min[0] {
                    x = max[0];
                    state = 4;
                }
                (x, min[2])
            }
            _ => {
                out[i] = out[0];
                continue;
            }
        };
        out[i][0] = x;
        out[i][2] = z;
    }
    true
}

/// `CreateLines` (`FUN_1410143a0`): the slots in use (in order) form lines by their offsets on x,
/// a slot joining the first line whose x is within [`LINE_EPSILON`] of its own, else starting a
/// new one at its x; the lines' array grows to them (`FUN_14102d490`), the lines in use are
/// them, and each line (reset, [`reset_line`]) holds its slots at its index as depth.
pub fn create_lines(formation: &mut Formation) {
    let mut xs: Vec<f32> = Vec::new();
    let mut members: Vec<Vec<u32>> = Vec::new();
    for (i, s) in formation.slots.iter().enumerate() {
        let x = s.offset[0];
        if let Some(j) = xs.iter().position(|&lx| (lx - x).abs() < LINE_EPSILON) {
            members[j].push(i as u32);
        } else {
            xs.push(x);
            members.push(vec![i as u32]);
        }
    }
    let lines = xs.len();
    formation.resize_lines(lines);
    formation.number_lines = lines as i32;
    for (k, slots) in members.iter().enumerate() {
        let line = &mut formation.lines[k];
        reset_line(line, slots.len() as i32, k as i32);
        for (m, &slot) in slots.iter().enumerate() {
            if m < line.indices.len() {
                line.indices[m] = slot;
            }
        }
    }
}

/// `createBox` (`FUN_141011a00`) up to its transform: false without members. Every slot turns
/// inactive and none is in use; then category by category from 5 down to 0, the first unit type
/// statistic of the category with units puts that many slots on a box ([`place_box_perimeter`]
/// into the box's positions buffer, `DAT_143f15f00`, whose heights stay 0): the first such
/// category on the box of half the spacing each way (a lone unit in the middle, the box then a
/// point), each later one on a box a spacing wider each way than the last; the slots' array
/// grows to hold them, each slot takes no unit, its place as offset, the world's time, active
/// and the category. Then no lines, a box (4), transform type 1, the box's width on x as its
/// width and depth (truncated), and the lines of its slots ([`create_lines`]).
///
/// # Errors
///
/// A category of more than [`BOX_POSITIONS`] units: the perimeter refuses it and the game's
/// slots read the buffer's earlier places and the memory past it.
pub fn create_box(
    formation: &mut Formation,
    group: &UnitGroup,
    world_time: u32,
) -> Result<bool, Unsupported> {
    if group.members.is_empty() {
        return Ok(false);
    }
    let s = group.desired_spacing;
    let mut max_x = s * BOX_HALF;
    let mut min_x = (-s) * BOX_HALF;
    let mut min_z = min_x;
    let mut max_z = max_x;
    for slot in formation.every_slot_mut() {
        slot.active = 0;
    }
    formation.use_slots(0);
    let mut positions = [[0.0f32; 3]; BOX_POSITIONS];
    let mut used = 0usize;
    for c in (0..=5).rev() {
        let Some(stat) = group
            .unit_types
            .iter()
            .find(|u| u.formation_category == c)
            .filter(|u| !u.units.is_empty())
        else {
            continue;
        };
        let count = stat.units.len();
        if BOX_POSITIONS < count {
            return Err(Unsupported(
                "a box side of more than 60 units (FUN_14102ab30 refuses it, the slots read past \
                 the buffer DAT_143f15f00)",
            ));
        }
        formation.resize_slots(used + count);
        if used == 0 && count == 1 {
            positions[0] = [0.0; 3];
            (min_x, min_z, max_x, max_z) = (0.0, 0.0, 0.0, 0.0);
        } else {
            let (mut min, mut max) = if used == 0 {
                let low = (-s) * BOX_HALF;
                let high = s * BOX_HALF;
                ([low, 0.0, low], [high, 0.0, high])
            } else {
                ([min_x - s, 0.0, min_z - s], [max_x + s, 0.0, max_z + s])
            };
            // the game ignores its result: false only for a count refused above
            let _ = place_box_perimeter(count, &mut positions, &mut min, &mut max, s);
            (min_x, min_z, max_x, max_z) = (min[0], min[2], max[0], max[2]);
        }
        for (j, &p) in positions.iter().enumerate().take(count) {
            let at = used + j;
            *formation.slot_mut(at) = Slot {
                unit_id: -1,
                offset: p,
                transform_position: [0.0; 3],
                desired_transform_position: [0.0; 3],
                time: world_time,
                state: 0,
                depth: 0,
                position_filled: 0,
                active: 1,
                desired_category: c,
                desired_sub_category: -1,
                transformed: formation.slot_mut(at).transformed,
            };
        }
        used += count;
        formation.use_slots(used);
    }
    formation.number_lines = 0;
    formation.kind = BOX;
    formation.transform_type = 1;
    formation.maximum_width = (max_x - min_x) as i32;
    formation.maximum_depth = (max_x - min_x) as i32;
    create_lines(formation);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::group::{Member, UnitTypeStatistic};

    fn group(members: usize, spacing: f32, stats: &[(i32, usize)]) -> UnitGroup {
        let mut g = UnitGroup::new(1);
        g.members = (0..members)
            .map(|i| Member {
                id: i as i32,
                ..Member::default()
            })
            .collect();
        g.desired_spacing = spacing;
        g.motion.position[0] = [10.0, 0.0, 10.0];
        g.unit_types = stats
            .iter()
            .map(|&(category, n)| UnitTypeStatistic {
                formation_category: category,
                units: (0..n as i32).collect(),
                ..UnitTypeStatistic::default()
            })
            .collect();
        g
    }

    const fn place(id: i32, x: f32, z: f32, category: i32) -> MemberPlace {
        MemberPlace {
            id,
            at: [x, 0.0, z],
            category,
        }
    }

    /// `FUN_141013eb0`: a held member's slot at its index takes it, its offset from the group's
    /// place (10, 0, 10), longer ones cut to 5; an unheld member's slot stays fresh and inactive.
    #[test]
    fn a_horde_takes_its_members_places_within_reach() {
        let g = group(3, 1.0, &[]);
        let mut f = Formation::new();
        let members = [
            Some(place(7, 12.0, 10.0, 1)),
            None,
            Some(place(9, 20.0, 10.0, 3)),
        ];
        assert!(create_by_place(&mut f, &g, &members, false, 640));
        assert_eq!(
            (
                f.kind,
                f.transform_type,
                f.maximum_width,
                f.maximum_depth,
                f.number_lines
            ),
            (HORDE, 3, -1, -1, 0)
        );
        assert_eq!(f.slots.len(), 3);
        let s = &f.slots[0];
        assert_eq!(
            (s.unit_id, s.offset, s.transform_position),
            (7, [2.0, 0.0, 0.0], [12.0, 0.0, 10.0])
        );
        assert_eq!(s.desired_transform_position, [12.0, 0.0, 10.0]);
        assert_eq!(
            (
                s.transformed,
                s.time,
                s.position_filled,
                s.active,
                s.desired_category
            ),
            (1, 640, 1, 1, 1)
        );
        assert_eq!(s.desired_sub_category, -1);
        assert_eq!((f.slots[1].unit_id, f.slots[1].active), (-1, 0));
        assert_eq!(f.slots[2].offset, [5.0, 0.0, 0.0]);
        // no members, no layout
        let mut f = Formation::new();
        assert!(!create_by_place(&mut f, &group(0, 1.0, &[]), &[], false, 0));
        assert_eq!(f, Formation::new());
    }

    /// `FUN_141014f60`: the offsets on x move back by the farthest ahead (10) plus 0.5, the
    /// unheld member's slot too, then are cut to 5.
    #[test]
    fn a_user_defined_formation_moves_back_behind_its_farthest_member() {
        let g = group(3, 1.0, &[]);
        let mut f = Formation::new();
        let members = [
            Some(place(7, 12.0, 10.0, 1)),
            None,
            Some(place(9, 20.0, 10.0, 3)),
        ];
        assert!(create_by_place(&mut f, &g, &members, true, 640));
        assert_eq!((f.kind, f.transform_type), (USER_DEFINED, 1));
        let offsets: Vec<Vec3> = f.slots.iter().map(|s| s.offset).collect();
        assert_eq!(
            offsets,
            [[-5.0, 0.0, 0.0], [-5.0, 0.0, 0.0], [-0.5, 0.0, 0.0]]
        );
    }

    /// `FUN_1410147b0` for 4 members of spacing 2: lines of 1, 2 and the rest (1); an even line
    /// starts half a step forward; the slots take category 1's three units, then category 3's.
    #[test]
    fn a_phalanx_grows_by_a_slot_a_line() {
        let g = group(4, 2.0, &[(3, 1), (1, 3)]);
        let mut f = Formation::new();
        assert!(create_phalanx(&mut f, &g, 99));
        assert_eq!(
            (
                f.kind,
                f.transform_type,
                f.maximum_width,
                f.maximum_depth,
                f.number_lines
            ),
            (PHALANX, 0, 3, 3, 3)
        );
        let slots: Vec<(Vec3, i32, i32)> = f
            .slots
            .iter()
            .map(|s| (s.offset, s.depth, s.desired_category))
            .collect();
        assert_eq!(
            slots,
            [
                ([0.0, 0.0, 0.0], 0, 1),
                ([0.0, 0.0, -1.0], 1, 1),
                ([0.0, 0.0, 1.0], 1, 1),
                ([0.0, 0.0, 0.0], 2, 3),
            ]
        );
        let lines: Vec<(i32, Vec<u32>)> = f
            .lines
            .iter()
            .map(|l| (l.depth, l.indices.clone()))
            .collect();
        assert_eq!(lines, [(0, vec![0]), (1, vec![1, 2]), (2, vec![3])]);
        assert!(
            f.slots
                .iter()
                .all(|s| (s.unit_id, s.active, s.time) == (-1, 1, 99))
        );
        // without statistics the slots take no category
        let mut f = Formation::new();
        assert!(create_phalanx(&mut f, &group(2, 2.0, &[]), 0));
        assert!(f.slots.iter().all(|s| s.desired_category == -1));
    }

    /// `FUN_14102ab30`'s fixed places for 3 units, a box too small for 2 units of spacing 10
    /// (grown by 1.5 each way), and the walk of 16 around a box 4 wide and 2 deep: up the side
    /// at max x, the first corner's x is max z less the step's rest (1), back along max z and
    /// down min x, then from that corner's x along min z until past min x, then the first place.
    #[test]
    fn the_box_perimeter_places_its_units_as_the_game_walks_it() {
        let mut out = [[0.0f32; 3]; BOX_POSITIONS];
        let (mut min, mut max) = ([-1.0, 0.0, -1.0], [1.0, 0.0, 1.0]);
        assert!(place_box_perimeter(3, &mut out, &mut min, &mut max, 1.0));
        assert_eq!(
            out[..3],
            [[1.0, 0.0, -1.0], [1.0, 0.0, 1.0], [-1.0, 0.0, 0.0]]
        );
        assert!(place_box_perimeter(2, &mut out, &mut min, &mut max, 10.0));
        assert_eq!((min, max), ([-2.5, 0.0, -2.5], [2.5, 0.0, 2.5]));
        assert_eq!(out[..2], [[2.5, 0.0, 0.0], [-2.5, 0.0, 0.0]]);
        let (mut min, mut max) = ([-2.0, 0.0, -1.0], [2.0, 0.0, 1.0]);
        assert!(place_box_perimeter(16, &mut out, &mut min, &mut max, 0.5));
        let walked: Vec<(f32, f32)> = out[..16].iter().map(|p| (p[0], p[2])).collect();
        assert_eq!(
            walked,
            [
                (2.0, -1.0),
                (2.0, 0.0),
                (1.0, 1.0),
                (0.0, 1.0),
                (-1.0, 1.0),
                (-2.0, 1.0),
                (-2.0, 0.0),
                (1.0, -1.0),
                (0.0, -1.0),
                (-1.0, -1.0),
                (2.0, -1.0),
                (2.0, -1.0),
                (2.0, -1.0),
                (2.0, -1.0),
                (2.0, -1.0),
                (2.0, -1.0),
            ]
        );
        // none, or more than the buffer holds, writes nothing
        assert!(!place_box_perimeter(0, &mut out, &mut min, &mut max, 1.0));
        assert!(!place_box_perimeter(61, &mut out, &mut min, &mut max, 1.0));
    }

    /// `FUN_1410143a0`: slots within 1e-7 on x share a line; a line of an array large enough
    /// keeps its distance back and loses its attack range.
    #[test]
    fn create_lines_groups_the_slots_by_their_offset_on_x() {
        let mut f = Formation::new();
        f.slots = [0.25, -1.0, 0.250_000_06, 3.0]
            .iter()
            .map(|&x| Slot {
                offset: [x, 0.0, 0.0],
                ..Slot::default()
            })
            .collect();
        f.lines = vec![
            FormationLine {
                attack_range: 4.0,
                distance_back: 2.0,
                ..FormationLine::default()
            };
            3
        ];
        create_lines(&mut f);
        assert_eq!(f.number_lines, 3);
        let lines: Vec<(i32, Vec<u32>)> = f
            .lines
            .iter()
            .map(|l| (l.depth, l.indices.clone()))
            .collect();
        assert_eq!(lines, [(0, vec![0, 2]), (1, vec![1]), (2, vec![3])]);
        assert_eq!(
            (f.lines[0].attack_range, f.lines[0].distance_back),
            (0.0, 2.0)
        );
    }

    /// `FUN_141011a00`: category 5's lone unit in the middle, then category 1's three on the box
    /// a spacing (2) wider each way; the box 4 wide; its lines by x.
    #[test]
    fn a_box_rings_its_categories_from_the_middle_out() {
        let g = group(4, 2.0, &[(1, 3), (5, 1)]);
        let mut f = Formation::new();
        assert!(create_box(&mut f, &g, 5).unwrap());
        assert_eq!(
            (f.kind, f.transform_type, f.maximum_width, f.maximum_depth),
            (BOX, 1, 4, 4)
        );
        let slots: Vec<(Vec3, i32)> = f
            .slots
            .iter()
            .map(|s| (s.offset, s.desired_category))
            .collect();
        assert_eq!(
            slots,
            [
                ([0.0, 0.0, 0.0], 5),
                ([2.0, 0.0, -2.0], 1),
                ([2.0, 0.0, 2.0], 1),
                ([-2.0, 0.0, 0.0], 1),
            ]
        );
        let lines: Vec<Vec<u32>> = f.lines.iter().map(|l| l.indices.clone()).collect();
        assert_eq!(lines, [vec![0], vec![1, 2], vec![3]]);
        assert_eq!(f.number_lines, 3);
        // more than the buffer holds of one category
        let g = group(61, 2.0, &[(1, 61)]);
        assert!(create_box(&mut Formation::new(), &g, 5).is_err());
    }
}
