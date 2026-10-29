//! The walked level's path request (state 2) and its end: the planners, the link of the
//! found path, the give-up.

use crate::GenieError;
use crate::groups::Motion;
use crate::groups::motion::{
    CLOSE, CONTINUE, EPSILON, LAST_TRIES, Lead, LeadGroup, NEAR, NEW_PATH_FLAGS, PATROL_PAUSE,
    Pass, SHORT_LENGTH, SHORT_REACH, STUCK_PAUSE, StepWorld, ground, hold, point, reset_level,
};
use crate::map::spatial::{Placed, Spatial};
use crate::math::fmath::Vec3;
use crate::movement::{
    LongWalker, PathData, TileAsk, long_path_for, short_path_for, tile_plan_for,
};
use crate::pathing::tile_path::Path;

/// What the request plans toward: the walked level's number, range and target, its waypoint,
/// the group's place (x, height, y; `from` its ground point) and the ground distance between
/// them.
struct Leg {
    level: u8,
    range: f32,
    target_id: i32,
    waypoint: Vec3,
    at: Vec3,
    from: [f32; 2],
    way: f32,
}

/// The path the planners made so far, and what the routine's flags say of it.
#[derive(Default)]
struct PathSoFar {
    path: Path,
    /// The path reaches the waypoint.
    found: bool,
    /// The tile pathfinder made it: a top level's plan, linked as level 1.
    planned: bool,
    /// A path stands to link.
    has_plan: bool,
}

/// The path request of the walked level in state 2 (`0x14104a80b` on): see [`path_step`]. Its
/// turn first ([`take_turn`]); then the tile pathfinder for a top level ([`tile_plan`]); without
/// a plan of three points or more, the short pather for a waypoint within [`SHORT_REACH`]
/// ([`short_plan`]), and with still no plan the straight way when the waypoint (less the
/// level's range at a top level's last waypoint) or the target is nearer than [`CLOSE`]
/// ([`straight_way`]); with none found, or a path of fewer than two points or shorter than
/// [`SHORT_LENGTH`], the long pather ([`long_plan`]). The path goes to the link ([`link`]).
///
/// [`path_step`]: crate::groups::motion::step::path_step
pub(super) fn request(
    motion: &mut Motion,
    lead: &Lead,
    world: &mut StepWorld<'_, '_>,
    budget: bool,
    target: Option<&Placed>,
    target_gap2: f32,
) -> Result<Option<Pass>, GenieError> {
    if let Some(pass) = take_turn(motion, lead, world, budget) {
        return Ok(Some(pass));
    }
    let level = &motion.paths[0];
    let (at, waypoint) = (motion.position[0], point(level, level.waypoint));
    let leg = Leg {
        level: level.level,
        range: level.range,
        target_id: level.target,
        waypoint,
        at,
        from: [at[0], at[2]],
        way: ground(waypoint, at),
    };
    let mut plan = PathSoFar::default();
    if leg.level == 2 {
        tile_plan(lead, world, &leg, target, &mut plan)?;
    }
    let level = &motion.paths[0];
    let (short_range, beyond) = if leg.level == 2 && level.points.len() as i32 == level.waypoint + 1
    {
        (leg.range, leg.way - leg.range)
    } else {
        (0.0, leg.way)
    };
    let mut long = false;
    if !plan.has_plan || plan.path.points.len() < 3 {
        if leg.way <= SHORT_REACH
            && let Some(pass) = short_plan(
                motion,
                lead,
                world,
                &leg,
                (target, target_gap2),
                short_range,
                &mut plan,
            )?
        {
            return Ok(Some(pass));
        }
        if !plan.has_plan {
            if beyond < CLOSE || target_gap2 < CLOSE {
                straight_way(lead, world.spatial, &leg, &mut plan)?;
            }
            long = !plan.found;
        }
    }
    if long || plan.path.points.len() < 2 || plan.path.length < SHORT_LENGTH {
        long_plan(lead, world.spatial, &leg, target, &mut plan)?;
    }
    let now = world.plan.now;
    Ok(link(
        motion,
        lead,
        plan.path,
        (plan.found, plan.planned),
        now,
    ))
}

/// The request's turn (`0x14104a80b`): with `budget` a player over its path budget waits (its
/// wait delays counted, the group stands). Else the plan counts against the player's budget,
/// the wait delays go back to 0, and the walked level spends one of its plans when the top level
/// has no path, or does not loop and the lead's group does not patrol: the last one spent, it
/// gives up ([`give_up`]). None when the request goes on.
fn take_turn(
    motion: &mut Motion,
    lead: &Lead,
    world: &mut StepWorld<'_, '_>,
    budget: bool,
) -> Option<Pass> {
    if budget
        && !world
            .plan
            .pathing
            .budget(lead.player)
            .allows(motion.number_wait_delays)
    {
        motion.number_wait_delays += 1;
        motion.stand();
        return Some(Pass::Done(false));
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
            return Some(Pass::Done(false));
        }
    }
    None
}

/// The tile pathfinder for a top level (`DAT_1439e3cc4`) from the group's place to the
/// waypoint, within the target's radius plus the lead's: without a planner for the lead's
/// restriction the straight way is the plan; a result of 1, 2 or above 4 is a plan, dropped when
/// its end lies farther from the waypoint than the group does.
fn tile_plan(
    lead: &Lead,
    world: &mut StepWorld<'_, '_>,
    leg: &Leg,
    target: Option<&Placed>,
    plan: &mut PathSoFar,
) -> Result<(), GenieError> {
    let spatial = &mut *world.spatial;
    let radii = target.map_or(0.0, |t| t.radius[0]) + lead.walker.radius;
    let mut planning = world.plan.reborrow();
    planning.computer = lead.computer;
    let r = tile_plan_for(
        lead.walker,
        leg.from,
        target.map(|_| leg.target_id),
        spatial,
        &mut planning,
        TileAsk {
            to: leg.waypoint,
            radii,
            range: leg.range,
            target,
        },
        &mut plan.path,
    )?;
    match r {
        None => {
            let _ = plan.path.insert(0, leg.at);
            let _ = plan.path.push(leg.waypoint);
            (plan.planned, plan.has_plan, plan.found) = (true, true, true);
        }
        Some(r) if (1..=2).contains(&r) || 4 < r => {
            (plan.planned, plan.found, plan.has_plan) = (true, true, true);
            let end = plan.path.points.last().copied().unwrap_or([0.0; 3]);
            if leg.way < ground(leg.waypoint, end) {
                plan.path.points.clear();
                (plan.found, plan.has_plan) = (false, false);
            }
        }
        Some(_) => {}
    }
    plan.path.measure();
    Ok(())
}

/// The short pather (`DAT_1439e3cc0`) for a near waypoint, within `short_range` of it, the
/// lead's differentiator units (else the lead) ignored; it runs only for a lead it serves (else
/// its result is 0 with no path). A result other than 3 or 4 with a path of two points or more
/// longer than [`EPSILON`] is the plan; with result 0 and no such path the group is where it
/// was going ([`in_range`]). None when the request goes on (a plan only when the pather made
/// one).
fn short_plan(
    motion: &mut Motion,
    lead: &Lead,
    world: &mut StepWorld<'_, '_>,
    leg: &Leg,
    (target, target_gap2): (Option<&Placed>, f32),
    short_range: f32,
    plan: &mut PathSoFar,
) -> Result<Option<Pass>, GenieError> {
    let spatial = &mut *world.spatial;
    plan.planned = false;
    plan.path.points.clear();
    let ignore = lead
        .group
        .as_ref()
        .map_or_else(|| vec![lead.walker.id], |g| g.differentiator_units.clone());
    let mut r = 0;
    if lead.solid {
        let mut planning = world.plan.reborrow();
        planning.computer = lead.computer;
        r = short_path_for(
            lead.walker,
            leg.from,
            spatial,
            &mut planning,
            (leg.waypoint, short_range, leg.target_id),
            &ignore,
            &mut plan.path,
        )?;
    }
    plan.path.measure();
    plan.has_plan = plan.found;
    if r == 3 || r == 4 {
        return Ok(None);
    }
    if EPSILON < plan.path.length && 1 < plan.path.points.len() {
        (plan.found, plan.has_plan) = (true, true);
        return Ok(None);
    }
    if r != 0 || !(plan.path.points.len() < 2 || plan.path.length < EPSILON) {
        return Ok(None);
    }
    let reach = if target.is_none() || 0.0 < leg.range {
        leg.range
    } else {
        NEAR
    };
    Ok(Some(in_range(motion, reach, target_gap2, world.plan.now)))
}

/// The short pather found no way, the group at its waypoint: a group farther than `reach` from
/// the target (every group without one, its gap the largest float) counts the waypoint reached,
/// and at the last waypoint of a level made in this tick (`now`) the walk ends there (the group
/// stands, the level resets and takes state 6: `FUN_140f9b4b0`); a group within reach arrived
/// (state 4).
fn in_range(motion: &mut Motion, reach: f32, target_gap2: f32, now: u32) -> Pass {
    if reach < target_gap2 {
        let level = &motion.paths[0];
        if level.waypoint == level.points.len() as i32 - 1 && level.created == now {
            motion.stand();
            reset_level(&mut motion.paths);
            motion.paths[0].state = 6;
            return Pass::Done(true);
        }
        return Pass::Reached;
    }
    motion.paths[0].state = 4;
    Pass::Next
}

/// The straight way when nothing stands on it (`FUN_1411570c0` with its tiles, from the group's
/// place to the waypoint, both held inside the map, the lead's group's members, else the lead,
/// ignored): the two points are the plan, found.
fn straight_way(
    lead: &Lead,
    spatial: &mut Spatial,
    leg: &Leg,
    plan: &mut PathSoFar,
) -> Result<(), GenieError> {
    let ignore = lead
        .group
        .as_ref()
        .map_or_else(|| vec![lead.walker.id], |g| g.members.clone());
    let hits = spatial.line_query(
        hold(leg.at, spatial.map),
        hold(leg.waypoint, spatial.map),
        lead.walker.radius,
        Some(lead.walker.mask),
        &ignore,
    )?;
    if hits.is_empty() {
        plan.path.points.clear();
        let _ = plan.path.insert(0, leg.at);
        let _ = plan.path.push(leg.waypoint);
        plan.path.measure();
        plan.found = true;
    }
    Ok(())
}

/// The long pather (`DAT_1439e3cc8`) for the lead from the group's place to the waypoint within
/// the level's range (and to the target): its path is the plan, found when it reaches; a found
/// path whose end lies farther from the waypoint than the group does keeps no points (length
/// 0), and a path shorter than [`EPSILON`] is not found.
fn long_plan(
    lead: &Lead,
    spatial: &Spatial,
    leg: &Leg,
    target: Option<&Placed>,
    plan: &mut PathSoFar,
) -> Result<(), GenieError> {
    let walker = LongWalker {
        id: lead.walker.id,
        position: lead.position,
        radius: lead.radius,
        mask: lead.walker.mask,
        moves: lead.moves,
        ignore_units: &lead.ignore_units,
        group_units: lead.group.as_ref().map(|g| g.lowered.clone()),
    };
    let (reached, long) = long_path_for(
        &walker,
        leg.from,
        spatial,
        lead.computer,
        (leg.waypoint, leg.range),
        target.map(|t| (leg.target_id, t)),
    )?;
    plan.path = long;
    plan.path.measure();
    plan.found = reached;
    if EPSILON <= plan.path.length {
        if plan.found {
            let end = plan.path.points.last().copied().unwrap_or([0.0; 3]);
            if leg.way < ground(leg.waypoint, end) {
                plan.path.points.clear();
                plan.path.length = 0.0;
            }
        }
    } else {
        plan.found = false;
    }
    Ok(())
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
