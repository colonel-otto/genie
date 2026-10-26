//! The path's points: the tiles a node's path crosses into and aims at, traced back from the
//! goal.

use crate::math::fmath::Vec3;
use crate::tile_path::{E, HALF, N, Planner, S, W};

impl Planner {
    /// `FUN_140f7b9e0`: the steps from a child cell to the goal's tile at that level, counted
    /// only where the goal lies right of or below it; `0xffff` for a cell off the map or without
    /// edges.
    pub(super) fn steps_to(&self, tx: i16, ty: i16, level: u8, cx: i16, cy: i16) -> u16 {
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
    pub(super) fn tile_toward(&self, n: usize, gx: i16, gy: i16) -> (i16, i16) {
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
    pub(super) fn nearest_on_boundary(
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
    pub(super) fn crossing(&self, n: usize) -> Option<(i16, i16)> {
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
    pub(super) fn aim(&self, n: usize, tx: i16, ty: i16, lx: i16, ly: i16) -> Option<(i16, i16)> {
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

    pub(super) const fn on_map(&self, x: i16, y: i16) -> bool {
        0 <= x && x < self.size[0] && 0 <= y && y < self.size[1]
    }

    /// A tile's centre as a path point.
    pub(super) fn centre(x: i16, y: i16) -> Vec3 {
        [f32::from(x) + HALF, 0.0, f32::from(y) + HALF]
    }

    /// The path's points from the goal's node back to the start (the loops of `FUN_140f77f20` and
    /// `FUN_140f78ba0`): per node the tile where the path enters its parent, aimed from the last
    /// point toward the crossing two nodes on (the start's tile when there is none).
    pub(super) fn trace_back(&mut self, n: usize, start: [i16; 2]) {
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
    pub(super) fn clear_nodes(&mut self) {
        for node in &self.nodes {
            self.cells[node.cell][0] &= 0xf;
        }
        self.nodes.clear();
    }
}
