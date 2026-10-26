//! The way round a record on the line to the slot: the short pather, the long pather and the
//! tile planner.

use crate::group::motion::moved_vector;
use crate::group::steer::{
    BLOCKED, EPSILON, MOVES, OFF_MAP, PARTIAL, REACHED, ROUND_LENGTH, ROUND_RADIUS, ROUND_REACH,
    ROUND_STILL, ROUND_TIMER,
};
use crate::math::fmath::{Vec3, at_most_or_unordered};
use crate::turn::Turns;
use crate::{GenieError, math::fmath};

impl Turns {
    /// The way round a record on the line to the slot (`0x141026867`..`0x1410277e5`), for member
    /// `index` (`id`) of group `group` in slot `slot`, `gap` the ground distance to it. A new path
    /// record (`FUN_14152eea0`: flags 6, made at the world's time) takes the first way that
    /// serves, each try emptying its points first; the short pather's radius is the member's on x
    /// times [`ROUND_RADIUS`] and its ignore list `differentiators`; a position test is slot 0x270
    /// ([`Spatial::point_clear_among`], the records grown by the same radius, the same list):
    /// 1. The slot, when the member could stand there: for a unit with a footprint and a height
    ///    (`FUN_141194060`) whose footprint is within [`ROUND_REACH`] of it (`FUN_140a847c0`, the
    ///    squared gap; `COMISS`, `JBE`), the short pather from where it last moved to (slot 0x520);
    ///    a path of fewer than 2 points or no longer than [`EPSILON`] is blocked (3, its points
    ///    emptied), a result but blocked or off the map (3, 4) serves. Otherwise the tile planner
    ///    of its restriction (`FUN_140f7cb30` -> `FUN_140f7c530`: no radii, no range, no unit),
    ///    which marks the path as planned.
    /// 2. Unless reached (1): one tile from where it last moved to toward the slot (the
    ///    direction on the ground, the group's forward when both its axes are within
    ///    [`ROUND_STILL`]), held inside the map by its radius on x and, on y, its height radius
    ///    (`+0x5e0`), when it could stand there: the long pather from its place's tile for a unit
    ///    without a footprint and a height (reached: 1), else the short pather; a path longer than
    ///    [`ROUND_LENGTH`], of fewer than 2 points, no longer than [`EPSILON`] or not reached is
    ///    blocked.
    /// 3. Unless reached or partial: the group's place, when it could stand there, by the short
    ///    pather (as in 1, the result kept only for a path of 2 points longer than [`EPSILON`])
    ///    or the tile planner.
    /// 4. Blocked or off the map: the long pather from where it last moved to toward the slot's
    ///    tile; a path of more than 2 points serves, its last point the goal.
    ///
    /// Any other result (in range, none) or a failed long path releases the record. A served way
    /// that would end farther from the slot than `gap` while the group neither moves nor waits
    /// stops the member with its standing sprite, its failures cleared, the record released and
    /// the slot in state 3. Else slot 0x540 (`FUN_14114e0f0`) gives the member the record's path
    /// as its path data, the walked level reset: as level 0 walking it (state 3,
    /// `FUN_140f9b7c0`), or, planned, as level 2 to plan (state 2, `FUN_140f9b800`), range 0, no
    /// target, its repath time kept; its own move takes the record (reason 0, the goal, timer and
    /// distance [`ROUND_TIMER`], [`Turns::set_specific_move`]). Either way the slot takes state 0,
    /// the member stops (slot 0x5d0) and its failures count one more. Not held: the record's id
    /// (the own move's `mPathID`, -1 here: the slice holds the path in the member's path data).
    ///
    /// # Errors
    ///
    /// What the pathers, the planner and the position test do not model.
    pub(super) fn way_round(
        &mut self,
        group: i32,
        index: usize,
        slot: usize,
        gap: f32,
        differentiators: &[i32],
    ) -> Result<(), GenieError> {
        use crate::tile_path::Path;
        let g = self.groups.get(group).expect("held");
        let id = g.members[index].id;
        let to = g.current_formation.as_ref().expect("a formation").slots[slot].transform_position;
        let [place, forward, _, _] = g.motion.position;
        let (moving, waits) = (g.motion.moving(), g.motion.waits_to_move());
        let (p, u) = self.object(id).expect("the member is held");
        let master = self.master_of(p, u.life.master_id)?;
        let (radius, height) = (master.radius, master.height);
        let mover = u.mover.as_ref().expect(MOVES);
        let solid = mover.stats.radius.iter().all(|&r| 0.0 < r);
        let (at, here) = (moved_vector(u), u.position);
        let mask = mover.stats.restriction_mask;
        let restriction = mover.stats.restriction;
        let computer = self.players.get(p).is_some_and(|q| q.computer);
        let walker = crate::movement::Walker {
            radius: radius[0] * ROUND_RADIUS,
            ..mover.walker(id)
        };
        let grow = radius[0] * ROUND_RADIUS;
        // FUN_140a847c0: the squared gap from the unit's footprint (its place) to a point
        let near = |x: f32, y: f32| {
            let gx = ((x - here[0]).abs() - radius[0]).max(0.0);
            let gy = ((y - here[1]).abs() - radius[1]).max(0.0);
            gy * gy + gx * gx < ROUND_REACH
        };
        let mut record = Path::default();
        let mut result = BLOCKED;
        let mut planned = false;
        let mut goal = to;
        if self
            .spatial
            .point_clear_among(id, [to[0], to[2]], grow, mask, differentiators)?
        {
            let mut served = false;
            if solid && near(to[0], to[2]) {
                let r = self.round_short(
                    walker,
                    computer,
                    [at[0], at[2]],
                    to,
                    differentiators,
                    &mut record,
                )?;
                if record.points.len() < 2 || at_most_or_unordered(record.length, EPSILON) {
                    record.points.clear();
                } else {
                    result = r;
                    served = !matches!(r, BLOCKED | OFF_MAP);
                }
            }
            if !served
                && let Some(r) = self.round_plan(restriction, [at[0], at[2]], to, &mut record)?
            {
                result = r;
                planned = true;
            }
        }
        if result != REACHED {
            let d = fmath::sub(to, at);
            let mut dir = if d[0].abs() <= ROUND_STILL && d[2].abs() <= ROUND_STILL {
                forward
            } else {
                d
            };
            dir[1] = 0.0;
            let len = fmath::sqrt(dir[0] * dir[0] + 0.0 + dir[2] * dir[2]);
            let dir = [dir[0] / len, dir[1] / len, dir[2] / len];
            let mut step = fmath::add(at, fmath::scale(1.0, dir));
            let [w, h] = self.spatial.map;
            let (top_x, top_y) = (w as f32 - radius[0], h as f32 - height);
            if radius[0] > step[0] {
                step[0] = radius[0];
            } else if step[0] > top_x {
                step[0] = top_x;
            }
            if height > step[2] {
                step[2] = height;
            } else if step[2] > top_y {
                step[2] = top_y;
            }
            goal = step;
            if self.spatial.point_clear_among(
                id,
                [step[0], step[2]],
                grow,
                mask,
                differentiators,
            )? {
                record.points.clear();
                let mut reached = false;
                if solid {
                    result = self.round_short(
                        walker,
                        computer,
                        [at[0], at[2]],
                        step,
                        differentiators,
                        &mut record,
                    )?;
                } else {
                    let (found, path) = self.round_long(id, [here[0], here[1]], step)?;
                    record.points = path.points;
                    // a long path found is measured once and taken as it is
                    if found {
                        result = REACHED;
                        record.measure();
                        reached = true;
                    } else {
                        record.measure();
                    }
                }
                // COMISS with JA (a NaN length passes), then JNC
                if !reached
                    && (result != REACHED
                        || ROUND_LENGTH < record.length
                        || record.points.len() < 2
                        || at_most_or_unordered(record.length, EPSILON))
                {
                    result = BLOCKED;
                    record.points.clear();
                }
            }
            if !matches!(result, REACHED | PARTIAL) {
                goal = place;
                if self.spatial.point_clear_among(
                    id,
                    [place[0], place[2]],
                    grow,
                    mask,
                    differentiators,
                )? {
                    record.points.clear();
                    let short = solid && near(place[0], place[2]);
                    if short {
                        let r = self.round_short(
                            walker,
                            computer,
                            [at[0], at[2]],
                            place,
                            differentiators,
                            &mut record,
                        )?;
                        result = r;
                        if record.points.len() < 2 || at_most_or_unordered(record.length, EPSILON) {
                            result = BLOCKED;
                            record.points.clear();
                        }
                    } else if let Some(r) =
                        self.round_plan(restriction, [at[0], at[2]], place, &mut record)?
                    {
                        result = r;
                        planned = true;
                    }
                }
            }
        }
        let mut serves = matches!(result, REACHED | PARTIAL);
        if matches!(result, BLOCKED | OFF_MAP) {
            record.points.clear();
            let (_, path) = self.round_long(id, [at[0], at[2]], to)?;
            record.points = path.points;
            if 2 < record.points.len() {
                goal = *record.points.last().expect("more than 2 points");
                serves = true;
            }
        }
        if serves && !moving && !waits {
            // FUN_140f9bcd0 outside the path: its static point
            let end = record
                .points
                .last()
                .copied()
                .unwrap_or(crate::movement::OUTSIDE_PATH);
            let mut d = fmath::sub(to, end);
            d[1] = 0.0;
            if gap < fmath::sqrt(d[0] * d[0] + 0.0 + d[2] * d[2]) {
                self.stop_member(id)?;
                self.show_standing(id)?;
                self.groups.get_mut(group).expect("held").members[index]
                    .specific_move
                    .consecutive_move_failures = 0;
                self.set_slot_state(group, slot, 3);
                return Ok(());
            }
        }
        if serves {
            let now = self.world_time;
            if let Some((_, u)) = self.object_mut(id)
                && let Some(m) = u.mover.as_mut()
            {
                m.take_record_path(record.points, record.length, now, planned);
            }
            let _ = self.set_specific_move(group, id, (-1, 0), (goal, ROUND_TIMER, ROUND_TIMER));
        }
        self.set_slot_state(group, slot, 0);
        self.stop_member(id)?;
        self.groups.get_mut(group).expect("held").members[index]
            .specific_move
            .consecutive_move_failures += 1;
        Ok(())
    }

    /// The way round's short pather ([`Turns::way_round`], `FUN_140f9e540` with the record) for
    /// `walker` (of a `computer` player or not) from `from` to `to`, range 0, no target, `ignore`
    /// its ignore list; the record measured after (`FUN_140f95a60`). Its result.
    pub(super) fn round_short(
        &mut self,
        walker: crate::movement::Walker,
        computer: bool,
        from: [f32; 2],
        to: Vec3,
        ignore: &[i32],
        record: &mut crate::tile_path::Path,
    ) -> Result<u8, GenieError> {
        let mut plan = crate::movement::Planning {
            pathing: &mut self.pathing,
            now: self.world_time,
            computer,
            object: self.streams.get(crate::math::rng::Stream::Object),
            construction_rate: 0.0,
            groups: &self.groups,
        };
        let r = crate::movement::short_path_for(
            walker,
            from,
            &self.spatial,
            &mut plan,
            (to, 0.0, -1),
            ignore,
            record,
        )?;
        record.measure();
        Ok(r)
    }

    /// The way round's long pather (`FUN_140f9eb70` with the record) for member `id` from the
    /// tile of `from` to the tile of `to`, range 0, no target: whether it reached the goal, and
    /// the path.
    pub(super) fn round_long(
        &self,
        id: i32,
        from: [f32; 2],
        to: Vec3,
    ) -> Result<(bool, crate::tile_path::Path), GenieError> {
        let (p, u) = self.object(id).expect("the member is held");
        let mover = u.mover.as_ref().expect(MOVES);
        let computer = self.players.get(p).is_some_and(|q| q.computer);
        let walker = crate::movement::LongWalker {
            id,
            position: u.position,
            radius: [mover.stats.radius[0], mover.stats.radius[1]],
            mask: mover.stats.restriction_mask,
            moves: mover.stats.speed != 0.0 || u.life.master_id == crate::movement::LONG_PATH_MOVER,
            ignore_units: &mover.ignore_units,
            group_units: self.groups.group_units(u.group.id),
        };
        crate::movement::long_path_for(&walker, from, &self.spatial, computer, (to, 0.0), None)
    }

    /// The way round's tile planner (`FUN_140f7cb30` -> `FUN_140f7c530`): the planner of
    /// `restriction` from `from` to `to` with no radii, no range and no unit, appending to the
    /// record; its result, None without a planner.
    pub(super) fn round_plan(
        &mut self,
        restriction: usize,
        from: [f32; 2],
        to: Vec3,
        record: &mut crate::tile_path::Path,
    ) -> Result<Option<u8>, GenieError> {
        let tiles = self.spatial.tiles();
        let Some(planner) = self.pathing.planner(restriction, tiles) else {
            return Ok(None);
        };
        record.points.clear();
        let req = crate::tile_path::Request {
            from,
            to: [to[0], to[2]],
            radii: 0.0,
            range: 0.0,
            closed: Vec::new(),
            full: false,
        };
        let found = planner.plan::<GenieError>(&req, false, record, None)?;
        Ok(Some(found.code()))
    }
}
