//! The walked level's flags, its reset, a looping path's next waypoint and the waypoints'
//! points and ground lengths.

use crate::group::motion::NEAR;
use crate::math::fmath;
use crate::math::fmath::Vec3;
use crate::movement::PathData;

/// `FUN_140f9b4d0(level, bit, on)` on the walked level: its flag `bit` set or cleared; bit 2 is
/// then set on every parent too.
pub(super) fn set_flag(paths: &mut [PathData], bit: u32, on: bool) {
    let Some((level, parents)) = paths.split_first_mut() else {
        return;
    };
    if on {
        level.flags |= 1 << bit;
    } else {
        level.flags &= !(1 << bit);
    }
    if bit == 2 {
        for p in parents {
            p.flags |= 4;
        }
    }
}

/// `FUN_140f98e80` on the walked level: it and its parents go, the level holds the defaults
/// (its plans back to 10, flags clear; its repath time stays).
pub(super) fn reset_level(paths: &mut Vec<PathData>) {
    let repath_time = paths.first().map_or(0.0, |l| l.repath_time);
    paths.truncate(1);
    let level = PathData {
        repath_time,
        ..PathData::without_path()
    };
    if paths.is_empty() {
        paths.push(level);
    } else {
        paths[0] = level;
    }
}

/// `FUN_140f98bc0` with 1 on a looping path's level: the waypoint after `index`; on a path that
/// goes back and forth (flags bit 1) the walk turns at its ends (bit 2 the way back).
pub(super) const fn looping_next(level: &mut PathData, index: i32) -> i32 {
    let n = level.points.len() as i32;
    if level.path_flags & 1 == 0 {
        return -1;
    }
    if level.path_flags & 2 == 0 {
        return if index + 1 < n { index + 1 } else { 0 };
    }
    if level.path_flags & 4 == 0 {
        let next = index - 1;
        if next < 0 {
            level.path_flags |= 4;
            return 1;
        }
        next
    } else {
        let next = index + 1;
        if n <= next {
            level.path_flags &= !4;
            return n - 2;
        }
        next
    }
}

/// The waypoint at `index` (`FUN_140f9bcd0`), the game's static zero point outside the path.
pub(super) fn point(level: &PathData, index: i32) -> Vec3 {
    usize::try_from(index)
        .ok()
        .and_then(|i| level.points.get(i))
        .copied()
        .unwrap_or([0.0; 3])
}

/// The length on the ground from `a` to `b` (`FUN_14087e560`, the height zeroed).
pub(super) fn ground(a: Vec3, b: Vec3) -> f32 {
    let mut d = fmath::sub(a, b);
    d[1] = 0.0;
    fmath::sqrt(d[0] * d[0] + 0.0 + d[2] * d[2])
}

/// `FUN_141046130` for a walk from `at` toward `waypoint` that would end at `next`: 0 short of
/// it (`COMISS` then `JBE`: a NaN step or distance is short), 2 past it (or a step under 0.1),
/// 1 on it.
pub(in crate::group) fn passes_waypoint(waypoint: Vec3, at: Vec3, next: Vec3) -> u8 {
    let mut a = fmath::sub(waypoint, at);
    let mut b = fmath::sub(waypoint, next);
    let c = fmath::sub(next, at);
    a[1] = 0.0;
    b[1] = 0.0;
    let to_waypoint = fmath::sqrt(a[0] * a[0] + 0.0 + a[2] * a[2]);
    let beyond = fmath::sqrt(b[0] * b[0] + b[1] * b[1] + b[2] * b[2]);
    let step = fmath::sqrt(c[0] * c[0] + c[1] * c[1] + c[2] * c[2]);
    if fmath::at_most_or_unordered(step, to_waypoint) {
        0
    } else if to_waypoint < beyond || step < NEAR {
        2
    } else {
        1
    }
}

/// `FUN_1411570c0`'s ends held inside the map: below 0 at 0, from the map's size on at the size
/// less 0.1, per axis.
pub(in crate::group) fn hold(p: Vec3, map: [i32; 2]) -> [f32; 2] {
    let axis = |v: f32, size: i32| {
        if v < 0.0 {
            0.0
        } else if size as f32 <= v {
            size as f32 - NEAR
        } else {
            v
        }
    };
    [axis(p[0], map[0]), axis(p[2], map[1])]
}
