//! The spawn point search (`FUN_1411d12d0`): the building's sides in segments, each spot's
//! candidates (`FUN_1411c5b60`) and the corners' wall checks.

use crate::production::spawn::{GAP, NONE, SIDE_SHARE, SMALL, Site, Tested};
use crate::unit_ai::GenieError;

/// One spot (`FUN_1411c5b60`'s argument): its candidates `i` in `0..=count` lie at
/// `(x + dir.x * step.x * i, y + dir.y * step.y * i)`.
pub(super) struct Spot {
    pub(super) at: [f32; 2],
    pub(super) step: [f32; 2],
    pub(super) dir: [i32; 2],
    pub(super) count: i32,
}

/// `FUN_1411c5b60`: the first free candidate; a candidate blocked by an object that stays ends
/// the spot.
pub(super) fn try_spot(
    spot: &Spot,
    test: &mut dyn FnMut([f32; 2]) -> Result<Tested, GenieError>,
) -> Result<Option<[f32; 2]>, GenieError> {
    for i in 0..=spot.count {
        let x = spot.dir[0] as f32 * spot.step[0] * i as f32 + spot.at[0];
        let y = spot.dir[1] as f32 * spot.step[1] * i as f32 + spot.at[1];
        match test([x, y])? {
            Tested::Free => return Ok(Some([x, y])),
            Tested::Blocked { stays: true } => return Ok(None),
            Tested::Blocked { stays: false } => {}
        }
    }
    Ok(None)
}

/// `FUN_1411d12d0`: the spawn point beside `site` for a unit of radii `unit`. The segments of
/// the sides facing the gather point come first (the top side's left part, the left side's top
/// part, the top's right part, the left's lower part; then the right side down; then the bottom
/// side), each only where no wall closes its corner; a second pass tries every open side. Without
/// a gather point there is one pass. The search's later rings (cases 11 to 18: the spots on the
/// building's edge and the rings around it, `FUN_1411d0b20`) and its first test under
/// `DAT_143f01833` (`FUN_1411c52f0`, probed 0) are not modelled.
pub fn spawn_point(
    site: &Site,
    unit: [f32; 2],
    test: &mut dyn FnMut([f32; 2]) -> Result<Tested, GenieError>,
) -> Result<[f32; 2], GenieError> {
    let [mut sx, mut sy] = unit;
    if sx < SMALL && sy < SMALL {
        sx = sx + sx + GAP;
        sy = sy + sy + GAP;
    }
    let [bx, by] = site.radius;
    let [x, y, _] = site.position;
    let left = x - (bx + sx);
    let right = x + bx + sx;
    let top = y + by + sy;
    let bottom = y - (by + sy);
    let [left_top, right_top, right_bottom, left_bottom] = site.walls;
    let [gx, gy] = site.gather.unwrap_or([NONE, NONE]);
    let dx = (x - gx).abs();
    let dy = (y - gy).abs();
    let (no_x, no_y) = (gx == NONE, gy == NONE);
    // a spot of a horizontal segment tries its point and the next one up (or down); a vertical
    // segment's, the next one out
    let up = |at| Spot {
        at,
        step: [0.0, sy],
        dir: [0, 1],
        count: 1,
    };
    let down = |at| Spot {
        at,
        step: [0.0, sy],
        dir: [0, -1],
        count: 1,
    };
    let out_left = |at| Spot {
        at,
        step: [sx, 0.0],
        dir: [-1, 0],
        count: 1,
    };
    let out_right = |at| Spot {
        at,
        step: [sx, 0.0],
        dir: [1, 0],
        count: 1,
    };
    for pass in 1..=2 {
        let second = pass == 2;
        let mut tries: Vec<Spot> = Vec::new();
        let mut run = |spots: &mut Vec<Spot>| -> Result<Option<[f32; 2]>, GenieError> {
            for s in spots.drain(..) {
                if let Some(p) = try_spot(&s, test)? {
                    return Ok(Some(p));
                }
            }
            Ok(None)
        };
        if !left_top {
            // (1) the top side's left part
            if no_y || second || ((y < gy && dx <= dy) && (no_x || gx < x)) {
                let mut cx = left;
                while cx < bx * SIDE_SHARE + bx + left {
                    tries.push(up([cx, top]));
                    cx += sx;
                }
                if let Some(p) = run(&mut tries)? {
                    return Ok(p);
                }
            }
            // (2) the left side's top part
            if no_x || second || ((gx < x && dy < dx) && (no_y || y < gy)) {
                let mut cy = top;
                while top - (by * SIDE_SHARE + by) < cy {
                    tries.push(out_left([left, cy]));
                    cy -= sy;
                }
                if let Some(p) = run(&mut tries)? {
                    return Ok(p);
                }
            }
            // (3) the top side's right part
            if !right_top && (no_y || second || (y < gy && dx <= dy)) {
                let mut cx = bx * SIDE_SHARE + bx + left;
                while cx < bx + bx + left + sx + sx {
                    tries.push(up([cx, top]));
                    cx += sx;
                }
                if let Some(p) = run(&mut tries)? {
                    return Ok(p);
                }
            }
            // (4) the left side's lower part
            if !left_bottom && (no_x || second || (gx < x && dy < dx)) {
                let mut cy = top - (by * SIDE_SHARE + by);
                while top - (by + by + sy + sy) < cy {
                    tries.push(out_left([left, cy]));
                    cy -= sy;
                }
                if let Some(p) = run(&mut tries)? {
                    return Ok(p);
                }
            }
        }
        if !right_top && (no_x || second || (x < gx && dy < dx)) {
            // (5) the right side's top half
            if no_y || second || y < gy {
                let mut cy = top;
                while top - by < cy {
                    tries.push(out_right([right, cy]));
                    cy -= sy;
                }
                if let Some(p) = run(&mut tries)? {
                    return Ok(p);
                }
            }
            // (6) its lower half
            if !right_bottom {
                let mut cy = top - by;
                while top - (by + by + sy + sy) < cy {
                    tries.push(out_right([right, cy]));
                    cy -= sy;
                }
                if let Some(p) = run(&mut tries)? {
                    return Ok(p);
                }
            }
        }
        let bottom_closed = left_bottom || (!no_y && !second && (y <= gy || dy < dx));
        if !bottom_closed {
            // (7) the bottom side's left half
            if no_x || second || gx < x {
                let mut cx = left;
                while cx < left + bx {
                    tries.push(down([cx, bottom]));
                    cx += sx;
                }
                if let Some(p) = run(&mut tries)? {
                    return Ok(p);
                }
            }
            // (8) its right half
            if !right_bottom {
                let mut cx = left + bx;
                while cx < bx + bx + sx + sx + left {
                    tries.push(down([cx, bottom]));
                    cx += sx;
                }
                if let Some(p) = run(&mut tries)? {
                    return Ok(p);
                }
            }
        }
        // (9), (10): with the top right and bottom left corners walled, the second pass tries the
        // bottom side from the right and the right side from the bottom
        if !right_bottom && right_top && left_bottom && second {
            let mut cx = right;
            while right - bx < cx {
                tries.push(down([cx, bottom]));
                cx -= sx;
            }
            let mut cy = bottom;
            while cy < bottom + by {
                tries.push(out_right([right, cy]));
                cy += sy;
            }
            if let Some(p) = run(&mut tries)? {
                return Ok(p);
            }
        }
        if no_x {
            break;
        }
    }
    Err(GenieError::unsupported(
        "a spawn point beyond the building's sides (the search's later rings)",
    ))
}

/// The corners' tile pairs (`FUN_1411d12d0`'s wall checks, left top, right top, right bottom,
/// left bottom) for a unit of radii `unit` beside `site`.
pub(super) fn corner_tiles(
    position: [f32; 3],
    radius: [f32; 2],
    unit: [f32; 2],
) -> [[(i32, i32); 2]; 4] {
    let [mut sx, mut sy] = unit;
    if sx < SMALL && sy < SMALL {
        sx = sx + sx + GAP;
        sy = sy + sy + GAP;
    }
    let [x, y, _] = position;
    let [bx, by] = radius;
    let left = (x - (bx + sx)) as i32;
    let right = (x + bx + sx) as i32;
    let top = (y + by + sy) as i32;
    let bottom = (y - (by + sy)) as i32;
    [
        [(left, top - 1), (left + 1, top)],
        [(right - 1, top), (right, top - 1)],
        [(right, bottom + 1), (right - 1, bottom)],
        [(left + 1, bottom), (left, bottom + 1)],
    ]
}
