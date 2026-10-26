//! Where the units of an order go: their centroid, offsets, spreads and grid points.

/// The box of an order's units (`0x140e68b90`, the loop after the splitter): the least and the
/// greatest x and y, where a value below the least so far moves only the least (the greatest
/// starts at 0, the least at `BOX_START`), and the sums. A box at most `SPREAD` each way gives
/// the centroid (the sums alone without a point); a wider one None.
pub(super) fn centroid(points: &[[f32; 2]]) -> Option<[f32; 2]> {
    /// `DAT_142eea788`.
    const BOX_START: f32 = crate::math::image::order::BOX_START;
    /// `DAT_142eb9430`.
    const SPREAD: f32 = crate::math::image::order::SPREAD;
    let (mut least, mut most, mut sum) = ([BOX_START; 2], [0.0f32; 2], [0.0f32; 2]);
    for p in points {
        for k in 0..2 {
            let v = p[k];
            if least[k] <= v {
                if most[k] < v {
                    most[k] = v;
                }
            } else {
                least[k] = v;
            }
            sum[k] += v;
        }
    }
    if SPREAD < most[0] - least[0] || SPREAD < most[1] - least[1] {
        return None;
    }
    if !points.is_empty() {
        let n = points.len() as f32;
        sum = [sum[0] / n, sum[1] / n];
    }
    Some(sum)
}

/// A unit's point in an order to several (`0x140e68b90`): its offset from their centroid added to
/// the order's point, `EDGE` inside the map where it falls off it.
pub(super) fn offset_point(
    position: [f32; 2],
    centroid: [f32; 2],
    point: [f32; 2],
    map: [i32; 2],
) -> [f32; 2] {
    [
        inside_map(position[0] - centroid[0] + point[0], map[0]),
        inside_map(position[1] - centroid[1] + point[1], map[1]),
    ]
}

/// An order's point on one axis kept on a map of `size` tiles: below 0 it becomes `EDGE`, at
/// the size or beyond the size less `EDGE`.
pub(super) fn inside_map(v: f32, size: i32) -> f32 {
    /// `DAT_14330eb8c`.
    const EDGE: f32 = crate::math::image::order::EDGE;
    if v < 0.0 {
        EDGE
    } else if size as f32 <= v {
        size as f32 - EDGE
    } else {
        v
    }
}

/// FOLLOW's box (`FUN_14161c920`, the loop at `0x14161cc72`) over the places (x, y) of the loose
/// units that move, as its code has it: each unit's lesser and greater coordinate
/// (`CMOVBE` on `COMISS x, y` and `COMISS y, x`: a tie or NaN picks x) replace the last unit's,
/// which first fold into a running least (from [`BOX_START`]) and greatest (from 0), each by
/// `COMISS`/`CMOVBE` (a tie or NaN takes the last unit's); the sums run on.
/// The box passes when the last unit's spread and the running one's are each at most
/// [`SPREAD`] (`COMISS`, `JC`: NaN fails); then it gives the mean place (the sums over the
/// count, none without units).
pub(super) fn follow_spread(points: &[[f32; 2]]) -> Option<[f32; 2]> {
    use crate::math::fmath::{at_most_or_unordered, below_or_unordered};
    /// `DAT_142eea788`.
    const BOX_START: f32 = crate::math::image::order::BOX_START;
    /// `DAT_142eb9430`.
    const SPREAD: f32 = crate::math::image::order::SPREAD;
    let (mut lesser, mut least, mut greater, mut greatest) = (BOX_START, BOX_START, 0.0f32, 0.0f32);
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    for &[x, y] in points {
        least = if at_most_or_unordered(lesser, least) {
            lesser
        } else {
            least
        };
        greatest = if at_most_or_unordered(greatest, greater) {
            greater
        } else {
            greatest
        };
        lesser = if at_most_or_unordered(x, y) { x } else { y };
        greater = if at_most_or_unordered(y, x) { x } else { y };
        sx += x;
        sy += y;
    }
    if below_or_unordered(SPREAD, greater - lesser) || below_or_unordered(SPREAD, greatest - least)
    {
        return None;
    }
    if !points.is_empty() {
        let n = points.len() as i32 as f32;
        (sx, sy) = (sx / n, sy / n);
    }
    Some([sx, sy])
}

/// A FOLLOW offset's coordinate on a map of `size` tiles (`FUN_14161c920`): above the size less
/// [`EDGE_GAP`] (at least 0, `MAXSS`) it is that (`COMISS`, `JA`), below 0 it is 0, else itself
/// (a NaN too).
pub(super) fn follow_axis(v: f32, size: i32) -> f32 {
    /// `DAT_142eb99cc`.
    const EDGE_GAP: f32 = crate::math::image::group::SHORT_LENGTH;
    let most = crate::math::fmath::maxss(size as f32 - EDGE_GAP, 0.0);
    if v > most {
        most
    } else if 0.0 > v {
        0.0
    } else {
        v
    }
}

/// The cell of the `i`th of `n` units in MOVE's grid (`FUN_140e67800`): `side` cells a row, the
/// root of `n` times `GRID_AREA` truncated; the unit's column and row, less half the side, a
/// `GRID_STEP` each from the order's point, kept on the map ([`inside_map`]).
pub(super) fn grid_point(i: usize, n: usize, point: [f32; 2], map: [i32; 2]) -> [f32; 2] {
    /// `DAT_142eb4838`.
    const GRID_AREA: f32 = crate::math::image::order::GRID_AREA;
    /// `DAT_142e7f384`.
    const GRID_STEP: f32 = crate::math::image::order::GRID_STEP;
    let side = crate::math::fmath::sqrt(n as i32 as f32 * GRID_AREA) as i32;
    let (i, half) = (i as i32, side / 2);
    let x = (i % side - half) as f32 * GRID_STEP + point[0];
    let y = (i / side - half) as f32 * GRID_STEP + point[1];
    [inside_map(x, map[0]), inside_map(y, map[1])]
}
