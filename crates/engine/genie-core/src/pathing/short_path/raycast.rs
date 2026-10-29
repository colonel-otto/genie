//! The searches' walks over the mesh's tiles ([`Ctx`]): a point's holder, the raycast, the
//! axis sweeps and the sight test.

use crate::pathing::mesh::{ONE, Rect};
use crate::pathing::short_path::{
    Callback, Ctx, Hit, Point, Sweep, T_START, inside, ray_rect, strictly_inside, tile_of,
};

/// One tile of a raycast ([`Ctx::ray_tile`]'s arguments).
#[derive(Debug)]
pub(super) struct RayTile<'a> {
    /// The callback over the tile's shapes.
    pub(super) callback: Callback,
    /// The tile's shapes.
    pub(super) list: &'a [u32],
    /// The ray's start.
    pub(super) start: Point,
    /// The ray's end.
    pub(super) end: Point,
    /// The ray's fraction so far, which a hit lowers.
    pub(super) fraction: &'a mut i32,
    /// The hit it leaves.
    pub(super) hit: &'a mut Hit,
    /// The token a hit ignores, as the callback leaves it.
    pub(super) token: &'a mut u32,
}

impl Ctx<'_> {
    pub(super) fn ignored(&self, id: u32) -> bool {
        self.ignore.get(id as usize).copied().unwrap_or(false)
    }

    pub(super) fn tile(&self, tx: i32, ty: i32) -> &[u32] {
        let i = (ty.wrapping_mul(self.mesh.width as i32).wrapping_add(tx)) as u32 as usize;
        &self.mesh.tiles[i]
    }

    /// `FUN_140fa3dc0`: the first shape of `p`'s tile, not ignored, strictly holding `p` (shrunk).
    pub(super) fn holder(&self, p: Point) -> Option<u32> {
        self.tile(tile_of(p[0]), tile_of(p[1]))
            .iter()
            .copied()
            .find(|&id| {
                !self.ignored(id) && strictly_inside(self.mesh.shapes[id as usize], p, self.d)
            })
    }

    /// `FUN_140fa2630`: the first shape of `p`'s tile other than `excl`, not ignored, holding `p`
    /// with its edges.
    pub(super) fn holder_but(&self, p: Point, excl: u32, d: i32) -> Option<u32> {
        self.tile(tile_of(p[0]), tile_of(p[1]))
            .iter()
            .copied()
            .find(|&id| {
                id != excl && !self.ignored(id) && inside(self.mesh.shapes[id as usize], p, d)
            })
    }

    /// One tile of the raycast: the callback over the tile's shapes.
    pub(super) fn ray_tile(&self, tile: RayTile<'_>) -> i32 {
        let RayTile {
            callback: cb,
            list,
            start: a,
            end: b,
            fraction: t,
            hit,
            token,
        } = tile;
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
    pub(super) fn raycast(
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
            let r = self.ray_tile(RayTile {
                callback: cb,
                list,
                start: a,
                end: b,
                fraction: t,
                hit,
                token,
            });
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
    pub(super) fn axis_hit(
        from: Point,
        end: Point,
        r: Rect,
        d: i32,
        dir: u8,
    ) -> Option<(Point, u8)> {
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
    pub(super) fn sweep(&self, from: Point, to: Point, dir: u8) -> Sweep {
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
    pub(super) fn sight_blocked(&self, a: Point, b: Point) -> bool {
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
