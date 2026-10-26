//! createColumnLine (`FUN_141012280`): the sizes and the slots of its sub-formations.

use crate::GenieError;
use crate::groups::formation::{
    CATEGORIES, COLUMN, EPSILON, FLANK_SPACING, FLANKED, LINE, STAGGER_SHARE, STAGGERED, TWO,
};
use crate::groups::layout::{clear_line, reset_line};
use crate::groups::{Formation, Slot, UnitGroup};
use crate::math::conversions::cvttss2si;
use crate::math::fmath::{Vec3, add, scale, sub};
use crate::world::World;

/// What createColumnLine reads of one sub-formation's statistic.
#[derive(Clone, Debug)]
pub(super) struct Category {
    pub(super) count: i32,
    pub(super) spacing: f32,
    pub(super) types: i32,
    pub(super) unit_types: Vec<u32>,
    pub(super) unit_type_counts: Vec<u32>,
}

/// The line and column counts of a sub-formation of `count` units for a line (`by_line`, the
/// column ratio -1) or a column: the root of the count, rounded up, as the lines (a line's) or
/// the columns (a column's), then fewer lines until a line is at least twice as wide as the
/// formation is deep (a column the reverse). Returns (lines, columns).
pub(super) fn sizes(count: i32, by_line: bool) -> (i32, i32) {
    let root = cvttss2si(crate::math::fmath::sqrt(count as f32).ceil());
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

/// The slots of createColumnLine (`FUN_141012280`, [`World::create_column_line`])
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
/// ([`crate::math::conversions::cvttss2si`]), which the clamps to 1 take in.
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

impl World {
    /// createColumnLine (`FUN_141012280`) of `formation` for group `g`: a column (`column`, the
    /// column ratio 2 and line ratio -1) or a line (ratios -1 and 2), staggered or flanked;
    /// false without members. Its slots ([`column_line_slots`]), then the transform and the
    /// units ([`World::lay_units`]) with the column-to-line flag (0x141013d20..0x141013d3a): a
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
