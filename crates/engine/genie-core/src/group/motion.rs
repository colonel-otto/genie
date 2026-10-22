//! A group's motion (FD-036): `BMotion`'s update (`FUN_14104a090`, motion.cpp): its path step
//! (`FUN_14104a270`) over the path levels a unit's motion step keeps alike, planned for the
//! group's lead from the group's place, the walk toward the walked level's waypoint with its turn
//! (`FUN_141046270`, `FUN_14104c150`), then the velocity along the vectors.

use crate::fmath::{self, Vec3};
use crate::group::Motion;
use crate::movement::{
    LongWalker, PathData, Planning, Walker, footprint_gap, footprint_gap_squared, long_path_for,
    short_path_for, tile_plan_for,
};
use crate::spatial::{Placed, Spatial};
use crate::tile_path::Path;
use crate::turn::Turns;
use crate::unit_ai::Unsupported;

/// A move or a turn below this counts as none (`DAT_142eb0c60`).
const EPSILON: f32 = crate::image::group::EPSILON;
/// How near a point the walk counts as there (`DAT_142eb5690`).
const NEAR: f32 = crate::image::group::NEAR;
/// A waypoint or a target nearer than this takes the straight way when nothing stands on it
/// (`DAT_14330eb8c`).
const CLOSE: f32 = crate::image::group::CLOSE;
/// A path shorter than this is no path (`DAT_142eb99cc`).
const SHORT_LENGTH: f32 = crate::image::group::SHORT_LENGTH;
/// The short pather serves a waypoint this near (`DAT_142eb0f4c`).
const SHORT_REACH: f32 = crate::image::group::SHORT_REACH;
/// The members' centre pulls the group when it is this far (squared, `DAT_142eb0f48`) ...
const PULL_DISTANCE: f32 = crate::image::group::PULL_DISTANCE;
/// ... by this share (`DAT_142eae8d0`), the group keeping this much of its place
/// (`DAT_142ecb878`).
const PULL_SHARE: f32 = crate::image::group::PULL_SHARE;
const PULL_KEEP: f32 = crate::image::group::PULL_KEEP;
/// The passes of one step (`0x14 < local_7e0` ends it).
const PASSES: u32 = 20;
/// A level with fewer plans left than this gives up rather than wait (`FUN_140f95bf0 < 3`).
const LAST_TRIES: i32 = 3;
/// A level's plans (`FUN_140f8f670`, `FUN_140f9b800`).
const CONTINUE: i32 = 10;
/// A new path's flags (`FUN_14152eea0`).
const NEW_PATH_FLAGS: u32 = 6;
/// A patrolling group at its last waypoint pauses this long (the immediate `0x3f000000`).
const PATROL_PAUSE: f32 = 0.5;
/// A group without a path left pauses this long (the immediate `0x3dcccccd`).
const STUCK_PAUSE: f32 = 0.1;

/// What `BMotion` reads of the group's lead (`+0xc4`, `FUN_141016870`'s member).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Lead {
    /// What the planners read of it.
    pub(crate) walker: Walker,
    /// Its own place (x, y, height), from which the long pather measures its target.
    pub(crate) position: [f32; 3],
    /// Its master's radii on the ground.
    pub(crate) radius: [f32; 2],
    /// It moves (slot 0x208 not 0, or master 42), as the long pather reads it.
    pub(crate) moves: bool,
    /// Its master has a footprint and a height (`FUN_141194060` with 1): the short pather serves it.
    pub(crate) solid: bool,
    /// Its ignore units (`+0x670`).
    pub(crate) ignore_units: Vec<i32>,
    /// Its owner's index and whether that is a computer player (`FUN_141659cf0`).
    pub(crate) player: usize,
    pub(crate) computer: bool,
    /// Its group (`+0x6d0`), when the world holds it.
    pub(crate) group: Option<LeadGroup>,
}

/// What `BMotion` reads of the lead's group.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct LeadGroup {
    /// `mGroupFlags`: bit 11 marks a patrol.
    pub(crate) flags: u32,
    /// The members (`+0xf0`).
    pub(crate) members: Vec<i32>,
    /// The differentiator groups' members, at most 128 (`FUN_14100bbe0` with `0x80`).
    pub(crate) differentiator_units: Vec<i32>,
    /// The same, at most 255, as the long pather lowers them.
    pub(crate) lowered: Vec<i32>,
}

impl LeadGroup {
    const fn patrols(&self) -> bool {
        self.flags & 0x800 != 0
    }
}

impl Turns {
    /// Slot 0x280 (`FUN_141082d60`): the id of the object the unit's top action targets
    /// (`FUN_140a78450`: an id above -1 that the world holds), else -1.
    pub(crate) fn top_target_object(&self, u: &crate::turn::Unit) -> i32 {
        u.top_target()
            .filter(|&t| -1 < t && self.object(t).is_some())
            .unwrap_or(-1)
    }
}

/// What `BMotion` reads of the group itself besides its motion: its owner is a computer player
/// (`FUN_141659cf0` on the group's `+0xdc`), and per member its top action's target (slot 0x280,
/// `FUN_141082d60`: -1 without one) and where it last moved to (slot 0x520, x, height, y), each
/// None for a member the world does not hold.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct GroupView {
    pub(crate) computer: bool,
    pub(crate) members: Vec<Option<(i32, Vec3)>>,
}

/// One pass's end (the routine's labels).
enum Pass {
    /// `LAB_14104bd5d`: the next pass.
    Next,
    /// `LAB_14104bd53`: the walked level's waypoint counts as reached (its bit 0), then the next
    /// pass.
    Reached,
    /// The step ends with this answer.
    Done(bool),
}

/// `FUN_140f9b4d0(level, bit, on)` on the walked level: its flag `bit` set or cleared; bit 2 is
/// then set on every parent too.
fn set_flag(paths: &mut [PathData], bit: u32, on: bool) {
    let Some((level, parents)) = paths.split_first_mut() else {
        return;
    };
    if on {
        level.flags |= 1 << bit;
    } else {
        level.flags &= !(1 << bit);
    }
    if bit == 2 {
        for p in parents {
            p.flags |= 4;
        }
    }
}

/// `FUN_140f98e80` on the walked level: it and its parents go, the level holds the defaults
/// (its plans back to 10, flags clear; its repath time stays).
fn reset_level(paths: &mut Vec<PathData>) {
    let repath_time = paths.first().map_or(0.0, |l| l.repath_time);
    paths.truncate(1);
    let level = PathData {
        repath_time,
        ..PathData::without_path()
    };
    if paths.is_empty() {
        paths.push(level);
    } else {
        paths[0] = level;
    }
}

/// `FUN_140f98bc0` with 1 on a looping path's level: the waypoint after `index`; on a path that
/// goes back and forth (flags bit 1) the walk turns at its ends (bit 2 the way back).
const fn looping_next(level: &mut PathData, index: i32) -> i32 {
    let n = level.points.len() as i32;
    if level.path_flags & 1 == 0 {
        return -1;
    }
    if level.path_flags & 2 == 0 {
        return if index + 1 < n { index + 1 } else { 0 };
    }
    if level.path_flags & 4 == 0 {
        let next = index - 1;
        if next < 0 {
            level.path_flags |= 4;
            return 1;
        }
        next
    } else {
        let next = index + 1;
        if n <= next {
            level.path_flags &= !4;
            return n - 2;
        }
        next
    }
}

/// The waypoint at `index` (`FUN_140f9bcd0`), the game's static zero point outside the path.
fn point(level: &PathData, index: i32) -> Vec3 {
    usize::try_from(index)
        .ok()
        .and_then(|i| level.points.get(i))
        .copied()
        .unwrap_or([0.0; 3])
}

/// The length on the ground from `a` to `b` (`FUN_14087e560`, the height zeroed).
fn ground(a: Vec3, b: Vec3) -> f32 {
    let mut d = fmath::sub(a, b);
    d[1] = 0.0;
    fmath::sqrt(d[0] * d[0] + 0.0 + d[2] * d[2])
}

/// `FUN_141046130` for a walk from `at` toward `waypoint` that would end at `next`: 0 short of
/// it, 2 past it (or a step under 0.1), 1 on it.
fn passes_waypoint(waypoint: Vec3, at: Vec3, next: Vec3) -> u8 {
    let mut a = fmath::sub(waypoint, at);
    let mut b = fmath::sub(waypoint, next);
    let c = fmath::sub(next, at);
    a[1] = 0.0;
    b[1] = 0.0;
    let to_waypoint = fmath::sqrt(a[0] * a[0] + 0.0 + a[2] * a[2]);
    let beyond = fmath::sqrt(b[0] * b[0] + b[1] * b[1] + b[2] * b[2]);
    let step = fmath::sqrt(c[0] * c[0] + c[1] * c[1] + c[2] * c[2]);
    if step <= to_waypoint {
        0
    } else if to_waypoint < beyond || step < NEAR {
        2
    } else {
        1
    }
}

/// `FUN_1411570c0`'s ends held inside the map: below 0 at 0, from the map's size on at the size
/// less 0.1, per axis.
fn hold(p: Vec3, map: [i32; 2]) -> [f32; 2] {
    let axis = |v: f32, size: i32| {
        if v < 0.0 {
            0.0
        } else if size as f32 <= v {
            size as f32 - NEAR
        } else {
            v
        }
    };
    [axis(p[0], map[0]), axis(p[2], map[1])]
}

impl Motion {
    /// `FUN_141046600`: the motion moves (its velocity's length, `sqrt(x² + y² + z²)`, is above
    /// 0).
    #[must_use]
    pub fn moving(&self) -> bool {
        let v = self.velocity;
        0.0 < fmath::sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
    }

    /// `FUN_141046640`: the walked level waits for a path (its state 1 or 2); false without one.
    #[must_use]
    pub fn waits_to_move(&self) -> bool {
        self.paths.first().is_some_and(|l| matches!(l.state, 1 | 2))
    }

    /// `FUN_1410481f0`: with a walked level below another, the levels below the first of level
    /// 2 go (`FUN_140f98e80`, each alone); that level walks again from waypoint 1 in state 2 with
    /// 10 tries; the top reached without one is reset ([`reset_level`]). A lone walked level
    /// stays as it is.
    pub fn drop_to_high_level(&mut self) {
        if self.paths.len() < 2 {
            return;
        }
        while self.paths[0].level != 2 && 1 < self.paths.len() {
            let _ = self.paths.remove(0);
        }
        let level = &mut self.paths[0];
        if level.level == 2 {
            level.waypoint = 1;
            level.state = 2;
            level.continue_counter = 10;
        } else {
            reset_level(&mut self.paths);
        }
    }

    /// The walk's stop (`LAB_14104bd7e`): the forward speed and its acceleration go to 0.
    const fn stand(&mut self) {
        self.acceleration[2] = 0.0;
        self.velocity[2] = 0.0;
    }

    /// `BMotion::calculateYawAngle` (`FUN_141046270`): the angle from `forward` (`param_2`) to
    /// `direction` (`param_3`, `FUN_14087e650`); with `clamp` (`param_5` 1) at most `+0xa0`
    /// (the yaw per second moving) times `dt` (`param_4`), as `MINSS` takes it at 0x1410463e2
    /// (the limit when it is below the angle, else the angle, also when either is NaN); negated
    /// when `direction × forward` (`FUN_14087e4d0`) points up (`COMISS`/`JBE` at 0x141046400:
    /// not for a NaN). The warning for a `dt` of `f32::MAX` with a yaw per second above 1 and
    /// the one of the result are log lines only.
    #[must_use]
    pub fn calculate_yaw_angle(&self, forward: Vec3, direction: Vec3, dt: f32, clamp: bool) -> f32 {
        let mut angle = fmath::angle_between(direction, forward);
        if clamp {
            angle = fmath::minss(self.maximum_yaw_per_second_moving * dt, angle);
        }
        if 0.0 < fmath::cross(direction, forward)[1] {
            -angle
        } else {
            angle
        }
    }

    /// `FUN_14104c150`: a turn of `yaw` (above 1e-7 in size) about the new position's up: its
    /// forward and right turn (`FUN_14087e350`, `FUN_14087e2c0`) and are normalized
    /// (`FUN_141027d00`); motion flags bit 0.
    pub fn turn(&mut self, yaw: f32) {
        // only an ordered 1e-7 below the yaw's size turns: a NaN yaw does not
        if fmath::at_most_or_unordered(yaw.abs(), EPSILON) {
            return;
        }
        let r = fmath::rotation(yaw, self.new_position[2]);
        self.new_position[1] = fmath::normalize(fmath::rotate(&r, self.new_position[1]));
        self.new_position[3] = fmath::normalize(fmath::rotate(&r, self.new_position[3]));
        self.flags |= 1;
    }

    /// `BMotion`'s update (`FUN_14104a090`) after its path step ([`path_step`]): the forward
    /// speed grows by `dt` times its acceleration; the new position moves by `dt` times each
    /// speed along its vector (forward, right, up) when that is above 1e-7 in size (motion flags
    /// bit 0); then the position is the new position, all four vectors.
    pub fn integrate(&mut self, dt: f32) {
        self.velocity[2] += dt * self.acceleration[2];
        for (speed, along) in [
            (self.velocity[2], 1),
            (self.velocity[0], 3),
            (self.velocity[1], 2),
        ] {
            let d = dt * speed;
            if EPSILON < d.abs() {
                let v = fmath::scale(d, self.new_position[along]);
                let p = &mut self.new_position[0];
                *p = [p[0] + v[0], v[1] + p[1], v[2] + p[2]];
                self.flags |= 1;
            }
        }
        self.position = self.new_position;
    }
}

/// The world as `BMotion`'s path step reads it.
pub(crate) struct World<'w, 'p> {
    pub(crate) spatial: &'w Spatial,
    pub(crate) plan: &'w mut Planning<'p>,
    pub(crate) dt: f32,
}

/// `FUN_14104a270` (the path step of `BMotion`'s update) on `motion`, for a group whose lead is
/// `lead`: `budget` (`param_2`) tests a computer player's path budget before a plan. Returns the
/// routine's answer (1 when the walk goes on or arrived, 0 when it waits).
///
/// A group without path data does nothing; a paused walked level waits out its pause. Then up
/// to [`PASSES`] passes over the walked level, as a unit's motion step's: a reached waypoint
/// moves the level on (or back to the waypoint it left when it is not in range, or to its parent
/// waypoint on a looping path); an arrived level (states 4, 5) hands over to its parent by its
/// link; a level in state 2 gets a new level with a path to its waypoint, planned from the
/// group's place for the lead (the tile pathfinder for a top level, else for a waypoint within
/// [`SHORT_REACH`] the short pather, a near waypoint the straight way when nothing stands on it,
/// else the long pather), which becomes level 1 waiting for its stretches (a top level's plan)
/// or level 0 walked at once; a walking level steers toward its waypoint: a units' query along
/// the step finds what stands in the way (the target alone ends the step there; a unit moved
/// after the path was made sends the level back to its parent), else the group turns toward the
/// waypoint, the members' centre pulls it when every member acts on a target, and the forward
/// speed takes the top speed.
///
/// # Errors
///
/// What the planners or the queries do not model.
pub(crate) fn path_step(
    motion: &mut Motion,
    group: &GroupView,
    lead: &Lead,
    world: &mut World<'_, '_>,
    budget: bool,
) -> Result<bool, Unsupported> {
    if motion.paths.is_empty() {
        return Ok(false);
    }
    let budget = budget && group.computer;
    let dt = world.dt;
    let level = &mut motion.paths[0];
    if 0.0 < level.pause_time {
        level.pause_time -= dt;
        if 0.0 < level.pause_time {
            motion.stand();
            return Ok(false);
        }
        level.pause_time = 0.0;
    }
    let mut passes = 1;
    loop {
        match pass(motion, group, lead, world, budget)? {
            Pass::Done(r) => return Ok(r),
            Pass::Reached => set_flag(&mut motion.paths, 0, true),
            Pass::Next => {}
        }
        passes += 1;
        if PASSES < passes {
            return Ok(false);
        }
    }
}

/// One pass of [`path_step`] (`LAB_14104a3fd` to `LAB_14104bd5d`).
fn pass(
    motion: &mut Motion,
    group: &GroupView,
    lead: &Lead,
    world: &mut World<'_, '_>,
    budget: bool,
) -> Result<Pass, Unsupported> {
    let spatial = world.spatial;
    let level = &motion.paths[0];
    if !level.has_path {
        return Ok(Pass::Done(false));
    }
    let target_id = level.target;
    let target: Option<Placed> = if 0 <= target_id {
        spatial.get_known(target_id)?.copied()
    } else {
        None
    };
    let at = motion.position[0];
    // `fVar39`: the target's footprint gap to the group's place, squared (FUN_140a847c0)
    let target_gap2 = target.as_ref().map_or(f32::MAX, |t| {
        footprint_gap_squared(t.position, t.radius, [at[0], at[2], 0.0], [0.0, 0.0])
    });
    let flags = level.flags;
    if flags & 1 != 0 {
        reached(motion, target.as_ref());
    }
    let state = motion.paths[0].state;
    if state == 4 || state == 5 {
        if motion.paths.len() < 2 {
            reset_level(&mut motion.paths);
            motion.stand();
            return Ok(Pass::Done(true));
        }
        let linked = motion.paths[0].linked_type;
        if linked == 1 || flags & 4 != 0 {
            let _ = motion.paths.remove(0);
            return Ok(Pass::Reached);
        }
        if linked == 2 || linked == 3 {
            let _ = motion.paths.remove(0);
            let patrols = lead.group.as_ref().is_some_and(LeadGroup::patrols);
            let parent = &mut motion.paths[0];
            parent.state = 2;
            if linked == 2 && parent.has_path {
                if parent.path_flags & 1 != 0 {
                    let w = parent.waypoint;
                    parent.waypoint = looping_next(parent, w);
                }
                if patrols {
                    parent.waypoint += 1;
                    if parent.points.len() as i32 <= parent.waypoint {
                        parent.state = 4;
                    }
                }
            }
            return Ok(Pass::Next);
        }
        // another link: the level walks on in its state
    }
    if motion.paths[0].state == 2
        && let Some(p) = request(motion, lead, world, budget, target.as_ref(), target_gap2)?
    {
        return Ok(p);
    }
    steer(motion, group, lead, world, target.as_ref())
}

/// The reached waypoint (the walked level's bit 0, `0x14104a4bf`): with bit 1 the group takes
/// the waypoint's place; both bits clear and the level moves on. On a path that does not loop,
/// past its last waypoint, a group not yet within the range plus 0.1 of where it was going (its
/// target's footprint, else the waypoint it left) steps back to that waypoint and asks again
/// (state 2), else the level arrived (state 4); with waypoints left a level above 0 asks for the
/// next stretch (state 2). A level that reached its target with bit 2 clears it (and sets it on
/// its parents). A looping path takes its next waypoint (`FUN_140f98bc0`) and asks again.
fn reached(motion: &mut Motion, target: Option<&Placed>) {
    let level = &motion.paths[0];
    if level.flags & 2 != 0 {
        if level.waypoint < level.points.len() as i32 {
            let p = level.points[level.waypoint as usize];
            motion.new_position[0] = p;
            motion.position[0] = p;
        }
        set_flag(&mut motion.paths, 1, false);
    }
    set_flag(&mut motion.paths, 0, false);
    let at = motion.position[0];
    let level = &mut motion.paths[0];
    level.waypoint += 1;
    let left = level.waypoint - 1;
    if level.path_flags & 1 != 0 {
        level.waypoint = looping_next(level, left);
        level.state = 2;
        return;
    }
    let d = fmath::length(fmath::sub(at, point(level, left)));
    let mut beyond = match target {
        None => (d - level.range) - NEAR,
        Some(t) => {
            (footprint_gap(t.position, t.radius, [at[0], at[2], 0.0], [0.0, 0.0]) - level.range)
                - NEAR
        }
    };
    if target.is_some() && level.flags & 4 != 0 {
        beyond = 0.0;
        set_flag(&mut motion.paths, 2, false);
    }
    let level = &mut motion.paths[0];
    if level.waypoint < level.points.len() as i32 {
        if level.level != 0 {
            level.state = 2;
        }
    } else if EPSILON <= beyond
        && (target.is_some() || CLOSE <= beyond || level.level == 0)
        && EPSILON <= d
    {
        level.waypoint -= 1;
        level.state = 2;
    } else {
        level.state = 4;
    }
}

/// Where the request (`FUN_14104a270`'s state 2) goes on after a planner.
enum Stage {
    /// `LAB_14104b2ae`: a found path long enough goes to the link.
    Found,
    /// `LAB_14104b2cd`: the long pather.
    Long,
}

/// The path request of the walked level in state 2 (`0x14104a80b` on): see [`path_step`].
#[allow(clippy::too_many_lines)]
fn request(
    motion: &mut Motion,
    lead: &Lead,
    world: &mut World<'_, '_>,
    budget: bool,
    target: Option<&Placed>,
    target_gap2: f32,
) -> Result<Option<Pass>, Unsupported> {
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
                let hits = spatial.line_hits(
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
fn link(
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
fn give_up(motion: &mut Motion) {
    reset_level(&mut motion.paths);
    motion.stand();
}

/// The walk of the walked level (`LAB_14104b6b4`): see [`path_step`].
fn steer(
    motion: &mut Motion,
    group: &GroupView,
    lead: &Lead,
    world: &World<'_, '_>,
    target: Option<&Placed>,
) -> Result<Pass, Unsupported> {
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
    let hits = spatial.line_hits(
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
    pub fn motion_update(&mut self, group: i32, budget: bool) -> Result<(), Unsupported> {
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
                object: self.streams.get(crate::rng::Stream::Object),
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

    /// What `BMotion` reads of the lead `id` (a unit with movement state), None without one.
    fn lead_view(&self, id: i32) -> Option<Lead> {
        let (p, u) = self.object(id)?;
        let mover = u.mover.as_ref()?;
        let group = self.groups.get(u.group.id).map(|g| LeadGroup {
            flags: g.flags,
            members: g.members.iter().map(|m| m.id).collect(),
            differentiator_units: self.groups.differentiator_units(g, SHORT_IGNORED),
            lowered: self.groups.group_units(u.group.id).unwrap_or_default(),
        });
        Some(Lead {
            walker: mover.walker(id),
            position: u.position,
            radius: [mover.stats.radius[0], mover.stats.radius[1]],
            moves: mover.stats.speed != 0.0 || u.life.master_id == crate::movement::LONG_PATH_MOVER,
            solid: mover.stats.radius.iter().all(|&r| 0.0 < r),
            ignore_units: mover.ignore_units.clone(),
            player: p,
            computer: self.players.get(p).is_some_and(|q| q.computer),
            group,
        })
    }
}

/// The ignore list the short pather and the walk's query take of the lead's group, at most this
/// many (`FUN_14100bbe0` with `0x80`; the request's own loop stops past `0x7f`).
const SHORT_IGNORED: usize = 128;

/// Slot 0x520 of a unit: where it last moved to, as the game's vector (x, height, y).
pub(super) fn moved_vector(u: &crate::turn::Unit) -> Vec3 {
    u.mover
        .as_ref()
        .map_or([u.position[0], u.position[2], u.position[1]], |m| {
            [m.moved_at[0], m.moved_at[2], m.moved_at[1]]
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Slot 0x280 (`FUN_141082d60`) passes only a target id above -1 the world holds: a
    /// recycled object's negative id gives -1 like none.
    #[test]
    fn a_top_target_below_minus_one_is_none() {
        use crate::turn::{Player, Unit};
        let hunter = |target: i32| Unit {
            id: 5,
            hunt: Some(crate::hunt::tests::hunting(target)),
            ..Unit::default()
        };
        let held = |id: i32| Unit {
            id,
            ..Unit::default()
        };
        let player = Player {
            objects: vec![held(7), held(-3)],
            ..Player::default()
        };
        let t = Turns::new(0, 0, 0, vec![0, 0], vec![Player::default(), player]);
        assert_eq!(t.top_target_object(&hunter(7)), 7);
        assert_eq!(t.top_target_object(&hunter(-3)), -1);
        assert_eq!(t.top_target_object(&hunter(9)), -1);
        assert_eq!(t.top_target_object(&held(1)), -1);
    }

    /// `FUN_14104c150` turns only when 1e-7 is below the yaw's size (an ordered compare): a NaN
    /// yaw leaves the vectors and the motion flags alone.
    #[test]
    fn a_nan_yaw_does_not_turn() {
        let mut m = Motion::new();
        let before = m.clone();
        m.turn(f32::NAN);
        assert_eq!(m.flags, before.flags);
        assert_eq!(
            format!("{:?}", m.new_position),
            format!("{:?}", before.new_position)
        );
    }
    use crate::group::UnitGroups;
    use crate::pathing::Pathing;
    use crate::rng::Well512;

    fn level(level: u8, points: &[[f32; 2]], waypoint: i32, state: i32) -> PathData {
        PathData {
            level,
            points: points.iter().map(|&[x, y]| [x, 0.0, y]).collect(),
            has_path: true,
            path_flags: NEW_PATH_FLAGS,
            waypoint,
            state,
            ..PathData::without_path()
        }
    }

    /// `FUN_141046600` and `FUN_141046640`: a motion moves with any velocity, and waits to move
    /// while its walked level wants a path (state 1 or 2).
    #[test]
    fn a_motion_moves_with_a_velocity_and_waits_for_a_path() {
        let mut m = Motion::new();
        assert!(!m.moving() && !m.waits_to_move());
        m.velocity = [0.0, 0.0, -0.5];
        assert!(m.moving());
        m.paths = vec![level(0, &[], 0, 2)];
        assert!(m.waits_to_move());
        m.paths[0].state = 3;
        assert!(!m.waits_to_move());
    }

    /// `FUN_1410481f0`: levels 0 and 1 below the top (2) go and the top walks from waypoint 1
    /// (state 2, 10 tries); a top of another level resets; a lone level stays.
    #[test]
    fn the_walked_levels_below_the_high_level_go() {
        let mut m = Motion::new();
        let mut top = level(2, &[[0.0, 0.0], [5.0, 5.0], [9.0, 9.0]], 2, 4);
        top.continue_counter = 3;
        m.paths = vec![
            level(0, &[[1.0, 1.0]], 1, 3),
            level(1, &[[2.0, 2.0]], 1, 0),
            top,
        ];
        m.drop_to_high_level();
        assert_eq!(m.paths.len(), 1);
        let l = &m.paths[0];
        assert_eq!(
            (l.level, l.waypoint, l.state, l.continue_counter),
            (2, 1, 2, 10)
        );
        assert_eq!(l.points.len(), 3);
        m.paths = vec![level(0, &[[1.0, 1.0]], 1, 3), level(1, &[[2.0, 2.0]], 1, 0)];
        m.paths[0].repath_time = 4.0;
        m.drop_to_high_level();
        assert_eq!(m.paths, [PathData::without_path()]);
        let lone = vec![level(0, &[[1.0, 1.0]], 1, 3)];
        m.paths.clone_from(&lone);
        m.drop_to_high_level();
        assert_eq!(m.paths, lone);
    }

    /// `FUN_140f9b4d0`: a bit on or off the walked level; bit 2 also goes on for its parents.
    #[test]
    fn a_levels_flag_and_its_parents_bit_2() {
        let mut paths = vec![
            level(0, &[], 0, 3),
            level(1, &[], 0, 0),
            level(2, &[], 0, 0),
        ];
        set_flag(&mut paths, 0, true);
        assert_eq!(paths[0].flags, 1);
        set_flag(&mut paths, 2, false);
        assert_eq!((paths[0].flags, paths[1].flags, paths[2].flags), (1, 4, 4));
        set_flag(&mut paths, 0, false);
        assert_eq!(paths[0].flags, 0);
        assert_eq!(paths[1].flags, 4);
    }

    /// `FUN_140f98e80`: the walked level and its parents go; the level keeps its repath time.
    #[test]
    fn a_reset_level_drops_its_parents_and_keeps_its_repath_time() {
        let mut paths = vec![level(0, &[[1.0, 1.0]], 1, 3), level(2, &[], 0, 0)];
        paths[0].repath_time = 2.5;
        reset_level(&mut paths);
        assert_eq!(paths.len(), 1);
        assert_eq!(
            paths[0],
            PathData {
                repath_time: 2.5,
                ..PathData::without_path()
            }
        );
    }

    /// `FUN_140f98bc0`: a loop goes round; a path back and forth turns at its ends.
    #[test]
    fn a_looping_path_goes_round_or_back_and_forth() {
        let mut l = level(1, &[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]], 0, 2);
        assert_eq!(looping_next(&mut l, 0), -1);
        l.path_flags = 1;
        assert_eq!(looping_next(&mut l, 1), 2);
        assert_eq!(looping_next(&mut l, 2), 0);
        l.path_flags = 1 | 2;
        assert_eq!(looping_next(&mut l, 1), 0);
        assert_eq!(looping_next(&mut l, 0), 1);
        assert_eq!(l.path_flags & 4, 4);
        assert_eq!(looping_next(&mut l, 1), 2);
        assert_eq!(looping_next(&mut l, 2), 1);
        assert_eq!(l.path_flags & 4, 0);
    }

    /// `FUN_141046130`: short of the waypoint, on it, or past it (or a step under 0.1).
    #[test]
    fn a_step_falls_short_of_reaches_or_passes_its_waypoint() {
        let w = [10.0, 0.0, 0.0];
        assert_eq!(passes_waypoint(w, [0.0; 3], [5.0, 0.0, 0.0]), 0);
        // a step that ends on the waypoint is not past it
        assert_eq!(passes_waypoint(w, [9.95, 0.0, 0.0], [10.0, 0.0, 0.0]), 0);
        // one past it by less than 0.1 is
        assert_eq!(passes_waypoint(w, [9.95, 0.0, 0.0], [10.02, 0.0, 0.0]), 2);
        assert_eq!(passes_waypoint(w, [9.0, 0.0, 0.0], [10.0, 0.0, 0.0]), 0);
        // past it, but no farther beyond than it was before: on it
        assert_eq!(passes_waypoint(w, [9.0, 0.0, 0.0], [10.5, 0.0, 0.0]), 1);
        assert_eq!(passes_waypoint(w, [9.0, 0.0, 0.0], [11.0, 0.0, 0.0]), 1);
        assert_eq!(passes_waypoint(w, [9.0, 0.0, 0.0], [12.0, 0.0, 0.0]), 2);
    }

    /// `FUN_1411570c0`'s ends held inside the map.
    #[test]
    fn a_querys_ends_stay_inside_the_map() {
        assert_eq!(hold([-1.0, 5.0, 3.0], [10, 20]), [0.0, 3.0]);
        assert_eq!(
            hold([10.0, 0.0, 25.0], [10, 20]),
            [10.0 - NEAR, 20.0 - NEAR]
        );
    }

    /// `calculateYawAngle` with its clamp: a quarter turn to the left is negative, and limited
    /// by the yaw per second times dt; without the clamp (`param_5` 0) the whole angle.
    #[test]
    fn a_turn_is_limited_by_the_yaw_per_second() {
        let mut m = Motion::new();
        let (x, z) = ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        let free = m.calculate_yaw_angle(x, z, 0.1, true);
        assert_eq!(free, -std::f32::consts::FRAC_PI_2);
        m.maximum_yaw_per_second_moving = 1.0;
        assert_eq!(m.calculate_yaw_angle(x, z, 0.1, true), -0.1);
        assert_eq!(m.calculate_yaw_angle(z, x, 0.1, true), 0.1);
        assert_eq!(
            m.calculate_yaw_angle(z, x, 0.1, false),
            std::f32::consts::FRAC_PI_2
        );
    }

    /// The clamp is `MINSS limit, angle` (0x1410463e2): a NaN limit leaves the angle, a NaN
    /// angle stays NaN (an ordered `<=` would take the limit both times); a NaN cross product
    /// keeps the sign (`COMISS`/`JBE`).
    #[test]
    fn the_yaw_clamp_keeps_the_angle_for_a_nan_operand() {
        let mut m = Motion::new();
        let (x, z) = ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
        m.maximum_yaw_per_second_moving = f32::NAN;
        assert_eq!(
            m.calculate_yaw_angle(x, z, 0.1, true),
            -std::f32::consts::FRAC_PI_2
        );
        m.maximum_yaw_per_second_moving = 1.0;
        let nan = m.calculate_yaw_angle([f32::NAN, 0.0, 0.0], z, 0.1, true);
        assert!(nan.is_nan(), "{nan}");
    }

    /// `FUN_14104c150` turns the new position's forward and right about its up; a turn of 1e-7
    /// or less is none. The integration then moves the group along its forward and copies the
    /// new position.
    #[test]
    fn the_motion_turns_then_moves_along_its_forward() {
        let mut m = Motion::new();
        m.flags = 0;
        m.turn(1e-8);
        assert_eq!((m.new_position, m.flags), (m.position, 0));
        m.turn(-std::f32::consts::FRAC_PI_2);
        assert_eq!(m.flags, 1);
        let f = m.new_position[1];
        assert!((f[2] - 1.0).abs() < 1e-6 && f[0].abs() < 1e-6, "{f:?}");
        m.flags = 0;
        m.velocity = [0.0, 0.0, 2.0];
        m.acceleration = [0.0, 0.0, 1.0];
        m.integrate(0.5);
        assert_eq!(m.velocity[2], 2.5);
        let p = m.position[0];
        assert!((p[2] - 1.25).abs() < 1e-6 && p[0].abs() < 1e-6, "{p:?}");
        assert_eq!(m.position, m.new_position);
        assert_eq!(m.flags, 1);
    }

    struct Fixture {
        spatial: Spatial,
        pathing: Pathing,
        object: Well512,
        groups: UnitGroups,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                spatial: Spatial::new([40, 40]),
                pathing: Pathing::default(),
                object: Well512::default(),
                groups: UnitGroups::default(),
            }
        }

        fn step(&mut self, m: &mut Motion, group: &GroupView, lead: &Lead) -> bool {
            let mut plan = Planning {
                pathing: &mut self.pathing,
                now: 100,
                computer: false,
                object: &mut self.object,
                construction_rate: 0.0,
                groups: &self.groups,
            };
            let mut world = World {
                spatial: &self.spatial,
                plan: &mut plan,
                dt: 0.1,
            };
            path_step(m, group, lead, &mut world, false).unwrap()
        }
    }

    fn lead() -> Lead {
        Lead {
            walker: Walker {
                id: 7,
                radius: 0.2,
                restriction: 0,
                mask: 0xff,
            },
            position: [5.0, 5.0, 0.0],
            radius: [0.2, 0.2],
            moves: true,
            solid: true,
            ignore_units: Vec::new(),
            player: 1,
            computer: false,
            group: None,
        }
    }

    /// A walked level heads for its waypoint: the group turns to it and takes its top speed.
    #[test]
    fn a_walked_level_turns_the_group_toward_its_waypoint() {
        let mut fx = Fixture::new();
        let mut m = Motion::new();
        m.position[0] = [5.0, 0.0, 5.0];
        m.new_position[0] = [5.0, 0.0, 5.0];
        m.maximum_velocity_forward = 1.2;
        m.paths = vec![level(0, &[[5.0, 5.0], [5.0, 9.0]], 1, 3)];
        let group = GroupView::default();
        assert!(fx.step(&mut m, &group, &lead()));
        assert_eq!(m.velocity[2], 1.2);
        let f = m.new_position[1];
        assert!((f[2] - 1.0).abs() < 1e-6 && f[0].abs() < 1e-6, "{f:?}");
    }

    /// A reached last waypoint of a top level without a parent arrives: the level resets and
    /// the group stands.
    #[test]
    fn the_last_waypoint_of_the_top_level_ends_the_walk() {
        let mut fx = Fixture::new();
        let mut m = Motion::new();
        m.position[0] = [5.0, 0.0, 9.0];
        m.velocity = [0.0, 0.0, 1.2];
        let mut top = level(2, &[[5.0, 5.0], [5.0, 9.0]], 1, 3);
        top.flags = 1;
        m.paths = vec![top];
        assert!(fx.step(&mut m, &GroupView::default(), &lead()));
        assert_eq!(m.paths, vec![PathData::without_path()]);
        assert_eq!(m.velocity[2], 0.0);
    }

    /// An arrived walked level linked as found hands over to its parent, whose waypoint then
    /// counts as reached; a pause makes the group stand until it ran out.
    #[test]
    fn an_arrived_level_hands_over_and_a_pause_waits() {
        let mut fx = Fixture::new();
        let mut m = Motion::new();
        m.position[0] = [5.0, 0.0, 9.0];
        let mut walked = level(0, &[[5.0, 5.0], [5.0, 9.0]], 1, 4);
        walked.linked_type = 1;
        let parent = level(2, &[[5.0, 5.0], [5.0, 9.0]], 1, 0);
        m.paths = vec![walked, parent];
        // the parent's reached last waypoint ends the walk in the same step
        assert!(fx.step(&mut m, &GroupView::default(), &lead()));
        assert_eq!(m.paths.len(), 1);
        assert_eq!(m.paths[0].state, 0);
        let mut m = Motion::new();
        m.velocity = [0.0, 0.0, 1.0];
        let mut paused = level(0, &[[5.0, 5.0], [5.0, 9.0]], 1, 3);
        paused.pause_time = 0.3;
        m.paths = vec![paused];
        assert!(!fx.step(&mut m, &GroupView::default(), &lead()));
        assert_eq!(m.velocity[2], 0.0);
        assert!((m.paths[0].pause_time - 0.2).abs() < 1e-6);
    }
}
