//! The walked level's path request (state 2) and its end: the planners, the link of the
//! found path, the give-up.

use crate::GenieError;
use crate::groups::Motion;
use crate::groups::motion::{
    CLOSE, CONTINUE, EPSILON, LAST_TRIES, Lead, LeadGroup, NEAR, NEW_PATH_FLAGS, PATROL_PAUSE,
    Pass, SHORT_LENGTH, SHORT_REACH, STUCK_PAUSE, World, ground, hold, point, reset_level,
};
use crate::map::spatial::Placed;
use crate::movement::{LongWalker, PathData, long_path_for, short_path_for, tile_plan_for};
use crate::pathing::tile_path::Path;

/// Where the request (`FUN_14104a270`'s state 2) goes on after a planner.
pub(super) enum Stage {
    /// `LAB_14104b2ae`: a found path long enough goes to the link.
    Found,
    /// `LAB_14104b2cd`: the long pather.
    Long,
}

/// The path request of the walked level in state 2 (`0x14104a80b` on): see [`path_step`].
///
/// [`path_step`]: crate::groups::motion::step::path_step
#[allow(clippy::too_many_lines)]
pub(super) fn request(
    motion: &mut Motion,
    lead: &Lead,
    world: &mut World<'_, '_>,
    budget: bool,
    target: Option<&Placed>,
    target_gap2: f32,
) -> Result<Option<Pass>, GenieError> {
    let spatial = world.spatial;
    let now = world.plan.now;
    if budget
        && !world
            .plan
            .pathing
            .budget(lead.player)
            .allows(motion.number_wait_delays)
    {
        motion.number_wait_delays += 1;
        motion.stand();
        return Ok(Some(Pass::Done(false)));
    }
    world.plan.pathing.count_plan(lead.player);
    motion.number_wait_delays = 0;
    let top_loops = motion
        .paths
        .last()
        .filter(|t| t.has_path)
        .map(|t| t.path_flags & 1 != 0);
    let lead_group = lead.group.as_ref();
    let spends = top_loops.is_none_or(|loops| !loops && lead_group.is_some_and(|g| !g.patrols()));
    if spends {
        let level = &mut motion.paths[0];
        level.continue_counter -= 1;
        if level.continue_counter <= 0 {
            give_up(motion);
            return Ok(Some(Pass::Done(false)));
        }
    }
    let level = &motion.paths[0];
    let (level_number, range, target_id) = (level.level, level.range, level.target);
    let waypoint = point(level, level.waypoint);
    let at = motion.position[0];
    let from = [at[0], at[2]];
    let way = ground(waypoint, at);
    let mut path = Path::default();
    let mut found = false;
    let mut planned = false;
    let mut has_plan = false;
    let held_target = target.map(|_| target_id);
    // the tile pathfinder for a top level (DAT_1439e3cc4)
    if level_number == 2 {
        let radii = target.map_or(0.0, |t| t.radius[0]) + lead.walker.radius;
        let mut plan = world.plan.reborrow();
        plan.computer = lead.computer;
        let r = tile_plan_for(
            lead.walker,
            from,
            held_target,
            spatial,
            &mut plan,
            (waypoint, radii, range),
            target,
            &mut path,
        )?;
        match r {
            None => {
                // no planner for the lead's restriction: the straight way
                let _ = path.insert(0, at);
                let _ = path.push(waypoint);
                planned = true;
                has_plan = true;
                found = true;
            }
            Some(r) if (1..=2).contains(&r) || 4 < r => {
                planned = true;
                found = true;
                has_plan = true;
                let end = path.points.last().copied().unwrap_or([0.0; 3]);
                if way < ground(waypoint, end) {
                    path.points.clear();
                    found = false;
                    has_plan = false;
                }
            }
            Some(_) => {}
        }
        path.measure();
    }
    let (mut short_range, mut beyond) = (0.0, way);
    let level = &motion.paths[0];
    if level_number == 2 && level.points.len() as i32 == level.waypoint + 1 {
        short_range = range;
        beyond = way - range;
    }
    let mut stage = if !has_plan || path.points.len() < 3 {
        if way <= SHORT_REACH {
            // the short pather (DAT_1439e3cc0)
            planned = false;
            path.points.clear();
            let ignore =
                lead_group.map_or_else(|| vec![lead.walker.id], |g| g.differentiator_units.clone());
            let mut r = 0;
            if lead.solid {
                let mut plan = world.plan.reborrow();
                plan.computer = lead.computer;
                r = short_path_for(
                    lead.walker,
                    from,
                    spatial,
                    &mut plan,
                    (waypoint, short_range, target_id),
                    &ignore,
                    &mut path,
                )?;
            }
            path.measure();
            has_plan = found;
            if r != 3 && r != 4 {
                if EPSILON < path.length && 1 < path.points.len() {
                    found = true;
                    has_plan = true;
                } else if r == 0 && (path.points.len() < 2 || path.length < EPSILON) {
                    // in range of the waypoint: the new level goes
                    let reach = if target.is_none() || 0.0 < range {
                        range
                    } else {
                        NEAR
                    };
                    if reach < target_gap2 {
                        let level = &motion.paths[0];
                        if level.waypoint == level.points.len() as i32 - 1 && level.created == now {
                            motion.stand();
                            // FUN_140f9b4b0: the level resets and takes state 6
                            reset_level(&mut motion.paths);
                            motion.paths[0].state = 6;
                            return Ok(Some(Pass::Done(true)));
                        }
                        return Ok(Some(Pass::Reached));
                    }
                    motion.paths[0].state = 4;
                    return Ok(Some(Pass::Next));
                }
            }
        }
        if has_plan {
            Stage::Found
        } else {
            if beyond < CLOSE || target_gap2 < CLOSE {
                // the straight way when nothing stands on it (FUN_1411570c0 with its tiles)
                let ignore = lead_group.map_or_else(|| vec![lead.walker.id], |g| g.members.clone());
                let hits = spatial.line_query(
                    hold(at, spatial.map),
                    hold(waypoint, spatial.map),
                    lead.walker.radius,
                    Some(lead.walker.mask),
                    &ignore,
                )?;
                if hits.is_empty() {
                    path.points.clear();
                    let _ = path.insert(0, at);
                    let _ = path.push(waypoint);
                    path.measure();
                    found = true;
                }
            }
            if found { Stage::Found } else { Stage::Long }
        }
    } else {
        Stage::Found
    };
    if matches!(stage, Stage::Found) && (path.points.len() < 2 || path.length < SHORT_LENGTH) {
        stage = Stage::Long;
    }
    if matches!(stage, Stage::Long) {
        // the long pather (DAT_1439e3cc8)
        path.points.clear();
        let walker = LongWalker {
            id: lead.walker.id,
            position: lead.position,
            radius: lead.radius,
            mask: lead.walker.mask,
            moves: lead.moves,
            ignore_units: &lead.ignore_units,
            group_units: lead_group.map(|g| g.lowered.clone()),
        };
        let long_target = target.map(|t| (target_id, t));
        let (reached, long) = long_path_for(
            &walker,
            from,
            spatial,
            lead.computer,
            (waypoint, range),
            long_target,
        )?;
        path = long;
        path.measure();
        found = reached;
        if EPSILON <= path.length {
            if found {
                let end = path.points.last().copied().unwrap_or([0.0; 3]);
                if way < ground(waypoint, end) {
                    path.points.clear();
                    path.length = 0.0;
                }
            }
        } else {
            found = false;
        }
    }
    Ok(link(motion, lead, path, (found, planned), now))
}

/// The request's end (`LAB_14104b4c5`): a top level without a found path, or a path of fewer
/// than two points, drops the new level: a level not found with fewer than [`LAST_TRIES`] plans
/// left at its last waypoint pauses [`PATROL_PAUSE`] on a patrol, waits for its parent (state 5)
/// when that has waypoints left, else gives up; before its last it counts its waypoint reached;
/// with more plans it pauses [`STUCK_PAUSE`]. A found one that does not loop arrived (the level
/// resets, the group stands). Else the level waits (state 0, its plans back to 10) under the new
/// level: a top level's plan becomes level 1 in state 2, any other path level 0 walked at once
/// (state 3), linked as found (1) or not (2).
pub(super) fn link(
    motion: &mut Motion,
    lead: &Lead,
    path: Path,
    (found, planned): (bool, bool),
    now: u32,
) -> Option<Pass> {
    let level = &motion.paths[0];
    if (level.level == 2 && !found) || path.points.len() < 2 {
        if !found {
            if level.continue_counter < LAST_TRIES {
                if level.points.len() as i32 - 1 <= level.waypoint {
                    let patrols = lead.group.as_ref().is_some_and(LeadGroup::patrols);
                    if level.path_flags & 1 != 0 || patrols {
                        motion.paths[0].pause_time = PATROL_PAUSE;
                        return Some(Pass::Done(false));
                    }
                    let top_has_more = motion
                        .paths
                        .last()
                        .is_some_and(|t| t.has_path && t.waypoint < t.points.len() as i32 - 1);
                    if level.level < 2 && top_has_more {
                        motion.paths[0].state = 5;
                        return Some(Pass::Next);
                    }
                    give_up(motion);
                    return Some(Pass::Done(false));
                }
                return Some(Pass::Reached);
            }
            motion.paths[0].pause_time = STUCK_PAUSE;
            motion.stand();
            return Some(Pass::Done(false));
        }
        if level.path_flags & 1 == 0 {
            motion.stand();
            reset_level(&mut motion.paths);
            return Some(Pass::Done(true));
        }
        return Some(Pass::Reached);
    }
    let level = &mut motion.paths[0];
    level.state = 0;
    level.continue_counter = CONTINUE;
    let (range, target, top) = (level.range, level.target, level.level == 2);
    let new = PathData {
        points: path.points,
        has_path: true,
        path_flags: NEW_PATH_FLAGS,
        length: path.length,
        created: now,
        range,
        target,
        waypoint: 1,
        state: if top && planned { 2 } else { 3 },
        level: u8::from(top && planned),
        linked_type: if found { 1 } else { 2 },
        ..PathData::without_path()
    };
    motion.paths.insert(0, new);
    // a top level's plan waits for its stretches (the next pass); a walked level steers now
    if top && planned {
        Some(Pass::Next)
    } else {
        None
    }
}

/// `LAB_14104be63`: the walked level gives up (`FUN_140f9b4b0`, then `FUN_140f98e80`: it resets,
/// state 0), and the group stands.
pub(super) fn give_up(motion: &mut Motion) {
    reset_level(&mut motion.paths);
    motion.stand();
}
