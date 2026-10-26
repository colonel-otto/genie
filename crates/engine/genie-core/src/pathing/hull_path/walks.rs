//! The quick path (`findQuickPath`) and the walks out of a hull and into one
//! (`pathWithinHull`).

use crate::GenieError;
use crate::math::fmath::{at_most_or_unordered, maxss, sqrt};
use crate::pathing::hull_path::{
    DIRECTIONS, ENTRANCE_STEP, EXIT_STEP, HALF, HULL_ITERATIONS, HullPather, NUDGE, Node, ONE,
    PATHS, Point, SAME_END, SIDES, SQRT_2, Target, Waypoints, Within,
};

impl HullPather<'_> {
    /// The gap between `p` and the target's box grown by the radius; each axis held at 0 or
    /// more by `MAXSS` (0x140f86bcf, 0x140f85a18): an unordered one is 0.
    pub(super) fn target_gap(&self, t: &Target, p: Point) -> f32 {
        let r = self.radius;
        let gx = maxss((p[0] - t.at[0]).abs() - (t.radius[0] + r), 0.0);
        let gy = maxss((p[1] - t.at[1]).abs() - (t.radius[1] + r), 0.0);
        sqrt(gy * gy + gx * gx)
    }

    // The quick path.

    /// `MShortPather::findQuickPath` (`FUN_140f8c5a0`): the straight walk to `to`, when it meets
    /// nothing or only the target; else around the one square obstruction it meets (with the
    /// target, the other), by a point `(half side + radius) √2` from its middle on either side of
    /// the walk, when both walks by that point meet nothing (the second the target alone).
    pub(super) fn quick_path(
        &self,
        from: Point,
        to: Point,
        target: Option<i32>,
        path: &mut Waypoints,
    ) -> Result<bool, GenieError> {
        let hits = self.hits(from, to)?;
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
            let first = self.hits(from, p)?;
            if !first.is_empty() {
                continue;
            }
            let second = self.hits(p, to)?;
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
    pub(super) fn side_points(
        &mut self,
        hull: usize,
        root: usize,
        b: [f32; 4],
        p: Point,
        step: f32,
        reach: Option<(Point, f32)>,
    ) -> bool {
        let [x0, y0, x1, y1] = b;
        // `COMISS`/`JA`, then `SETBE`; `JBE`, then `CMOVA` (0x140f8d1fc..0x140f8d235,
        // 0x140f893a4..0x140f893d0): an unordered coordinate counts past the box's far side
        let col = if p[0] < x0 {
            0
        } else if p[0] < x1 {
            1
        } else {
            2
        };
        let row = if p[1] < y0 {
            0
        } else if p[1] < y1 {
            3
        } else {
            6
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
        // a point farther than the reach is left out (`COMISS`/`JA`, 0x140f8d620)
        let near = |q: Point| {
            reach.is_none_or(|(c, d)| {
                at_most_or_unordered(
                    (c[0] - q[0]) * (c[0] - q[0]) + (c[1] - q[1]) * (c[1] - q[1]),
                    d * d,
                )
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
    pub(super) fn first_steps(
        &mut self,
        hull: usize,
        root: usize,
        p: Point,
        goal: Option<Point>,
    ) -> bool {
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
    pub(super) const fn may_cast(n: &Node, side: i32) -> bool {
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
    pub(super) fn keep_path(
        &mut self,
        paths: &mut [Waypoints],
        count: &mut usize,
        end: usize,
    ) -> Result<Option<usize>, GenieError> {
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
    /// box). 2: nothing found, 3: no first step. From inside a box, the start's region about the
    /// box picks the sides whose points start the walk too (the jump table at 0x140f8d261,
    /// [`HullPather::side_points`]). `FindShortPath`, its only caller, gives it a hull, a start and
    /// paths (its result 6 is for none) and no array for the sides the ways out leave by (its
    /// argument 13).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn walk_out(
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
    ) -> Result<Within, GenieError> {
        *count = 0;
        let mut left_boxes = false;
        self.empty(1);
        let mut nearest: Option<usize> = None;
        let mut best = goal.map_or(0.0, |g| {
            sqrt((from[1] - g[1]) * (from[1] - g[1]) + (from[0] - g[0]) * (from[0] - g[0]))
        });
        let root = self.add_unrefused(1, true, None, from, goal, -1, -1);
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
    /// a box) with paths, 2 without, 3 without a first step. `FindShortPath`, its only caller,
    /// gives it a hull, a start, paths and the sides' array (its result 6 is for none) and its
    /// argument 10 set: each way out is nudged back into the hull.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn walk_in(
        &mut self,
        hull: usize,
        from: Point,
        to: Point,
        target: Option<Target>,
        paths: &mut [Waypoints],
        sides_out: &mut [usize; PATHS],
        most: usize,
        count: &mut usize,
    ) -> Result<Within, GenieError> {
        *count = 0;
        let mut left_boxes = false;
        self.empty(1);
        let root = self.add_unrefused(1, true, None, from, Some(to), -1, -1);
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
}
