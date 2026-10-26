//! `FormationTransform` (`FUN_14103a1f0`) and its passability: the grid over the box, the
//! slots' reach and the search for an unreachable slot's unit.

use crate::GenieError;
use crate::checkpoint::cvttss2si;
use crate::group::formation::{RIGHT_ANGLE, find_point_from_front, transform_slot};
use crate::group::{Formation, Slot, UnitGroup};
use crate::math::fmath::{Vec3, scale, sub};
use crate::math::image::group::{FARTHEST, SLOT_NUDGE_X, SLOT_NUDGE_Z};
use crate::turn::Turns;

/// The passability's box starts at this (`DAT_142eea788`) on its low sides, 0 on its high ones.
pub(super) const PASS_FAR: f32 = crate::math::image::group::PASS_FAR;
/// The passability's grid holds the box plus this (`DAT_142eb0f50`) and one more cell
/// ([`PASS_ONE`]) a side.
pub(super) const PASS_MARGIN: f32 = crate::math::image::group::PASS_MARGIN;
/// The grid starts this far below the box (`DAT_142ed6f20`).
pub(super) const PASS_HALF_MARGIN: f32 = crate::math::image::group::PASS_HALF_MARGIN;
/// Below this share of the slots' array reachable (`DAT_142eae8d0`), only the nearest slot is.
pub(super) const PASS_SHARE: f32 = crate::math::image::group::PASS_SHARE;
/// The grid's extra cell, and the spacing below which slots are nudged (`DAT_14330eb8c`).
pub(super) const PASS_ONE: f32 = crate::math::image::group::PASS_ONE;
/// A slot lies in its cell's middle (`DAT_142e7f384`).
pub(super) const PASS_HALF: f32 = crate::math::image::group::PASS_HALF;
/// The grid's sides are at most this many cells (the literal `0x64` at `0x14103bf01`).
pub(super) const PASS_CELLS: i32 = 100;
/// The grid's bytes (`local_2948`, cleared by the literal `0x2710` at `0x14103bf36`).
pub(super) const GRID_BYTES: usize = 10000;
/// The nudges of a slot by its cell's count (the tables `DAT_142f49470` and `DAT_142f49dd0`
/// with the literals 0 and 0.25 after them, `0x14103cd8c` and `0x14103cda4`), x and y.
pub(super) const SLOT_NUDGE: [[f32; 2]; 5] = [
    [SLOT_NUDGE_X[0], SLOT_NUDGE_Z[0]],
    [SLOT_NUDGE_X[1], SLOT_NUDGE_Z[1]],
    [SLOT_NUDGE_X[2], SLOT_NUDGE_Z[2]],
    [SLOT_NUDGE_X[3], SLOT_NUDGE_Z[3]],
    [0.0, 0.25],
];

impl Turns {
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
        let rotation = crate::math::fmath::rotation(yaw, up);
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
                        if crate::math::fmath::at_most_or_unordered(back, 0.0) {
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
    pub(super) fn passable_slots(
        &self,
        formation: &mut Formation,
        g: &UnitGroup,
    ) -> Result<(), GenieError> {
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
        // the box's temporary tile records (FUN_141052b90 at 0x14103b8fa) with the buffer the
        // last FUN_1410562d0 left, which the lines grow by too
        let grow = self.spatial.manager_grow();
        self.spatial
            .put_tile_records([min_x, min_z], [max_x, max_z], grow, mask);
        let members: Vec<i32> = g.members.iter().map(|m| m.id).collect();
        // the members are the manager's ignore list while the passability runs
        // (FUN_1410562e0, 0x14103b916), which every way out of it clears (0x14103ce93,
        // 0x14103cf93): none after it
        self.spatial.set_manager_ignore(&[]);
        let (mut reachable, mut unreachable) =
            self.slot_reach(formation, position, grow, &members)?;
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
            // the records go (FUN_1410561f0 at 0x14103cf9f)
            self.spatial.clear_tile_records();
            return Ok(());
        }
        let ox = cvttss2si(min_x - PASS_HALF_MARGIN);
        let oz = cvttss2si(min_z - PASS_HALF_MARGIN);
        let mut grid = PassGrid {
            cells: vec![0u8; GRID_BYTES],
            w,
            h,
            ox,
            oz,
        };
        let near = formation.slots[k].transform_position;
        for &r in &reachable {
            let p = formation.slots[r].transform_position;
            let gx = cvttss2si(p[0] - ox as f32);
            let gz = cvttss2si(p[2] - oz as f32);
            if !(0 <= gx && gx < w && 0 <= gz && gz < h) {
                continue;
            }
            let cell = (gz * w + gx) as usize;
            grid.cells[cell] = cell_mark(grid.cells[cell], near[0] - p[0], near[2] - p[2]);
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
            let Some((fx, fy, nudge)) =
                self.search_cell(&mut grid, &mut visited, (gx, gy), unit_mask)
            else {
                continue;
            };
            // CVTTSS2SI at 0x14103cb6a and 0x14103cb70
            let (x, y) = if slot_tile(formation.slots[k].transform_position) == (fx, fy) {
                (fx, fy)
            } else {
                let Some(at) = self.step_once_more(
                    &mut grid,
                    (fx, fy),
                    [unit_at[0], unit_at[1]],
                    grow,
                    &members,
                )?
                else {
                    continue;
                };
                at
            };
            let s = &mut formation.slots[u];
            // COMISS/JC at 0x14103cde3: a NaN spacing nudges too
            if crate::math::fmath::below_or_unordered(g.desired_spacing, PASS_ONE) {
                s.transform_position[0] = x as f32 + PASS_HALF + SLOT_NUDGE[nudge][0];
                s.transform_position[2] = y as f32 + PASS_HALF + SLOT_NUDGE[nudge][1];
            } else {
                s.transform_position[0] = x as f32 + PASS_HALF;
                s.transform_position[2] = y as f32 + PASS_HALF;
            }
        }
        // the records go (FUN_1410561f0 at 0x14103ce9f)
        self.spatial.clear_tile_records();
        Ok(())
    }

    /// [`Turns::passable_slots`]'s reach of the active slots from the group's `position`: a
    /// slot the line to which hits a record (the members ignored) is unreachable; the reachable
    /// and the unreachable slots' indices, in the slots' order.
    ///
    /// # Errors
    ///
    /// A line query near a record whose place the slice does not know.
    pub(super) fn slot_reach(
        &self,
        formation: &Formation,
        position: Vec3,
        grow: f32,
        members: &[i32],
    ) -> Result<(Vec<usize>, Vec<usize>), GenieError> {
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
                grow,
                members,
            )? {
                unreachable.push(j);
            } else {
                reachable.push(j);
            }
        }
        Ok((reachable, unreachable))
    }

    /// [`Turns::passable_slots`]'s breadth-first search of an unreachable slot's unit from its
    /// cell `start` through the cells it can stand on (`unit_mask`), x+1, x-1, y+1, y-1 in turn,
    /// for a reachable cell: one holding fewer than five slots takes it (counted: its tile and
    /// its count before), a fuller one has the searching cell take its place (one slot, pointing
    /// to it: the searching cell's tile and 0). None from a start off the grid or without a
    /// reachable cell.
    pub(super) fn search_cell(
        &self,
        grid: &mut PassGrid,
        visited: &mut [u8],
        (gx, gy): (i32, i32),
        unit_mask: u8,
    ) -> Option<(i32, i32, usize)> {
        let (w, h, ox, oz) = (grid.w, grid.h, grid.ox, grid.oz);
        if !(0 <= gx && 0 <= gy && gx < w && gy < h) {
            return None;
        }
        let mut queue = vec![(gx, gy)];
        visited[(gy * w + gx) as usize] = 1;
        let mut qi = 0;
        while qi < queue.len() {
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
                let b = grid.cells[cell];
                if b & 1 != 0 {
                    if b & 0x3e < 10 {
                        grid.cells[cell] = count_up(b);
                        return Some((ox + nx, oz + ny, usize::from(b >> 1 & 0x1f)));
                    }
                    grid.cells[(w * cy + cx) as usize] = mark;
                    return Some((ox + cx, oz + cy, 0));
                }
                queue.push((nx, ny));
            }
        }
        None
    }

    /// [`Turns::passable_slots`]'s step once more of a found cell (tile `found`) that is not the
    /// nearest slot's: the way its cell points, when that cell lies on the grid, holds at most
    /// five and the line from the unit (`unit_at`) to its middle is clear (the members ignored),
    /// the slot takes it (counted); the tile the slot takes, or none when the cell pointed to is
    /// off the grid (the slot stays where it is).
    ///
    /// # Errors
    ///
    /// A line query near a record whose place the slice does not know.
    pub(super) fn step_once_more(
        &self,
        grid: &mut PassGrid,
        (fx, fy): (i32, i32),
        unit_at: [f32; 2],
        grow: f32,
        members: &[i32],
    ) -> Result<Option<(i32, i32)>, GenieError> {
        let (w, h) = (grid.w, grid.h);
        let (rx, ry) = (fx - grid.ox, fy - grid.oz);
        let (ddx, ddy) = match grid.cells[(w * ry + rx) as usize] >> 6 {
            0 => (0, 1),
            1 => (1, 0),
            2 => (0, -1),
            _ => (-1, 0),
        };
        let (nx, ny) = (ddx + rx, ddy + ry);
        if ny < 0 || h <= ny || nx < 0 || w <= nx {
            return Ok(None);
        }
        let cell = (w * ny + nx) as usize;
        let b = grid.cells[cell];
        if b & 0x3e < 0xb
            && !self.spatial.still_line_hit(
                unit_at,
                [(ddx + fx) as f32 + PASS_HALF, (fy + ddy) as f32 + PASS_HALF],
                grow,
                members,
            )?
        {
            grid.cells[cell] = count_up(b);
            return Ok(Some((ddx + fx, fy + ddy)));
        }
        Ok(Some((fx, fy)))
    }
}

/// [`Turns::passable_slots`]'s grid over the box, a byte per tile (`w` by `h` from the tile
/// (`ox`, `oz`)): bit 0 a reachable slot's cell, bits 1 to 5 the slots it holds, bits 6 and 7
/// the way to the nearest slot.
pub(super) struct PassGrid {
    pub(super) cells: Vec<u8>,
    pub(super) w: i32,
    pub(super) h: i32,
    pub(super) ox: i32,
    pub(super) oz: i32,
}

/// `FUN_14103a1f0`'s grid cell count up: bits 1 to 5 (the slots a cell holds) count on (mod 32),
/// the others kept.
pub(super) const fn count_up(b: u8) -> u8 {
    (((b & 0xfe).wrapping_add(2) ^ b) & 0x3e) ^ b
}

/// The tile of a slot's place the passability tests (`FUN_141185690`'s arguments): x and y
/// by `CVTTSS2SI` (0x14103b5df, 0x14103b777), a NaN or out of range one `i32::MIN`.
pub(super) fn slot_tile(at: Vec3) -> (i32, i32) {
    (cvttss2si(at[0]), cvttss2si(at[2]))
}

/// The passability's box and nearest slot (0x14103b650..0x14103b758): from the box
/// ([`PASS_FAR`], [`PASS_FAR`]) to (0, 0), each active slot of `slots` widens it first by the
/// tester's place `at` (slot 0x520) and then by its own, as `MINSS new, old` and `MAXSS new, old`
/// take them (the old bound kept for a NaN either side); the nearest is the first active slot
/// whose squared distance to the group's place `position` (summed x, height, y) lies below the
/// best so far, from [`FARTHEST`] (`COMISS`/`CMOVBE`: a NaN distance keeps the earlier one, none
/// when no slot lies below `f32::MAX`). Returns min x, min y, max x, max y and the nearest.
pub(super) fn passability_box(
    slots: &[Slot],
    at: Vec3,
    position: Vec3,
) -> ([f32; 4], Option<usize>) {
    use crate::math::fmath::{maxss, minss};
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
pub(super) fn cell_mark(b: u8, dx: f32, dz: f32) -> u8 {
    use crate::math::fmath::{at_most_or_unordered, below_or_unordered};
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
