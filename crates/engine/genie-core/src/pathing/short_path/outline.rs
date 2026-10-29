//! The polygons' outlines: the tracer, the border walk, the edge that holds a point and the
//! walk of a new polygon.

use crate::GenieError;
use crate::pathing::mesh::Mesh;
use crate::pathing::short_path::{
    Corner, Foot, Kind, NONE, Point, Poly, Search, TRACE_STEPS, UNSET, cheb, clamp_to_box,
    contact_point, corner_emits, in_box, inside, shrunk_corner,
};

/// A vertex the tracer adds to the outline being traced ([`Search::append`]'s arguments after
/// the polygon).
#[derive(Clone, Copy, Debug)]
pub(super) struct Traced {
    /// The vertex.
    pub(super) vertex: Point,
    /// The outline's first vertex.
    pub(super) first: Point,
    /// Whether the start side runs vertically.
    pub(super) vertical: bool,
    /// The start side.
    pub(super) start_side: u32,
    /// The side the vertex is on.
    pub(super) side: u8,
    /// The shape it lies on.
    pub(super) shape: u32,
    /// The turns so far (4 a full one).
    pub(super) winding: i32,
}

impl Search<'_> {
    /// `FUN_140fa2fa0`: append a vertex to the polygon being traced, unless the outline closes:
    /// after a full turn (`|winding| == 4`) the vertex passes the first one along the start side's
    /// line. On closing, a first vertex whose side repeats the last one's is dropped.
    pub(super) fn append(poly: &mut Poly, vertex: Traced) -> bool {
        let Traced {
            vertex: pos,
            first,
            vertical,
            start_side: s0,
            side,
            shape,
            winding,
        } = vertex;
        if let (Some(&last), 4) = (poly.vertices.last(), winding.wrapping_abs()) {
            let closes = if vertical {
                last[0] == first[0]
                    && pos[0] == first[0]
                    && if s0 == 1 {
                        last[1] <= first[1] && first[1] <= pos[1]
                    } else {
                        first[1] <= last[1] && pos[1] <= first[1]
                    }
            } else {
                last[1] == first[1]
                    && pos[1] == first[1]
                    && if s0 == 0 {
                        last[0] <= first[0] && first[0] <= pos[0]
                    } else {
                        first[0] <= last[0] && pos[0] <= first[0]
                    }
            };
            if closes {
                if poly.sides.last() == poly.sides.first() {
                    let _ = poly.vertices.remove(0);
                    let _ = poly.sides.remove(0);
                    let _ = poly.shapes.remove(0);
                }
                return true;
            }
        }
        poly.vertices.push(pos);
        poly.sides.push(side);
        poly.shapes.push(shape);
        false
    }

    /// The coordinate of `p` along a side's axis (x for sides 0 and 2, y for 1 and 3).
    pub(super) const fn along(p: Point, side: u32) -> i32 {
        if side & 1 == 1 { p[1] } else { p[0] }
    }

    /// `FUN_140fa9630`: the trace left the bounds: walk the bounds' border from `pos` in the turn
    /// the side gives, to the first shape side the sweep meets (0) or the border's corner (1).
    pub(super) fn border_walk(
        &self,
        pos: &mut Point,
        first: &mut Point,
        side: &mut u32,
        shape: &mut u32,
        u: &mut i32,
        steps: i32,
    ) -> i32 {
        let b = self.bounds;
        let w = side.wrapping_sub(1) & 3;
        let (target, dir) = match w {
            0 => ([b[0], b[3]], 2),
            1 => ([b[0], b[1]], 3),
            2 => ([b[2], b[1]], 0),
            _ => ([b[2], b[3]], 1),
        };
        let sw = self.ctx.sweep(*pos, target, dir);
        let u_new = Self::along(*pos, w);
        let Some((hid, hs, hp)) = sw.counted() else {
            if steps == 0 {
                *first = target;
            }
            *u = u_new;
            *shape = NONE;
            *side = w;
            *pos = target;
            return 1;
        };
        if steps == 0 {
            *first = hp;
        }
        *u = u_new;
        *shape = hid;
        *side = u32::from(hs);
        *pos = hp;
        0
    }

    /// `FUN_140fa8fa0`: trace the outline of the cluster of touching shapes from shape `shape0`'s
    /// side `s0` at `start` (counter-clockwise, the shapes on the left): along each side to its
    /// nearest contact ahead (ties to the first in the list) or its end; around the bounds' border
    /// where it leaves them. 0 when the outline closes, 1 after 2,000 steps.
    pub(super) fn trace(
        &self,
        shape0: u32,
        s0: u32,
        start: Option<Point>,
        poly: &mut Poly,
    ) -> Result<i32, GenieError> {
        let d = self.ctx.d;
        let mesh = self.ctx.mesh;
        let b = self.bounds;
        let vertical = s0 == 1 || s0 == 3;
        let mut steps: i32 = 0;
        let mut winding: i32 = 0;
        let mut wprev: i32 = 0;
        let mut shape = shape0;
        let mut side = s0;
        let mut pos = start.unwrap_or_else(|| shrunk_corner(mesh.shapes[shape as usize], s0, d));
        let mut u = Self::along(pos, s0);
        let mut first = pos;
        'segment: loop {
            let contacts = &mesh.sides[shape as usize][side as usize];
            let next_side;
            if let Some(k) = self.nearest_contact(contacts, side, u) {
                let c = contacts[k];
                let q = contact_point(c.at, c.side, d, c.flag);
                pos = q;
                if steps == 0 {
                    first = q;
                }
                if side != c.side {
                    winding -= 1;
                }
                u = Self::along(q, c.side);
                side = c.side;
                shape = c.other;
            } else {
                next_side = (side + 1) & 3;
                let corner = shrunk_corner(mesh.shapes[shape as usize], next_side, d);
                if steps == 0 {
                    first = corner;
                }
                // an empty side list measures the old point; it lies on the side's line too
                u = if contacts.is_empty() {
                    Self::along(pos, side.wrapping_sub(1) & 3)
                } else {
                    Self::along(corner, side.wrapping_sub(1) & 3)
                };
                pos = corner;
                winding += 1;
                side = next_side;
            }
            let mut wp = wprev;
            loop {
                steps += 1;
                if TRACE_STEPS <= steps {
                    return Ok(1);
                }
                let out = !in_box(pos, b);
                if out {
                    side = side.wrapping_sub(2) & 3;
                    let clamped = clamp_to_box(pos, b);
                    if pos == first {
                        first = clamped;
                    }
                    pos = clamped;
                }
                loop {
                    if Self::append(
                        poly,
                        Traced {
                            vertex: pos,
                            first,
                            vertical,
                            start_side: s0,
                            side: side as u8,
                            shape,
                            winding: wp,
                        },
                    ) {
                        poly.costs = vec![UNSET; poly.vertices.len()];
                        return Ok(0);
                    }
                    if !out {
                        wprev = winding;
                        continue 'segment;
                    }
                    let r = self
                        .border_walk(&mut pos, &mut first, &mut side, &mut shape, &mut u, steps);
                    if r == 0 {
                        break;
                    }
                    wp -= 1;
                    steps += 1;
                    if TRACE_STEPS <= steps {
                        return Ok(1);
                    }
                    winding = wp;
                }
                winding = wp - 2;
            }
        }
    }

    /// [`Search::trace`]'s next contact on a side: of the side's contacts (`contacts`), those
    /// wider than twice `d`, not ignored and whose contact point lies in the bounds, the one
    /// nearest ahead of `u` along `side` (each one's run start moved by `d`, back for a flagged
    /// contact on the same side; the sides 2 and 3 measure backwards); ties to the first.
    pub(super) fn nearest_contact(
        &self,
        contacts: &[crate::pathing::mesh::Contact],
        side: u32,
        u: i32,
    ) -> Option<usize> {
        let d = self.ctx.d;
        let mut chosen = None;
        let mut best_key = 0x7fc0_0000_i32;
        for (k, c) in contacts.iter().enumerate() {
            if (d >= c.half) || self.ctx.ignored(c.other) {
                continue;
            }
            let q = contact_point(c.at, c.side, d, c.flag);
            if !in_box(q, self.bounds) {
                continue;
            }
            let forward = side == 0 || side == 1 || (side != 2 && side != 3);
            let off = if side == c.side {
                if c.flag != 0 { d.wrapping_neg() } else { d }
            } else {
                d
            };
            let mut key = if forward {
                c.along.wrapping_add(off)
            } else {
                c.along.wrapping_sub(off)
            };
            key = key.wrapping_sub(u);
            if side.wrapping_sub(2) < 2 {
                key = key.wrapping_neg();
            }
            if -1 < key && key < best_key {
                best_key = key;
                chosen = Some(k);
            }
        }
        chosen
    }

    /// `FUN_140fa77b0`: the polygon's edge that holds `p` going along `side` (the start of the
    /// edge's run), or the vertex `p` is when it lies in the overlap of its shape with `shape`;
    /// None if none.
    pub(super) fn edge_holding(
        poly: &Poly,
        p: Point,
        side: u32,
        mesh: &Mesh,
        shape: u32,
        d: i32,
    ) -> Option<u32> {
        if shape == NONE {
            return None;
        }
        let n = poly.vertices.len() as u32;
        if n == 0 {
            return None;
        }
        let v = &poly.vertices;
        let sides = &poly.sides;
        let mut limit = n;
        let mut prev = sides[n.wrapping_sub(2) as usize % sides.len()];
        let mut cur = n - 1;
        let mut i: u32 = 0;
        loop {
            let s_cur = sides[cur as usize];
            let mut s_i = sides[i as usize];
            let mut nxt = i;
            if s_i == s_cur {
                loop {
                    i = (i + 1) % n;
                    s_i = sides[i as usize];
                    if s_i != s_cur {
                        break;
                    }
                }
                nxt = i;
                if i < cur && cur != n - 1 {
                    limit = 0;
                }
            }
            let vc = v[cur as usize];
            if Self::vertex_in_overlap(mesh, (p, vc, poly.shapes[cur as usize]), shape, d)
                || (side == u32::from(s_cur)
                    && Self::on_edge(s_cur, (vc, v[nxt as usize]), p, (prev, s_i)))
            {
                return Some(cur);
            }
            i = nxt + 1;
            prev = s_cur;
            cur = nxt;
            if i >= limit {
                return None;
            }
        }
    }

    /// `FUN_140fa77b0`'s vertex test: the vertex `vc` of shape `vs` is `p`, and `p` lies in the
    /// overlap of that shape with `shape` (the higher low corner, the lower high one), shrunk by
    /// `d` on every side.
    pub(super) fn vertex_in_overlap(
        mesh: &Mesh,
        (p, vc, vs): (Point, Point, u32),
        shape: u32,
        d: i32,
    ) -> bool {
        if p != vc || vs == NONE {
            return false;
        }
        let sr = mesh.shapes[shape as usize];
        let vr = mesh.shapes[vs as usize];
        let y1 = if sr[3] - d < vr[3] - d {
            sr[3] - d
        } else {
            vr[3] - d
        };
        let x1 = if sr[2] - d < vr[2] - d {
            sr[2] - d
        } else {
            vr[2] - d
        };
        let y0 = if vr[1] + d < sr[1] + d { sr[1] } else { vr[1] };
        let x0 = if vr[0] + d < sr[0] + d { sr[0] } else { vr[0] };
        let o = [x0 + d, y0 + d, x1, y1];
        o[0] < o[2] && o[1] < o[3] && inside(o, p, 0)
    }

    /// `FUN_140fa77b0`'s edge test: `p` lies on the edge of side `s_cur` from `vc` to `vn`; the
    /// side before the edge's run (`prev`) and the one after it (`next`) decide whether each end
    /// counts.
    pub(super) const fn on_edge(
        s_cur: u8,
        (vc, vn): (Point, Point),
        p: Point,
        (prev, next): (u8, u8),
    ) -> bool {
        match s_cur {
            0 if vc[1] == p[1] => {
                let (a, b) = (vc[0], vn[0]);
                let start = if prev < 2 { a <= p[0] } else { a < p[0] };
                start
                    && if next == 3 || next == 0 {
                        p[0] <= b
                    } else {
                        p[0] < b
                    }
            }
            1 if vc[0] == p[0] => {
                let (a, b) = (vc[1], vn[1]);
                let start = if prev.wrapping_sub(1) < 2 {
                    a <= p[1]
                } else {
                    a < p[1]
                };
                start && if next < 2 { p[1] <= b } else { p[1] < b }
            }
            2 if vc[1] == p[1] => {
                let (a, b) = (vc[0], vn[0]);
                let start = if next.wrapping_sub(1) < 2 {
                    b <= p[0]
                } else {
                    b < p[0]
                };
                start
                    && if 1 < prev.wrapping_sub(2) {
                        p[0] < a
                    } else {
                        p[0] <= a
                    }
            }
            3 if vc[0] == p[0] => {
                let (a, b) = (vc[1], vn[1]);
                let start = if next.wrapping_sub(2) < 2 {
                    b <= p[1]
                } else {
                    b < p[1]
                };
                start
                    && if prev == 0 || prev == 3 {
                        p[1] <= a
                    } else {
                        p[1] < a
                    }
            }
            _ => false,
        }
    }

    /// `FUN_140fa7b50`: from `p` on a new polygon's closing edge, walk the outline both ways at
    /// once, the cheaper side first (ties forward): each corner the goal test does not end
    /// becomes a corner node toward the goal when it emits; each edge gets its foot test.
    pub(super) fn walk_new(&mut self, pi: u32, p: Point, g0: i32, parent: u32) {
        let poly = self.polys[pi as usize].clone();
        let v = &poly.vertices;
        let n = v.len() as u32;
        let last = n - 1;
        let dxb = v[last as usize][0] - p[0];
        let dyb = v[last as usize][1] - p[1];
        let dxf = v[0][0] - p[0];
        let dyf = v[0][1] - p[1];
        self.foot_test(Foot {
            poly: pi,
            start: p,
            end: v[last as usize],
            x_extent: dxb,
            low: last,
            high: 0,
            dir: 1,
            start_cost: g0,
            parent,
        });
        self.foot_test(Foot {
            poly: pi,
            start: p,
            end: v[0],
            x_extent: dxf,
            low: 0,
            high: 0,
            dir: 0,
            start_cost: g0,
            parent,
        });
        let mut cb = (if dyb.wrapping_abs() < dxb.wrapping_abs() {
            dxb.wrapping_abs()
        } else {
            dyb.wrapping_abs()
        })
        .wrapping_add(g0);
        let mut cf = (if dyf.wrapping_abs() < dxf.wrapping_abs() {
            dxf.wrapping_abs()
        } else {
            dyf.wrapping_abs()
        })
        .wrapping_add(g0);
        let mut bi = last;
        let mut fi: u32 = 0;
        if bi == 0 {
            return;
        }
        loop {
            if cb < cf {
                let vv = v[bi as usize];
                let sp = poly.sides[(bi - 1) as usize];
                let sn = poly.sides[bi as usize];
                let sh = poly.shapes[bi as usize];
                if !self.goal_test(Corner {
                    cost: cb,
                    parent,
                    dir: 1,
                    low: bi,
                    high: 0,
                    poly: pi,
                    point: vv,
                }) && sp != sn
                    && sh != NONE
                    && let Some(token) = corner_emits(sp, sn, vv, self.goal.at)
                {
                    let kind = Kind::Corner {
                        dir: 1,
                        lo: bi,
                        hi: 0,
                        poly: pi,
                        token: u32::from(token),
                        prev: sp,
                        next: sn,
                    };
                    let node = self.add(cb, parent, vv, kind);
                    self.push(node);
                    let pm = &mut self.polys[pi as usize];
                    pm.flag = true;
                    pm.costs[bi as usize] = cb;
                }
                let w = v[(bi - 1) as usize];
                let dx = vv[0] - w[0];
                self.foot_test(Foot {
                    poly: pi,
                    start: vv,
                    end: w,
                    x_extent: dx,
                    low: bi - 1,
                    high: 0,
                    dir: 1,
                    start_cost: cb,
                    parent,
                });
                cb = cb.wrapping_add(cheb(vv, w));
                bi -= 1;
            } else {
                let vv = v[fi as usize];
                let sn = poly.sides[fi as usize];
                let sh = poly.shapes[fi as usize];
                let sp = poly.sides[((n - 1 + fi) % n) as usize];
                if !self.goal_test(Corner {
                    cost: cf,
                    parent,
                    dir: 0,
                    low: 0,
                    high: fi,
                    poly: pi,
                    point: vv,
                }) && sp != sn
                    && sh != NONE
                    && let Some(token) = corner_emits(sp, sn, vv, self.goal.at)
                {
                    let kind = Kind::Corner {
                        dir: 0,
                        lo: 0,
                        hi: fi,
                        poly: pi,
                        token: u32::from(token),
                        prev: sp,
                        next: sn,
                    };
                    let node = self.add(cf, parent, vv, kind);
                    self.push(node);
                    let pm = &mut self.polys[pi as usize];
                    pm.flag = true;
                    pm.costs[fi as usize] = cf;
                }
                fi += 1;
                let w = v[fi as usize];
                let dx = vv[0] - w[0];
                self.foot_test(Foot {
                    poly: pi,
                    start: vv,
                    end: w,
                    x_extent: dx,
                    low: 0,
                    high: fi,
                    dir: 0,
                    start_cost: cf,
                    parent,
                });
                cf = cf.wrapping_add(cheb(vv, w));
            }
            if fi >= bi {
                return;
            }
        }
    }
}
