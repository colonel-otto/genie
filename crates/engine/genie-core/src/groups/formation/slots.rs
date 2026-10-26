//! The slots' units: the unit categories, `ClosestUnit`, the assignment by variance, the
//! population in order and a line after a column.

use crate::GenieError;
use crate::groups::formation::{BOX, COLUMN};
use crate::groups::{Formation, UnitGroup, sub_formation};
use crate::math::fmath::{Vec3, sub};
use crate::math::image::group::FARTHEST;
use crate::world::Turns;

/// The capacity of `ClosestUnit`'s list of the units the slots took (`DAT_143f20710`, 0x118
/// bytes).
pub(super) const USED_UNITS: usize = 70;
/// The feature's assignment's arrays (`FUN_14100a0c0`): 70 members or slots a side
/// (`DAT_143f1fb50`, `DAT_143f1fc70`, 0x118 bytes each) ...
pub(super) const ASSIGNMENT_SIDE: usize = 70;
/// ... the costs and the preferences 70 × 70 (`DAT_143f161e0`, `DAT_143f1ae70`, 0x4c90 bytes
/// each) ...
pub(super) const ASSIGNMENT_CELLS: usize = ASSIGNMENT_SIDE * ASSIGNMENT_SIDE;
/// ... and the taken marks 80 bytes (`DAT_143f1fb00`).
pub(super) const ASSIGNMENT_MARKS: usize = 80;

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
        object: Option<crate::groups::WorldObject<'_>>,
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
    pub(super) fn slot_distance(&self, at: Vec3, id: i32) -> Option<f32> {
        let object = self.world_object(id)?;
        let mut d = sub(at, object.moved_vector());
        d[1] = 0.0;
        Some(crate::math::fmath::sqrt(d[0] * d[0] + 0.0 + d[2] * d[2]))
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
    /// takes fewer than 70 members ([`crate::groups::create`]'s `AddUnit`) and its layouts a slot
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
        let preferences = slot_preferences(costs, n, s);
        let mut taken = [false; ASSIGNMENT_MARKS];
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
            let variance = pairs_variance(&pairs.0[..s], &pairs.1[..s], costs, n, sum / n as f32);
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
    pub(super) fn populate_in_order(
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
}

/// [`Turns::assign_slots_by_variance`]'s preferences: for each of the `n` members (rows of `n`
/// cells in `costs`) its `s` slots from the cheapest on, the first untaken one winning a tie.
pub(super) fn slot_preferences(costs: &[f32], n: usize, s: usize) -> Vec<i32> {
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
    preferences
}

/// [`Turns::assign_slots_by_variance`]'s variance of one pairing: of the pairs (`members`,
/// `slots`) with neither −1, the mean squared gap of their costs to `mean` (the sum over the
/// group's size).
pub(super) fn pairs_variance(
    members: &[i32],
    slots: &[i32],
    costs: &[f32],
    n: usize,
    mean: f32,
) -> f32 {
    let (mut variance, mut count) = (0.0f32, 0i32);
    for (&r, &c) in members.iter().zip(slots) {
        if r != -1 && c != -1 {
            count += 1;
            let d = costs[r as usize * n + c as usize] - mean;
            variance += d * d;
        }
    }
    variance / count as f32
}
