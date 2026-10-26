//! The planner's entry (`FUN_140f7c530`): the search over the levels, the shortcut near the
//! start and the search on the tiles.

use crate::math::fmath;
use crate::tile_path::{
    Found, GOAL_GAP, NODE, Path, Planner, Request, SHORTCUT_REACH, SegmentTest, TILE_SEARCH_LENGTH,
    smooth, tile,
};

impl Planner {
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
    pub(super) fn near_passable(&self, x: i16, y: i16) -> bool {
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
    pub(super) fn shorter_start(&mut self, req: &Request, levels: &mut Path) {
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
