//! The velocity step, a new object's setup, the position setter and the motion step's loop.

use crate::math::fmath::Vec3;
use crate::movement::{
    ANNEX_GAP, EDGE_GAP, EDGE_POSITION_GAP, EPSILON, FAR, Goal, Me, Motion, MoveStats, Mover, Next,
    Planning, RANGE_FLOOR, Reach, Step, footprint_gap, gate_gap, ground, include_kept,
    map_position, moved_far, pause_holds, point_or_zero_of, reach, vector,
};
use crate::pathing::Pathing;
use crate::spatial::{Placed, Spatial};
use crate::{GenieError, math::fmath};

impl Mover {
    /// The velocity part of `RGE_Moving_Object::update` (`FUN_140f9ba80`): the forward speed moves
    /// toward the acceleration's target, capped by the top speed (object slot 188, `0x14114f300`),
    /// then the unit moves `dt × speed` along its forward vector (slot 182, `0x141158490`). Returns
    /// the position after it.
    pub fn step_velocity(
        &mut self,
        position: [f32; 3],
        dt: f32,
        map: [i32; 2],
        terrain: Option<&crate::terrain::Terrain>,
        now: u32,
    ) -> [f32; 3] {
        let Some(v) = self.velocity.as_mut() else {
            return position;
        };
        let wanted = v.acceleration[2] * dt + v.velocity[2];
        let speed = if self.stats.speed < wanted {
            self.stats.speed
        } else if wanted < 0.0 {
            0.0
        } else {
            wanted
        };
        v.velocity[2] = speed;
        // the position setter runs only for a distance it moves
        let Some(at) = self.move_forward(dt * speed, map) else {
            return position;
        };
        let at = self.grounded(at, terrain);
        self.mark_move(at, now);
        at
    }

    /// The static `new_position`'s end (`0x1411a58b0`, slot 0x60: `FUN_141158000` ->
    /// `FUN_141193a10`): the place takes the ground's height ([`crate::terrain::Terrain::ground`])
    /// by the master's fly mode: 0 puts it on the ground, 1 lifts it to the ground when below,
    /// others keep it. Without the map's elevations the height stays; `+0x7f8` (the place is on
    /// the ground) is not held.
    #[must_use]
    pub fn grounded(
        &self,
        [x, y, z]: [f32; 3],
        terrain: Option<&crate::terrain::Terrain>,
    ) -> [f32; 3] {
        let z = terrain
            .and_then(|t| t.ground(x, y).ok())
            .map_or(z, |ground| {
                crate::missile::grounded(z, ground, false, self.stats.fly_mode)
            });
        [x, y, z]
    }

    /// The moving setup (`FUN_1411580a0`) of a new object of `stats` at `at`: it faces x (its
    /// right −z), does not wait, has no path and moveForward's factor 1; the position setter
    /// (`FUN_141161950`) makes its place a move at `now`. Its angle and its sprite's facets are
    /// the caller's. Every moving object (types 30 and up) gets it, a building too, though it
    /// never moves: the walk reads the last move time of a building it meets (slot 0x5e8,
    /// `FUN_14106d2d0`).
    pub(crate) const fn new_object(
        stats: MoveStats,
        at: [f32; 3],
        now: u32,
        angle: f32,
        sprite_facets: Option<crate::facing::SpriteFacets>,
    ) -> Self {
        Self {
            forward: [1.0, 0.0, 0.0],
            right: [0.0, 0.0, crate::spawn::START_RIGHT_Z],
            velocity: None,
            paths: Vec::new(),
            factor: 1.0,
            stats,
            waiting: 0,
            wait_delays: 0,
            attack_moving: false,
            last_move: now,
            moved_at: at,
            substitute_count: 0,
            ignore_units: Vec::new(),
            user_waypoints: Vec::new(),
            last_turn: 0.0,
            angle,
            move_to_point: None,
            speed_stopped: false,
            pauses: Vec::new(),
            in_the_way: Vec::new(),
            sprite_facets,
        }
    }

    /// The position setter's end (`FUN_141161950`): a place `at` more than 1e-7 off where it
    /// last moved to on any axis is a move at `now` (`+0x870`), and where it last moved to.
    pub(crate) fn mark_move(&mut self, at: [f32; 3], now: u32) {
        if moved_far(self.moved_at, at) {
            self.last_move = now;
            self.moved_at = at;
        }
    }

    /// `moveForward` (slot 182, `FUN_141158490`): `distance × factor` along the forward vector
    /// from where the unit last moved to (`+0x824`, [`Mover::moved_at`]), through the position
    /// setter; a distance below 1e-7 moves nothing.
    pub(super) fn move_forward(&self, distance: f32, map: [i32; 2]) -> Option<[f32; 3]> {
        let d = distance * self.factor;
        if d.abs() < EPSILON {
            return None;
        }
        let to = fmath::add(vector(self.moved_at), fmath::scale(d, self.forward));
        Some(self.set_position(map_position(to), map))
    }

    /// The position setter (slot 11 -> `FUN_141161950` -> `0x1411a58b0`): a unit that can move
    /// keeps its footprint 0.01 inside the map, then any position stays 0.001 below the map's size.
    pub(crate) fn set_position(&self, [x, y, z]: [f32; 3], map: [i32; 2]) -> [f32; 3] {
        let inside = |v: f32, radius: f32, size: i32| {
            let low = radius + EDGE_GAP;
            if v < low {
                low
            } else if size as f32 - low < v {
                (size as f32 - radius) - EDGE_GAP
            } else {
                v
            }
        };
        // both clamps read the map's +0x18, its width (0x14116220d for y): on a map that is not
        // square the y clamp takes the width
        let (x, y) = if EPSILON < self.stats.speed {
            (
                inside(x, self.stats.radius[0], map[0]),
                inside(y, self.stats.radius[1], map[0]),
            )
        } else {
            (x, y)
        };
        let on_map = |v: f32, size: i32| {
            let v = v.max(0.0);
            if size as f32 <= v {
                size as f32 - EDGE_POSITION_GAP
            } else {
                v
            }
        };
        [on_map(x, map[0]), on_map(y, map[1]), z]
    }

    /// The motion step (object slot 193 -> `0x141150f00`, `doIntelligentMovementUpdate`) along the
    /// path a walking unit follows, as the game's loop: each pass looks at the level the unit
    /// walks. A level whose waypoint was reached moves on to the next one ([`Mover::reached`]), an
    /// arrived level hands over to its parent or ends the walk ([`Mover::arrived`]), a level that
    /// wants a path gets one ([`Mover::request`]); a walking level steers the unit toward its
    /// waypoint ([`Mover::walk`]). A pass that changes the levels starts the step again. Unit
    /// groups and obstruction avoidance are not modelled. `plan` is None where the caller has no
    /// planners: a path request then stops the slice. The unit's place may change (it takes a
    /// reached waypoint's), in `me` and `world`.
    pub fn motion(
        &mut self,
        me: &mut Me,
        dt: f32,
        world: &mut Spatial,
        mut plan: Option<&mut Planning<'_>>,
    ) -> Result<Motion, GenieError> {
        let result = self.motion_passes(me, dt, world, plan.as_deref_mut());
        // every exit of the step removes the kept hits' temporary shapes (FUN_141055970)
        if let Some(p) = plan {
            p.pathing.drop_temporary_shapes();
        }
        result
    }

    /// [`Mover::motion_passes`]'s measure of the level walked toward `target_id`: the target's
    /// place (`FUN_1415417a0`: none for -1 and for an object that is gone, which then measures as
    /// a walk to a point, `LAB_1411512df`), the gap to it, and the gap to it, else to where the
    /// top level's path ends.
    ///
    /// # Errors
    ///
    /// A target whose place the slice does not know.
    pub(super) fn level_gaps(
        &self,
        me: &Me,
        world: &Spatial,
        target_id: i32,
        ground_radius: [f32; 2],
    ) -> Result<(Option<Placed>, f32, f32), GenieError> {
        // FUN_1415417a0 on the level's target: none for -1 and for an object that is gone,
        // which then measures as a walk to a point (LAB_1411512df)
        let target = match target_id {
            -1 => None,
            id => world.get_known(id)?.copied(),
        };
        // the gap to the target, else to where the top level's path ends; a gate's is the
        // least over it and its first two annexes (slot 0x680), each annex's less 0.1
        // (`MINSS`, 0x1411512b8)
        let gap = target.as_ref().map_or(FAR, |t| {
            let own = footprint_gap(me.position, ground_radius, t.position, t.radius);
            let near = (me.position, ground_radius);
            gate_gap(world, near, (target_id, t), own, |g| g - ANNEX_GAP)
        });
        let to_end = match (&target, self.paths.len()) {
            (Some(_), _) => gap,
            (None, 1) => FAR,
            (None, _) => self
                .top_path()
                .and_then(|p| p.last())
                .map_or(FAR, |&[x, _, y]| {
                    footprint_gap(me.position, ground_radius, [x, y, 0.0], [0.0, 0.0])
                }),
        };
        Ok((target, gap, to_end))
    }

    /// [`Mover::motion_passes`]'s dispatch on the level's state toward `goal`: an arrived level (4,
    /// 5) hands over ([`Mover::arrived`]), a level that wants a path (2) gets one
    /// ([`Mover::request`]: again, on to the walk, a pause that keeps the unit moving, or a
    /// request given up); any other goes on to the walk.
    ///
    /// # Errors
    ///
    /// A path request without the world's planners, and what the hand-over and the request do not
    /// model.
    pub(super) fn level_state(
        &mut self,
        me: &Me,
        world: &Spatial,
        plan: Option<&mut Planning<'_>>,
        goal: Goal<'_>,
        plans: &mut u32,
        include: &[i32],
    ) -> Result<Next, GenieError> {
        match self.paths[0].state {
            4 | 5 => self.arrived(goal),
            2 => {
                let plan = plan.ok_or(GenieError::unsupported(
                    "a path request without the world's planners",
                ))?;
                Ok(match self.request(me, world, plan, goal, plans, include)? {
                    Step::Again => Next::Again,
                    Step::Walk => Next::Dispatch,
                    Step::Paused => Next::Done(Motion::Moving),
                    Step::GaveUp => Next::Done(Motion::Done),
                })
            }
            _ => Ok(Next::Dispatch),
        }
    }

    /// [`Mover::motion`]'s passes.
    pub(super) fn motion_passes(
        &mut self,
        me: &mut Me,
        dt: f32,
        world: &mut Spatial,
        mut plan: Option<&mut Planning<'_>>,
    ) -> Result<Motion, GenieError> {
        if self.stats.speed == 0.0 {
            return Ok(Motion::Done);
        }
        let Some(level) = self.paths.first_mut() else {
            return Ok(Motion::Done);
        };
        // a paused walk stands (slot 0x5d0) until its pause ran out
        if pause_holds(level, dt) {
            self.stop()?;
            return Ok(Motion::Moving);
        }
        let ground_radius = [self.stats.radius[0], self.stats.radius[1]];
        let mut plans = 0;
        // the walk's kept hits, which the step's short paths include (`DAT_143f4a000`, its count
        // `local_19bc`, 0 at the step's start)
        let mut include = Vec::new();
        // the waypoint the step's walk last went for (`local_1860`), zero before one
        let mut walked = [0.0; 3];
        // the direction the step's walk last took (slot 0x610 into `[RBP+0x148]`), zero before
        // one: a loop's end turns toward it
        let mut heading = [0.0; 3];
        loop {
            // a level without a path ends the step (`LAB_14115582c`), as no level does
            let Some(level) = self.paths.first().filter(|l| l.has_path) else {
                return Ok(Motion::Done);
            };
            let (target, gap, to_end) = self.level_gaps(me, world, level.target, ground_radius)?;
            let goal = Goal {
                target: target.as_ref(),
                gap,
                to_end,
            };
            if self.paths[0].flags & 1 != 0 {
                let now = plan.as_deref().map_or(0, |p| p.now);
                let pathing = plan.as_deref_mut().map(|p| &mut *p.pathing);
                match self.reached(me, world, pathing, goal, now)? {
                    Next::Again => continue,
                    Next::Dispatch => {}
                    Next::Done(m) => return Ok(m),
                    Next::LoopEnd => return self.loop_end(dt, walked, heading),
                }
            }
            match self.level_state(me, world, plan.as_deref_mut(), goal, &mut plans, &include)? {
                Next::Done(m) => return Ok(m),
                Next::Again => continue,
                // a link of another type: on past the request (state 4 or 5) to the walk
                Next::Dispatch => {}
                Next::LoopEnd => unreachable!("only a reached waypoint ends a loop's step"),
            }
            let (now, computer) = plan.as_deref().map_or((0, false), |p| (p.now, p.computer));
            let groups = plan.as_deref().map(|p| p.groups);
            let pathing = plan.as_deref_mut().map(|p| &mut *p.pathing);
            let step = self.walk(
                me,
                dt,
                (world, groups),
                pathing,
                goal,
                (now, computer),
                &mut include,
                (&mut walked, &mut heading),
            )?;
            if let Next::Done(m) = step {
                return Ok(m);
            }
        }
    }

    /// The walk along the level's path (`0x141150f00` from `LAB_141153bd1`): toward the waypoint
    /// in a straight line, turning to face it. On the waypoint, or when the step would lead away
    /// from it, the waypoint counts as reached and the motion step starts again; a step that
    /// reaches it marks it reached and the walk goes on. A walk to a target ends in range of it
    /// (the waypoint reached); a walk to a point with a range ends where its top path ends (the
    /// unit stops, the step returns 2). A walk that may meet an obstruction is not modelled.
    /// `walked` takes the waypoint once it is not left behind (`local_1860`). `groups` are the
    /// world's unit groups (none for a walk without the world's planners).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn walk(
        &mut self,
        me: &Me,
        dt: f32,
        (world, groups): (&Spatial, Option<&crate::group::UnitGroups>),
        pathing: Option<&mut Pathing>,
        goal: Goal<'_>,
        (now, computer): (u32, bool),
        include: &mut Vec<i32>,
        (walked, heading): (&mut Vec3, &mut Vec3),
    ) -> Result<Next, GenieError> {
        let ground_radius = [self.stats.radius[0], self.stats.radius[1]];
        // LAB_141153bd1: the waypoint's distance, the direction (slot 0x610), the step's end
        // (slot 0x638) and its query (FUN_14114cc60) start at +0x824, where the unit last moved to
        let position = vector(self.moved_at);
        let level = &self.paths[0];
        let waypoint = point_or_zero_of(level, level.waypoint);
        if ground(position, waypoint) < EPSILON {
            self.paths[0].flags |= 3;
            return Ok(Next::Again);
        }
        let direction = self.direction_to(position, waypoint);
        *heading = direction;
        let projected = self.projected(direction, dt);
        match reach(waypoint, position, projected) {
            Reach::Away => {
                self.paths[0].flags |= 3;
                return Ok(Next::Again);
            }
            Reach::Reached => self.paths[0].flags |= 3,
            Reach::Short if 2 < self.substitute_count => self.paths[0].flags |= 3,
            Reach::Short => {}
        }
        *walked = waypoint;
        if self.stats.radius.iter().all(|&r| 0.0 < r) {
            // FUN_14114e8a0 ignores the unit itself (and a building's annexes), then
            // FUN_14114cc60 asks for the step's hits, grown by the unit's radius times 1
            let (from, to) = self.query_ends(position, projected, world.map);
            let rect = [
                from[0].min(to[0]),
                from[1].min(to[1]),
                from[0].max(to[0]),
                from[1].max(to[1]),
            ];
            let grow = self.stats.radius[0];
            world.check_known_records([
                rect[0] - grow,
                rect[1] - grow,
                rect[2] + grow,
                rect[3] + grow,
            ])?;
            let player = world.get(me.id).map(|p| p.player);
            let hits = world.walk_hits(
                from,
                to,
                rect,
                grow,
                self.stats.min_collision,
                player,
                &[me.id],
            )?;
            if !hits.is_empty() {
                let avoided =
                    self.avoid(me, (world, groups), &hits, projected, dt, (now, computer))?;
                if !avoided.kept.is_empty() {
                    include_kept(world, pathing, &avoided.kept, include)?;
                }
                if avoided.at_target {
                    // FUN_140f9b4d0 three times: bit 0 on and bit 1 off on the walked level, bit 2
                    // on for it and its parents; then back to the top of the step
                    let level = &mut self.paths[0];
                    level.flags = (level.flags | 1) & !2;
                    for l in &mut self.paths {
                        l.flags |= 4;
                    }
                    return Ok(Next::Again);
                }
                self.pauses.extend(avoided.pauses);
                if !avoided.kept.is_empty() {
                    return self.go_round(me, world, &hits, computer);
                }
            }
        }
        let level = &self.paths[0];
        match goal.target {
            Some(t) => {
                let range = level.range;
                let own = footprint_gap(me.position, ground_radius, t.position, t.radius) - range;
                let near = (me.position, ground_radius);
                let gap = gate_gap(world, near, (level.target, t), own, |g| g - range);
                if gap < EPSILON {
                    // in range of the target: the waypoint counts as reached
                    let level = &mut self.paths[0];
                    level.flags &= !2;
                    level.flags |= 1;
                    return Ok(Next::Again);
                }
            }
            // a walk to a point with a range ends where its top level's path ends
            None if level.path_flags & 1 == 0 && RANGE_FLOOR < level.range => {
                if let Some(&[x, _, y]) = self.top_path().and_then(|p| p.last()) {
                    let gap = footprint_gap(me.position, ground_radius, [x, y, 0.0], [0.0, 0.0]);
                    if gap - level.range < EPSILON {
                        self.reset_level();
                        self.stop()?;
                        return Ok(Next::Done(Motion::Arrived));
                    }
                }
            }
            None => {}
        }
        let clipped = self.steer(direction, dt)?;
        self.control_speed(waypoint, clipped);
        Ok(Next::Done(Motion::Moving))
    }

    /// The motion step's end on a loop back at its first waypoint (`LAB_141155637`): the unit
    /// turns from its forward vector on the ground (`FUN_141194060` with 3 holds, which zeroes
    /// the height; `FUN_141027d00` divides by the length) toward `heading`, the direction the
    /// step's walk last took (`[RBP+0x148]`, zero when no walk ran in this step: the angle to it
    /// is then π/2, [`fmath::angle_between`]) ([`Mover::steer`]), controls its speed toward
    /// `walked` (slot 0x630, [`Mover::control_speed`]) and, its level waiting for a path (state
    /// 2), stops (slot 0x5d0).
    pub(super) fn loop_end(
        &mut self,
        dt: f32,
        walked: Vec3,
        heading: Vec3,
    ) -> Result<Motion, GenieError> {
        let clipped = self.steer(heading, dt)?;
        self.control_speed(walked, clipped);
        if self.paths.first().is_some_and(|l| l.state == 2) {
            self.stop()?;
        }
        Ok(Motion::Moving)
    }
}
