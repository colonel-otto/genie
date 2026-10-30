//! The fixed-point geometry: a shrunk rectangle's corners and sides, a contact's point, the
//! segment and ray hits.

use crate::pathing::mesh::Rect;
use crate::pathing::short_path::FixedPoint;

/// `FUN_140fa8460`: corner `k` of `r` shrunk by `d`.
pub(super) const fn shrunk_corner(r: Rect, k: u32, d: i32) -> FixedPoint {
    match k {
        0 => [r[0] + d, r[1] + d],
        1 => [r[2] - d, r[1] + d],
        2 => [r[2] - d, r[3] - d],
        3 => [r[0] + d, r[3] - d],
        _ => [r[0], r[1]],
    }
}

/// `FUN_140fa8350`: a contact's junction moved by `d` into the free space beside its side.
pub(super) const fn contact_point([x, y]: FixedPoint, side: u32, d: i32, flag: u8) -> FixedPoint {
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
pub(super) fn snap_to_side(p: &mut FixedPoint, r: Rect, side: u8, d: i32) {
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
pub(super) fn segment_hit(
    a: FixedPoint,
    b: FixedPoint,
    c: FixedPoint,
    e: FixedPoint,
    t: &mut i32,
) -> i32 {
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
pub(super) fn ray_rect(a: FixedPoint, b: FixedPoint, r: Rect, t: &mut i32, side: &mut u8) -> i32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let mut found = 0;
    // the side across x it faces: the right (1) for a ray going left, the left (3) for one going
    // right
    let (k, s) = match dx {
        ..0 => (ray_right_side(a, b, r, t), 1),
        0 => (0, 1),
        _ => (ray_left_side(a, b, r, t), 3),
    };
    if k != 0 {
        *side = s;
        found = k;
    }
    // then the bottom (0) for a ray going up in y, the top (2) for one going down
    let (k, s) = match dy {
        1.. => (ray_bottom_side(a, b, r, t), 0),
        0 => return found,
        _ => (ray_top_side(a, b, r, t), 2),
    };
    if k == 0 {
        return found;
    }
    *side = s;
    k
}

/// [`ray_rect`]'s ratio of a side's offset `num` along the ray's `den` (`num << 22 / den`).
pub(super) fn ray_ratio(num: i32, den: i32) -> i32 {
    ((i64::from(num) << 22) / i64::from(den)) as i32
}

/// [`ray_rect`]'s right side (x1) for the ray `a`–`b` going left: a ray along x (no y step)
/// crosses it within its open span from at or right of it to left of it, nearer than `t` (1, its
/// ratio into `t`); any other ray as [`segment_hit`]; 0 for none.
pub(super) fn ray_right_side(
    a: FixedPoint,
    b: FixedPoint,
    [_, ry0, rx1, ry1]: Rect,
    t: &mut i32,
) -> i32 {
    if b[1] - a[1] != 0 {
        return segment_hit(a, b, [rx1, ry0], [rx1, ry1], t);
    }
    if a[1] <= ry0 || ry1 <= a[1] {
        return 0;
    }
    if rx1 <= b[0] || a[0] < rx1 {
        return 0;
    }
    let u = ray_ratio(rx1 - a[0], b[0] - a[0]);
    if *t <= u {
        return 0;
    }
    *t = u;
    1
}

/// [`ray_rect`]'s left side (x0) for the ray `a`–`b` going right, as [`ray_right_side`]: a ray
/// along x crosses it from at or left of it to right of it.
pub(super) fn ray_left_side(
    a: FixedPoint,
    b: FixedPoint,
    [rx0, ry0, _, ry1]: Rect,
    t: &mut i32,
) -> i32 {
    if b[1] - a[1] != 0 {
        return segment_hit(a, b, [rx0, ry0], [rx0, ry1], t);
    }
    if a[1] <= ry0 || ry1 <= a[1] {
        return 0;
    }
    if b[0] <= rx0 || rx0 < a[0] {
        return 0;
    }
    let u = ray_ratio(rx0 - a[0], b[0] - a[0]);
    if *t <= u {
        return 0;
    }
    *t = u;
    1
}

/// [`ray_rect`]'s bottom side (y0) for the ray `a`–`b` going up in y: a ray along y (no x step)
/// crosses it within its open span from at or below it to above it, nearer than `t` (1, its
/// ratio into `t`); any other ray as [`segment_hit`]; 0 for none.
pub(super) fn ray_bottom_side(
    a: FixedPoint,
    b: FixedPoint,
    [rx0, ry0, rx1, _]: Rect,
    t: &mut i32,
) -> i32 {
    if b[0] - a[0] != 0 {
        return segment_hit(a, b, [rx0, ry0], [rx1, ry0], t);
    }
    if !(rx0 < a[0] && a[0] < rx1) {
        return 0;
    }
    if b[1] <= ry0 || ry0 < a[1] {
        return 0;
    }
    let u = ray_ratio(ry0 - a[1], b[1] - a[1]);
    if *t <= u {
        return 0;
    }
    *t = u;
    1
}

/// [`ray_rect`]'s top side (y1) for the ray `a`–`b` going down in y, as [`ray_bottom_side`]: a
/// ray along y crosses it from at or above it to below it.
pub(super) fn ray_top_side(
    a: FixedPoint,
    b: FixedPoint,
    [rx0, _, rx1, ry1]: Rect,
    t: &mut i32,
) -> i32 {
    if b[0] - a[0] != 0 {
        return segment_hit(a, b, [rx0, ry1], [rx1, ry1], t);
    }
    if !(rx0 < a[0] && a[0] < rx1) {
        return 0;
    }
    if ry1 <= b[1] || a[1] < ry1 {
        return 0;
    }
    let u = ray_ratio(ry1 - a[1], b[1] - a[1]);
    if *t <= u {
        return 0;
    }
    *t = u;
    1
}
