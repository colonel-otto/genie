//! A convex hull ([`Hull`], `MConvexHull`): the point test, the intersection, a box's overlap,
//! the merge and the rays to a box or out of the hull.

use crate::math::fmath::at_most_or_unordered;
use crate::pathing::hull_path::{
    EPSILON, FAR, FULL_TURN, HALF, HALF_TURN, Hull, MOST, NO_ANGLE, NUDGE, ONE, Point, QUADRANT,
    T_HIGH, T_LOW, TURN, UNTURN,
};

impl Hull {
    /// `MConvexHull::InsideHull` (`FUN_140f87760`): whether `p`, moved a little toward the bounding
    /// box's middle, lies inside the hull (the crossings of a ray to the right).
    pub(super) fn inside(&self, p: Point) -> bool {
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
    pub(super) fn intersection(
        &self,
        at_start: bool,
        a: Point,
        b: Point,
    ) -> Option<(Point, usize)> {
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
    pub(super) fn shape_overlap(&self, min: Point, max: Point) -> (u8, [bool; 4]) {
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
    pub(super) fn merge(&mut self, more: &[Point]) {
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
    pub(super) fn box_holding(&self, p: Point) -> Option<usize> {
        self.objects.iter().position(|o| o.holds(p))
    }

    /// `FUN_140f8ea30`: the nearest box side a ray from `p` meets toward `side` (0 down, 1 up,
    /// 2 left, 3 right), with the box and the box's side (0 its top, 1 its right, 2 its bottom,
    /// 3 its left: the far side when `p` is inside the box). The box `own` counts only strictly
    /// beyond `p`. The span tests skip a box by `COMISS`/`JA` (0x140f8ec1c, 0x140f8eb6e): an
    /// unordered coordinate lies in every span.
    pub(super) fn ray_to_box(&self, p: Point, side: i32, own: i32) -> Option<(Point, usize, i32)> {
        let mut best = MOST;
        let mut found = None;
        let within =
            |lo: f32, v: f32, hi: f32| at_most_or_unordered(lo, v) && at_most_or_unordered(v, hi);
        for (k, o) in self.objects.iter().enumerate() {
            let [x0, y0, x1, y1] = o.bounds;
            let mine = k as i32 == own;
            let hit = match side {
                0 if within(x0, p[0], x1) => {
                    let past = if mine { y1 < p[1] } else { y1 <= p[1] };
                    if past {
                        Some((y1, 0, p[1] - y1))
                    } else if p[1] <= y0 {
                        None
                    } else {
                        Some((y0, 2, p[1] - y0))
                    }
                }
                1 if within(x0, p[0], x1) => {
                    let past = if mine { p[1] < y0 } else { p[1] <= y0 };
                    if past {
                        Some((y0, 2, y0 - p[1]))
                    } else if y1 <= p[1] {
                        None
                    } else {
                        Some((y1, 0, y1 - p[1]))
                    }
                }
                2 if within(y0, p[1], y1) => {
                    let past = if mine { x1 < p[0] } else { x1 <= p[0] };
                    if past {
                        Some((x1, 1, p[0] - x1))
                    } else if p[0] <= x0 {
                        None
                    } else {
                        Some((x0, 3, p[0] - x0))
                    }
                }
                3 if within(y0, p[1], y1) => {
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
    pub(super) fn ray_to_hull(&self, p: Point, side: i32) -> Option<(Point, usize)> {
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
pub(super) fn pseudo_angle(dx: f32, dy: f32) -> f32 {
    let t = dy.abs() + dx.abs();
    let a = if t == 0.0 { 0.0 } else { dy / t };
    if 0.0 <= dx {
        if dy < 0.0 { a + FULL_TURN } else { a }
    } else {
        HALF_TURN - a
    }
}

/// An angle past a full turn turns back once.
pub(super) fn turn(angle: f32) -> f32 {
    if TURN < angle { angle + UNTURN } else { angle }
}
