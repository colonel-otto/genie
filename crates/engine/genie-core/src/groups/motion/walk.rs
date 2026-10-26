//! The walk toward the walked level's waypoint, and `BMotion`'s update with what it reads of
//! the world.

use crate::groups::Motion;
use crate::groups::motion::{
    EPSILON, GroupView, Lead, NEAR, PULL_DISTANCE, PULL_KEEP, PULL_SHARE, Pass, World, ground,
    hold, passes_waypoint, path_step, point, reset_level, set_flag,
};
use crate::map::spatial::Placed;
use crate::math::fmath::Vec3;
use crate::movement::{Planning, footprint_gap};
use crate::turn::Turns;
use crate::{GenieError, math::fmath};

/// The walk of the walked level (`LAB_14104b6b4`): see [`path_step`].
pub(super) fn steer(
    motion: &mut Motion,
    group: &GroupView,
    lead: &Lead,
    world: &World<'_, '_>,
    target: Option<&Placed>,
) -> Result<Pass, GenieError> {
    let spatial = world.spatial;
    let dt = world.dt;
    let level = &motion.paths[0];
    let waypoint = point(level, level.waypoint);
    let at = motion.position[0];
    if ground(at, waypoint) < EPSILON {
        set_flag(&mut motion.paths, 1, true);
        return Ok(Pass::Reached);
    }
    // FUN_141046080: the direction on the ground
    let mut direction = fmath::sub(waypoint, at);
    direction[1] = 0.0;
    let l = fmath::sqrt(direction[0] * direction[0] + 0.0 + direction[2] * direction[2]);
    direction = [direction[0] / l, direction[1] / l, direction[2] / l];
    let step = (dt * motion.acceleration[2] + motion.velocity[2]) * dt;
    let next = fmath::add(at, fmath::scale(step, direction));
    match passes_waypoint(waypoint, at, next) {
        2 => {
            set_flag(&mut motion.paths, 1, true);
            return Ok(Pass::Reached);
        }
        1 => {
            set_flag(&mut motion.paths, 1, true);
            set_flag(&mut motion.paths, 0, true);
        }
        _ => {}
    }
    let lead_group = lead.group.as_ref();
    let ignore = match lead_group {
        Some(g) if !g.differentiator_units.is_empty() => g.differentiator_units.clone(),
        _ => vec![lead.walker.id],
    };
    // FUN_1411570c0 sets the manager's ignore list to it (FUN_1410562e0)
    let hits = spatial.line_query(
        hold(at, spatial.map),
        hold(next, spatial.map),
        lead.walker.radius,
        None,
        &ignore,
    )?;
    if !hits.is_empty() {
        let level = &motion.paths[0];
        let top_ignore = motion
            .paths
            .last()
            .map(|t| t.ignore.clone())
            .unwrap_or_default();
        let mut on_target = false;
        let mut newer = 0;
        for &hit in &hits {
            if spatial.get(hit).is_none() {
                continue;
            }
            if level.ignore.contains(&hit) || top_ignore.contains(&hit) {
                continue;
            }
            if 0 <= level.target && hit == level.target {
                on_target = true;
            }
            let last_move = spatial.gait(hit).map_or(0, |g| g.last_move);
            if level.created < last_move {
                newer += 1;
            }
        }
        if hits.len() == 1 && on_target {
            set_flag(&mut motion.paths, 0, true);
            set_flag(&mut motion.paths, 2, true);
            return Ok(Pass::Next);
        }
        if 0 < newer {
            // back to the parent, and past one whose waypoint the lead cannot stand on
            for _ in 0..2 {
                if motion.paths.len() < 2 {
                    reset_level(&mut motion.paths);
                    motion.stand();
                    return Ok(Pass::Done(true));
                }
                motion.paths[1].state = 2;
                let _ = motion.paths.remove(0);
                let parent = &motion.paths[0];
                if !parent.has_path {
                    break;
                }
                let p = point(parent, parent.waypoint);
                if spatial.standable(p[0] as i32, p[2] as i32, lead.walker.mask) {
                    break;
                }
            }
            return Ok(Pass::Next);
        }
    }
    // LAB_14104bcb6: in range of where the walk ends, it ends
    let level = &motion.paths[0];
    match target {
        None => {
            let top = motion.paths.last().filter(|t| t.has_path);
            if level.path_flags & 1 == 0
                && NEAR < level.range
                && let Some(top) = top
            {
                let end = point(top, top.points.len() as i32 - 1);
                let d = fmath::sub(at, end);
                let gap = fmath::sqrt(d[1] * d[1] + d[0] * d[0] + d[2] * d[2]);
                if gap - level.range < EPSILON {
                    reset_level(&mut motion.paths);
                    motion.stand();
                    return Ok(Pass::Done(true));
                }
            }
        }
        Some(t) => {
            let gap = footprint_gap(t.position, t.radius, [at[0], at[2], 0.0], [0.0, 0.0]);
            if (gap - level.range).abs() < EPSILON {
                return Ok(Pass::Reached);
            }
        }
    }
    // the turn toward the waypoint, from the forward on the ground
    let mut forward = motion.position[1];
    forward[1] = 0.0;
    let f = fmath::sqrt(forward[0] * forward[0] + 0.0 + forward[2] * forward[2]);
    let forward = [forward[0] / f, forward[1] / f, forward[2] / f];
    let yaw = motion.calculate_yaw_angle(forward, direction, dt, true);
    motion.turn(yaw);
    // the members' centre pulls the group while every member acts on a target
    let all_acting = group
        .members
        .iter()
        .all(|m| m.is_some_and(|(target, _)| target != -1));
    if all_acting {
        let n = group.members.len();
        let mut centre = [0.0f32; 3];
        if n != 0 {
            for &(_, p) in group.members.iter().flatten() {
                centre = [centre[0] + p[0], p[1] + centre[1], p[2] + centre[2]];
            }
            let n = n as f32;
            centre = [centre[0] / n, centre[1] / n, centre[2] / n];
        }
        let c = [centre[0], 0.0, centre[2]];
        let here = [motion.position[0][0], 0.0, motion.position[0][2]];
        let d = fmath::sub(here, c);
        if PULL_DISTANCE <= d[0] * d[0] + d[1] * d[1] + d[2] * d[2] {
            motion.new_position[0] = fmath::add(
                fmath::scale(PULL_SHARE, c),
                fmath::scale(PULL_KEEP, motion.new_position[0]),
            );
        }
    }
    motion.velocity[2] = motion.maximum_velocity_forward;
    Ok(Pass::Done(true))
}

impl Turns {
    /// `BMotion`'s update (`FUN_14104a090`) of group `group`, `budget` its `param_2`: the path
    /// step for the group's lead ([`path_step`]), then the velocity along the vectors
    /// ([`Motion::integrate`]). The group update calls it only with a lead the world holds, a
    /// moving unit; without one, or for a group the world does not hold, nothing changes.
    ///
    /// # Errors
    ///
    /// What the path step does not model.
    ///
    /// # Panics
    ///
    /// When the world does not hold `group`: the game's routine gets the group itself, from a
    /// caller that holds it.
    pub fn motion_update(&mut self, group: i32, budget: bool) -> Result<(), GenieError> {
        let Some(g) = self.groups.get(group) else {
            return Ok(());
        };
        let Some(lead) = self.lead_view(g.motion.lead) else {
            return Ok(());
        };
        let view = GroupView {
            computer: self
                .players
                .get(g.owner as usize)
                .is_some_and(|p| p.computer),
            members: g
                .members
                .iter()
                .map(|m| {
                    self.object(m.id)
                        .map(|(_, u)| (self.top_target_object(u), moved_vector(u)))
                })
                .collect(),
        };
        let g = self.groups.get_mut(group).expect("the group found above");
        let mut motion = std::mem::take(&mut g.motion);
        let result = {
            let mut plan = Planning {
                pathing: &mut self.pathing,
                now: self.world_time,
                computer: lead.computer,
                object: self.streams.get(crate::math::rng::Stream::Object),
                construction_rate: 0.0,
                groups: &self.groups,
            };
            let mut world = World {
                spatial: &self.spatial,
                plan: &mut plan,
                dt: self.dt_seconds,
            };
            path_step(&mut motion, &view, &lead, &mut world, budget)
        };
        if result.is_ok() {
            motion.integrate(self.dt_seconds);
        }
        self.groups
            .get_mut(group)
            .expect("the group found above")
            .motion = motion;
        result.map(|_| ())
    }
}

/// The ignore list the short pather and the walk's query take of the lead's group, at most this
/// many (`FUN_14100bbe0` with `0x80`; the request's own loop stops past `0x7f`).
pub(super) const SHORT_IGNORED: usize = 128;

/// Slot 0x520 of a unit: where it last moved to, as the game's vector (x, height, y).
pub(in crate::groups) fn moved_vector(u: &crate::turn::Unit) -> Vec3 {
    u.mover
        .as_ref()
        .map_or([u.position[0], u.position[2], u.position[1]], |m| {
            [m.moved_at[0], m.moved_at[2], m.moved_at[1]]
        })
}
