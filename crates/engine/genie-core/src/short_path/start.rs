//! The start's node, the hits' points and the two legs of an L.

use crate::mesh::ONE;
use crate::short_path::{
    Hit, Kind, NONE, Node, Point, Search, TO_FIXED, TO_FLOAT, contact_point, in_box, shrunk_corner,
    snap_to_side, strictly_inside, through_float, tile_of,
};
use crate::{GenieError, fmath};

impl Search<'_> {
    /// `FUN_140fa4410`: the start's node (type 0, cost 0, the Euclidean estimate); 3 when the start
    /// already lies within range, 0 when it is free (the node opened), else the escape
    /// candidates (type 2) around it within twice the radius: 2 for fewer than two, else 1 with
    /// only the cheapest kept open.
    pub(super) fn start(&mut self, p: Point) -> i32 {
        const PAIRS: [(u32, u32); 4] = [(0, 1), (1, 2), (3, 2), (0, 3)];
        const DIRS: [Point; 4] = [[ONE, 0], [0, ONE], [ONE, 0], [0, ONE]];
        let g = &self.goal;
        if g.range2 >= g.rect_distance2(p) {
            return 3;
        }
        let fx = (g.at[0].wrapping_sub(p[0])) as f32 * TO_FLOAT;
        let fy = (g.at[1].wrapping_sub(p[1])) as f32 * TO_FLOAT;
        // CVTTSS2SI RAX at 0x140fa4558, EAX stored: a start 512 tiles or more away wraps
        let f = crate::checkpoint::cvttss2si_64(fmath::sqrt(fx * fx + fy * fy) * TO_FIXED) as i32;
        let start = Node {
            g: 0,
            f,
            idx: self.arena.len() as u32,
            parent: NONE,
            at: p,
            kind: Kind::Start,
        };
        self.arena.push(start);
        if self.ctx.holder(p).is_none() {
            self.push(start);
            return 0;
        }
        let d = self.ctx.d;
        let mesh = self.ctx.mesh;
        let w = mesh.width as i32;
        let total = mesh.width * mesh.width;
        let rr = ((i64::from(self.goal.r) << 23) >> 22) as i32;
        let rr2 = i64::from(((i64::from(rr) * i64::from(rr)) >> 22) as i32);
        let [px, py] = p;
        let tile = |x: i32, y: i32| (tile_of(y).wrapping_mul(w)).wrapping_add(tile_of(x)) as u32;
        let tiles = [
            tile(px - rr, py - rr),
            tile(px + rr, py - rr),
            tile(px + rr, py + rr),
            tile(px - rr, py + rr),
        ];
        let root = start.idx;
        let fdist = |a: Point| -> i32 {
            let dy = (py.wrapping_sub(a[1])) as f32 * TO_FLOAT;
            let dx = (px.wrapping_sub(a[0])) as f32 * TO_FLOAT;
            (fmath::sqrt(dx * dx + dy * dy) * TO_FIXED) as i32
        };
        let b = self.bounds;
        for ti in 0..4 {
            if tiles[..ti].contains(&tiles[ti]) || total <= tiles[ti] {
                continue;
            }
            let list = mesh.tiles[tiles[ti] as usize].clone();
            for &id in &list {
                if self.ctx.ignored(id) {
                    continue;
                }
                let r = mesh.shapes[id as usize];
                for k in 0..4u32 {
                    let a = shrunk_corner(r, PAIRS[k as usize].0, d);
                    let e = shrunk_corner(r, PAIRS[k as usize].1, d);
                    let dir = DIRS[k as usize];
                    let side = k as u8;
                    let horizontal = k == 0 || k == 2;
                    let (t, dist2, span) = if horizontal {
                        let u = i64::from(dir[0]);
                        let t = ((i64::from(px - a[0]) * u) >> 22) as i32;
                        let perp = i64::from((px - a[0]) - (((i64::from(t) * u) >> 22) as i32));
                        let dy = i64::from(py - a[1]);
                        (t, ((dy * dy) >> 22) + ((perp * perp) >> 22), e[0] - a[0])
                    } else {
                        let u = i64::from(dir[1]);
                        let t = ((i64::from(py - a[1]) * u) >> 22) as i32;
                        let perp = i64::from((py - a[1]) - (((i64::from(t) * u) >> 22) as i32));
                        let dx = i64::from(px - a[0]);
                        (t, ((perp * perp) >> 22) + ((dx * dx) >> 22), e[1] - a[1])
                    };
                    if dist2 > rr2 {
                        continue;
                    }
                    let h = (fmath::sqrt((rr2 - dist2) as f32 * TO_FLOAT) * TO_FIXED) as i32;
                    for c in [t - h, h + t] {
                        if 0 < c && c < span {
                            let q = [
                                ((i64::from(dir[0]) * i64::from(c)) >> 22) as i32 + a[0],
                                ((i64::from(dir[1]) * i64::from(c)) >> 22) as i32 + a[1],
                            ];
                            if in_box(q, b) && self.ctx.holder_but(q, id, d).is_none() {
                                self.side_node(fdist(q), root, id, side, q);
                            }
                        }
                    }
                    // the start's foot on the side
                    if horizontal {
                        if a[0] < px && px < e[0] {
                            let q = [px, a[1]];
                            if in_box(q, b) && self.ctx.holder(q).is_none() {
                                self.side_node((py - a[1]).wrapping_abs(), root, id, side, q);
                            }
                        }
                    } else if a[1] < py && py < e[1] {
                        let q = [a[0], py];
                        if in_box(q, b) && self.ctx.holder(q).is_none() {
                            self.side_node((a[0] - px).wrapping_abs(), root, id, side, q);
                        }
                    }
                    // the side's first corner
                    if in_box(a, b) && self.ctx.holder_but(a, id, d).is_none() {
                        let dx = i64::from(px - a[0]);
                        let dy = i64::from(py - a[1]);
                        let d2 = ((dx * dx) >> 22) + ((dy * dy) >> 22);
                        if d2 < rr2 {
                            let g = (fmath::sqrt(d2 as f32 * TO_FLOAT) * TO_FIXED) as i32;
                            self.side_node(g, root, id, side, a);
                        }
                    }
                    // the side's contacts
                    for ci in 0..mesh.sides[id as usize][k as usize].len() {
                        let c = mesh.sides[id as usize][k as usize][ci];
                        if (d >= c.half) || self.ctx.ignored(c.other) {
                            continue;
                        }
                        let q = contact_point(c.at, c.side, d, c.flag);
                        if !in_box(q, b) {
                            continue;
                        }
                        if c.side == k {
                            let mut nudged = q;
                            match k {
                                0 => nudged[1] += 1,
                                1 => nudged[0] += 1,
                                2 => nudged[1] -= 1,
                                _ => nudged[0] -= 1,
                            }
                            if !strictly_inside(mesh.shapes[c.other as usize], nudged, d) {
                                continue;
                            }
                        }
                        if self.ctx.holder(q).is_some() {
                            continue;
                        }
                        let dx = i64::from(px - q[0]);
                        let dy = i64::from(py - q[1]);
                        let d2 = ((dy * dy) >> 22) + ((dx * dx) >> 22);
                        if d2 < rr2 {
                            let g0 = (fmath::sqrt(d2 as f32 * TO_FLOAT) * TO_FIXED) as i32;
                            let g = if c.side == k { g0 + 0x32 } else { g0 };
                            self.side_node(g, root, id, side, q);
                        }
                    }
                }
            }
        }
        if self.open.len() < 2 {
            return 2;
        }
        let keep = self.open.len() - 1;
        let _ = self.open.drain(..keep);
        let n = &mut self.open[0];
        let f = self.goal.heuristic(n.at).wrapping_add(n.g);
        n.f = f;
        let idx = n.idx as usize;
        self.arena[idx].f = f;
        1
    }

    /// The point where the walk from `p` toward `t` meets the hit at fraction `frac` (`>> 22`).
    pub(super) fn at_fraction(p: Point, t: Point, frac: i32) -> Point {
        [
            ((i64::from(t[0].wrapping_sub(p[0])) * i64::from(frac)) >> 22) as i32 + p[0],
            ((i64::from(t[1].wrapping_sub(p[1])) * i64::from(frac)) >> 22) as i32 + p[1],
        ]
    }

    /// A hit point snapped onto the hit shape's side, then onto that side of every shape of its
    /// tile that still holds it, until none does (`LAB_140fa7102`, `LAB_140fa5f50`, unbounded as
    /// the game's). It ends: a holder holds the point strictly inside, so the snap keeps the other
    /// coordinate and moves this one past the holder's side, always the same way (side 0 and 3
    /// lower, 1 and 2 higher); no shape holds it again, and the mesh has finitely many.
    pub(super) fn settle(&self, h: &mut Point, hit: &mut Hit) {
        let d = self.ctx.d;
        snap_to_side(h, self.ctx.mesh.shapes[hit.shape as usize], hit.side, d);
        while let Some(id) = self.ctx.holder(*h) {
            snap_to_side(h, self.ctx.mesh.shapes[id as usize], hit.side, d);
            hit.shape = id;
        }
    }

    /// `FUN_140fa2b20`: the two legs of an L toward `c` from node `b`, the first along direction
    /// `a`, the second along `second`; each leg's cost is |Δx|, then |Δy|, of `delta` through float.
    pub(super) fn legs(
        &mut self,
        b: Node,
        c: Point,
        a: u8,
        second: u8,
        delta: Point,
    ) -> Result<(), GenieError> {
        let p = b.at;
        let m = if a == 1 || a == 3 {
            [c[0], p[1]]
        } else {
            [p[0], c[1]]
        };
        let sw = self.ctx.sweep(p, m, a);
        let (g, parent, sw) = if sw.found {
            (
                through_float(delta[0].wrapping_abs()).wrapping_add(b.g),
                b.idx,
                sw,
            )
        } else {
            let mid = self.add(
                through_float(delta[0].wrapping_abs()).wrapping_add(b.g),
                b.idx,
                m,
                Kind::Waypoint,
            );
            let sw2 = self.ctx.sweep(m, c, second);
            (
                through_float(delta[1].wrapping_abs()).wrapping_add(mid.g),
                mid.idx,
                sw2,
            )
        };
        let (Some(shape), Some(side), Some(at)) = (sw.shape, sw.side, sw.point) else {
            return Err(GenieError::unsupported(
                "an L toward a shape's centre that meets nothing (stack garbage)",
            ));
        };
        self.side_node(g, parent, shape, side, at);
        Ok(())
    }
}
