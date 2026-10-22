//! A unit's movement (`RGE_Moving_Object`, MovingObject.cpp in 158041): the velocity step that
//! moves it along its forward vector, and the motion step that steers it along its path.
//!
//! Per tick, `RGE_Moving_Object::update` (`0x14114dbb0`) runs first: it brings the forward speed
//! toward the acceleration's target and moves the unit `dt × speed` along the forward vector it had
//! after the last tick ([`Mover::step_velocity`]). The actions run later in the same update; a
//! `MoveTo` calls the motion step (object slot 193, `0x141150f00`, [`Mover::motion`]), which turns the
//! unit toward its waypoint and sets the acceleration for the next tick.
//!
//! Vectors are the game's `BVector`s ([`fmath::Vec3`]: x, height, y).

use crate::fmath::{self, Vec3};
use crate::hull_path::HullPather;
use crate::pathing::{Pathing, PlanBudget};
use crate::rng::Well512;
use crate::short_path::{self, Call, Entry, Listed};
use crate::spatial::{Placed, Spatial};
use crate::tile_path::{Path, Request};
use crate::unit_ai::Unsupported;

/// The velocity data (`+0x868`): velocity and acceleration in the unit's own frame; component 2 is
/// along the forward vector.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Velocity {
    /// The velocity.
    pub velocity: Vec3,
    /// The acceleration.
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
    /// The level holds a path (`+0x18`, or the one the world's table finds by its id `+0x20`): a
    /// reset level (id −1) holds none, nor does a bird's top level, and the motion step ends at
    /// it.
    pub has_path: bool,
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
    /// The path's hash set of ignored units (`+0x18`, `FUN_140f96120`), which the save writes as
    /// `numberIgnoreUnits` and the ids after it (`0x140f98f00`); empty in this game (the probe
    /// `path_sets.py`).
    pub ignore: Vec<i32>,
    /// The path's ignore units (`+0x450`, the ids from `+0x454`), which the walk's avoidance reads
    /// (`FUN_140f98d20`): a short path made in the tick its top path was made takes the unit's
    /// own (`+0x670`, `FUN_140f9b7a0`). The save does not hold them: a loaded path takes them
    /// back by that rule (the oracle's import).
    pub ignored: Vec<i32>,
}

/// What movement reads of the unit's master.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveStats {
    /// The master's `fly_mode` (`+0x608`): how its place takes the ground's height
    /// ([`Mover::grounded`]).
    pub fly_mode: u8,
    /// The top speed (object slot 65, `0x141202c70` -> `FUN_14114d1d0`): the master's speed
    /// (`+0x740`), times the current sprite's speed factor when that is above 0.
    pub speed: f32,
    /// The top speed with the master's walking sprite (`+0x748`) and with its running one
    /// (`+0x750`, the walking one without it): a `MoveTo` that runs or walks shows one, and the
    /// unit moves at its speed.
    pub walk_speed: f32,
    /// The running speed.
    pub run_speed: f32,
    /// The top speed with the master's standing sprite (`+0x470`), which a non-quiet `MoveTo`'s end
    /// (`set_state` 1 or 13, slot 0x68 with slot 0x798's) and a stop in state 2 show.
    pub stand_speed: f32,
    /// Master `+0x774`: the most the unit turns per second ([`turn_rate`]).
    pub turn_rate: f32,
    /// Master `+0x76c` and `+0x770`: the turn radius is `turn_radius × speed / turn_radius_speed`
    /// (0 and the largest float for every master the game copies, [`turn_rate`]).
    pub turn_radius: f32,
    /// `+0x770`: the speed at which the turn radius is its value.
    pub turn_radius_speed: f32,
    /// Master `+0x5d8`, `+0x5dc`, `+0x5e0`.
    pub radius: [f32; 3],
    /// The master's terrain restriction (`+0x606`) and the terrain classes it lets the unit stand
    /// on (its mask).
    pub restriction: usize,
    /// The terrain classes the restriction lets it stand on (its mask).
    pub restriction_mask: u8,
    /// The master's turn time (`+0x758`, the `.dat`'s `rotation_speed`): the seconds between the
    /// facet steps of a unit facing a target; 0 faces at once (`FUN_141162db0`).
    pub turn_time: f32,
    /// The master's move algorithm (`+0x768`, the `.dat`'s `move_algorithm`): bit 0 has a stop
    /// reset the unit's sprite (slot 0x5d8).
    pub move_algorithm: u8,
    /// Master `+0x780`, `min_collision_size_multiplier`: the walk's query grows moving allies by
    /// the unit's radius times this.
    pub min_collision: f32,
    /// The master's object flags (`+0x654`) have bit 1: the unit turns a hundred times faster.
    pub fast_turn: bool,
    /// The facets the facing routines read of the unit's current sprite: the master's, when
    /// every sprite it shows has the same count and bit 1 (None otherwise, or without sprites).
    pub facets: Option<crate::facing::SpriteFacets>,
    /// The master's weapon range (slot 0x230, `+0x828`; 0 for an object without combat values),
    /// which the drop-off walker's start test reads (`FUN_140fa0110`).
    pub weapon_range: f32,
}

/// A moving unit's movement state.
#[derive(Clone, Debug, PartialEq)]
pub struct Mover {
    /// `+0x830`, a unit vector on the ground.
    pub forward: Vec3,
    /// `+0x83c`.
    pub right: Vec3,
    /// Its velocity data, when it has any.
    pub velocity: Option<Velocity>,
    /// The path data, the level the unit walks first, each next one its parent.
    pub paths: Vec<PathData>,
    /// `+0x878`: moveForward's distance factor.
    pub factor: f32,
    /// What its master gives its movement.
    pub stats: MoveStats,
    /// `+0x7f9`, `waitingToMoveValue`.
    pub waiting: u8,
    /// `+0x7fa`, `numberWaitDelays`: plans that failed for want of budget (trade carts).
    pub wait_delays: u8,
    /// `+0x7fb`, `mAttackMoving`: a level spends no try, and one without a path pauses.
    pub attack_moving: bool,
    /// `+0x870`, `lastMoveTimeValue`: the world's time when the unit last moved.
    pub last_move: u32,
    /// `+0x824`..`+0x82c`, `positionValue`: where it last moved to (x, y, height), which the
    /// position setter compares the new place with (`FUN_141161950`).
    pub moved_at: [f32; 3],
    /// `+0x874`, `mConsecutiveSubstituteCount`: above 2 a walk counts its waypoint reached.
    pub substitute_count: i32,
    /// The unit's ignore list (`+0x670`, count `+0x678`; the save's `numberUnits` and `tempInt`
    /// of the static part): the other units of an order to several (`FUN_14118c790`), which the
    /// short pather ignores for a top path made this update. A stop empties it.
    pub ignore_units: Vec<i32>,
    /// `+0x7ec`, `turn_towards_time`: the world time in seconds of its last facet step toward a
    /// target (`FUN_141162db0`).
    pub last_turn: f32,
    /// `+0x7e8`: where the unit faces, radians 0 to 2π ([`crate::facing`]).
    pub angle: f32,
    /// The point of the `MoveTo` the unit walks with (`+0x38`..`+0x40`), when it walks to a target
    /// object: where the target stood when the `MoveTo` last asked for a path, which its check of
    /// the target reads ([`crate::move_to::target_moved`]). A stop leaves it (the motion step's
    /// pause stops a unit whose `MoveTo` goes on); a new `MoveTo` sets it.
    pub move_to_point: Option<[f32; 3]>,
    /// A stop (slot 0x5d0) cleared the object's animation speed (`+0x7b8`), which the unit
    /// holds: [`crate::turn::Unit`] takes it.
    pub speed_stopped: bool,
    /// The pauses its walk gave units it met (slot 0x5f0 on each: the met unit's walked level's
    /// `+0x34`), for the caller to hand on.
    pub pauses: Vec<(i32, f32)>,
    /// The kept hits its walk met of the walked level's target's master, of a player neither
    /// counts an ally: the motion step tells the unit AI of each (its slot 0x38 with
    /// [`crate::unit_ai::IN_THE_WAY`] and the hit's id), which the caller passes on.
    pub in_the_way: Vec<i32>,
    /// The facets of the unit's current sprite (`+0x140` -> `+0xa`, and its sequence flag bit
    /// 1), which the yaw and the facing read: the sprite the slice holds as the unit's update
    /// starts ([`crate::turn::Unit`]'s), or a new unit's standing sprite; None for neither (the
    /// master's [`MoveStats::facets`] then).
    pub sprite_facets: Option<crate::facing::SpriteFacets>,
}

/// The moving unit, as the motion step needs it besides its [`Mover`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Me {
    /// The unit.
    pub id: i32,
    /// x, y, z on the map.
    pub position: [f32; 3],
    /// Its top action's state (slot 0x2b0, `RGE_Action_Object::actionState`; 0 without one),
    /// which the short pather's ignore list reads: an action that walks after changing its state
    /// sets its own.
    pub action_state: u8,
    /// Its group (`mGroupID`, `+0x6d0`; -1 for none), whose differentiator units the long pather
    /// lowers (`FUN_140f9eb70`).
    pub group: i32,
}

/// What the motion step returns to the `MoveTo` action.
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
const EPSILON: f32 = crate::image::movement::EPSILON;
/// The world's up axis (`DAT_143cc2f30`), the yaw's axis.
const UP: Vec3 = [0.0, 1.0, 0.0];
/// How far inside the map's edge a moving unit's footprint stays (`DAT_142eb99cc`), and how far
/// below the edge a position stays (`DAT_142eb1e9c`).
const EDGE_GAP: f32 = crate::image::movement::EDGE_GAP;
const EDGE_POSITION_GAP: f32 = crate::image::movement::EDGE_POSITION_GAP;
/// A walk to a point checks its range only above this (`DAT_142eb5690`).
const RANGE_FLOOR: f32 = crate::image::movement::RANGE_FLOOR;
/// The class the motion step measures by its annexes (`-0xe85` encoded).
const GATE: i16 = 39;
/// A point within this many tiles is the short pather's to reach (`FUN_140fa0370`,
/// `DAT_142eb0f4c`).
const REACH_SHORT: f32 = crate::image::movement::REACH_SHORT;
/// A point whose squared gap to the unit's footprint is below this is not reached when the
/// short pather did not reach it (`FUN_140fa0370`, `DAT_14330eb8c`).
const REACH_NEAR: f32 = crate::image::movement::REACH_NEAR;
/// The master the long pather lets move without a speed (`FUN_140f9eb70`, raw id `0x2a`).
pub(crate) const LONG_PATH_MOVER: i16 = 42;
/// A target of this master (raw `0x60c9`) or class gives the tile planner of a reach test this
/// much more range (`FUN_140fa0370`, `DAT_142eb1c98`).
const WIDE_MASTER: i16 = 45;
const WIDE_CLASS: i16 = 33;
const WIDE_RANGE: f32 = crate::image::movement::WIDE_RANGE;
/// The trade cart's class (19), whose path requests the owner's budget limits (`FUN_14165d6f0`).
const TRADE_CART: i16 = 19;
/// The classes a long path's stretch may not cross (`0x141150f00` after `FUN_140f9eb70`, raw
/// `-0xe91`, `-0xe78`, `-0xea9`): walls, towers, and buildings that do not move; gates
/// ([`GATE`]) too. A villager's (`-0xea8`) path crosses them.
const WALL_CLASS: i16 = 27;
const TOWER_CLASS: i16 = 52;
const BUILDING_CLASS: i16 = 3;
const VILLAGER_CLASS: i16 = 4;
/// A long path's stretch is checked for walls with the unit's radius times this
/// (`DAT_142e85190`).
const LONG_PATH_GROW: f32 = crate::image::movement::LONG_PATH_GROW;
/// No distance (`DAT_14330ebf4`).
const FAR: f32 = crate::image::movement::FAR;
/// The motion step's switches (`DAT_1439e3cc0`, `DAT_1439e3cc4`, `DAT_1439e3cc8`): the short
/// pather, the tile pathfinder and the long pather run; all three are 1 at run time.
const SHORT_PATHER: bool = true;
const TILE_PATHER: bool = true;
const LONG_PATHER: bool = true;
/// The short pather serves a waypoint this near (`DAT_142ec9ec0`), and one this near a target
/// before the tile pathfinder (`DAT_142eb0f4c`).
const SHORT_REACH: f32 = crate::image::movement::SHORT_REACH;
const TARGET_SHORT_REACH: f32 = crate::image::movement::TARGET_SHORT_REACH;
/// A waypoint or target nearer than this takes the close-in path (`DAT_14330eb8c`).
const CLOSE: f32 = crate::image::movement::CLOSE;
/// A unit pushed into a met object counts beyond its radius times this (`DAT_142eb880c`).
const PUSHED: f32 = crate::image::movement::PUSHED;

/// `FUN_1411615a0`: how deep two boxes (centre, radii on the ground) overlap, the lesser of the
/// two axes; 0 when they do not.
fn overlap(a: Vec3, [ar, ad]: [f32; 2], b: Vec3, [br, bd]: [f32; 2]) -> f32 {
    let (ax1, ax0, ay0, ay1) = (a[0] + ar, a[0] - ar, a[2] - ad, a[2] + ad);
    let (by0, by1, bx0, bx1) = (b[2] - bd, b[2] + bd, b[0] - br, b[0] + br);
    // COMISS/JNC at 0x1411615fd..0x141161610: only an ordered `>=` leaves, a NaN goes on
    if bx0 >= ax1 || ax0 >= bx1 || by0 >= ay1 || ay0 >= by1 {
        return 0.0;
    }
    let x = fmath::minss(ax1, bx1) - fmath::maxss(ax0, bx0);
    let y = fmath::minss(ay1, by1) - fmath::maxss(ay0, by0);
    fmath::minss(x, y)
}

/// How far level 0's look ahead reaches on its parent's path, squared (`DAT_14330ebcc`).
const LOOK_AHEAD: f32 = crate::image::movement::LOOK_AHEAD;
/// `FUN_141142750` grows the walk's query by the unit's radius times `DAT_142e7f384`.
const STRAIGHT_GROW: f32 = crate::image::movement::STRAIGHT_GROW;
/// A path shorter than this is too short (`DAT_142eb99cc`).
const SHORTEST: f32 = crate::image::movement::SHORTEST;
/// The point `FUN_140f9bcd0` returns outside a path: `DAT_143f0dc88`, in `.bss` (zero at the
/// load; probed zero in records/425906646 with no write in the update after).
const OUTSIDE_PATH: Vec3 = [0.0; 3];

/// A planned path shorter than this replaces no short path (`DAT_143cc2f28`), and a path is long
/// when it is longer than this and than 1.25 times the way (`DAT_14330ebb8`, `DAT_142eb56a8`).
const LONG: f32 = 5.0;
const DETOUR: f32 = 1.25;
/// A new level's stretches longer than this squared get a midpoint (`DAT_142ed6f20`).
const SPLIT: f32 = crate::image::movement::SPLIT;
/// How many plans one motion step makes at most.
const PLANS: u32 = 3;
/// A level's plans before it gives up (`FUN_140f8f670`).
const CONTINUE: i32 = 10;

/// A position on the map as the game's vector (x, height, y), and back.
const fn vector([x, y, z]: [f32; 3]) -> Vec3 {
    [x, z, y]
}

const fn map_position([x, z, y]: Vec3) -> [f32; 3] {
    [x, y, z]
}

impl PathData {
    /// A new level (`FUN_140f8f670`) with a new path (`FUN_14152eea0`, flags 6, made `now`).
    const fn new(now: u32) -> Self {
        Self {
            level: 0,
            linked_type: 0,
            points: Vec::new(),
            has_path: true,
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
            ignore: Vec::new(),
            ignored: Vec::new(),
        }
    }

    /// A new level without a path (`FUN_140f8f670` alone): target -1, the plans
    /// [`CONTINUE`], everything else 0.
    #[must_use]
    pub const fn without_path() -> Self {
        Self {
            level: 0,
            linked_type: 0,
            points: Vec::new(),
            has_path: false,
            path_flags: 0,
            waypoint: 0,
            state: 0,
            range: 0.0,
            target: -1,
            pause_time: 0.0,
            flags: 0,
            length: 0.0,
            created: 0,
            continue_counter: CONTINUE,
            repath_time: 0.0,
            ignore: Vec::new(),
            ignored: Vec::new(),
        }
    }

    /// The waypoint at `index` (`FUN_140f9bcd0`); outside the path the game returns its static
    /// point `DAT_143f0dc88` ([`OUTSIDE_PATH`]).
    fn point(&self, index: i32) -> Vec3 {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.points.get(i))
            .copied()
            .unwrap_or(OUTSIDE_PATH)
    }

    /// `FUN_140f96050`'s test of the top level: a level without a path (`+0x18` null and none in
    /// the world's table by its id `+0x20`) has none (0); else waypoints after the current one:
    /// not `count - 1 <= waypoint` in 32 bits, so an empty path has more for a waypoint below -1.
    const fn has_more(&self) -> bool {
        self.has_path && self.waypoint < self.points.len() as i32 - 1
    }
}

/// `FUN_140f98d50`: a path's point `i` goes, the later ones move down (true); an index outside
/// the path (below 0 or past its last point) changes nothing (false).
fn remove_point(points: &mut Vec<Vec3>, i: i32) -> bool {
    match usize::try_from(i) {
        Ok(k) if k < points.len() => {
            let _ = points.remove(k);
            true
        }
        _ => false,
    }
}

/// `FUN_140f98dd0`: the path ends before its point `i` (the points from `i` on go) and is
/// measured again (`FUN_140f95a60`, true); an index outside the path, or an empty path, changes
/// nothing (false).
fn cut_path(path: &mut Path, i: i32) -> bool {
    match usize::try_from(i) {
        Ok(k) if k < path.points.len() => {
            path.points.truncate(k);
            path.measure();
            true
        }
        _ => false,
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

/// The motion step's start-test line for the ignore list (`FUN_141150f00`'s calls of
/// `FUN_14114ece0`): from the unit's place to the waypoint, both on the ground (x, y).
const fn ground_line(at: [f32; 3], waypoint: Vec3) -> ([f32; 2], [f32; 2]) {
    ([at[0], at[1]], [waypoint[0], waypoint[2]])
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

/// A master whose object flags have bit 1 turns as if dt were this many times longer
/// (`DAT_14330ebcc`).
const FAST_TURN: f32 = crate::image::movement::FAST_TURN;
/// The pause a kept hit's ranking gives a unit the walker outranks: 0.1 for an ally
/// (`DAT_142eb5690`), 0.5 for another (`DAT_142e7f384`).
const ALLY_PAUSE: f32 = crate::image::movement::ALLY_PAUSE;
const OTHER_PAUSE: f32 = crate::image::movement::OTHER_PAUSE;
/// The pause a level at its last waypoint without a path gives its own unit while it may still
/// try (`DAT_142eb5690`).
const STUCK_PAUSE: f32 = crate::image::movement::STUCK_PAUSE;
/// What [`Mover::avoid`] makes of the walk's hits: the hits it keeps and the pauses it gives.
struct Avoided {
    kept: Vec<i32>,
    pauses: Vec<(i32, f32)>,
    /// The step hit only the walked level's target: the waypoint counts as reached.
    at_target: bool,
}
/// 2π (`DAT_142eb56ac`): over a revolution's time, the most a unit turns per second.
const TWO_PI: f32 = crate::image::movement::TWO_PI;

/// The most a unit of a moving master turns per second (`+0x774`) as every copy of the master
/// sets it (master slot 0x80, `FUN_1410e3a70`, which also leaves the turn radius 0 and its
/// speed the largest float): with a turn speed (`+0x758`) and a walking sprite, the sprite's
/// facets times the turn speed are the time of a revolution (`+0x778`, slot 0x98
/// `FUN_1410e39e0`), and 2π over that is the rate (`+0x77c`, copied to `+0x774`); without either
/// the largest float. The `.dat`'s own turning values do not survive the copy.
#[must_use]
pub fn turn_rate(turn_speed: f32, walking_facets: Option<i16>) -> f32 {
    let Some(facets) = walking_facets.filter(|_| turn_speed != 0.0) else {
        return f32::MAX;
    };
    let revolution = f32::from(facets) * turn_speed;
    if revolution == 0.0 {
        f32::MAX
    } else {
        TWO_PI / revolution
    }
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
    // a signed `count - 1`: an empty path has no stretch
    if path.points.len() as i64 - 1 <= at as i64 {
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
    let _ = path.insert(at + 1, fmath::add(a, step));
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
#[derive(Debug)]
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
    /// The world's unit groups as they are now, which the long pather reads for a unit in a
    /// group (its group's differentiator units, `FUN_140f9eb70`).
    pub groups: &'a crate::group::UnitGroups,
}

impl Planning<'_> {
    /// The same planning for a call that ends before this one is used again.
    pub const fn reborrow(&mut self) -> Planning<'_> {
        Planning {
            pathing: &mut *self.pathing,
            now: self.now,
            computer: self.computer,
            object: &mut *self.object,
            construction_rate: self.construction_rate,
            groups: self.groups,
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
    /// The unit stopped (and may have paused itself): the step returns 0.
    Paused,
    /// The level gave up and the unit stopped: the step returns 1.
    GaveUp,
}

/// The squared distance between two footprints on the ground (`FUN_14118f8c0`): per axis the
/// centre distance minus both radii, at least 0, squared and summed.
#[must_use]
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
#[must_use]
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
    fn move_forward(&self, distance: f32, map: [i32; 2]) -> Option<[f32; 3]> {
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
        let result = self.motion_passes(me, dt, world, plan.as_deref_mut());
        // every exit of the step removes the kept hits' temporary shapes (FUN_141055970)
        if let Some(p) = plan {
            p.pathing.drop_temporary_shapes();
        }
        result
    }

    /// [`Mover::motion`]'s passes.
    fn motion_passes(
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
        // a paused walk stands (slot 0x5d0) until its pause ran out
        if 0.0 < level.pause_time {
            level.pause_time -= dt;
            if 0.0 < level.pause_time {
                self.stop()?;
                return Ok(Motion::Moving);
            }
            level.pause_time = 0.0;
        }
        let ground_radius = [self.stats.radius[0], self.stats.radius[1]];
        let mut plans = 0;
        loop {
            // a level without a path ends the step (`LAB_14115582c`), as no level does
            let Some(level) = self.paths.first().filter(|l| l.has_path) else {
                return Ok(Motion::Done);
            };
            let target = match level.target {
                -1 => None,
                id => Some(
                    *world
                        .get_known(id)?
                        .ok_or(Unsupported("a path target that is gone"))?,
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
                        Step::Paused => return Ok(Motion::Moving),
                        Step::GaveUp => return Ok(Motion::Done),
                    }
                }
                _ => {}
            }
            let (now, computer) = plan.as_deref().map_or((0, false), |p| (p.now, p.computer));
            let pathing = plan.as_deref_mut().map(|p| &mut *p.pathing);
            if let Next::Done(m) = self.walk(me, dt, world, pathing, goal, now, computer)? {
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
    #[allow(clippy::too_many_arguments)]
    fn walk(
        &mut self,
        me: &Me,
        dt: f32,
        world: &Spatial,
        pathing: Option<&mut Pathing>,
        goal: Goal<'_>,
        now: u32,
        computer: bool,
    ) -> Result<Next, Unsupported> {
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
                let avoided = self.avoid(me, world, &hits, projected, dt, now, computer)?;
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
                    return self.go_round(me, world, pathing, &hits, &avoided.kept, computer);
                }
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
        let clipped = self.steer(direction, dt)?;
        self.control_speed(waypoint, clipped);
        Ok(Next::Done(Motion::Moving))
    }

    /// The waypoint the level walks to was reached (`LAB_14115136c`, the level's bit 0): with
    /// bit 1 the unit takes the waypoint's place (slot 0x58, the position setter); the level moves
    /// on to its next waypoint, dropping those too near the map's edge for the unit
    /// (`FUN_140f98d50`). Then (`LAB_14115192d`) a level with waypoints left waits for a path to
    /// the next (state 2, not level 0); at its last, a unit not yet in range of where it was going
    /// steps back to the waypoint it left and asks again, else the level arrived (state 4). Level
    /// 0 on a level 1 parent looks ahead first ([`Mover::look_ahead`]). A looping path and a
    /// waypoint some object stands on are not modelled.
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
                let p = level.point(level.waypoint);
                let at = self.set_position(map_position(p), world.map);
                let at = self.grounded(at, world.terrain.as_ref());
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
            let [x, _, y] = level.point(level.waypoint);
            if rx <= x && ry <= y && x <= w as f32 - rx && y <= h as f32 - ry {
                break;
            }
            // the index is the waypoint just incremented, and no writer sets one below -1: it is
            // never negative, so the point goes (the game's loop would spin on a failed removal)
            let removed = remove_point(&mut level.points, level.waypoint);
            debug_assert!(removed, "a waypoint below -1");
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
                world.check_known_records([x, y, x, y])?;
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

    /// Level 0's look ahead on its level 1 parent, at a waypoint it reached (`LAB_14115136c`):
    /// from the parent's fifth waypoint on (at most its last) back toward its current one, the
    /// first within 10 tiles (squared, `DAT_14330ebcc`) that the unit can walk to straight
    /// (`FUN_141142750`) becomes the parent's waypoint; level 0 goes, and the parent asks for a
    /// path (state 2) as the motion step starts again. True when it did.
    fn look_ahead(&mut self, me: &Me, world: &Spatial) -> Result<bool, Unsupported> {
        let target = self.paths[0].target;
        let parent = &self.paths[1];
        let at = vector(me.position);
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
    fn straight(
        &self,
        me: &Me,
        p: Vec3,
        target: i32,
        world: &Spatial,
    ) -> Result<bool, Unsupported> {
        let [rx, ry, rz] = self.stats.radius;
        let [w, h] = world.map;
        let (x, y) = (p[0], p[2]);
        if !(rx <= x && rz <= y && x <= w as f32 - rx && y <= h as f32 - ry) {
            return Ok(false);
        }
        let ignore = world.query_ignore(&[me.id, target]);
        let (from, to) = self.query_ends(vector(me.position), p, world.map);
        let grow = rx * STRAIGHT_GROW;
        world.check_known_records([
            from[0].min(to[0]) - grow,
            from[1].min(to[1]) - grow,
            from[0].max(to[0]) + grow,
            from[1].max(to[1]) + grow,
        ])?;
        Ok(!world.segment_blocked(from, to, grow, self.stats.restriction_mask, &ignore))
    }

    /// `FUN_141142750` with its last argument 0, the close-in path to a waypoint or target nearer
    /// than a tile: the point must lie within the map less the unit's radii (as
    /// [`Mover::straight`]); then the walk's query from the unit to the point (`FUN_14114cc60`
    /// with the tiles' records, flags `0x90`, its radius) must find nothing but the unit and its
    /// target: no tile its terrain mask lacks ([`Spatial::tiles_block`]) and no record the walk
    /// hits ([`Spatial::walk_hits`]). The game writes the straight path into the new level.
    fn close_in(
        &self,
        me: &Me,
        p: Vec3,
        target: i32,
        world: &Spatial,
    ) -> Result<bool, Unsupported> {
        let [rx, ry, rz] = self.stats.radius;
        let [w, h] = world.map;
        let (x, y) = (p[0], p[2]);
        if !(rx <= x && rz <= y && x <= w as f32 - rx && y <= h as f32 - ry) {
            return Ok(false);
        }
        let ignore = world.query_ignore(&[me.id, target]);
        let (from, to) = self.query_ends(vector(me.position), p, world.map);
        let rect = [
            from[0].min(to[0]),
            from[1].min(to[1]),
            from[0].max(to[0]),
            from[1].max(to[1]),
        ];
        world.check_known_records([rect[0] - rx, rect[1] - rx, rect[2] + rx, rect[3] + rx])?;
        if world.tiles_block(from, to, rx, self.stats.restriction_mask) {
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
    fn arrived(&mut self, goal: Goal<'_>) -> Result<Next, Unsupported> {
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
                if parent.path_flags & 1 != 0 {
                    return Err(Unsupported(
                        "a looping parent's next waypoint (FUN_140f98bc0)",
                    ));
                }
            }
            3 => {
                let _ = self.paths.remove(0);
                self.paths[0].state = 2;
            }
            _ => return Err(Unsupported("an arrived level of another link")),
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
    fn clear_bit_2(&mut self) {
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
    pub(crate) fn stand(&mut self) -> Result<(), Unsupported> {
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
        self.ignore_units.clear();
        self.speed_stopped = true;
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
    fn top_has_more(&self) -> bool {
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

    /// The motion step's path request for the level it walks, in state 2 (`0x141150f00`, its
    /// `_Memory[10] == 2` block): unless the unit is in range of its target (state 4) or on its
    /// waypoint, the level spends one of its tries and a new level gets a path to the waypoint:
    /// the short pather's (`FUN_140f9e540`) for a near waypoint, else or when that fails the tile
    /// pathfinder's (`FUN_140f7c530`; a top level's plan becomes a level of its own, whose
    /// stretches the next pass serves). The new level goes under the walked one, which waits
    /// (state 0). A waypoint or target nearer than a tile takes the close-in path when the short
    /// pather found none ([`Mover::close_in`]); a far waypoint takes the long pather's path
    /// ([`Mover::long_plan`]). The escape from a game type 12 tile, a trade cart's budget and the
    /// other exits that stop the unit (slot 0x5d0) are not modelled.
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
                            let line = ground_line(me.position, waypoint);
                            let ignore =
                                self.ignore_list(me, world, plan, target, target_id, line)?;
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
                    let line = ground_line(me.position, waypoint);
                    let ignore = self.ignore_list(me, world, plan, target, target_id, line)?;
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
                let far = match target {
                    None => CLOSE <= way,
                    Some(_) if CLOSE <= gap => true,
                    Some(_) => {
                        return Err(Unsupported("a near target's master test (0x60c9)"));
                    }
                };
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
    fn long_plan(
        &mut self,
        me: &Me,
        world: &Spatial,
        plan: &Planning<'_>,
        waypoint: Vec3,
        way: f32,
        range: f32,
        target: Option<(i32, &Placed)>,
        path: &mut Path,
    ) -> Result<Option<(bool, Option<f32>)>, Unsupported> {
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
        let (mut found, long) = long_path_for(
            &walker,
            [me.position[0], me.position[1]],
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
    fn walled_off(
        &self,
        me: &Me,
        world: &Spatial,
        target_id: i32,
        path: &mut Path,
    ) -> Result<bool, Unsupported> {
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
    /// the end of its path (state 5); at its last waypoint while it may still try (its continue
    /// counter, `FUN_140f95bf0` with 0) the unit pauses for `STUCK_PAUSE` and stops; the exits
    /// that stop the unit otherwise are not modelled.
    fn no_path(
        &mut self,
        found: bool,
        target: Option<&Placed>,
        to_end: f32,
    ) -> Result<Step, Unsupported> {
        let top_has_more = self.top_has_more();
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
                    // the unit pauses itself (slot 0x5f0, the walked level's `+0x34`) and stops
                    // (slot 0x5d0); the step returns 0
                    level.pause_time = STUCK_PAUSE;
                    self.stop()?;
                    return Ok(Step::Paused);
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
            // the walked level lets its path go (FUN_140f95b70): it holds none (path id -1),
            // which reads as no points, flags, length or time
            let level = &mut self.paths[0];
            level.has_path = false;
            level.points.clear();
            level.path_flags = 0;
            level.length = 0.0;
            level.created = 0;
        }
        self.paths.insert(0, new);
        Step::Walk
    }

    /// The objects the short pather ignores (`FUN_14114ece0`, its path from the unit, `line` the two
    /// points its arguments 3 to 6 give the start test): the
    /// unit, its target, the unit's own list but itself when its top path was made this update
    /// (`+0x858`'s path at `+0x8`), and, for a standing unit with a target whose action is in
    /// state 8 (a drop-off), the workers standing within its radius and a tenth (the records
    /// `FUN_141051900` finds; one whose current path level's path was made this update is
    /// passed, [`crate::spatial::Gait::path_made`]) of the classes 2, 4,
    /// 19 and 21 (or whose slot 0x4f0 says so: 0 for a combat object, a task of action type 0x99
    /// on the master for a building of the classes 3, 27, 39, 49 and 52), when the
    /// unit's start is blocked (`FUN_140fa0110`: [`short_path::Pather::start_blocked`] along
    /// `line` with its radius, its weapon range and the target's radii, no shape ignored), allied with the unit and farther from the target than the unit's radius
    /// squared.
    fn ignore_list(
        &self,
        me: &Me,
        world: &Spatial,
        plan: &mut Planning<'_>,
        target: Option<&Placed>,
        target_id: i32,
        line: ([f32; 2], [f32; 2]),
    ) -> Result<Vec<i32>, Unsupported> {
        let now = plan.now;
        let mut list = vec![me.id];
        if target.is_some() {
            list.push(target_id);
            if !self.is_moving() && me.action_state == 8 {
                let grow = self.stats.radius[0] + RANGE_FLOOR;
                let [x, y, _] = me.position;
                let rect = [x, y, x, y];
                world.check_known_records([x - grow, y - grow, x + grow, y + grow])?;
                let t = target.ok_or(Unsupported("unreachable: a drop-off without its target"))?;
                let owner = world.get(me.id).map(|p| p.player);
                let r = self.stats.radius[0];
                let mut blocked: Option<bool> = None;
                for id in world.obstruction_candidates(rect, grow, &list) {
                    let Some(p) = world.get(id) else { continue };
                    let gait = world.gait(id);
                    if gait.is_some_and(|g| g.moving) {
                        continue;
                    }
                    // a worker whose path is this update's walks: it is passed
                    if gait.and_then(|g| g.path_made) == Some(now) {
                        continue;
                    }
                    // slot 0x4f0: a building's master with a task of action type 0x99
                    // (FUN_1415c60b0), 0 for anything else
                    let worker_building = matches!(p.class, 3 | 27 | 39 | 49 | 52)
                        && world
                            .task_0x99
                            .get(p.player)
                            .is_some_and(|m| m.contains(&p.master));
                    if !matches!(p.class, 2 | 4 | 19 | 21) && !worker_building {
                        continue;
                    }
                    let start_blocked = if let Some(b) = blocked {
                        b
                    } else {
                        let b = self.start_blocked(world, plan, t, line)?;
                        blocked = Some(b);
                        b
                    };
                    if !start_blocked {
                        continue;
                    }
                    let allied = owner.is_some_and(|o| {
                        let relations = world.relations.get(p.player).map_or(&[][..], |r| &r[..]);
                        crate::tasks::allied(relations, p.player, o, true)
                    });
                    let gap = footprint_gap_squared(p.position, p.radius, t.position, t.radius);
                    if !allied || gap <= r * r {
                        continue;
                    }
                    list.push(id);
                }
            }
        }
        if self.paths.last().is_some_and(|top| top.created == now) {
            list.extend(self.ignore_units.iter().filter(|&&id| id != me.id));
        }
        Ok(list)
    }

    /// `FUN_140fa0110`: whether the unit's start is blocked on its way along `line` (its arguments
    /// 3 to 6, in fixed point by truncation), with its radius, its weapon range (slot 0x230) and
    /// the target's radii (`FUN_140faa170`, [`short_path::Pather::start_blocked`]).
    fn start_blocked(
        &self,
        world: &Spatial,
        plan: &mut Planning<'_>,
        target: &Placed,
        (from, to): ([f32; 2], [f32; 2]),
    ) -> Result<bool, Unsupported> {
        // CVTTSS2SI into a 64-bit register, its low 32 bits kept
        let fixed = |f: f32| crate::checkpoint::cvttss2si_64(f * short_path::TO_FIXED) as i32;
        let query = short_path::Query {
            from: [fixed(from[0]), fixed(from[1])],
            to: [fixed(to[0]), fixed(to[1])],
            radius: self.stats.radius[0],
            target_size: target.radius,
            range: self.stats.weapon_range,
            blocked: u64::from(self.stats.restriction_mask ^ 0xff),
        };
        let tiles = world.tiles();
        let words = |x: u32, y: u32| tiles.word(x as i32, y as i32).unwrap_or(0);
        let (mesh, pather) = plan
            .pathing
            .mesh_and_pather()
            .ok_or(Unsupported("a start test without the game's mesh"))?;
        Ok(pather.start_blocked(mesh, &words, &query))
    }

    /// After a short path (`FUN_140f9e540`) for the new level: when the top path (`FUN_140f960f0`
    /// of the walked level) was made this tick, the new path takes the unit's ignore units
    /// (`FUN_140f9b7a0`).
    fn give_ignored(&self, new: &mut PathData, now: u32) {
        if self.paths.last().is_some_and(|t| t.created == now) {
            new.ignored.clone_from(&self.ignore_units);
        }
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
        let walker = self.walker(me.id);
        short_path_for(
            walker,
            [me.position[0], me.position[1]],
            world,
            plan,
            (to, range, target),
            ignore,
            out,
        )
    }

    /// What the planners read of this unit ([`Walker`]).
    pub(crate) const fn walker(&self, id: i32) -> Walker {
        Walker {
            id,
            radius: self.stats.radius[0],
            restriction: self.stats.restriction,
            mask: self.stats.restriction_mask,
        }
    }
    /// The tile pathfinder (`FUN_140f7c530`) from the unit to `to`, appended to `out`, smoothed
    /// with the obstruction manager's segment test for the unit (the records of the smoothing's
    /// ignore list ignored, [`Pathing::smoothing_ignore`]); None without a planner. The planner reads the first byte of the target's record
    /// (`+0x658`): 3, a box, for the obstruction types 2, 10, 11 and 13 (`FUN_141056460`).
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
        // the smoothing's ignore list: the unit and the level's target object (the planner's
        // argument 12, the object the world holds)
        let level_target = self
            .paths
            .first()
            .map(|l| l.target)
            .filter(|&t| t != -1 && world.get(t).is_some());
        tile_plan_for(
            self.walker(me.id),
            [me.position[0], me.position[1]],
            level_target,
            world,
            plan,
            (to, radii, range),
            target,
            out,
        )
    }

    /// Slot 0x3d8 of the unit (`FUN_14114ffc0` -> `FUN_140fa0370` without a target object):
    /// whether it can walk to within `range` of the point `to` (x, y on the map), as
    /// [`Mover::reach_test`] decides. `master` is the unit's master's id.
    pub fn reaches(
        &self,
        me: &Me,
        world: &Spatial,
        plan: &mut Planning<'_>,
        to: [f32; 2],
        range: f32,
        master: i16,
    ) -> Result<bool, Unsupported> {
        self.reach_test(me, world, plan, to, None, range, master)
    }

    /// Slot 0x3d0 of the unit (`FUN_14114fea0` -> `FUN_140fa0370` with the target object): whether
    /// it can walk to within `range` of object `target` (its place for the target's slot 0x520),
    /// as [`Mover::reach_test`] decides; a target that is gone is not reached.
    pub fn reaches_object(
        &self,
        me: &Me,
        world: &Spatial,
        plan: &mut Planning<'_>,
        target: i32,
        range: f32,
        master: i16,
    ) -> Result<bool, Unsupported> {
        let Some(t) = world.get_known(target)? else {
            return Ok(false);
        };
        let to = [t.position[0], t.position[1]];
        self.reach_test(me, world, plan, to, Some((target, t)), range, master)
    }

    /// Slot 0x3e8 of the unit (`FUN_14114f790`): the length of its short path to object `target`
    /// within `range`: the obstruction manager's old short pather ([`HullPather::find`], with the
    /// unit's place, radius and terrain mask, ignoring the unit and the target) when it finds a
    /// path within range or none is needed (its result below 2); None for another result or a
    /// target that is gone.
    pub fn short_reach(
        &self,
        me: &Me,
        world: &Spatial,
        target: i32,
        range: f32,
    ) -> Result<Option<f32>, Unsupported> {
        let Some(t) = world.get_known(target)? else {
            return Ok(None);
        };
        let to = [t.position[0], t.position[1]];
        let mask = self.stats.restriction_mask;
        let mut pather = HullPather::new(world, me.id, target, mask, self.stats.radius[0]);
        let (result, path) = pather.find([me.position[0], me.position[1]], to, target, range)?;
        Ok((result < 2).then(|| path.length()))
    }

    /// The length of the tile planner's path (`FUN_140f7c530` with the radii `radii`, range 0, no
    /// closed tiles, the tile search for a human player, no smoothing) from the unit to `to`, and
    /// the planner's result; None without a planner for the unit's restriction.
    pub fn plan_length(
        &self,
        me: &Me,
        world: &Spatial,
        plan: &mut Planning<'_>,
        to: [f32; 2],
        radii: f32,
    ) -> Result<Option<(u8, f32)>, Unsupported> {
        let full = !plan.computer;
        let Some(planner) = plan.pathing.planner(self.stats.restriction, world.tiles()) else {
            return Ok(None);
        };
        let req = Request {
            from: [me.position[0], me.position[1]],
            to,
            radii,
            range: 0.0,
            closed: Vec::new(),
            full,
        };
        let mut path = Path::default();
        let found = planner.plan::<Unsupported>(&req, false, &mut path, None)?;
        path.measure();
        Ok(Some((found.code(), path.length)))
    }

    /// `FUN_140fa0370`: both ends lie on the map. With a planner for its restriction, the tile
    /// planner (`FUN_140f7c530` without the tile search, closed tiles or smoothing; the radii are
    /// the unit's and the target's, the range 3 more for a target of master 45 or class 33) must
    /// reach the goal or its range (result 0 or 1). Then a unit with a footprint and a height
    /// (`FUN_141194060` with 1) no more than 10 tiles from the goal asks the short pather
    /// (`FUN_140f9e540` with its ignore list, `FUN_14114ece0`), whose path to the goal or into the
    /// range (a result below 2) answers yes. Otherwise a goal at least a tile from the unit's
    /// footprint (the squared gap, to the target's footprint for a target, at least 1) is the long
    /// pather's to answer ([`crate::long_path::reaches`], from the unit's tile to the goal's, the
    /// planner's other arguments unset: no path, the reach within a tile, no player's objects); a
    /// nearer one is not reached. `master` is the unit's master's id (42 moves for the long
    /// pather without a speed).
    #[allow(clippy::too_many_arguments)]
    fn reach_test(
        &self,
        me: &Me,
        world: &Spatial,
        plan: &mut Planning<'_>,
        to: [f32; 2],
        target: Option<(i32, &Placed)>,
        range: f32,
        master: i16,
    ) -> Result<bool, Unsupported> {
        let [x, y, _] = me.position;
        let [w, h] = world.map;
        let on_map = |px: f32, py: f32| 0.0 <= px && 0.0 <= py && px < w as f32 && py < h as f32;
        if !on_map(x, y) || !on_map(to[0], to[1]) {
            return Ok(false);
        }
        let radius = self.stats.radius;
        let ground = [radius[0], radius[1]];
        let (target_id, target_radius) = target.map_or((-1, 0.0), |(id, t)| (id, t.radius[0]));
        let bonus = target.is_some_and(|(_, t)| t.master == WIDE_MASTER || t.class == WIDE_CLASS);
        let mut path = Path::default();
        if let Some(planner) = plan.pathing.planner(self.stats.restriction, world.tiles()) {
            let req = Request {
                from: [x, y],
                to,
                radii: radius[0] + target_radius,
                range: if bonus { range + WIDE_RANGE } else { range },
                closed: Vec::new(),
                full: false,
            };
            let found = planner.plan::<Unsupported>(&req, false, &mut path, None)?;
            if 1 < found.code() {
                return Ok(false);
            }
        }
        let solid = 0.0 < radius[2] && 0.0 < radius[0] && 0.0 < radius[1];
        let (dx, dy) = (to[0] - x, to[1] - y);
        if solid && fmath::sqrt(dx * dx + 0.0 + dy * dy) <= REACH_SHORT {
            let goal = [to[0], 0.0, to[1]];
            // FUN_140fa0370 hands the start test both vectors' first two floats: x and the
            // height, the unit's (`+0x824`) and the target's (slot 0x520), not x and y
            let height = target.map_or(0.0, |(_, t)| t.position[2]);
            let line = ([x, me.position[2]], [to[0], height]);
            let ignore =
                self.ignore_list(me, world, plan, target.map(|(_, t)| t), target_id, line)?;
            if self.short_path(me, world, plan, goal, range, target_id, &ignore, &mut path)? < 2 {
                return Ok(true);
            }
        }
        let long_target = if let Some((_, t)) = target {
            let gap2 = footprint_gap_squared(me.position, ground, t.position, t.radius);
            if gap2 < REACH_NEAR {
                return Ok(false);
            }
            let gap = footprint_gap(me.position, ground, t.position, t.radius);
            Some(crate::long_path::LongTarget::new(
                t.position, t.radius, gap, gap2,
            ))
        } else {
            let gap = |d: f32, r: f32| {
                let g = d.abs() - r;
                if 0.0 <= g { g } else { 0.0 }
            };
            let (gx, gy) = (gap(dx, radius[0]), gap(dy, radius[1]));
            if gx * gx + gy * gy < REACH_NEAR {
                return Ok(false);
            }
            None
        };
        if !self.ignore_units.is_empty() {
            return Err(Unsupported(
                "the long pather's ignored units (slots 0x2a8, 0x5c8)",
            ));
        }
        // FUN_140f60440 lowers the tall count under the unit and the target
        let lowered = [Some(me.id), target.map(|(id, _)| id)]
            .into_iter()
            .flatten()
            .filter_map(|id| world.get(id))
            .filter(|p| p.tall)
            .map(|p| [p.position[0] as i32, p.position[1] as i32])
            .collect();
        let req = crate::long_path::LongRequest {
            from: [x as i32, y as i32],
            to: [to[0] as i32, to[1] as i32],
            range,
            target: long_target,
            moves: self.stats.speed != 0.0 || master == LONG_PATH_MOVER,
            mask: self.stats.restriction_mask,
            computer: plan.computer,
            lowered,
            within_range: false,
            near_player: false,
            allies: world.get(me.id).map_or(0, |p| world.allies_of(p.player)),
        };
        crate::long_path::reaches(world.tiles(), world.map, &req)
    }

    /// The test before a non-quiet `MoveTo`'s path end follows its moved target (`0x140aa2a70` in
    /// state 4): the top path (`FUN_140f960f0`) and the walked level's own path, that level being
    /// level 0, are both held, and the level's current waypoint (`FUN_140f9bcd0`) is the top
    /// path's last point (`FUN_140f95fb0`, compared x, y, z). A top path of one point, where the
    /// game compares with its static point, is not modelled.
    pub fn walks_to_path_end(&self) -> Result<bool, Unsupported> {
        let Some(level) = self
            .paths
            .first()
            .filter(|l| l.level == 0 && !l.points.is_empty())
        else {
            return Ok(false);
        };
        let Some(top) = self.top_path() else {
            return Ok(false);
        };
        let [.., _, last] = top.as_slice() else {
            return Err(Unsupported("a top path of one point (FUN_140f95fb0)"));
        };
        Ok(point_or_zero_of(level, level.waypoint) == *last)
    }

    /// A non-quiet `MoveTo`'s path end follows its target to `point` (`0x140aa2a70` after the line
    /// test): the walked level's current waypoint (`FUN_140f9b890` at its index) and the top
    /// path's last point take it (the paths' layout, [`vector`]); bits 1 and 0 go off the walked
    /// level (`FUN_140f9b4d0` twice).
    pub fn follow_path_end(&mut self, point: [f32; 3]) {
        let point = vector(point);
        if let Some(level) = self.paths.first_mut() {
            if let Some(w) = usize::try_from(level.waypoint)
                .ok()
                .and_then(|i| level.points.get_mut(i))
            {
                *w = point;
            }
            level.flags &= !3;
        }
        if let Some(last) = self.paths.last_mut().and_then(|l| l.points.last_mut()) {
            *last = point;
        }
    }

    /// The path of the top level (`FUN_140f960f0`): the last level's, or the walked level's when
    /// it has no parent; None when that level holds no path.
    fn top_path(&self) -> Option<&Vec<Vec3>> {
        self.paths
            .last()
            .map(|l| &l.points)
            .filter(|p| !p.is_empty())
    }

    /// The path request of a `MoveTo` (`FUN_14114e410`, without waypoints of its own): one level of
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
        // a new path's flags are 6 (FUN_140f8f5d0); the request clears bits 0 and 1. The walked
        // level, if any, is reset for it (FUN_140f98e80: its repath time stays)
        let path_flags = 6 & !1 & !2;
        let repath_time = self.paths.first().map_or(0.0, |l| l.repath_time);
        self.paths = vec![PathData {
            level: 2,
            points,
            path_flags,
            waypoint: 1,
            state: 2,
            range,
            target,
            repath_time,
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
            // COMISS eps, length; JBE at 0x14114f2aa: a NaN length divides too
            if fmath::at_most_or_unordered(EPSILON, length) {
                return [x / length, 0.0, z / length];
            }
        }
        self.forward
    }

    /// Where this tick's speed would take the unit along `direction` (slot 199, `0x141161860`)
    /// from where it last moved to (`+0x824`, [`Mover::moved_at`]): the forward speed after one
    /// more step of acceleration, capped by the top speed (COMISS/JA, then JC: a NaN speed stays).
    fn projected(&self, direction: Vec3, dt: f32) -> Vec3 {
        let position = vector(self.moved_at);
        let Some(v) = &self.velocity else {
            return position;
        };
        let mut next = fmath::add(v.velocity, fmath::scale(dt, v.acceleration));
        if self.stats.speed < next[2] || next[2] <= 0.0 {
            next[2] = self.stats.speed;
        }
        fmath::add(position, fmath::scale(dt * next[2], direction))
    }

    /// The walk's avoidance of what its step hits (`0x141150f00`, after `FUN_14114cc60`). The first
    /// loop keeps a hit the unit must go round: none whose overlap with the unit's box the step
    /// shrinks (`FUN_1411615a0`); else one in the walked path's ignore units (`FUN_140f98d20`)
    /// that moved before the top path was made, or one that moved after the walked path was made
    /// (or at the same time and outranks the unit: a higher player, or the same and a higher id).
    /// A step whose one hit is the walked level's target counts its waypoint reached (the
    /// caller's, [`Avoided::at_target`]). A kept hit (its temporary shape `FUN_141054fe0`, the
    /// step's slide `FUN_141159220` and the re-plan) is the caller's ([`Mover::go_round`]).
    /// Then, on the walked level's first two waypoints (or the third of a path more
    /// than 500 old), each hit the unit outranks pauses for `dt` when its own pause is shorter: a
    /// human walker outranks another player's unit, or one of its own ordered at another time,
    /// whose last human order (`+0x650`) is older; else the lower id outranks (a group's order
    /// would decide, the slice holds none); a Gaia unit is outranked by any non-Gaia walker; the
    /// pause itself (slot 0x5f0) is not modelled. The paths'
    /// hash sets (`+0x18`, `FUN_140f96120`) are empty in this game (the probe `path_sets.py`),
    /// and the slice holds no unit groups (`+0x6d0`).
    #[allow(clippy::too_many_arguments)]
    fn avoid(
        &self,
        me: &Me,
        world: &Spatial,
        hits: &[i32],
        projected: Vec3,
        dt: f32,
        now: u32,
        computer: bool,
    ) -> Result<Avoided, Unsupported> {
        let level = &self.paths[0];
        let top = self.paths.last().filter(|l| !l.points.is_empty());
        let player = world
            .get(me.id)
            .ok_or(Unsupported("a walker without a place"))?
            .player;
        let [rx, ry, _] = self.stats.radius;
        let at = vector(me.position);
        let met = |id: i32| -> Result<(i32, &Placed), Unsupported> {
            let id = world.annex_of(id).unwrap_or(id);
            let p = world
                .get_known(id)?
                .ok_or(Unsupported("a met object without a place"))?;
            Ok((id, p))
        };
        let mut target_hit = false;
        let mut kept = Vec::new();
        for &hit in hits {
            let (id, other) = met(hit)?;
            if 0 <= level.target && id == level.target {
                target_hit = true;
            }
            let o = vector(other.position);
            let now_overlap = overlap(at, [rx, ry], o, other.radius);
            let next_overlap = overlap(projected, [rx, ry], o, other.radius);
            let gait = world.gait(id);
            let last_move = gait.map_or(0, |g| g.last_move);
            let mut pushed = false;
            if let Some(top) = top.filter(|_| now_overlap <= next_overlap) {
                if top.created == now && rx * PUSHED < next_overlap && level.ignored.contains(&id) {
                    pushed = true;
                }
                if top.created < now && level.ignored.contains(&id) {
                    pushed = last_move < top.created;
                }
            }
            let outranks = player < other.player || (player == other.player && me.id < id);
            let outranks = level.created < last_move || (outranks && level.created == last_move);
            if next_overlap < now_overlap || (!pushed && !outranks) {
                continue;
            }
            kept.push(id);
        }
        // FUN_141159220 slides the projected step out of the hits of no speed of classes 3, 27,
        // 39 and 52 (buildings, walls, gates): only the kept hits' re-plan reads the slid point,
        // and with no kept hit the slide changes nothing
        if hits.len() == 1 && target_hit {
            return Ok(Avoided {
                kept: Vec::new(),
                pauses: Vec::new(),
                at_target: true,
            });
        }
        if !kept.is_empty() {
            return Ok(Avoided {
                kept,
                pauses: Vec::new(),
                at_target: false,
            });
        }
        let mut pauses = Vec::new();
        if level.waypoint < 2 || (level.waypoint < 3 && level.created.wrapping_add(500) < now) {
            for &hit in hits {
                let (id, other) = met(hit)?;
                if 0 <= level.target && id == level.target {
                    continue;
                }
                if player == 0 || other.player != 0 {
                    // a human walker ranks another player's unit, or one of its own ordered at
                    // another time, by their last human order times (+0x650): the older order
                    // pauses; else by id (slot 0x5f8)
                    let mine = world.gait(me.id).map_or(0, |g| g.human_order);
                    let theirs = world.gait(id).map_or(0, |g| g.human_order);
                    let outranks = if !computer && (other.player != player || theirs != mine) {
                        theirs < mine
                    } else {
                        me.id < id
                    };
                    if !outranks {
                        continue;
                    }
                }
                // slot 0x528's walked level, paused for dt more when its pause is shorter
                if let Some(p) = self.pause_of(world, id).filter(|&p| p < dt) {
                    pauses.push((id, p + dt));
                }
            }
        }
        Ok(Avoided {
            kept,
            pauses,
            at_target: false,
        })
    }

    /// The pause of unit `id`'s walked level as slot 0x5f0 leaves it: the last this walk gave it
    /// (the game's call writes it at once, a later pass of the same step reads it), else the
    /// world's ([`crate::spatial::Gait::pause`]).
    fn pause_of(&self, world: &Spatial, id: i32) -> Option<f32> {
        self.pauses
            .iter()
            .rev()
            .find(|&&(i, _)| i == id)
            .map(|&(_, p)| p)
            .or_else(|| world.gait(id).and_then(|g| g.pause))
    }

    /// The walk's kept hits (`0x141150f00` after its first hit loop): each kept hit without a
    /// mesh shape of its own got a temporary one there (`FUN_141054fe0` with 1, not the object's:
    /// its `+0x64c` stays -1), which the step's every exit removes. Then each hit but the walked
    /// level's target, a building's annex standing for it: one the walker outranks (as the
    /// pause loop of [`Mover::avoid`] ranks them; any Gaia hit of a walker of a player) pauses
    /// (slot 0x5f0: 0.1 for one of a player the walker's counts an ally, else 0.5), and one of
    /// the target's master that neither player counts an ally notifies the walker's unit AI
    /// (0x215, [`Mover::in_the_way`]). Then the walked level goes and its parent asks for a path again
    /// (state 2), which plans around the temporary shapes; without a parent the unit stops (slot
    /// 0x5d0).
    fn go_round(
        &mut self,
        me: &Me,
        world: &Spatial,
        pathing: Option<&mut Pathing>,
        hits: &[i32],
        kept: &[i32],
        computer: bool,
    ) -> Result<Next, Unsupported> {
        let pathing = pathing.ok_or(Unsupported("a kept hit without the world's mesh"))?;
        for &id in kept {
            if pathing.shape(id).is_none() {
                let p = world
                    .get_known(id)?
                    .ok_or(Unsupported("a kept hit without a place"))?;
                if p.obstruction_type == 11 {
                    return Err(Unsupported(
                        "a kept hit of obstruction type 11 (its outline)",
                    ));
                }
                pathing.add_temporary_shape(p);
            }
        }
        let level = &self.paths[0];
        let player = world
            .get(me.id)
            .ok_or(Unsupported("a walker without a place"))?
            .player;
        for &hit in hits {
            let id = world.annex_of(hit).unwrap_or(hit);
            if 0 <= level.target && id == level.target {
                continue;
            }
            let Some(other) = world.get_known(id)? else {
                continue;
            };
            let outranks = if player == 0 || other.player != 0 {
                let mine = world.gait(me.id).map_or(0, |g| g.human_order);
                let theirs = world.gait(id).map_or(0, |g| g.human_order);
                if !computer && (other.player != player || theirs != mine) {
                    theirs < mine
                } else {
                    me.id < id
                }
            } else {
                true
            };
            // slot 0x5f0 with the walker's player's view of the hit's (FUN_141663d90 with the
            // ally flag): an ally pauses 0.1, anyone else 0.5 (Gaia counts for no one)
            let allied = world.allied_with(player, other.player);
            if outranks && world.gait(id).is_some_and(|g| g.pause.is_some()) {
                let pause = if allied { ALLY_PAUSE } else { OTHER_PAUSE };
                self.pauses.push((id, pause));
            }
            // a hit that neither counts the other an ally, of the walked level's target's
            // master, notifies the walker's unit AI (0x215)
            if !allied && !world.allied_with(other.player, player) && 0 <= level.target {
                let target = world.get(level.target).map(|t| t.master);
                if target.is_some_and(|m| m == other.master) {
                    self.in_the_way.push(id);
                }
            }
        }
        if self.paths.len() < 2 {
            return Err(Unsupported(
                "a kept hit on a walk without a parent level (its stop)",
            ));
        }
        let _ = self.paths.remove(0);
        self.paths[0].state = 2;
        Ok(Next::Again)
    }

    /// The obstruction query's ends (`FUN_14114cc60`): the step's two ends, each held within 0.1
    /// of the map's far edges (`FUN_141051900` orders them into its rectangle).
    fn query_ends(&self, from: Vec3, to: Vec3, map: [i32; 2]) -> ([f32; 2], [f32; 2]) {
        let far = |size: i32| size as f32 - 0.1;
        let hold = |v: f32, max: f32| if v < 0.0 { 0.0 } else { v.min(max) };
        (
            [hold(from[0], far(map[0])), hold(from[2], far(map[1]))],
            [hold(to[0], far(map[0])), hold(to[2], far(map[1]))],
        )
    }

    /// The facets of the unit's current sprite ([`Mover::sprite_facets`]), else its master's when
    /// every sprite it shows has the same.
    #[must_use]
    pub fn facets(&self) -> Option<crate::facing::SpriteFacets> {
        self.sprite_facets.or(self.stats.facets)
    }

    /// Turn toward `direction` (slot 195, `0x14114f630`, then the yaw, slot 184, `0x141163140`):
    /// by the angle between the flat forward vector and the direction, at most the turn rate times
    /// dt (a hundred times dt for a master whose object flags have bit 1), about the up axis; then
    /// the forward vector and its right are renormalised (slot 173, `0x14115d150`, which rebuilds
    /// the right vector from up and forward, and takes the forward vector's facet,
    /// `FUN_14119d0c0`); the angle becomes the facet's (`FUN_14114d780`, [`crate::facing`]).
    /// Returns whether the turn fell short (the angle was above the most), which brakes the unit
    /// ([`Mover::control_speed`]). A unit whose sprites differ in their facets is not modelled.
    fn steer(&mut self, direction: Vec3, dt: f32) -> Result<bool, Unsupported> {
        let mut flat = self.forward;
        flat[1] = 0.0;
        let flat = fmath::normalize(flat);
        let angle = fmath::angle_between(direction, flat);
        let dt = if self.stats.fast_turn {
            dt * FAST_TURN
        } else {
            dt
        };
        let most = self.stats.turn_rate * dt;
        let clipped = most < angle;
        let mut turn = if angle <= most { angle } else { most };
        if 0.0 < fmath::cross(direction, flat)[1] {
            turn = -turn;
        }
        if self.paths[0].flags & 1 != 0 && turn == 0.0 {
            return Ok(clipped);
        }
        let m = fmath::rotation(turn, UP);
        let forward = fmath::normalize(fmath::rotate(&m, self.forward));
        self.forward = forward;
        self.right = fmath::normalize(fmath::cross(UP, forward));
        let facets = self.facets().ok_or(Unsupported(
            "the yaw of a unit whose sprites differ in their facets",
        ))?;
        let facet = crate::facing::facet_of_forward(forward, facets);
        self.angle = crate::facing::angle_of_facet(facet, facets);
        Ok(clipped)
    }

    /// The speed control toward the waypoint (slot 198, `0x141150a80`): a unit whose turn fell
    /// short stops (its speed 0, slot 0x598) and brakes; else it accelerates to the top speed
    /// unless the waypoint lies inside the turning circle, where it brakes. The way to the
    /// waypoint and the circle start where the unit last moved to (`+0x824`,
    /// [`Mover::moved_at`]).
    fn control_speed(&mut self, waypoint: Vec3, clipped: bool) {
        let position = vector(self.moved_at);
        let Some(v) = self.velocity.as_mut() else {
            self.velocity = Some(Velocity {
                velocity: [0.0; 3],
                acceleration: [0.0, 0.0, f32::MAX],
            });
            return;
        };
        if clipped {
            v.acceleration[2] = -f32::MAX;
            v.velocity[2] = 0.0;
            return;
        }
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
    // COMISS eps, after; JA: only an ordered 1e-7 above `after` reaches at once
    if fmath::at_most_or_unordered(EPSILON, after) {
        // COMISS step, now; JBE: short also when either is NaN
        if fmath::at_most_or_unordered(step, now) {
            return Reach::Short;
        }
        if now < after {
            return Reach::Away;
        }
    }
    Reach::Reached
}

/// What the long pather reads of its unit (`FUN_140f9eb70`'s argument 6): its id, its own place
/// (the target's gaps are from it), its radii on the ground, its restriction's mask, whether it
/// moves (its speed, slot 0x208, is not 0, or its master is 42) and its ignore units (`+0x670`).
#[derive(Clone, Debug)]
pub(crate) struct LongWalker<'a> {
    pub(crate) id: i32,
    pub(crate) position: [f32; 3],
    pub(crate) radius: [f32; 2],
    pub(crate) mask: u8,
    pub(crate) moves: bool,
    pub(crate) ignore_units: &'a [i32],
    /// For a walker in a group the world holds (`+0x6d0`, `FUN_14152ff40`), its group's
    /// differentiator units (`FUN_14100bbe0` with 255, [`crate::group::UnitGroups::group_units`]).
    pub(crate) group_units: Option<Vec<i32>>,
}

/// The units whose tiles the long pather lowers (`FUN_140f9eb70`'s list for `FUN_140f60440`):
/// the walker and its ignore units that wait to move or move (slots 0x2a8 and 0x5c8), not the
/// target, at most 254; for a walker in a group the units of the group's differentiator groups
/// instead ([`LongWalker::group_units`]); then the target.
pub(crate) fn lowered_units(
    walker: &LongWalker<'_>,
    world: &Spatial,
    target: Option<i32>,
) -> Vec<i32> {
    /// The list's room for the walker's ignore units (`0xfe < lVar26` breaks).
    const IGNORED: usize = 254;
    let target_id = target.unwrap_or(-1);
    let own = || {
        std::iter::once(walker.id)
            .chain(
                walker
                    .ignore_units
                    .iter()
                    .copied()
                    .filter(|&id| {
                        id != walker.id
                            && id != target_id
                            && world.get(id).is_some()
                            && world.gait(id).is_some_and(|g| g.waits || g.moving)
                    })
                    .take(IGNORED),
            )
            .collect()
    };
    let mut listed: Vec<i32> = walker.group_units.clone().unwrap_or_else(own);
    if let Some(id) = target {
        listed.push(id);
    }
    listed
}

/// The long pather (`FUN_140f9eb70`) for `walker` from `from` (x, y on the map, as tiles) to the
/// tile of `to` within `range` of `target`: whether it reaches the goal, and the path at the
/// walker's height. The search lowers the tall counts of the tiles of its list
/// (`FUN_140f60440`): the walker and its ignore units that wait to move or move (slots 0x2a8 and
/// 0x5c8; not the target, at most 254), or, for a walker in a group, the units of the group's
/// differentiator groups instead ([`lowered_units`]); then the
/// target. Each listed object the world holds whose master has a height lowers its tile.
///
/// # Errors
///
/// What the search does not model.
pub(crate) fn long_path_for(
    walker: &LongWalker<'_>,
    from: [f32; 2],
    world: &Spatial,
    computer: bool,
    (to, range): (Vec3, f32),
    target: Option<(i32, &Placed)>,
) -> Result<(bool, Path), Unsupported> {
    let long_target = target.map(|(_, t)| {
        let gap2 = footprint_gap_squared(walker.position, walker.radius, t.position, t.radius);
        let gap = footprint_gap(walker.position, walker.radius, t.position, t.radius);
        crate::long_path::LongTarget::new(t.position, t.radius, gap, gap2)
    });
    let listed = lowered_units(walker, world, target.map(|(id, _)| id));
    let lowered = listed
        .into_iter()
        .filter_map(|id| world.get(id))
        .filter(|p| p.tall)
        .map(|p| [p.position[0] as i32, p.position[1] as i32])
        .collect();
    let req = crate::long_path::LongRequest {
        from: [from[0] as i32, from[1] as i32],
        to: [to[0] as i32, to[2] as i32],
        range,
        target: long_target,
        moves: walker.moves,
        mask: walker.mask,
        computer,
        lowered,
        within_range: false,
        near_player: false,
        allies: world
            .get(walker.id)
            .map_or(0, |p| world.allies_of(p.player)),
    };
    crate::long_path::path(world.tiles(), world.map, &req, walker.position[2])
}

/// The tile pathfinder (`FUN_140f7c530`) for `walker` from `from` (x, y on the map) to `to`,
/// its walker's and the target's radii `radii` added, within `range`; appended to `out`,
/// smoothed with the obstruction manager's segment test for the walker (the records of the
/// smoothing's ignore list ignored, [`Pathing::smoothing_ignore`]: the walker and
/// `level_target`, the object the world holds, with a main building's annexes over it); None
/// without a planner for the walker's restriction. The planner reads the first byte of the
/// target's record (`+0x658`): 3, a box, for the obstruction types 2, 10, 11 and 13
/// (`FUN_141056460`). A computer's plan (`plan.computer`) is not full.
///
/// # Errors
///
/// What the planner does not model.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tile_plan_for(
    walker: Walker,
    from: [f32; 2],
    level_target: Option<i32>,
    world: &Spatial,
    plan: &mut Planning<'_>,
    (to, radii, range): (Vec3, f32, f32),
    target: Option<&Placed>,
    out: &mut Path,
) -> Result<Option<u8>, Unsupported> {
    let target_box = target
        .and_then(|t| t.obstruction)
        .is_some_and(|o| matches!(o.kind, 2 | 10 | 11 | 13));
    let req = Request {
        from,
        to: [to[0], to[2]],
        radii,
        range,
        closed: Vec::new(),
        full: !plan.computer,
    };
    let annexes = level_target
        .filter(|&t| world.annex_of(t).is_none())
        .map(|t| world.annexes_of(t))
        .unwrap_or_default();
    let (slots, ignore) = plan
        .pathing
        .smoothing_ignore(walker.id, level_target.map(|t| (t, annexes.as_slice())));
    let (radius, mask) = (walker.radius, walker.mask);
    let Some(planner) = plan.pathing.planner(walker.restriction, world.tiles()) else {
        return Ok(None);
    };
    let smoothed = std::cell::Cell::new(false);
    let mut blocked = |a: [f32; 2], b: [f32; 2]| {
        smoothed.set(true);
        Ok(world.segment_blocked(a, b, radius, mask, &ignore))
    };
    let found = planner.plan(&req, target_box, out, Some(&mut blocked))?;
    if smoothed.get() {
        plan.pathing.smoothing_ran(slots);
    }
    Ok(Some(found.code()))
}

/// `0x141150f00`'s test before a solid unit's plan: a trade cart needs its owner's budget
/// (`FUN_14165d6f0` on its wait delays `+0x7fa`), and no unit makes more than [`PLANS`] plans in
/// one step (`local_19a8 < 3`). A walker without an owner has no budget to test.
fn may_plan(trade_cart: bool, budget: Option<PlanBudget>, wait_delays: u8, plans: u32) -> bool {
    let allowed = !trade_cart || budget.is_none_or(|b| b.allows(i32::from(wait_delays)));
    allowed && plans < PLANS
}

/// What the planners read of the walking unit (the unit, or a group's lead): its id, radius on
/// x (master `+0x5d8`), terrain restriction (`+0x606`) and that restriction's mask.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Walker {
    pub(crate) id: i32,
    pub(crate) radius: f32,
    pub(crate) restriction: usize,
    pub(crate) mask: u8,
}

/// The short pather (`FUN_140f9e540`) for `walker` from `from` (x, y on the map) to `to` within
/// `range` of `target` (-1 for none), `ignore` its ignore list; its points appended to `out`.
/// Returns its result (0 in range, 1 reached, 2 partial, 3 blocked, 4 invalid).
///
/// # Errors
///
/// What the short pather does not model, or a short path without the game's mesh.
pub(crate) fn short_path_for(
    walker: Walker,
    from: [f32; 2],
    world: &Spatial,
    plan: &mut Planning<'_>,
    (to, range, target): (Vec3, f32, i32),
    ignore: &[i32],
    out: &mut Path,
) -> Result<u8, Unsupported> {
    let [x, y] = from;
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
        radius: walker.radius,
        range,
        target,
        listed,
        map: world.map,
        mask: walker.mask,
    };
    // the search's box: four tiles around its ends
    let reach = 4.0 + walker.radius;
    world.check_known_records([
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
                let _ = out.push(p);
            }
            Ok(found.result)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spatial::{Gait, Obstruction, Placed};

    const DT: f32 = 0.02;

    /// MarvinBlue/426048731's boar (master 48): a walking sprite of 16 facets at a turn speed of
    /// 0.05 turns 2π / 0.8 a second, 11.25° in a tick of 0.025 s; a villager turns at once.
    #[test]
    fn a_master_s_turn_rate_comes_from_its_turn_speed_and_walking_facets() {
        let rate = turn_rate(0.05, Some(16));
        assert_eq!(rate, TWO_PI / (16.0f32 * 0.05));
        assert_eq!(rate.to_bits(), 7.853_982f32.to_bits());
        assert_eq!(turn_rate(0.0, Some(16)), f32::MAX);
        assert_eq!(turn_rate(0.05, None), f32::MAX);
    }

    fn stats() -> MoveStats {
        MoveStats {
            fly_mode: 0,
            speed: 0.8,
            turn_rate: f32::MAX,
            turn_radius: 0.0,
            turn_radius_speed: f32::MAX,
            radius: [0.2, 0.2, 0.5],
            restriction: 7,
            restriction_mask: 6,
            turn_time: 0.0,
            move_algorithm: 0,
            min_collision: 1.0,
            fast_turn: false,
            walk_speed: 1.0,
            stand_speed: 1.0,
            run_speed: 1.0,
            facets: Some(crate::facing::SpriteFacets {
                count: 16,
                flag_1: true,
            }),
            weapon_range: 0.0,
        }
    }

    fn level(level: u8, points: &[[f32; 2]], state: i32) -> PathData {
        PathData {
            level,
            linked_type: i32::from(level == 0),
            points: points.iter().map(|&[x, y]| [x, 0.0, y]).collect(),
            has_path: true,
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
            ignore: Vec::new(),
            ignored: Vec::new(),
        }
    }

    /// MarvinBlue/426048731's boar 9729 in update 4193: its Attack's `MoveTo` (`+0x61` clear)
    /// walks to its path's end when its target, the hunter 10049, walks off; the walked level's
    /// waypoint and the top path's end follow the hunter, in the paths' layout (x, z, y).
    #[test]
    fn a_path_s_end_follows_a_moved_target_in_the_paths_layout() {
        let mut m = builder_9177();
        m.paths[0].flags = 3;
        assert!(m.walks_to_path_end().unwrap());
        m.follow_path_end([99.515_114, 41.147_972, 0.0]);
        assert_eq!(m.paths[0].points[1], [99.515_114, 0.0, 41.147_972]);
        assert_eq!(m.paths[1].points[1], [99.515_114, 0.0, 41.147_972]);
        assert_eq!(m.paths[0].flags, 0);
        assert!(m.walks_to_path_end().unwrap());
    }

    /// MarvinBlue/426048731's builder 9177 as the game saved it after tick 679 (update 680): its
    /// first path toward the house foundation 10028, facing along it, not yet moving.
    fn builder_9177() -> Mover {
        Mover {
            forward: [0.894_427_2, 0.0, -0.447_213_62],
            right: [-0.447_213_62, 0.0, -0.894_427_2],
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
            moved_at: [28.0, 89.0, 0.0],
            substitute_count: 0,
            ignore_units: Vec::new(),
            last_turn: 0.0,
            angle: 0.0,
            move_to_point: None,
            speed_stopped: false,
            pauses: Vec::new(),
            in_the_way: Vec::new(),
            sprite_facets: None,
        }
    }

    /// `FUN_140f98d50` and `FUN_140f98dd0`: a point goes, or the path ends before it and is
    /// measured again; an index outside the path, or an empty path, changes nothing.
    #[test]
    fn a_path_loses_a_point_or_its_end_only_for_an_index_inside_it() {
        let mut points = vec![[0.0; 3], [1.0, 0.0, 0.0], [1.0, 0.0, 2.0]];
        assert!(!remove_point(&mut points, -1));
        assert!(!remove_point(&mut points, 3));
        assert!(remove_point(&mut points, 1));
        assert_eq!(points, [[0.0; 3], [1.0, 0.0, 2.0]]);
        let mut path = Path {
            points: vec![[0.0; 3], [3.0, 0.0, 0.0], [3.0, 0.0, 4.0], [0.0; 3]],
            length: 12.0,
        };
        assert!(!cut_path(&mut path, -1));
        assert!(!cut_path(&mut path, 4));
        assert_eq!(path.length, 12.0);
        assert!(cut_path(&mut path, 3));
        assert_eq!((path.points.len(), path.length), (3, 7.0));
        assert!(cut_path(&mut path, 0));
        assert_eq!((path.points.len(), path.length), (0, 0.0));
        assert!(!cut_path(&mut path, 0));
    }

    /// `FUN_140f96050`: the top level decides, not the walked one; a level without a path has
    /// no more; else `count - 1 <= waypoint` in 32 bits says none.
    #[test]
    fn the_top_level_has_more_waypoints_unless_its_count_less_one_reaches_its_index() {
        let mut m = builder_9177();
        // the top level's two points, its waypoint 1: none after it, whatever the walked level has
        m.paths[0].waypoint = 0;
        assert!(!m.top_has_more());
        m.paths[1].waypoint = 0;
        assert!(m.top_has_more());
        // without a path: none
        m.paths[1].has_path = false;
        assert!(!m.top_has_more());
        // an empty path: -1 <= waypoint, but a waypoint below -1 has more
        m.paths[1].has_path = true;
        m.paths[1].points.clear();
        assert!(!m.top_has_more());
        m.paths[1].waypoint = -2;
        assert!(m.top_has_more());
        m.paths.clear();
        assert!(!m.top_has_more());
    }

    /// `FUN_140f9b8f0` compares `i` with the point count less one, signed: an empty path or the
    /// last point has no stretch to split; any other stretch gets its midpoint.
    #[test]
    fn a_split_takes_the_midpoint_of_a_stretch_and_skips_an_empty_path() {
        let mut empty = Path::default();
        split(&mut empty, 0);
        assert_eq!(empty.points, Vec::<Vec3>::new());
        let mut path = Path {
            points: vec![[0.0, 0.0, 0.0], [4.0, 0.0, 0.0]],
            length: 4.0,
        };
        split(&mut path, 1);
        assert_eq!(path.points.len(), 2);
        split(&mut path, 0);
        assert_eq!(
            path.points,
            [[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [4.0, 0.0, 0.0]]
        );
    }

    /// Slot 196 (`FUN_14114f400`) at 0x14114f520..0x14114f52d: a NaN distance to the waypoint
    /// goes past the first test and stops at the second (JBE on unordered): short.
    #[test]
    fn a_nan_waypoint_distance_is_short() {
        let w = [f32::NAN, 0.0, 5.0];
        assert!(matches!(reach(w, [0.0; 3], [1.0, 0.0, 0.0]), Reach::Short));
        assert!(matches!(
            reach([5.0, 0.0, 0.0], [0.0; 3], [1.0, 0.0, 0.0]),
            Reach::Short
        ));
        assert!(matches!(
            reach([5.0, 0.0, 0.0], [0.0; 3], [5.0, 0.0, 0.0]),
            Reach::Reached
        ));
    }

    /// The ground direction (`FUN_14114f210`): a NaN length divides (JBE on unordered), so the
    /// direction is NaN, not the forward vector.
    #[test]
    fn a_direction_of_a_nan_length_is_nan() {
        let m = crate::hunt::tests::walker();
        let d = m.direction_to([f32::NAN, 0.0, 0.0], [1.0, 0.0, 1.0]);
        assert!(d[0].is_nan() && d[2].is_nan());
        let d = m.direction_to([0.0, 0.0, 0.0], [3.0, 0.0, 4.0]);
        assert_eq!(d, [0.6, 0.0, 0.8]);
    }

    #[test]
    fn two_boxes_overlap_by_the_lesser_of_their_axes() {
        // the cow 9517 at (21.8, 92.2) and Gaia's cow 9519 at (22.5, 91.5), radii 0.5 and 0.5
        let a = [21.8, 0.0, 92.2];
        let b = [22.5, 0.0, 91.5];
        let d = overlap(a, [0.5, 0.5], b, [0.5, 0.5]);
        assert_eq!(d, (21.8f32 + 0.5) - (22.5 - 0.5));
        assert_eq!(overlap(a, [0.2, 0.2], b, [0.2, 0.2]), 0.0);
        // a NaN goes past the four tests (COMISS/JNC) into MINSS/MAXSS, whose second operand wins
        let nan = overlap([f32::NAN, 0.0, 92.2], [0.5, 0.5], b, [0.5, 0.5]);
        assert_eq!(
            nan,
            fmath::minss(
                (22.5f32 + 0.5) - (22.5 - 0.5),
                (91.5f32 + 0.5) - (92.2 - 0.5)
            )
        );
    }

    #[test]
    fn level_0_looks_ahead_to_the_farthest_parent_waypoint_it_walks_to_straight() {
        // a cow at its level 0 waypoint (21.8, 92.2), its level 1 parent's waypoint 1 of three
        let mut m = builder_9177();
        m.paths = vec![
            level(0, &[[21.5, 91.5], [21.8, 92.2], [25.15, 92.85]], 3),
            level(
                1,
                &[[21.5, 91.5], [28.5, 93.5], [28.9, 94.1], [40.0, 94.0]],
                0,
            ),
        ];
        m.paths[1].waypoint = 1;
        let me = Me {
            id: 9517,
            position: [21.8, 92.2, 0.0],
            group: -1,
            action_state: 0,
        };
        // land of a terrain class the unit's mask (6) has
        let mut w = Spatial::with_tiles(
            [120, 120],
            crate::spatial::Tiles::new(120, 120, vec![2; 14400]),
        );
        // on open ground: waypoint 3 is beyond 10 tiles, waypoint 2 within, so it takes 2
        assert_eq!(m.look_ahead(&me, &w), Ok(true));
        assert_eq!(m.paths.len(), 1);
        assert_eq!(
            (m.paths[0].level, m.paths[0].waypoint, m.paths[0].state),
            (1, 2, 2)
        );
        // a tree in the way of both leaves the levels as they are
        let mut m = builder_9177();
        m.paths = vec![
            level(0, &[[21.5, 91.5], [21.8, 92.2]], 3),
            level(1, &[[21.5, 91.5], [28.5, 93.5], [28.9, 94.1]], 0),
        ];
        m.paths[1].waypoint = 1;
        w.insert(
            9357,
            Placed {
                position: [25.0, 93.0, 0.0],
                radius: [0.5, 0.5],
                class: 15,
                player: 0,
                master: 0,
                obstruction_type: 2,
                buildable_on: 0,
                tall: true,
                speed: 0.0,
                unknown: None,
                obstruction: Some(Obstruction { kind: 2, class: 1 }),
            },
        );
        assert_eq!(m.look_ahead(&me, &w), Ok(false));
        assert_eq!(m.paths.len(), 2);
    }

    /// A trade cart plans only within its owner's budget; any unit at most three times a step.
    #[test]
    fn a_trade_cart_plans_within_its_owners_budget() {
        let spent = PlanBudget {
            attempt_cap: 6,
            delay_cap: 2,
            plans: 6,
        };
        assert!(!may_plan(true, Some(spent), 0, 0));
        // a villager is not held to the budget
        assert!(may_plan(false, Some(spent), 0, 0));
        // past the delay cap the budget lets the cart plan again
        assert!(may_plan(true, Some(spent), 3, 0));
        assert!(may_plan(true, Some(PlanBudget { plans: 5, ..spent }), 0, 0));
        assert!(may_plan(true, None, 0, 0));
        assert!(!may_plan(false, None, 0, PLANS));
    }

    /// `FUN_140f9eb70`'s list for `FUN_140f60440`: a walker in no group lists itself and its
    /// ignore units that move (not one that stands, not the target, not one the world lacks),
    /// then the target; a walker in a group lists its group's differentiator units instead.
    #[test]
    fn the_long_pather_lowers_the_walker_or_its_groups_units_and_the_target() {
        let mut s = world();
        let moving = Gait {
            moving: true,
            ..Gait::default()
        };
        s.set_gait(9175, Some(moving));
        s.set_gait(10028, Some(moving));
        let ignore = [9175, 9177, 10028, 4242];
        let mut walker = LongWalker {
            id: 9177,
            position: [28.0, 89.0, 0.0],
            radius: [0.2, 0.2],
            mask: 1,
            moves: true,
            ignore_units: &ignore,
            group_units: None,
        };
        // 10028 moves but is the target: it comes last, once
        assert_eq!(lowered_units(&walker, &s, Some(10028)), [9177, 9175, 10028]);
        assert_eq!(lowered_units(&walker, &s, None), [9177, 9175, 10028]);
        walker.group_units = Some(vec![9177, 31, 32]);
        assert_eq!(
            lowered_units(&walker, &s, Some(10028)),
            [9177, 31, 32, 10028]
        );
        // a group whose differentiator groups are gone lists none, not even the walker
        walker.group_units = Some(Vec::new());
        assert_eq!(lowered_units(&walker, &s, None), Vec::<i32>::new());
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
        let at = m.step_velocity(at, DT, world.map, None, 0);
        world.set_position(9177, at);
        let motion = m
            .motion(
                &mut Me {
                    id: 9177,
                    position: at,
                    group: -1,
                    action_state: 0,
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
        assert_eq!(m.forward, [0.894_427_2, 0.0, -0.447_213_62]);
    }

    /// `moveForward` (`FUN_141158490`), slot 199 (`FUN_141161860`) and slot 198
    /// (`FUN_141150a80`) start from `+0x824`, where the unit last moved to, not from the place
    /// the caller holds.
    #[test]
    fn the_step_and_the_speed_control_start_where_the_unit_last_moved_to() {
        let mut m = builder_9177();
        m.forward = [1.0, 0.0, 0.0];
        m.moved_at = [10.0, 20.0, 0.0];
        m.velocity.as_mut().unwrap().velocity = [0.0, 0.0, 0.5];
        m.velocity.as_mut().unwrap().acceleration = [0.0; 3];
        // the step: 0.5 tiles a second for a second, from (10, 20) whatever the caller holds
        let at = m.step_velocity([40.0, 40.0, 0.0], 1.0, [120, 120], None, 0);
        assert_eq!(at, [10.5, 20.0, 0.0]);
        assert_eq!(m.moved_at, [10.5, 20.0, 0.0]);
        // the projection along +x: 0.5 tiles from (10.5, 20)
        assert_eq!(m.projected([1.0, 0.0, 0.0], 1.0), [11.0, 0.0, 20.0]);
        // the speed control's turning circle around (10.5, 20) grown to the right by 1: a
        // waypoint inside it brakes, one beyond it speeds up
        m.stats.turn_radius = 2.0;
        m.stats.turn_radius_speed = 1.0;
        m.right = [0.0, 0.0, -1.0];
        m.control_speed([10.5, 0.0, 19.2], false);
        assert_eq!(m.velocity.as_ref().unwrap().acceleration[2], -f32::MAX);
        m.velocity.as_mut().unwrap().acceleration = [0.0; 3];
        m.control_speed([12.0, 0.0, 17.0], false);
        assert_eq!(m.velocity.as_ref().unwrap().acceleration[2], f32::MAX);
    }

    #[test]
    fn the_second_tick_turns_toward_the_waypoint_from_where_the_first_step_ended() {
        let mut m = builder_9177();
        let mut w = world();
        let (at, _) = tick(&mut m, [28.0, 89.0, 0.0], &mut w);
        let (at, _) = tick(&mut m, at, &mut w);
        assert_eq!(at, [28.028_622, 88.98569, 0.0]);
        // after update 682 the game's forward vector points from there to (30, 88)
        assert_eq!(m.forward, [0.894_581_56, 0.0, -0.446_904_78]);
        assert_eq!(m.right, [-0.446_904_78, 0.0, -0.894_581_56]);
        let (at, _) = tick(&mut m, at, &mut w);
        assert_eq!(at, [28.042_934, 88.97854, 0.0]);
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
            group: -1,
            action_state: 0,
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
        m.moved_at = [29.51, 89.49, 0.0];
        let mut me = Me {
            id: 9175,
            position: [29.51, 89.49, 0.0],
            group: -1,
            action_state: 0,
        };
        assert_eq!(m.motion(&mut me, DT, &mut world, None), Ok(Motion::Moving));
        assert_eq!(m.paths[0].flags, 3);
        assert_eq!(m.motion(&mut me, DT, &mut world, None), Ok(Motion::Arrived));
        assert_eq!(me.position, [29.5, 89.5, 0.0]);
        assert_eq!(m.paths.len(), 1);
        assert_eq!(m.paths[0].state, 0);
    }

    /// MarvinBlue/426048731's 9175 in update 682: the path request of its `MoveTo` off the house
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
            group: -1,
            action_state: 0,
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
            group: -1,
            action_state: 0,
        };
        assert_eq!(m.motion(&mut me, DT, &mut world(), None), Ok(Motion::Done));
        let mut m = builder_9177();
        m.paths.clear();
        assert_eq!(m.motion(&mut me, DT, &mut world(), None), Ok(Motion::Done));
    }

    /// The 4v4's hawk 30594 in update 2007: its level 0 arrived under a top level without a path
    /// (a bird's); the parent takes the reached mark and the step ends there, the unit flying on.
    #[test]
    fn an_arrived_level_under_one_without_a_path_ends_the_step() {
        let mut m = builder_9177();
        m.velocity.as_mut().unwrap().velocity = [0.0, 0.0, 1.32];
        m.paths = vec![
            level(0, &[[28.0, 89.0], [30.0, 88.0]], 4),
            PathData {
                has_path: false,
                path_flags: 0,
                created: 0,
                ..level(2, &[], 0)
            },
        ];
        for l in &mut m.paths {
            l.target = -1;
        }
        let mut me = Me {
            id: 9177,
            position: [30.0, 88.0, 0.0],
            group: -1,
            action_state: 0,
        };
        assert_eq!(m.motion(&mut me, DT, &mut world(), None), Ok(Motion::Done));
        assert_eq!(m.paths.len(), 1);
        assert_eq!((m.paths[0].flags, m.paths[0].state), (1, 0));
        assert_eq!(m.velocity.as_ref().unwrap().velocity, [0.0, 0.0, 1.32]);
    }

    #[test]
    fn a_human_walker_pauses_a_met_unit_of_an_older_human_order() {
        // 9177 meets 9175 of its own player: ordered at the same time, the lower id outranks
        // and 9177 walks on; once 9175's last human order is the older, 9177 pauses it for dt
        // more than its pause (slot 0x5f0)
        let mut w = world();
        let mut close = *w.get(9175).unwrap();
        close.position = [28.3, 88.9, 0.0];
        w.insert(9175, close);
        let me = Me {
            id: 9177,
            position: [28.0, 89.0, 0.0],
            group: -1,
            action_state: 0,
        };
        let walker = |human_order| Gait {
            human_order,
            ..Gait::default()
        };
        w.set_gait(9177, Some(walker(17000)));
        w.set_gait(
            9175,
            Some(Gait {
                pause: Some(0.0),
                ..walker(17000)
            }),
        );
        let mut walker_9177 = builder_9177();
        assert!(
            walker_9177
                .motion(&mut me.clone(), DT, &mut w, None)
                .is_ok()
        );
        assert_eq!(walker_9177.pauses, Vec::<(i32, f32)>::new());
        w.set_gait(
            9175,
            Some(Gait {
                pause: Some(0.0),
                ..walker(16000)
            }),
        );
        let mut walker_9177 = builder_9177();
        assert!(
            walker_9177
                .motion(&mut me.clone(), DT, &mut w, None)
                .is_ok()
        );
        assert_eq!(walker_9177.pauses, [(9175, 0.0 + DT)]);
    }
}
