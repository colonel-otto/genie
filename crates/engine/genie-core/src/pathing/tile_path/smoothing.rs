//! The smoothing of a found path (`FUN_140f7dbb0`): its straight stretches wherever the unit
//! can walk straight.

use crate::math::fmath;
use crate::math::fmath::Vec3;
use crate::pathing::tile_path::Path;

/// The longest step between the points the smoothing puts on a straight stretch
/// (`DAT_142f430e8`).
pub(super) const SMOOTH_STEP: f32 = crate::math::image::tile_path::SMOOTH_STEP;
/// The level-0 search runs only for paths shorter than this (`DAT_142f430ec`).
pub(super) const TILE_SEARCH_LENGTH: f32 = crate::math::image::tile_path::TILE_SEARCH_LENGTH;
/// The squared distance from the start within which the planner looks for a shorter first
/// stretch (`DAT_14330ebcc`).
pub(super) const SHORTCUT_REACH: f32 = crate::math::image::tile_path::SHORTCUT_REACH;

/// The obstruction manager as the smoothing asks it for a unit (`FUN_140f7dbb0`).
pub trait SegmentTest<E> {
    /// Whether the unit's straight walk from `a` to `b` (x, y) meets an obstruction
    /// (`FUN_141052b90`, then `FUN_141052350` with flags `0x100`).
    ///
    /// # Errors
    ///
    /// What the test cannot answer.
    fn blocked(&mut self, a: [f32; 2], b: [f32; 2]) -> Result<bool, E>;

    /// A pass that tested walks ends (`FUN_1410561f0`): the temporary tile records its walks put
    /// go.
    fn pass_ends(&mut self);
}

/// The point at `index`, or the zero point outside the path (`FUN_140f9bcd0`).
pub(super) fn point_at(path: &Path, index: i32) -> Vec3 {
    usize::try_from(index)
        .ok()
        .and_then(|i| path.points.get(i))
        .copied()
        .unwrap_or([0.0; 3])
}

/// `FUN_140f761b0`: the points between `first` and `last` replaced by evenly spaced points at
/// most 7.5 apart on the straight line (through the path's own insert, which keeps no two equal
/// points together); returns how many fewer points lie between them now, 0 with none between.
pub(super) fn straighten(path: &mut Path, first: i32, last: i32) -> i32 {
    let between = (last - first) - 1;
    if between < 1 {
        return 0;
    }
    let a = point_at(path, first);
    let d = fmath::sub(point_at(path, last), a);
    let length = fmath::sqrt(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
    let steps = crate::math::conversions::cvttss2si(length / SMOOTH_STEP);
    let fewer = between - steps;
    let at = (first + 1) as usize;
    if fewer < 0 {
        for _ in 0..-fewer {
            let _ = path.insert(at, [0.0; 3]);
        }
    } else {
        for _ in 0..fewer {
            if at < path.points.len() {
                let _ = path.points.remove(at);
            }
        }
    }
    let direction = [d[0] / length, d[1] / length, d[2] / length];
    for t in 1..=steps {
        let step = fmath::scale(length / (steps + 1) as f32, direction);
        let p = fmath::add(a, fmath::scale(t as f32, step));
        if let Some(slot) = usize::try_from(first + t)
            .ok()
            .and_then(|i| path.points.get_mut(i))
        {
            *slot = p;
        }
    }
    fewer
}

/// `FUN_140f7dbb0`: straighten a found path wherever the unit can walk straight: from each
/// kept point, the line to two points on is tested (`walls`, the obstruction manager's segment
/// test); where it is blocked, the stretch up to the point before becomes straight, and the last
/// stretch is straightened at the end. A path of more than two points ends the pass
/// ([`SegmentTest::pass_ends`]), as the game does after its last test.
///
/// # Errors
///
/// What the segment test cannot answer (the pass ends all the same).
pub fn smooth<E>(path: &mut Path, walls: &mut dyn SegmentTest<E>) -> Result<(), E> {
    if path.points.len() <= 2 {
        return Ok(());
    }
    let smoothed = smooth_pass(path, walls);
    walls.pass_ends();
    smoothed
}

/// [`smooth`]'s tests and straight stretches over a path of more than two points.
pub(super) fn smooth_pass<E>(path: &mut Path, walls: &mut dyn SegmentTest<E>) -> Result<(), E> {
    let mut first: i32 = 0;
    let mut probe: i32 = 2;
    while (probe as usize) < path.points.len() {
        let (a, b) = (point_at(path, first), point_at(path, probe));
        let next;
        if walls.blocked([a[0], a[2]], [b[0], b[2]])? {
            let between = (probe - first) - 2;
            if between < 1 {
                first = probe - 1;
                next = probe + 1;
            } else {
                let fewer = straighten(path, first, probe - 1);
                first = (probe - 1) - fewer;
                next = first + 2;
            }
        } else {
            next = probe + 1;
            if next as usize == path.points.len() {
                let _ = straighten(path, first, probe);
            }
        }
        probe = next;
    }
    Ok(())
}
