//! A search's request and goal: its ranges and the marks of the cells and tiles within them.

use crate::tile_path::{CLOSED, FAR_SIDE, GOAL, HALF, LONE, NODE, ONE, Planner};

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
pub(super) struct Goal {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) radii: f32,
    pub(super) range: f32,
}

impl Request {
    pub(super) const fn goal(&self) -> Goal {
        Goal {
            x: self.to[0],
            y: self.to[1],
            radii: self.radii,
            range: self.range,
        }
    }
}

/// `(short)(int)v`: the game's tile of a coordinate.
pub(super) const fn tile(v: f32) -> i16 {
    v as i32 as i16
}

impl Planner {
    /// `FUN_140f7ba80`: whether a cell at `level` lies within the goal's ranges of (`x`, `y`): one
    /// of its passable (or lone) tiles, measured from the tile's corner.
    pub(super) fn in_range(&self, level: u8, cx: i16, cy: i16, goal: Goal) -> bool {
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
    pub(super) fn goal_square(&mut self, goal: Goal, far: f32) {
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
    pub(super) fn toggle_goal(&mut self, i: usize, set: bool) -> bool {
        let b1 = self.cells[i][1];
        if set == (b1 & GOAL != 0) {
            return false;
        }
        self.cells[i][1] = if set { b1 + GOAL } else { b1 - GOAL };
        true
    }

    /// `FUN_140f7d540`: mark (or clear) the node cells within the goal's ranges; when none is, the
    /// coarsest node cell holding the goal's tile.
    pub(super) fn mark_goal(&mut self, goal: Goal, set: bool) {
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
    pub(super) fn mark_cells(&mut self, level: u8, cx: i16, cy: i16, goal: Goal, set: bool) -> i32 {
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
    pub(super) fn mark_goal_tiles(&mut self, goal: Goal, set: bool) {
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
    pub(super) fn close_rect(&mut self, [x0, y0, x1, y1]: [i16; 4], set: bool) {
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
    pub(super) fn close_cells(&mut self, mut level: u8, mut x: i16, mut y: i16, set: bool) {
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
}
