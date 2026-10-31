//! The search's nodes: the open list, the goal and foot tests, a polygon corner's test and
//! the clip to the box.

use crate::GenieError;
use crate::pathing::ShapeId;
use crate::pathing::short_path::{FixedPoint, Kind, NONE, Node, RECT_HEURISTIC, Search};
use crate::world::slots::CheckedIndex;

/// A polygon corner reached as a goal candidate (`FUN_140fa3860`'s arguments, [`Search::goal_test`]).
#[derive(Clone, Copy, Debug)]
pub(super) struct Corner {
    /// The cost to the corner.
    pub(super) cost: i32,
    /// The node it was reached from.
    pub(super) parent: u32,
    /// The walk's direction round the polygon (0 forward, 1 backward).
    pub(super) dir: u8,
    /// The first vertex of the walked run.
    pub(super) low: u32,
    /// The last vertex of the walked run.
    pub(super) high: u32,
    /// The polygon.
    pub(super) poly: u32,
    /// The corner's point.
    pub(super) point: FixedPoint,
}

/// A polygon's axis edge whose foot the goal may lie on (`FUN_140fa8210`'s arguments,
/// [`Search::foot_test`]).
#[derive(Clone, Copy, Debug)]
pub(super) struct Foot {
    /// The polygon.
    pub(super) poly: u32,
    /// The edge's start.
    pub(super) start: FixedPoint,
    /// The edge's end.
    pub(super) end: FixedPoint,
    /// The edge's x extent: horizontal when not 0.
    pub(super) x_extent: i32,
    /// The first vertex of the walked run.
    pub(super) low: u32,
    /// The last vertex of the walked run.
    pub(super) high: u32,
    /// The walk's direction round the polygon (0 forward, 1 backward).
    pub(super) dir: u8,
    /// The cost at the edge's start.
    pub(super) start_cost: i32,
    /// The node it was reached from.
    pub(super) parent: u32,
}

/// `FUN_140fa3bc0`: whether a polygon corner at `v` between sides `prev` and `next` emits a node
/// toward `g`, and the token its raycast ignores (0xff none).
pub(super) fn corner_emits(prev: u8, next: u8, v: FixedPoint, g: FixedPoint) -> Option<u8> {
    let ([vx, vy], [gx, gy]) = (v, g);
    match prev {
        0 if next == 3 => (gx <= vx && gy <= vy).then_some(0xff),
        0 => (!(gx <= vx && vy <= gy)).then_some(u8::from(gx <= vx)),
        1 if next == 0 => (vx <= gx && gy <= vy).then_some(0xff),
        1 => (!(gx <= vx && gy <= vy)).then_some(1 + u8::from(vx < gx)),
        2 if next == 1 => (vx <= gx && vy <= gy).then_some(0xff),
        2 => (!(vx <= gx && gy <= vy)).then_some(2 + u8::from(vx <= gx)),
        3 if next == 2 => (gx <= vx && vy <= gy).then_some(0xff),
        3 => (!(vx <= gx && vy <= gy)).then_some(if gx < vx { 0 } else { 3 }),
        _ => None,
    }
}

/// `FUN_140fa16d0`: Cohen-Sutherland: the segment `a`–`b` clipped to the box; None when it lies
/// outside.
pub(super) fn clip(
    mut a: FixedPoint,
    mut b: FixedPoint,
    bx: [i32; 4],
) -> Option<(FixedPoint, FixedPoint)> {
    let (xmin, xmax) = if bx[2] < bx[0] {
        (bx[2], bx[0])
    } else {
        (bx[0], bx[2])
    };
    let (ymin, ymax) = if bx[3] < bx[1] {
        (bx[3], bx[1])
    } else {
        (bx[1], bx[3])
    };
    let code = |[x, y]: FixedPoint| {
        let mut c = if x < xmin {
            1
        } else if xmax < x {
            2
        } else {
            0
        };
        c |= if y < ymin {
            4
        } else if ymax < y {
            8
        } else {
            0
        };
        c
    };
    let part = |num: i32, mul: i32, den: i32| {
        let inner = ((i64::from(num) * i64::from(mul)) >> 22) as i32;
        ((i64::from(inner) << 22) / i64::from(den)) as i32
    };
    let (mut ca, mut cb) = (code(a), code(b));
    let (mut x, mut y) = (0, 0);
    while ca != 0 || cb != 0 {
        if ca & cb != 0 {
            return None;
        }
        let c = if ca != 0 { ca } else { cb };
        // the differences in 32 bits, the products and the division in 64
        let (dx, dy) = (b[0].wrapping_sub(a[0]), b[1].wrapping_sub(a[1]));
        if c & 1 != 0 {
            y = a[1].wrapping_add(part(dy, xmin.wrapping_sub(a[0]), dx));
            x = xmin;
        } else if c & 2 != 0 {
            y = a[1].wrapping_add(part(xmax.wrapping_sub(a[0]), dy, dx));
            x = xmax;
        } else if c & 4 != 0 {
            x = a[0].wrapping_add(part(ymin.wrapping_sub(a[1]), dx, dy));
            y = ymin;
        } else if c & 8 != 0 {
            x = a[0].wrapping_add(part(ymax.wrapping_sub(a[1]), dx, dy));
            y = ymax;
        }
        if c == ca {
            a = [x, y];
            ca = code(a);
        } else {
            b = [x, y];
            cb = code(b);
        }
    }
    Some((a, b))
}

pub(super) const fn in_box(p: FixedPoint, b: [i32; 4]) -> bool {
    b[0] <= p[0] && b[1] <= p[1] && p[0] <= b[2] && p[1] <= b[3]
}

/// `p` held in the box `b` on each axis (`[x0, y0, x1, y1]`).
pub(super) const fn clamp_to_box(p: FixedPoint, b: [i32; 4]) -> FixedPoint {
    [
        if b[0] <= p[0] {
            if b[2] < p[0] { b[2] } else { p[0] }
        } else {
            b[0]
        },
        if b[1] <= p[1] {
            if b[3] < p[1] { b[3] } else { p[1] }
        } else {
            b[1]
        },
    ]
}

/// Chebyshev distance (`max(|dx|, |dy|)`), the polygon walks' cost.
pub(super) const fn cheb(a: FixedPoint, b: FixedPoint) -> i32 {
    let dy = (a[1].wrapping_sub(b[1])).wrapping_abs();
    let dx = (a[0].wrapping_sub(b[0])).wrapping_abs();
    if dy < dx { dx } else { dy }
}

impl Search<'_> {
    /// Append a node to the arena (`FUN_140fa3fe0`, `FUN_140fa40d0`, `FUN_140fa4210`,
    /// `FUN_140fa4340`): `f = g + h(at)`.
    pub(super) fn add(&mut self, g: i32, parent: u32, at: FixedPoint, kind: Kind) -> Node {
        let n = Node {
            g,
            f: self.goal.heuristic(at).wrapping_add(g),
            idx: self.arena.len() as u32,
            parent,
            at,
            kind,
        };
        self.arena.push(n);
        n
    }

    /// `FUN_140fa2eb0`: into the open list, sorted by f descending, before the equal ones from the
    /// back (so equal f leave first in, first out).
    pub(super) fn push(&mut self, n: Node) {
        let at = self
            .open
            .iter()
            .rposition(|o| n.f < o.f)
            .map_or(0, |i| i + 1);
        self.open.insert(at, n);
    }

    pub(super) fn side_node(
        &mut self,
        g: i32,
        parent: u32,
        shape: ShapeId,
        side: u8,
        at: FixedPoint,
    ) {
        let n = self.add(g, parent, at, Kind::Side { shape, side });
        self.push(n);
    }

    /// `FUN_140fa3860`: a node at `pt` reached with cost `g`: within range and better than the goal
    /// so far, it becomes the goal (true); else, nearer than the best so far, the best.
    pub(super) fn goal_test(&mut self, corner: Corner) -> Result<bool, GenieError> {
        let Corner {
            cost: g,
            parent,
            dir,
            low: lo,
            high: hi,
            poly,
            point: pt,
        } = corner;
        let d2: i64 = if RECT_HEURISTIC {
            self.goal.rect_distance2(pt)
        } else {
            let dx = i64::from(self.goal.at[0].wrapping_sub(pt[0]));
            let dy = i64::from(self.goal.at[1].wrapping_sub(pt[1]));
            i64::from((((dy * dy) >> 22) as i32).wrapping_add(((dx * dx) >> 22) as i32))
        };
        let kind = Kind::Corner {
            dir,
            lo,
            hi,
            poly,
            token: 5,
            prev: 0,
            next: 0,
        };
        let better = self.goal_node == NONE || g < self.arena.checked(self.goal_node as usize)?.g;
        if self.goal.range2 < d2 || !better {
            let b = *self.arena.checked(self.best as usize)?;
            let h = i64::from(b.f.wrapping_sub(b.g));
            if d2 < (h * h) >> 22 {
                let n = self.add(g, parent, pt, kind);
                self.best = n.idx;
            }
            return Ok(false);
        }
        let n = self.add(g, parent, pt, kind);
        self.goal_node = n.idx;
        Ok(true)
    }

    /// `FUN_140fa8210`: the goal's foot on an axis edge `a`–`b` (horizontal when `dx != 0`) as a
    /// goal candidate.
    pub(super) fn foot_test(&mut self, edge: Foot) -> Result<(), GenieError> {
        let Foot {
            poly,
            start: a,
            end: b,
            x_extent: dx,
            low: lo,
            high: hi,
            dir,
            start_cost: g0,
            parent,
        } = edge;
        let (s, e, gv) = if dx != 0 {
            (a[0], b[0], self.goal.at[0])
        } else {
            (a[1], b[1], self.goal.at[1])
        };
        let low = if e < s { e } else { s };
        let high = if s < e { e } else { s };
        if low <= gv && gv <= high {
            let foot = if dx != 0 { [gv, a[1]] } else { [a[0], gv] };
            let g = g0.wrapping_add((gv.wrapping_sub(s)).wrapping_abs());
            let _ = self.goal_test(Corner {
                cost: g,
                parent,
                dir,
                low: lo,
                high: hi,
                poly,
                point: foot,
            })?;
        }
        Ok(())
    }

    /// `FUN_140fa2740`, a popped node's test: the best (lowest estimate) so far; in range and
    /// cheaper than the goal: the new goal (true); no cheaper than the goal: pruned (true).
    pub(super) fn pop_test(&mut self, n: &Node) -> Result<bool, GenieError> {
        let h = n.f.wrapping_sub(n.g);
        let b = *self.arena.checked(self.best as usize)?;
        if h < b.f.wrapping_sub(b.g) {
            self.best = n.idx;
        }
        let better = self.goal_node == NONE || n.g < self.arena.checked(self.goal_node as usize)?.g;
        if (h < self.goal.range || self.goal.rect_distance2(n.at) <= self.goal.range2) && better {
            self.goal_node = n.idx;
            return Ok(true);
        }
        Ok(!better)
    }
}
