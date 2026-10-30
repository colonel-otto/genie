//! The walked level's flags, its reset, a looping path's next waypoint and the waypoints'
//! points and ground lengths.

use crate::GenieError;
use crate::groups::motion::NEAR;
use crate::map::GridSize;
use crate::map::Point;
use crate::math::fmath;
use crate::math::fmath::BVector;
use crate::movement::PathData;
use crate::world::slots::CheckedIndex;

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
pub(super) fn reset_level(paths: &mut Vec<PathData>) -> Result<(), GenieError> {
    let repath_time = paths.first().map_or(0.0, |l| l.repath_time);
    paths.truncate(1);
    let level = PathData {
        repath_time,
        ..PathData::without_path()
    };
    if paths.is_empty() {
        paths.push(level);
    } else {
        *paths.checked_mut(0)? = level;
    }
    Ok(())
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
pub(super) fn point(level: &PathData, index: i32) -> BVector {
    usize::try_from(index)
        .ok()
        .and_then(|i| level.points.get(i))
        .copied()
        .unwrap_or(BVector::ZERO)
}

/// The length on the ground from `a` to `b` (`FUN_14087e560`, the height zeroed).
pub(super) fn ground(a: BVector, b: BVector) -> f32 {
    let mut d = fmath::sub(a, b);
    d = d.with_height(0.0);
    fmath::sqrt(d.x() * d.x() + 0.0 + d.y() * d.y())
}

/// `FUN_141046130` for a walk from `at` toward `waypoint` that would end at `next`: 0 short of
/// it (`COMISS` then `JBE`: a NaN step or distance is short), 2 past it (or a step under 0.1),
/// 1 on it.
pub(in crate::groups) fn passes_waypoint(waypoint: BVector, at: BVector, next: BVector) -> u8 {
    let mut a = fmath::sub(waypoint, at);
    let mut b = fmath::sub(waypoint, next);
    let c = fmath::sub(next, at);
    a = a.with_height(0.0);
    b = b.with_height(0.0);
    let to_waypoint = fmath::sqrt(a.x() * a.x() + 0.0 + a.y() * a.y());
    let beyond = fmath::sqrt(b.x() * b.x() + b.height() * b.height() + b.y() * b.y());
    let step = fmath::sqrt(c.x() * c.x() + c.height() * c.height() + c.y() * c.y());
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
pub(in crate::groups) fn hold(p: BVector, map: GridSize) -> Point {
    let axis = |v: f32, size: i32| {
        if v < 0.0 {
            0.0
        } else if size as f32 <= v {
            size as f32 - NEAR
        } else {
            v
        }
    };
    Point::new(axis(p.x(), map.width()), axis(p.y(), map.height()))
}
