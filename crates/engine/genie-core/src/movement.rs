//! A unit's movement (`RGE_Moving_Object`, MovingObject.cpp in 158041): the velocity step that
//! moves it along its forward vector, and the motion step that steers it along its path.
//!
//! Per tick, `RGE_Moving_Object::update` (`0x14114dbb0`) runs first: it brings the forward speed
//! toward the acceleration's target and moves the unit `dt × speed` along the forward vector it had
//! after the last tick ([`Mover::step_velocity`]). The actions run later in the same update; a
//! MoveTo calls the motion step (object slot 193, `0x141150f00`, [`Mover::motion`]), which turns the
//! unit toward its waypoint and sets the acceleration for the next tick.
//!
//! Vectors are the game's `BVector`s ([`fmath::Vec3`]: x, height, y).

use crate::fmath::{self, Vec3};
use crate::pathing::Pathing;
use crate::rng::Well512;
use crate::short_path::{self, Call, Entry, Listed};
use crate::spatial::{Placed, Spatial};
use crate::tile_path::{Path, Request};
use crate::unit_ai::Unsupported;

/// The velocity data (`+0x868`): velocity and acceleration in the unit's own frame; component 2 is
/// along the forward vector.
#[derive(Clone, Debug, PartialEq)]
pub struct Velocity {
    pub velocity: Vec3,
    pub acceleration: Vec3,
}

/// One level of a unit's path data (`+0x858`; each level's parent at `+8`).
#[derive(Clone, Debug, PartialEq)]
pub struct PathData {
    /// `+0x14`, `mWaypointLevel`: 0 is the path the unit walks.
    pub level: u8,
    /// `+0x10`, `mLinkedPathType`: what a finished level hands its parent.
    pub linked_type: i32,
    /// The path's waypoints (`+0x18`, the path's `+0x438`..`+0x440`).
    pub points: Vec<Vec3>,
    /// The path's own flags (`+0xc`).
    pub path_flags: u32,
    /// `+0x24`: the index of the waypoint the unit walks to.
    pub waypoint: i32,
    /// `+0x28`: 2 wants a path, 3 walks, 4 and 5 arrived.
    pub state: i32,
    /// `+0x2c`: how near the target counts as there.
    pub range: f32,
    /// `+0x30`: the target object's id, −1 for a point.
    pub target: i32,
    /// `+0x34`, in seconds.
    pub pause_time: f32,
    /// `+0x3c`: bit 0 a waypoint was reached, bit 1 it was the waypoint itself, bit 3 the tile
    /// pathfinder found only the nearest point.
    pub flags: u32,
    /// The path's length (its `+4`, `mPathLengthValue`).
    pub length: f32,
    /// The path's creation time (its `+8`, `mCreationTimeValue`): the world's time when it was
    /// made.
    pub created: u32,
    /// `+0x38`, `mContinueCounter`: the plans the level may still try before it gives up.
    pub continue_counter: i32,
    /// `+0x40`, `mRepathTime`.
    pub repath_time: f32,
}

/// What movement reads of the unit's master.
#[derive(Clone, Debug, PartialEq)]
pub struct MoveStats {
    /// The top speed (object slot 65, `0x141202c70`): the master's speed (`+0x740`), times the
    /// current sprite's speed factor when that is above 0.
    pub speed: f32,
    /// Master `+0x774`: the most the unit turns per second.
    pub turn_rate: f32,
    /// Master `+0x76c` and `+0x770`: the turn radius is `turn_radius × speed / turn_radius_speed`.
    pub turn_radius: f32,
    pub turn_radius_speed: f32,
    /// Master `+0x5d8`, `+0x5dc`, `+0x5e0`.
    pub radius: [f32; 3],
    /// The master's terrain restriction (`+0x606`) and the terrain classes it lets the unit stand
    /// on (its mask).
    pub restriction: usize,
    pub restriction_mask: u8,
    /// The master's stationary yaw revolution time (`+0x758`) is 0: the unit faces a target at
    /// once (`FUN_141162db0`).
    pub turns_at_once: bool,
    /// The master's move algorithm (`+0x768`, the `.dat`'s `move_algorithm`): bit 0 has a stop
    /// reset the unit's sprite (slot 0x5d8).
    pub move_algorithm: u8,
}

/// A moving unit's movement state.
#[derive(Clone, Debug, PartialEq)]
pub struct Mover {
    /// `+0x830`, a unit vector on the ground.
    pub forward: Vec3,
    /// `+0x83c`.
    pub right: Vec3,
    pub velocity: Option<Velocity>,
    /// The path data, the level the unit walks first, each next one its parent.
    pub paths: Vec<PathData>,
    /// `+0x878`: moveForward's distance factor.
    pub factor: f32,
    pub stats: MoveStats,
    /// `+0x7f9`, `waitingToMoveValue`.
    pub waiting: u8,
    /// `+0x7fa`, `numberWaitDelays`: plans that failed for want of budget (trade carts).
    pub wait_delays: u8,
    /// `+0x7fb`, `mAttackMoving`: a level spends no try, and one without a path pauses.
    pub attack_moving: bool,
    /// `+0x870`, `lastMoveTimeValue`: the world's time when the unit last moved.
    pub last_move: u32,
    /// `+0x874`, `mConsecutiveSubstituteCount`: above 2 a walk counts its waypoint reached.
    pub substitute_count: i32,
}

/// The moving unit, as the motion step needs it besides its [`Mover`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Me {
    pub id: i32,
    /// x, y, z on the map.
    pub position: [f32; 3],
}

/// What the motion step returns to the MoveTo action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    /// 0: still on the way.
    Moving,
    /// 1: the unit cannot move or has no path.
    Done,
    /// 2: it arrived.
    Arrived,
}

/// The gap below which the game takes a distance as zero (`DAT_142eb0c60`).
const EPSILON: f32 = 1e-7;
/// The world's up axis (`DAT_143cc2f30`), the yaw's axis.
const UP: Vec3 = [0.0, 1.0, 0.0];
/// How far inside the map's edge a moving unit's footprint stays (`DAT_142eb99cc`), and how far
/// below the edge a position stays (`DAT_142eb1e9c`).
const EDGE_GAP: f32 = 0.01;
const EDGE_POSITION_GAP: f32 = 0.001;
/// A walk to a point checks its range only above this (`DAT_142eb5690`).
const RANGE_FLOOR: f32 = 0.1;
/// The class the motion step measures by its annexes (`-0xe85` encoded).
const GATE: i16 = 39;
/// The trade cart's class (19), whose path requests the owner's budget limits (`FUN_14165d6f0`).
const TRADE_CART: i16 = 19;
/// No distance (`DAT_14330ebf4`).
const FAR: f32 = f32::MAX;
/// The motion step's switches (`DAT_1439e3cc0`, `DAT_1439e3cc4`, `DAT_1439e3cc8`): the short
/// pather, the tile pathfinder and the long pather run; all three are 1 at run time.
const SHORT_PATHER: bool = true;
const TILE_PATHER: bool = true;
const LONG_PATHER: bool = true;
/// The short pather serves a waypoint this near (`DAT_142ec9ec0`), and one this near a target
/// before the tile pathfinder (`DAT_142eb0f4c`).
const SHORT_REACH: f32 = 16.0;
const TARGET_SHORT_REACH: f32 = 10.0;
/// A waypoint or target nearer than this takes the close-in path (`DAT_14330eb8c`).
const CLOSE: f32 = 1.0;
/// A path shorter than this is too short (`DAT_142eb99cc`).
const SHORTEST: f32 = 0.01;
/// A planned path shorter than this replaces no short path (`DAT_143cc2f28`), and a path is long
/// when it is longer than this and than 1.25 times the way (`DAT_14330ebb8`, `DAT_142eb56a8`).
const LONG: f32 = 5.0;
const DETOUR: f32 = 1.25;
/// A new level's stretches longer than this squared get a midpoint (`DAT_142ed6f20`).
const SPLIT: f32 = 25.0;
/// How many plans one motion step makes at most.
const PLANS: u32 = 3;
/// A level's plans before it gives up (`FUN_140f8f670`).
const CONTINUE: i32 = 10;

/// A position on the map as the game's vector (x, height, y), and back.
fn vector([x, y, z]: [f32; 3]) -> Vec3 {
    [x, z, y]
}

fn map_position([x, z, y]: Vec3) -> [f32; 3] {
    [x, y, z]
}

impl PathData {
    /// A new level (`FUN_140f8f670`) with a new path (`FUN_14152eea0`, flags 6, made `now`).
    fn new(now: u32) -> Self {
        PathData {
            level: 0,
            linked_type: 0,
            points: Vec::new(),
            path_flags: 6,
            waypoint: 0,
            state: 0,
            range: 0.0,
            target: -1,
            pause_time: 0.0,
            flags: 0,
            length: 0.0,
            created: now,
            continue_counter: CONTINUE,
            repath_time: 0.0,
        }
    }

    /// The waypoint at `index` (`FUN_140f9bcd0`); the game returns a static zero point outside the
    /// path, which the slice does not follow.
    fn point(&self, index: i32) -> Result<Vec3, Unsupported> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.points.get(i))
            .copied()
            .ok_or(Unsupported("a waypoint index outside its path"))
    }

    /// The level has waypoints after the current one (`FUN_140f96050` on a top level).
    fn has_more(&self) -> bool {
        !self.points.is_empty() && self.waypoint < self.points.len() as i32 - 1
    }
}

/// A point of a path (`FUN_140f9bcd0`): outside it, the game's static point (`DAT_143f0dc88`),
/// zero.
fn point_or_zero(path: &Path, index: i32) -> Vec3 {
    usize::try_from(index)
        .ok()
        .and_then(|i| path.points.get(i))
        .copied()
        .unwrap_or([0.0; 3])
}

/// A waypoint of a level's path, or the game's static zero point outside it.
fn point_or_zero_of(level: &PathData, index: i32) -> Vec3 {
    usize::try_from(index)
        .ok()
        .and_then(|i| level.points.get(i))
        .copied()
        .unwrap_or([0.0; 3])
}

/// The position setter's test for a move (`FUN_141161950`): a coordinate changed by more than
/// 1e-7.
fn moved_far(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).any(|i| EPSILON < (b[i] - a[i]).abs())
}

/// The length on the ground from `a` to `b`.
fn ground(a: Vec3, b: Vec3) -> f32 {
    let mut d = fmath::sub(a, b);
    d[1] = 0.0;
    fmath::sqrt(d[0] * d[0] + 0.0 + d[2] * d[2])
}

/// The short pather's range for a request (`0x141150f00`): the level's range less 0.1 when its
/// goal is in reach ([`Mover::near_goal`]), at least 0; else 0.
fn short_range(near: bool, range: f32) -> f32 {
    if !near {
        return 0.0;
    }
    let r = range - RANGE_FLOOR;
    if r <= 0.0 { 0.0 } else { r }
}

/// A path too short to walk: fewer than two points or shorter than 0.01.
fn too_short(path: &Path) -> bool {
    path.points.len() < 2 || path.length < SHORTEST
}

/// `FUN_140f9b8f0`: a midpoint between points `i` and `i + 1` (from `i` half the stretch along
/// it), unless the stretch is shorter than 1e-7.
fn split(path: &mut Path, i: i32) {
    let Ok(at) = usize::try_from(i) else {
        return;
    };
    if path.points.len() - 1 <= at {
        return;
    }
    let (a, b) = (path.points[at], path.points[at + 1]);
    let d = fmath::sub(b, a);
    let length = fmath::sqrt(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
    if length < EPSILON {
        return;
    }
    let again = fmath::sqrt(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
    let half = length * 0.5;
    let step = [
        (d[0] / again) * half,
        (d[1] / again) * half,
        (d[2] / again) * half,
    ];
    path.insert(at + 1, fmath::add(a, step));
}

/// What the motion step knows of where the walked level goes: its target, the gap to it
/// (`fVar38`, far without one) and the gap to where the top level's path ends, or the target's
/// (`fVar40`).
#[derive(Clone, Copy)]
struct Goal<'w> {
    target: Option<&'w Placed>,
    gap: f32,
    to_end: f32,
}

/// What the motion step's planning needs of the world besides the objects' places.
pub struct Planning<'a> {
    /// The planners, the mesh and the short pather.
    pub pathing: &'a mut Pathing,
    /// The world's time this update (`FUN_1415300d0`), which a new path records.
    pub now: u32,
    /// The owner is a computer player (`FUN_141659cf0`, `player+0x10 == 3`): its plans may stop
    /// near the goal.
    pub computer: bool,
    /// The Object stream (`DAT_143f07160`), which a long new path's repath time draws from.
    pub object: &'a mut Well512,
    /// The owner's construction rate (attribute 195), which a builder's work reads.
    pub construction_rate: f32,
}

impl Planning<'_> {
    /// The same planning for a call that ends before this one is used again.
    pub fn reborrow(&mut self) -> Planning<'_> {
        Planning {
            pathing: &mut *self.pathing,
            now: self.now,
            computer: self.computer,
            object: &mut *self.object,
            construction_rate: self.construction_rate,
        }
    }
}

/// Where the motion step goes on after a pass over the walked level.
enum Next {
    /// Back to the top of the step (`LAB_141151124`).
    Again,
    /// On to the level's state (`LAB_141151b6d`) with what the pass knew of its goal.
    Dispatch,
    /// The step returns.
    Done(Motion),
}

/// Where the motion step goes on after a level's path request.
enum Step {
    /// Back to the top of the step (`LAB_141151124`) for the level now walked.
    Again,
    /// On to the walk (`LAB_141153bd1`).
    Walk,
}

/// The squared distance between two footprints on the ground (`FUN_14118f8c0`): per axis the
/// centre distance minus both radii, at least 0, squared and summed.
pub fn footprint_gap_squared(
    a: [f32; 3],
    a_radius: [f32; 2],
    b: [f32; 3],
    b_radius: [f32; 2],
) -> f32 {
    let x = ((b[0] - a[0]).abs() - (a_radius[0] + b_radius[0])).max(0.0);
    let y = ((b[1] - a[1]).abs() - (a_radius[1] + b_radius[1])).max(0.0);
    x * x + y * y
}

/// The distance between two footprints on the ground (`FUN_14118f680`): per axis the centre
/// distance minus both radii, at least 0, then the length of the two.
pub fn footprint_gap(a: [f32; 3], a_radius: [f32; 2], b: [f32; 3], b_radius: [f32; 2]) -> f32 {
    let x = ((b[0] - a[0]).abs() - (b_radius[0] + a_radius[0])).max(0.0);
    let y = ((b[1] - a[1]).abs() - (b_radius[1] + a_radius[1])).max(0.0);
    fmath::sqrt(x * x + y * y)
}

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
        let at = self.move_forward(dt * speed, position, map);
        if moved_far(position, at) {
            self.last_move = now;
        }
        at
    }

    /// `moveForward` (slot 182): `distance × factor` along the forward vector, through the
    /// position setter; a distance below 1e-7 moves nothing.
    fn move_forward(&self, distance: f32, position: [f32; 3], map: [i32; 2]) -> [f32; 3] {
        let d = distance * self.factor;
        if d.abs() < EPSILON {
            return position;
        }
        let to = fmath::add(vector(position), fmath::scale(d, self.forward));
        self.set_position(map_position(to), map)
    }

    /// The position setter (slot 11 -> `FUN_141161950` -> `0x1411a58b0`): a unit that can move
    /// keeps its footprint 0.01 inside the map, then any position stays 0.001 below the map's size.
    fn set_position(&self, [x, y, z]: [f32; 3], map: [i32; 2]) -> [f32; 3] {
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
        let (x, y) = if EPSILON < self.stats.speed {
            (
                inside(x, self.stats.radius[0], map[0]),
                inside(y, self.stats.radius[1], map[1]),
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
    ) -> Result<Motion, Unsupported> {
        if self.stats.speed == 0.0 {
            return Ok(Motion::Done);
        }
        let Some(level) = self.paths.first_mut() else {
            return Ok(Motion::Done);
        };
        if 0.0 < level.pause_time {
            level.pause_time -= dt;
            if 0.0 < level.pause_time {
                return Err(Unsupported("a paused path (stopMoving)"));
            }
            level.pause_time = 0.0;
        }
        let ground_radius = [self.stats.radius[0], self.stats.radius[1]];
        let mut plans = 0;
        loop {
            let Some(level) = self.paths.first() else {
                return Ok(Motion::Done);
            };
            let target = match level.target {
                -1 => None,
                id => Some(
                    world
                        .get_known(id)?
                        .ok_or(Unsupported("a path target that is gone"))?
                        .clone(),
                ),
            };
            if target.as_ref().is_some_and(|t| t.class == GATE) {
                return Err(Unsupported("a path to a gate (its annexes)"));
            }
            // the gap to the target, else to where the top level's path ends
            let gap = target.as_ref().map_or(FAR, |t| {
                footprint_gap(me.position, ground_radius, t.position, t.radius)
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
                }
            }
            match self.paths[0].state {
                4 | 5 => match self.arrived(goal)? {
                    Next::Done(m) => return Ok(m),
                    _ => continue,
                },
                2 => {
                    let plan = plan
                        .as_deref_mut()
                        .ok_or(Unsupported("a path request without the world's planners"))?;
                    match self.request(me, world, plan, goal, &mut plans)? {
                        Step::Again => continue,
                        Step::Walk => {}
                    }
                }
                _ => {}
            }
            match self.walk(me, dt, world, goal)? {
                Next::Done(m) => return Ok(m),
                _ => continue,
            }
        }
    }

    /// The walk along the level's path (`0x141150f00` from `LAB_141153bd1`): toward the waypoint
    /// in a straight line, turning to face it. On the waypoint, or when the step would lead away
    /// from it, the waypoint counts as reached and the motion step starts again; a step that
    /// reaches it marks it reached and the walk goes on. A walk to a target ends in range of it
    /// (the waypoint reached); a walk to a point with a range ends where its top path ends (the
    /// unit stops, the step returns 2). A walk that may meet an obstruction is not modelled.
    fn walk(
        &mut self,
        me: &Me,
        dt: f32,
        world: &Spatial,
        goal: Goal<'_>,
    ) -> Result<Next, Unsupported> {
        let ground_radius = [self.stats.radius[0], self.stats.radius[1]];
        let position = vector(me.position);
        let level = &self.paths[0];
        let waypoint = point_or_zero_of(level, level.waypoint);
        if ground(position, waypoint) < EPSILON {
            self.paths[0].flags |= 3;
            return Ok(Next::Again);
        }
        let direction = self.direction_to(position, waypoint);
        let projected = self.projected(position, direction, dt);
        match reach(waypoint, position, projected) {
            Reach::Away => {
                self.paths[0].flags |= 3;
                return Ok(Next::Again);
            }
            Reach::Reached => self.paths[0].flags |= 3,
            Reach::Short if 2 < self.substitute_count => self.paths[0].flags |= 3,
            Reach::Short => {}
        }
        if self.stats.radius.iter().all(|&r| 0.0 < r) {
            let rect = self.query_rect(position, projected, world.map);
            let grow = self.stats.radius[0];
            world.check_known([
                rect[0] - grow,
                rect[1] - grow,
                rect[2] + grow,
                rect[3] + grow,
            ])?;
            let near = world.obstruction_candidates(rect, grow, &[me.id]);
            if !near.is_empty() {
                return Err(Unsupported("a walker that may meet an obstruction"));
            }
        }
        let level = &self.paths[0];
        match goal.target {
            Some(t) => {
                let gap =
                    footprint_gap(me.position, ground_radius, t.position, t.radius) - level.range;
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
        self.steer(direction, dt);
        self.control_speed(position, waypoint);
        Ok(Next::Done(Motion::Moving))
    }

    /// The waypoint the level walks to was reached (`LAB_14115136c`, the level's bit 0): with
    /// bit 1 the unit takes the waypoint's place (slot 0x58, the position setter); the level moves
    /// on to its next waypoint, dropping those too near the map's edge for the unit
    /// (`FUN_140f98d50`). Then (`LAB_14115192d`) a level with waypoints left waits for a path to
    /// the next (state 2, not level 0); at its last, a unit not yet in range of where it was going
    /// steps back to the waypoint it left and asks again, else the level arrived (state 4). A
    /// looping path, a waypoint some object stands on and level 0's look ahead on a level 1
    /// parent (`FUN_141142750`) are not modelled.
    fn reached(
        &mut self,
        me: &mut Me,
        world: &mut Spatial,
        pathing: Option<&mut Pathing>,
        goal: Goal<'_>,
        now: u32,
    ) -> Result<Next, Unsupported> {
        let ground_radius = [self.stats.radius[0], self.stats.radius[1]];
        let level = &mut self.paths[0];
        if level.flags & 2 != 0 {
            if level.waypoint < level.points.len() as i32 {
                let p = level.point(level.waypoint)?;
                let at = self.set_position(map_position(p), world.map);
                self.moved_to(me, at, world, pathing, now);
            }
            self.paths[0].flags &= !2;
        }
        let level = &mut self.paths[0];
        level.flags &= !1;
        level.waypoint += 1;
        let [rx, ry, _] = self.stats.radius;
        let [w, h] = world.map;
        while level.waypoint < level.points.len() as i32 {
            let [x, _, y] = level.points[level.waypoint as usize];
            if rx <= x && ry <= y && x <= w as f32 - rx && y <= h as f32 - ry {
                break;
            }
            level.points.remove(level.waypoint as usize);
        }
        if level.path_flags & 1 != 0 {
            return Err(Unsupported(
                "a looping path's next waypoint (FUN_140f98bc0)",
            ));
        }
        let next = level.waypoint + 1;
        if level.level != 0 && next < level.points.len() as i32 {
            let d = fmath::sub(vector(me.position), level.points[next as usize]);
            if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < TARGET_SHORT_REACH {
                let [x, _, y] = level.points[level.waypoint as usize];
                world.check_known([x, y, x, y])?;
                if !world
                    .obstruction_candidates([x, y, x, y], 0.0, &[])
                    .is_empty()
                {
                    return Err(Unsupported("a next waypoint some object stands on"));
                }
            }
        }
        let parent_is_level_1 = self
            .paths
            .get(1)
            .is_some_and(|p| p.level == 1 && !p.points.is_empty());
        if self.paths[0].level == 0 && parent_is_level_1 {
            return Err(Unsupported(
                "level 0's look ahead on its level 1 parent (FUN_141142750)",
            ));
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
        let mut beyond = match goal.target {
            None => (back - level.range) - RANGE_FLOOR,
            Some(t) => {
                footprint_gap(me.position, ground_radius, t.position, t.radius)
                    - level.range
                    - RANGE_FLOOR
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
                return Err(Unsupported(
                    "a top level short of its goal at its end stops the unit",
                ));
            }
        }
        level.state = 4;
        Ok(Next::Dispatch)
    }

    /// An arrived level (states 4 and 5, `LAB_141151b6d`): the top level, or one in range of its
    /// target, is reset (`FUN_140f98e80`) and the unit stops (slot 0x5d0): the step returns 2. Any
    /// other hands over to its parent by its link (`mLinkedPathType`): 1 marks the parent's
    /// waypoint reached, 2 has the parent ask again (a looping parent moves on first), 3 has it
    /// ask again; the step starts again with the parent.
    fn arrived(&mut self, goal: Goal<'_>) -> Result<Next, Unsupported> {
        let level = &self.paths[0];
        if self.paths.len() == 1 || goal.gap <= level.range {
            self.reset_level();
            self.stop()?;
            return Ok(Next::Done(Motion::Arrived));
        }
        match level.linked_type {
            1 => {
                self.paths.remove(0);
                self.paths[0].flags |= 1;
            }
            2 => {
                self.paths.remove(0);
                let parent = &mut self.paths[0];
                parent.state = 2;
                if parent.path_flags & 1 != 0 {
                    return Err(Unsupported(
                        "a looping parent's next waypoint (FUN_140f98bc0)",
                    ));
                }
            }
            3 => {
                self.paths.remove(0);
                self.paths[0].state = 2;
            }
            _ => return Err(Unsupported("an arrived level of another link")),
        }
        Ok(Next::Again)
    }

    /// `FUN_140f98e80` on the walked level: its path goes, its parents with it, and it holds the
    /// defaults (its tries back to 10, flags clear; its repath time stays).
    fn reset_level(&mut self) {
        self.paths.truncate(1);
        let level = &mut self.paths[0];
        *level = PathData {
            repath_time: level.repath_time,
            created: 0,
            path_flags: 0,
            ..PathData::new(0)
        };
    }

    /// `FUN_140f9b4d0(level, 2, 0)`: bit 2 goes off the walked level and on for its parents.
    fn clear_bit_2(&mut self) {
        self.paths[0].flags &= !4;
        for parent in &mut self.paths[1..] {
            parent.flags |= 4;
        }
    }

    /// Slot 0x5d0 (`FUN_14108c2f0` -> `FUN_141161730`): the unit stops: its speed and its
    /// acceleration along its forward vector go to 0 (slots 0x598 and 0x5a8, on velocity data it
    /// has), it stops waiting to move (`+0x7f9`, slot 0x2a0) and its own ignore list (`+0x678`)
    /// empties. `+0x7b8` is cleared too, which the slice does not hold; a master whose move
    /// algorithm has bit 0 also resets the unit's sprite (`+0x78c`, `+0x958`), which is not
    /// modelled.
    pub(crate) fn stop(&mut self) -> Result<(), Unsupported> {
        if self.stats.move_algorithm & 1 != 0 {
            return Err(Unsupported(
                "a stop that resets the sprite (move algorithm bit 0)",
            ));
        }
        if let Some(v) = self.velocity.as_mut() {
            v.velocity[2] = 0.0;
            v.acceleration[2] = 0.0;
        }
        self.waiting = 0;
        Ok(())
    }

    /// The unit's place changed inside the motion step: the world's places and the mesh follow
    /// (`0x1411a58b0` -> `FUN_14118c540`), and the time of the last move with them
    /// (`FUN_141161950`, `+0x870`).
    fn moved_to(
        &mut self,
        me: &mut Me,
        at: [f32; 3],
        world: &mut Spatial,
        pathing: Option<&mut Pathing>,
        now: u32,
    ) {
        if moved_far(me.position, at) {
            self.last_move = now;
        }
        me.position = at;
        world.set_position(me.id, at);
        if let (Some(pathing), Some(p)) = (pathing, world.get(me.id)) {
            pathing.moved(me.id, p, self.is_moving());
        }
    }

    /// Object slot 185 (`0x1411583d0`): the unit has velocity data whose velocity is longer than 0.
    pub fn is_moving(&self) -> bool {
        self.velocity.as_ref().is_some_and(|v| {
            let [x, y, z] = v.velocity;
            0.0 < fmath::sqrt(x * x + y * y + z * z)
        })
    }

    /// The motion step's path request for the level it walks, in state 2 (`0x141150f00`, its
    /// `_Memory[10] == 2` block): unless the unit is in range of its target (state 4) or on its
    /// waypoint, the level spends one of its tries and a new level gets a path to the waypoint:
    /// the short pather's (`FUN_140f9e540`) for a near waypoint, else or when that fails the tile
    /// pathfinder's (`FUN_140f7c530`; a top level's plan becomes a level of its own, whose
    /// stretches the next pass serves). The new level goes under the walked one, which waits
    /// (state 0). The long pather (`FUN_140f9eb70`), the close-in path (`FUN_141142750`), the
    /// escape from a game type 12 tile, a trade cart's budget and the exits that stop the unit
    /// (slot 0x5d0) are not modelled.
    fn request(
        &mut self,
        me: &Me,
        world: &Spatial,
        plan: &mut Planning<'_>,
        goal: Goal<'_>,
        plans: &mut u32,
    ) -> Result<Step, Unsupported> {
        let Goal {
            target,
            gap,
            to_end,
        } = goal;
        let position = vector(me.position);
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
        let waypoint = level.point(level.waypoint)?;
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
            if trade_cart {
                return Err(Unsupported("a trade cart's path budget (FUN_14165d6f0)"));
            }
            if PLANS <= *plans {
                return Err(Unsupported(
                    "a fourth plan in one motion step stops the unit",
                ));
            }
            // the owner's count of plans (player +0x67c) only limits trade carts
            *plans += 1;
            self.wait_delays = 0;
        }
        // the level spends a try unless its top path loops
        let top_loops = self
            .paths
            .last()
            .is_some_and(|t| !t.points.is_empty() && t.path_flags & 1 != 0);
        if !top_loops && !self.attack_moving {
            let top_has_more = self.paths.last().is_some_and(PathData::has_more);
            let level = &mut self.paths[0];
            level.continue_counter -= 1;
            if level.continue_counter < 1 {
                if 1 < level.level || !top_has_more {
                    if level.level == 2 && level.waypoint < level.points.len() as i32 - 1 {
                        level.flags |= 1;
                        level.continue_counter = CONTINUE;
                        return Ok(Step::Again);
                    }
                    return Err(Unsupported("a level out of tries stops the unit"));
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
        if !solid {
            let level = &self.paths[0];
            path.insert(0, position);
            let to = if level.waypoint < level.points.len() as i32 {
                level.point(level.waypoint)?
            } else {
                position
            };
            path.push(to);
            path.measure();
            found = true;
        } else {
            let way = ground(waypoint, position);
            // FUN_141158210 is 1 only on an `E` tile in game type 12
            if plan.pathing.game_type == 12 {
                return Err(Unsupported(
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
                            let ignore = self.ignore_list(me, world, target, target_id)?;
                            let r = self.short_path(
                                me,
                                world,
                                plan,
                                waypoint,
                                short_range(near, range),
                                target_id,
                                &ignore,
                                &mut path,
                            )?;
                            path.measure();
                            new.created = plan.now;
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
                    path.insert(0, position);
                    path.push(waypoint);
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
                    let ignore = self.ignore_list(me, world, target, target_id)?;
                    let r = self.short_path(
                        me,
                        world,
                        plan,
                        waypoint,
                        short_range(near, range),
                        target_id,
                        &ignore,
                        &mut path,
                    )?;
                    path.measure();
                    new.created = plan.now;
                    let blocked = r == 3
                        || r == 4
                        || path.length <= EPSILON
                            && (r != 0 || (1 < path.points.len() && EPSILON <= path.length));
                    if blocked {
                        if LONG < to_end && to_end != FAR {
                            return Err(Unsupported("a blocked unit's notice (FUN_140f8a230)"));
                        }
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
                                return Err(Unsupported(
                                    "a new path in range stops the unit (slot 0x5d0)",
                                ));
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
                } else {
                    if way < CLOSE || gap < CLOSE {
                        return Err(Unsupported("the close-in path (FUN_141142750)"));
                    }
                    true
                }
            } else {
                too_short(&path)
            };
            if tail && LONG_PATHER && (trade_cart || SHORT_REACH < way) {
                // LAB_141152ec1: a far waypoint takes the long pather
                match target {
                    None if CLOSE <= way => {
                        return Err(Unsupported("the long pather (FUN_140f9eb70)"));
                    }
                    Some(_) if CLOSE <= gap => {
                        return Err(Unsupported("the long pather (FUN_140f9eb70)"));
                    }
                    Some(_) => {
                        return Err(Unsupported("a near target's master test (0x60c9)"));
                    }
                    None => {}
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
                                path.push(p);
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
        }
        if path.points.len() < 2 || path.length < EPSILON {
            return self.no_path(found, target, to_end);
        }
        Ok(self.link(new, path, found, partial, planned, solid))
    }

    /// Whether the unit's restriction has a planner (`FUN_140f7cb30`); the slice builds one for
    /// any restriction its data has.
    fn planner_exists(&self, world: &Spatial, plan: &mut Planning<'_>) -> bool {
        plan.pathing
            .planner(self.stats.restriction, world.tiles())
            .is_some()
    }

    /// `FUN_14114d3d0`: the walked level's goal is in reach: its target's gap (else where its top
    /// path ends) within its range, or a target's last waypoint at most 16 tiles beyond its
    /// range on a top level.
    fn near_goal(&self, way: f32, gap: f32, to_end: f32) -> bool {
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

    /// The new level's path is no path (`0x141150f00` after the plans, fewer than two points or
    /// no length): the level marks its waypoint reached and the step starts again, or it waits at
    /// the end of its path (state 5); the exits that stop or pause the unit are not modelled.
    fn no_path(
        &mut self,
        found: bool,
        target: Option<&Placed>,
        to_end: f32,
    ) -> Result<Step, Unsupported> {
        let top_has_more = self.paths.last().is_some_and(PathData::has_more);
        if target.is_some() && self.paths[0].flags & 4 != 0 {
            // FUN_140f9b4d0(level, 2, 0): bit 2 goes off the level and on for its parents
            self.paths[0].flags &= !4;
            for parent in &mut self.paths[1..] {
                parent.flags |= 4;
            }
        }
        let level = &mut self.paths[0];
        if !found || level.range < to_end {
            if level.points.len() as i32 - 1 <= level.waypoint {
                if 0 < level.continue_counter {
                    return Err(Unsupported(
                        "a level at its last waypoint without a path pauses the unit",
                    ));
                }
                if level.path_flags & 1 != 0 || self.attack_moving {
                    return Err(Unsupported(
                        "a looping level without a path pauses the unit",
                    ));
                }
                if 1 < level.level || !top_has_more {
                    return Err(Unsupported("a level without a path stops the unit"));
                }
                level.state = 5;
                return Ok(Step::Again);
            }
        } else if level.path_flags & 1 == 0 {
            return Err(Unsupported("a found path without length stops the unit"));
        }
        level.flags |= 1;
        Ok(Step::Again)
    }

    /// The new level goes under the walked one (`0x141150f00` from `0x141153b0c`): the walked
    /// level waits (state 0, its tries back to 10). A top level's plan becomes a level of its own
    /// in state 2 (`FUN_140f9b800`, level 1), whose stretches the step serves next; any other path
    /// is walked at once (`FUN_140f9b7c0`, level 0, state 3) after its long stretches got a
    /// midpoint each (`FUN_140f9b8f0`), linked to the walked level as found (1) or not (2).
    fn link(
        &mut self,
        mut new: PathData,
        mut path: Path,
        found: bool,
        partial: bool,
        planned: bool,
        solid: bool,
    ) -> Step {
        let level = &mut self.paths[0];
        level.state = 0;
        level.continue_counter = CONTINUE;
        new.range = level.range;
        new.target = level.target;
        new.waypoint = 1;
        if level.level == 2 && planned {
            new.length = path.length;
            new.points = path.points;
            new.state = 2;
            new.level = 1;
            new.linked_type = if found { 1 } else { 2 };
            self.paths.insert(0, new);
            return if SHORT_PATHER || LONG_PATHER {
                Step::Again
            } else {
                Step::Walk
            };
        }
        let mut i = path.points.len() as i32 - 1;
        while 0 < i {
            let (a, b) = (path.points[i as usize], path.points[i as usize - 1]);
            let mut d = fmath::sub(a, b);
            d[1] = 0.0;
            if SPLIT < d[0] * d[0] + 0.0 + d[2] * d[2] {
                split(&mut path, i - 1);
            }
            i -= 1;
        }
        new.length = path.length;
        new.points = path.points;
        new.state = 3;
        new.level = 0;
        new.linked_type = if !found || partial { 2 } else { 1 };
        if !solid {
            // the walked level lets its path go (FUN_140f95b70)
            self.paths[0].points.clear();
        }
        self.paths.insert(0, new);
        Step::Walk
    }

    /// The objects the short pather ignores (`FUN_14114ece0`): the unit, its target, and with a
    /// target the allied workers about to work on it near a standing unit, which the slice does
    /// not model (it stops when any object is that near). The unit's own list (`+0x670`, for a
    /// top path made this update) is empty in the slice.
    fn ignore_list(
        &self,
        me: &Me,
        world: &Spatial,
        target: Option<&Placed>,
        target_id: i32,
    ) -> Result<Vec<i32>, Unsupported> {
        let mut list = vec![me.id];
        if target.is_some() {
            list.push(target_id);
            if !self.is_moving() {
                let grow = self.stats.radius[0] + RANGE_FLOOR;
                let [x, y, _] = me.position;
                let rect = [x, y, x, y];
                world.check_known([x - grow, y - grow, x + grow, y + grow])?;
                if !world.obstruction_candidates(rect, grow, &list).is_empty() {
                    return Err(Unsupported(
                        "the ignore list's workers near the unit (FUN_14114ece0)",
                    ));
                }
            }
        }
        Ok(list)
    }

    /// The short pather (`FUN_140f9e540`) from the unit to `to`, its points appended to `out`;
    /// returns its result (0 in range, 1 reached, 2 partial, 3 blocked, 4 invalid).
    #[allow(clippy::too_many_arguments)]
    fn short_path(
        &self,
        me: &Me,
        world: &Spatial,
        plan: &mut Planning<'_>,
        to: Vec3,
        range: f32,
        target: i32,
        ignore: &[i32],
        out: &mut Path,
    ) -> Result<u8, Unsupported> {
        let [x, y, _] = me.position;
        let target = match target {
            -1 => None,
            id => world.get_known(id)?.map(|t| short_path::Target {
                id,
                position: [t.position[0], t.position[1]],
                half_size: t.radius,
            }),
        };
        let listed = ignore
            .iter()
            .map(|&id| Listed {
                id,
                shape: plan.pathing.shape(id),
            })
            .collect();
        let call = Call {
            from: [x, y],
            to: [to[0], to[2]],
            radius: self.stats.radius[0],
            range,
            target,
            listed,
            map: world.map,
            mask: self.stats.restriction_mask,
        };
        // the search's box: four tiles around its ends
        let reach = 4.0 + self.stats.radius[0];
        world.check_known([
            x.min(to[0]) - reach,
            y.min(to[2]) - reach,
            x.max(to[0]) + reach,
            y.max(to[2]) + reach,
        ])?;
        match call.entry() {
            Entry::InRange => {
                out.points.clear();
                Ok(0)
            }
            Entry::Invalid => Ok(4),
            Entry::Search { query, ignore } => {
                let tiles = world.tiles();
                let words = |x: u32, y: u32| tiles.word(x as i32, y as i32).unwrap_or(0);
                let (mesh, pather) = plan
                    .pathing
                    .mesh_and_pather()
                    .ok_or(Unsupported("a short path without the game's mesh"))?;
                let found = pather.search(mesh, &words, &query, &ignore)?;
                for p in short_path::to_path(&found.points).points {
                    out.push(p);
                }
                Ok(found.result)
            }
        }
    }

    /// The tile pathfinder (`FUN_140f7c530`) from the unit to `to`, appended to `out`, smoothed
    /// with the obstruction manager's segment test for the unit (its own and its target's records
    /// ignored); None without a planner. A target with an obstruction record is not modelled (the
    /// planner reads its record's first byte).
    #[allow(clippy::too_many_arguments)]
    fn tile_plan(
        &self,
        me: &Me,
        world: &Spatial,
        plan: &mut Planning<'_>,
        to: Vec3,
        radii: f32,
        range: f32,
        target: Option<&Placed>,
        out: &mut Path,
    ) -> Result<Option<u8>, Unsupported> {
        if target.is_some_and(|t| t.obstruction.is_some()) {
            return Err(Unsupported("a plan to a target with an obstruction record"));
        }
        let [x, y, _] = me.position;
        let req = Request {
            from: [x, y],
            to: [to[0], to[2]],
            radii,
            range,
            closed: Vec::new(),
            full: !plan.computer,
        };
        let mut ignore = vec![me.id];
        if let Some(t) = self.paths.first().map(|l| l.target).filter(|&t| t != -1) {
            ignore.push(t);
        }
        let (radius, mask) = (self.stats.radius[0], self.stats.restriction_mask);
        let Some(planner) = plan.pathing.planner(self.stats.restriction, world.tiles()) else {
            return Ok(None);
        };
        let mut blocked =
            |a: [f32; 2], b: [f32; 2]| Ok(world.segment_blocked(a, b, radius, mask, &ignore));
        let found = planner.plan(&req, false, out, Some(&mut blocked))?;
        Ok(Some(found.code()))
    }

    /// The path of the top level (`FUN_140f960f0`): the last level's, or the walked level's when
    /// it has no parent; None when that level holds no path.
    fn top_path(&self) -> Option<&Vec<Vec3>> {
        self.paths
            .last()
            .map(|l| &l.points)
            .filter(|p| !p.is_empty())
    }

    /// The path request of a MoveTo (`FUN_14114e410`, without waypoints of its own): one level of
    /// path data, the top (level 2), whose path runs from the unit to the goal (`x, y, z` on the
    /// map) unless the two are less than 1e-7 apart on the ground, in state 2: the next motion step
    /// plans the path to walk. A unit with future path data (`+0x860`) or waypoints of its own
    /// (`+0x7fc`) goes another way, which the slice does not model (it holds neither). The path is
    /// made `now`.
    pub fn request_path(
        &mut self,
        from: [f32; 3],
        goal: [f32; 3],
        range: f32,
        target: i32,
        now: u32,
    ) {
        let start = vector(from);
        let end = vector(goal);
        let mut points = vec![start];
        let mut d = fmath::sub(end, start);
        d[1] = 0.0;
        if EPSILON < fmath::sqrt(d[0] * d[0] + 0.0 + d[2] * d[2]) {
            points.push(end);
        }
        // a new path's flags are 6 (FUN_140f8f5d0); the request clears bits 0 and 1
        let path_flags = 6 & !1 & !2;
        self.paths = vec![PathData {
            level: 2,
            points,
            path_flags,
            waypoint: 1,
            state: 2,
            range,
            target,
            ..PathData::new(now)
        }];
    }

    /// The unit vector on the ground from `from` to `to` (slot 194, `0x14114f210`); the forward
    /// vector when the two are less than 1e-7 apart or on the same point.
    fn direction_to(&self, from: Vec3, to: Vec3) -> Vec3 {
        if from[0] != to[0] || from[2] != to[2] {
            let x = to[0] - from[0];
            let z = to[2] - from[2];
            let length = fmath::sqrt(x * x + 0.0 + z * z);
            if EPSILON <= length {
                return [x / length, 0.0, z / length];
            }
        }
        self.forward
    }

    /// Where this tick's speed would take the unit along `direction` (slot 199, `0x141161860`):
    /// the forward speed after one more step of acceleration, capped by the top speed.
    fn projected(&self, position: Vec3, direction: Vec3, dt: f32) -> Vec3 {
        let Some(v) = &self.velocity else {
            return position;
        };
        let mut next = fmath::add(v.velocity, fmath::scale(dt, v.acceleration));
        if self.stats.speed < next[2] || next[2] <= 0.0 {
            next[2] = self.stats.speed;
        }
        fmath::add(position, fmath::scale(dt * next[2], direction))
    }

    /// The obstruction query's rectangle (`FUN_14114cc60`): the step's two ends, each held within
    /// 0.1 of the map's far edges, then ordered (`FUN_141051900`).
    fn query_rect(&self, from: Vec3, to: Vec3, map: [i32; 2]) -> [f32; 4] {
        let far = |size: i32| size as f32 - 0.1;
        let hold = |v: f32, max: f32| if v < 0.0 { 0.0 } else { v.min(max) };
        let (x0, y0) = (hold(from[0], far(map[0])), hold(from[2], far(map[1])));
        let (x1, y1) = (hold(to[0], far(map[0])), hold(to[2], far(map[1])));
        [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)]
    }

    /// Turn toward `direction` (slot 195, `0x14114f630`, then the yaw, slot 184, `0x141163140`):
    /// by the angle between the flat forward vector and the direction, at most the turn rate times
    /// dt, about the up axis; then the forward vector and its right are renormalised (slot 173,
    /// `0x14115d150`, which rebuilds the right vector from up and forward).
    fn steer(&mut self, direction: Vec3, dt: f32) {
        let mut flat = self.forward;
        flat[1] = 0.0;
        let flat = fmath::normalize(flat);
        let angle = fmath::angle_between(direction, flat);
        let most = self.stats.turn_rate * dt;
        let mut turn = if angle <= most { angle } else { most };
        if 0.0 < fmath::cross(direction, flat)[1] {
            turn = -turn;
        }
        if self.paths[0].flags & 1 != 0 && turn == 0.0 {
            return;
        }
        let m = fmath::rotation(turn, UP);
        let forward = fmath::normalize(fmath::rotate(&m, self.forward));
        self.forward = forward;
        self.right = fmath::normalize(fmath::cross(UP, forward));
    }

    /// The speed control toward the waypoint (slot 198, `0x141150a80`): accelerate to the top speed
    /// unless the waypoint lies inside the turning circle, where the unit brakes.
    fn control_speed(&mut self, position: Vec3, waypoint: Vec3) {
        let Some(v) = self.velocity.as_mut() else {
            self.velocity = Some(Velocity {
                velocity: [0.0; 3],
                acceleration: [0.0, 0.0, f32::MAX],
            });
            return;
        };
        let d = fmath::sub(waypoint, position);
        let length = fmath::sqrt(d[1] * d[1] + d[0] * d[0] + d[2] * d[2]);
        let toward = [d[0] / length, d[1] / length, d[2] / length];
        let angle = fmath::angle_between(self.forward, toward);
        let radius = (self.stats.turn_radius * v.velocity[2]) / self.stats.turn_radius_speed;
        if 0.0 < v.velocity[2] && EPSILON < angle.abs() {
            let mut side = self.right;
            if 0.0 < fmath::cross(toward, self.forward)[1] {
                side = side.map(|c| -c);
            }
            let centre = [
                side[0] * radius + position[0],
                side[1] * radius + position[1],
                side[2] * radius + position[2],
            ];
            if fmath::length(fmath::sub(waypoint, centre)) < radius {
                v.acceleration[2] = -f32::MAX;
                return;
            }
        }
        if self.stats.speed <= v.velocity[2] {
            return;
        }
        v.acceleration[2] = f32::MAX;
    }
}

/// How a step relates to its waypoint (slot 196, `0x14114f400`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reach {
    /// 0: the step ends short of it.
    Short,
    /// 1: the step reaches or passes it.
    Reached,
    /// 2: the unit moves away from it.
    Away,
}

/// Compare the distances on the ground: waypoint to position, waypoint to the step's end, and the
/// step's own length.
fn reach(waypoint: Vec3, position: Vec3, projected: Vec3) -> Reach {
    let mut now = fmath::sub(waypoint, position);
    let mut after = fmath::sub(waypoint, projected);
    let step = fmath::sub(projected, position);
    now[1] = 0.0;
    after[1] = 0.0;
    let (now, after, step) = (
        fmath::length(now),
        fmath::length(after),
        fmath::length(step),
    );
    if EPSILON <= after {
        if step <= now {
            return Reach::Short;
        }
        if now < after {
            return Reach::Away;
        }
    }
    Reach::Reached
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spatial::{Obstruction, Placed};

    const DT: f32 = 0.02;

    fn stats() -> MoveStats {
        MoveStats {
            speed: 0.8,
            turn_rate: f32::MAX,
            turn_radius: 0.0,
            turn_radius_speed: f32::MAX,
            radius: [0.2, 0.2, 0.5],
            restriction: 7,
            restriction_mask: 6,
            turns_at_once: true,
            move_algorithm: 0,
        }
    }

    fn level(level: u8, points: &[[f32; 2]], state: i32) -> PathData {
        PathData {
            level,
            linked_type: if level == 0 { 1 } else { 0 },
            points: points.iter().map(|&[x, y]| [x, 0.0, y]).collect(),
            path_flags: if level == 0 { 6 } else { 4 },
            waypoint: 1,
            state,
            range: 0.1,
            target: 10028,
            pause_time: 0.0,
            flags: 0,
            length: 0.0,
            created: 16360,
            continue_counter: 10,
            repath_time: 0.0,
        }
    }

    /// MarvinBlue/426048731's builder 9177 as the game saved it after tick 679 (update 680): its
    /// first path toward the house foundation 10028, facing along it, not yet moving.
    fn builder_9177() -> Mover {
        Mover {
            forward: [0.8944272, 0.0, -0.44721362],
            right: [-0.44721362, 0.0, -0.8944272],
            velocity: Some(Velocity {
                velocity: [0.0; 3],
                acceleration: [0.0, 0.0, f32::MAX],
            }),
            paths: vec![
                level(0, &[[28.0, 89.0], [30.0, 88.0]], 3),
                level(2, &[[28.0, 89.0], [30.0, 88.0]], 0),
            ],
            factor: 1.0,
            stats: stats(),
            waiting: 0,
            wait_delays: 0,
            attack_moving: false,
            last_move: 0,
            substitute_count: 0,
        }
    }

    fn world() -> Spatial {
        let mut s = Spatial::new([120, 120]);
        let unit = |x, y| Placed {
            position: [x, y, 0.0],
            radius: [0.2, 0.2],
            class: 4,
            player: 2,
            master: 0,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: Some(Obstruction { kind: 5, class: 2 }),
        };
        s.insert(9175, unit(30.0, 89.0));
        s.insert(9177, unit(28.0, 89.0));
        s.insert(
            10028,
            Placed {
                position: [30.0, 88.0, 0.0],
                radius: [1.0, 1.0],
                class: 3,
                player: 2,
                master: 0,
                obstruction_type: 0,
                buildable_on: 0,
                tall: true,
                speed: 0.0,
                unknown: None,
                obstruction: None,
            },
        );
        s
    }

    /// One tick of the walker: the velocity step, then the motion step from where it got.
    fn tick(m: &mut Mover, at: [f32; 3], world: &mut Spatial) -> ([f32; 3], Motion) {
        let at = m.step_velocity(at, DT, world.map, 0);
        world.set_position(9177, at);
        let motion = m
            .motion(
                &mut Me {
                    id: 9177,
                    position: at,
                },
                DT,
                world,
                None,
            )
            .unwrap();
        (at, motion)
    }

    #[test]
    fn the_first_step_reaches_top_speed_and_moves_along_the_saved_forward_vector() {
        let mut m = builder_9177();
        let mut w = world();
        let (at, motion) = tick(&mut m, [28.0, 89.0, 0.0], &mut w);
        // the tick oracle's positions after update 681 (snapshot 680)
        assert_eq!(at, [28.01431, 88.99284, 0.0]);
        assert_eq!(motion, Motion::Moving);
        assert_eq!(m.velocity.as_ref().unwrap().velocity, [0.0, 0.0, 0.8]);
        assert_eq!(
            m.velocity.as_ref().unwrap().acceleration,
            [0.0, 0.0, f32::MAX]
        );
        assert_eq!(m.forward, [0.8944272, 0.0, -0.44721362]);
    }

    #[test]
    fn the_second_tick_turns_toward_the_waypoint_from_where_the_first_step_ended() {
        let mut m = builder_9177();
        let mut w = world();
        let (at, _) = tick(&mut m, [28.0, 89.0, 0.0], &mut w);
        let (at, _) = tick(&mut m, at, &mut w);
        assert_eq!(at, [28.028622, 88.98569, 0.0]);
        // after update 682 the game's forward vector points from there to (30, 88)
        assert_eq!(m.forward, [0.89458156, 0.0, -0.44690478]);
        assert_eq!(m.right, [-0.44690478, 0.0, -0.89458156]);
        let (at, _) = tick(&mut m, at, &mut w);
        assert_eq!(at, [28.042934, 88.97854, 0.0]);
    }

    #[test]
    fn the_gap_is_the_distance_between_the_footprints() {
        // MarvinBlue/426048731: a house at (30, 88), builders at (30, 89), (28, 89), (29, 99)
        let (villager, house) = ([0.2, 0.2], [1.0, 1.0]);
        let gap = |x, y| footprint_gap([x, y, 0.0], villager, [30.0, 88.0, 0.0], house);
        assert_eq!(gap(30.0, 89.0), 0.0);
        assert_eq!(gap(28.0, 89.0), 2.0f32 - 1.2);
        assert_eq!(gap(29.0, 99.0), (11.0f32 - 1.2).hypot(0.0));
    }

    /// MarvinBlue/426048731's hawk 9926 after tick 679: a flight to a point, no range.
    #[test]
    fn a_walk_to_a_point_goes_on_until_its_range() {
        let mut hawk = builder_9177();
        for l in &mut hawk.paths {
            l.target = -1;
            l.range = 0.0;
        }
        let mut me = Me {
            id: 9926,
            position: [28.0, 89.0, 0.0],
        };
        assert_eq!(
            hawk.motion(&mut me, DT, &mut Spatial::new([120, 120]), None),
            Ok(Motion::Moving)
        );
        // a deer's walk with range 0.5 whose top path ends within it
        let mut deer = builder_9177();
        for l in &mut deer.paths {
            l.target = -1;
            l.range = 0.5;
        }
        deer.paths[1].points = vec![[28.0, 0.0, 89.0], [28.6, 0.0, 89.0]];
        deer.velocity.as_mut().unwrap().velocity = [0.0, 0.0, 0.8];
        assert_eq!(
            deer.motion(&mut me, DT, &mut Spatial::new([120, 120]), None),
            Ok(Motion::Arrived)
        );
        // the walk ends: one level, reset, and the deer stops
        assert_eq!(deer.paths.len(), 1);
        assert_eq!(
            (
                deer.paths[0].state,
                deer.paths[0].target,
                &deer.paths[0].points
            ),
            (0, -1, &vec![])
        );
        assert_eq!(
            deer.velocity
                .as_ref()
                .map(|v| (v.velocity[2], v.acceleration[2])),
            Some((0.0, 0.0))
        );
    }

    /// MarvinBlue/426048731's 9175 in update 724..726: the step that reaches its waypoint marks it
    /// reached; the next motion step puts the unit onto it, the level arrives and hands over to the
    /// top level, which arrives too: the walk ends.
    #[test]
    fn a_reached_waypoint_ends_the_walk_at_the_end_of_the_path() {
        let mut m = builder_9177();
        m.paths[0].points = vec![[30.0, 0.0, 89.0], [29.5, 0.0, 89.5]];
        m.paths[1].points = vec![[30.0, 0.0, 89.0], [29.5, 0.0, 89.5]];
        for l in &mut m.paths {
            l.target = -1;
            l.range = 0.0;
        }
        m.forward = [
            -std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
        ];
        m.velocity.as_mut().unwrap().velocity = [0.0, 0.0, 0.8];
        let mut world = Spatial::new([120, 120]);
        let mut me = Me {
            id: 9175,
            position: [29.51, 89.49, 0.0],
        };
        assert_eq!(m.motion(&mut me, DT, &mut world, None), Ok(Motion::Moving));
        assert_eq!(m.paths[0].flags, 3);
        assert_eq!(m.motion(&mut me, DT, &mut world, None), Ok(Motion::Arrived));
        assert_eq!(me.position, [29.5, 89.5, 0.0]);
        assert_eq!(m.paths.len(), 1);
        assert_eq!(m.paths[0].state, 0);
    }

    /// MarvinBlue/426048731's 9175 in update 682: the path request of its MoveTo off the house
    /// footprint, which the game saves as the top level of its path data.
    #[test]
    fn a_path_request_makes_the_top_level_from_the_unit_to_its_goal() {
        let mut m = builder_9177();
        m.request_path([30.0, 89.0, 0.0], [29.5, 89.5, 0.0], 0.0, -1, 16400);
        assert_eq!(m.paths.len(), 1);
        let top = &m.paths[0];
        assert_eq!(
            (
                top.level,
                top.path_flags,
                top.waypoint,
                top.state,
                top.target
            ),
            (2, 4, 1, 2, -1)
        );
        assert_eq!(top.points, vec![[30.0, 0.0, 89.0], [29.5, 0.0, 89.5]]);
        // the motion step then plans the path to walk, which the slice does not model
        let mut me = Me {
            id: 9175,
            position: [30.0, 89.0, 0.0],
        };
        assert_eq!(
            m.motion(&mut me, DT, &mut Spatial::new([120, 120]), None),
            Err(Unsupported("a path request without the world's planners"))
        );
    }

    #[test]
    fn a_unit_with_no_speed_or_no_path_is_done() {
        let mut m = builder_9177();
        m.stats.speed = 0.0;
        let mut me = Me {
            id: 9177,
            position: [28.0, 89.0, 0.0],
        };
        assert_eq!(m.motion(&mut me, DT, &mut world(), None), Ok(Motion::Done));
        let mut m = builder_9177();
        m.paths.clear();
        assert_eq!(m.motion(&mut me, DT, &mut world(), None), Ok(Motion::Done));
    }

    #[test]
    fn a_walker_that_may_meet_an_obstruction_is_not_modelled() {
        let mut m = builder_9177();
        let mut w = world();
        let mut close = w.get(9175).unwrap().clone();
        close.position = [28.3, 88.9, 0.0];
        w.insert(9175, close);
        let mut me = Me {
            id: 9177,
            position: [28.0, 89.0, 0.0],
        };
        assert!(m.motion(&mut me, DT, &mut w, None).is_err());
    }
}
