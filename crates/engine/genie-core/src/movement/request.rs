//! The motion step's path request: the long pather's plan, the walled-off stretches and the
//! goal's reach.

use crate::GenieError;
use crate::math::fmath::Vec3;
use crate::movement::{
    BUILDING_CLASS, CLOSE, CONTINUE, DETOUR, EPSILON, FAR, GATE, Goal, LONG, LONG_PATH_GROW,
    LONG_PATH_MOVER, LONG_PATHER, LongWalker, Me, Mover, PathData, Planning, RANGE_FLOOR,
    SHORT_PATHER, SHORT_REACH, Step, TARGET_SHORT_REACH, TILE_PATHER, TOWER_CLASS, TRADE_CART,
    VILLAGER_CLASS, WALL_CLASS, cut_path, far_waypoint, footprint_gap, ground, ground_line,
    long_path_for, may_plan, point_or_zero, short_range, too_short, vector,
};
use crate::spatial::{Placed, Spatial};
use crate::tile_path::Path;

impl Mover {
    /// The motion step's path request for the level it walks, in state 2 (`0x141150f00`, its
    /// `_Memory[10] == 2` block): unless the unit is in range of its target (state 4) or on its
    /// waypoint, the level spends one of its tries and a new level gets a path to the waypoint:
    /// the short pather's (`FUN_140f9e540`) for a near waypoint, else or when that fails the tile
    /// pathfinder's (`FUN_140f7c530`; a top level's plan becomes a level of its own, whose
    /// stretches the next pass serves; the short pather takes the walk's kept hits, `include`, as
    /// its include list). The new level goes under the walked one, which waits
    /// (state 0). A waypoint or target nearer than a tile takes the close-in path when the short
    /// pather found none ([`Mover::close_in`]); a far waypoint takes the long pather's path
    /// ([`Mover::long_plan`]). The escape from a game type 12 tile, a trade cart's budget and the
    /// other exits that stop the unit (slot 0x5d0) are not modelled.
    pub(super) fn request(
        &mut self,
        me: &Me,
        world: &Spatial,
        plan: &mut Planning<'_>,
        goal: Goal<'_>,
        plans: &mut u32,
        include: &[i32],
    ) -> Result<Step, GenieError> {
        let Goal {
            target,
            gap,
            to_end,
        } = goal;
        // the request measures and plans from +0x824 (`pfVar14`, `local_18c8`), where the unit
        // last moved to
        let position = vector(self.moved_at);
        let radius = self.stats.radius[0];
        let ground_radius = [radius, self.stats.radius[1]];
        let level = &mut self.paths[0];
        let reach = if level.range <= 0.0 {
            RANGE_FLOOR
        } else {
            level.range
        };
        if target.is_some() && gap <= reach {
            level.state = 4;
            return Ok(Step::Again);
        }
        let waypoint = level.point(level.waypoint);
        if ground(position, waypoint) == 0.0 {
            let away = target.is_none_or(|t| {
                EPSILON < footprint_gap(me.position, ground_radius, t.position, t.radius)
            });
            if away {
                level.flags |= 2;
            }
            level.flags |= 1;
            return Ok(Step::Again);
        }
        let solid = self.stats.radius.iter().all(|&r| 0.0 < r);
        let trade_cart = world.get(me.id).is_some_and(|p| p.class == TRADE_CART);
        if solid {
            // a trade cart plans only within its owner's budget (FUN_14165d6f0 on its wait
            // delays); a fourth plan in one step, or no budget: the unit stops (slot 0x5d0),
            // counts a wait delay (`+0x7fa`) and the step returns 0
            let owner = world.get(me.id).map(|p| p.player);
            let budget = owner.map(|o| plan.pathing.budget(o));
            if !may_plan(trade_cart, budget, self.wait_delays, *plans) {
                self.stop()?;
                self.wait_delays = self.wait_delays.wrapping_add(1);
                return Ok(Step::Paused);
            }
            // the owner's count of plans (player +0x67c, FUN_141663c50)
            if let Some(o) = owner {
                plan.pathing.count_plan(o);
            }
            *plans += 1;
            self.wait_delays = 0;
        }
        // the level spends a try unless its top path loops
        let top_loops = self
            .paths
            .last()
            .is_some_and(|t| !t.points.is_empty() && t.path_flags & 1 != 0);
        if !top_loops && !self.attack_moving {
            let top_has_more = self.top_has_more();
            let level = &mut self.paths[0];
            level.continue_counter -= 1;
            if level.continue_counter < 1 {
                if 1 < level.level || !top_has_more {
                    if level.level == 2 && level.waypoint < level.points.len() as i32 - 1 {
                        level.flags |= 1;
                        level.continue_counter = CONTINUE;
                        return Ok(Step::Again);
                    }
                    // FUN_140f9b4b0: the level resets (FUN_140f98e80) and takes state 6; the unit
                    // stops (slot 0x5d0)
                    self.reset_level();
                    self.paths[0].state = 6;
                    self.stop()?;
                    return Ok(Step::GaveUp);
                }
                level.state = 5;
                return Ok(Step::Again);
            }
        }
        // the unit stops waiting to move (slot 0x2a0); a new level with a new path
        self.waiting = 0;
        let mut new = PathData::new(plan.now);
        let mut path = Path::default();
        // `local_19d0`, `bVar35`, `bVar6` and `bVar4`
        let mut found = false;
        let mut partial = false;
        let mut planned = false;
        let mut failed = false;
        if solid {
            let way = ground(waypoint, position);
            // FUN_141158210 is 1 only on an `E` tile in game type 12
            if plan.pathing.game_type == 12 {
                return Err(GenieError::unsupported(
                    "a path request in game type 12 (its tile escape)",
                ));
            }
            let (level_number, range, target_id) = {
                let l = &self.paths[0];
                (l.level, l.range, l.target)
            };
            if level_number == 2 && TILE_PATHER {
                if self.planner_exists(world, plan) {
                    let mut short_found = false;
                    let mut target_moves = false;
                    let mut target_radius = 0.0;
                    if let Some(t) = target {
                        target_moves = 0.0 < t.speed;
                        if !target_moves {
                            target_radius = t.radius[0];
                        }
                        if way <= TARGET_SHORT_REACH {
                            let near = self.near_goal(way, gap, to_end);
                            let line = ground_line(self.moved_at, waypoint);
                            let ignore =
                                self.ignore_list(me, world, plan, target, target_id, line)?;
                            let r = self.short_path(
                                me,
                                world,
                                plan,
                                waypoint,
                                short_range(near, range),
                                target_id,
                                (&ignore, include),
                                &mut path,
                            )?;
                            path.measure();
                            new.created = plan.now;
                            self.give_ignored(&mut new, plan.now);
                            if r != 1 || path.length <= EPSILON {
                                failed = true;
                                path.points.clear();
                            } else {
                                found = true;
                                short_found = true;
                            }
                        }
                    }
                    if !short_found {
                        let r = self
                            .tile_plan(
                                me,
                                world,
                                plan,
                                waypoint,
                                target_radius + radius,
                                range,
                                target,
                                &mut path,
                            )?
                            .expect("the planner exists");
                        if r != 3 && r != 4 && r != 0 {
                            let level = &mut self.paths[0];
                            if !(r == 2 && (level.level != 2 || level.flags & 8 != 0)) {
                                planned = true;
                                let last = point_or_zero(&path, path.points.len() as i32 - 1);
                                let farther = way < ground(waypoint, last);
                                if farther {
                                    path.points.clear();
                                }
                                found = !farther;
                                if !target_moves && !farther && level.level == 2 && r == 2 {
                                    level.flags |= 8;
                                }
                            }
                        }
                    }
                } else {
                    // no planner: the straight way
                    let _ = path.insert(0, position);
                    let _ = path.push(waypoint);
                    planned = true;
                    found = true;
                }
                path.measure();
                new.created = plan.now;
            }
            let near = self.near_goal(way, gap, to_end);
            // `fVar41`: how far from the waypoint the short path ends
            let mut end_gap = FAR;
            let tail = if !found || path.points.len() < 3 {
                if SHORT_PATHER && !failed && (way <= SHORT_REACH || near) {
                    planned = false;
                    path.points.clear();
                    let line = ground_line(self.moved_at, waypoint);
                    let ignore = self.ignore_list(me, world, plan, target, target_id, line)?;
                    let r = self.short_path(
                        me,
                        world,
                        plan,
                        waypoint,
                        short_range(near, range),
                        target_id,
                        (&ignore, include),
                        &mut path,
                    )?;
                    path.measure();
                    new.created = plan.now;
                    self.give_ignored(&mut new, plan.now);
                    let blocked = r == 3
                        || r == 4
                        || path.length <= EPSILON
                            && (r != 0 || (1 < path.points.len() && EPSILON <= path.length));
                    if blocked {
                        // far from its end (LONG < to_end) the game tells the units ahead to
                        // make way (FUN_140f8a230, FUN_140f88750), which return at once: their
                        // switch is a zeroed byte of .data that nothing writes (0x143f01836)
                    } else if path.length <= EPSILON {
                        // in range of the waypoint: the new level is dropped
                        let reach = if target.is_none() || 0.0 < range {
                            range
                        } else {
                            RANGE_FLOOR
                        };
                        let level = &mut self.paths[0];
                        if reach < gap {
                            if level.waypoint == level.points.len() as i32 - 1
                                && level.created == plan.now
                            {
                                // on the last waypoint of a path made this update: the unit
                                // stops (slot 0x5d0), the level resets and takes state 6
                                // (FUN_140f9b4b0) and the step returns 1
                                self.stop()?;
                                self.reset_level();
                                self.paths[0].state = 6;
                                return Ok(Step::GaveUp);
                            }
                            level.flags |= 1;
                        } else {
                            level.state = 4;
                        }
                        return Ok(Step::Again);
                    } else {
                        found = true;
                        partial = r == 2;
                        let last = point_or_zero(&path, path.points.len() as i32 - 1);
                        end_gap = ground(waypoint, last);
                    }
                }
                if found {
                    too_short(&path)
                } else if (way < CLOSE || gap < CLOSE)
                    && self.close_in(me, waypoint, target_id, world)?
                {
                    // the close-in path: the straight way to the waypoint
                    path.points.clear();
                    let _ = path.push(position);
                    let _ = path.push(waypoint);
                    path.measure();
                    new.created = plan.now;
                    found = true;
                    false
                } else {
                    // no path, or a blocked close-in one (LAB_141152ec1)
                    true
                }
            } else {
                too_short(&path)
            };
            if tail && LONG_PATHER && (trade_cart || SHORT_REACH < way) {
                // LAB_141152ec1: a far waypoint takes the long pather
                let far = far_waypoint(
                    way,
                    target.map(|t| {
                        let gap = footprint_gap(me.position, ground_radius, t.position, t.radius);
                        (gap, t.master)
                    }),
                );
                if far {
                    planned = false;
                    let target = target.map(|t| (target_id, t));
                    let Some((reached, farther)) =
                        self.long_plan(me, world, plan, waypoint, way, range, target, &mut path)?
                    else {
                        return Ok(Step::Paused);
                    };
                    new.created = plan.now;
                    found = reached;
                    if let Some(end) = farther {
                        end_gap = end;
                    }
                }
            }
            // a failed or short path: the tile pathfinder once more, on a path of its own
            if (!found || too_short(&path)) && level_number == 2 && TILE_PATHER {
                let mut target_radius = 0.0;
                if let Some(t) = target.filter(|t| t.speed <= 0.0) {
                    target_radius = t.radius[0];
                }
                let mut other = Path::default();
                let r = self.tile_plan(
                    me,
                    world,
                    plan,
                    waypoint,
                    target_radius + radius,
                    range,
                    target,
                    &mut other,
                )?;
                if let Some(r) = r {
                    let last = point_or_zero(&other, other.points.len() as i32 - 1);
                    let end = ground(waypoint, last);
                    if end <= way {
                        if (r.wrapping_sub(1) < 2 || 4 < r) && end < end_gap {
                            path.points.clear();
                            for &p in &other.points {
                                let _ = path.push(p);
                            }
                            planned = true;
                            found = true;
                            path.measure();
                            new.created = plan.now;
                            if path.length < LONG {
                                path.points.clear();
                                found = false;
                            }
                        }
                    } else {
                        path.points.clear();
                    }
                }
            }
            if !(path.length <= way * DETOUR || path.length <= LONG) {
                new.repath_time = (plan.object.rand15() as f32 * 0.5) / 32767.0 + 2.0;
            }
        } else {
            let level = &self.paths[0];
            let _ = path.insert(0, position);
            let to = if level.waypoint < level.points.len() as i32 {
                level.point(level.waypoint)
            } else {
                position
            };
            let _ = path.push(to);
            path.measure();
            found = true;
        }
        if path.points.len() < 2 || path.length < EPSILON {
            return self.no_path(found, target, to_end);
        }
        Ok(self.link(new, path, found, partial, planned, solid))
    }

    /// The long pather's plan for a far waypoint (`0x141150f00` at `LAB_141152f9c`): the new path
    /// empties and the long pather writes it (`FUN_140f9eb70` from the unit's tile to the
    /// waypoint's, the level's range and target; [`crate::long_path::path`]), made now. Its tall
    /// counts leave out the unit, its ignored units that wait to move or move (slots 0x2a8 and
    /// 0x5c8; not the target) and the target (`FUN_140f60440`). A path shorter than `EPSILON`
    /// stops the unit (slot 0x5d0; the new level and its path go, `FUN_14152f540` and
    /// `FUN_140f8f7f0`) and the step returns 0: None. A path ending farther from the waypoint
    /// than the unit is (on the ground) empties, its end's gap to the waypoint returned; else
    /// its stretches are checked for walls ([`Mover::walled_off`]). Returns whether it reached
    /// the goal and that gap.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn long_plan(
        &mut self,
        me: &Me,
        world: &Spatial,
        plan: &Planning<'_>,
        waypoint: Vec3,
        way: f32,
        range: f32,
        target: Option<(i32, &Placed)>,
        path: &mut Path,
    ) -> Result<Option<(bool, Option<f32>)>, GenieError> {
        let master = world.get(me.id).map_or(-1, |p| p.master);
        let walker = LongWalker {
            id: me.id,
            position: me.position,
            radius: [self.stats.radius[0], self.stats.radius[1]],
            mask: self.stats.restriction_mask,
            moves: self.stats.speed != 0.0 || master == LONG_PATH_MOVER,
            ignore_units: &self.ignore_units,
            group_units: plan.groups.group_units(me.group),
        };
        let target_id = target.map_or(-1, |(id, _)| id);
        // from +0x824, the motion step's `local_18c8`
        let (mut found, long) = long_path_for(
            &walker,
            [self.moved_at[0], self.moved_at[1]],
            world,
            plan.computer,
            (waypoint, range),
            target,
        )?;
        *path = long;
        path.measure();
        if path.length < EPSILON {
            self.stop()?;
            return Ok(None);
        }
        let end = ground(waypoint, point_or_zero(path, path.points.len() as i32 - 1));
        if way < end {
            path.points.clear();
            path.length = 0.0;
            return Ok(Some((found, Some(end))));
        }
        if !self.walled_off(me, world, target_id, path)? {
            found = false;
        }
        Ok(Some((found, None)))
    }

    /// The long path's stretches (`0x141150f00` after `FUN_140f9eb70`, for a unit with a
    /// footprint and a height, `FUN_141194060` with 1): each stretch's obstruction query
    /// (`FUN_14114cc60` as the walk's, grown by the radius times `LONG_PATH_GROW`; the unit and
    /// the target ignored with their annexes, `FUN_14114e8a0`) that meets a wall, a gate, a tower
    /// or a building that does not move (classes 27, 39, 52, and 3 at speed 0, slot 0x208) ends
    /// the path before that stretch (`FUN_140f98dd0`), or, on the first one, empties it (its
    /// length kept) and false; a villager's path is never cut.
    pub(super) fn walled_off(
        &self,
        me: &Me,
        world: &Spatial,
        target_id: i32,
        path: &mut Path,
    ) -> Result<bool, GenieError> {
        let me_at = world.get(me.id);
        if me_at.is_some_and(|p| p.class == VILLAGER_CLASS) {
            return Ok(true);
        }
        let player = me_at.map(|p| p.player);
        let ignore = world.query_ignore(&[me.id, target_id]);
        let grow = self.stats.radius[0] * LONG_PATH_GROW;
        for i in 0..path.points.len().saturating_sub(1) {
            let (from, to) = self.query_ends(path.points[i], path.points[i + 1], world.map);
            let rect = [
                from[0].min(to[0]),
                from[1].min(to[1]),
                from[0].max(to[0]),
                from[1].max(to[1]),
            ];
            world.check_known_records([
                rect[0] - grow,
                rect[1] - grow,
                rect[2] + grow,
                rect[3] + grow,
            ])?;
            let hits = world.walk_hits(
                from,
                to,
                rect,
                grow,
                self.stats.min_collision,
                player,
                &ignore,
            )?;
            let walled = hits.iter().any(|&h| {
                world.get(h).is_some_and(|p| {
                    matches!(p.class, WALL_CLASS | GATE | TOWER_CLASS)
                        || (p.class == BUILDING_CLASS && p.speed <= 0.0)
                })
            });
            if walled {
                if i == 0 {
                    path.points.clear();
                    return Ok(false);
                }
                // i + 1 lies inside the path: the cut always happens
                let _ = cut_path(path, i as i32 + 1);
                return Ok(true);
            }
        }
        Ok(true)
    }

    /// Whether the unit's restriction has a planner (`FUN_140f7cb30`); the slice builds one for
    /// any restriction its data has.
    pub(super) fn planner_exists(&self, world: &Spatial, plan: &mut Planning<'_>) -> bool {
        plan.pathing
            .planner(self.stats.restriction, world.tiles())
            .is_some()
    }

    /// `FUN_14114d3d0`: the walked level's goal is in reach: its target's gap (else where its top
    /// path ends) within its range, or a target's last waypoint at most 16 tiles beyond its
    /// range on a top level.
    pub(super) fn near_goal(&self, way: f32, gap: f32, to_end: f32) -> bool {
        let level = &self.paths[0];
        let d = if level.target == -1 { to_end } else { gap };
        if d - level.range < EPSILON {
            return true;
        }
        level.target != -1
            && level.points.len() as i32 == level.waypoint + 1
            && way - level.range <= SHORT_REACH
            && level.level == 2
    }
}
