//! The tile pathfinder: a planner per terrain restriction in use (the world's list at
//! `world + 0x140`, made by `FUN_140f7d310`), which searches a four-level grid of the map's tiles
//! for the path a unit's top path level asks for (`FUN_140f7c530`, called from the motion step's
//! state-2 branch).
//!
//! The grid (`+0x3d8`, two bytes per cell) holds level 0, one cell per tile, rounded up to whole
//! level-3 cells of 8 × 8 tiles, and three coarser levels, each cell covering 2 × 2 cells of the
//! level below. Byte 0 of a cell holds the edges a unit can cross ([`N`], [`W`], [`E`], [`S`]) and,
//! during a search, [`OPEN`] and [`CLOSED`]; byte 1 the children that lie off the map (its low
//! nibble), [`GOAL`], [`NODE`] (the cell is uniform and searched as one node), [`MIXED`] (its
//! children are searched instead) and [`LONE`] (a passable tile with no passable neighbour).
//!
//! A search runs A* over the coarsest node cells, from the start's cell to any cell marked as
//! the goal, with the open nodes in buckets by their whole cost ([`OpenList`]). The planner lives
//! in the world, not in the save: the slice builds it from the map's terrain, the restriction's
//! passability and the obstruction manager's blocked tiles, as the game does at load.

use crate::fmath::{self, Vec3};
use crate::msvc;

/// Byte 0: the edges to the cells above (y − 1), left, right and below (y + 1).
pub const N: u8 = 1;
/// The edge to the cell left.
pub const W: u8 = 2;
/// The edge to the cell right.
pub const E: u8 = 4;
/// The edge to the cell below (y + 1).
pub const S: u8 = 8;
/// Byte 0 during a search: a node of the cell was opened, or closed.
pub const OPEN: u8 = 0x10;
/// A node of the cell was closed.
pub const CLOSED: u8 = 0x20;
/// Byte 1: the goal's cell.
pub const GOAL: u8 = 0x10;
/// Byte 1: the cell is uniform and searched as one node.
pub const NODE: u8 = 0x20;
/// Byte 1: the cell's children are searched instead of it.
pub const MIXED: u8 = 0x40;
/// Byte 1: a passable tile with no passable neighbour.
pub const LONE: u8 = 0x80;

/// The cost the best node starts from in each search (`0x497423f0`).
const NO_COST: f32 = 999_999.0;
/// The goal's own tile counts when the goal is this near it (`DAT_142eb99cc`).
const GOAL_GAP: f32 = crate::image::tile_path::GOAL_GAP;
/// The planner's half tile (`DAT_142e7f384`) and its −0.5 (`DAT_142eb6e58`).
const HALF: f32 = 0.5;
/// A goal's range below 1 grows the marked square by 1 (`DAT_14330eb8c`).
const ONE: f32 = crate::image::tile_path::ONE;
/// The hierarchical search marks 0.1 more on the square's far sides (`DAT_142eb5690`).
const FAR_SIDE: f32 = crate::image::tile_path::FAR_SIDE;
/// A computer player's search stops early near the goal (`DAT_14330ebb8`, `DAT_142eb0f4c`).
const EARLY_LEFT: f32 = crate::image::tile_path::EARLY_LEFT;
const EARLY_START: f32 = crate::image::tile_path::EARLY_START;
/// The level-0 search costs a turn's shortcut from the node before at 1.1 times its cost
/// (`DAT_142e86c14`).
const SHORTCUT_COST: f32 = crate::image::tile_path::SHORTCUT_COST;

/// What a search returns (its result byte).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
    /// 0: the start or the goal lies off the map.
    OffMap,
    /// 1: the path reaches the goal.
    Goal,
    /// 2: the path ends at the node nearest the goal.
    Nearest,
    /// 3: no node at all.
    Nothing,
}

impl Found {
    /// The search's result as the game's code (0 to 3).
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::OffMap => 0,
            Self::Goal => 1,
            Self::Nearest => 2,
            Self::Nothing => 3,
        }
    }
}

/// A path's points and its length, as the game's path data holds them (`+0x438`..`+0x440`, `+4`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    /// The points.
    pub points: Vec<Vec3>,
    /// The length.
    pub length: f32,
}

impl Path {
    /// Insert `p` at `index` (`FUN_140f95060`) unless it equals the point before or the point at
    /// `index`; false when it does or `index` lies beyond the end.
    pub fn insert(&mut self, index: usize, p: Vec3) -> bool {
        let n = self.points.len();
        if n < index
            || (0 < index && self.points[index - 1] == p)
            || (index < n && self.points[index] == p)
        {
            return false;
        }
        self.points.insert(index, p);
        true
    }

    /// Append `p` (`FUN_140f95a10`).
    pub fn push(&mut self, p: Vec3) -> bool {
        self.insert(self.points.len(), p)
    }

    /// The length along the points in 3D (`FUN_140f95a60`).
    pub fn measure(&mut self) {
        let mut total = 0.0;
        for w in self.points.windows(2) {
            let d = fmath::sub(w[1], w[0]);
            total += fmath::sqrt(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
        }
        self.length = total;
    }

    /// Append `other`'s points to these (`FUN_140f95c00`); the length is `other`'s, or measured
    /// anew when `other`'s is at most 1e-7 or unordered (`COMISS`, `JBE` at 0x140f95d66).
    pub fn append(&mut self, other: &Self) {
        for &p in &other.points {
            let _ = self.push(p);
        }
        if fmath::at_most_or_unordered(other.length, 1e-7) {
            self.measure();
        } else {
            self.length = other.length;
        }
    }
}

/// The longest step between the points the smoothing puts on a straight stretch
/// (`DAT_142f430e8`).
const SMOOTH_STEP: f32 = crate::image::tile_path::SMOOTH_STEP;
/// The level-0 search runs only for paths shorter than this (`DAT_142f430ec`).
const TILE_SEARCH_LENGTH: f32 = crate::image::tile_path::TILE_SEARCH_LENGTH;
/// The squared distance from the start within which the planner looks for a shorter first
/// stretch (`DAT_14330ebcc`).
const SHORTCUT_REACH: f32 = crate::image::tile_path::SHORTCUT_REACH;

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
fn point_at(path: &Path, index: i32) -> Vec3 {
    usize::try_from(index)
        .ok()
        .and_then(|i| path.points.get(i))
        .copied()
        .unwrap_or([0.0; 3])
}

/// `FUN_140f761b0`: the points between `first` and `last` replaced by evenly spaced points at
/// most 7.5 apart on the straight line (through the path's own insert, which keeps no two equal
/// points together); returns how many fewer points lie between them now, 0 with none between.
fn straighten(path: &mut Path, first: i32, last: i32) -> i32 {
    let between = (last - first) - 1;
    if between < 1 {
        return 0;
    }
    let a = point_at(path, first);
    let d = fmath::sub(point_at(path, last), a);
    let length = fmath::sqrt(d[0] * d[0] + d[1] * d[1] + d[2] * d[2]);
    let steps = crate::checkpoint::cvttss2si(length / SMOOTH_STEP);
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
fn smooth_pass<E>(path: &mut Path, walls: &mut dyn SegmentTest<E>) -> Result<(), E> {
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

/// A search node (`0x28` bytes, from the planner's pool `+0x428`, `FUN_140f79c90`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Node {
    /// `+0`: the cell's index.
    cell: usize,
    /// `+8`, `+0xa`: the cell at its level.
    x: i16,
    y: i16,
    /// `+0xc`, `+0xe`: the tile the cost is measured from.
    entry: [i16; 2],
    /// `+0x10`: cost so far plus the estimate to the goal; `+0x14`: the cost so far.
    f: f32,
    g: f32,
    /// `+0x18`.
    level: u8,
    /// `+0x19`: the edge back to the parent.
    dir: u8,
    /// `+0x1a`: the cost was measured from the grandparent.
    shortcut: bool,
    /// `+0x20`.
    parent: Option<usize>,
}

/// The open nodes (`+0x480`): a bucket per whole cost, the lowest one sorted with its cheapest
/// node last (`FUN_140f9cba0`, `FUN_140f9cd40`).
#[derive(Clone, Debug, Default, PartialEq)]
struct OpenList {
    buckets: Vec<Vec<usize>>,
    /// `+0x18` with its flag `+0x1c`, `+0x20` with `+0x24`.
    min: Option<i32>,
    max: Option<i32>,
}

impl OpenList {
    /// `FUN_140f9c9f0`.
    fn clear(&mut self) {
        for b in &mut self.buckets {
            b.clear();
        }
        self.min = None;
        self.max = None;
    }

    const fn is_empty(&self) -> bool {
        self.min.is_none()
    }

    /// `FUN_140f9cba0`: into the lowest bucket before the first node from the back that costs
    /// more; into a new lowest or a higher bucket at its end.
    fn push(&mut self, node: usize, nodes: &[Node]) {
        let f = nodes[node].f;
        let b = f as i32;
        let index = b as usize;
        if self.buckets.len() <= index {
            self.buckets.resize_with(index + 1, Vec::new);
        }
        match self.min {
            Some(m) if b == m => {
                let bucket = &mut self.buckets[index];
                let at = (1..=bucket.len())
                    .rev()
                    .find(|&k| f < nodes[bucket[k - 1]].f)
                    .unwrap_or(0);
                bucket.insert(at, node);
                return;
            }
            Some(m) if m < b => {}
            _ => self.min = Some(b),
        }
        if self.max.is_none_or(|m| m < b) {
            self.max = Some(b);
        }
        self.buckets[index].push(node);
    }

    /// `FUN_140f9cd40`: the last node of the lowest bucket; an emptied bucket hands over to the
    /// next one that holds nodes, which is sorted then.
    fn pop(&mut self, nodes: &[Node]) -> Option<usize> {
        let m = self.min?;
        let bucket = &mut self.buckets[m as usize];
        let node = bucket.pop()?;
        if bucket.is_empty() {
            let max = self.max.unwrap_or(m);
            if let Some(i) = (m + 1..=max).find(|&i| !self.buckets[i as usize].is_empty()) {
                self.min = Some(i);
                msvc::sort(&mut self.buckets[i as usize], &|a: usize, b: usize| {
                    nodes[a].f > nodes[b].f
                });
            } else {
                self.min = None;
                self.max = None;
            }
        }
        Some(node)
    }
}

/// A terrain restriction's planner (`0x4a8` bytes, built by `FUN_140f74f90`).
#[derive(Clone, Debug, PartialEq)]
pub struct Planner {
    /// `+0x400`, `+0x402`: the map's size.
    size: [i16; 2],
    /// Per level: `+0x3f0` width, `+0x3f8` height, `+0x3e0` first cell.
    widths: [i16; 4],
    heights: [i16; 4],
    offsets: [i32; 4],
    cells: Vec<[u8; 2]>,
    /// `+0x3c0`: the restriction's passability per terrain.
    passable: Vec<f32>,
    /// Each tile's terrain (the map's tile `+9`), row by row.
    terrain: Vec<u8>,
    /// `+0x408`..`+0x40e`: the tiles an update or the goal's marks cover.
    region: [i16; 4],
    /// `+0x410`, `+0x412`: the goal's tile.
    goal: [i16; 2],
    /// `+0x414`, `+0x416`: the tile the path was last built to.
    last: [i16; 2],
    /// `+0x478`: the goal's marks found a node cell.
    goal_marked: bool,
    nodes: Vec<Node>,
    open: OpenList,
    /// `+0x470` and `+0xb8`: the node nearest the goal and its estimate.
    best: Option<usize>,
    best_cost: f32,
    /// `+0x448`: the path's points from the goal back.
    points: Vec<Vec3>,
}

/// What the grid reads of the map: whether the obstruction manager marks a tile blocked
/// (`DAT_143f0a088 + 0x98`, bit `0x4000`).
pub trait Blocked {
    /// Tile (`x`, `y`) is blocked.
    fn blocked(&self, x: i32, y: i32) -> bool;
}

impl<F: Fn(i32, i32) -> bool> Blocked for F {
    fn blocked(&self, x: i32, y: i32) -> bool {
        self(x, y)
    }
}

impl Planner {
    /// Whether two planners hold the same grid: the map's size, the levels, the cells, the
    /// restriction's passability and the terrain. What a search leaves (the region, the goal, the
    /// last tile, the goal's marks, the nodes, the open list, the best node and the points) is
    /// not compared: a planner built afresh has searched nothing.
    #[must_use]
    pub fn same_grid(&self, other: &Self) -> bool {
        self.size == other.size
            && self.widths == other.widths
            && self.heights == other.heights
            && self.offsets == other.offsets
            && self.cells == other.cells
            && self.passable == other.passable
            && self.terrain == other.terrain
    }

    /// Build the grid (`FUN_140f74f90`): every cell's off-map children (`FUN_140f7da90`), then
    /// every tile's edges and the levels above them (`FUN_140f7e4b0` over the whole map).
    /// `terrain` holds each tile's terrain row by row, `passable` the restriction's value per
    /// terrain (above 0 passes).
    pub fn new(size: [i32; 2], terrain: Vec<u8>, passable: Vec<f32>, map: &impl Blocked) -> Self {
        let (w, h) = (size[0] as i16, size[1] as i16);
        let w0 = (w.wrapping_add(7)) & !7;
        let h0 = (h.wrapping_add(7)) & !7;
        let widths = [w0, w0 >> 1, w0 >> 2, w0 >> 3];
        let heights = [h0, h0 >> 1, h0 >> 2, h0 >> 3];
        let area = |l: usize| i32::from(widths[l]) * i32::from(heights[l]);
        let offsets = [0, area(0), area(0) + area(1), area(0) + area(1) + area(2)];
        let total = (offsets[3] + area(3)).max(0) as usize;
        let mut planner = Self {
            size: [w, h],
            widths,
            heights,
            offsets,
            cells: vec![[0, 0]; total],
            passable,
            terrain,
            region: [0; 4],
            goal: [0; 2],
            last: [0; 2],
            goal_marked: false,
            nodes: Vec::new(),
            open: OpenList::default(),
            best: None,
            best_cost: NO_COST,
            points: Vec::new(),
        };
        for x in (0..widths[3]).rev() {
            for y in (0..heights[3]).rev() {
                let _ = planner.init_cell(3, x, y);
            }
        }
        planner.update([0, 0, w, h], map);
        planner
    }

    /// The restriction's passability per terrain the planner was made for.
    #[must_use]
    pub fn passable(&self) -> &[f32] {
        &self.passable
    }

    /// The grid of `level`, row by row, both bytes of each cell.
    #[must_use]
    pub fn level(&self, level: usize) -> &[[u8; 2]] {
        let start = self.offsets[level] as usize;
        let n = (i32::from(self.widths[level]) * i32::from(self.heights[level])) as usize;
        &self.cells[start..start + n]
    }

    /// The level sizes: widths, then heights.
    #[must_use]
    pub const fn level_sizes(&self) -> ([i16; 4], [i16; 4]) {
        (self.widths, self.heights)
    }

    fn index(&self, level: u8, x: i16, y: i16) -> usize {
        let l = usize::from(level);
        let i =
            i64::from(self.widths[l]) * i64::from(y) + i64::from(x) + i64::from(self.offsets[l]);
        usize::try_from(i).expect("a cell outside the planner's grid")
    }

    fn cell(&self, level: u8, x: i16, y: i16) -> [u8; 2] {
        self.cells[self.index(level, x, y)]
    }

    /// `FUN_140f7da90`: clear a cell and mark its children that lie off the map; true when all of
    /// it does.
    fn init_cell(&mut self, level: u8, x: i16, y: i16) -> bool {
        let i = self.index(level, x, y);
        self.cells[i] = [0, 0];
        if level == 0 {
            if self.size[0] <= x || self.size[1] <= y {
                self.cells[i][1] = 0xf;
            }
        } else {
            let (x2, y2, l) = (x.wrapping_mul(2), y.wrapping_mul(2), level - 1);
            let children = [(x2, y2), (x2 + 1, y2), (x2, y2 + 1), (x2 + 1, y2 + 1)];
            for (bit, (cx, cy)) in children.into_iter().enumerate() {
                if self.init_cell(l, cx, cy) {
                    self.cells[i][1] += 1 << bit;
                }
            }
        }
        self.cells[i][1] & 0xf == 0xf
    }

    /// `FUN_140f799b0` (through the passability lambda, `0x140f7b110`): 1 when the tile is
    /// blocked, else a bit per passable neighbour (2 left, 4 right, 8 above, 0x10 below).
    fn adjacency(&self, x: i32, y: i32, map: &impl Blocked) -> u32 {
        let (w, h) = (i32::from(self.size[0]), i32::from(self.size[1]));
        let passes = |x: i32, y: i32| {
            let t = self.terrain[(y * w + x) as usize];
            0.0 < self.passable[usize::from(t)] && !map.blocked(x, y)
        };
        if !passes(x, y) {
            return 1;
        }
        let mut bits = 0;
        if 0 < x && passes(x - 1, y) {
            bits = 2;
        }
        if x < w - 1 && passes(x + 1, y) {
            bits |= 4;
        }
        if 0 < y && passes(x, y - 1) {
            bits |= 8;
        }
        if y < h - 1 && passes(x, y + 1) {
            bits |= 0x10;
        }
        bits
    }

    /// Rebuild the cells over a changed rectangle of tiles and its border (`FUN_140f7e4b0`, which
    /// the obstruction manager calls back through `FUN_140f7d4b0` when tiles change).
    pub fn update(&mut self, [x0, y0, x1, y1]: [i16; 4], map: &impl Blocked) {
        let (lo_x, hi_x) = (x0.min(x1), x0.max(x1).wrapping_add(1));
        let (lo_y, hi_y) = (y0.min(y1), y0.max(y1).wrapping_add(1));
        let rx0 = (lo_x - 1).max(0);
        let ry0 = (lo_y - 1).max(0);
        let rx1 = if self.size[0] <= hi_x {
            self.size[0] - 1
        } else {
            hi_x
        };
        let ry1 = if self.size[1] <= hi_y {
            self.size[1] - 1
        } else {
            hi_y
        };
        self.region = [rx0, ry0, rx1, ry1];
        for cx in (rx0 >> 3)..=(rx1 >> 3) {
            for cy in (ry0 >> 3)..=(ry1 >> 3) {
                let _ = self.rebuild(3, cx, cy, map);
            }
        }
    }

    /// `FUN_140f7e5e0`: a cell's edges and flags from its tile (level 0) or its children; returns
    /// both bytes.
    fn rebuild(&mut self, level: u8, x: i16, y: i16, map: &impl Blocked) -> [u8; 2] {
        let i = self.index(level, x, y);
        let old = self.cells[i][1];
        let mut low = old & 0xf;
        if level == 0 {
            self.cells[i][1] = low;
            if old & LONE != 0 {
                self.cells[i][1] = low + LONE;
            }
            let [rx0, ry0, rx1, ry1] = self.region;
            if rx0 <= x && ry0 <= y && x <= rx1 && y <= ry1 {
                let a = self.adjacency(i32::from(x), i32::from(y), map);
                self.cells[i][0] = 0;
                if a & 1 == 0 {
                    let mut b = 0;
                    if a & 2 != 0 {
                        b = W;
                    }
                    if a & 4 != 0 {
                        b |= E;
                    }
                    if a & 8 != 0 {
                        b |= N;
                    }
                    if a & 0x10 != 0 {
                        b |= S;
                    }
                    self.cells[i][0] = b;
                    if b == 0 {
                        self.cells[i][1] |= LONE | NODE;
                    } else {
                        self.cells[i][1] &= !LONE;
                    }
                }
            }
            return self.cells[i];
        }
        let (x2, y2, l) = (x.wrapping_mul(2), y.wrapping_mul(2), level - 1);
        // the children in the game's order: (2x, 2y), (2x + 1, 2y), (2x, 2y + 1), (2x + 1, 2y + 1)
        let at = [(x2, y2), (x2 + 1, y2), (x2, y2 + 1), (x2 + 1, y2 + 1)];
        let mut child = [[0u8; 2]; 4];
        for (k, &(cx, cy)) in at.iter().enumerate() {
            if low == 0 || self.cells[i][1] & (1 << k) == 0 {
                child[k] = self.rebuild(l, cx, cy, map);
            }
        }
        let [d0, d1, d2, d3] = child.map(|c| c[0]);
        low = self.cells[i][1] & 0xf;
        let dirs = ((d3 | d2) & S) + ((d3 | d1) & E) + ((d2 | d0) & W) + ((d1 | d0) & N);
        let complex = child.iter().any(|c| c[1] & (MIXED | LONE) != 0)
            || self.splits(child.map(|c| c[0]), dirs);
        if complex {
            low += MIXED;
            for (k, &(cx, cy)) in at.iter().enumerate() {
                if child[k][1] & (NODE | MIXED) == 0 {
                    let c = self.index(l, cx, cy);
                    self.cells[c][1] = self.cells[c][1].wrapping_add(NODE);
                }
            }
        } else if level == 3 {
            low += NODE;
        }
        self.cells[i] = [dirs, low];
        self.cells[i]
    }

    /// Whether four children with edges `d` fall apart as one cell (`FUN_140f7e5e0`'s labels):
    /// three links among the ring of children, no edge out but a passable child, or children with
    /// edges that join none of the others.
    fn splits(&self, [d0, d1, d2, d3]: [u8; 4], dirs: u8) -> bool {
        let link01 = d0 & E;
        let mut b4: u8 = if d1 != 0 { 2 } else { 0 };
        if link01 != 0 {
            b4 = u8::from(d0 != 0);
        }
        let mut links: u8 = u8::from(link01 != 0) + 1;
        let mut b15 = b4;
        if d1 & S == 0 {
            links = u8::from(link01 != 0);
            b15 = if d3 != 0 { 4 } else { 0 };
        }
        let mut links2 = links + 1;
        let mut b10 = b15;
        if d3 & W == 0 {
            links2 = links;
            b10 = if d2 != 0 { 3 } else { 0 };
        }
        let mut links3 = links2 + 1;
        let mut b9 = b10;
        if d2 & N == 0 {
            b9 = u8::from(d0 != 0);
            links3 = links2;
        }
        if link01 != 0 {
            b4 = b9;
        }
        if d1 & S != 0 {
            b15 = b4;
        }
        let labels = [b9, b4, b10, b15];
        if links3 == 3 || (dirs == 0 && labels != [0; 4]) {
            return true;
        }
        let mut seen = 0;
        for b in labels {
            if b != 0 {
                if seen == 0 || seen == b {
                    seen = b;
                } else {
                    return true;
                }
            }
        }
        false
    }
}

/// A search's request: from a point to a goal with its ranges, as the planner's entry passes
/// them on (`FUN_140f7c530`'s arguments 3 to 11).
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// x, y on the map.
    pub from: [f32; 2],
    /// x, y of the goal on the map.
    pub to: [f32; 2],
    /// The unit's and its target's radii, and how near the goal counts as there (the path
    /// level's range).
    pub radii: f32,
    /// How near the goal counts as there (the path level's range).
    pub range: f32,
    /// Rectangles of tiles closed for this search (x0, y0, x1, y1).
    pub closed: Vec<[i16; 4]>,
    /// A human player's search: no early stop near the goal.
    pub full: bool,
}

/// The goal of a search: its point on the map and both ranges.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Goal {
    x: f32,
    y: f32,
    radii: f32,
    range: f32,
}

impl Request {
    const fn goal(&self) -> Goal {
        Goal {
            x: self.to[0],
            y: self.to[1],
            radii: self.radii,
            range: self.range,
        }
    }
}

/// `(short)(int)v`: the game's tile of a coordinate.
const fn tile(v: f32) -> i16 {
    v as i32 as i16
}

impl Planner {
    /// `FUN_140f79c90`: a new node from the pool.
    #[allow(clippy::too_many_arguments)]
    fn alloc(
        &mut self,
        x: i16,
        y: i16,
        level: u8,
        g: f32,
        h: f32,
        parent: Option<usize>,
        dir: u8,
        entry: [i16; 2],
    ) -> usize {
        let cell = self.index(level, x, y);
        self.nodes.push(Node {
            cell,
            x,
            y,
            entry,
            f: g + h,
            g,
            level,
            dir,
            shortcut: false,
            parent,
        });
        self.nodes.len() - 1
    }

    /// The estimate from a tile to the goal's, in whole tiles (`FUN_140f764c0`, `FUN_140f76840`).
    fn estimate_from(&self, x: i16, y: i16) -> f32 {
        let dx = i32::from(self.goal[0]) - i32::from(x);
        let dy = i32::from(self.goal[1]) - i32::from(y);
        fmath::sqrt((dy * dy + dx * dx) as f32)
    }

    /// The start's estimate (`FUN_140f78850`, `FUN_140f79250`): in f32 differences.
    fn start_estimate(&self, x: i16, y: i16) -> f32 {
        let dx = f32::from(self.goal[0]) - f32::from(x);
        let dy = f32::from(self.goal[1]) - f32::from(y);
        fmath::sqrt(dy * dy + dx * dx)
    }

    /// Where a new node's cost is measured from (`FUN_140f764c0`, `FUN_140f76840`): a turn
    /// (a move across the parent's own move) measures from the grandparent, unless the parent
    /// already did; returns that node's cost so far, its tile and whether it was the grandparent.
    fn measured_from(&self, parent: usize, dir: u8) -> (f32, [i16; 2], bool) {
        let p = &self.nodes[parent];
        if let (Some(gp), false) = (p.parent, p.shortcut) {
            let turn = if dir == W || dir == E {
                p.dir == N || p.dir == S
            } else {
                p.dir == W || p.dir == E
            };
            if turn {
                let gp = &self.nodes[gp];
                return (gp.g, gp.entry, true);
            }
        }
        (p.g, p.entry, false)
    }

    /// `FUN_140f764c0`: the neighbour across edge `dir` (the neighbour's edge back) of a node, at
    /// the level where it is a node cell; Some when it lies in the goal.
    #[allow(clippy::too_many_arguments)]
    fn expand(
        &mut self,
        level: u8,
        x: i16,
        y: i16,
        g: f32,
        parent: usize,
        dir: u8,
    ) -> Option<usize> {
        let i = self.index(level, x, y);
        let [b0, b1] = self.cells[i];
        if dir & b0 == 0 {
            return None;
        }
        if b1 & NODE == 0 {
            if b1 & MIXED == 0 {
                return self.expand(level + 1, x >> 1, y >> 1, g, parent, dir);
            }
            let (x2, y2, l) = (x.wrapping_mul(2), y.wrapping_mul(2), level - 1);
            if dir & 3 != 0
                && let Some(n) = self.expand(l, x2, y2, g, parent, dir)
            {
                return Some(n);
            }
            if dir & 5 != 0
                && let Some(n) = self.expand(l, x2 + 1, y2, g, parent, dir)
            {
                return Some(n);
            }
            if dir & 0b1010 != 0
                && let Some(n) = self.expand(l, x2, y2 + 1, g, parent, dir)
            {
                return Some(n);
            }
            if dir & 0xc == 0 {
                return None;
            }
            return self.expand(l, x2 + 1, y2 + 1, g, parent, dir);
        }
        if b0 & CLOSED != 0 {
            return None;
        }
        let (cx, cy) = if 0 < level {
            let half = 1i16 << (level - 1);
            ((x << level) + half, (y << level) + half)
        } else {
            (x, y)
        };
        let (g0, [ex, ey], shortcut) = match self.measured_from(parent, dir) {
            (_, _, false) => (g, self.nodes[parent].entry, false),
            found => found,
        };
        let dx = i32::from(ex) - i32::from(cx);
        let dy = i32::from(ey) - i32::from(cy);
        let d = fmath::sqrt((dy * dy + dx * dx) as f32);
        let h = self.estimate_from(cx, cy);
        let n = self.alloc(x, y, level, d + g0, h, Some(parent), dir, [cx, cy]);
        self.open.push(n, &self.nodes);
        self.nodes[n].shortcut = shortcut;
        if self.cells[i][0] & OPEN == 0 {
            self.cells[i][0] += OPEN;
        }
        (self.cells[i][1] & GOAL != 0).then_some(n)
    }

    /// `FUN_140f76840`: the level-0 neighbour across edge `dir`; a turn's cost from the
    /// grandparent counts 1.1 times the grandparent's cost so far.
    fn expand0(&mut self, x: i16, y: i16, g: f32, parent: usize, dir: u8) -> Option<usize> {
        let i = self.index(0, x, y);
        let b0 = self.cells[i][0];
        if dir & b0 == 0 || b0 & CLOSED != 0 {
            return None;
        }
        let (g0, [ex, ey], shortcut) = match self.measured_from(parent, dir) {
            (gp, entry, true) => (gp * SHORTCUT_COST, entry, true),
            _ => (g, self.nodes[parent].entry, false),
        };
        let dx = i32::from(ex) - i32::from(x);
        let dy = i32::from(ey) - i32::from(y);
        let d = fmath::sqrt((dy * dy + dx * dx) as f32);
        let h = self.estimate_from(x, y);
        let n = self.alloc(x, y, 0, d + g0, h, Some(parent), dir, [x, y]);
        self.open.push(n, &self.nodes);
        self.nodes[n].shortcut = shortcut;
        if self.cells[i][0] & OPEN == 0 {
            self.cells[i][0] += OPEN;
        }
        (self.cells[i][1] & GOAL != 0).then_some(n)
    }

    /// Close a popped node and keep it as the best when its estimate is the lowest yet.
    fn close(&mut self, n: usize) -> Option<u8> {
        let cell = self.nodes[n].cell;
        let b = self.cells[cell][0];
        if b & CLOSED != 0 {
            return None;
        }
        self.cells[cell][0] = b + CLOSED;
        let h = self.nodes[n].f - self.nodes[n].g;
        if h < self.best_cost {
            self.best_cost = h;
            self.best = Some(n);
        }
        Some(b)
    }

    /// `FUN_140f78850`: A* over the node cells from the start's cell at `level`; Some node in the
    /// goal, None when the open list runs dry (or a computer player's search stops early, keeping
    /// the node it stopped at as the best).
    fn search_levels(
        &mut self,
        level: u8,
        lx: i16,
        ly: i16,
        start: [i16; 2],
        full: bool,
    ) -> Option<usize> {
        let [b0, b1] = self.cell(level, lx, ly);
        if b1 & LONE == 0 && b0 == 0 {
            return None;
        }
        let h0 = self.start_estimate(start[0], start[1]);
        let mut count: u32 = 0;
        let n = self.alloc(lx, ly, level, 0.0, h0, None, 0, start);
        let cell = self.nodes[n].cell;
        if self.cells[cell][1] & GOAL != 0 {
            return Some(n);
        }
        self.cells[cell][0] += OPEN;
        self.open.push(n, &self.nodes);
        self.best_cost = NO_COST;
        while !self.open.is_empty() {
            let n = self.open.pop(&self.nodes)?;
            count += 1;
            let Some(b) = self.close(n) else { continue };
            let node = self.nodes[n];
            let h = node.f - node.g;
            if !full && h <= EARLY_LEFT && h0 <= EARLY_START && (h0 as i32 * 10) as u32 <= count {
                self.best = Some(n);
                return None;
            }
            let steps = [(W, -1, 0, E), (E, 1, 0, W), (N, 0, -1, S), (S, 0, 1, N)];
            for (edge, dx, dy, back) in steps {
                if b & edge != 0
                    && let Some(found) =
                        self.expand(node.level, node.x + dx, node.y + dy, node.g, n, back)
                {
                    return Some(found);
                }
            }
        }
        None
    }

    /// `FUN_140f79250`: A* over the tiles from the start's tile.
    fn search_tiles(&mut self, start: [i16; 2]) -> Option<usize> {
        let [x, y] = start;
        if self.cell(0, x, y)[0] == 0 {
            return None;
        }
        let h0 = self.start_estimate(x, y);
        let n = self.alloc(x, y, 0, 0.0, h0, None, 0, start);
        let cell = self.nodes[n].cell;
        if self.cells[cell][1] & GOAL != 0 {
            return Some(n);
        }
        self.open.clear();
        self.cells[cell][0] += OPEN;
        self.open.push(n, &self.nodes);
        self.best_cost = NO_COST;
        while !self.open.is_empty() {
            let n = self.open.pop(&self.nodes)?;
            let Some(b) = self.close(n) else { continue };
            let node = self.nodes[n];
            let steps = [(W, -1, 0, E), (E, 1, 0, W), (N, 0, -1, S), (S, 0, 1, N)];
            for (edge, dx, dy, back) in steps {
                if b & edge != 0
                    && let Some(found) = self.expand0(node.x + dx, node.y + dy, node.g, n, back)
                {
                    return Some(found);
                }
            }
        }
        None
    }

    /// `FUN_140f7ba80`: whether a cell at `level` lies within the goal's ranges of (`x`, `y`): one
    /// of its passable (or lone) tiles, measured from the tile's corner.
    fn in_range(&self, level: u8, cx: i16, cy: i16, goal: Goal) -> bool {
        let Goal { x, y, radii, range } = goal;
        let [rx0, ry0, rx1, ry1] = self.region;
        if !(rx0 >> level <= cx && ry0 >> level <= cy && cx <= rx1 >> level && cy <= ry1 >> level) {
            return false;
        }
        let r = range + HALF;
        let step = 1i16 << level;
        let off = |v: f32, centre: f32| {
            let d = (v - (centre + -HALF)).abs();
            let d = if d <= radii { 0.0 } else { d - radii };
            if d == radii {
                if 0.0 <= d - r { d - r } else { 0.0 }
            } else {
                d
            }
        };
        let (x0, y0) = (cx << level, cy << level);
        for tx in x0..=step - 1 + x0 {
            for ty in y0..=step - 1 + y0 {
                let [b0, b1] = self.cell(0, tx, ty);
                if b0 == 0 && b1 & LONE == 0 {
                    continue;
                }
                let ex = off(f32::from(tx), x);
                if r < ex {
                    continue;
                }
                let ey = off(f32::from(ty), y);
                if ey <= r && ey * ey + ex * ex <= r * r {
                    return true;
                }
            }
        }
        false
    }

    /// The square of tiles the goal's ranges may reach (`FUN_140f7d540`, `FUN_140f7a990`): the
    /// goal ± the radii and the range (+ 1 when the range is below 1); the far sides grow by `far`.
    fn goal_square(&mut self, goal: Goal, far: f32) {
        let Goal { x, y, radii, range } = goal;
        let mut r = radii + range;
        if range < ONE {
            r += ONE;
        }
        let x0 = tile(x - r).max(0);
        let y0 = tile(y - r).max(0);
        let mut x1 = tile(r + x + far);
        let mut y1 = tile(r + y + far);
        if self.size[0] <= x1 {
            x1 = self.size[0] - 1;
        }
        if self.size[1] <= y1 {
            y1 = self.size[1] - 1;
        }
        self.region = [x0, y0, x1, y1];
        self.goal = [tile(x), tile(y)];
    }

    /// Toggle a cell's goal mark; true when it changed.
    fn toggle_goal(&mut self, i: usize, set: bool) -> bool {
        let b1 = self.cells[i][1];
        if set == (b1 & GOAL != 0) {
            return false;
        }
        self.cells[i][1] = if set { b1 + GOAL } else { b1 - GOAL };
        true
    }

    /// `FUN_140f7d540`: mark (or clear) the node cells within the goal's ranges; when none is, the
    /// coarsest node cell holding the goal's tile.
    fn mark_goal(&mut self, goal: Goal, set: bool) {
        self.goal_square(goal, FAR_SIDE);
        self.goal_marked = false;
        let [x0, y0, x1, y1] = self.region;
        let mut marked = 0;
        for cx in (x0 >> 3)..=(x1 >> 3) {
            for cy in (y0 >> 3)..=(y1 >> 3) {
                marked += self.mark_cells(3, cx, cy, goal, set);
            }
        }
        if marked != 0 {
            self.goal_marked = true;
            return;
        }
        let Some(i) = (0..=3u8).rev().find_map(|l| {
            let i = self.index(l, self.goal[0] >> l, self.goal[1] >> l);
            (self.cells[i][1] & NODE != 0).then_some(i)
        }) else {
            return;
        };
        if self.toggle_goal(i, set) && set {
            self.goal_marked = true;
        }
    }

    /// `FUN_140f7d7a0`: the node cells under a cell within the goal's ranges, marked or cleared;
    /// returns how many changed.
    fn mark_cells(&mut self, level: u8, cx: i16, cy: i16, goal: Goal, set: bool) -> i32 {
        let [rx0, ry0, rx1, ry1] = self.region;
        if !(rx0 >> level <= cx && ry0 >> level <= cy && cx <= rx1 >> level && cy <= ry1 >> level) {
            return 0;
        }
        let i = self.index(level, cx, cy);
        if self.cells[i][1] & NODE == 0 {
            if level == 0 {
                return 0;
            }
            let (x2, y2, l) = (cx.wrapping_mul(2), cy.wrapping_mul(2), level - 1);
            return [(x2, y2), (x2 + 1, y2), (x2, y2 + 1), (x2 + 1, y2 + 1)]
                .into_iter()
                .map(|(a, b)| self.mark_cells(l, a, b, goal, set))
                .sum();
        }
        i32::from(self.in_range(level, cx, cy, goal) && self.toggle_goal(i, set))
    }

    /// `FUN_140f7a990`: mark (or clear) the tiles within the goal's ranges; when none is, the
    /// goal's own tile.
    fn mark_goal_tiles(&mut self, goal: Goal, set: bool) {
        self.goal_square(goal, 0.0);
        let [x0, y0, x1, y1] = self.region;
        let mut changed = false;
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                if self.in_range(0, tx, ty, goal) {
                    let i = self.index(0, tx, ty);
                    changed |= self.toggle_goal(i, set);
                }
            }
        }
        if !changed {
            let i = self.index(0, self.goal[0], self.goal[1]);
            let _ = self.toggle_goal(i, set);
        }
    }

    /// `FUN_140f7be80`: close (or reopen) the node cells over a rectangle of tiles
    /// (`FUN_140f7e070`).
    fn close_rect(&mut self, [x0, y0, x1, y1]: [i16; 4], set: bool) {
        let (lo_x, hi_x) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
        let (lo_y, hi_y) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
        let rx0 = lo_x.max(0);
        let ry0 = lo_y.max(0);
        let rx1 = if self.size[0] <= hi_x {
            self.size[0] - 1
        } else {
            hi_x
        };
        let ry1 = if self.size[1] <= hi_y {
            self.size[1] - 1
        } else {
            hi_y
        };
        self.region = [rx0, ry0, rx1, ry1];
        for cx in (rx0 >> 3)..=(rx1 >> 3) {
            for cy in (ry0 >> 3)..=(ry1 >> 3) {
                self.close_cells(3, cx, cy, set);
            }
        }
    }

    /// `FUN_140f7e070`.
    fn close_cells(&mut self, mut level: u8, mut x: i16, mut y: i16, set: bool) {
        while self.cell(level, x, y)[1] & NODE == 0 {
            if level == 0 {
                return;
            }
            level -= 1;
            x = x.wrapping_mul(2);
            let y2 = y.wrapping_mul(2);
            self.close_cells(level, x, y2, set);
            self.close_cells(level, x + 1, y2, set);
            self.close_cells(level, x + 1, y2 + 1, set);
            y = y2 + 1;
        }
        let [rx0, ry0, rx1, ry1] = self.region;
        if rx0 >> level <= x && x <= rx1 >> level && ry0 >> level <= y && y <= ry1 >> level {
            let i = self.index(level, x, y);
            let b0 = self.cells[i][0];
            if set != (b0 & CLOSED != 0) {
                self.cells[i][0] = if set { b0 + CLOSED } else { b0 - CLOSED };
            }
        }
    }

    /// `FUN_140f7b9e0`: the steps from a child cell to the goal's tile at that level, counted
    /// only where the goal lies right of or below it; `0xffff` for a cell off the map or without
    /// edges.
    fn steps_to(&self, tx: i16, ty: i16, level: u8, cx: i16, cy: i16) -> u16 {
        let dy = (ty - cy).max(0);
        let dx = (tx - cx).max(0);
        let [b0, b1] = self.cell(level, cx, cy);
        if b1 & 0xf != 0xf && b0 != 0 {
            (dy + dx) as u16
        } else {
            0xffff
        }
    }

    /// `FUN_140f77d00`: the tile of a node cell nearest the goal's tile, child by child down to
    /// level 0 (ties to the earlier child).
    fn tile_toward(&self, n: usize, gx: i16, gy: i16) -> (i16, i16) {
        let node = self.nodes[n];
        let (mut x, mut y) = (node.x, node.y);
        for l in (1..=node.level).rev() {
            let cl = l - 1;
            let (x2, y2) = (x.wrapping_mul(2), y.wrapping_mul(2));
            let (tx, ty) = (gx >> (l - 1), gy >> (l - 1));
            let d = [
                self.steps_to(tx, ty, cl, x2, y2),
                self.steps_to(tx, ty, cl, x2, y2 + 1),
                self.steps_to(tx, ty, cl, x2 + 1, y2),
                self.steps_to(tx, ty, cl, x2 + 1, y2 + 1),
            ];
            let mut best = if d[0] < 9999 { d[0] } else { 9999 };
            let mut which: i32 = if 9998 < d[0] { -1 } else { 0 };
            for (k, &dk) in d.iter().enumerate().skip(1).take(2) {
                if -1 < dk as i16 && (dk as i16) < best as i16 {
                    which = k as i32;
                    best = dk;
                }
            }
            (x, y) = if -1 < d[3] as i16 && (d[3] as i16) < best as i16 {
                (x2 + 1, y2 + 1)
            } else {
                match which {
                    1 => (x2, y2 + 1),
                    2 => (x2 + 1, y2),
                    _ => (x2, y2),
                }
            };
        }
        (x, y)
    }

    /// The scan along a boundary for the tile nearest `at` that has edge `dir`: first from `at` up
    /// to `hi`, then down to `lo` while nearer than the one found (`FUN_140f775a0`,
    /// `FUN_140f794f0`).
    fn nearest_on_boundary(
        &self,
        at: i16,
        lo: i16,
        hi: i16,
        dir: u8,
        tile_at: impl Fn(i16) -> [u8; 2],
    ) -> Option<i16> {
        let mut off: i16 = 9999;
        let mut found = None;
        let mut s = at;
        while s <= hi {
            if tile_at(s)[0] & dir != 0 {
                off = s - at;
                found = Some(s);
                break;
            }
            s += 1;
        }
        let mut s = at;
        while lo <= s && 0 < off {
            if tile_at(s)[0] & dir != 0 {
                return Some(s);
            }
            s -= 1;
            off -= 1;
        }
        found
    }

    /// `FUN_140f775a0`: the tile where a node's path crosses into its parent: on the node's edge
    /// toward the parent, nearest the middle of the smaller of the two cells.
    fn crossing(&self, n: usize) -> Option<(i16, i16)> {
        let node = self.nodes[n];
        let parent = self.nodes[node.parent?];
        let (x, y, l) = if parent.level < node.level {
            (parent.x, parent.y, parent.level)
        } else {
            (node.x, node.y, node.level)
        };
        let dir = node.dir;
        let span = |v: i16| {
            let lo = v << l;
            if 0 < l {
                (lo, (1 << (l - 1)) + lo, (1 << l) - 1 + lo)
            } else {
                (lo, lo, lo)
            }
        };
        let (mut ox, mut oy) = match dir {
            N | S => {
                let (lo, mid, hi) = span(x);
                let row = if dir == N {
                    node.y << node.level
                } else {
                    ((node.y + 1) << node.level) - 1
                };
                let x = if 0 < l {
                    self.nearest_on_boundary(mid, lo, hi, dir, |s| self.cell(0, s, row))
                        .unwrap_or(mid)
                } else {
                    mid
                };
                (x, row)
            }
            W | E => {
                let (lo, mid, hi) = span(y);
                let col = if dir == W {
                    node.x << node.level
                } else {
                    ((node.x + 1) << node.level) - 1
                };
                let y = if 0 < l {
                    self.nearest_on_boundary(mid, lo, hi, dir, |s| self.cell(0, col, s))
                        .unwrap_or(mid)
                } else {
                    mid
                };
                (col, y)
            }
            _ => return None,
        };
        if self.size[0] - 1 < ox {
            ox = self.size[0] - 1;
        }
        if self.size[1] - 1 < oy {
            oy = self.size[1] - 1;
        }
        Some((ox, oy))
    }

    /// `FUN_140f794f0`: the tile where the path from the last point (`lx`, `ly`) toward the
    /// look-ahead (`tx`, `ty`) crosses a node's edge into its parent, moved along the edge to the
    /// nearest tile with that edge; -1 along the edge when none has it.
    fn aim(&self, n: usize, tx: i16, ty: i16, lx: i16, ly: i16) -> Option<(i16, i16)> {
        let node = self.nodes[n];
        let parent = self.nodes[node.parent?];
        let (bx, by, bl) = if parent.level < node.level {
            (parent.x, parent.y, parent.level)
        } else {
            (node.x, node.y, node.level)
        };
        let dir = node.dir;
        let (dxl, dyl) = (tx.wrapping_sub(lx), ty.wrapping_sub(ly));
        // where the line meets the edge, `d` tiles from the last point along the other axis
        let along = |base: i16, d: i16, num: i16, den: i16| {
            if d == 0 || den == 0 {
                base
            } else {
                (i32::from(base) + (i32::from(d) * i32::from(num)) / i32::from(den)) as i16
            }
        };
        let (lo, hi, at, fixed) = if dir == N || dir == S {
            let lo = bx << bl;
            let hi = ((bx + 1) << bl) - 1;
            let bottom = ((node.y + 1) << node.level) - 1;
            let top = node.y << node.level;
            let hi = hi.min(self.size[0] - 1);
            let bottom = bottom.min(self.size[1] - 1);
            if dir == N {
                let d = if top <= ly { top - ly } else { 0 };
                (lo, hi, along(lx, d, dxl, dyl), top)
            } else if ly <= bottom {
                (lo, hi, along(lx, bottom - ly, dxl, dyl), bottom)
            } else {
                (lo, hi, lx, bottom)
            }
        } else {
            let left = node.x << node.level;
            let right = ((node.x + 1) << node.level) - 1;
            let lo = by << bl;
            let hi = ((by + 1) << bl) - 1;
            if dir == W {
                let d = if left <= lx { left - lx } else { 0 };
                (lo, hi, along(ly, d, dyl, dxl), left)
            } else if lx <= right {
                (lo, hi, along(ly, right - lx, dyl, dxl), right)
            } else {
                (lo, hi, ly, right)
            }
        };
        let passes = |s: i16| {
            let c = if dir == N || dir == S {
                self.cell(0, s, fixed)
            } else {
                self.cell(0, fixed, s)
            };
            c[0] & dir != 0
        };
        let result = if lo < at {
            if at < hi {
                let mut off: i16 = 9999;
                let mut found = -1;
                let mut s = at;
                while s <= hi {
                    if passes(s) {
                        off = s - at;
                        found = s;
                        break;
                    }
                    s += 1;
                }
                let mut cur = at;
                loop {
                    if off < 1 {
                        break found;
                    }
                    if passes(cur) {
                        break cur;
                    }
                    cur -= 1;
                    off -= 1;
                    if cur < lo {
                        break found;
                    }
                }
            } else {
                (lo..=hi).rev().find(|&s| passes(s)).unwrap_or(-1)
            }
        } else {
            (lo..=hi).find(|&s| passes(s)).unwrap_or(-1)
        };
        Some(if dir == N || dir == S {
            (result, fixed)
        } else {
            (fixed, result)
        })
    }

    const fn on_map(&self, x: i16, y: i16) -> bool {
        0 <= x && x < self.size[0] && 0 <= y && y < self.size[1]
    }

    /// A tile's centre as a path point.
    fn centre(x: i16, y: i16) -> Vec3 {
        [f32::from(x) + HALF, 0.0, f32::from(y) + HALF]
    }

    /// The path's points from the goal's node back to the start (the loops of `FUN_140f77f20` and
    /// `FUN_140f78ba0`): per node the tile where the path enters its parent, aimed from the last
    /// point toward the crossing two nodes on (the start's tile when there is none).
    fn trace_back(&mut self, n: usize, start: [i16; 2]) {
        let mut node = n;
        loop {
            let parent = self.nodes[node].parent;
            let (tx, ty) = parent
                .and_then(|p| self.nodes[p].parent)
                .and_then(|gp| self.crossing(gp))
                .unwrap_or_else(|| start.into());
            if let Some((ox, oy)) = self.aim(node, tx, ty, self.last[0], self.last[1]) {
                self.points.push(Self::centre(ox, oy));
                self.last = [ox, oy];
            }
            match parent {
                Some(p) => node = p,
                None => break,
            }
        }
    }

    /// The searched cells' flags cleared and the pool emptied, after the goal's marks.
    fn clear_nodes(&mut self) {
        for node in &self.nodes {
            self.cells[node.cell][0] &= 0xf;
        }
        self.nodes.clear();
    }

    /// `FUN_140f77f20`: the search over the levels: from the coarsest node cell that holds the
    /// start, toward the node cells within the goal's ranges; the path enters each node cell at
    /// the tile nearest the line toward the next crossing.
    pub fn search_over_levels(&mut self, req: &Request, out: &mut Path) -> Found {
        for &r in &req.closed {
            self.close_rect(r, true);
        }
        let (sx, sy) = (tile(req.from[0]), tile(req.from[1]));
        let (gx, gy) = (tile(req.to[0]), tile(req.to[1]));
        if !self.on_map(sx, sy) || !self.on_map(gx, gy) {
            return Found::OffMap;
        }
        self.open.clear();
        self.mark_goal(req.goal(), true);
        self.best = None;
        let mut found = (None, Found::Nearest);
        for level in (0..=3u8).rev() {
            let (lx, ly) = (sx >> level, sy >> level);
            if level == 0 || self.cell(level, lx, ly)[1] & NODE != 0 {
                found = match self.search_levels(level, lx, ly, [sx, sy], req.full) {
                    None => (self.best, Found::Nearest),
                    Some(n) if self.goal_marked => (Some(n), Found::Goal),
                    Some(n) => (Some(n), Found::Nearest),
                };
                break;
            }
        }
        out.points.clear();
        self.points.clear();
        let result = match found {
            (None, _) => Found::Nothing,
            (Some(n), result) => {
                self.last = [gx, gy];
                let mut end = [req.to[0], 0.0, req.to[1]];
                let toward = result != Found::Goal || self.cell(0, gx, gy)[0] == 0;
                if toward {
                    let (cx, cy) = self.tile_toward(n, gx, gy);
                    let dy = f32::from(gy) - f32::from(cy);
                    let dx = f32::from(gx) - f32::from(cx);
                    if result != Found::Goal
                        || req.radii + GOAL_GAP < fmath::sqrt(dy * dy + dx * dx)
                    {
                        end = Self::centre(cx, cy);
                    }
                }
                self.points.push(end);
                self.trace_back(n, [sx, sy]);
                self.points.push([req.from[0], 0.0, req.from[1]]);
                result
            }
        };
        for &p in self.points.iter().rev() {
            let _ = out.push(p);
        }
        self.mark_goal(req.goal(), false);
        self.clear_nodes();
        for &r in &req.closed {
            self.close_rect(r, false);
        }
        result
    }

    /// `FUN_140f7d160`: the tile or one of its four neighbours has an edge.
    fn near_passable(&self, x: i16, y: i16) -> bool {
        let [mx, my] = [self.size[0] - 1, self.size[1] - 1];
        let edges =
            |x: i16, y: i16| 0 <= x && x <= mx && 0 <= y && y <= my && self.cell(0, x, y)[0] != 0;
        edges(x, y) || edges(x + 1, y) || edges(x, y + 1) || edges(x - 1, y) || edges(x, y - 1)
    }

    /// The planner's entry (`FUN_140f7c530`): the search over the levels; for a human player's
    /// path to the goal under 35 tiles between passable tiles also the search on the tiles, else
    /// a shorter first stretch to the last point within 10 tiles; both smoothed (`blocked`, the
    /// obstruction manager's segment test for the unit, None without a unit); the shorter, or
    /// the one with the better result, appended to `out`. `target_box` is the target's flag the
    /// game reads at `+0x658` (a goal tile without edges then counts if a neighbour has some).
    pub fn plan<E>(
        &mut self,
        req: &Request,
        target_box: bool,
        out: &mut Path,
        walls: Option<&mut dyn SegmentTest<E>>,
    ) -> Result<Found, E> {
        let (w, h) = (f32::from(self.size[0]), f32::from(self.size[1]));
        if req.to[0] < 0.0 || w <= req.to[0] || req.to[1] < 0.0 || h <= req.to[1] {
            return Ok(Found::OffMap);
        }
        let mut levels = Path::default();
        let mut tiles = Path::default();
        let first = self.search_over_levels(req, &mut levels);
        let mut second = None;
        let (gx, gy) = (tile(req.to[0]), tile(req.to[1]));
        // the start's cell is read only once the search over the levels reached the goal
        if req.range == 0.0
            && req.full
            && first == Found::Goal
            && self.cell(0, tile(req.from[0]), tile(req.from[1]))[0] != 0
            && (self.cell(0, gx, gy)[0] != 0 || (target_box && self.near_passable(gx, gy)))
            && {
                levels.measure();
                levels.length < TILE_SEARCH_LENGTH
            }
        {
            second = Some(self.search_on_tiles(req, &mut tiles));
        }
        if req.full && second != Some(Found::Goal) && !levels.points.is_empty() {
            self.shorter_start(req, &mut levels);
        }
        if let Some(walls) = walls {
            let smoothed = |r: Found| !matches!(r, Found::OffMap | Found::Nothing);
            if smoothed(first) {
                smooth(&mut levels, walls)?;
            }
            if second.is_some_and(smoothed) {
                smooth(&mut tiles, walls)?;
            }
        }
        levels.measure();
        tiles.measure();
        let second_code = second.map_or(0, Found::code);
        let (chosen, result) = if second_code != first.code() || levels.length < tiles.length {
            (&levels, first)
        } else {
            (&tiles, second.unwrap_or(first))
        };
        out.append(chosen);
        Ok(result)
    }

    /// `FUN_140f7c390`: the planner's path from `from` to `to` ([`Planner::plan`] with no radii,
    /// no range, no closed tiles, no tile search, no target and no smoothing, in a path the
    /// planner's pool lends, `+0x238` and `+0x278`); when it found one (a result other than 0),
    /// the last of its points whose tile has an edge (byte 0 of its level-0 cell, the tile of the
    /// point's `(short)(int)` coordinates), searched from the end; None without one.
    pub fn last_open_point(&mut self, from: [f32; 2], to: [f32; 2]) -> Option<[f32; 2]> {
        let req = Request {
            from,
            to,
            radii: 0.0,
            range: 0.0,
            closed: Vec::new(),
            full: false,
        };
        let mut path = Path::default();
        let found = self
            .plan::<std::convert::Infallible>(&req, false, &mut path, None)
            .unwrap_or_else(|never| match never {});
        if found == Found::OffMap {
            return None;
        }
        path.points
            .iter()
            .rev()
            .find(|p| self.cell(0, tile(p[0]), tile(p[2]))[0] != 0)
            .map(|p| [p[0], p[2]])
    }

    /// `FUN_140f7c530`'s shortcut: the last point of the path within 10 tiles of its start (past
    /// the second, short of the end) is searched for on the tiles; when that reaches it, the path
    /// becomes that search's path and the rest after the point.
    fn shorter_start(&mut self, req: &Request, levels: &mut Path) {
        let n = levels.points.len();
        let p0 = levels.points[0];
        let mut i = 1;
        if n <= 1 {
            return;
        }
        loop {
            let pi = levels.points[i];
            let d = (p0[1] - pi[1]) * (p0[1] - pi[1])
                + (p0[0] - pi[0]) * (p0[0] - pi[0])
                + (p0[2] - pi[2]) * (p0[2] - pi[2]);
            if SHORTCUT_REACH < d {
                i -= 1;
                break;
            }
            i += 1;
            if n <= i {
                break;
            }
        }
        if i <= 1 || i == n {
            return;
        }
        let pt = levels.points[i];
        let mut shortcut = Path::default();
        let to_point = Request {
            to: [pt[0], pt[2]],
            ..req.clone()
        };
        if self.search_on_tiles(&to_point, &mut shortcut) == Found::Goal {
            for &p in &levels.points[i + 1..] {
                let _ = shortcut.push(p);
            }
            // the game empties the path before it appends the shortcut (0x140f7c940)
            levels.points.clear();
            levels.append(&shortcut);
        }
    }

    /// `FUN_140f78ba0`: the search on level 0, tile by tile, toward the tiles within the goal's
    /// ranges.
    pub fn search_on_tiles(&mut self, req: &Request, out: &mut Path) -> Found {
        for &r in &req.closed {
            self.close_rect(r, true);
        }
        let (sx, sy) = (tile(req.from[0]), tile(req.from[1]));
        let (gx, gy) = (tile(req.to[0]), tile(req.to[1]));
        if !self.on_map(sx, sy) || !self.on_map(gx, gy) {
            return Found::OffMap;
        }
        self.mark_goal_tiles(req.goal(), true);
        self.best = None;
        let found = match self.search_tiles([sx, sy]) {
            None => (self.best, Found::Nearest),
            Some(n) => (Some(n), Found::Goal),
        };
        out.points.clear();
        self.points.clear();
        let result = match found {
            (None, _) => Found::Nothing,
            (Some(n), result) => {
                self.last = [gx, gy];
                let end = if result == Found::Goal && self.cell(0, gx, gy)[0] != 0 {
                    [req.to[0], 0.0, req.to[1]]
                } else {
                    let (cx, cy) = self.tile_toward(n, gx, gy);
                    Self::centre(cx, cy)
                };
                self.points.push(end);
                self.trace_back(n, [sx, sy]);
                self.points.push([req.from[0], 0.0, req.from[1]]);
                result
            }
        };
        for &p in self.points.iter().rev() {
            let _ = out.push(p);
        }
        self.mark_goal_tiles(req.goal(), false);
        self.clear_nodes();
        for &r in &req.closed {
            self.close_rect(r, false);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_search_leaves_the_grid_as_it_was() {
        let terrain = vec![0u8; 64];
        let open = |_: i32, _: i32| false;
        let built = Planner::new([8, 8], terrain.clone(), vec![1.0], &open);
        let mut searched = built.clone();
        searched.goal = [3, 4];
        searched.goal_marked = true;
        assert!(searched != built && searched.same_grid(&built));
        let other = Planner::new([8, 8], terrain, vec![0.0], &open);
        assert!(!other.same_grid(&built));
    }

    fn open_map(_: i32, _: i32) -> bool {
        false
    }

    /// `FUN_140f7c530` reads the start's cell only after the search over the levels found the
    /// goal: a start off the map ends the search there, without the tile search.
    #[test]
    fn a_plan_from_off_the_map_reads_no_start_cell() {
        let mut p = Planner::new([16, 8], vec![0; 128], vec![1.0], &open_map);
        let req = Request {
            from: [-3.0, 2.0],
            to: [5.5, 2.5],
            radii: 0.2,
            range: 0.0,
            closed: Vec::new(),
            full: true,
        };
        let mut out = Path::default();
        let found = p.plan::<()>(&req, false, &mut out, None).unwrap();
        assert_eq!(found, Found::OffMap);
    }

    #[test]
    fn the_last_open_point_walks_the_path_back_to_a_tile_with_an_edge() {
        // water (terrain 0) left of x = 10, land (1, impassable) right of it
        let terrain: Vec<u8> = (0..128).map(|i| u8::from(10 <= i % 16)).collect();
        let mut p = Planner::new([16, 8], terrain, vec![1.0, 0.0], &open_map);
        // a goal on the water: the path's end
        assert_eq!(p.last_open_point([2.5, 4.5], [7.5, 4.5]), Some([7.5, 4.5]));
        // ashore: the path ends at the node nearest the goal, the last water tile
        assert_eq!(p.last_open_point([2.5, 4.5], [13.5, 4.5]), Some([9.5, 4.5]));
        // a goal off the map (result 0): none
        assert_eq!(p.last_open_point([2.5, 4.5], [16.5, 4.5]), None);
    }

    /// The scan along a boundary for the tile nearest `at` with the edge: the first upward, then
    /// downward while strictly nearer; neither: none.
    #[test]
    fn the_nearest_tile_with_the_edge_is_found_up_then_nearer_down() {
        let p = Planner::new([8, 8], vec![0; 64], vec![1.0], &open_map);
        let edges = |bits: [u8; 8]| move |s: i16| [bits[s as usize], 0];
        // above at 7 (2 off), below at 4 (1 off): the nearer below
        assert_eq!(
            p.nearest_on_boundary(5, 0, 7, N, edges([0, 0, 0, 0, N, 0, 0, N])),
            Some(4)
        );
        // one off either way: a tie goes up
        assert_eq!(
            p.nearest_on_boundary(5, 0, 7, N, edges([0, 0, 0, 0, N, 0, N, 0])),
            Some(6)
        );
        // above at 6 only
        assert_eq!(
            p.nearest_on_boundary(5, 0, 7, N, edges([0, 0, 0, 0, 0, 0, N, 0])),
            Some(6)
        );
        // below at 1 only, beyond nothing found above
        assert_eq!(
            p.nearest_on_boundary(5, 0, 7, N, edges([0, N, 0, 0, 0, 0, 0, 0])),
            Some(1)
        );
        // another edge only: none
        assert_eq!(p.nearest_on_boundary(5, 0, 7, N, edges([S; 8])), None);
    }

    /// `FUN_140f78ba0` on an open map: the tiles' A* (`FUN_140f79250`, its neighbours by
    /// `FUN_140f76840`) reaches the goal, and the path runs from the start to the goal point.
    #[test]
    fn the_tile_search_reaches_a_goal_on_an_open_map() {
        let mut p = Planner::new([16, 8], vec![0; 128], vec![1.0], &open_map);
        let req = Request {
            from: [1.5, 2.5],
            to: [6.5, 5.5],
            radii: 0.2,
            range: 0.0,
            closed: Vec::new(),
            full: true,
        };
        let mut out = Path::default();
        assert_eq!(p.search_on_tiles(&req, &mut out), Found::Goal);
        assert_eq!(out.points.first(), Some(&[1.5, 0.0, 2.5]));
        assert_eq!(out.points.last(), Some(&[6.5, 0.0, 5.5]));
        // the search leaves its marks behind: the same search again finds the same path
        let mut again = Path::default();
        assert_eq!(p.search_on_tiles(&req, &mut again), Found::Goal);
        assert_eq!(again.points, out.points);
    }

    /// A wall across the map: the tile search ends at the tile nearest the goal (its centre),
    /// on the start's side.
    #[test]
    fn the_tile_search_ends_nearest_the_goal_behind_a_wall() {
        let wall = |x: i32, _: i32| x == 3;
        let mut p = Planner::new([8, 8], vec![0; 64], vec![1.0], &wall);
        let req = Request {
            from: [1.5, 2.5],
            to: [6.5, 2.5],
            radii: 0.2,
            range: 0.0,
            closed: Vec::new(),
            full: true,
        };
        let mut out = Path::default();
        assert_eq!(p.search_on_tiles(&req, &mut out), Found::Nearest);
        assert_eq!(out.points.first(), Some(&[1.5, 0.0, 2.5]));
        assert_eq!(out.points.last(), Some(&[2.5, 0.0, 2.5]));
    }

    /// `FUN_140f7c530` for a human player's search (`full`): a short path over the levels is
    /// searched again on the tiles; a long one has its start shortened over the tiles
    /// (`shorter_start`); either way the path runs from the start to the goal.
    #[test]
    fn a_full_plan_searches_the_tiles_near_and_shortens_the_start_far() {
        let mut p = Planner::new([64, 8], vec![0; 512], vec![1.0], &open_map);
        for to in [[6.5, 5.5], [60.5, 5.5]] {
            let req = Request {
                from: [1.5, 2.5],
                to,
                radii: 0.2,
                range: 0.0,
                closed: Vec::new(),
                full: true,
            };
            let mut out = Path::default();
            assert_eq!(p.plan::<()>(&req, false, &mut out, None), Ok(Found::Goal));
            assert_eq!(out.points.first(), Some(&[1.5, 0.0, 2.5]));
            assert_eq!(out.points.last(), Some(&[to[0], 0.0, to[1]]));
        }
    }

    #[test]
    fn an_open_map_is_one_node_cell_per_eight_tiles() {
        let p = Planner::new([16, 8], vec![0; 128], vec![1.0], &open_map);
        assert_eq!(p.level_sizes(), ([16, 8, 4, 2], [8, 4, 2, 1]));
        // level 3: two uniform node cells, joined across their shared edge only
        assert_eq!(p.level(3), &[[E, NODE], [W, NODE]]);
        // an inner tile has all four edges; a corner tile two
        let tile = |x: usize, y: usize| p.level(0)[y * 16 + x];
        assert_eq!(tile(3, 3), [N | W | E | S, 0]);
        assert_eq!(tile(0, 0), [E | S, 0]);
    }

    #[test]
    fn padding_tiles_of_a_map_that_is_not_a_multiple_of_eight_lie_off_the_map() {
        let p = Planner::new([6, 6], vec![0; 36], vec![1.0], &open_map);
        assert_eq!(p.level_sizes(), ([8, 4, 2, 1], [8, 4, 2, 1]));
        let tile = |x: usize, y: usize| p.level(0)[y * 8 + x];
        assert_eq!(tile(6, 0), [0, 0xf]);
        assert_eq!(tile(5, 5), [N | W, 0]);
        // the level-1 cell (3, 0) covers tiles 6 and 7: off the map, both of its children
        assert_eq!(p.level(1)[3][1] & 0xf, 0b1111);
    }

    #[test]
    fn a_wall_splits_its_level_three_cell_into_its_children() {
        // a blocked column at x = 3 cuts the first 8 × 8 cell in two
        let wall = |x: i32, _: i32| x == 3;
        let p = Planner::new([8, 8], vec![0; 64], vec![1.0], &wall);
        assert_eq!(p.level(3)[0][1] & (NODE | MIXED), MIXED);
        assert_eq!(p.level(0)[3], [0, 0]);
        assert_eq!(p.level(0)[2][0], W | S);
    }

    #[test]
    fn a_path_skips_a_point_equal_to_its_neighbour() {
        let mut path = Path::default();
        assert!(path.push([1.0, 0.0, 2.0]));
        assert!(!path.push([1.0, 0.0, 2.0]));
        assert!(path.push([4.0, 0.0, 6.0]));
        path.measure();
        assert_eq!(path.length, 5.0);
    }

    /// `FUN_140f761b0` with fewer than one point between `first` and `last`: no change, 0 fewer.
    #[test]
    fn a_stretch_without_points_between_is_left_as_it_is() {
        let mut path = Path {
            points: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [30.0, 0.0, 0.0]],
            length: 0.0,
        };
        let before = path.points.clone();
        assert_eq!(straighten(&mut path, 0, 1), 0);
        assert_eq!(straighten(&mut path, 1, 1), 0);
        assert_eq!(path.points, before);
        // two apart: one point between, the line 30 long takes four points at 7.5 apart: 3 more
        assert_eq!(straighten(&mut path, 0, 2), -3);
    }

    /// MarvinBlue/426048731's 9520 in update 758 (the probe's log): the search's twelve tile
    /// points become three where the walk is blocked at (103.5, 46.5) and beyond.
    #[test]
    fn the_smoothing_keeps_the_points_before_a_blocked_walk() {
        let points = [
            [109.5, 42.5],
            [108.5, 42.5],
            [108.5, 43.5],
            [107.5, 43.5],
            [107.5, 44.5],
            [106.5, 44.5],
            [105.5, 44.5],
            [104.5, 44.5],
            [103.5, 44.5],
            [103.5, 45.5],
            [103.5, 46.5],
            [103.604_164, 46.854_168],
        ];
        let mut path = Path {
            points: points.iter().map(|&[x, y]| [x, 0.0, y]).collect(),
            length: 0.0,
        };
        // blocked: any walk from the start that ends beyond y = 44.5
        let mut walls = Walls::default();
        smooth(&mut path, &mut walls).unwrap();
        let ends: Vec<[f32; 2]> = path.points.iter().map(|p| [p[0], p[2]]).collect();
        assert_eq!(
            ends,
            [[109.5, 42.5], [103.5, 44.5], [103.604_164, 46.854_168]]
        );
        assert_eq!(walls.passes, 1);
        // a path of two points tests nothing and ends no pass
        let mut short = Path {
            points: vec![[0.0, 0.0, 0.0], [5.0, 0.0, 5.0]],
            length: 0.0,
        };
        smooth(&mut short, &mut walls).unwrap();
        assert_eq!((walls.tests, walls.passes), (10, 1));
    }

    /// The smoothing test's stand-in: any walk from (109.5, 42.5) that ends beyond y = 44.5 is
    /// blocked; it counts the tests and the passes that ended.
    #[derive(Default)]
    struct Walls {
        tests: usize,
        passes: usize,
    }

    impl SegmentTest<()> for Walls {
        fn blocked(&mut self, a: [f32; 2], b: [f32; 2]) -> Result<bool, ()> {
            self.tests += 1;
            Ok(a == [109.5, 42.5] && 44.5 < b[1])
        }

        fn pass_ends(&mut self) {
            self.passes += 1;
        }
    }

    /// `FUN_140f7be80` orders each pair of corners, holds the rectangle on the map (its low
    /// corner at 0, its high one at `+0x404`/`+0x406`, the size less one) and closes, from each
    /// level-3 cell over it, the node cells under it within the rectangle at their level
    /// (`FUN_140f7e070`): a mixed cell's children in the order (2x, 2y), (2x + 1, 2y),
    /// (2x + 1, 2y + 1), (2x, 2y + 1). Closing a closed cell or opening an open one changes nothing.
    /// `FUN_140f95c00` appends each point through `FUN_140f95060` at the end (a point equal to
    /// the last is left out), then takes the source's length or, at most 1e-7, measures anew:
    /// `COMISS` with `JBE` (0x140f95d66), so an unordered (NaN) length measures too.
    #[test]
    fn an_appended_path_takes_the_source_s_length_or_measures_a_short_or_nan_one() {
        let mut path = Path::default();
        let _ = path.push([0.0, 0.0, 0.0]);
        let other = Path {
            points: vec![[0.0, 0.0, 0.0], [3.0, 0.0, 4.0]],
            length: 9.0,
        };
        path.append(&other);
        assert_eq!(path.points, [[0.0, 0.0, 0.0], [3.0, 0.0, 4.0]]);
        assert_eq!(path.length, 9.0);
        for short in [1e-7, 0.0, f32::NAN] {
            let mut path = Path::default();
            path.append(&Path {
                length: short,
                ..other.clone()
            });
            assert_eq!(path.length, 5.0, "{short}");
        }
    }

    #[test]
    fn a_closed_rectangle_closes_the_node_cells_under_it() {
        // tile (0, 0) has no passable neighbour: its level-3, 2 and 1 cells are mixed, and the
        // four tiles of the level-1 cell are nodes
        let walls = |x: i32, y: i32| (x, y) == (1, 0) || (x, y) == (0, 1);
        let mut p = Planner::new([16, 8], vec![0; 128], vec![1.0], &walls);
        let open = p.clone();
        assert_eq!(p.level(1)[0][1] & (NODE | MIXED), MIXED);
        assert_eq!(p.level(0)[0][1] & NODE, NODE);
        // the corners swapped and the low y off the map
        p.close_rect([5, 3, 1, -2], true);
        assert_eq!(p.region, [1, 0, 5, 3]);
        let closed = |p: &Planner, l: usize, x: usize, y: usize| {
            let w = p.level_sizes().0[l] as usize;
            p.level(l)[y * w + x][0] & CLOSED != 0
        };
        let expected: [(usize, usize, usize, bool); 14] = [
            (0, 0, 0, false),
            (0, 1, 0, true),
            (0, 1, 1, true),
            (0, 0, 1, false),
            (1, 0, 0, false),
            (1, 1, 0, true),
            (1, 1, 1, true),
            (1, 0, 1, true),
            (2, 0, 0, false),
            (2, 1, 0, true),
            (2, 1, 1, false),
            (2, 0, 1, false),
            (3, 0, 0, false),
            (3, 1, 0, false),
        ];
        for (l, x, y, c) in expected {
            assert_eq!(closed(&p, l, x, y), c, "level {l} ({x}, {y})");
        }
        // the closed bit is added once
        let once = p.clone();
        p.close_rect([1, 0, 5, 3], true);
        assert_eq!(p, once);
        assert_eq!(p.level(0)[1][0], CLOSED);
        // the high corner past the map's size holds at its last tile: everything reopens
        p.close_rect([0, 0, 20, 9], false);
        assert_eq!(p.region, [0, 0, 15, 7]);
        p.region = open.region;
        assert_eq!(p, open);
    }

    #[test]
    fn the_open_list_pops_the_cheapest_and_the_older_of_equal_costs_first() {
        let node = |f: f32| Node {
            cell: 0,
            x: 0,
            y: 0,
            entry: [0, 0],
            f,
            g: 0.0,
            level: 0,
            dir: 0,
            shortcut: false,
            parent: None,
        };
        let nodes = [node(5.5), node(5.25), node(5.25), node(7.0), node(6.5)];
        let mut open = OpenList::default();
        for i in 0..nodes.len() {
            open.push(i, &nodes);
        }
        let order: Vec<usize> = std::iter::from_fn(|| open.pop(&nodes)).collect();
        assert_eq!(order, [1, 2, 0, 4, 3]);
        assert!(open.is_empty());
    }
}
