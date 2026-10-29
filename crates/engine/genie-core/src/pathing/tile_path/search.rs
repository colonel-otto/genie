//! The A* searches over the node cells and over the tiles: the nodes, the open list, the
//! estimates and the expansions.

use crate::pathing::tile_path::{
    CLOSED, E, EARLY_LEFT, EARLY_START, GOAL, LONE, MIXED, N, NO_COST, NODE, OPEN, Planner, S,
    SHORTCUT_COST, W,
};
use crate::{math::fmath, math::msvc};

/// A search node (`0x28` bytes, from the planner's pool `+0x428`, `FUN_140f79c90`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Node {
    /// `+0`: the cell's index.
    pub(super) cell: usize,
    /// `+8`, `+0xa`: the cell at its level.
    pub(super) x: i16,
    pub(super) y: i16,
    /// `+0xc`, `+0xe`: the tile the cost is measured from.
    pub(super) entry: [i16; 2],
    /// `+0x10`: cost so far plus the estimate to the goal; `+0x14`: the cost so far.
    pub(super) f: f32,
    pub(super) g: f32,
    /// `+0x18`.
    pub(super) level: u8,
    /// `+0x19`: the edge back to the parent.
    pub(super) dir: u8,
    /// `+0x1a`: the cost was measured from the grandparent.
    pub(super) shortcut: bool,
    /// `+0x20`.
    pub(super) parent: Option<usize>,
}

/// The open nodes (`+0x480`): a bucket per whole cost, the lowest one sorted with its cheapest
/// node last (`FUN_140f9cba0`, `FUN_140f9cd40`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct OpenList {
    pub(super) buckets: Vec<Vec<usize>>,
    /// `+0x18` with its flag `+0x1c`, `+0x20` with `+0x24`.
    pub(super) min: Option<i32>,
    pub(super) max: Option<i32>,
}

impl OpenList {
    /// `FUN_140f9c9f0`.
    pub(super) fn clear(&mut self) {
        for b in &mut self.buckets {
            b.clear();
        }
        self.min = None;
        self.max = None;
    }

    pub(super) const fn is_empty(&self) -> bool {
        self.min.is_none()
    }

    /// `FUN_140f9cba0`: into the lowest bucket before the first node from the back that costs
    /// more; into a new lowest or a higher bucket at its end.
    pub(super) fn push(&mut self, node: usize, nodes: &[Node]) {
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
    pub(super) fn pop(&mut self, nodes: &[Node]) -> Option<usize> {
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

/// A node from the pool ([`Planner::alloc`]'s arguments, `FUN_140f79c90`'s).
#[derive(Clone, Copy, Debug)]
pub(super) struct NewNode {
    /// The tile's x.
    pub(super) x: i16,
    /// The tile's y.
    pub(super) y: i16,
    /// The level of the tile's cell.
    pub(super) level: u8,
    /// The cost so far.
    pub(super) g: f32,
    /// The estimate to the goal.
    pub(super) h: f32,
    /// The node it comes from.
    pub(super) parent: Option<usize>,
    /// The edge it was entered by.
    pub(super) dir: u8,
    /// The tile its cost is measured from.
    pub(super) entry: [i16; 2],
}

impl Planner {
    /// `FUN_140f79c90`: a new node from the pool.
    pub(super) fn alloc(&mut self, new: NewNode) -> usize {
        let NewNode {
            x,
            y,
            level,
            g,
            h,
            parent,
            dir,
            entry,
        } = new;
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
    pub(super) fn estimate_from(&self, x: i16, y: i16) -> f32 {
        let dx = i32::from(self.goal[0]) - i32::from(x);
        let dy = i32::from(self.goal[1]) - i32::from(y);
        fmath::sqrt((dy * dy + dx * dx) as f32)
    }

    /// The start's estimate (`FUN_140f78850`, `FUN_140f79250`): in f32 differences.
    pub(super) fn start_estimate(&self, x: i16, y: i16) -> f32 {
        let dx = f32::from(self.goal[0]) - f32::from(x);
        let dy = f32::from(self.goal[1]) - f32::from(y);
        fmath::sqrt(dy * dy + dx * dx)
    }

    /// Where a new node's cost is measured from (`FUN_140f764c0`, `FUN_140f76840`): a turn
    /// (a move across the parent's own move) measures from the grandparent, unless the parent
    /// already did; returns that node's cost so far, its tile and whether it was the grandparent.
    pub(super) fn measured_from(&self, parent: usize, dir: u8) -> (f32, [i16; 2], bool) {
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
    pub(super) fn expand(
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
        let n = self.alloc(NewNode {
            x,
            y,
            level,
            g: d + g0,
            h,
            parent: Some(parent),
            dir,
            entry: [cx, cy],
        });
        self.open.push(n, &self.nodes);
        self.nodes[n].shortcut = shortcut;
        if self.cells[i][0] & OPEN == 0 {
            self.cells[i][0] += OPEN;
        }
        (self.cells[i][1] & GOAL != 0).then_some(n)
    }

    /// `FUN_140f76840`: the level-0 neighbour across edge `dir`; a turn's cost from the
    /// grandparent counts 1.1 times the grandparent's cost so far.
    pub(super) fn expand0(
        &mut self,
        x: i16,
        y: i16,
        g: f32,
        parent: usize,
        dir: u8,
    ) -> Option<usize> {
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
        let n = self.alloc(NewNode {
            x,
            y,
            level: 0,
            g: d + g0,
            h,
            parent: Some(parent),
            dir,
            entry: [x, y],
        });
        self.open.push(n, &self.nodes);
        self.nodes[n].shortcut = shortcut;
        if self.cells[i][0] & OPEN == 0 {
            self.cells[i][0] += OPEN;
        }
        (self.cells[i][1] & GOAL != 0).then_some(n)
    }

    /// Close a popped node and keep it as the best when its estimate is the lowest yet.
    pub(super) fn close(&mut self, n: usize) -> Option<u8> {
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
    pub(super) fn search_levels(
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
        let n = self.alloc(NewNode {
            x: lx,
            y: ly,
            level,
            g: 0.0,
            h: h0,
            parent: None,
            dir: 0,
            entry: start,
        });
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
    pub(super) fn search_tiles(&mut self, start: [i16; 2]) -> Option<usize> {
        let [x, y] = start;
        if self.cell(0, x, y)[0] == 0 {
            return None;
        }
        let h0 = self.start_estimate(x, y);
        let n = self.alloc(NewNode {
            x,
            y,
            level: 0,
            g: 0.0,
            h: h0,
            parent: None,
            dir: 0,
            entry: start,
        });
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
}
