//! The search's run: its main loop and a popped node's expansion by its type.

use crate::pathing::short_path::{
    Callback, Corner, Ctx, Hit, Kind, NONE, NUDGES, Node, Point, Poly, RECT_HEURISTIC, Search,
    ShortPath, T_START, TO_FIXED, TO_FLOAT, UNSET, cheb, corner_emits, float_distance, in_box,
    isqrt, through_float,
};
use crate::world::slots::CheckedIndex;
use crate::{GenieError, math::fmath};

/// What a polygon's re-walk keeps fixed: the node it comes from, the polygon and its vertex count.
struct Rewalk {
    parent: u32,
    poly: u32,
    count: u32,
}

impl Search<'_> {
    /// Type 2, a node on a shape's side: a polygon already holding its point is walked again from
    /// that edge (both ways, while cheaper than the vertices' costs); else the outline of its
    /// cluster is traced into a new polygon and walked. Some(4) when the outline fails.
    pub(super) fn side_expand(
        &mut self,
        n: Node,
        shape: u32,
        side: u8,
    ) -> Result<Option<ShortPath>, GenieError> {
        let p = n.at;
        let d = self.ctx.d;
        for i in 0..self.polys.len() {
            let Some(e) = Self::edge_holding(
                self.polys.checked(i)?,
                p,
                u32::from(side),
                self.ctx.mesh,
                shape,
                d,
            ) else {
                continue;
            };
            if !RECT_HEURISTIC || !self.polys.checked(i)?.flag {
                return Ok(None);
            }
            self.rewalk(n, i as u32, e)?;
            return Ok(None);
        }
        self.polys.push(Poly::default());
        let mut poly = Poly::default();
        let mut rc = self.trace(shape, u32::from(side), Some(p), &mut poly)?;
        let last = self.polys.len() - 1;
        'older: for j in 0..last {
            for k in 0..poly.vertices.len() {
                let hit = Self::edge_holding(
                    self.polys.checked(j)?,
                    *poly.vertices.checked(k)?,
                    u32::from(*poly.sides.checked(k)?),
                    self.ctx.mesh,
                    *poly.shapes.checked(k)?,
                    d,
                );
                if hit.is_some() {
                    rc = 2;
                    continue 'older;
                }
            }
        }
        let empty = poly.vertices.is_empty();
        *self.polys.checked_mut(last)? = poly;
        if rc == 1 || rc == 2 || empty {
            return Ok(Some(ShortPath {
                result: 4,
                points: Vec::new(),
            }));
        }
        self.walk_new(last as u32, p, n.g, n.idx)?;
        Ok(None)
    }

    /// The re-walk of a polygon from edge `e` (0x140fa63d4..0x140fa6639): backward from `v[e]`,
    /// forward from `v[e + 1]`, the forward side on equal costs, each while its cost stays below
    /// the vertex's; vertices no corner node reached are passed over.
    pub(super) fn rewalk(&mut self, n: Node, pi: u32, e: u32) -> Result<(), GenieError> {
        let p = n.at;
        let poly = self.polys.checked(pi as usize)?;
        let m = poly.vertices.len() as u32;
        let next0 = (e + 1) % m;
        let mut bv = e;
        let mut fv = next0;
        let mut gb = cheb(*poly.vertices.checked(e as usize)?, p).wrapping_add(n.g);
        let mut gf = cheb(*poly.vertices.checked(next0 as usize)?, p).wrapping_add(n.g);
        let alive = |cost: i32, g: i32| g < cost || cost == UNSET;
        let mut alive_b = alive(*poly.costs.checked(e as usize)?, gb);
        let mut alive_f = alive(*poly.costs.checked(next0 as usize)?, gf);
        let walk = Rewalk {
            parent: n.idx,
            poly: pi,
            count: m,
        };
        loop {
            if !(alive_b || alive_f) {
                return Ok(());
            }
            let forward = (gf <= gb && alive_f) || !alive_b;
            let poly = self.polys.checked(pi as usize)?.clone();
            if forward {
                self.rewalk_corner(&walk, &poly, (fv, gf), (0, e, fv))?;
                (fv, gf, alive_f) = self.rewalk_advance(&walk, &poly, (fv, gf), true)?;
            } else {
                self.rewalk_corner(&walk, &poly, (bv, gb), (1, bv, next0))?;
                (bv, gb, alive_b) = self.rewalk_advance(&walk, &poly, (bv, gb), false)?;
            }
        }
    }

    /// The re-walk's corner at vertex `at` of `poly`, reached with cost `g` as a walk in
    /// direction `dir` over the run `lo`..`hi`: a vertex a corner node reached before and no goal
    /// candidate, where the sides differ and the corner emits a token, becomes a corner node.
    fn rewalk_corner(
        &mut self,
        walk: &Rewalk,
        poly: &Poly,
        (at, g): (u32, i32),
        (dir, lo, hi): (u8, u32, u32),
    ) -> Result<(), GenieError> {
        let m = walk.count;
        let v = *poly.vertices.checked(at as usize)?;
        let sp = *poly.sides.checked(((at + m - 1) % m) as usize)?;
        let sn = *poly.sides.checked(at as usize)?;
        let sh = *poly.shapes.checked(at as usize)?;
        if *poly.costs.checked(at as usize)? != UNSET
            && !self.goal_test(Corner {
                cost: g,
                parent: walk.parent,
                dir,
                low: lo,
                high: hi,
                poly: walk.poly,
                point: v,
            })?
            && sp != sn
            && sh != NONE
            && let Some(token) = corner_emits(sp, sn, v, self.goal.at)
        {
            let kind = Kind::Corner {
                dir,
                lo,
                hi,
                poly: walk.poly,
                token: u32::from(token),
                prev: sp,
                next: sn,
            };
            let node = self.add(g, walk.parent, v, kind);
            self.push(node);
            *self
                .polys
                .checked_mut(walk.poly as usize)?
                .costs
                .checked_mut(at as usize)? = g;
        }
        Ok(())
    }

    /// The re-walk on from vertex `from` (cost `g`) to the next vertex a corner node reached,
    /// forward or backward: its place, the cost there and whether it is still below the
    /// vertex's.
    fn rewalk_advance(
        &self,
        walk: &Rewalk,
        poly: &Poly,
        (from, g): (u32, i32),
        forward: bool,
    ) -> Result<(u32, i32, bool), GenieError> {
        let m = walk.count;
        let costs = &self.polys.checked(walk.poly as usize)?.costs;
        let (mut vi, mut g) = (from, g);
        let mut at = *poly.vertices.checked(from as usize)?;
        loop {
            vi = if forward {
                (vi + 1) % m
            } else {
                (m - 1 + vi) % m
            };
            let w = *poly.vertices.checked(vi as usize)?;
            g = g.wrapping_add(cheb(at, w));
            let cost = *costs.checked(vi as usize)?;
            at = w;
            if cost != UNSET {
                return Ok((vi, g, g < cost));
            }
        }
    }

    /// Type 3, a polygon corner: the raycast toward the goal's range point ignoring the corner's
    /// own touch (the token); a touch tries the four nudged points; then a crossing becomes a side
    /// node, a clear ray a terminal goal candidate, and a ray that still touches the axis L.
    pub(super) fn corner_expand(
        &mut self,
        n: Node,
        token: u32,
        prev: u8,
        next: u8,
    ) -> Result<(), GenieError> {
        let p = n.at;
        self.token = token;
        let mut t = range_point(self.goal.range, p, self.goal.at)?;
        let mut frac = T_START;
        let reach = n.g.wrapping_add(float_distance(t, p));
        if self.goal_node != NONE && (reach >= self.arena.checked(self.goal_node as usize)?.g) {
            return Ok(());
        }
        let mut hit = Hit {
            shape: NONE,
            side: 0,
        };
        let mut k =
            self.ctx
                .raycast(p, t, &mut frac, &mut hit, Callback::Corner, &mut self.token)?;
        if k == 2 {
            let b = self.bounds;
            for dir in NUDGES {
                let tj = [
                    ((i64::from(dir[0]) * i64::from(self.goal.rx)) >> 22) as i32 + t[0],
                    ((i64::from(dir[1]) * i64::from(self.goal.ry)) >> 22) as i32 + t[1],
                ];
                if !in_box(tj, b) {
                    continue;
                }
                let Some(tok) = corner_emits(prev, next, p, tj) else {
                    continue;
                };
                self.token = u32::from(tok);
                let mut f2 = T_START;
                k = self.ctx.raycast(
                    p,
                    tj,
                    &mut f2,
                    &mut hit,
                    Callback::Corner,
                    &mut self.token,
                )?;
                if k != 2 {
                    frac = f2;
                    t = tj;
                    break;
                }
            }
            if k == 2 {
                self.corner_axis(n, t)?;
                return Ok(());
            }
        }
        if k == 1 {
            let mut h = Self::at_fraction(p, t, frac);
            self.settle(&mut h, &mut hit)?;
            if h != p {
                let g = n.g.wrapping_add(float_distance(p, h));
                self.side_node(g, n.idx, hit.shape, hit.side, h);
            }
            return Ok(());
        }
        let g = n.g.wrapping_add(float_distance(p, t));
        let kind = Kind::Corner {
            dir: 0,
            lo: NONE,
            hi: NONE,
            poly: NONE,
            token: NONE,
            prev: 0xff,
            next: 0xff,
        };
        let node = self.add(g, n.idx, t, kind);
        if self.goal_node == NONE || node.g < self.arena.checked(self.goal_node as usize)?.g {
            self.goal_node = node.idx;
        }
        Ok(())
    }

    /// Type 3 when every ray touches: along x toward `t`, then along y; a leg's hit becomes a
    /// side node; with both legs clear, one tile toward `t` (at most) becomes a waypoint that may
    /// be the best (by cost, not estimate).
    pub(super) fn corner_axis(&mut self, n: Node, t: Point) -> Result<(), GenieError> {
        let p = n.at;
        let dx = t[0].wrapping_sub(p[0]);
        let dy = t[1].wrapping_sub(p[1]);
        let m = [t[0], p[1]];
        let sw = self.ctx.sweep(p, m, if 0 < dx { 1 } else { 3 })?;
        if let Some((shape, side, at)) = sw.counted()? {
            let g = n.g.wrapping_add(float_distance(p, at));
            self.side_node(g, n.idx, shape, side, at);
            return Ok(());
        }
        let sw2 = self.ctx.sweep(m, t, if 0 < dy { 2 } else { 0 })?;
        let Some((shape, side, at)) = sw2.counted()? else {
            let step = if dx.wrapping_abs() < 0x40_0000 {
                dx
            } else {
                (1.0f32.copysign(dx as f32 * TO_FLOAT) * TO_FIXED) as i32
            };
            let w = self.add(
                n.g.wrapping_add(dx.wrapping_abs()),
                n.idx,
                [step.wrapping_add(p[0]), p[1]],
                Kind::Waypoint,
            );
            if self.best == NONE || w.g < self.arena.checked(self.best as usize)?.g {
                self.best = w.idx;
            }
            return Ok(());
        };
        let mid = self.add(
            through_float(dx.wrapping_abs()).wrapping_add(n.g),
            n.idx,
            m,
            Kind::Waypoint,
        );
        let g = through_float((at[1].wrapping_sub(p[1])).wrapping_abs()).wrapping_add(mid.g);
        self.side_node(g, mid.idx, shape, side, at);
        Ok(())
    }
}

impl Search<'_> {
    /// The search itself, after the prologue: the start, the main loop, the path.
    pub(super) fn run(&mut self, start: Point) -> Result<ShortPath, GenieError> {
        let res4410 = self.start(start)?;
        match res4410 {
            3 => {
                return Ok(ShortPath {
                    result: 0,
                    points: Vec::new(),
                });
            }
            2 => {
                return Ok(ShortPath {
                    result: 3,
                    points: Vec::new(),
                });
            }
            _ => {}
        }
        self.goal_node = NONE;
        self.best = 0;
        while let Some(n) = self.open.pop() {
            if let Some(direct) = self.expand(n, start)? {
                return Ok(direct);
            }
        }
        let mut result = 1;
        if self.goal_node == NONE {
            result = 2;
            self.goal_node = self.best;
        }
        // the path from the goal back, without the root
        let mut pts: Vec<Point> = Vec::new();
        let mut node = *self.arena.checked(self.goal_node as usize)?;
        let mut is_root = node.parent == NONE;
        while !is_root {
            let parent = *self.arena.checked(node.parent as usize)?;
            match node.kind {
                Kind::Corner {
                    poly, dir, lo, hi, ..
                } if poly != NONE => {
                    pts.push(node.at);
                    let v = &self.polys.checked(poly as usize)?.vertices;
                    let n = v.len() as u32;
                    if dir == 1 {
                        let mut k = lo;
                        while k != hi {
                            pts.push(*v.checked(k as usize)?);
                            k = (k + 1) % n;
                        }
                    } else {
                        let mut k = hi;
                        while k != lo {
                            k = (k + n - 1) % n;
                            pts.push(*v.checked(k as usize)?);
                        }
                    }
                }
                _ => pts.push(node.at),
            }
            is_root = parent.parent == NONE;
            node = parent;
        }
        if res4410 == 0 {
            pts.push(start);
        }
        // the smoothing, with half the radius
        let r2 = ((i64::from(self.goal.r) * 0x40_0000
            + ((i64::from(self.goal.r) >> 63) & 0x7f_ffff))
            >> 23) as i32;
        let sight = Ctx {
            mesh: self.ctx.mesh,
            ignore: self.ctx.ignore,
            d: 0x20_0000 - r2,
        };
        let mut i = 0;
        while i < pts.len() {
            let mut j = i + 2;
            while j < pts.len() {
                if !sight.sight_blocked(*pts.checked(j)?, *pts.checked(i)?)? {
                    let _ = pts.drain(i + 1..j);
                    j = i + 1;
                }
                j += 1;
            }
            i += 1;
        }
        if res4410 != 0 {
            pts.push(start);
        }
        pts.reverse();
        Ok(ShortPath {
            result,
            points: pts,
        })
    }

    /// One popped node of the main loop; Some when the search ends at once (the direct path).
    pub(super) fn expand(
        &mut self,
        n: Node,
        start: Point,
    ) -> Result<Option<ShortPath>, GenieError> {
        let p = n.at;
        let d = self.ctx.d;
        match n.kind {
            Kind::Waypoint => {
                let _ = self.pop_test(&n)?;
            }
            Kind::Start => {
                if self.pop_test(&n)? {
                    return Ok(None);
                }
                let t = range_point(self.goal.range, p, self.goal.at)?;
                let mut frac = T_START;
                let mut hit = Hit {
                    shape: NONE,
                    side: 0,
                };
                let mut token = self.token;
                let k = self
                    .ctx
                    .raycast(p, t, &mut frac, &mut hit, Callback::Box, &mut token)?;
                self.token = token;
                match k {
                    0 => {
                        return Ok(Some(ShortPath {
                            result: 1,
                            points: vec![start, p, t],
                        }));
                    }
                    2 => self.touch_from(n, p, t, frac, hit)?,
                    _ => {
                        let mut h = Self::at_fraction(p, t, frac);
                        self.settle(&mut h, &mut hit)?;
                        let g = n.g.wrapping_add(float_distance(p, h));
                        self.side_node(g, n.idx, hit.shape, hit.side, h);
                    }
                }
            }
            Kind::Side { shape, side } => {
                if self.pop_test(&n)? {
                    return Ok(None);
                }
                let _ = self.side_expand(n, shape, side)?;
            }
            Kind::Corner {
                token, prev, next, ..
            } => {
                if self.pop_test(&n)? {
                    return Ok(None);
                }
                self.corner_expand(n, token, prev, next)?;
            }
        }
        let _ = d;
        Ok(None)
    }

    /// Type 0, `k == 2`: back off to a free point toward `t`, then the two L-shaped legs toward
    /// the touched shape's centre.
    pub(super) fn touch_from(
        &mut self,
        n: Node,
        p: Point,
        t: Point,
        hit_t: i32,
        hit: Hit,
    ) -> Result<(), GenieError> {
        let mut frac = hit_t - 0x6_6666;
        let mut base = n;
        while 1 <= frac {
            let qp = Self::at_fraction(p, t, frac);
            if self.ctx.holder(qp)?.is_none() {
                let dx = ((i64::from(t[0].wrapping_sub(p[0])) * i64::from(frac)) >> 22) as i32;
                let dy = ((i64::from(t[1].wrapping_sub(p[1])) * i64::from(frac)) >> 22) as i32;
                let fy = dy as f32 * TO_FLOAT;
                let fx = dx as f32 * TO_FLOAT;
                let g = (fmath::sqrt(fx * fx + fy * fy) * TO_FIXED) as i32;
                base = self.add(n.g.wrapping_add(g), n.idx, qp, Kind::Waypoint);
                break;
            }
            frac -= 0x6_6666;
        }
        let r = *self.ctx.mesh.shapes.checked(hit.shape as usize)?;
        let centre = [
            r[0].wrapping_add(
                (((i64::from(r[2] - r[0]) * 0x40_0000)
                    + ((i64::from(r[2] - r[0]) >> 63) & 0x7f_ffff))
                    >> 23) as i32,
            ),
            r[1].wrapping_add(
                (((i64::from(r[3] - r[1]) * 0x40_0000)
                    + ((i64::from(r[3] - r[1]) >> 63) & 0x7f_ffff))
                    >> 23) as i32,
            ),
        ];
        let b = self.bounds;
        let c = [
            if b[0] <= centre[0] {
                if b[2] < centre[0] { b[2] } else { centre[0] }
            } else {
                b[0]
            },
            if b[1] <= centre[1] {
                if b[3] < centre[1] { b[3] } else { centre[1] }
            } else {
                b[1]
            },
        ];
        let delta = [c[0].wrapping_sub(p[0]), c[1].wrapping_sub(p[1])];
        let dir_x = if 0 < delta[0] { 1 } else { 3 };
        let dir_y = if 0 < delta[1] { 2 } else { 0 };
        self.legs(base, c, dir_x, dir_y, delta)?;
        self.legs(base, c, dir_y, dir_x, delta)
    }
}

/// `FUN_140fa31b0`: the point on the circle of `range` around `goal` toward `p` (the goal itself
/// for no range).
pub(super) fn range_point(range: i32, p: Point, goal: Point) -> Result<Point, GenieError> {
    let mut out = goal;
    if range != 0 {
        let dx = i64::from(p[0].wrapping_sub(goal[0]));
        let dy = i64::from(p[1].wrapping_sub(goal[1]));
        let len = isqrt(((dy * dy) as u64 & !0x3f_ffff).wrapping_add((dx * dx) as u64 & !0x3f_ffff))
            as i32;
        if len == 0 {
            return Err(GenieError::unsupported(
                "the range point of a point on the goal (a division by zero)",
            ));
        }
        let ux = ((dx << 22) / i64::from(len)) as i32;
        let uy = ((dy << 22) / i64::from(len)) as i32;
        out[0] = out[0].wrapping_add(((i64::from(ux) * i64::from(range)) >> 22) as i32);
        out[1] = out[1].wrapping_add(((i64::from(uy) * i64::from(range)) >> 22) as i32);
    }
    Ok(out)
}
