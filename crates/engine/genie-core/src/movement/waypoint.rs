//! A reached waypoint and what follows it: the next waypoint, a level's end and the moving state.

use crate::math::fmath::Vec3;
use crate::movement::{
    CLOSE, EPSILON, Goal, LOOK_AHEAD, Me, Motion, Mover, Next, PathData, RANGE_FLOOR,
    STRAIGHT_GROW, TARGET_SHORT_REACH, drop_edge_waypoints, footprint_gap, gate_gap, looping_next,
    map_position, point_or_zero_of, remove_point, vector,
};
use crate::pathing::Pathing;
use crate::spatial::Spatial;
use crate::{GenieError, math::fmath};

impl Mover {
    /// The waypoint the level walks to was reached (`LAB_14115136c`, the level's bit 0): with
    /// bit 1 the unit takes the waypoint's place (slot 0x58, the position setter); the level moves
    /// on to its next waypoint, dropping those too near the map's edge for the unit
    /// (`FUN_140f98d50`). Then (`LAB_14115192d`) a level with waypoints left waits for a path to
    /// the next (state 2, not level 0); at its last, a unit not yet in range of where it was going
    /// steps back to the waypoint it left and asks again, else the level arrived (state 4). Level
    /// 0 on a level 1 parent looks ahead first ([`Mover::look_ahead`]). A loop (the path's flags
    /// bit 0) moves on from the waypoint reached instead (`FUN_140f98bc0` with 1,
    /// [`looping_next`]) and waits for a path to it (state 2): back at its first waypoint the step
    /// ends ([`Mover::loop_end`]), else on to the level's state. A waypoint some object stands on
    /// is not modelled.
    pub(super) fn reached(
        &mut self,
        me: &mut Me,
        world: &mut Spatial,
        pathing: Option<&mut Pathing>,
        goal: Goal<'_>,
        now: u32,
    ) -> Result<Next, GenieError> {
        let ground_radius = [self.stats.radius[0], self.stats.radius[1]];
        if self.paths[0].flags & 2 != 0 {
            self.take_waypoint_place(me, world, pathing, now);
        }
        let level = &mut self.paths[0];
        level.flags &= !1;
        level.waypoint += 1;
        drop_edge_waypoints(level, self.stats.radius, world.map);
        if level.path_flags & 1 != 0 {
            let reached = level.waypoint - 1;
            level.waypoint = looping_next(level, reached);
            level.state = 2;
            return Ok(if level.waypoint == 0 {
                Next::LoopEnd
            } else {
                Next::Dispatch
            });
        }
        self.drop_held_waypoint(me, world)?;
        let parent_is_level_1 = self
            .paths
            .get(1)
            .is_some_and(|p| p.level == 1 && !p.points.is_empty());
        if self.paths[0].level == 0 && parent_is_level_1 && self.look_ahead(me, world)? {
            return Ok(Next::Again);
        }
        // LAB_14115192d
        let level = &self.paths[0];
        let left = point_or_zero_of(level, level.waypoint - 1);
        let back = footprint_gap(
            me.position,
            ground_radius,
            [left[0], left[2], 0.0],
            [0.0, 0.0],
        );
        let range = level.range;
        let mut beyond = match goal.target {
            None => (back - range) - RANGE_FLOOR,
            Some(t) => {
                let own = (footprint_gap(me.position, ground_radius, t.position, t.radius) - range)
                    - RANGE_FLOOR;
                let near = (me.position, ground_radius);
                gate_gap(world, near, (level.target, t), own, |g| {
                    (g - range) - RANGE_FLOOR
                })
            }
        };
        if goal.target.is_some() && level.flags & 4 != 0 {
            self.clear_bit_2();
            beyond = 0.0;
        }
        let level = &mut self.paths[0];
        if level.waypoint < level.points.len() as i32 {
            if level.level != 0 {
                level.state = 2;
            }
            return Ok(Next::Dispatch);
        }
        if EPSILON <= beyond && (goal.target.is_some() || CLOSE <= beyond || level.level == 0) {
            if EPSILON <= back {
                level.waypoint -= 1;
                level.state = 2;
                return Ok(Next::Dispatch);
            }
            if level.level == 2 {
                return self.level_2_short(me, world, goal);
            }
        }
        level.state = 4;
        Ok(Next::Dispatch)
    }

    /// [`Mover::reached`]'s bit 1: the unit takes the waypoint's place (slot 0x58, the position
    /// setter, on the ground), and the bit goes.
    pub(super) fn take_waypoint_place(
        &mut self,
        me: &mut Me,
        world: &mut Spatial,
        pathing: Option<&mut Pathing>,
        now: u32,
    ) {
        let level = &self.paths[0];
        if level.waypoint < level.points.len() as i32 {
            let p = level.point(level.waypoint);
            let at = self.set_position(map_position(p), world.map);
            let at = self.grounded(at, world.terrain.as_ref());
            self.moved_to(me, at, world, pathing, now);
        }
        self.paths[0].flags &= !2;
    }

    /// [`Mover::reached`]'s next waypoint on a level but 0: when the unit lies within
    /// [`TARGET_SHORT_REACH`] (squared) of the one after it, the manager's point query at it
    /// (`FUN_141051900` with flags 0x20; its buffer and ignore list as the last
    /// `FUN_1410562d0` and `FUN_1410562e0` left them; a gate the owner may pass has no record
    /// during its owner's update, [`crate::turn::Turns::open_gates`], so slot 0x678 passes none
    /// of those found) drops it when an object holds it.
    ///
    /// # Errors
    ///
    /// A frozen object the query may meet.
    pub(super) fn drop_held_waypoint(
        &mut self,
        me: &Me,
        world: &Spatial,
    ) -> Result<(), GenieError> {
        let level = &self.paths[0];
        let next = level.waypoint + 1;
        if level.level == 0 || level.points.len() as i32 <= next {
            return Ok(());
        }
        let d = fmath::sub(vector(me.position), level.points[next as usize]);
        // a NaN distance is not within
        if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < TARGET_SHORT_REACH {
            let [x, _, y] = level.points[level.waypoint as usize];
            let ignore = world.manager_ignore();
            if world.object_holds([x, y], world.manager_grow(), &ignore, false)? {
                let level = &mut self.paths[0];
                let removed = remove_point(&mut level.points, level.waypoint);
                debug_assert!(removed, "the waypoint lies within the path");
            }
        }
        Ok(())
    }

    /// [`Mover::reached`] at level 2's last waypoint short of its goal (`0x141154f8a`): a walker
    /// without a weapon range (slot 0x230; `UCOMISS`, a NaN has one) beside a target that moved
    /// since the level's path was made (slot 0x5e8, 0 for an object that never moves, against
    /// the path's `+8`; on a tie the target of a higher player number or a higher id) lets its
    /// path go and stops: it arrived (`FUN_140f98e80`, slot 0x5d0; the step returns 2). Else the
    /// level takes state 6 and the unit stops (1). The step's temporary shapes go either way.
    ///
    /// # Errors
    ///
    /// What the stop does not model.
    pub(super) fn level_2_short(
        &mut self,
        me: &Me,
        world: &Spatial,
        goal: Goal<'_>,
    ) -> Result<Next, GenieError> {
        let level = &self.paths[0];
        let created = level.created;
        let target = level.target;
        let own = world.get(me.id).map(|p| p.player);
        let yields = self.stats.weapon_range == 0.0
            && goal.target.is_some_and(|t| {
                let moved = world.gait(target).map_or(0, |g| g.last_move);
                moved > created
                    || (moved == created && (own.is_some_and(|o| o < t.player) || me.id < target))
            });
        if yields {
            self.reset_level();
            self.stop()?;
            return Ok(Next::Done(Motion::Arrived));
        }
        self.paths[0].state = 6;
        self.stop()?;
        Ok(Next::Done(Motion::Done))
    }

    /// Level 0's look ahead on its level 1 parent, at a waypoint it reached (`LAB_14115136c`):
    /// from the parent's fifth waypoint on (at most its last) back toward its current one, the
    /// first within 10 tiles (squared, `DAT_14330ebcc`) that the unit can walk to straight
    /// (`FUN_141142750`) becomes the parent's waypoint; level 0 goes, and the parent asks for a
    /// path (state 2) as the motion step starts again. True when it did.
    pub(super) fn look_ahead(&mut self, me: &Me, world: &Spatial) -> Result<bool, GenieError> {
        let target = self.paths[0].target;
        let parent = &self.paths[1];
        // the motion step's place of the unit: +0x824, where it last moved to
        let at = vector(self.moved_at);
        let last = parent.points.len() as i32 - 1;
        let from = parent.waypoint;
        let mut k = (from + 5).min(last);
        while from < k {
            let p = parent.points[k as usize];
            let d = fmath::sub(p, at);
            if d[0] * d[0] + 0.0 + d[2] * d[2] <= LOOK_AHEAD
                && self.straight(me, p, target, world)?
            {
                break;
            }
            k -= 1;
        }
        if k == from {
            return Ok(false);
        }
        let _ = self.paths.remove(0);
        let parent = &mut self.paths[0];
        parent.waypoint = k;
        parent.state = 2;
        Ok(true)
    }

    /// `FUN_141142750` with its last argument 1: whether the unit walks straight to `p`. The point
    /// must lie within the map less the unit's radii (the game takes the height radius for the
    /// low edge in y); then the walk's query (`FUN_14114cc60` with the tiles' records, flags
    /// `0x190`, its radius times 0.5) must find nothing but the unit and its target with their
    /// annexes ([`Spatial::query_ignore`]): no tile its
    /// terrain mask lacks, no static record (kinds 2, 11, 13) ([`Spatial::segment_blocked`]).
    /// The game also writes the straight path into a scratch path, which it drops.
    pub(super) fn straight(
        &self,
        me: &Me,
        p: Vec3,
        target: i32,
        world: &Spatial,
    ) -> Result<bool, GenieError> {
        let [rx, ry, rz] = self.stats.radius;
        let [w, h] = world.map;
        let (x, y) = (p[0], p[2]);
        if !(rx <= x && rz <= y && x <= w as f32 - rx && y <= h as f32 - ry) {
            return Ok(false);
        }
        let ignore = world.query_ignore(&[me.id, target]);
        // the query starts at +0x824, where the unit last moved to
        let (from, to) = self.query_ends(vector(self.moved_at), p, world.map);
        let grow = rx * STRAIGHT_GROW;
        world.check_known_records([
            from[0].min(to[0]) - grow,
            from[1].min(to[1]) - grow,
            from[0].max(to[0]) + grow,
            from[1].max(to[1]) + grow,
        ])?;
        let blocked = world.segment_blocked(from, to, grow, self.stats.restriction_mask, &ignore);
        // FUN_14114cc60 resets the buffer it set for the query (FUN_1410562d0 with 0)
        world.set_manager_buffer(None);
        Ok(!blocked)
    }

    /// `FUN_141142750` with its last argument 0, the close-in path to a waypoint or target nearer
    /// than a tile: the point must lie within the map less the unit's radii (as
    /// [`Mover::straight`]); then the walk's query from the unit to the point (`FUN_14114cc60`
    /// with the tiles' records, flags `0x90`, its radius) must find nothing but the unit and its
    /// target: no tile its terrain mask lacks ([`Spatial::tiles_block`]) and no record the walk
    /// hits ([`Spatial::walk_hits`]). The game writes the straight path into the new level.
    pub(super) fn close_in(
        &self,
        me: &Me,
        p: Vec3,
        target: i32,
        world: &Spatial,
    ) -> Result<bool, GenieError> {
        let [rx, ry, rz] = self.stats.radius;
        let [w, h] = world.map;
        let (x, y) = (p[0], p[2]);
        if !(rx <= x && rz <= y && x <= w as f32 - rx && y <= h as f32 - ry) {
            return Ok(false);
        }
        let ignore = world.query_ignore(&[me.id, target]);
        // the query starts at +0x824, where the unit last moved to
        let (from, to) = self.query_ends(vector(self.moved_at), p, world.map);
        let rect = [
            from[0].min(to[0]),
            from[1].min(to[1]),
            from[0].max(to[0]),
            from[1].max(to[1]),
        ];
        world.check_known_records([rect[0] - rx, rect[1] - rx, rect[2] + rx, rect[3] + rx])?;
        if world.tiles_block(from, to, rx, self.stats.restriction_mask) {
            // FUN_14114cc60 resets the buffer it set for the query (FUN_1410562d0 with 0)
            world.set_manager_buffer(None);
            return Ok(false);
        }
        let player = world.get(me.id).map(|q| q.player);
        let hits = world.walk_hits(
            from,
            to,
            rect,
            rx,
            self.stats.min_collision,
            player,
            &ignore,
        )?;
        Ok(hits.is_empty())
    }

    /// An arrived level (states 4 and 5, `LAB_141151b6d`): the top level, or one in range of its
    /// target, is reset (`FUN_140f98e80`) and the unit stops (slot 0x5d0): the step returns 2. Any
    /// other hands over to its parent by its link (`mLinkedPathType`): 1 marks the parent's
    /// waypoint reached, 2 has the parent ask again (a looping parent moves on first), 3 has it
    /// ask again; the step starts again with the parent.
    pub(super) fn arrived(&mut self, goal: Goal<'_>) -> Result<Next, GenieError> {
        let level = &self.paths[0];
        if self.paths.len() == 1 || goal.gap <= level.range {
            self.reset_level();
            self.stop()?;
            return Ok(Next::Done(Motion::Arrived));
        }
        match level.linked_type {
            1 => {
                let _ = self.paths.remove(0);
                self.paths[0].flags |= 1;
            }
            2 => {
                let _ = self.paths.remove(0);
                let parent = &mut self.paths[0];
                parent.state = 2;
                // a looping parent path moves on (FUN_140f98bc0 with 1)
                if parent.has_path && parent.path_flags & 1 != 0 {
                    parent.waypoint = looping_next(parent, parent.waypoint);
                }
            }
            3 => {
                let _ = self.paths.remove(0);
                self.paths[0].state = 2;
            }
            // the game's switch has no case: the level stays and the step goes on past the
            // request to the walk (`LAB_141153bd1`)
            _ => return Ok(Next::Dispatch),
        }
        Ok(Next::Again)
    }

    /// `FUN_140f98e80` on the walked level: its path goes, its parents with it, and it holds the
    /// defaults (its tries back to 10, flags clear; its repath time stays).
    pub(crate) fn reset_level(&mut self) {
        self.paths.truncate(1);
        let level = &mut self.paths[0];
        *level = PathData {
            repath_time: level.repath_time,
            created: 0,
            has_path: false,
            path_flags: 0,
            ..PathData::new(0)
        };
    }

    /// The unit shows its master's standing sprite (slot 0x68 with slot 0x798's): its top speed
    /// (slot 0x208, the master's speed times the current sprite's factor) is the standing
    /// sprite's.
    pub(crate) const fn shows_standing(&mut self) {
        self.stats.speed = self.stats.stand_speed;
    }

    /// `FUN_140f9b4d0(level, 2, 0)`: bit 2 goes off the walked level and on for its parents.
    pub(super) fn clear_bit_2(&mut self) {
        self.paths[0].flags &= !4;
        for parent in &mut self.paths[1..] {
            parent.flags |= 4;
        }
    }

    /// Slot 0x560 (`FUN_1411616f0`): the unit stops ([`Mover::stop`], slot 0x5d0) and its walked
    /// level, if any, resets (`FUN_140f98e80`, [`Mover::reset_level`]).
    ///
    /// # Errors
    ///
    /// As [`Mover::stop`].
    pub(crate) fn stand(&mut self) -> Result<(), GenieError> {
        self.stop()?;
        if !self.paths.is_empty() {
            self.reset_level();
        }
        Ok(())
    }

    /// Slot 0x5d0 (`FUN_14108c2f0` -> `FUN_141161730`): the unit stops: its speed and its
    /// acceleration along its forward vector go to 0 (slots 0x598 and 0x5a8, on velocity data it
    /// has), it stops waiting to move (`+0x7f9`, slot 0x2a0) and its own ignore list (`+0x678`)
    /// empties. `+0x7b8` is cleared too ([`Mover::speed_stopped`]); a master whose move
    /// algorithm has bit 0 also resets the unit's sprite (`+0x78c`, `+0x958`), which is not
    /// modelled.
    pub(crate) fn stop(&mut self) -> Result<(), GenieError> {
        if self.stats.move_algorithm & 1 != 0 {
            return Err(GenieError::unsupported(
                "a stop that resets the sprite (move algorithm bit 0)",
            ));
        }
        if let Some(v) = self.velocity.as_mut() {
            v.velocity[2] = 0.0;
            v.acceleration[2] = 0.0;
        }
        self.waiting = 0;
        self.ignore_units.clear();
        self.speed_stopped = true;
        Ok(())
    }

    /// The unit's place changed inside the motion step: the world's places and the mesh follow
    /// (`0x1411a58b0` -> `FUN_14118c540`), and the time of the last move with them
    /// (`FUN_141161950`, `+0x870`).
    pub(super) fn moved_to(
        &mut self,
        me: &mut Me,
        at: [f32; 3],
        world: &mut Spatial,
        pathing: Option<&mut Pathing>,
        now: u32,
    ) {
        self.mark_move(at, now);
        me.position = at;
        world.set_position(me.id, at);
        if let (Some(pathing), Some(p)) = (pathing, world.get(me.id)) {
            pathing.moved(me.id, p, self.is_moving());
        }
    }

    /// `FUN_140f96050` on the walked level: up its parents (`+0x8`) to the top level, the last of
    /// [`Mover::paths`], whose path has waypoints after its current one
    /// ([`PathData::has_more`]).
    pub(super) fn top_has_more(&self) -> bool {
        self.paths.last().is_some_and(PathData::has_more)
    }

    /// Object slot 185 (`0x1411583d0`): the unit has velocity data whose velocity is longer than 0.
    #[must_use]
    pub fn is_moving(&self) -> bool {
        self.velocity.as_ref().is_some_and(|v| {
            let [x, y, z] = v.velocity;
            0.0 < fmath::sqrt(x * x + y * y + z * z)
        })
    }
}
