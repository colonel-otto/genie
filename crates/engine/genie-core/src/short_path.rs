//! The short pather (`FUN_140f9e540` -> `FUN_140fa53d0`): a fixed-point any-angle search around the
//! shapes of the obstruction manager's [`Mesh`], for the path a unit walks next.
//!
//! All coordinates are fixed point, 2^22 units per tile ([`ONE`]). A unit of radius `r` tests every
//! shape shrunk by `d = 0x200000 - r` (shapes are stored grown by half a tile). The map of the
//! search, its node types and its ties: `docs/AoE2DE_s_exe_SHORT_PATHER.md`.

use crate::GenieError;
use crate::fmath;
use crate::mesh::{Mesh, ONE, Rect};

/// A point in fixed point.
pub type Point = [i32; 2];

/// `DAT_142f43b10`, `DAT_142f43a10`: fixed point to float and back.
const TO_FLOAT: f32 = crate::image::short_path::TO_FLOAT;
pub(crate) const TO_FIXED: f32 = crate::image::short_path::TO_FIXED;
/// The raycast's start `t` (`0x466666`, 1.1).
const T_START: i32 = 0x46_6666;

/// `(v + 0x3fffff) >> 22` for a negative `v`: the tile of a coordinate, truncated toward zero.
const fn tile_of(v: i32) -> i32 {
    if v < 0 {
        v.wrapping_add(0x3f_ffff) >> 22
    } else {
        v >> 22
    }
}

/// `FUN_14087c720`: the integer square root of a 64-bit value, bit by bit (its low 32 bits).
#[must_use]
pub const fn isqrt(mut v: u64) -> u32 {
    let mut root: u64 = 0;
    let mut bit: u64 = 1 << 62;
    if v <= 0x3fff_ffff_ffff_ffff {
        loop {
            bit >>= 2;
            if v >= bit {
                break;
            }
        }
    }
    while bit != 0 {
        if bit + root <= v {
            v -= bit + root;
            root += bit * 2;
        }
        root >>= 1;
        bit >>= 2;
    }
    root as u32
}

/// `(int)((float)v * 2^-22 * 2^22)`: a cost through float, 24 bits kept.
fn through_float(v: i32) -> i32 {
    ((v as f32) * TO_FLOAT * TO_FIXED) as i32
}

/// The Euclidean distance in fixed point through float (`sqrtf` of the float differences).
fn float_distance(a: Point, b: Point) -> i32 {
    let dy = (a[1].wrapping_sub(b[1])) as f32 * TO_FLOAT;
    let dx = (a[0].wrapping_sub(b[0])) as f32 * TO_FLOAT;
    (fmath::sqrt(dx * dx + dy * dy) * TO_FIXED) as i32
}

/// `FUN_14087c610`: `p` strictly inside `r` shrunk by `k`.
const fn strictly_inside(r: Rect, [x, y]: Point, k: i32) -> bool {
    r[0] + k < x && r[1] + k < y && x < r[2] - k && y < r[3] - k
}

/// `FUN_14087c650`: `p` inside `r` shrunk by `k`, edges included.
const fn inside(r: Rect, [x, y]: Point, k: i32) -> bool {
    r[0] + k <= x && r[1] + k <= y && x <= r[2] - k && y <= r[3] - k
}

/// `FUN_140fa8460`: corner `k` of `r` shrunk by `d`.
const fn shrunk_corner(r: Rect, k: u32, d: i32) -> Point {
    match k {
        0 => [r[0] + d, r[1] + d],
        1 => [r[2] - d, r[1] + d],
        2 => [r[2] - d, r[3] - d],
        3 => [r[0] + d, r[3] - d],
        _ => [r[0], r[1]],
    }
}

/// `FUN_140fa8350`: a contact's junction moved by `d` into the free space beside its side.
const fn contact_point([x, y]: Point, side: u32, d: i32, flag: u8) -> Point {
    let f = flag == 1;
    match side {
        1 => {
            if f {
                [x - d, y + d]
            } else {
                [x - d, y - d]
            }
        }
        2 => {
            if f {
                [x - d, y - d]
            } else {
                [x + d, y - d]
            }
        }
        3 => {
            if f {
                [x + d, y - d]
            } else {
                [x + d, y + d]
            }
        }
        _ => {
            if f {
                [x + d, y + d]
            } else {
                [x - d, y + d]
            }
        }
    }
}

/// `FUN_140fa39f0`: `p` moved onto side `side` of `r` shrunk by `d`, held within the side.
fn snap_to_side(p: &mut Point, r: Rect, side: u8, d: i32) {
    let hold = |v: i32, lo: i32, hi: i32| {
        if v < lo {
            lo
        } else if hi < v {
            hi
        } else {
            v
        }
    };
    match side {
        0 => {
            p[0] = hold(p[0], r[0] + d, r[2] - d);
            p[1] = r[1] + d;
        }
        1 => {
            p[0] = r[2] - d;
            p[1] = hold(p[1], r[1] + d, r[3] - d);
        }
        2 => {
            p[0] = hold(p[0], r[0] + d, r[2] - d);
            p[1] = r[3] - d;
        }
        3 => {
            p[0] = r[0] + d;
            p[1] = hold(p[1], r[1] + d, r[3] - d);
        }
        _ => {}
    }
}

/// `FUN_140faa380`: segment `a`–`b` against segment `c`–`e` in fixed point; `t` the parameter along
/// `a`–`b` (`ONE` is `b`). 1: a proper crossing nearer than `t`; 2: a touch or graze (tolerances
/// 1e-4 and 1e-3); 3: a touch at `a` itself on an end of `c`–`e`; 0: none. Sets `t` on a hit.
fn segment_hit(a: Point, b: Point, c: Point, e: Point, t: &mut i32) -> i32 {
    let w = |v: i32| i64::from(v);
    let (x1, y1, x2, y2) = (w(a[0]), w(a[1]), w(b[0]), w(b[1]));
    let (x3, y3, x4, y4) = (w(c[0]), w(c[1]), w(e[0]), w(e[1]));
    let den = (((y4 - y3) * (x2 - x1)) >> 22) - (((y2 - y1) * (x4 - x3)) >> 22);
    if den == 0 {
        return 0;
    }
    let ua =
        (((((y1 - y3) * (x4 - x3)) >> 22) - (((y4 - y3) * (x1 - x3)) >> 22)) * 0x40_0000) / den;
    let ub =
        (((((y1 - y3) * (x2 - x1)) >> 22) - (((y2 - y1) * (x1 - x3)) >> 22)) * 0x40_0000) / den;
    let current = i64::from(*t);
    let mut test_ub_low = true;
    if ua < 0x1a3 {
        if !(-0x1062..=0x40_1062).contains(&ua) || current <= ua {
            return 0;
        }
    } else if 0x3f_fe5d < ua {
        if 0x40_1062 < ua || current <= ua {
            return 0;
        }
    } else {
        if current <= ua {
            return 0;
        }
        if 0x1a2 < ub {
            if ub < 0x3f_fe5e {
                *t = ua as i32;
                return 1;
            }
            test_ub_low = false;
        }
    }
    if test_ub_low && ub < -0x1062 {
        return 0;
    }
    if 0x40_1062 < ub {
        return 0;
    }
    *t = ua as i32;
    if ua == 0 && (ub & !0x40_0000) == 0 {
        3
    } else {
        2
    }
}

/// `FUN_140faa4e0`: the ray `a`–`b` against the sides of the rectangle `r` it faces (the right side
/// for a ray going left, the left for one going right, then the bottom or the top); `side` the side
/// hit. Returns 0..3 as [`segment_hit`]; a y side wins only when strictly nearer.
fn ray_rect(a: Point, b: Point, r: Rect, t: &mut i32, side: &mut u8) -> i32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let [rx0, ry0, rx1, ry1] = r;
    let ratio = |num: i32, den: i32| ((i64::from(num) << 22) / i64::from(den)) as i32;
    let mut found = 0;
    if dx < 1 {
        if dx < 0 {
            let k;
            if dy == 0 {
                if a[1] <= ry0 || ry1 <= a[1] {
                    k = 0;
                } else {
                    if rx1 <= b[0] || a[0] < rx1 {
                        return 0;
                    }
                    let u = ratio(rx1 - a[0], dx);
                    if *t <= u {
                        return 0;
                    }
                    *t = u;
                    k = 1;
                }
            } else {
                k = segment_hit(a, b, [rx1, ry0], [rx1, ry1], t);
            }
            if k != 0 {
                *side = 1;
                found = k;
            }
        }
    } else {
        let k;
        if dy == 0 {
            if a[1] <= ry0 || ry1 <= a[1] {
                k = 0;
            } else {
                if b[0] <= rx0 || rx0 < a[0] {
                    return 0;
                }
                let u = ratio(rx0 - a[0], dx);
                if *t <= u {
                    return 0;
                }
                *t = u;
                k = 1;
            }
        } else {
            k = segment_hit(a, b, [rx0, ry0], [rx0, ry1], t);
        }
        if k != 0 {
            *side = 3;
            found = k;
        }
    }
    if 0 < dy {
        let k = if dx == 0 {
            if rx0 < a[0] && a[0] < rx1 {
                if b[1] <= ry0 || ry0 < a[1] {
                    return found;
                }
                let u = ratio(ry0 - a[1], dy);
                if *t <= u {
                    return found;
                }
                *t = u;
                *side = 0;
                return 1;
            }
            0
        } else {
            segment_hit(a, b, [rx0, ry0], [rx1, ry0], t)
        };
        if k == 0 {
            return found;
        }
        *side = 0;
        return k;
    }
    if 0 <= dy {
        return found;
    }
    let k = if dx == 0 {
        if rx0 < a[0] && a[0] < rx1 {
            if ry1 <= b[1] || a[1] < ry1 {
                return found;
            }
            let u = ratio(ry1 - a[1], dy);
            if *t <= u {
                return found;
            }
            *t = u;
            *side = 2;
            return 1;
        }
        0
    } else {
        segment_hit(a, b, [rx0, ry1], [rx1, ry1], t)
    };
    if k == 0 {
        return found;
    }
    *side = 2;
    k
}

/// A hit of the raycast or a sweep: the shape and its side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hit {
    /// The shape hit.
    pub shape: u32,
    /// Its side.
    pub side: u8,
}

/// An axis sweep's outputs as the game leaves them: whether a shape counted, the shape, and the
/// point and side of the last hit tested (written even for a shape the sweep then let pass).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Sweep {
    found: bool,
    shape: Option<u32>,
    point: Option<Point>,
    side: Option<u8>,
}

impl Sweep {
    /// The counted shape with the last hit's side and point: a sweep that counts a shape has
    /// written a point and a side by then (the hit that counts writes them first).
    fn counted(&self) -> Option<(u32, u8, Point)> {
        if !self.found {
            return None;
        }
        let (Some(shape), Some(side), Some(at)) = (self.shape, self.side, self.point) else {
            unreachable!("a counting hit writes its point and side before it counts");
        };
        Some((shape, side, at))
    }
}

/// The raycast's callbacks (`FUN_140fa1f60`, `FUN_140fa2180`, `FUN_140fa2850`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Callback {
    /// A: a touch at the corner the ray starts from on the token's side (or of the token's shape)
    /// is ignored.
    Corner,
    /// B: the shapes whose box meets the ray's box, a flat box grown by 1.
    Box,
    /// C: as B, without the growth (the smoothing's line of sight).
    Sight,
}

/// What the searches share for a unit: the mesh, the shapes it ignores, its shrinking.
#[derive(Debug)]
pub struct Ctx<'a> {
    /// The mesh.
    pub mesh: &'a Mesh,
    /// Per shape id: ignored (`pather + 0xd8`).
    pub ignore: &'a [bool],
    /// How far the searches shrink a shape's box (`FUN_140fa8460`, `FUN_14087c650`).
    pub d: i32,
}

impl Ctx<'_> {
    fn ignored(&self, id: u32) -> bool {
        self.ignore.get(id as usize).copied().unwrap_or(false)
    }

    fn tile(&self, tx: i32, ty: i32) -> &[u32] {
        let i = (ty.wrapping_mul(self.mesh.width as i32).wrapping_add(tx)) as u32 as usize;
        &self.mesh.tiles[i]
    }

    /// `FUN_140fa3dc0`: the first shape of `p`'s tile, not ignored, strictly holding `p` (shrunk).
    fn holder(&self, p: Point) -> Option<u32> {
        self.tile(tile_of(p[0]), tile_of(p[1]))
            .iter()
            .copied()
            .find(|&id| {
                !self.ignored(id) && strictly_inside(self.mesh.shapes[id as usize], p, self.d)
            })
    }

    /// `FUN_140fa2630`: the first shape of `p`'s tile other than `excl`, not ignored, holding `p`
    /// with its edges.
    fn holder_but(&self, p: Point, excl: u32, d: i32) -> Option<u32> {
        self.tile(tile_of(p[0]), tile_of(p[1]))
            .iter()
            .copied()
            .find(|&id| {
                id != excl && !self.ignored(id) && inside(self.mesh.shapes[id as usize], p, d)
            })
    }

    /// One tile of the raycast: the callback over the tile's shapes.
    #[allow(clippy::too_many_arguments)]
    fn ray_tile(
        &self,
        cb: Callback,
        list: &[u32],
        a: Point,
        b: Point,
        t: &mut i32,
        hit: &mut Hit,
        token: &mut u32,
    ) -> i32 {
        let d = self.d;
        let mut result = 0;
        // B and C first test the shape against the ray's box
        let mut bx = [
            if a[0] < b[0] { a[0] } else { b[0] },
            if a[1] < b[1] { a[1] } else { b[1] },
            if b[0] < a[0] { a[0] } else { b[0] },
            if b[1] < a[1] { a[1] } else { b[1] },
        ];
        if cb == Callback::Box {
            if a[0] == b[0] {
                bx[0] -= 1;
                bx[2] += 1;
            }
            if a[1] == b[1] {
                bx[1] -= 1;
                bx[3] += 1;
            }
        }
        for &id in list {
            if self.ignored(id) {
                continue;
            }
            let r = self.mesh.shapes[id as usize];
            let shrunk = [r[0] + d, r[1] + d, r[2] - d, r[3] - d];
            match cb {
                Callback::Corner => {
                    let mut tt = *t;
                    let mut side = 0u8;
                    let k = ray_rect(a, b, shrunk, &mut tt, &mut side);
                    if k == 0 {
                        continue;
                    }
                    let k = if k == 3 {
                        let own = id.wrapping_add(4);
                        if u32::from(side) == *token || own == *token {
                            *token = own;
                            continue;
                        }
                        2
                    } else {
                        k
                    };
                    result = k;
                    hit.side = side;
                    *t = tt;
                    hit.shape = id;
                }
                Callback::Box | Callback::Sight => {
                    let clip = [
                        if bx[0] < shrunk[0] { shrunk[0] } else { bx[0] },
                        if bx[1] < shrunk[1] { shrunk[1] } else { bx[1] },
                        if shrunk[2] < bx[2] { shrunk[2] } else { bx[2] },
                        if shrunk[3] < bx[3] { shrunk[3] } else { bx[3] },
                    ];
                    if !(clip[0] < clip[2] && clip[1] < clip[3]) {
                        continue;
                    }
                    let mut tt = *t;
                    let mut side = 0u8;
                    let k = ray_rect(a, b, shrunk, &mut tt, &mut side);
                    if k != 0 {
                        result = if k == 3 { 2 } else { k };
                        hit.shape = id;
                        hit.side = side;
                        *t = tt;
                    }
                }
            }
        }
        result
    }

    /// `FUN_140faa820`: the segment `a`–`b` walked tile by tile (4-connected, x first on a corner),
    /// the callback per tile; after a hit the walk goes on only to the hit's tile. Returns the last
    /// non-zero callback result (the nearest hit's: hits only replace nearer ones).
    fn raycast(
        &self,
        a: Point,
        b: Point,
        t: &mut i32,
        hit: &mut Hit,
        cb: Callback,
        token: &mut u32,
    ) -> i32 {
        let (x0, y0, x1, y1) = (a[0], a[1], b[0], b[1]);
        let dx = x1.wrapping_sub(x0);
        let adx = dx.wrapping_abs();
        let ady = y1.wrapping_sub(y0).wrapping_abs();
        let mut tx = tile_of(x0);
        let mut ty = tile_of(y0);
        let mut count: i32 = 1;
        let (step_x, err_x);
        if adx == 0 {
            step_x = 0;
            err_x = 0x7fc0_0000_i32;
        } else {
            let frac;
            if x0 < x1 {
                step_x = 1;
                count = tile_of(x1) - tx;
                frac = (tile_of(x0) + 1).wrapping_mul(ONE).wrapping_sub(x0);
            } else {
                step_x = -1;
                count = tx - tile_of(x1);
                frac = x0.wrapping_sub(tile_of(x0).wrapping_mul(ONE));
            }
            count += 1;
            err_x = ((i64::from(frac) * i64::from(ady)) >> 22) as i32;
        }
        let (step_y, mut err);
        if ady == 0 {
            step_y = 0;
            err = i32::MIN;
        } else {
            let frac = if y0 < y1 {
                step_y = 1;
                count += tile_of(y1) - ty;
                tile_of(y0)
                    .wrapping_mul(ONE)
                    .wrapping_sub(y0)
                    .wrapping_add(ONE)
            } else {
                step_y = -1;
                count += ty - tile_of(y1);
                y0.wrapping_sub(tile_of(y0).wrapping_mul(ONE))
            };
            err = err_x.wrapping_sub(((i64::from(frac) * i64::from(adx)) >> 22) as i32);
        }
        let mut result = 0;
        while 0 < count {
            let e = err;
            let list = self.tile(tx, ty);
            let r = self.ray_tile(cb, list, a, b, t, hit, token);
            let mut left = count;
            if r != 0 {
                let hy = ((i64::from(y1.wrapping_sub(y0)) * i64::from(*t)) >> 22) as i32 + y0;
                let hx = ((i64::from(dx) * i64::from(*t)) >> 22) as i32 + x0;
                let span =
                    (tile_of(hx) - tx).wrapping_abs() + 1 + (tile_of(hy) - ty).wrapping_abs();
                result = r;
                left = if count < span { count } else { span };
            }
            count = left - 1;
            err = err.wrapping_add(if err < 1 { ady } else { adx.wrapping_neg() });
            if e < 1 {
                tx += step_x;
            } else {
                ty += step_y;
            }
        }
        result
    }

    /// `FUN_140fa2d00`: an axis ray from `from` toward `end` in direction `dir` (0 −y, 1 +x, 2 +y,
    /// 3 −x) against the near side of shape `r` (shrunk), the ray's line strictly inside the
    /// shape's span; the hit point and side.
    fn axis_hit(from: Point, end: Point, r: Rect, d: i32, dir: u8) -> Option<(Point, u8)> {
        let [x, y] = from;
        match dir {
            1 => {
                let h = d + r[0];
                (r[1] + d < y && y < r[3] - d && h < end[0] && x <= h).then_some(([h, y], 3))
            }
            2 => {
                let h = d + r[1];
                (r[0] + d < x && x < r[2] - d && h < end[1] && y <= h).then_some(([x, h], 0))
            }
            3 => {
                let h = r[2] - d;
                (r[1] + d < y && y < r[3] - d && end[0] < h && h <= x).then_some(([h, y], 1))
            }
            _ => {
                let h = r[3] - d;
                (r[0] + d < x && x < r[2] - d && end[1] < h && h <= y).then_some(([x, h], 2))
            }
        }
    }

    /// `FUN_140fa8930` (and `FUN_140fa2420` per tile): an axis sweep from `from` to `to` in
    /// direction `dir` over the tiles of its row or column: the nearest shape side it meets
    /// (later hits must be nearer). A hit at `from` itself while the end is still `to` is checked
    /// backward from `to`: landing on `from` again means a shape too thin to count; the game has
    /// already written that hit's point and side by then ([`Sweep`]).
    fn sweep(&self, from: Point, to: Point, dir: u8) -> Sweep {
        let d = self.d;
        let mut end = to;
        let mut out = Sweep::default();
        let visit = |list: &[u32], end: &mut Point, out: &mut Sweep| {
            for &id in list {
                if self.ignored(id) {
                    continue;
                }
                let r = self.mesh.shapes[id as usize];
                let Some((hp, side)) = Self::axis_hit(from, *end, r, d, dir) else {
                    continue;
                };
                out.point = Some(hp);
                out.side = Some(side);
                if from == hp && *end == to {
                    let back = Self::axis_hit(*end, from, r, d, (dir.wrapping_sub(2)) & 3);
                    if back.is_some_and(|(bp, _)| bp == from) {
                        continue;
                    }
                }
                *end = hp;
                out.shape = Some(id);
                out.found = true;
            }
        };
        match dir {
            1 => {
                let row = tile_of(from[1]);
                let mut x = tile_of(from[0]) as u32;
                while x <= tile_of(to[0]) as u32 {
                    visit(self.tile(x as i32, row), &mut end, &mut out);
                    x = x.wrapping_add(1);
                }
            }
            2 => {
                let col = tile_of(from[0]);
                let mut y = tile_of(from[1]) as u32;
                while y <= tile_of(to[1]) as u32 {
                    visit(self.tile(col, y as i32), &mut end, &mut out);
                    y = y.wrapping_add(1);
                }
            }
            3 => {
                let row = tile_of(from[1]);
                let mut x = tile_of(from[0]);
                while tile_of(to[0]) <= x {
                    visit(self.tile(x, row), &mut end, &mut out);
                    x -= 1;
                }
            }
            _ => {
                let col = tile_of(from[0]);
                let mut y = tile_of(from[1]);
                while tile_of(to[1]) <= y {
                    visit(self.tile(col, y), &mut end, &mut out);
                    y -= 1;
                }
            }
        }
        out
    }

    /// `FUN_140fa8e60`: whether the straight walk from `a` to `b` is blocked (the smoothing's line of
    /// sight): an axis sweep along an axis, else the raycast with callback C.
    fn sight_blocked(&self, a: Point, b: Point) -> bool {
        if a[0] == b[0] {
            let dir = if 0 < b[1] - a[1] { 2 } else { 0 };
            return self.sweep(a, b, dir).found;
        }
        if a[1] == b[1] {
            let dir = if 0 < b[0] - a[0] { 1 } else { 3 };
            return self.sweep(a, b, dir).found;
        }
        let mut t = T_START;
        let mut hit = Hit::default();
        let mut token = 0;
        self.raycast(a, b, &mut t, &mut hit, Callback::Sight, &mut token) != 0
    }
}

/// `DAT_1439e60e5`: 1 in the image and at run time: the rectangle heuristic (0 is the Euclidean).
const RECT_HEURISTIC: bool = true;
/// `DAT_143cc1f50`: a polygon vertex no corner node reached yet.
const UNSET: i32 = -0x40_0000;
/// `DAT_143cc1f54`: the outline tracer's step limit.
const TRACE_STEPS: i32 = 2000;
/// `DAT_143cc1f60`: the four unit directions (x, y) of a type-3 node's nudges.
const NUDGES: [Point; 4] = [[0, ONE], [ONE, 0], [0, -ONE], [-ONE, 0]];
/// No shape, node or polygon.
const NONE: u32 = u32::MAX;

/// The goal of a search (`local_148`): its point, the unit's radius, the target's half sizes,
/// their sums and the range.
#[derive(Clone, Copy, Debug)]
struct Goal {
    at: Point,
    r: i32,
    rx: i32,
    ry: i32,
    rrx: i64,
    rry: i64,
    range: i32,
    range2: i64,
}

impl Goal {
    /// The squared distance from `p` to the target's box grown by the radius (`>> 22` each).
    fn rect_distance2(&self, p: Point) -> i64 {
        let lx = i64::from((p[0].wrapping_sub(self.at[0])).wrapping_abs()) - self.rrx;
        let ly = i64::from((p[1].wrapping_sub(self.at[1])).wrapping_abs()) - self.rry;
        let a = if lx < 0 { 0 } else { lx * lx };
        let b = if ly < 0 { 0 } else { ly * ly };
        (b >> 22) + (a >> 22)
    }

    /// `FUN_140fa3f10`: the estimate from `p` to the goal.
    fn heuristic(&self, p: Point) -> i32 {
        if !RECT_HEURISTIC {
            let fy = (self.at[1].wrapping_sub(p[1])) as f32 * TO_FLOAT;
            let fx = (self.at[0].wrapping_sub(p[0])) as f32 * TO_FLOAT;
            return (fmath::sqrt(fx * fx + fy * fy) * TO_FIXED) as i32;
        }
        let lx = i64::from((p[0].wrapping_sub(self.at[0])).wrapping_abs()) - self.rrx;
        let ly = i64::from((p[1].wrapping_sub(self.at[1])).wrapping_abs()) - self.rry;
        let a = if lx < 0 { 0 } else { (lx * lx) as u64 };
        let b = if ly < 0 { 0 } else { (ly * ly) as u64 };
        isqrt((b & !0x3f_ffff).wrapping_add(a & !0x3f_ffff)) as i32
    }
}

/// A node's payload (`+0x18..+0x30`, a `std::variant`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// 0: the start. Its shape field is always none: `FUN_140fa4410`, the only maker of a type-0
    /// node, writes -1 there (`local_178`), so the expansion's branch for a type-0 node on a
    /// shape (a side node, `FUN_140fa3fe0`) never runs.
    Start,
    /// 1: a waypoint.
    Waypoint,
    /// 2: a point on a shape's side.
    Side { shape: u32, side: u8 },
    /// 3: a polygon corner or a terminal node: the walk's direction (1 backward), the vertices
    /// it covers, its polygon, the callback A token and the sides around the corner.
    Corner {
        dir: u8,
        lo: u32,
        hi: u32,
        poly: u32,
        token: u32,
        prev: u8,
        next: u8,
    },
}

/// A node (`0x34` bytes).
#[derive(Clone, Copy, Debug)]
struct Node {
    g: i32,
    f: i32,
    idx: u32,
    parent: u32,
    at: Point,
    kind: Kind,
}

/// A polygon (`0x68` bytes): the outline of a cluster of touching shapes, held in the bounds.
#[derive(Clone, Debug, Default)]
struct Poly {
    vertices: Vec<Point>,
    sides: Vec<u8>,
    shapes: Vec<u32>,
    costs: Vec<i32>,
    flag: bool,
}

/// One search's state (the pather's arrays at `+0x08`..`+0x80`).
struct Search<'a> {
    ctx: Ctx<'a>,
    goal: Goal,
    bounds: [i32; 4],
    open: Vec<Node>,
    arena: Vec<Node>,
    polys: Vec<Poly>,
    /// `local_1a0`, `local_194`.
    goal_node: u32,
    best: u32,
    /// `DAT_143f0f360`, callback A's token: a global in the game, but set before each raycast that
    /// reads it (type 3), so no value carries over between searches.
    token: u32,
}

/// `FUN_140fa3bc0`: whether a polygon corner at `v` between sides `prev` and `next` emits a node
/// toward `g`, and the token its raycast ignores (0xff none).
fn corner_emits(prev: u8, next: u8, v: Point, g: Point) -> Option<u8> {
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
fn clip(mut a: Point, mut b: Point, bx: [i32; 4]) -> Option<(Point, Point)> {
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
    let code = |[x, y]: Point| {
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

const fn in_box(p: Point, b: [i32; 4]) -> bool {
    b[0] <= p[0] && b[1] <= p[1] && p[0] <= b[2] && p[1] <= b[3]
}

/// `p` held in the box `b` on each axis (`[x0, y0, x1, y1]`).
const fn clamp_to_box(p: Point, b: [i32; 4]) -> Point {
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
const fn cheb(a: Point, b: Point) -> i32 {
    let dy = (a[1].wrapping_sub(b[1])).wrapping_abs();
    let dx = (a[0].wrapping_sub(b[0])).wrapping_abs();
    if dy < dx { dx } else { dy }
}

impl Search<'_> {
    /// Append a node to the arena (`FUN_140fa3fe0`, `FUN_140fa40d0`, `FUN_140fa4210`,
    /// `FUN_140fa4340`): `f = g + h(at)`.
    fn add(&mut self, g: i32, parent: u32, at: Point, kind: Kind) -> Node {
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
    fn push(&mut self, n: Node) {
        let at = (0..self.open.len())
            .rev()
            .find(|&i| n.f < self.open[i].f)
            .map_or(0, |i| i + 1);
        self.open.insert(at, n);
    }

    fn side_node(&mut self, g: i32, parent: u32, shape: u32, side: u8, at: Point) {
        let n = self.add(g, parent, at, Kind::Side { shape, side });
        self.push(n);
    }

    /// `FUN_140fa3860`: a node at `pt` reached with cost `g`: within range and better than the goal
    /// so far, it becomes the goal (true); else, nearer than the best so far, the best.
    #[allow(clippy::too_many_arguments)]
    fn goal_test(
        &mut self,
        g: i32,
        parent: u32,
        dir: u8,
        lo: u32,
        hi: u32,
        poly: u32,
        pt: Point,
    ) -> bool {
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
        let better = self.goal_node == NONE || g < self.arena[self.goal_node as usize].g;
        if self.goal.range2 < d2 || !better {
            let b = self.arena[self.best as usize];
            let h = i64::from(b.f.wrapping_sub(b.g));
            if d2 < (h * h) >> 22 {
                let n = self.add(g, parent, pt, kind);
                self.best = n.idx;
            }
            return false;
        }
        let n = self.add(g, parent, pt, kind);
        self.goal_node = n.idx;
        true
    }

    /// `FUN_140fa8210`: the goal's foot on an axis edge `a`–`b` (horizontal when `dx != 0`) as a
    /// goal candidate.
    #[allow(clippy::too_many_arguments)]
    fn foot_test(
        &mut self,
        poly: u32,
        a: Point,
        b: Point,
        dx: i32,
        lo: u32,
        hi: u32,
        dir: u8,
        g0: i32,
        parent: u32,
    ) {
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
            let _ = self.goal_test(g, parent, dir, lo, hi, poly, foot);
        }
    }

    /// `FUN_140fa2740`, a popped node's test: the best (lowest estimate) so far; in range and
    /// cheaper than the goal: the new goal (true); no cheaper than the goal: pruned (true).
    fn pop_test(&mut self, n: &Node) -> bool {
        let h = n.f.wrapping_sub(n.g);
        let b = self.arena[self.best as usize];
        if h < b.f.wrapping_sub(b.g) {
            self.best = n.idx;
        }
        let better = |s: &Self| s.goal_node == NONE || n.g < s.arena[s.goal_node as usize].g;
        if (h < self.goal.range || self.goal.rect_distance2(n.at) <= self.goal.range2)
            && better(self)
        {
            self.goal_node = n.idx;
            return true;
        }
        !better(self)
    }
}

impl Search<'_> {
    /// `FUN_140fa2fa0`: append a vertex to the polygon being traced, unless the outline closes:
    /// after a full turn (`|winding| == 4`) the vertex passes the first one along the start side's
    /// line. On closing, a first vertex whose side repeats the last one's is dropped.
    #[allow(clippy::too_many_arguments)]
    fn append(
        poly: &mut Poly,
        pos: Point,
        first: Point,
        vertical: bool,
        s0: u32,
        side: u8,
        shape: u32,
        winding: i32,
    ) -> bool {
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
    const fn along(p: Point, side: u32) -> i32 {
        if side & 1 == 1 { p[1] } else { p[0] }
    }

    /// `FUN_140fa9630`: the trace left the bounds: walk the bounds' border from `pos` in the turn
    /// the side gives, to the first shape side the sweep meets (0) or the border's corner (1).
    #[allow(clippy::too_many_arguments)]
    fn border_walk(
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
    fn trace(
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
                    if Self::append(poly, pos, first, vertical, s0, side as u8, shape, wp) {
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
    fn nearest_contact(
        &self,
        contacts: &[crate::mesh::Contact],
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
    fn edge_holding(
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
            let vs = poly.shapes[cur as usize];
            if p == vc && vs != NONE {
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
                if o[0] < o[2] && o[1] < o[3] && inside(o, p, 0) {
                    return Some(cur);
                }
            }
            if side == u32::from(s_cur) {
                let vn = v[nxt as usize];
                let hit = match s_cur {
                    0 if vc[1] == p[1] => {
                        let (a, b) = (vc[0], vn[0]);
                        let start = if prev < 2 { a <= p[0] } else { a < p[0] };
                        start
                            && if s_i == 3 || s_i == 0 {
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
                        start && if s_i < 2 { p[1] <= b } else { p[1] < b }
                    }
                    2 if vc[1] == p[1] => {
                        let (a, b) = (vc[0], vn[0]);
                        let start = if s_i.wrapping_sub(1) < 2 {
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
                        let start = if s_i.wrapping_sub(2) < 2 {
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
                };
                if hit {
                    return Some(cur);
                }
            }
            i = nxt + 1;
            prev = s_cur;
            cur = nxt;
            if i >= limit {
                return None;
            }
        }
    }

    /// `FUN_140fa7b50`: from `p` on a new polygon's closing edge, walk the outline both ways at
    /// once, the cheaper side first (ties forward): each corner the goal test does not end
    /// becomes a corner node toward the goal when it emits; each edge gets its foot test.
    fn walk_new(&mut self, pi: u32, p: Point, g0: i32, parent: u32) {
        let poly = self.polys[pi as usize].clone();
        let v = &poly.vertices;
        let n = v.len() as u32;
        let last = n - 1;
        let dxb = v[last as usize][0] - p[0];
        let dyb = v[last as usize][1] - p[1];
        let dxf = v[0][0] - p[0];
        let dyf = v[0][1] - p[1];
        self.foot_test(pi, p, v[last as usize], dxb, last, 0, 1, g0, parent);
        self.foot_test(pi, p, v[0], dxf, 0, 0, 0, g0, parent);
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
                if !self.goal_test(cb, parent, 1, bi, 0, pi, vv)
                    && sp != sn
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
                self.foot_test(pi, vv, w, dx, bi - 1, 0, 1, cb, parent);
                cb = cb.wrapping_add(cheb(vv, w));
                bi -= 1;
            } else {
                let vv = v[fi as usize];
                let sn = poly.sides[fi as usize];
                let sh = poly.shapes[fi as usize];
                let sp = poly.sides[((n - 1 + fi) % n) as usize];
                if !self.goal_test(cf, parent, 0, 0, fi, pi, vv)
                    && sp != sn
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
                self.foot_test(pi, vv, w, dx, 0, fi, 0, cf, parent);
                cf = cf.wrapping_add(cheb(vv, w));
            }
            if fi >= bi {
                return;
            }
        }
    }
}

impl Search<'_> {
    /// `FUN_140fa4410`: the start's node (type 0, cost 0, the Euclidean estimate); 3 when the start
    /// already lies within range, 0 when it is free (the node opened), else the escape
    /// candidates (type 2) around it within twice the radius: 2 for fewer than two, else 1 with
    /// only the cheapest kept open.
    fn start(&mut self, p: Point) -> i32 {
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
    fn at_fraction(p: Point, t: Point, frac: i32) -> Point {
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
    fn settle(&self, h: &mut Point, hit: &mut Hit) {
        let d = self.ctx.d;
        snap_to_side(h, self.ctx.mesh.shapes[hit.shape as usize], hit.side, d);
        while let Some(id) = self.ctx.holder(*h) {
            snap_to_side(h, self.ctx.mesh.shapes[id as usize], hit.side, d);
            hit.shape = id;
        }
    }

    /// `FUN_140fa2b20`: the two legs of an L toward `c` from node `b`, the first along direction
    /// `a`, the second along `second`; each leg's cost is |Δx|, then |Δy|, of `delta` through float.
    fn legs(
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

impl Search<'_> {
    /// Type 2, a node on a shape's side: a polygon already holding its point is walked again from
    /// that edge (both ways, while cheaper than the vertices' costs); else the outline of its
    /// cluster is traced into a new polygon and walked. Some(4) when the outline fails.
    fn side_expand(
        &mut self,
        n: Node,
        shape: u32,
        side: u8,
    ) -> Result<Option<ShortPath>, GenieError> {
        let p = n.at;
        let d = self.ctx.d;
        for i in 0..self.polys.len() {
            let Some(e) =
                Self::edge_holding(&self.polys[i], p, u32::from(side), self.ctx.mesh, shape, d)
            else {
                continue;
            };
            if !RECT_HEURISTIC || !self.polys[i].flag {
                return Ok(None);
            }
            self.rewalk(n, i as u32, e);
            return Ok(None);
        }
        self.polys.push(Poly::default());
        let mut poly = Poly::default();
        let mut rc = self.trace(shape, u32::from(side), Some(p), &mut poly)?;
        let last = self.polys.len() - 1;
        'older: for j in 0..last {
            for k in 0..poly.vertices.len() {
                let hit = Self::edge_holding(
                    &self.polys[j],
                    poly.vertices[k],
                    u32::from(poly.sides[k]),
                    self.ctx.mesh,
                    poly.shapes[k],
                    d,
                );
                if hit.is_some() {
                    rc = 2;
                    continue 'older;
                }
            }
        }
        let empty = poly.vertices.is_empty();
        self.polys[last] = poly;
        if rc == 1 || rc == 2 || empty {
            return Ok(Some(ShortPath {
                result: 4,
                points: Vec::new(),
            }));
        }
        self.walk_new(last as u32, p, n.g, n.idx);
        Ok(None)
    }

    /// The re-walk of a polygon from edge `e` (0x140fa63d4..0x140fa6639): backward from `v[e]`,
    /// forward from `v[e + 1]`, the forward side on equal costs, each while its cost stays below
    /// the vertex's; vertices no corner node reached are passed over.
    fn rewalk(&mut self, n: Node, pi: u32, e: u32) {
        let p = n.at;
        let poly = &self.polys[pi as usize];
        let m = poly.vertices.len() as u32;
        let next0 = (e + 1) % m;
        let mut bv = e;
        let mut fv = next0;
        let mut gb = cheb(poly.vertices[e as usize], p).wrapping_add(n.g);
        let mut gf = cheb(poly.vertices[next0 as usize], p).wrapping_add(n.g);
        let alive = |cost: i32, g: i32| g < cost || cost == UNSET;
        let mut alive_b = alive(poly.costs[e as usize], gb);
        let mut alive_f = alive(poly.costs[next0 as usize], gf);
        loop {
            if !(alive_b || alive_f) {
                return;
            }
            let forward = (gf <= gb && alive_f) || !alive_b;
            let poly = self.polys[pi as usize].clone();
            if forward {
                let v = poly.vertices[fv as usize];
                let sp = poly.sides[((fv + m - 1) % m) as usize];
                let sn = poly.sides[fv as usize];
                let sh = poly.shapes[fv as usize];
                if poly.costs[fv as usize] != UNSET
                    && !self.goal_test(gf, n.idx, 0, e, fv, pi, v)
                    && sp != sn
                    && sh != NONE
                    && let Some(token) = corner_emits(sp, sn, v, self.goal.at)
                {
                    let kind = Kind::Corner {
                        dir: 0,
                        lo: e,
                        hi: fv,
                        poly: pi,
                        token: u32::from(token),
                        prev: sp,
                        next: sn,
                    };
                    let node = self.add(gf, n.idx, v, kind);
                    self.push(node);
                    self.polys[pi as usize].costs[fv as usize] = gf;
                }
                let costs = &self.polys[pi as usize].costs;
                let mut at = v;
                loop {
                    fv = (fv + 1) % m;
                    let w = poly.vertices[fv as usize];
                    gf = gf.wrapping_add(cheb(at, w));
                    alive_f = gf < costs[fv as usize];
                    at = w;
                    if costs[fv as usize] != UNSET {
                        break;
                    }
                }
            } else {
                let v = poly.vertices[bv as usize];
                let sp = poly.sides[((bv + m - 1) % m) as usize];
                let sn = poly.sides[bv as usize];
                let sh = poly.shapes[bv as usize];
                if poly.costs[bv as usize] != UNSET
                    && !self.goal_test(gb, n.idx, 1, bv, next0, pi, v)
                    && sp != sn
                    && sh != NONE
                    && let Some(token) = corner_emits(sp, sn, v, self.goal.at)
                {
                    let kind = Kind::Corner {
                        dir: 1,
                        lo: bv,
                        hi: next0,
                        poly: pi,
                        token: u32::from(token),
                        prev: sp,
                        next: sn,
                    };
                    let node = self.add(gb, n.idx, v, kind);
                    self.push(node);
                    self.polys[pi as usize].costs[bv as usize] = gb;
                }
                let costs = &self.polys[pi as usize].costs;
                let mut at = v;
                loop {
                    bv = (m - 1 + bv) % m;
                    let w = poly.vertices[bv as usize];
                    gb = gb.wrapping_add(cheb(at, w));
                    alive_b = gb < costs[bv as usize];
                    at = w;
                    if costs[bv as usize] != UNSET {
                        break;
                    }
                }
            }
        }
    }

    /// Type 3, a polygon corner: the raycast toward the goal's range point ignoring the corner's
    /// own touch (the token); a touch tries the four nudged points; then a crossing becomes a side
    /// node, a clear ray a terminal goal candidate, and a ray that still touches the axis L.
    fn corner_expand(&mut self, n: Node, token: u32, prev: u8, next: u8) -> Result<(), GenieError> {
        let p = n.at;
        self.token = token;
        let mut t = range_point(self.goal.range, p, self.goal.at)?;
        let mut frac = T_START;
        let reach = n.g.wrapping_add(float_distance(t, p));
        if self.goal_node != NONE && (reach >= self.arena[self.goal_node as usize].g) {
            return Ok(());
        }
        let mut hit = Hit {
            shape: NONE,
            side: 0,
        };
        let mut k = self
            .ctx
            .raycast(p, t, &mut frac, &mut hit, Callback::Corner, &mut self.token);
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
                k = self
                    .ctx
                    .raycast(p, tj, &mut f2, &mut hit, Callback::Corner, &mut self.token);
                if k != 2 {
                    frac = f2;
                    t = tj;
                    break;
                }
            }
            if k == 2 {
                self.corner_axis(n, t);
                return Ok(());
            }
        }
        if k == 1 {
            let mut h = Self::at_fraction(p, t, frac);
            self.settle(&mut h, &mut hit);
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
        if self.goal_node == NONE || node.g < self.arena[self.goal_node as usize].g {
            self.goal_node = node.idx;
        }
        Ok(())
    }

    /// Type 3 when every ray touches: along x toward `t`, then along y; a leg's hit becomes a
    /// side node; with both legs clear, one tile toward `t` (at most) becomes a waypoint that may
    /// be the best (by cost, not estimate).
    fn corner_axis(&mut self, n: Node, t: Point) {
        let p = n.at;
        let dx = t[0].wrapping_sub(p[0]);
        let dy = t[1].wrapping_sub(p[1]);
        let m = [t[0], p[1]];
        let sw = self.ctx.sweep(p, m, if 0 < dx { 1 } else { 3 });
        if let Some((shape, side, at)) = sw.counted() {
            let g = n.g.wrapping_add(float_distance(p, at));
            self.side_node(g, n.idx, shape, side, at);
            return;
        }
        let sw2 = self.ctx.sweep(m, t, if 0 < dy { 2 } else { 0 });
        if !sw2.found {
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
            if self.best == NONE || w.g < self.arena[self.best as usize].g {
                self.best = w.idx;
            }
            return;
        }
        let mid = self.add(
            through_float(dx.wrapping_abs()).wrapping_add(n.g),
            n.idx,
            m,
            Kind::Waypoint,
        );
        // the second sweep counted a shape (`found`, tested above): its outputs are written
        let (shape, side, at) = sw2
            .counted()
            .expect("a sweep that counts a shape writes its point and side");
        let g = through_float((at[1].wrapping_sub(p[1])).wrapping_abs()).wrapping_add(mid.g);
        self.side_node(g, mid.idx, shape, side, at);
    }
}

/// The short pather's arguments (`FUN_140f9d3a0`'s block, which `FUN_140f9e540` and
/// `FUN_140fa0110` fill): the start and the goal in fixed point (the callers truncate them), the
/// unit's radius, the target's half sizes (when the goal is the target's own position) and the
/// range as floats, and the terrain classes the unit cannot cross (`mask ^ 0xff`). The search
/// ([`Pather::search`]) and the start test ([`Pather::start_blocked`]) truncate the floats.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Query {
    /// The start.
    pub from: Point,
    /// The goal.
    pub to: Point,
    /// The unit's radius (`+0xc`).
    pub radius: f32,
    /// The target's half sizes, when the goal is its own position (`+0x10`, `+0x14`).
    pub target_size: [f32; 2],
    /// The range (`+0x18`).
    pub range: f32,
    /// The terrain classes the unit cannot cross, as a tile mask (`+0x1c`).
    pub blocked: u64,
}

impl Query {
    /// `FUN_140f9d300`'s and `FUN_140f9d3e0`'s casts: each float times 2^22 by `CVTTSS2SI` into
    /// a 64-bit register, its low 32 bits kept (the radius through `uint`, the same bits).
    fn fixed(&self) -> Fixed {
        let fixed = |f: f32| crate::checkpoint::cvttss2si_64(f * TO_FIXED) as i32;
        Fixed {
            from: self.from,
            to: self.to,
            radius: fixed(self.radius),
            target_size: [fixed(self.target_size[0]), fixed(self.target_size[1])],
            range: fixed(self.range),
            blocked: self.blocked,
        }
    }
}

/// A [`Query`] in fixed point, as `FUN_140fa53d0` and `FUN_140faa170` take it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fixed {
    from: Point,
    to: Point,
    radius: i32,
    target_size: [i32; 2],
    range: i32,
    blocked: u64,
}

/// What a search returns (`FUN_140fa53d0`'s result) with the path's points from the start on, in
/// fixed point (the game appends them as floats, `{x, 0, y}`, through the path's own dedup).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortPath {
    /// 0 already in range, 1 reached, 2 a partial path, 3 the start blocked, 4 invalid.
    pub result: u8,
    /// The path's points.
    pub points: Vec<Point>,
}

/// The target of a short path (`FUN_140f9e540`'s argument 5): its position and its master's half
/// sizes (`+0x5d8`, `+0x5dc`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    /// The target.
    pub id: i32,
    /// Its place.
    pub position: [f32; 2],
    /// Its master's half sizes (`+0x5d8`, `+0x5dc`).
    pub half_size: [f32; 2],
}

/// An object the call is asked to ignore (arguments 6 and 7): its id and its shape in the mesh
/// (`+0x64c`), if it has one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Listed {
    /// The object.
    pub id: i32,
    /// Its shape in the mesh (`+0x64c`), if it has one.
    pub shape: Option<u32>,
}

/// A call of the short pather in the game's own terms (`FUN_140f9e540`, the short branch): the
/// start and the goal on the map (x, y), the unit's radius and range, the target, the objects to
/// ignore, the map's size and the unit's terrain mask.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    /// The start.
    pub from: [f32; 2],
    /// The goal.
    pub to: [f32; 2],
    /// The unit's radius.
    pub radius: f32,
    /// Its range.
    pub range: f32,
    /// The target, when the goal is an object.
    pub target: Option<Target>,
    /// The objects it ignores.
    pub listed: Vec<Listed>,
    /// The map's size in tiles.
    pub map: [i32; 2],
    /// The terrain classes the unit can cross (its restriction's mask).
    pub mask: u8,
}

/// What the entry makes of a call.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    /// 0: the start lies within range of the target (`FUN_14118faf0` less the radius): the path is
    /// cleared.
    InRange,
    /// 4: the start or the goal lies off the map.
    Invalid,
    /// A search with the shapes it ignores.
    Search {
        /// The query.
        query: Query,
        /// The shapes it ignores.
        ignore: Vec<u32>,
    },
}

impl Call {
    /// `FUN_140f9e540` up to the search: the target's range test, the map test, fixed point by
    /// truncation (`(int)(longlong)(f * 2^22)`); the listed objects' shapes are ignored, except the
    /// target's when the goal is the target's own position, whose half sizes then count. The
    /// target's gap (`FUN_14118faf0`) clamps each axis by `MAXSS` (an unordered one is 0) and is
    /// in range by `COMISS`/`JC` (0x140f9e5f5: an unordered one is not); an end lies off the map
    /// below 0 (`COMISS`/`JA`) or at the size (`JNC`), so an unordered coordinate is on it.
    #[must_use]
    pub fn entry(&self) -> Entry {
        if let Some(t) = self.target {
            let fx = fmath::maxss((self.from[0] - t.position[0]).abs() - t.half_size[0], 0.0);
            let fy = fmath::maxss((self.from[1] - t.position[1]).abs() - t.half_size[1], 0.0);
            if fmath::sqrt(fy * fy + fx * fx) - self.radius <= self.range {
                return Entry::InRange;
            }
        }
        let on_map = |[x, y]: [f32; 2]| {
            fmath::at_most_or_unordered(0.0, x)
                && fmath::at_most_or_unordered(0.0, y)
                && fmath::below_or_unordered(x, self.map[0] as f32)
                && fmath::below_or_unordered(y, self.map[1] as f32)
        };
        if !on_map(self.from) || !on_map(self.to) {
            return Entry::Invalid;
        }
        // CVTTSS2SI into a 64-bit register, its low 32 bits kept
        let fixed = |f: f32| crate::checkpoint::cvttss2si_64(f * TO_FIXED) as i32;
        let to = [fixed(self.to[0]), fixed(self.to[1])];
        let at_target = self
            .target
            .filter(|t| to == [fixed(t.position[0]), fixed(t.position[1])]);
        let ignore = self
            .listed
            .iter()
            .filter(|l| at_target.is_none_or(|t| l.id != t.id))
            .filter_map(|l| l.shape)
            .collect();
        let size = at_target.map_or([0.0, 0.0], |t| t.half_size);
        Entry::Search {
            query: Query {
                from: [fixed(self.from[0]), fixed(self.from[1])],
                to,
                radius: self.radius,
                target_size: size,
                range: self.range,
                blocked: u64::from(self.mask ^ 0xff),
            },
            ignore,
        }
    }
}

/// A path's fixed-point points as the game appends them (`{x * 2^-22, 0, y * 2^-22}` through the
/// path's dedup, `FUN_140f95a10`).
#[must_use]
pub fn to_path(points: &[Point]) -> crate::tile_path::Path {
    let mut path = crate::tile_path::Path::default();
    for &[x, y] in points {
        let _ = path.push([x as f32 * TO_FLOAT, 0.0, y as f32 * TO_FLOAT]);
    }
    path
}

/// The pather's state between searches: its temporary shapes (`pather + 0x98`, removed at the end
/// of each call by `FUN_140f9d2b0`) and the flag per ignored shape (`+0xd8`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pather {
    temp: Vec<u32>,
    ignore: Vec<bool>,
}

impl Pather {
    /// `FUN_140fa8650`: the runs of tiles in the bounds whose terrain the unit cannot cross, as
    /// temporary shapes (each tile ±half a tile).
    fn add_terrain(
        &mut self,
        mesh: &mut Mesh,
        words: &dyn Fn(u32, u32) -> u64,
        blocked: u64,
        b: [i32; 4],
    ) {
        let mut ty = tile_of(b[1]) as u32;
        let mut y1 = (ty as i32).wrapping_mul(ONE).wrapping_add(0x60_0000);
        while ty <= tile_of(b[3]) as u32 {
            let mut run: Option<u32> = None;
            let mut tx = tile_of(b[0]) as u32;
            while tx <= tile_of(b[2]) as u32 {
                if words(tx, ty) & blocked == 0 {
                    if let Some(a) = run.take() {
                        let r = [
                            (a as i32).wrapping_mul(ONE) - 0x20_0000,
                            (ty as i32).wrapping_mul(ONE) - 0x20_0000,
                            (tx as i32).wrapping_mul(ONE) + 0x20_0000,
                            y1,
                        ];
                        self.temp.push(mesh.add(r));
                    }
                } else if run.is_none() {
                    run = Some(tx);
                }
                tx += 1;
            }
            if let Some(a) = run {
                let r = [
                    (a as i32).wrapping_mul(ONE) - 0x20_0000,
                    (ty as i32).wrapping_mul(ONE) - 0x20_0000,
                    (tx as i32).wrapping_mul(ONE) + 0x20_0000,
                    y1,
                ];
                self.temp.push(mesh.add(r));
            }
            ty += 1;
            y1 = y1.wrapping_add(ONE);
        }
        if self.ignore.len() < mesh.shapes.len() {
            self.ignore.resize(mesh.shapes.len(), false);
        }
    }

    /// `FUN_140fa3b60`: the temporary shapes leave the mesh, in the order they came.
    fn remove_terrain(&mut self, mesh: &mut Mesh) {
        for id in self.temp.drain(..) {
            mesh.remove(id);
        }
    }

    /// One call of the short pather (`FUN_140f9d2f0` the ignored shapes, `FUN_140f9d300` the
    /// search with the query's floats in fixed point, `FUN_140f9d2b0` the cleanup): `ignore` the
    /// shape ids of the units it ignores, `words` the obstruction manager's tile words.
    pub fn search(
        &mut self,
        mesh: &mut Mesh,
        words: &dyn Fn(u32, u32) -> u64,
        q: &Query,
        ignore: &[u32],
    ) -> Result<ShortPath, GenieError> {
        if self.ignore.len() < mesh.shapes.len() {
            self.ignore.resize(mesh.shapes.len(), false);
        }
        for &id in ignore {
            self.ignore[id as usize] = true;
        }
        let result = self.core(mesh, words, &q.fixed());
        for &id in ignore {
            if let Some(f) = self.ignore.get_mut(id as usize) {
                *f = false;
            }
        }
        self.remove_terrain(mesh);
        result
    }

    /// `FUN_140faa170` (from `FUN_140fa0110`, the drop-off walker's start test): the search's
    /// prologue as [`Pather::search`]'s, without ignored shapes, then only the start
    /// (`FUN_140fa4410`): true when it is out of the goal's range and blocked with fewer than
    /// two escape candidates (its 2). The temporary terrain shapes leave the mesh after it
    /// (`FUN_140f9d2b0`).
    pub fn start_blocked(
        &mut self,
        mesh: &mut Mesh,
        words: &dyn Fn(u32, u32) -> u64,
        q: &Query,
    ) -> bool {
        let q = &q.fixed();
        if self.ignore.len() < mesh.shapes.len() {
            self.ignore.resize(mesh.shapes.len(), false);
        }
        let r = q.radius;
        let d = 0x20_0000 - r;
        let w = mesh.width as i32;
        let cap = w.wrapping_mul(ONE) - 1 - r;
        let [sx, sy] = q.from;
        let [gx, gy] = q.to;
        let hold_high = |v: i32| {
            if v.wrapping_add(0x100_0000) < cap {
                v.wrapping_add(0x100_0000)
            } else {
                cap
            }
        };
        let hold_low = |v: i32| {
            if r < v.wrapping_sub(0x100_0000) {
                v.wrapping_sub(0x100_0000)
            } else {
                r
            }
        };
        let bounds = [
            hold_low(if sx < gx { sx } else { gx }),
            hold_low(if sy < gy { sy } else { gy }),
            hold_high(if gx < sx { sx } else { gx }),
            hold_high(if gy < sy { sy } else { gy }),
        ];
        self.remove_terrain(mesh);
        self.add_terrain(mesh, words, q.blocked, bounds);
        let goal = Goal {
            at: [gx, gy],
            r,
            rx: q.target_size[0],
            ry: q.target_size[1],
            rrx: i64::from(r.wrapping_add(q.target_size[0])),
            rry: i64::from(r.wrapping_add(q.target_size[1])),
            range: q.range,
            range2: (i64::from(q.range) * i64::from(q.range)) >> 22,
        };
        let ignore = self.ignore.clone();
        let mut s = Search {
            ctx: Ctx {
                mesh,
                ignore: &ignore,
                d,
            },
            goal,
            bounds,
            open: Vec::new(),
            arena: Vec::new(),
            polys: Vec::new(),
            goal_node: NONE,
            best: 0,
            token: 0,
        };
        let blocked = s.start(q.from) == 2;
        self.remove_terrain(mesh);
        blocked
    }

    /// `FUN_140fa53d0`. Its call counter (`DAT_143f0f364 += 1` at entry) has no reader (its only
    /// xref is that read-write) and is not held.
    fn core(
        &mut self,
        mesh: &mut Mesh,
        words: &dyn Fn(u32, u32) -> u64,
        q: &Fixed,
    ) -> Result<ShortPath, GenieError> {
        let r = q.radius;
        let d = 0x20_0000 - r;
        let w = mesh.width as i32;
        let cap = w.wrapping_mul(ONE) - 1 - r;
        let [sx, sy] = q.from;
        let [mut gx, mut gy] = q.to;
        let hold_high = |v: i32| {
            if v.wrapping_add(0x100_0000) < cap {
                v.wrapping_add(0x100_0000)
            } else {
                cap
            }
        };
        let hold_low = |v: i32| {
            if r < v.wrapping_sub(0x100_0000) {
                v.wrapping_sub(0x100_0000)
            } else {
                r
            }
        };
        let bounds = [
            hold_low(if sx < gx { sx } else { gx }),
            hold_low(if sy < gy { sy } else { gy }),
            hold_high(if gx < sx { sx } else { gx }),
            hold_high(if gy < sy { sy } else { gy }),
        ];
        let invalid = ShortPath {
            result: 4,
            points: Vec::new(),
        };
        if sx < bounds[0] || sy < bounds[1] || bounds[2] < sx || bounds[3] < sy {
            return Ok(invalid);
        }
        if gx < bounds[0] || gy < bounds[1] || bounds[2] < gx || bounds[3] < gy {
            match clip(q.from, [gx, gy], bounds) {
                Some((_, end)) => [gx, gy] = end,
                None => return Ok(invalid),
            }
        }
        self.remove_terrain(mesh);
        self.add_terrain(mesh, words, q.blocked, bounds);
        let goal = Goal {
            at: [gx, gy],
            r,
            rx: q.target_size[0],
            ry: q.target_size[1],
            rrx: i64::from(r.wrapping_add(q.target_size[0])),
            rry: i64::from(r.wrapping_add(q.target_size[1])),
            range: q.range,
            range2: (i64::from(q.range) * i64::from(q.range)) >> 22,
        };
        let ignore = self.ignore.clone();
        let mut s = Search {
            ctx: Ctx {
                mesh,
                ignore: &ignore,
                d,
            },
            goal,
            bounds,
            open: Vec::new(),
            arena: Vec::new(),
            polys: Vec::new(),
            goal_node: NONE,
            best: 0,
            token: 0,
        };
        s.run(q.from)
    }
}

impl Search<'_> {
    /// The search itself, after the prologue: the start, the main loop, the path.
    fn run(&mut self, start: Point) -> Result<ShortPath, GenieError> {
        let res4410 = self.start(start);
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
        let mut node = self.arena[self.goal_node as usize];
        let mut is_root = node.parent == NONE;
        while !is_root {
            let parent = self.arena[node.parent as usize];
            match node.kind {
                Kind::Corner {
                    poly, dir, lo, hi, ..
                } if poly != NONE => {
                    pts.push(node.at);
                    let v = &self.polys[poly as usize].vertices;
                    let n = v.len() as u32;
                    if dir == 1 {
                        let mut k = lo;
                        while k != hi {
                            pts.push(v[k as usize]);
                            k = (k + 1) % n;
                        }
                    } else {
                        let mut k = hi;
                        while k != lo {
                            k = (k + n - 1) % n;
                            pts.push(v[k as usize]);
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
                if !sight.sight_blocked(pts[j], pts[i]) {
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
    fn expand(&mut self, n: Node, start: Point) -> Result<Option<ShortPath>, GenieError> {
        let p = n.at;
        let d = self.ctx.d;
        match n.kind {
            Kind::Waypoint => {
                let _ = self.pop_test(&n);
            }
            Kind::Start => {
                if self.pop_test(&n) {
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
                    .raycast(p, t, &mut frac, &mut hit, Callback::Box, &mut token);
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
                        self.settle(&mut h, &mut hit);
                        let g = n.g.wrapping_add(float_distance(p, h));
                        self.side_node(g, n.idx, hit.shape, hit.side, h);
                    }
                }
            }
            Kind::Side { shape, side } => {
                if self.pop_test(&n) {
                    return Ok(None);
                }
                let _ = self.side_expand(n, shape, side)?;
            }
            Kind::Corner {
                token, prev, next, ..
            } => {
                if self.pop_test(&n) {
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
    fn touch_from(
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
            if self.ctx.holder(qp).is_none() {
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
        let r = self.ctx.mesh.shapes[hit.shape as usize];
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
fn range_point(range: i32, p: Point, goal: Point) -> Result<Point, GenieError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A search on an empty mesh toward a point target at `at` within `range` (fixed point), its
    /// start node at the origin with cost 0.
    fn search_toward(mesh: &Mesh, at: Point, range: i32) -> Search<'_> {
        let goal = Goal {
            at,
            r: 0,
            rx: 0,
            ry: 0,
            rrx: 0,
            rry: 0,
            range,
            range2: (i64::from(range) * i64::from(range)) >> 22,
        };
        let mut s = Search {
            ctx: Ctx {
                mesh,
                ignore: &[],
                d: 0,
            },
            goal,
            bounds: [0, 0, 64 * ONE, 64 * ONE],
            open: Vec::new(),
            arena: Vec::new(),
            polys: Vec::new(),
            goal_node: NONE,
            best: 0,
            token: 0,
        };
        let _ = s.add(0, NONE, [0, 0], Kind::Start);
        s
    }

    /// `FUN_140fa3860`: a node within range becomes the goal while it is cheaper than the goal so
    /// far; any other node becomes the best when it lies nearer than the best's estimate, and a
    /// node no nearer adds nothing.
    #[test]
    fn a_goal_candidate_in_range_and_cheaper_becomes_the_goal_else_maybe_the_best() {
        let mesh = Mesh::new(64);
        let mut s = search_toward(&mesh, [10 * ONE, 10 * ONE], 2 * ONE);
        assert!(s.goal_test(5 * ONE, 0, 0, 0, 0, 0, [11 * ONE, 10 * ONE]));
        assert_eq!(s.goal_node, 1);
        // in range but dearer: the best instead, as it lies nearer than the start's estimate
        assert!(!s.goal_test(6 * ONE, 0, 0, 0, 0, 0, [11 * ONE, 10 * ONE]));
        assert_eq!((s.goal_node, s.best), (1, 2));
        // out of range and farther than the best's estimate (one tile): nothing
        assert!(!s.goal_test(ONE, 0, 0, 0, 0, 0, [30 * ONE, 30 * ONE]));
        assert_eq!((s.arena.len(), s.best), (3, 2));
    }

    /// `FUN_140fa8210`: the goal's foot on an axis edge that spans it, reached at the edge's
    /// start cost plus the walk along the edge, is a goal candidate; an edge that does not span
    /// the goal gives none.
    #[test]
    fn the_goal_s_foot_on_a_spanning_edge_is_a_goal_candidate() {
        let mesh = Mesh::new(64);
        let mut s = search_toward(&mesh, [10 * ONE, 10 * ONE], 2 * ONE);
        s.foot_test(
            0,
            [9 * ONE, 12 * ONE],
            [9 * ONE, 20 * ONE],
            0,
            0,
            0,
            0,
            ONE,
            0,
        );
        assert_eq!(s.arena.len(), 1);
        s.foot_test(
            0,
            [15 * ONE, 9 * ONE],
            [5 * ONE, 9 * ONE],
            1,
            0,
            0,
            0,
            ONE,
            0,
        );
        let goal = s.arena[s.goal_node as usize];
        assert_eq!((goal.at, goal.g), ([10 * ONE, 9 * ONE], 6 * ONE));
    }

    /// `FUN_140fa8350`: a junction moved `d` into the free space beside each side, the flag
    /// turning it along the side; side 0 and any side past 3 take the same path.
    #[test]
    fn a_contact_point_moves_off_its_side() {
        let p = [100, 200];
        let moved: Vec<Point> = (0..5)
            .flat_map(|side| [0, 1].map(|flag| contact_point(p, side, 3, flag)))
            .collect();
        assert_eq!(
            moved,
            [
                [97, 203],
                [103, 203],
                [97, 197],
                [97, 203],
                [103, 197],
                [97, 197],
                [103, 203],
                [103, 197],
                [97, 203],
                [103, 203],
            ]
        );
    }

    /// `FUN_140fa39f0`: a point goes onto a side of the rectangle shrunk by `d` (the side's
    /// coordinate set, the other held between the shrunk corners); a side past 3 leaves it.
    #[test]
    fn a_point_snaps_onto_a_side_within_its_corners() {
        let r = [0, 0, 100, 50];
        let snap = |p: Point, side| {
            let mut q = p;
            snap_to_side(&mut q, r, side, 5);
            q
        };
        assert_eq!(snap([-20, 30], 0), [5, 5]);
        assert_eq!(snap([200, 30], 0), [95, 5]);
        assert_eq!(snap([40, 30], 0), [40, 5]);
        assert_eq!(snap([40, -9], 1), [95, 5]);
        assert_eq!(snap([40, 99], 1), [95, 45]);
        assert_eq!(snap([40, 20], 2), [40, 45]);
        assert_eq!(snap([40, 20], 3), [5, 20]);
        assert_eq!(snap([40, 20], 4), [40, 20]);
    }

    #[test]
    fn the_integer_square_root_is_exact_below_and_truncates_between_squares() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(1), 1);
        assert_eq!(isqrt(15), 3);
        assert_eq!(isqrt(16), 4);
        assert_eq!(isqrt(u64::from(u32::MAX) * u64::from(u32::MAX)), u32::MAX);
        // a tile squared in fixed point (2^44) gives a tile
        assert_eq!(isqrt(1 << 44), ONE as u32);
    }

    #[test]
    fn a_ray_crosses_the_side_it_faces_and_reports_how_far() {
        // a ray from (0, 1) to (4, 1) against the box (2, 0)-(3, 2): its left side at x = 2
        let r = [2 * ONE, 0, 3 * ONE, 2 * ONE];
        let mut t = T_START;
        let mut side = 9;
        let k = ray_rect([0, ONE], [4 * ONE, ONE], r, &mut t, &mut side);
        assert_eq!((k, side, t), (1, 3, ONE / 2));
        // a ray going the other way meets the right side
        let mut t = T_START;
        let k = ray_rect([4 * ONE, ONE], [0, ONE], r, &mut t, &mut side);
        assert_eq!((k, side, t), (1, 1, ONE / 4));
        // a ray beside the box meets nothing
        let mut t = T_START;
        assert_eq!(
            ray_rect([0, 3 * ONE], [4 * ONE, 3 * ONE], r, &mut t, &mut side),
            0
        );
    }

    /// `FUN_140fa16d0`: a segment through the box keeps the part inside, each end moved to the
    /// edge it crossed (`(dy * (edge - x) >> 22 << 22) / dx`, truncated); a segment beside the
    /// box is none, one inside it stays; the box's corners may come in any order.
    #[test]
    fn a_segment_clipped_to_the_box_keeps_its_part_inside() {
        let b = [0, 0, 2 * ONE, 2 * ONE];
        assert_eq!(
            clip([-ONE, ONE], [3 * ONE, ONE], b),
            Some(([0, ONE], [2 * ONE, ONE]))
        );
        // first the left edge, then the bottom, then the right edge for the far end
        assert_eq!(
            clip([-ONE, -ONE], [3 * ONE, 2 * ONE], b),
            Some(([1_398_101, 0], [2 * ONE, 5_242_880]))
        );
        assert_eq!(clip([3 * ONE, 0], [4 * ONE, ONE], b), None);
        let inside = ([ONE / 2, ONE], [ONE, ONE / 2]);
        assert_eq!(clip(inside.0, inside.1, b), Some(inside));
        assert_eq!(
            clip([-ONE, ONE], [3 * ONE, ONE], [2 * ONE, 2 * ONE, 0, 0]),
            Some(([0, ONE], [2 * ONE, ONE]))
        );
    }

    /// `FUN_140faa170` (through `FUN_140f9d3e0`): blocked when the start, out of range, lies in
    /// a shape with fewer than two escape candidates; a free start or one in range is not. The
    /// terrain's temporary shapes leave the mesh after it.
    #[test]
    fn a_start_deep_in_blocked_terrain_is_blocked_unless_in_range() {
        let blocked = |_: u32, _: u32| 1;
        let free = |_: u32, _: u32| 0;
        let mut q = Query {
            from: [5 * ONE + ONE / 2, 5 * ONE + ONE / 2],
            to: [8 * ONE + ONE / 2, 5 * ONE + ONE / 2],
            radius: 0.25,
            target_size: [0.0, 0.0],
            range: 0.0,
            blocked: 1,
        };
        let mut mesh = Mesh::new(16);
        let mut pather = Pather::default();
        // each row of blocked tiles is a shape half a tile wider all round: the start sits 0.75
        // tiles inside row 5's, beyond twice the radius from its sides
        assert!(pather.start_blocked(&mut mesh, &blocked, &q));
        let n = mesh.shapes.len() as u32;
        assert!(0 < n && (0..n).all(|id| !mesh.is_shape(id)));
        assert!(!pather.start_blocked(&mut mesh, &free, &q));
        q.range = 3.0;
        assert!(!pather.start_blocked(&mut mesh, &blocked, &q));
    }

    /// `FUN_140f9e540`'s entry tests as compiled: the target's gap clamps each axis by `MAXSS`
    /// (`FUN_14118faf0`: an unordered one is 0) and is in range by `COMISS`/`JC` (0x140f9e5f5);
    /// each end lies off the map by `COMISS`/`JA` below 0 and `JNC` at the size (0x140f9e67c..
    /// 0x140f9e6da): an unordered coordinate is on it.
    #[test]
    fn the_entry_s_float_tests_take_the_game_s_nan_paths() {
        let call = |from: [f32; 2], target: Option<Target>| Call {
            from,
            to: [7.0, 5.0],
            radius: 0.2,
            range: 0.0,
            target,
            listed: Vec::new(),
            map: [16, 16],
            mask: 6,
        };
        let t = Target {
            id: 9,
            position: [5.0, 5.0],
            half_size: [0.5, 0.5],
        };
        assert_eq!(call([f32::NAN, 5.0], Some(t)).entry(), Entry::InRange);
        assert!(matches!(
            call([f32::NAN, 5.0], None).entry(),
            Entry::Search { .. }
        ));
        assert_eq!(call([-0.1, 5.0], None).entry(), Entry::Invalid);
        assert_eq!(call([16.0, 5.0], None).entry(), Entry::Invalid);
        assert_eq!(call([5.0, 16.0], None).entry(), Entry::Invalid);
        assert!(matches!(
            call([0.0, 15.9], None).entry(),
            Entry::Search { .. }
        ));
    }

    /// A search on a 32-tile mesh holding `walls` (tiles, `[x0, y0, x1, y1]`) from `from` to `to`
    /// (tiles) with a radius of a quarter tile; the path's points in tiles. With `ignored`, every
    /// wall is ignored.
    fn around(
        walls: &[[f32; 4]],
        from: [f32; 2],
        to: [f32; 2],
        ignored: bool,
    ) -> (u8, Vec<[f32; 2]>) {
        let fixed = |v: f32| (v * ONE as f32) as i32;
        let mut mesh = Mesh::new(32);
        let ids: Vec<u32> = walls
            .iter()
            .map(|w| mesh.add([fixed(w[0]), fixed(w[1]), fixed(w[2]), fixed(w[3])]))
            .collect();
        let q = Query {
            from: [fixed(from[0]), fixed(from[1])],
            to: [fixed(to[0]), fixed(to[1])],
            radius: 0.25,
            target_size: [0.0, 0.0],
            range: 0.0,
            blocked: 0,
        };
        let ignore = if ignored { ids } else { Vec::new() };
        let found = Pather::default()
            .search(&mut mesh, &|_, _| 0, &q, &ignore)
            .unwrap();
        let tiles = |v: i32| v as f32 / ONE as f32;
        let points = found
            .points
            .iter()
            .map(|p| [tiles(p[0]), tiles(p[1])])
            .collect();
        (found.result, points)
    }

    /// A shape keeps the unit's centre `d` (half a tile less the radius) inside its rectangle: the
    /// search walks round a wall (10.25..10.75 by 4.25..11.75 for the centre) at its nearer end
    /// (3.25 tiles down against 4.25 up), over its two corners; ignored, the wall leaves the path
    /// direct (the start, the start node's own point, the goal).
    #[test]
    fn a_path_bends_round_a_wall_at_its_nearer_end() {
        let wall = [[10.0, 4.0, 11.0, 12.0]];
        assert_eq!(
            around(&wall, [6.5, 8.5], [14.5, 8.5], false),
            (
                1,
                vec![[6.5, 8.5], [10.25, 11.75], [10.75, 11.75], [14.5, 8.5]]
            )
        );
        assert_eq!(
            around(&wall, [6.5, 8.5], [14.5, 8.5], true),
            (1, vec![[6.5, 8.5], [6.5, 8.5], [14.5, 8.5]])
        );
    }

    /// A start inside a U open away from the goal: the path leaves by the open side round the
    /// nearer arm's end (the lower, 2.75 tiles down against 3.75 up) and runs along it to the
    /// corner it shares with the U's back.
    #[test]
    fn a_path_leaves_a_u_by_its_open_side() {
        let u = [
            [8.0, 4.0, 9.0, 12.0],
            [4.0, 4.0, 9.0, 5.0],
            [4.0, 11.0, 9.0, 12.0],
        ];
        assert_eq!(
            around(&u, [6.5, 8.5], [14.5, 8.5], false),
            (
                1,
                vec![
                    [6.5, 8.5],
                    [4.25, 11.25],
                    [4.25, 11.75],
                    [8.75, 11.75],
                    [14.5, 8.5]
                ]
            )
        );
    }

    /// A goal shut inside a box: the path is partial (result 2) and ends at the point nearest the
    /// goal the unit's centre reaches, on the box's outer side.
    #[test]
    fn a_goal_shut_in_a_box_gives_a_partial_path_to_its_nearest_point() {
        let walls = [
            [10.0, 6.0, 15.0, 7.0],
            [10.0, 10.0, 15.0, 11.0],
            [10.0, 6.0, 11.0, 11.0],
            [14.0, 6.0, 15.0, 11.0],
        ];
        assert_eq!(
            around(&walls, [6.5, 8.5], [12.5, 8.5], false),
            (2, vec![[6.5, 8.5], [10.25, 8.5]])
        );
        assert_eq!(around(&walls, [6.5, 8.5], [12.5, 8.5], true).0, 1);
    }

    /// Two staggered walls: the path reaches the goal over one corner of the second wall (a
    /// shortest path bends only at corners); the first wall's lower end lies off it.
    #[test]
    fn a_path_past_staggered_walls_bends_only_at_their_corners() {
        let walls = [[10.0, 4.0, 11.0, 9.0], [12.0, 8.0, 13.0, 14.0]];
        let (result, points) = around(&walls, [6.5, 6.5], [16.5, 12.5], false);
        assert_eq!(result, 1);
        assert_eq!(points.first(), Some(&[6.5, 6.5]));
        assert_eq!(points.last(), Some(&[16.5, 12.5]));
        let corners = [[12.25, 8.25], [12.75, 8.25], [12.25, 13.75], [12.75, 13.75]];
        let between = &points[1..points.len() - 1];
        assert!(2 < points.len(), "{points:?}");
        assert!(between.iter().all(|p| corners.contains(p)), "{points:?}");
    }

    /// A query from (5, 5) to (7, 5) with a radius of a quarter tile and range `range`.
    fn range_query(range: f32) -> Query {
        Query {
            from: [5 * ONE, 5 * ONE],
            to: [7 * ONE, 5 * ONE],
            radius: 0.25,
            target_size: [0.0, 0.0],
            range,
            blocked: 0,
        }
    }

    #[test]
    fn the_search_truncates_its_radius_and_range_to_fixed_point() {
        // FUN_140f9d300: (int)(longlong)(range * 2^22). The goal lies 1.75 tiles beyond the
        // radius: a range of 1.75 holds it at once (result 0); the float just below truncates to
        // 0x6fffff and searches
        let words = |_: u32, _: u32| 0;
        let below = f32::from_bits(1.75f32.to_bits() - 1);
        let mut mesh = Mesh::new(16);
        let mut pather = Pather::default();
        let found = pather.search(&mut mesh, &words, &range_query(1.75), &[]);
        assert_eq!(found.map(|f| f.result), Ok(0));
        let found = pather.search(&mut mesh, &words, &range_query(below), &[]);
        assert_eq!(found.map(|f| f.result), Ok(1));
    }
}
