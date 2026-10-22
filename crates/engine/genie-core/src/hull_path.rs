//! The obstruction manager's old short pather (`MShortPather`, the names of the `AoK HD` PDB; build
//! 158041's `FindShortPath` is `FUN_140f85580`): a search around convex hulls of the obstructions
//! near a straight walk, for the length of a unit's short path to an object (slot 0x3e8).
//!
//! The pather first tries the quick path (`findQuickPath`): the straight walk, or one detour
//! around a single square obstruction. Else it wraps the obstructions of the tiles about the walk
//! (the area: the walk's tiles and two more on each side) into hulls (`buildHulls`): each run of
//! blocked tiles of a row is one box, each tall object on a tile its box, all grown by the unit's
//! radius and merged into a hull with every hull they overlap (`addShapeToHulls`, a gift wrap of
//! both point sets: `MergePointsIntoHull`). A start inside a hull walks out of it first along the
//! sides of the boxes (`pathWithinHull`, the exit paths), a goal inside one is reached the same
//! way backwards (the entrance paths, whose ends join the hull's points); then an A* over the
//! hulls' points: a node that sees the goal ends the search, else its walk to the goal crosses the
//! nearest hull, whose points it follows both ways around. A node beyond the area grows the area
//! by two tiles each way, once. The path found is smoothed (`buildPath`: a node that sees its
//! grandparent skips its parent).
//!
//! Every line test is the obstruction manager's (flags 0x40: moving objects do not count), with the
//! unit's radius halved as the buffer, and the pather's ignore list: the unit, its target and the
//! tile under a one-tile target that the unit's terrain cannot cross.

use crate::fmath::sqrt;
use crate::spatial::{Placed, Spatial, tile_record_id};
use crate::unit_ai::Unsupported;

/// A point on the map, x and y.
type Point = [f32; 2];

/// `MShortPather::IterationCap` (`+0x18`, 0x40 from the constructor `FUN_140f84e40`): how many
/// nodes the search expands.
const ITERATION_CAP: i32 = 0x40;
/// How many nodes a walk out of a hull or into one expands (`pathWithinHull`'s cap, 0x40).
const HULL_ITERATIONS: i32 = 0x40;
/// The exit and the entrance paths the search keeps, each (the pather's `+0x108` and `+0x110`).
const PATHS: usize = 4;
/// How far the area may grow before the search gives up: a grown area of more than 100 tiles.
const MOST_AREA: i32 = 100;
/// How many times the area grows (`FindShortPath`'s loop: twice ends it).
const GROWTHS: i32 = 2;

/// `DAT_142e7f384`: half (the buffer is half the radius; a box's middle).
const HALF: f32 = crate::image::hull_path::HALF;
/// `DAT_142f43288`: the nudge toward a hull's middle, off a box's side, and the range's slack.
const NUDGE: f32 = crate::image::hull_path::NUDGE;
/// `DAT_142eb0c60`: two coordinates this close are the same point.
const EPSILON: f32 = crate::image::hull_path::EPSILON;
/// `DAT_14330eb8c`: one (a direction's length; the intersection's inverse).
const ONE: f32 = crate::image::hull_path::ONE;
/// `DAT_142ed55d4`, `DAT_142f4328c`: the parameters within which two segments meet.
const T_LOW: f32 = crate::image::hull_path::T_LOW;
const T_HIGH: f32 = crate::image::hull_path::T_HIGH;
/// `DAT_142f43298`: the intersection search's first distance.
const FAR: f32 = crate::image::hull_path::FAR;
/// `DAT_14330ebf4`: the largest float, the first distance of the ray searches.
const MOST: f32 = crate::image::hull_path::MOST;
/// `DAT_142eb99cc`: two exit or entrance paths ending this close end at the same point.
const SAME_END: f32 = crate::image::hull_path::SAME_END;
/// `DAT_142ef9004`, `DAT_142eb880c`: the step along a box's side, of its length, of the walk out of
/// a hull and of the walk into one.
const EXIT_STEP: f32 = crate::image::hull_path::EXIT_STEP;
const ENTRANCE_STEP: f32 = crate::image::hull_path::ENTRANCE_STEP;
/// `DAT_142f43290`: the square root of 2, a detour point's distance from a square's middle.
const SQRT_2: f32 = crate::image::hull_path::SQRT_2;
/// `DAT_142eb0f48`: within 2 tiles (squared) of a moving target the quick path stops short of it.
const NEAR_TARGET: f32 = crate::image::hull_path::NEAR_TARGET;
/// The gift wrap's pseudo-angle (`MergePointsIntoHull`): none yet (`DAT_14330ec20`), the quadrants
/// added left of the y axis and below the x axis (`DAT_142e8c150` 2, `DAT_142eb0f48` 4), the degrees
/// of one (`DAT_142f43294`) and a turn (`DAT_142f02860`, `DAT_142f4329c`).
const NO_ANGLE: f32 = crate::image::hull_path::NO_ANGLE;
const HALF_TURN: f32 = crate::image::hull_path::HALF_TURN;
const FULL_TURN: f32 = crate::image::hull_path::FULL_TURN;
const QUADRANT: f32 = crate::image::hull_path::QUADRANT;
const TURN: f32 = crate::image::hull_path::TURN;
const UNTURN: f32 = crate::image::hull_path::UNTURN;
/// `DAT_1439e6870`: the 16 directions of the first steps out of a box (the game's table: its 12th
/// repeats the 11th).
const DIRECTIONS: [[f32; 2]; 16] = crate::image::hull_path::DIRECTIONS;
/// `DAT_1439e68f0`: the four ray directions of a walk out of a hull or into one (down, up, left,
/// right).
const SIDES: [[f32; 2]; 4] = crate::image::hull_path::SIDES;

/// A box in a hull (`ObstructionInfo`, 0x14 bytes): an obstruction's box grown by the unit's
/// radius and the obstruction's id (an object's, or a tile's [`tile_record_id`]).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Shape {
    bounds: [f32; 4],
    id: i32,
}

impl Shape {
    /// Whether `p` lies inside the box, its sides excluded (`FUN_140f89dc0`'s test).
    fn holds(&self, p: Point) -> bool {
        let [x0, y0, x1, y1] = self.bounds;
        x0 < p[0] && p[0] < x1 && y0 < p[1] && p[1] < y1
    }
}

/// A convex hull (`MConvexHull`, 0x40 bytes): its points, the entrance path ending at each point
/// (`+0x18`, the goal's hull only), its bounding box (`+0x20`) and its boxes (`+0x30`).
#[derive(Clone, Debug, Default, PartialEq)]
struct Hull {
    points: Vec<Point>,
    entrances: Vec<i32>,
    min: Point,
    max: Point,
    objects: Vec<Shape>,
}

impl Hull {
    /// `MConvexHull::InsideHull` (`FUN_140f87760`): whether `p`, moved a little toward the bounding
    /// box's middle, lies inside the hull (the crossings of a ray to the right).
    fn inside(&self, p: Point) -> bool {
        let px = p[0] + ((self.max[0] + self.min[0]) * HALF - p[0]) * NUDGE;
        let py = p[1] + ((self.max[1] + self.min[1]) * HALF - p[1]) * NUDGE;
        let n = self.points.len();
        let mut inside = false;
        let mut j = n.wrapping_sub(1);
        for i in 0..n {
            let ([xi, yi], [xj, yj]) = (self.points[i], self.points[j]);
            if ((yi <= py && py < yj) || (yj <= py && py < yi))
                && px < ((py - yi) * (xj - xi)) / (yj - yi) + xi
            {
                inside = !inside;
            }
            j = i;
        }
        inside
    }

    /// `MConvexHull::getHullIntersection` (`FUN_140f879e0`): the nearest point where the walk
    /// `a` -> `b` meets the hull's sides, and the side (from point `k - 1` to point `k`, the last
    /// side 0); unless `at_start`, a meeting at `a` itself does not count.
    fn intersection(&self, at_start: bool, a: Point, b: Point) -> Option<(Point, usize)> {
        let n = self.points.len();
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let mut best = FAR;
        let mut found: Option<(Point, usize)> = None;
        for k in 1..=n {
            let p0 = self.points[k - 1];
            let p1 = if k == n {
                self.points[0]
            } else {
                self.points[k]
            };
            let (ex, ey) = (p1[0] - p0[0], p1[1] - p0[1]);
            let (rx, ry) = (a[0] - p0[0], a[1] - p0[1]);
            let denom = dx * ey - dy * ex;
            if denom.abs() < EPSILON {
                continue;
            }
            let inv = ONE / denom;
            let t = inv * (ry * ex - rx * ey);
            let u = (ry * dx - dy * rx) * inv;
            if !((T_LOW..=T_HIGH).contains(&t) && (T_LOW..=T_HIGH).contains(&u)) {
                continue;
            }
            let p = [dx * t + a[0], dy * t + a[1]];
            if !at_start && (p[0] - a[0]).abs() < EPSILON && (p[1] - a[1]).abs() < EPSILON {
                continue;
            }
            let d = (p[1] - a[1]) * (p[1] - a[1]) + (p[0] - a[0]) * (p[0] - a[0]);
            if found.is_none() || d < best {
                found = Some((p, k));
                best = d;
            }
        }
        found.map(|(p, k)| (p, if k == n { 0 } else { k }))
    }

    /// `MConvexHull::DoesShapeIntersect` (`FUN_140f85330`) for the box `min`..`max`: 3 when the
    /// box lies inside the hull, 2 when some of its corners do (with which do not, by corner: low
    /// left, low right, high right, high left), 1 when a hull point lies in the box or a box side
    /// meets the hull, else 0.
    fn shape_overlap(&self, min: Point, max: Point) -> (u8, [bool; 4]) {
        let ([x0, y0], [x1, y1]) = (min, max);
        let mut outside = [true; 4];
        if !(x0 <= self.max[0] && y0 <= self.max[1] && self.min[0] <= x1 && self.min[1] <= y1) {
            return (0, outside);
        }
        let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        let mut inside = 0;
        for (k, c) in corners.iter().enumerate() {
            if self.inside(*c) {
                inside += 1;
                outside[k] = false;
            }
        }
        if inside == 4 {
            return (3, outside);
        }
        if inside != 0 {
            return (2, outside);
        }
        if self
            .points
            .iter()
            .any(|p| x0 <= p[0] && p[0] <= x1 && y0 <= p[1] && p[1] <= y1)
        {
            return (1, outside);
        }
        let sides = [
            ([x0, y0], [x1, y0]),
            ([x1, y1], [x1, y0]),
            ([x1, y1], [x0, y1]),
            ([x0, y0], [x0, y1]),
        ];
        let meets = sides
            .iter()
            .any(|&(a, b)| self.intersection(true, a, b).is_some());
        (u8::from(meets), outside)
    }

    /// `MConvexHull::MergePointsIntoHull` (`FUN_140f87ca0`): the hull of its points and `more` (a
    /// new point the same as one of the hull's within [`EPSILON`] is left out; the new points are
    /// not compared with each other): from the highest point (the rightmost of the highest), a
    /// gift wrap by the largest pseudo-angle turned by the last step's (the farthest of equals),
    /// until no point is left or the first point turns no less.
    fn merge(&mut self, more: &[Point]) {
        let old = self.points.len();
        let mut scratch = self.points.clone();
        for p in more {
            let seen = self.points[..old]
                .iter()
                .any(|s| (p[0] - s[0]).abs() < EPSILON && (p[1] - s[1]).abs() < EPSILON);
            if !seen {
                scratch.push(*p);
            }
        }
        if scratch.len() > self.entrances.len() {
            self.entrances.resize(scratch.len(), -1);
        }
        let mut k = 0;
        let mut first = scratch[0];
        for (i, s) in scratch.iter().enumerate() {
            if first[1] <= s[1] && (first[1] < s[1] || first[0] < s[0]) {
                first = *s;
                k = i;
            }
        }
        self.min = scratch[k];
        self.max = scratch[k];
        self.points.clear();
        let mut turned = 0.0;
        let mut step = 0.0;
        let mut left = scratch.len();
        loop {
            let cur = scratch[k];
            self.points.push(cur);
            if cur[0] < self.min[0] {
                self.min[0] = cur[0];
            }
            if cur[1] < self.min[1] {
                self.min[1] = cur[1];
            }
            if self.max[0] < cur[0] {
                self.max[0] = cur[0];
            }
            if self.max[1] < cur[1] {
                self.max[1] = cur[1];
            }
            if k < left - 1 {
                scratch[k] = scratch[left - 1];
            }
            left -= 1;
            if left == 0 {
                return;
            }
            let mut best: Option<usize> = None;
            let mut best_angle = NO_ANGLE;
            let mut best_d = NO_ANGLE;
            for (i, s) in scratch[..left].iter().enumerate() {
                let a = pseudo_angle(s[0] - cur[0], s[1] - cur[1]);
                let angle = turn(a * QUADRANT + turned);
                if best_angle <= angle {
                    let d = (cur[1] - s[1]) * (cur[1] - s[1]) + (cur[0] - s[0]) * (cur[0] - s[0]);
                    if angle <= best_angle {
                        if d <= best_d {
                            continue;
                        }
                    } else {
                        best_angle = angle;
                    }
                    best_d = d;
                    step = a * QUADRANT;
                    best = Some(i);
                }
            }
            let Some(next) = best else {
                return;
            };
            if 1 < self.points.len() {
                let a = pseudo_angle(first[0] - cur[0], first[1] - cur[1]);
                let angle = turn(a * QUADRANT + turned);
                if best_angle < angle {
                    return;
                }
                let d = (cur[1] - first[1]) * (cur[1] - first[1])
                    + (cur[0] - first[0]) * (cur[0] - first[0]);
                if angle == best_angle && best_d < d {
                    return;
                }
            }
            turned = TURN - step;
            k = next;
        }
    }

    /// `FUN_140f89dc0`: the first box holding `p` (sides excluded).
    fn box_holding(&self, p: Point) -> Option<usize> {
        self.objects.iter().position(|o| o.holds(p))
    }

    /// `FUN_140f8ea30`: the nearest box side a ray from `p` meets toward `side` (0 down, 1 up,
    /// 2 left, 3 right), with the box and the box's side (0 its top, 1 its right, 2 its bottom,
    /// 3 its left: the far side when `p` is inside the box). The box `own` counts only strictly
    /// beyond `p`.
    fn ray_to_box(&self, p: Point, side: i32, own: i32) -> Option<(Point, usize, i32)> {
        let mut best = MOST;
        let mut found = None;
        for (k, o) in self.objects.iter().enumerate() {
            let [x0, y0, x1, y1] = o.bounds;
            let mine = k as i32 == own;
            let hit = match side {
                0 if x0 <= p[0] && p[0] <= x1 => {
                    let past = if mine { y1 < p[1] } else { y1 <= p[1] };
                    if past {
                        Some((y1, 0, p[1] - y1))
                    } else if p[1] <= y0 {
                        None
                    } else {
                        Some((y0, 2, p[1] - y0))
                    }
                }
                1 if x0 <= p[0] && p[0] <= x1 => {
                    let past = if mine { p[1] < y0 } else { p[1] <= y0 };
                    if past {
                        Some((y0, 2, y0 - p[1]))
                    } else if y1 <= p[1] {
                        None
                    } else {
                        Some((y1, 0, y1 - p[1]))
                    }
                }
                2 if y0 <= p[1] && p[1] <= y1 => {
                    let past = if mine { x1 < p[0] } else { x1 <= p[0] };
                    if past {
                        Some((x1, 1, p[0] - x1))
                    } else if p[0] <= x0 {
                        None
                    } else {
                        Some((x0, 3, p[0] - x0))
                    }
                }
                3 if y0 <= p[1] && p[1] <= y1 => {
                    let past = if mine { p[0] < x0 } else { p[0] <= x0 };
                    if past {
                        Some((x0, 3, x0 - p[0]))
                    } else if x1 <= p[0] {
                        None
                    } else {
                        Some((x1, 1, x1 - p[0]))
                    }
                }
                _ => None,
            };
            if let Some((edge, box_side, d)) = hit
                && d < best
            {
                best = d;
                let at = if side < 2 { [p[0], edge] } else { [edge, p[1]] };
                found = Some((at, k, box_side));
            }
        }
        found
    }

    /// `FUN_140f8e530`: where a ray from `p` toward `side` leaves the hull, and the hull side
    /// (from point `k - 1` to point `k`) it leaves by; along a side parallel to the ray, at `p`
    /// itself while `p` is within the side's span.
    fn ray_to_hull(&self, p: Point, side: i32) -> Option<(Point, usize)> {
        let n = self.points.len();
        let mut best = MOST;
        let mut found = None;
        // the ray's axis (a), the other (b), and whether it runs toward smaller values
        let (a, b, down) = match side {
            0 => (1, 0, true),
            1 => (1, 0, false),
            2 => (0, 1, true),
            3 => (0, 1, false),
            _ => return None,
        };
        let mut j = n.wrapping_sub(1);
        for i in 0..n {
            let (pi, pj) = (self.points[i], self.points[j]);
            let (bi, bj) = (pi[b], pj[b]);
            let (lo_b, hi_b) = if bi < bj { (bi, bj) } else { (bj, bi) };
            if lo_b <= p[b] && p[b] <= hi_b {
                let (ai, aj) = (pi[a], pj[a]);
                let sloped = EPSILON <= (bj - bi).abs();
                let at = |q: f32| ((q - bi) / (bj - bi)) * (aj - ai) + ai;
                let (lo, hi) = if ai < aj { (ai, aj) } else { (aj, ai) };
                let hit = if down {
                    if lo <= p[a] {
                        if sloped {
                            let y = at(p[b]);
                            (y <= p[a]).then_some(y)
                        } else if p[a] < hi {
                            Some(p[a])
                        } else {
                            Some(hi)
                        }
                    } else {
                        None
                    }
                } else {
                    if p[a] <= hi {
                        if sloped {
                            let y = at(p[b]);
                            (p[a] <= y).then_some(y)
                        } else if lo < p[a] {
                            Some(p[a])
                        } else {
                            Some(lo)
                        }
                    } else {
                        None
                    }
                };
                if let Some(e) = hit {
                    let d = if down { p[a] - e } else { e - p[a] };
                    if d < best {
                        best = d;
                        let mut q = p;
                        q[a] = e;
                        found = Some((q, i));
                    }
                }
            }
            j = i;
        }
        found
    }
}

/// The gift wrap's pseudo-angle of a step, in quadrants (0 to 4, counterclockwise from the x
/// axis): the diamond angle of `MergePointsIntoHull`.
fn pseudo_angle(dx: f32, dy: f32) -> f32 {
    let t = dy.abs() + dx.abs();
    let a = if t == 0.0 { 0.0 } else { dy / t };
    if 0.0 <= dx {
        if dy < 0.0 { a + FULL_TURN } else { a }
    } else {
        HALF_TURN - a
    }
}

/// An angle past a full turn turns back once.
fn turn(angle: f32) -> f32 {
    if TURN < angle { angle + UNTURN } else { angle }
}

/// A search node (`PathingNode`, 0x38 bytes).
#[derive(Clone, Copy, Debug)]
struct Node {
    at: Point,
    parent: Option<usize>,
    next: Option<usize>,
    prev: Option<usize>,
    /// The straight distance to the goal (`+0x20`).
    to_goal: f32,
    /// The walk from the start (`+0x24`).
    cost: f32,
    /// The hull of a hull point; in a walk out of a hull, the box the node lies on (`+0x28`).
    hull: i32,
    /// The point of that hull; in a walk out of a hull, the box's side (`+0x2c`).
    vertex: i32,
    /// The ray that made the node in a walk out of a hull (`+0x30`).
    direction: i32,
}

impl Node {
    const fn head() -> Self {
        Self {
            at: [0.0, 0.0],
            parent: None,
            next: None,
            prev: None,
            to_goal: 0.0,
            cost: 0.0,
            hull: -1,
            vertex: -1,
            direction: -1,
        }
    }
}

/// A path (`BPath`): its waypoints (x, y; the game keeps a height of 0 between) and the cost the
/// search gave it (`+4`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Waypoints {
    /// The waypoints.
    pub points: Vec<Point>,
    /// The cost.
    pub cost: f32,
}

impl Waypoints {
    /// `BPath::InsertWaypoint` (`FUN_140f95060`): a waypoint at `index`, unless the one before or
    /// at it is the same point.
    fn insert(&mut self, index: usize, p: Point) {
        if 0 < index && self.points.get(index - 1) == Some(&p) {
            return;
        }
        if self.points.get(index) == Some(&p) {
            return;
        }
        self.points.insert(index, p);
    }

    /// `FUN_140f95a10`: a waypoint at the end.
    fn push(&mut self, p: Point) {
        self.insert(self.points.len(), p);
    }

    /// `BPath::calculatePathLength` (`FUN_140f95a60`): the waypoints' distances summed.
    #[must_use]
    pub fn length(&self) -> f32 {
        let mut total = 0.0;
        for w in self.points.windows(2) {
            let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
            total += sqrt(dx * dx + 0.0 * 0.0 + dy * dy);
        }
        total
    }
}

/// What the pather knows of its target: its id, place and radii.
#[derive(Clone, Copy, Debug)]
struct Target {
    id: i32,
    at: Point,
    radius: [f32; 2],
    speed: f32,
}

/// A walk out of a hull or into one (`pathWithinHull`): its result and whether a ray left the
/// hull's boxes (`*param_18`).
struct Within {
    result: u8,
    left_boxes: bool,
}

/// The old short pather over a world, for one unit: its terrain mask, player and radius, and the
/// ignore list.
#[derive(Debug)]
pub struct HullPather<'w> {
    world: &'w Spatial,
    mask: u8,
    radius: f32,
    /// The manager's buffer for the line tests (`+0x57`, `+0x58`): half the radius.
    buffer: Option<f32>,
    /// The ignore list (`+0xc0` on, `+0xc4` count, `+0xd0` ids), which the manager's tests read.
    ignore: Vec<i32>,
    hulls: Vec<Hull>,
    /// The nodes; 0 and 1 head the open lists (`+0x20`, `+0x58`).
    nodes: Vec<Node>,
    /// The closed lists (`+0x90`, `+0xa0`).
    closed: [Vec<usize>; 2],
    /// The area x0, y0, x1, y1 in tiles (`+0xe8`).
    area: [i32; 4],
    start: Point,
    goal: Point,
    exits: Vec<Waypoints>,
    entrances: Vec<Waypoints>,
    /// The map less the radius, x0, y0, x1, y1 (`+0x118`).
    bounds: [f32; 4],
}

/// `canQuickPath` (object slot 0x3e0, `FUN_1411506e0`) of object `me` (its master's terrain
/// `mask` and `radius` on x) to object `target` within `range`: the old short pather's quick path
/// alone ([`HullPather::find_quick`]) from its place to the target's, ignoring both, finds it
/// there or reaches it (a result below 2); false for a target the world does not hold.
///
/// # Errors
///
/// Where the pather meets what the slice does not model.
pub fn quick_reach(
    world: &Spatial,
    me: i32,
    target: i32,
    mask: u8,
    radius: f32,
    range: f32,
) -> Result<bool, Unsupported> {
    let Some(t) = world.get_known(target)? else {
        return Ok(false);
    };
    let from = world
        .get(me)
        .ok_or(Unsupported("a quick path of an object without a place"))?
        .position;
    let mut pather = HullPather::new(world, me, target, mask, radius);
    let to = [t.position[0], t.position[1]];
    let (result, _) = pather.find_quick([from[0], from[1]], to, target, range)?;
    Ok(result < 2)
}

impl<'w> HullPather<'w> {
    /// The pather for unit `unit` (terrain `mask`, `radius`) with the ignore list slot 0x3e8 sets
    /// (`FUN_140f89e40` with the unit and `target`): both objects, and the tile under a target of
    /// one tile on a tile's middle whose terrain the mask lacks.
    #[must_use]
    pub fn new(world: &'w Spatial, unit: i32, target: i32, mask: u8, radius: f32) -> Self {
        let mut ignore = vec![unit, target];
        for id in [unit, target] {
            let Some(p) = world.get(id) else {
                continue;
            };
            let [x, y, _] = p.position;
            if p.radius == [HALF, HALF] && x % ONE == HALF && y % ONE == HALF {
                let (tx, ty) = (x.floor() as i32, y.floor() as i32);
                let word = world.tiles().word(tx, ty).unwrap_or(0);
                if word & u64::from(mask ^ 0xff) != 0 {
                    ignore.push(tile_record_id(tx, ty));
                }
            }
        }
        Self {
            world,
            mask,
            radius,
            buffer: None,
            ignore,
            hulls: Vec::new(),
            nodes: vec![Node::head(), Node::head()],
            closed: [Vec::new(), Vec::new()],
            area: [-1, -1, -1, -1],
            start: [0.0, 0.0],
            goal: [0.0, 0.0],
            exits: vec![Waypoints::default(); PATHS],
            entrances: vec![Waypoints::default(); PATHS],
            bounds: [0.0; 4],
        }
    }

    // The node lists.

    /// `MShortPather::emptyLists` (`FUN_140f8c300`): list `l`'s nodes go.
    fn empty(&mut self, l: usize) {
        self.nodes[l].next = None;
        self.closed[l].clear();
    }

    /// `MShortPather::closeAllOpenNodes` (`FUN_140f8c130`): list `l`'s open nodes close, and every
    /// closed node forgets its hull point.
    fn close_all(&mut self, l: usize) {
        let mut n = self.nodes[l].next;
        while let Some(i) = n {
            self.closed[l].push(i);
            n = self.nodes[i].next;
        }
        self.nodes[l].next = None;
        for &i in &self.closed[l] {
            self.nodes[i].hull = -1;
            self.nodes[i].vertex = -1;
        }
    }

    /// `MShortPather::moveToClosedList` (`FUN_140f8ce00`).
    fn close(&mut self, l: usize, i: usize) {
        let (prev, next) = (self.nodes[i].prev, self.nodes[i].next);
        if let Some(p) = prev {
            self.nodes[p].next = next;
        }
        if let Some(n) = next {
            self.nodes[n].prev = prev;
        }
        self.closed[l].push(i);
    }

    /// `MShortPather::addNode` (`FUN_140f8aa10`): a node at `at` from `parent` (its cost the
    /// parent's plus the step; its distance to `goal`, 0 without), on list `l`: into the open list
    /// in order of cost plus distance, or straight into the closed list. An open node replaces an
    /// open one of the same hull point (`by_place`: within [`EPSILON`] of the same place) that
    /// costs more, which closes; it is not made when such a node costs no more, nor when a closed
    /// node does (`by_place`: of the closed list but its last).
    #[allow(clippy::too_many_arguments)]
    fn add_node(
        &mut self,
        l: usize,
        open: bool,
        parent: Option<usize>,
        at: Point,
        goal: Option<Point>,
        hull: i32,
        vertex: i32,
        direction: i32,
        by_place: bool,
    ) -> Option<usize> {
        let cost = parent.map_or(0.0, |p| {
            let q = self.nodes[p];
            sqrt((q.at[1] - at[1]) * (q.at[1] - at[1]) + (q.at[0] - at[0]) * (q.at[0] - at[0]))
                + q.cost
        });
        let to_goal = goal.map_or(0.0, |g| {
            sqrt((at[1] - g[1]) * (at[1] - g[1]) + (at[0] - g[0]) * (at[0] - g[0]))
        });
        let mut after: Option<usize> = None;
        if open && (by_place || (0 <= hull && 0 <= vertex)) {
            let same = |n: &Node| {
                if by_place {
                    (at[0] - n.at[0]).abs() < EPSILON && (at[1] - n.at[1]).abs() < EPSILON
                } else {
                    n.hull == hull && n.vertex == vertex
                }
            };
            let mut n = self.nodes[l].next;
            let mut replaced = false;
            while let Some(i) = n {
                let node = self.nodes[i];
                if same(&node) {
                    if node.cost <= cost {
                        return None;
                    }
                    // the worse node closes, and the closed list is not searched
                    self.close(l, i);
                    replaced = true;
                    break;
                }
                if after.is_none() && to_goal + cost < node.cost + node.to_goal {
                    after = node.prev;
                }
                n = node.next;
            }
            // the place test leaves the closed list's last node out
            let closed = if by_place {
                &self.closed[l][..self.closed[l].len().saturating_sub(1)]
            } else {
                &self.closed[l][..]
            };
            if !replaced
                && closed.iter().any(|&i| {
                    let node = &self.nodes[i];
                    same(node) && node.cost <= cost
                })
            {
                return None;
            }
        }
        let i = self.nodes.len();
        self.nodes.push(Node {
            at,
            parent,
            next: None,
            prev: None,
            to_goal,
            cost,
            hull,
            vertex,
            direction,
        });
        if !open {
            self.closed[l].push(i);
            return Some(i);
        }
        let mut s = after.unwrap_or(l);
        if after.is_none() {
            while let Some(n) = self.nodes[s].next {
                if self.nodes[n].cost + self.nodes[n].to_goal < to_goal + cost {
                    s = n;
                } else {
                    break;
                }
            }
        }
        let next = self.nodes[s].next;
        self.nodes[i].next = next;
        self.nodes[s].next = Some(i);
        self.nodes[i].prev = Some(s);
        if let Some(n) = next {
            self.nodes[n].prev = Some(i);
        }
        Some(i)
    }

    // The world's tests.

    /// The manager's line test with the pather's buffer and ignore list.
    fn blocked(&self, a: Point, b: Point) -> Result<bool, Unsupported> {
        self.world
            .pather_blocked(a, b, self.buffer, self.mask, &self.ignore)
    }

    /// `MShortPather::buildPath` (`FUN_140f8bcd0`): from `end` back, a node that sees its
    /// grandparent skips its parent; then the nodes from the first into `path`.
    fn build_path(&mut self, path: &mut Waypoints, end: usize) -> Result<(), Unsupported> {
        let mut cur = Some(end);
        while let Some(c) = cur {
            let Some(p) = self.nodes[c].parent else {
                break;
            };
            let Some(gp) = self.nodes[p].parent else {
                break;
            };
            if self.blocked(self.nodes[c].at, self.nodes[gp].at)? {
                cur = Some(p);
            } else {
                self.nodes[c].parent = Some(gp);
            }
        }
        let mut n = Some(end);
        while let Some(i) = n {
            path.insert(0, self.nodes[i].at);
            n = self.nodes[i].parent;
        }
        Ok(())
    }

    // The hulls.

    /// `MShortPather::addShapeToHulls` (`FUN_140f8af60`): the box `min`..`max` into the hulls:
    /// into the first it overlaps (its corners outside it, or all four when it only touches),
    /// which then takes in each hull it overlaps in turn; a box inside a hull changes nothing;
    /// else a new hull. Returns the hull's index.
    fn add_shape(&mut self, min: Point, max: Point) -> usize {
        let mut index = 0;
        let mut hull: Option<usize> = None;
        while index < self.hulls.len() {
            let (overlap, outside) = self.hulls[index].shape_overlap(min, max);
            let corners = [
                [min[0], min[1]],
                [max[0], min[1]],
                [max[0], max[1]],
                [min[0], max[1]],
            ];
            match overlap {
                1 => {
                    self.hulls[index].merge(&corners);
                    hull = Some(index);
                }
                2 => {
                    let some: Vec<Point> =
                        (0..4).filter(|&k| outside[k]).map(|k| corners[k]).collect();
                    self.hulls[index].merge(&some);
                    hull = Some(index);
                }
                3 => return index,
                _ => {
                    index += 1;
                    continue;
                }
            }
            break;
        }
        let Some(mut mine) = hull else {
            self.hulls.push(Hull {
                points: vec![
                    [max[0], max[1]],
                    [max[0], min[1]],
                    [min[0], min[1]],
                    [min[0], max[1]],
                ],
                entrances: vec![-1; 4],
                min,
                max,
                objects: Vec::new(),
            });
            return self.hulls.len() - 1;
        };
        'merging: loop {
            for j in 0..self.hulls.len() {
                if j == mine {
                    continue;
                }
                let (h, o) = (&self.hulls[mine], &self.hulls[j]);
                if !(h.min[0] <= o.max[0]
                    && h.min[1] <= o.max[1]
                    && o.min[0] <= h.max[0]
                    && o.min[1] <= h.max[1])
                {
                    continue;
                }
                let n = o.points.len();
                let overlaps = h.points.iter().any(|p| o.inside(*p))
                    || o.points.iter().any(|p| h.inside(*p))
                    // the other's sides k = 0..n-1 (`cmp edi, esi; jl` at 0x140f8b2c4, esi = n - 1)
                    || (0..n.saturating_sub(1)).any(|k| {
                        let prev = if k == 0 { n - 1 } else { k - 1 };
                        h.intersection(true, o.points[k], o.points[prev]).is_some()
                    });
                if !overlaps {
                    continue;
                }
                let other = self.hulls[j].clone();
                for s in &other.objects {
                    if !self.hulls[mine].objects.iter().any(|m| m.id == s.id) {
                        self.hulls[mine].objects.push(*s);
                    }
                }
                self.hulls[mine].merge(&other.points);
                let last = self.hulls.len() - 1;
                self.hulls.swap(j, last);
                let _ = self.hulls.pop();
                if mine == last {
                    mine = j;
                }
                continue 'merging;
            }
            return mine;
        }
    }

    /// `MShortPather::addTileObjects` (`FUN_140f8b460`): the tall objects of tile (`x`, `y`)'s
    /// list, in its order, but those ignored or moving (slot 0x5c8) and gates its player may pass
    /// (slot 0x678): each box, grown by the radius, into `hull` (when given and the object is at
    /// most a tile wide) or its own place in the hulls, once per hull.
    fn add_tile_objects(&mut self, x: i32, y: i32, hull: Option<usize>) -> Result<(), Unsupported> {
        let world = self.world;
        for &id in world.tiles().list(x, y) {
            let Some(p) = world.get(id) else {
                continue;
            };
            if !p.tall {
                continue;
            }
            if p.class == 39 {
                return Err(Unsupported(
                    "a gate in the old short pather's area (its players' passage, slot 0x678)",
                ));
            }
            if self.ignore.contains(&id) || world.gait(id).is_some_and(|g| g.moving) {
                continue;
            }
            self.add_box(id, p, hull);
        }
        Ok(())
    }

    /// One object's box into the hulls ([`HullPather::add_tile_objects`]).
    fn add_box(&mut self, id: i32, p: &Placed, hull: Option<usize>) {
        let r = self.radius;
        let [x, y, _] = p.position;
        let [rx, ry] = p.radius;
        let bounds = [(x - rx) - r, (y - ry) - r, x + rx + r, y + ry + r];
        let h = match hull {
            Some(h) if rx <= HALF && ry <= HALF => h,
            _ => self.add_shape([bounds[0], bounds[1]], [bounds[2], bounds[3]]),
        };
        let objects = &mut self.hulls[h].objects;
        if !objects.iter().any(|o| o.id == id) {
            objects.push(Shape { bounds, id });
        }
    }

    /// `MShortPather::processTileObstructions` (`FUN_140f8e1d0`): tile (`x`, `y`)'s box when its
    /// terrain is not the unit's (unless ignored), into `hull` or its own place, then its objects,
    /// and those of its building's anchor tile when that lies outside the `area`.
    fn process_tile(
        &mut self,
        x: i32,
        y: i32,
        area: [i32; 4],
        hull: Option<usize>,
    ) -> Result<(), Unsupported> {
        let [w, h] = self.world.map;
        if x < 0 || y < 0 || w <= x || h <= y {
            return Ok(());
        }
        let word = self.world.tiles().word(x, y).unwrap_or(0);
        if word & u64::from(self.mask ^ 0xff) != 0 {
            let id = tile_record_id(x, y);
            if !self.ignore.contains(&id) {
                let r = self.radius;
                let bounds = [
                    x as f32 - r,
                    y as f32 - r,
                    (x + 1) as f32 + r,
                    (y + 1) as f32 + r,
                ];
                let k = hull.unwrap_or_else(|| {
                    self.add_shape([bounds[0], bounds[1]], [bounds[2], bounds[3]])
                });
                let objects = &mut self.hulls[k].objects;
                if !objects.iter().any(|o| o.id == id) {
                    objects.push(Shape { bounds, id });
                }
            }
        }
        self.add_tile_objects(x, y, hull)?;
        let sight = self.world.sight.as_ref().ok_or(Unsupported(
            "the old short pather without the map's building links",
        ))?;
        if sight.link(x, y).0 & 0x60 != 0
            && let Some([ax, ay]) = sight.anchor_of(x, y)
            && (ax < area[0] || area[2] < ax || ay < area[1] || area[3] < ay)
        {
            self.add_tile_objects(ax, ay, hull)?;
        }
        Ok(())
    }

    /// `MShortPather::buildHulls` (`FUN_140f8b8f0`): the area's tiles row by row: a run of tiles
    /// the unit cannot cross (its terrain, or a static record's mark but a class 5 one) is one box
    /// whose tiles' boxes and small objects go into its hull; each other tile's objects go each to
    /// its own place. Then the cliffs about the area.
    fn build_hulls(&mut self) -> Result<(), Unsupported> {
        self.hulls.clear();
        let [w, h] = self.world.map;
        self.area[0] = self.area[0].max(0);
        self.area[1] = self.area[1].max(0);
        self.area[2] = self.area[2].min(w - 1);
        self.area[3] = self.area[3].min(h - 1);
        let r = self.radius;
        let not_mask = u64::from(self.mask ^ 0xff);
        for y in self.area[1]..=self.area[3] {
            let mut run: Option<i32> = None;
            let mut x = self.area[0];
            while x <= self.area[2] {
                let word = self.world.tiles().word(x, y).unwrap_or(0);
                let open =
                    word & not_mask == 0 && ((word >> 14) & 1 == 0 || word & 0x3f00 == 0x500);
                if open {
                    if let Some(x0) = run.take() {
                        self.add_run(x0, x, y, r)?;
                    }
                    self.process_tile(x, y, self.area, None)?;
                } else if run.is_none() {
                    run = Some(x);
                }
                x += 1;
            }
            if let Some(x0) = run {
                self.add_run(x0, x, y, r)?;
            }
        }
        self.process_cliffs()
    }

    /// A run of tiles `x0`..`x1` of row `y` the unit cannot cross: one box, then its tiles.
    fn add_run(&mut self, x0: i32, x1: i32, y: i32, r: f32) -> Result<(), Unsupported> {
        let min = [x0 as f32 - r, y as f32 - r];
        let max = [x1 as f32 + r, (y + 1) as f32 + r];
        let hull = self.add_shape(min, max);
        for t in x0..x1 {
            self.process_tile(t, y, self.area, Some(hull))?;
        }
        Ok(())
    }

    /// `MShortPather::processCliffs` (`FUN_140f8e020`): the objects of the cliff tiles (a static
    /// record of class 6) within two tiles about the area.
    fn process_cliffs(&mut self) -> Result<(), Unsupported> {
        let [w, h] = self.world.map;
        let [x0, y0, x1, y1] = self.area;
        let (ex0, ey0) = ((x0 - 2).max(0), (y0 - 2).max(0));
        let (ex1, ey1) = ((x1 + 2).min(w - 1), (y1 + 2).min(h - 1));
        for y in ey0..=ey1 {
            for x in (ex0..x0).chain(x1 + 1..=ex1) {
                self.cliff(x, y)?;
            }
        }
        for y in (ey0..y0).chain(y1 + 1..=ey1) {
            for x in x0..=x1 {
                self.cliff(x, y)?;
            }
        }
        Ok(())
    }

    /// `FUN_140f8a890`: a cliff tile's objects, each to its own place.
    fn cliff(&mut self, x: i32, y: i32) -> Result<(), Unsupported> {
        let word = self.world.tiles().word(x, y).unwrap_or(0);
        if word & 0x3f00 == 0x600 {
            let world = self.world;
            for &id in world.tiles().list(x, y) {
                let Some(p) = world.get(id) else {
                    continue;
                };
                if !p.tall {
                    continue;
                }
                if p.class == 39 {
                    return Err(Unsupported(
                        "a gate in the old short pather's area (its players' passage, slot 0x678)",
                    ));
                }
                if self.ignore.contains(&id) || world.gait(id).is_some_and(|g| g.moving) {
                    continue;
                }
                self.add_box(id, p, None);
            }
        }
        Ok(())
    }

    /// `MShortPather::checkPointInsideHull` (`FUN_140f8be30`): the first hull holding `p`.
    fn hull_of(&self, p: Point) -> i32 {
        self.hulls
            .iter()
            .position(|h| !h.points.is_empty() && h.inside(p))
            .map_or(-1, |i| i as i32)
    }

    /// Whether `p` lies within the area's tiles, `r` from their edge.
    fn in_area(&self, p: Point) -> bool {
        let r = self.radius;
        let [x0, y0, x1, y1] = self.area;
        x0 as f32 + r <= p[0]
            && p[0] <= (x1 + 1) as f32 - r
            && y0 as f32 + r <= p[1]
            && p[1] <= (y1 + 1) as f32 - r
    }

    /// Whether `p` lies on the map, `r` from its edge.
    fn on_map(&self, p: Point) -> bool {
        let r = self.radius;
        let [w, h] = self.world.map;
        !(p[0] < r || w as f32 - r < p[0] || p[1] < r || h as f32 - r < p[1])
    }

    /// The gap between `p` and the target's box grown by the radius.
    fn target_gap(&self, t: &Target, p: Point) -> f32 {
        let r = self.radius;
        let gx = (p[0] - t.at[0]).abs() - (t.radius[0] + r);
        let gy = (p[1] - t.at[1]).abs() - (t.radius[1] + r);
        let gx = if gx <= 0.0 { 0.0 } else { gx };
        let gy = if gy <= 0.0 { 0.0 } else { gy };
        sqrt(gy * gy + gx * gx)
    }

    // The quick path.

    /// `MShortPather::findQuickPath` (`FUN_140f8c5a0`): the straight walk to `to`, when it meets
    /// nothing or only the target; else around the one square obstruction it meets (with the
    /// target, the other), by a point `(half side + radius) √2` from its middle on either side of
    /// the walk, when both walks by that point meet nothing (the second the target alone).
    fn quick_path(
        &self,
        from: Point,
        to: Point,
        target: Option<i32>,
        path: &mut Waypoints,
    ) -> Result<bool, Unsupported> {
        let hits = self
            .world
            .pather_hits(from, to, self.buffer, self.mask, &self.ignore)?;
        let is_target = |id: i32| target.is_some_and(|t| t == id);
        let obstacle = match hits[..] {
            [] => {
                path.push(from);
                path.push(to);
                return Ok(true);
            }
            [h] if is_target(h.id) => {
                path.push(from);
                path.push(to);
                return Ok(true);
            }
            [h] => h,
            [a, b] if target.is_some() => {
                if is_target(a.id) {
                    b
                } else if is_target(b.id) {
                    a
                } else {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        };
        let [x0, y0, x1, y1] = obstacle.bounds;
        let half = (x1 - x0) * HALF;
        if half != (y1 - y0) * HALF {
            return Ok(false);
        }
        let (mut dx, mut dy) = (from[0] - to[0], from[1] - to[1]);
        let (cx, cy) = ((x1 + x0) * HALF, (y1 + y0) * HALF);
        let len = dy * dy + dx * dx;
        if 0.0 < len {
            let len = sqrt(len);
            dx *= ONE / len;
            dy *= ONE / len;
        }
        let off = (half + self.radius) * SQRT_2;
        let (px, py) = (-dy * off, off * dx);
        for k in 0..2 {
            let p = if k == 0 {
                [cx + px, cy + py]
            } else {
                [cx - px, cy - py]
            };
            let [bx0, by0, bx1, by1] = self.bounds;
            if !(bx0 <= p[0] && p[0] <= bx1 && by0 <= p[1] && p[1] <= by1) {
                continue;
            }
            let first = self
                .world
                .pather_hits(from, p, self.buffer, self.mask, &self.ignore)?;
            if !first.is_empty() {
                continue;
            }
            let second = self
                .world
                .pather_hits(p, to, self.buffer, self.mask, &self.ignore)?;
            let fine = match second[..] {
                [] => true,
                [h] => target.is_some_and(|t| t == h.id || self.world.annex_of(h.id) == Some(t)),
                _ => false,
            };
            if fine {
                path.push(from);
                path.push(p);
                path.push(to);
                return Ok(true);
            }
        }
        Ok(false)
    }

    // The walks within a hull.

    /// The points about box `b` a walk within a hull starts from: from the sides facing the region
    /// of `p` about the box (inside it, its nearest side and those within the radius), a point every
    /// `step` of the side's length, [`NUDGE`] off the box; with `reach`, only those within it of
    /// `p`; none inside a box of the hull. Returns whether one was made.
    #[allow(clippy::too_many_arguments)]
    fn side_points(
        &mut self,
        hull: usize,
        root: usize,
        b: [f32; 4],
        p: Point,
        step: f32,
        reach: Option<(Point, f32)>,
    ) -> bool {
        let [x0, y0, x1, y1] = b;
        let col = if p[0] < x0 {
            0
        } else {
            1 + i32::from(x1 <= p[0])
        };
        let row = if y0 <= p[1] {
            if p[1] < y1 { 3 } else { 6 }
        } else {
            0
        };
        let sides = match row + col {
            0 => [true, false, false, true],
            1 => [true, false, false, false],
            2 => [true, true, false, false],
            3 => [false, false, false, true],
            4 => {
                let (down, right, up) = (p[1] - y0, x1 - p[0], y1 - p[1]);
                let mut m = down;
                let mut side = usize::from(right < down);
                if right < down {
                    m = right;
                }
                let r = self.radius;
                let mut sides = [down < r, right < r, up < r, p[0] - x0 < r];
                if up < m {
                    side = 2;
                    m = up;
                }
                if p[0] - x0 < m {
                    side = 3;
                }
                sides[side] = true;
                sides
            }
            5 => [false, true, false, false],
            6 => [false, false, true, true],
            7 => [false, false, true, false],
            _ => [false, true, true, false],
        };
        let near = |q: Point| {
            reach.is_none_or(|(c, d)| {
                (c[0] - q[0]) * (c[0] - q[0]) + (c[1] - q[1]) * (c[1] - q[1]) <= d * d
            })
        };
        let mut made = false;
        let mut add = |me: &mut Self, q: Point| {
            if near(q) && me.hulls[hull].box_holding(q).is_none() {
                let _ = me.add_node(1, true, Some(root), q, None, -1, -1, -1, false);
                made = true;
            }
        };
        if sides[0] {
            let y = y0 - NUDGE;
            let mut x = x0 + NUDGE;
            let d = (x1 - x0) * step;
            while x < x1 {
                add(self, [x, y]);
                x += d;
            }
        }
        if sides[1] {
            let x = x1 + NUDGE;
            let mut y = y0 + NUDGE;
            while y < y1 {
                add(self, [x, y]);
                y += (y1 - y0) * step;
            }
        }
        if sides[2] {
            let y = y1 + NUDGE;
            let mut x = x1 - NUDGE;
            while x0 <= x {
                add(self, [x, y]);
                x -= (x1 - x0) * step;
            }
        }
        if sides[3] {
            let x = x0 - NUDGE;
            let mut y = y1 - NUDGE;
            while y0 <= y {
                add(self, [x, y]);
                y -= (y1 - y0) * step;
            }
        }
        made
    }

    /// The 16 first steps out of the box holding the start of a walk within a hull: each a
    /// diameter toward one of [`DIRECTIONS`], or to where it leaves the hull, unless inside a box.
    fn first_steps(&mut self, hull: usize, root: usize, p: Point, goal: Option<Point>) -> bool {
        let d = self.radius + self.radius;
        let mut made = false;
        for dir in DIRECTIONS {
            let mut q = [d * dir[0] + p[0], d * dir[1] + p[1]];
            if let Some((at, _)) = self.hulls[hull].intersection(false, p, q) {
                q = at;
            }
            if self.hulls[hull].box_holding(q).is_none() {
                let _ = self.add_node(1, true, Some(root), q, goal, -1, -1, -1, false);
                made = true;
            }
        }
        made
    }

    /// Whether a node of a walk within a hull may cast a ray toward `side`: not back along the
    /// ray that made it, nor into the side of the box it lies on.
    const fn may_cast(n: &Node, side: i32) -> bool {
        if n.direction == side {
            return false;
        }
        let back = matches!((n.direction, side), (0, 1) | (1, 0) | (2, 3) | (3, 2));
        let into = matches!((n.vertex, side), (0, 0) | (1, 2) | (2, 1) | (3, 3));
        !(back || into)
    }

    /// Keeps a walk's way out of a hull at `end` as one of `paths` (`count` kept, at most `most`):
    /// a path ending within [`SAME_END`] of one kept replaces it when it costs no more, else is
    /// dropped. Returns the path's slot.
    fn keep_path(
        &mut self,
        paths: &mut [Waypoints],
        count: &mut usize,
        end: usize,
    ) -> Result<Option<usize>, Unsupported> {
        let at = self.nodes[end].at;
        let cost = self.nodes[end].cost;
        let mut k = 0;
        while k < *count {
            let last = *paths[k].points.last().unwrap_or(&[f32::NAN, f32::NAN]);
            if (at[0] - last[0]).abs() < SAME_END && (at[1] - last[1]).abs() < SAME_END {
                if paths[k].cost < cost {
                    return Ok(None);
                }
                break;
            }
            k += 1;
        }
        if k == *count {
            *count += 1;
        } else {
            paths[k].points.clear();
        }
        self.build_path(&mut paths[k], end)?;
        paths[k].cost = cost;
        Ok(Some(k))
    }

    /// `MShortPather::pathWithinHull` out of a hull (`FUN_140f8cf00`): from `from` inside hull
    /// `hull`, a search of rays along the hull's boxes (list 1, [`HULL_ITERATIONS`]). Toward a
    /// `goal`, a node that sees the goal (or is within range of it or of the target) ends it in
    /// `paths[0]` (result 0), else the node nearest the goal does (1); without one, each ray that
    /// leaves the hull keeps its way out (`most` paths, result 4, or 5 when the start was inside a
    /// box). 2: nothing found, 3: no first step.
    #[allow(clippy::too_many_arguments)]
    fn walk_out(
        &mut self,
        hull: usize,
        from: Point,
        goal: Option<Point>,
        target: Option<Target>,
        range: f32,
        nudge: bool,
        paths: &mut [Waypoints],
        most: usize,
        count: &mut usize,
    ) -> Result<Within, Unsupported> {
        *count = 0;
        let mut left_boxes = false;
        self.empty(1);
        let mut nearest: Option<usize> = None;
        let mut best = goal.map_or(0.0, |g| {
            sqrt((from[1] - g[1]) * (from[1] - g[1]) + (from[0] - g[0]) * (from[0] - g[0]))
        });
        let root = self
            .add_node(1, true, None, from, goal, -1, -1, -1, false)
            .ok_or(Unsupported("the old short pather's first node"))?;
        let inside = self.hulls[hull].box_holding(from);
        if let Some(b) = inside {
            self.close(1, root);
            let mut made = self.first_steps(hull, root, from, goal);
            let bounds = self.hulls[hull].objects[b].bounds;
            let d = self.radius + self.radius;
            made |= self.side_points(hull, root, bounds, from, EXIT_STEP, Some((from, d)));
            if !made {
                return Ok(Within {
                    result: 3,
                    left_boxes,
                });
            }
        }
        let mut iterations = 0;
        let mut found: Option<usize> = None;
        let mut ended_at: Option<usize> = None;
        'search: loop {
            let node = loop {
                if HULL_ITERATIONS <= iterations || most <= *count {
                    break 'search;
                }
                let Some(n) = self.nodes[1].next else {
                    break 'search;
                };
                iterations += 1;
                self.close(1, n);
                if self.on_map(self.nodes[n].at) {
                    break n;
                }
            };
            let at = self.nodes[node].at;
            if !self.in_area(at) {
                break;
            }
            match (target.as_ref(), goal) {
                (None, None) => {}
                (None, Some(g)) => {
                    let d = (at[1] - g[1]) * (at[1] - g[1]) + (at[0] - g[0]) * (at[0] - g[0]);
                    if d <= (self.radius + range) * (self.radius + range) + NUDGE {
                        ended_at = Some(node);
                        break 'search;
                    }
                }
                (Some(t), _) => {
                    if self.target_gap(t, at) <= range + NUDGE {
                        found = Some(node);
                        break 'search;
                    }
                }
            }
            if let Some(g) = goal {
                if self.nodes[node].to_goal < best {
                    best = self.nodes[node].to_goal;
                    nearest = Some(node);
                }
                if !self.blocked(at, g)? {
                    let end = self.add_node(1, false, Some(node), g, Some(g), -1, -1, 1, false);
                    ended_at = end;
                    if end.is_some() {
                        break 'search;
                    }
                    // a node the list cannot take ends the walk at the nearest
                    break 'search;
                }
            }
            for side in 0..4 {
                let n = self.nodes[node];
                if !Self::may_cast(&n, side) {
                    continue;
                }
                if let Some((q, b, box_side)) = self.hulls[hull].ray_to_box(at, side, n.hull) {
                    let _ =
                        self.add_node(1, true, Some(node), q, goal, b as i32, box_side, side, true);
                    continue;
                }
                left_boxes = true;
                let Some((mut q, edge)) = self.hulls[hull].ray_to_hull(at, side) else {
                    continue;
                };
                if goal.is_some() {
                    let _ =
                        self.add_node(1, true, Some(node), q, goal, -1, edge as i32, side, true);
                    continue;
                }
                if *count < most {
                    if nudge {
                        let s = SIDES[side as usize];
                        q = [q[0] + s[0] * NUDGE, q[1] + s[1] * NUDGE];
                    }
                    if let Some(end) =
                        self.add_node(1, false, Some(node), q, None, -1, -1, -1, false)
                    {
                        let _ = self.keep_path(paths, count, end)?;
                    }
                }
            }
        }
        // the search's end
        // without a goal the walk's ways out are its answer, even past a target within range
        let Some(_) = goal else {
            let result = match (*count, inside) {
                (0, _) => 2,
                (_, Some(_)) => 5,
                (_, None) => 4,
            };
            return Ok(Within { result, left_boxes });
        };
        let end = found.or(ended_at);
        let Some(n) = end.or(nearest) else {
            return Ok(Within {
                result: 2,
                left_boxes,
            });
        };
        self.build_path(&mut paths[0], n)?;
        Ok(Within {
            result: u8::from(end.is_none()),
            left_boxes,
        })
    }

    /// `MShortPather::pathWithinHull` into a hull (`FUN_140f88ee0`): the walk out of hull `hull`
    /// from the goal `from` toward the start `to`, whose ways out (at most `most`) are the
    /// entrance paths, each with the hull side it leaves by. Result 4 (5 when the goal was inside
    /// a box) with paths, 2 without, 3 without a first step.
    #[allow(clippy::too_many_arguments)]
    fn walk_in(
        &mut self,
        hull: usize,
        from: Point,
        to: Point,
        target: Option<Target>,
        paths: &mut [Waypoints],
        sides_out: &mut [usize; PATHS],
        most: usize,
        count: &mut usize,
    ) -> Result<Within, Unsupported> {
        *count = 0;
        let mut left_boxes = false;
        self.empty(1);
        let root = self
            .add_node(1, true, None, from, Some(to), -1, -1, -1, false)
            .ok_or(Unsupported("the old short pather's first node"))?;
        let inside = self.hulls[hull].box_holding(from);
        if inside.is_some() || target.is_some() {
            self.close(1, root);
            let mut made = self.first_steps(hull, root, from, Some(to));
            let bounds = match target {
                None => self.hulls[hull].objects[inside.unwrap_or(0)].bounds,
                Some(t) => {
                    let r = self.radius;
                    [
                        (t.at[0] - t.radius[0]) - r,
                        (t.at[1] - t.radius[1]) - r,
                        t.at[0] + t.radius[0] + r,
                        t.at[1] + t.radius[1] + r,
                    ]
                }
            };
            made |= self.side_points(hull, root, bounds, to, ENTRANCE_STEP, None);
            if !made {
                return Ok(Within {
                    result: 3,
                    left_boxes,
                });
            }
        }
        let mut iterations = 0;
        'search: loop {
            let node = loop {
                if HULL_ITERATIONS <= iterations || most <= *count {
                    break 'search;
                }
                let Some(n) = self.nodes[1].next else {
                    break 'search;
                };
                iterations += 1;
                self.close(1, n);
                if self.on_map(self.nodes[n].at) {
                    break n;
                }
            };
            let at = self.nodes[node].at;
            if !self.in_area(at) {
                break;
            }
            for side in 0..4 {
                let n = self.nodes[node];
                if !Self::may_cast(&n, side) {
                    continue;
                }
                if let Some((q, b, box_side)) = self.hulls[hull].ray_to_box(at, side, n.hull) {
                    let _ = self.add_node(
                        1,
                        true,
                        Some(node),
                        q,
                        Some(to),
                        b as i32,
                        box_side,
                        side,
                        true,
                    );
                    continue;
                }
                left_boxes = true;
                let Some((mut q, edge)) = self.hulls[hull].ray_to_hull(at, side) else {
                    continue;
                };
                if most <= *count {
                    continue;
                }
                let s = SIDES[side as usize];
                q = [q[0] - s[0] * NUDGE, q[1] - s[1] * NUDGE];
                if let Some(end) =
                    self.add_node(1, false, Some(node), q, Some(to), -1, -1, -1, false)
                    && let Some(k) = self.keep_path(paths, count, end)?
                {
                    sides_out[k] = edge;
                }
            }
        }
        let result = if *count < 1 {
            2
        } else if inside.is_some() {
            5
        } else {
            4
        };
        Ok(Within { result, left_boxes })
    }

    /// `MShortPather::FindShortPath` (`FUN_140f85580`) as slot 0x3e8 calls it (the area new, the
    /// quick path first, no unit to include): a path from `from` to `to`, the place of object
    /// `target_id`, within `range` of its footprint. Returns the result (0 already there, 1 a
    /// path within range, 2 a path partway, 3 none, 4 a place off the map) and the path.
    ///
    /// # Errors
    ///
    /// Where the pather meets what the slice does not model.
    pub fn find(
        &mut self,
        from: Point,
        to: Point,
        target_id: i32,
        range: f32,
    ) -> Result<(u8, Waypoints), Unsupported> {
        self.find_with(from, to, target_id, range, false)
    }

    /// [`HullPather::find`] as slot 0x3e0 calls it (its last argument 1): the quick path alone,
    /// none (3) when it is blocked.
    ///
    /// # Errors
    ///
    /// Where the pather meets what the slice does not model.
    pub fn find_quick(
        &mut self,
        from: Point,
        to: Point,
        target_id: i32,
        range: f32,
    ) -> Result<(u8, Waypoints), Unsupported> {
        self.find_with(from, to, target_id, range, true)
    }

    #[allow(clippy::too_many_lines)]
    fn find_with(
        &mut self,
        from: Point,
        to: Point,
        target_id: i32,
        range: f32,
        quick_only: bool,
    ) -> Result<(u8, Waypoints), Unsupported> {
        let r = self.radius;
        let [w, h] = self.world.map;
        let mut path = Waypoints::default();
        self.area = [-1, -1, -1, -1];
        self.bounds = [r, r, w as f32 - r, h as f32 - r];
        let [bx0, by0, bx1, by1] = self.bounds;
        if from[0] < r || bx1 < from[0] || from[1] < r || by1 < from[1] {
            return Ok((4, path));
        }
        if to[0] < r || bx1 < to[0] || to[1] < r || by1 < to[1] {
            return Err(Unsupported(
                "the old short pather to a goal at the map's edge (FUN_140f89d80)",
            ));
        }
        let _ = (bx0, by0);
        self.start = from;
        self.goal = to;
        // the target counts only where it stands on the goal
        let mut target_id = target_id;
        let target = if target_id < 0 {
            None
        } else {
            match self.world.get_known(target_id)? {
                Some(p) if p.position[0] == to[0] && p.position[1] == to[1] => Some(Target {
                    id: target_id,
                    at: [p.position[0], p.position[1]],
                    radius: p.radius,
                    speed: p.speed,
                }),
                Some(_) => {
                    target_id = -1;
                    None
                }
                None => None,
            }
        };
        let straight =
            sqrt((from[1] - to[1]) * (from[1] - to[1]) + (from[0] - to[0]) * (from[0] - to[0]));
        let gap = target
            .as_ref()
            .map_or(straight - r, |t| self.target_gap(t, from));
        if (from[0] == to[0] && from[1] == to[1]) || gap <= range {
            return Ok((0, path));
        }
        self.buffer = (0.0 < r).then_some(r * HALF);
        // the quick path, to the goal moved by the range or short of the target
        let quick = if range == 0.0
            || target
                .as_ref()
                .is_some_and(|t| t.at[0] != to[0] && t.at[1] != to[1])
        {
            let mut q = to;
            if let Some(t) = target.as_ref()
                && t.at[0] == to[0]
                && t.at[1] == to[1]
            {
                let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
                let d = dy * dy + dx * dx;
                if t.speed != 0.0 || d <= NEAR_TARGET {
                    let len = sqrt(d);
                    q = [
                        q[0] - (dx / len) * (r + t.radius[0]),
                        q[1] - (dy / len) * (r + t.radius[1]),
                    ];
                }
            }
            q
        } else {
            let v = [to[0] - from[0], to[1] - from[1], 0.0];
            let len = sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
            let (ux, uy) = (v[0] / len, v[1] / len);
            [ux * range + to[0], uy * range + to[1]]
        };
        let quick_target = (target_id != -1).then_some(target_id);
        if self.quick_path(from, quick, quick_target, &mut path)? {
            return Ok((1, path));
        }
        if quick_only {
            return Ok((3, path));
        }
        // the area: the walk's tiles and two more each way, on the map
        let (lo_x, hi_x) = (
            if to[0] <= from[0] { to[0] } else { from[0] },
            if from[0] <= to[0] { to[0] } else { from[0] },
        );
        let (lo_y, hi_y) = (
            if to[1] <= from[1] { to[1] } else { from[1] },
            if from[1] <= to[1] { to[1] } else { from[1] },
        );
        self.area = [
            (lo_x as i32 - 2).max(0),
            (lo_y as i32 - 2).max(0),
            if w <= hi_x as i32 + 2 {
                w - 1
            } else {
                hi_x as i32 + 2
            },
            if h <= hi_y as i32 + 2 {
                h - 1
            } else {
                hi_y as i32 + 2
            },
        ];
        let Some(target) = target else {
            return Err(Unsupported(
                "the old short pather without a target on its goal (the goal's point query)",
            ));
        };
        let [ax0, ay0, ax1, ay1] = self.area;
        self.world.check_known_records([
            (ax0 - 5) as f32,
            (ay0 - 5) as f32,
            (ax1 + 6) as f32,
            (ay1 + 6) as f32,
        ])?;
        self.empty(0);
        let mut nearest_d =
            sqrt((from[1] - to[1]) * (from[1] - to[1]) + (from[0] - to[0]) * (from[0] - to[0]));
        let mut found = false;
        let mut entrance: i32 = -1;
        let mut iterations = 0;
        let mut growths = 0;
        let mut within = false;
        let mut entered_box = false;
        let mut best_node: Option<usize> = None;
        let mut start_hull = -1;
        let mut goal_hull = -1;
        'outer: loop {
            if ITERATION_CAP <= iterations || GROWTHS <= growths {
                break;
            }
            self.close_all(0);
            if growths == 0 {
                self.build_hulls()?;
            }
            let root = self
                .add_node(0, true, None, from, Some(to), -1, -1, -1, false)
                .ok_or(Unsupported("the old short pather's first node"))?;
            start_hull = self.hull_of(from);
            goal_hull = self.hull_of(to);
            within = false;
            if 0 <= start_hull {
                let sh = start_hull as usize;
                if start_hull == goal_hull {
                    path.points.clear();
                    let mut count = 0;
                    let walk = self.walk_out(
                        sh,
                        from,
                        Some(to),
                        Some(target),
                        range,
                        false,
                        std::slice::from_mut(&mut path),
                        1,
                        &mut count,
                    )?;
                    if walk.result == 0 {
                        return Ok((if target.id < 1 { 2 } else { 1 }, path));
                    }
                    if walk.result == 3 || !walk.left_boxes {
                        return Ok((3, path));
                    }
                    within = walk.result == 1;
                }
                let mut exits = std::mem::take(&mut self.exits);
                for e in &mut exits {
                    e.points.clear();
                }
                let mut count = 0;
                let walk = self.walk_out(
                    sh, from, None, None, range, true, &mut exits, PATHS, &mut count,
                );
                self.exits = exits;
                // the walk's result is not read: its paths are
                let _ = walk?;
                if count < 1 {
                    return Ok((3, path));
                }
                self.close(0, root);
                for k in 0..count {
                    let points = self.exits[k].points.clone();
                    let n = points.len();
                    if n <= 1 {
                        continue;
                    }
                    let mut node = Some(root);
                    let mut j = 1;
                    let mut whole = true;
                    while j < n - 1 {
                        node = self.add_node(
                            0,
                            false,
                            node,
                            points[j],
                            Some(to),
                            start_hull,
                            -1,
                            -1,
                            false,
                        );
                        if node.is_none() {
                            whole = false;
                            break;
                        }
                        j += 1;
                    }
                    if whole {
                        let _ = self.add_node(
                            0,
                            true,
                            node,
                            points[j],
                            Some(to),
                            start_hull,
                            -1,
                            -1,
                            false,
                        );
                    }
                }
            }
            entered_box = false;
            if 0 <= goal_hull {
                let gh = goal_hull as usize;
                let mut entrances = std::mem::take(&mut self.entrances);
                for e in &mut entrances {
                    e.points.clear();
                }
                let n = self.hulls[gh].points.len();
                self.hulls[gh].entrances = vec![-1; n];
                let mut count = 0;
                let mut sides = [0usize; PATHS];
                let walk = self.walk_in(
                    gh,
                    to,
                    from,
                    Some(target),
                    &mut entrances,
                    &mut sides,
                    PATHS,
                    &mut count,
                );
                self.entrances = entrances;
                entered_box = walk?.result == 5;
                self.join_entrances(gh, count, &sides);
            }
            // the search
            loop {
                if ITERATION_CAP <= iterations {
                    continue 'outer;
                }
                iterations += 1;
                let Some(best) = self.nodes[0].next else {
                    break 'outer;
                };
                let mut n = Some(best);
                let mut off = false;
                while let Some(i) = n {
                    if !self.on_map(self.nodes[i].at) {
                        off = true;
                        break;
                    }
                    n = self.nodes[i].next;
                }
                if off {
                    self.close(0, best);
                    continue;
                }
                if !self.in_area(self.nodes[best].at) {
                    if !self.grow_area()? {
                        break 'outer;
                    }
                    growths += 1;
                    continue 'outer;
                }
                self.close(0, best);
                let at = self.nodes[best].at;
                if self.target_gap(&target, at) <= range + NUDGE {
                    found = target_id >= 0;
                    best_node = Some(best);
                    break 'outer;
                }
                if self.nodes[best].to_goal < nearest_d {
                    nearest_d = self.nodes[best].to_goal;
                    best_node = Some(best);
                }
                let vertex = self.nodes[best].vertex;
                let mut crossing: Option<(usize, Point, usize)> = None;
                let mut crossing_d = MOST;
                for (i, hull) in self.hulls.iter().enumerate() {
                    if let Some((p, edge)) = hull.intersection(vertex < 0, at, to) {
                        let d = (p[1] - at[1]) * (p[1] - at[1]) + (p[0] - at[0]) * (p[0] - at[0]);
                        if d < crossing_d {
                            crossing = Some((i, p, edge));
                            crossing_d = d;
                        }
                    }
                }
                let Some((hit, p, edge)) = crossing else {
                    // no hull between the node and the goal
                    if !self.blocked(at, to)? {
                        best_node =
                            self.add_node(0, false, Some(best), to, Some(to), -1, -1, -1, false);
                        found = true;
                        break 'outer;
                    }
                    if 0 <= goal_hull && goal_hull == self.nodes[best].hull && 0 <= vertex {
                        let entrances = &self.hulls[goal_hull as usize].entrances;
                        if let Some(&e) = entrances.get(vertex as usize)
                            && (vertex as usize) < self.hulls[goal_hull as usize].points.len()
                            && 0 <= e
                        {
                            found = true;
                            best_node = Some(best);
                            entrance = e;
                            break 'outer;
                        }
                    }
                    continue;
                };
                if start_hull == -1 && goal_hull == -1 && self.blocked(at, p)? {
                    continue;
                }
                if hit as i32 == self.nodes[best].hull && 0 <= vertex {
                    continue;
                }
                let node = self
                    .add_node(0, false, Some(best), p, Some(to), hit as i32, -1, -1, false)
                    .ok_or(Unsupported("the old short pather's crossing node"))?;
                if self.nodes[node].to_goal < nearest_d {
                    nearest_d = self.nodes[node].to_goal;
                    best_node = Some(node);
                }
                let np = self.hulls[hit].points.len();
                let mut back = if edge >= 1 { edge } else { np } as i32 - 1;
                let mut fwd = edge;
                let (mut a, mut b) = (Some(node), Some(node));
                for _ in 0..np / 2 {
                    if let Some(x) = a {
                        let q = self.hulls[hit].points[back as usize];
                        a = self.add_node(
                            0,
                            true,
                            Some(x),
                            q,
                            Some(to),
                            hit as i32,
                            back,
                            -1,
                            false,
                        );
                    }
                    if let Some(x) = b {
                        let q = self.hulls[hit].points[fwd];
                        b = self.add_node(
                            0,
                            true,
                            Some(x),
                            q,
                            Some(to),
                            hit as i32,
                            fwd as i32,
                            -1,
                            false,
                        );
                    }
                    back -= 1;
                    if back < 0 {
                        back = np as i32 - 1;
                    }
                    fwd = if fwd + 1 < np { fwd + 1 } else { 0 };
                }
                if np % 2 == 1 {
                    let (qa, qb) = (
                        self.hulls[hit].points[back as usize],
                        self.hulls[hit].points[fwd],
                    );
                    match (a, b) {
                        (None, None) => {}
                        (Some(x), y)
                            if y.is_none_or(|y| self.nodes[x].cost < self.nodes[y].cost) =>
                        {
                            let _ = self.add_node(
                                0,
                                true,
                                Some(x),
                                qa,
                                Some(to),
                                hit as i32,
                                back,
                                -1,
                                false,
                            );
                        }
                        (_, Some(y)) => {
                            let _ = self.add_node(
                                0,
                                true,
                                Some(y),
                                qb,
                                Some(to),
                                hit as i32,
                                fwd as i32,
                                -1,
                                false,
                            );
                        }
                        (Some(_), None) => unreachable!("taken by the arm above"),
                    }
                }
            }
        }
        let _ = (start_hull, goal_hull);
        // the end: the path to the node found, or to the one nearest the goal
        let Some(end) = best_node else {
            if !within {
                return Ok((3, path));
            }
            return Ok((self.partway(&mut path, from, to, range), path));
        };
        if !found && within {
            let last = *path.points.last().unwrap_or(&[f32::NAN, f32::NAN]);
            let e = self.nodes[end].at;
            let d_path =
                (last[1] - to[1]) * (last[1] - to[1]) + (last[0] - to[0]) * (last[0] - to[0]);
            let d_node = (e[1] - to[1]) * (e[1] - to[1]) + (e[0] - to[0]) * (e[0] - to[0]);
            // a node nearer the goal than the walk's end is the end (never with a NaN)
            if d_node.partial_cmp(&d_path) != Some(std::cmp::Ordering::Less) {
                return Ok((self.partway(&mut path, from, to, range), path));
            }
        }
        path.points.clear();
        self.build_path(&mut path, end)?;
        if found {
            if entrance < 0 {
                return Ok((1, path));
            }
            let e = self.entrances[entrance as usize].points.clone();
            for j in (0..e.len().saturating_sub(1)).rev() {
                path.push(e[j]);
            }
            if !entered_box {
                return Ok((1, path));
            }
            let k = path.points.len() - 1;
            let hits = self.world.pather_hits(
                path.points[k],
                path.points[k - 1],
                self.buffer,
                self.mask,
                &self.ignore,
            )?;
            if hits.len() < 2 && (hits.len() != 1 || hits[0].id == target_id) {
                return Ok((1, path));
            }
            let _ = path.points.remove(k);
        }
        Ok((self.partway(&mut path, from, to, range), path))
    }

    /// `FindShortPath`'s end for a path partway (2): none when it ends at the start (3); within
    /// range of the goal it is a path (1); one that ends no nearer the goal than the start is none.
    fn partway(&self, path: &mut Waypoints, from: Point, to: Point, range: f32) -> u8 {
        let mut result = 2;
        let last = *path.points.last().unwrap_or(&[f32::NAN, f32::NAN]);
        if (last[0] - self.start[0]).abs() < EPSILON && (last[1] - self.start[1]).abs() < EPSILON {
            path.points.clear();
            result = 3;
        }
        let d = (last[1] - to[1]) * (last[1] - to[1]) + (last[0] - to[0]) * (last[0] - to[0]);
        if d < (from[1] - to[1]) * (from[1] - to[1]) + (from[0] - to[0]) * (from[0] - to[0]) {
            if d < range * range {
                result = 1;
            }
        } else {
            path.points.clear();
            result = 3;
        }
        result
    }

    /// The area grows by two tiles each way on the map (its new tiles' obstructions, then the
    /// cliffs about it); false when it would hold more than [`MOST_AREA`] tiles.
    fn grow_area(&mut self) -> Result<bool, Unsupported> {
        let [w, h] = self.world.map;
        let x0 = (self.area[0] - 2).max(0);
        let y0 = (self.area[1] - 2).max(0);
        let x1 = (self.area[2] + 2).min(w - 1);
        let y1 = (self.area[3] + 2).min(h - 1);
        if MOST_AREA < (x1 - x0) * (y1 - y0) {
            return Ok(false);
        }
        let grown = [x0, y0, x1, y1];
        for x in x0..=x1 {
            for y in y0..=y1 {
                self.process_tile(x, y, grown, None)?;
            }
        }
        self.area = grown;
        self.process_cliffs()?;
        Ok(true)
    }

    /// The entrance paths' ends join the goal's hull (`FindShortPath`): each on the hull side it
    /// left by, by side and then by its distance from the side's first point, the later ones
    /// after the earlier; an end on the last side goes last. Each joined point keeps its path.
    fn join_entrances(&mut self, hull: usize, count: usize, sides: &[usize; PATHS]) {
        if count == 0 {
            return;
        }
        let mut order: Vec<usize> = (0..count).collect();
        let mut by: Vec<usize> = sides[..count].to_vec();
        let mut d: Vec<f32> = (0..count)
            .map(|k| {
                let last = *self.entrances[k].points.last().unwrap_or(&[0.0, 0.0]);
                let points = &self.hulls[hull].points;
                let i = if by[k] == 0 { points.len() } else { by[k] };
                let p = points[i - 1];
                (last[1] - p[1]) * (last[1] - p[1]) + (last[0] - p[0]) * (last[0] - p[0])
            })
            .collect();
        for a in 0..count {
            for b in a + 1..count {
                if by[b] < by[a] || (by[a] == by[b] && d[b] < d[a]) {
                    by.swap(a, b);
                    d.swap(a, b);
                    order.swap(a, b);
                }
            }
        }
        let mut joined = 0;
        for k in 0..count {
            let e = order[k];
            let last = *self.entrances[e].points.last().unwrap_or(&[0.0, 0.0]);
            let hull = &mut self.hulls[hull];
            let at = if joined + by[k] != 0 {
                joined + by[k]
            } else {
                hull.points.len()
            };
            hull.points.insert(at, last);
            hull.entrances.insert(at, e as i32);
            if by[k] != 0 {
                joined += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spatial::{Obstruction, Tiles};

    /// The unit square as `addShapeToHulls` makes a hull of it: high right, low right, low left,
    /// high left.
    fn square() -> Hull {
        Hull {
            points: vec![[1.0, 1.0], [1.0, 0.0], [0.0, 0.0], [0.0, 1.0]],
            entrances: vec![-1; 4],
            min: [0.0, 0.0],
            max: [1.0, 1.0],
            objects: Vec::new(),
        }
    }

    #[test]
    fn a_hull_holds_the_points_inside_it() {
        let h = square();
        assert!(h.inside([0.5, 0.5]));
        assert!(!h.inside([1.5, 0.5]));
        assert!(!h.inside([0.5, -0.25]));
    }

    #[test]
    fn a_walk_meets_the_nearest_side_it_crosses() {
        let h = square();
        // the side from point 2 (low left) to point 3 (high left)
        assert_eq!(
            h.intersection(false, [-1.0, 0.5], [2.0, 0.5]),
            Some(([0.0, 0.5], 3))
        );
        // the last side ends at point 0: it is side 0
        assert_eq!(
            h.intersection(false, [0.5, 2.0], [0.5, 0.5]),
            Some(([0.5, 1.0], 0))
        );
        assert_eq!(h.intersection(false, [2.0, 2.0], [3.0, 2.0]), None);
    }

    #[test]
    fn the_gift_wrap_turns_clockwise_from_the_highest_point() {
        let mut h = square();
        h.merge(&[[2.0, 0.5]]);
        assert_eq!(
            h.points,
            [[1.0, 1.0], [2.0, 0.5], [1.0, 0.0], [0.0, 0.0], [0.0, 1.0]]
        );
        assert_eq!((h.min, h.max), ([0.0, 0.0], [2.0, 1.0]));
        // a point inside changes nothing
        let mut inside = square();
        inside.merge(&[[0.5, 0.5]]);
        assert_eq!(inside.points, square().points);
    }

    #[test]
    fn a_box_overlapping_a_hull_reports_its_corners_outside() {
        let h = square();
        let (overlap, outside) = h.shape_overlap([0.5, 0.5], [1.5, 1.5]);
        assert_eq!((overlap, outside), (2, [false, true, true, true]));
        assert_eq!(h.shape_overlap([0.25, 0.25], [0.75, 0.75]).0, 3);
        assert_eq!(h.shape_overlap([2.0, 2.0], [3.0, 3.0]).0, 0);
    }

    #[test]
    fn a_ray_leaves_a_hull_by_the_side_it_meets() {
        let h = square();
        assert_eq!(h.ray_to_hull([0.5, 0.5], 0), Some(([0.5, 0.0], 2)));
        assert_eq!(h.ray_to_hull([0.5, 0.5], 3), Some(([1.0, 0.5], 1)));
    }

    fn placed(x: f32, y: f32, r: f32, kind: u8) -> Placed {
        Placed {
            position: [x, y, 0.0],
            radius: [r, r],
            class: if kind == 5 { 4 } else { 10 },
            player: 1,
            master: 0,
            obstruction_type: kind,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: Some(Obstruction { kind, class: 2 }),
        }
    }

    #[test]
    fn the_quick_path_goes_around_one_square_obstruction() {
        let mut w = Spatial::with_tiles([20, 20], Tiles::new(20, 20, vec![4; 400]));
        // a villager, a tree in its way and its target past the tree
        w.insert(9171, placed(2.0, 5.5, 0.2, 5));
        w.insert(48, placed(5.5, 5.5, 0.5, 2));
        w.insert(9149, placed(10.5, 5.5, 0.5, 2));
        let mut pather = HullPather::new(&w, 9171, 9149, 6, 0.2);
        let (result, path) = pather.find([2.0, 5.5], [10.5, 5.5], 9149, 0.1).unwrap();
        assert_eq!(result, 1);
        // by the tree's lower side: (half side + radius) √2 below its middle
        let [start, by, end] = path.points[..] else {
            panic!("{path:?}")
        };
        assert_eq!(start, [2.0, 5.5]);
        assert!((by[0] - 5.5).abs() < 1e-5 && (by[1] - (5.5 - 0.7 * SQRT_2)).abs() < 1e-5);
        // the goal moved on by the range
        assert!((end[0] - 10.6).abs() < 1e-5 && (end[1] - 5.5).abs() < 1e-6);
    }

    /// `FUN_140f8af60`'s merge scan tests the other hull's sides k = 0..n-1 (`cmp edi, esi; jl`
    /// at 0x140f8b2c4 with esi = n - 1): a box hull whose bottom side alone crosses the grown
    /// hull merges into it.
    #[test]
    fn a_hull_crossing_only_the_other_s_last_tested_side_merges() {
        let w = Spatial::with_tiles([20, 20], Tiles::new(20, 20, vec![4; 400]));
        let mut pather = HullPather::new(&w, -1, -1, 6, 0.2);
        // a thin strip across the box's corner at (0, 0), a vertex of neither inside the other
        let strip = Hull {
            points: vec![[-0.9, 2.1], [2.1, -0.9], [2.0, -1.0], [-1.0, 2.0]],
            entrances: vec![-1; 4],
            min: [-1.0, -1.0],
            max: [2.1, 2.1],
            objects: Vec::new(),
        };
        // a new hull's order: max-max, max-min, min-min, min-max; the side min-min to max-min
        // (k = 2 of 4) is the only one the strip crosses that the scan tests
        let square = Hull {
            points: vec![[10.0, 10.0], [10.0, 0.0], [0.0, 0.0], [0.0, 10.0]],
            entrances: vec![-1; 4],
            min: [0.0, 0.0],
            max: [10.0, 10.0],
            objects: Vec::new(),
        };
        pather.hulls = vec![strip, square];
        // a box at the strip's far end goes into the strip, which then meets the square
        let hull = pather.add_shape([-1.2, 1.9], [-0.8, 2.3]);
        assert_eq!((hull, pather.hulls.len()), (0, 1));
        assert!(pather.hulls[0].points.contains(&[10.0, 10.0]));
    }

    /// Slot 0x3e8 (`FUN_14114f790`): the old short pather from the unit's place with its radius
    /// and terrain mask, ignoring it and the target; a path found or none needed gives its
    /// length, a target the world does not hold none.
    #[test]
    fn a_unit_s_short_reach_is_its_old_short_path_s_length() {
        let mut w = Spatial::with_tiles([20, 20], Tiles::new(20, 20, vec![4; 400]));
        w.insert(9171, placed(2.0, 5.5, 0.2, 5));
        w.insert(48, placed(5.5, 5.5, 0.5, 2));
        w.insert(9149, placed(10.5, 5.5, 0.5, 2));
        let mut mover = crate::hunt::tests::walker();
        mover.stats.radius = [0.2, 0.2, 0.2];
        mover.stats.restriction_mask = 6;
        let me = crate::movement::Me {
            id: 9171,
            position: [2.0, 5.5, 0.0],
            action_state: 0,
            group: -1,
        };
        let length = mover.short_reach(&me, &w, 9149, 0.1).unwrap().unwrap();
        let mut pather = HullPather::new(&w, 9171, 9149, 6, 0.2);
        let (_, path) = pather.find([2.0, 5.5], [10.5, 5.5], 9149, 0.1).unwrap();
        assert_eq!(length, path.length());
        let by = 5.5 - 0.7 * SQRT_2;
        let expected = (3.5f32 * 3.5 + (5.5 - by) * (5.5 - by)).sqrt()
            + (5.1f32 * 5.1 + (5.5 - by) * (5.5 - by)).sqrt();
        assert!((length - expected).abs() < 1e-4, "{length} {expected}");
        assert_eq!(mover.short_reach(&me, &w, 777, 0.1).unwrap(), None);
    }

    /// An empty 20 × 20 map of terrain 4, and a pather on it with radius 0.2 and mask 6.
    fn open_world() -> Spatial {
        Spatial::with_tiles([20, 20], Tiles::new(20, 20, vec![4; 400]))
    }

    /// List `l`'s open nodes from its head, each linked back to the one before.
    fn open_nodes(p: &HullPather<'_>, l: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut before = l;
        let mut n = p.nodes[l].next;
        while let Some(i) = n {
            assert_eq!(p.nodes[i].prev, Some(before), "node {i}'s prev");
            out.push(i);
            before = i;
            n = p.nodes[i].next;
        }
        out
    }

    /// `MShortPather::addNode` (`FUN_140f8aa10`): the cost is the parent's plus the step, the
    /// distance to the goal 0 without one. An open node goes after the open nodes whose cost
    /// plus distance is smaller (`COMISS`/`JBE` at 0x140f8ae2d: before equal ones); with a hull
    /// point (both `+0x28` and `+0x2c` at least 0, compared as integers), after the node before
    /// the first larger one of the scan. An open node of the same hull point that costs more
    /// closes and the new one takes its place; one that costs no more, or a closed one (all of
    /// them), refuses the new node. Not open: straight into the closed list.
    #[test]
    fn a_node_joins_the_open_list_by_cost_and_distance_and_replaces_a_dearer_one() {
        let w = open_world();
        let mut p = HullPather::new(&w, -1, -1, 6, 0.2);
        let goal = Some([10.0, 0.0]);
        let root = p
            .add_node(0, true, None, [0.0, 0.0], goal, -1, -1, -1, false)
            .unwrap();
        assert_eq!((p.nodes[root].cost, p.nodes[root].to_goal), (0.0, 10.0));
        // cost 5, distance √65: 13.06
        let a = p
            .add_node(0, true, Some(root), [3.0, 4.0], goal, 0, 1, 7, false)
            .unwrap();
        assert_eq!((p.nodes[a].cost, p.nodes[a].parent), (5.0, Some(root)));
        assert_eq!(
            (p.nodes[a].hull, p.nodes[a].vertex, p.nodes[a].direction),
            (0, 1, 7)
        );
        // 6 + 4 = 10: the scan puts it after the node before `a`, past the root's equal 10
        let b = p
            .add_node(0, true, Some(root), [6.0, 0.0], goal, 0, 2, -1, false)
            .unwrap();
        assert_eq!(open_nodes(&p, 0), [root, b, a]);
        // no hull point: no scan, the walk stops at the first node of no smaller sum
        let c = p
            .add_node(0, true, Some(root), [6.0, 0.0], goal, -1, -1, -1, false)
            .unwrap();
        assert_eq!(open_nodes(&p, 0), [c, root, b, a]);
        // `a`'s hull point at no smaller cost: refused
        assert_eq!(
            p.add_node(0, true, Some(root), [0.0, 5.0], goal, 0, 1, -1, false),
            None
        );
        // cheaper (3): `a` closes, the new node goes in by its sum (3 + 7)
        let d = p
            .add_node(0, true, Some(root), [3.0, 0.0], goal, 0, 1, -1, false)
            .unwrap();
        assert_eq!(open_nodes(&p, 0), [d, c, root, b]);
        assert_eq!(p.closed[0], [a]);
        // a closed node of the hull point that costs no more refuses one (`b`, 6, a cost-7
        // node); a cheaper one is made
        p.close(0, b);
        assert_eq!(
            p.add_node(0, true, Some(root), [0.0, 7.0], goal, 0, 2, -1, false),
            None
        );
        let e = p
            .add_node(0, true, Some(root), [5.0, 0.0], goal, 0, 2, -1, false)
            .unwrap();
        assert_eq!(p.nodes[e].cost, 5.0);
        // a node not open goes to the closed list without a test
        let f = p
            .add_node(0, false, Some(e), [5.0, 0.0], None, 0, 2, -1, false)
            .unwrap();
        assert_eq!((p.nodes[f].cost, p.nodes[f].to_goal), (5.0, 0.0));
        assert_eq!(p.closed[0], [a, b, f]);
        assert!(!open_nodes(&p, 0).contains(&f));
    }

    /// `addNode` by place (its last argument set): the same place is within [`EPSILON`] on both
    /// axes (`COMISS`/`JA`, an unordered difference is no match), with or without a hull point;
    /// of the closed list, all but the last node count.
    #[test]
    fn a_node_by_place_matches_within_epsilon_and_skips_the_last_closed_node() {
        let w = open_world();
        let mut p = HullPather::new(&w, -1, -1, 6, 0.2);
        let root = p
            .add_node(1, true, None, [0.0, 0.0], None, -1, -1, -1, true)
            .unwrap();
        let a = p
            .add_node(1, true, Some(root), [2.0, 0.0], None, -1, -1, -1, true)
            .unwrap();
        // within EPSILON of `a`, dearer: refused; cheaper (from a nearer parent): replaces it
        let near = [2.0 + EPSILON * 0.5, 0.0];
        assert_eq!(
            p.add_node(1, true, Some(a), near, None, -1, -1, -1, true),
            None
        );
        let q = p
            .add_node(1, false, None, [1.5, 0.0], None, -1, -1, -1, false)
            .unwrap();
        let b = p
            .add_node(1, true, Some(q), near, None, -1, -1, -1, true)
            .unwrap();
        assert_eq!(p.closed[1], [q, a]);
        assert_eq!(open_nodes(&p, 1), [root, b]);
        // a place EPSILON away is another place
        assert!(
            p.add_node(
                1,
                true,
                Some(a),
                [2.0 + EPSILON * 2.0, 0.0],
                None,
                -1,
                -1,
                -1,
                true
            )
            .is_some()
        );
        // the closed list's last node does not count, the ones before it do
        p.empty(1);
        let r = p
            .add_node(1, true, None, [9.0, 9.0], None, -1, -1, -1, true)
            .unwrap();
        p.close(1, r);
        let s = p
            .add_node(1, false, None, [4.0, 4.0], None, -1, -1, -1, false)
            .unwrap();
        assert_eq!(p.closed[1], [r, s]);
        assert!(
            p.add_node(1, true, Some(s), [4.0, 4.0], None, -1, -1, -1, true)
                .is_some()
        );
        assert!(
            p.add_node(1, true, Some(s), [9.0, 9.0], None, -1, -1, -1, true)
                .is_none()
        );
    }

    /// `moveToClosedList` (`FUN_140f8ce00`) unlinks the node (its own links stay) and appends it
    /// to the closed list; `closeAllOpenNodes` (`FUN_140f8c130`) closes the open nodes in their
    /// order and makes every closed node forget its hull point (`+0x28`, `+0x2c` -1); `emptyLists`
    /// (`FUN_140f8c300`) drops the list's open and closed nodes.
    #[test]
    fn nodes_close_one_by_one_or_all_and_the_lists_empty() {
        let w = open_world();
        let mut p = HullPather::new(&w, -1, -1, 6, 0.2);
        let goal = Some([10.0, 0.0]);
        let n: Vec<usize> = (0..4)
            .map(|k| {
                p.add_node(0, true, None, [k as f32, 0.0], goal, 0, k, 2, false)
                    .unwrap()
            })
            .collect();
        // the nearest to the goal first
        assert_eq!(open_nodes(&p, 0), [n[3], n[2], n[1], n[0]]);
        p.close(0, n[2]);
        assert_eq!(open_nodes(&p, 0), [n[3], n[1], n[0]]);
        assert_eq!(
            (p.nodes[n[2]].prev, p.nodes[n[2]].next),
            (Some(n[3]), Some(n[1]))
        );
        // the last open node: no next to relink
        p.close(0, n[0]);
        assert_eq!(open_nodes(&p, 0), [n[3], n[1]]);
        assert_eq!(p.closed[0], [n[2], n[0]]);
        assert_eq!((p.nodes[n[2]].hull, p.nodes[n[2]].vertex), (0, 2));
        p.close_all(0);
        assert_eq!(open_nodes(&p, 0), []);
        assert_eq!(p.closed[0], [n[2], n[0], n[3], n[1]]);
        for &i in &n {
            assert_eq!((p.nodes[i].hull, p.nodes[i].vertex), (-1, -1));
            assert_eq!(p.nodes[i].direction, 2);
        }
        // list 1 is another list
        let other = p
            .add_node(1, true, None, [1.0, 1.0], goal, 3, 3, -1, false)
            .unwrap();
        p.empty(0);
        assert_eq!((open_nodes(&p, 0), p.closed[0].len()), (vec![], 0));
        assert_eq!(open_nodes(&p, 1), [other]);
        p.empty(1);
        assert_eq!(open_nodes(&p, 1), []);
    }

    /// `MShortPather::checkPointInsideHull` (`FUN_140f8be30`): the first hull with points that
    /// holds the point, moved [`NUDGE`] of the way toward its bounding box's middle (a point on
    /// a side counts as inside), or -1.
    #[test]
    fn a_point_lies_in_the_first_hull_holding_it() {
        let w = open_world();
        let mut p = HullPather::new(&w, -1, -1, 6, 0.2);
        let shifted = |dx: f32| Hull {
            points: square().points.iter().map(|q| [q[0] + dx, q[1]]).collect(),
            min: [dx, 0.0],
            max: [dx + 1.0, 1.0],
            ..square()
        };
        p.hulls = vec![Hull::default(), shifted(0.0), shifted(0.5)];
        assert_eq!(p.hull_of([0.75, 0.5]), 1);
        assert_eq!(p.hull_of([1.25, 0.5]), 2);
        // the left side of hull 1 and the right side of hull 2, nudged inward
        assert_eq!(p.hull_of([0.0, 0.5]), 1);
        assert_eq!(p.hull_of([1.5, 0.5]), 2);
        assert_eq!(p.hull_of([1.75, 0.5]), -1);
        assert_eq!(p.hull_of([f32::NAN, 0.5]), -1);
        p.hulls.clear();
        assert_eq!(p.hull_of([0.75, 0.5]), -1);
    }

    #[test]
    fn a_unit_within_range_needs_no_path() {
        let mut w = Spatial::with_tiles([20, 20], Tiles::new(20, 20, vec![4; 400]));
        w.insert(9171, placed(9.75, 5.5, 0.2, 5));
        w.insert(9149, placed(10.5, 5.5, 0.5, 2));
        let mut pather = HullPather::new(&w, 9171, 9149, 6, 0.2);
        let (result, path) = pather.find([9.75, 5.5], [10.5, 5.5], 9149, 0.1).unwrap();
        assert_eq!((result, path.length()), (0, 0.0));
    }
}
