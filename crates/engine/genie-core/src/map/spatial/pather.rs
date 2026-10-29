//! The old short pather's queries, the map's edge records and the segment tests.

use crate::GenieError;
use crate::map::spatial::{QUERY_HITS, Spatial, TESTED_KINDS};
use crate::objects::masters::class;

/// A record a pather query met (an entry of `FUN_141051900`'s list): an object's (its id) or a
/// temporary tile record's (`x << 12 | y | 0xf0000000`, `FUN_1410510c0`), with its box
/// (`+0x20`..`+0x2c`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// The record's id (`+0x18`).
    pub id: i32,
    /// Its box x0, y0, x1, y1.
    pub bounds: [f32; 4],
}

/// The id of tile (`x`, `y`)'s temporary record (`FUN_1410510c0`) and of the tile in a pather's
/// ignore list (`FUN_140f89e40`): `x << 12 | y` with the top four bits set.
#[must_use]
pub const fn tile_record_id(x: i32, y: i32) -> i32 {
    (x << 12) | y | (0xf000_0000_u32 as i32)
}

impl Spatial {
    /// The old short pather's line queries (flags 0x40: moving objects left out, every kind
    /// tested), after `FUN_141052b90` put the temporary records (kind 3, the tile's box) of the
    /// walk's box, grown by `buffer`, that the unit's `mask` lacks ([`Spatial::put_tile_records`];
    /// they stay with those of the pather's earlier walks until `FindShortPath` removes them,
    /// [`Spatial::clear_tile_records`]): the records whose box, grown by `buffer`, meets
    /// the walk `a` -> `b` (`FUN_1410512f0`, a circle's `FUN_141051630` alike), whose cell the
    /// query visits ([`Spatial::visits`]) and whose id is not in `ignore` (the manager's list
    /// `+0x2c`, the pather's). An object that moves (its record's
    /// `+3` bit 0x10, set from slot 0x5c8 at the end of its update, [`Gait::moving`]) is left
    /// out, as is a gate the player may pass (slot 0x678, not modelled). `first` stops at the
    /// first hit (`FUN_141052350`); else at the 25th (`FUN_141051900`). The hits are in the
    /// tiles' row order, then by id; the game's are in its grid's order, which its callers do not
    /// read. A walk whose grown box leaves the map is blocked (`FUN_141052350`) or meets the
    /// map's edge records first ([`Spatial::edge_records`]).
    ///
    /// [`Gait::moving`]: crate::map::spatial::Gait::moving
    pub(super) fn pather_query(
        &mut self,
        a: [f32; 2],
        b: [f32; 2],
        buffer: Option<f32>,
        mask: u8,
        ignore: &[i32],
        first: bool,
    ) -> Result<Vec<Hit>, GenieError> {
        const MOST_HITS: usize = 25;
        let lo_x = if b[0] <= a[0] { b[0] } else { a[0] };
        let hi_x = if a[0] <= b[0] { b[0] } else { a[0] };
        let lo_y = if b[1] <= a[1] { b[1] } else { a[1] };
        let hi_y = if a[1] <= b[1] { b[1] } else { a[1] };
        let g = buffer.unwrap_or(0.0);
        self.put_tile_records(a, b, g, mask);
        let [w, h] = self.map;
        let grown = [lo_x - g, lo_y - g, g + hi_x, g + hi_y];
        let mut hits = Vec::new();
        if grown[0] < 0.0 || grown[1] < 0.0 || (w as f32) < grown[2] || (h as f32) < grown[3] {
            hits = self.edge_records(grown, QUERY_HITS);
            if first {
                return Ok(hits);
            }
        }
        self.check_known_records(grown)?;
        let walk = [a[0], a[1], b[0], b[1]];
        let meets = |kind: u8, bx: [f32; 4]| {
            let r = [bx[0] - g, bx[1] - g, g + bx[2], g + bx[3]];
            lo_x <= r[2]
                && r[0] <= hi_x
                && lo_y <= r[3]
                && r[1] <= hi_y
                && record_meets_segment(kind, bx, g, walk)
        };
        let level = self.grid.level(0);
        let shift = self.grid.shift(level);
        for &(y, x) in &self.tile_records {
            let id = tile_record_id(x, y);
            let bx = [x as f32, y as f32, x as f32 + 1.0, y as f32 + 1.0];
            if self.visits_cell(level, x >> shift, y >> shift, grown)
                && !ignore.contains(&id)
                && meets(3, bx)
            {
                hits.push(Hit { id, bounds: bx });
                if first || MOST_HITS <= hits.len() {
                    return Ok(hits);
                }
            }
        }
        for (id, p) in self.records_in(grown) {
            let Some(o) = p.obstruction else {
                continue;
            };
            let kind = if o.kind == 10 { 2 } else { o.kind };
            if !TESTED_KINDS.contains(&kind)
                || self.gaits.get(&id).is_some_and(|g| g.moving)
                || ignore.contains(&id)
                || !self.visits(p, kind, grown)
            {
                continue;
            }
            let bx = p.bounds();
            if !meets(kind, bx) {
                continue;
            }
            if kind == 11 {
                return Err(GenieError::unsupported(
                    "a pather query meeting a record of kind 11 (its own radii)",
                ));
            }
            if p.class == class::GATE {
                return Err(GenieError::unsupported(
                    "a pather query meeting a gate (its players' passage, slot 0x678)",
                ));
            }
            hits.push(Hit { id, bounds: bx });
            if first || MOST_HITS <= hits.len() {
                break;
            }
        }
        Ok(hits)
    }

    /// Whether the old short pather's walk `a` -> `b` meets a record ([`Spatial::pather_query`]:
    /// `FUN_141052b90`, then `FUN_141052350` with flags 0x40).
    pub fn pather_blocked(
        &mut self,
        a: [f32; 2],
        b: [f32; 2],
        buffer: Option<f32>,
        mask: u8,
        ignore: &[i32],
    ) -> Result<bool, GenieError> {
        Ok(!self
            .pather_query(a, b, buffer, mask, ignore, true)?
            .is_empty())
    }

    /// The records the old short pather's walk `a` -> `b` meets, at most 25
    /// ([`Spatial::pather_query`]: `FUN_141052b90`, then `FUN_141051900` with flags 0x40).
    pub fn pather_hits(
        &mut self,
        a: [f32; 2],
        b: [f32; 2],
        buffer: Option<f32>,
        mask: u8,
        ignore: &[i32],
    ) -> Result<Vec<Hit>, GenieError> {
        self.pather_query(a, b, buffer, mask, ignore, false)
    }
}

impl Spatial {
    /// The map edge's records a query rectangle `rect` (`x0, y0, x1, y1`) that leaves the map meets
    /// (`FUN_141057290`, which `FUN_141051900` lists first, before any test of kind, flags or
    /// ignore list): for each edge it crosses (left, top, right, bottom), the manager's record for
    /// that edge (`+0xb0`, `+0xb8`, `+0xc0`, `+0xc8`), set to the tile box where the rectangle's
    /// diagonal `(x0, y0)` -> `(x1, y1)` crosses the edge's line, id `x << 12 | y | 0xff000000`.
    /// The list takes at most `cap` records (its `*param_6`): the routine returns once the list
    /// holds that many, after any edge. The terms are the game's, each with its zero factor
    /// (`0 × dy0`, `(w − w) × dy`, ...), so an infinite or NaN operand gives NaN as there; each
    /// crossing is cast by `CVTTSS2SI` into 32 bits (a NaN or huge one is `i32::MIN`).
    #[must_use]
    // the game's (w - w) and (h - h) factors: 0, but NaN times an infinite or NaN term
    #[allow(clippy::eq_op)]
    pub fn edge_records(&self, rect: [f32; 4], cap: usize) -> Vec<Hit> {
        use crate::math::conversions::cvttss2si;
        let [x0, y0, x1, y1] = rect;
        // the manager's map size as floats (+0xa8, +0xac)
        let (w, h) = (self.map[0] as f32, self.map[1] as f32);
        let (dx, dy) = (x1 - x0, y1 - y0);
        let (dy0, dx0) = (y0 - 0.0, x0 - 0.0);
        let at = |t: f32| {
            let x = cvttss2si(dx * t + x0);
            let y = cvttss2si(dy * t + y0);
            Hit {
                id: ((x << 12) | y | 0xff00_0000_u32 as i32),
                bounds: [x as f32, y as f32, x as f32 + 1.0, y as f32 + 1.0],
            }
        };
        let mut hits = Vec::new();
        let mut edges: [Option<f32>; 4] = [None; 4];
        if x0 < 0.0 {
            let hh = h - 0.0;
            edges[0] = Some((dy0 * 0.0 - hh * dx0) / (dx * hh - dy * 0.0));
        }
        if y0 < 0.0 {
            let hh = h - 0.0;
            edges[1] = Some((hh * dy0 - dx0 * 0.0) / (dx * 0.0 - hh * dy));
        }
        if w < x1 {
            let hh = h - 0.0;
            edges[2] = Some(((w - w) * dy0 - (x0 - w) * hh) / (hh * dx - (w - w) * dy));
        }
        if h < y1 {
            let ww = w - 0.0;
            edges[3] = Some(((y0 - h) * ww - (h - h) * dx0) / ((h - h) * dx - ww * dy));
        }
        for t in edges.into_iter().flatten() {
            hits.push(at(t));
            if cap <= hits.len() {
                break;
            }
        }
        hits
    }
}

/// `FUN_1410576a0`, Sedgewick's ccw: 1 when `p0`, `p1`, `p2` turn counter-clockwise (or lie in
/// a line with `p2` beyond `p1`), -1 clockwise (or `p0` between), 0 when `p2` lies on the segment.
pub(super) fn ccw(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2]) -> i32 {
    let (dx1, dy1) = (p1[0] - p0[0], p1[1] - p0[1]);
    let (dx2, dy2) = (p2[0] - p0[0], p2[1] - p0[1]);
    // every test is COMISS/JA (0x1410576ee..0x14105772e): an unordered one falls through, so a
    // NaN reaches the last answer, 0
    if dy2 * dx1 > dx2 * dy1 {
        return 1;
    }
    if dx2 * dy1 > dy2 * dx1 || 0.0 > dx2 * dx1 || 0.0 > dy2 * dy1 {
        return -1;
    }
    if dy2 * dy2 + dx2 * dx2 > dy1 * dy1 + dx1 * dx1 {
        return 1;
    }
    0
}

/// The smoothing's segment test for a unit (`FUN_140f7dbb0`, [`crate::pathing::tile_path::SegmentTest`]):
/// each walk puts its temporary tile records ([`Spatial::put_tile_records`]) and is tested
/// ([`Spatial::segment_meets`]) against them and those of the pass's earlier walks; the pass's
/// end removes them ([`Spatial::clear_tile_records`]).
#[derive(Debug)]
pub struct SmoothingWalls<'a> {
    pub(super) world: &'a mut Spatial,
    pub(super) radius: f32,
    pub(super) mask: u8,
    pub(super) ignore: &'a [i32],
    /// Whether a walk was tested.
    pub ran: bool,
}

impl<'a> SmoothingWalls<'a> {
    /// The test of a unit `radius` wide (the manager's buffer) whose terrain mask is `mask`,
    /// with the manager's ignore list `ignore`.
    #[must_use]
    pub const fn new(world: &'a mut Spatial, radius: f32, mask: u8, ignore: &'a [i32]) -> Self {
        Self {
            world,
            radius,
            mask,
            ignore,
            ran: false,
        }
    }
}

impl<E> crate::pathing::tile_path::SegmentTest<E> for SmoothingWalls<'_> {
    fn blocked(&mut self, a: [f32; 2], b: [f32; 2]) -> Result<bool, E> {
        self.ran = true;
        // the pass's buffer, the unit's radius (FUN_1410562d0 with 1)
        self.world.set_manager_buffer(Some(self.radius));
        self.world.put_tile_records(a, b, self.radius, self.mask);
        Ok(self.world.segment_meets(a, b, self.radius, self.ignore))
    }

    fn pass_ends(&mut self) {
        self.world.set_manager_buffer(None);
        self.world.clear_tile_records();
    }
}

/// A straight walk's test against records (`FUN_141052350`, `FUN_141052800`): the walk's box
/// (its ends' lower and upper x and y) and the query's box, grown by the manager's buffer.
pub(super) struct WalkTest {
    pub(super) walk: [f32; 4],
    pub(super) lo: [f32; 2],
    pub(super) hi: [f32; 2],
    pub(super) grow: f32,
    pub(super) query: [f32; 4],
}

impl WalkTest {
    /// The walk from `a` to `b` of a unit `grow` wide; None when its grown box leaves a map of
    /// `map` tiles (the query then counts as met).
    pub(super) fn new(a: [f32; 2], b: [f32; 2], grow: f32, [w, h]: [i32; 2]) -> Option<Self> {
        let lo = [
            if b[0] <= a[0] { b[0] } else { a[0] },
            if b[1] <= a[1] { b[1] } else { a[1] },
        ];
        let hi = [
            if a[0] <= b[0] { b[0] } else { a[0] },
            if a[1] <= b[1] { b[1] } else { a[1] },
        ];
        let query = [lo[0] - grow, lo[1] - grow, grow + hi[0], grow + hi[1]];
        if query[0] < 0.0 || query[1] < 0.0 || (w as f32) < query[2] || (h as f32) < query[3] {
            return None;
        }
        Some(Self {
            walk: [a[0], a[1], b[0], b[1]],
            lo,
            hi,
            grow,
            query,
        })
    }

    /// Whether a record's box `bx`, grown by the buffer, meets the walk's box (edges included)
    /// and segment (`FUN_141052e10`).
    pub(super) fn meets(&self, bx: [f32; 4]) -> bool {
        let g = self.grow;
        let grown = [bx[0] - g, bx[1] - g, g + bx[2], g + bx[3]];
        self.lo[0] <= grown[2]
            && grown[0] <= self.hi[0]
            && self.lo[1] <= grown[3]
            && grown[1] <= self.hi[1]
            && box_meets_segment(bx, g, self.walk)
    }
}

/// `FUN_141054ec0`: whether segments `p1`–`p2` and `p3`–`p4` meet.
pub(super) fn segments_meet(p1: [f32; 2], p2: [f32; 2], p3: [f32; 2], p4: [f32; 2]) -> bool {
    (ccw(p1, p2, p4) * ccw(p1, p2, p3)) as f32 <= 0.0
        && (ccw(p3, p4, p2) * ccw(p3, p4, p1)) as f32 <= 0.0
}

/// `FUN_141052e10` (the segment test's) and `FUN_1410512f0` (the walk test's) with the box grown
/// by `r` (the manager's `+0x57`/`+0x58`): the walk `[x0, y0, x1, y1]` crosses one of the grown
/// box's edges or starts or ends inside it, the box ordered (low and high edges) for the inside
/// tests.
pub(super) fn box_meets_segment([bx0, by0, bx1, by1]: [f32; 4], r: f32, walk: [f32; 4]) -> bool {
    let lo_x = if bx1 < bx0 { bx1 } else { bx0 };
    let hi_x = if bx0 < bx1 { bx1 } else { bx0 };
    let lo_y = if by1 < by0 { by1 } else { by0 };
    let hi_y = if by0 < by1 { by1 } else { by0 };
    segment_meets_box([bx0, by0, bx1, by1], r, walk, [lo_x, lo_y, hi_x, hi_y])
}

/// A record's test against the walk by its kind (`FUN_141051e00`, `FUN_141052800`): a circle's
/// (kind 5, `FUN_141051630` and `FUN_141053130`) reads its box as it stands for the inside tests;
/// the others' (kinds 2, 3, 11, 13) order it ([`box_meets_segment`]).
pub(super) fn record_meets_segment(kind: u8, b: [f32; 4], r: f32, walk: [f32; 4]) -> bool {
    if kind == 5 {
        segment_meets_box(b, r, walk, b)
    } else {
        box_meets_segment(b, r, walk)
    }
}

/// The four edges of box `[bx0, by0, bx1, by1]` grown by `r`, each tested against the walk
/// (`FUN_141052ca0`: `FUN_141054ec0`), then the inside tests of the walk's ends against `inside`
/// grown by `r`. `FUN_141052ca0` also lists each crossing (record, id, point) at the manager's
/// `+0x40` while its `+0x54` is set; `FUN_141051900` sets `+0x54` only for a list at `+0x40`
/// with a cap `+0x50` of at least 1, and nothing gives the manager one: the only stores to
/// `+0x40`, `+0x48` and `+0x50` in its code (0x14104f000..0x141060000) are its constructor's
/// and `FUN_1410560b0`'s zeros, and none of the routines that address it through
/// `DAT_143f0a088` writes them. So the listing never runs, and the edges' order (the
/// `FUN_1410512f0` tests all four, `FUN_141052e10` stops at the first crossing) does not show.
pub(super) fn segment_meets_box(
    [bx0, by0, bx1, by1]: [f32; 4],
    r: f32,
    walk: [f32; 4],
    [lo_x, lo_y, hi_x, hi_y]: [f32; 4],
) -> bool {
    let (start, end) = ([walk[0], walk[1]], [walk[2], walk[3]]);
    let edges = [
        ([bx0 - r, by0 - r], [r + bx1, by0 - r]),
        ([r + bx1, by0 - r], [r + bx1, r + by1]),
        ([r + bx1, r + by1], [bx0 - r, r + by1]),
        ([bx0 - r, r + by1], [bx0 - r, by0 - r]),
    ];
    if edges
        .iter()
        .any(|&(e0, e1)| segments_meet(start, end, e0, e1))
    {
        return true;
    }
    // COMISS/JC (0x141052fb8..0x141052ff9, 0x1410530dc..0x141053119; 0x1410514ad..0x141051620,
    // 0x14105189e..0x1410518f2): a NaN coordinate is out
    let outside = |p: [f32; 2]| {
        crate::math::fmath::below_or_unordered(p[0], lo_x - r)
            || crate::math::fmath::below_or_unordered(r + hi_x, p[0])
            || crate::math::fmath::below_or_unordered(p[1], lo_y - r)
            || crate::math::fmath::below_or_unordered(r + hi_y, p[1])
    };
    !(outside(start) && outside(end))
}
