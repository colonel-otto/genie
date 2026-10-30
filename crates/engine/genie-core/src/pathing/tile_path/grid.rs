//! A terrain restriction's planner ([`Planner`]) and its four-level grid: built, read and
//! rebuilt over a changed rectangle.

use crate::map::GridSize;
use crate::math::fmath::BVector;
use crate::pathing::tile_path::{E, LONE, MIXED, N, NO_COST, NODE, Node, OpenList, S, W};

/// A terrain restriction's planner (`0x4a8` bytes, built by `FUN_140f74f90`).
#[derive(Clone, Debug, PartialEq)]
pub struct Planner {
    /// `+0x400`, `+0x402`: the map's size.
    pub(super) size: [i16; 2],
    /// Per level: `+0x3f0` width, `+0x3f8` height, `+0x3e0` first cell.
    pub(super) widths: [i16; 4],
    pub(super) heights: [i16; 4],
    pub(super) offsets: [i32; 4],
    pub(super) cells: Vec<[u8; 2]>,
    /// `+0x3c0`: the restriction's passability per terrain.
    pub(super) passable: Vec<f32>,
    /// Each tile's terrain (the map's tile `+9`), row by row.
    pub(super) terrain: Vec<u8>,
    /// `+0x408`..`+0x40e`: the tiles an update or the goal's marks cover.
    pub(super) region: [i16; 4],
    /// `+0x410`, `+0x412`: the goal's tile.
    pub(super) goal: [i16; 2],
    /// `+0x414`, `+0x416`: the tile the path was last built to.
    pub(super) last: [i16; 2],
    /// `+0x478`: the goal's marks found a node cell.
    pub(super) goal_marked: bool,
    pub(super) nodes: Vec<Node>,
    pub(super) open: OpenList,
    /// `+0x470` and `+0xb8`: the node nearest the goal and its estimate.
    pub(super) best: Option<usize>,
    pub(super) best_cost: f32,
    /// `+0x448`: the path's points from the goal back.
    pub(super) points: Vec<BVector>,
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
    pub fn new(size: GridSize, terrain: Vec<u8>, passable: Vec<f32>, map: &impl Blocked) -> Self {
        let (w, h) = (size.width() as i16, size.height() as i16);
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

    pub(super) fn index(&self, level: u8, x: i16, y: i16) -> usize {
        let l = usize::from(level);
        let i =
            i64::from(self.widths[l]) * i64::from(y) + i64::from(x) + i64::from(self.offsets[l]);
        usize::try_from(i).expect("a cell outside the planner's grid")
    }

    pub(super) fn cell(&self, level: u8, x: i16, y: i16) -> [u8; 2] {
        self.cells[self.index(level, x, y)]
    }

    /// `FUN_140f7da90`: clear a cell and mark its children that lie off the map; true when all of
    /// it does.
    pub(super) fn init_cell(&mut self, level: u8, x: i16, y: i16) -> bool {
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
    pub(super) fn adjacency(&self, x: i32, y: i32, map: &impl Blocked) -> u32 {
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
    pub(super) fn rebuild(&mut self, level: u8, x: i16, y: i16, map: &impl Blocked) -> [u8; 2] {
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
    pub(super) fn splits(&self, [d0, d1, d2, d3]: [u8; 4], dirs: u8) -> bool {
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
