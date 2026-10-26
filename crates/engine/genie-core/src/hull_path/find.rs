//! `FindShortPath` (`FUN_140f85580`) and its search.

use crate::GenieError;
use crate::checkpoint::cvttss2si;
use crate::fmath::{below_or_unordered, maxss, minss, sqrt};
use crate::hull_path::{
    GROWTHS, HALF, HullPather, ITERATION_CAP, MOST, NEAR_TARGET, NUDGE, PATHS, Point, Target,
    Waypoints,
};

impl HullPather<'_> {
    /// `MShortPather::FindShortPath` (`FUN_140f85580`) as slot 0x3e8 and `FUN_140f9e540` call it
    /// (the area new, the quick path first): a path from `from` to `to` within `range` of object
    /// `target_id`'s footprint when it stands on the goal, else of the goal (for -1 or an object
    /// off the goal; an object the goal query `FUN_141051900` finds holding the goal is then a
    /// target whose id stays -1). Returns the result (0 already there, 1 a path within range, 2 a path partway,
    /// 3 none, 4 a place off the map) and the path.
    ///
    /// # Errors
    ///
    /// Where the pather meets what the slice does not model.
    pub fn find(
        &mut self,
        from: Point,
        to: Point,
        target_id: i32,
        range: f32,
    ) -> Result<(u8, Waypoints), GenieError> {
        self.find_with(from, to, target_id, range, false, false)
    }

    /// [`HullPather::find`] with `FindShortPath`'s argument 11 set: the area and the hulls of this
    /// pather's last search stay (when it has an area) and the search builds none.
    ///
    /// # Errors
    ///
    /// Where the pather meets what the slice does not model.
    pub fn find_keeping(
        &mut self,
        from: Point,
        to: Point,
        target_id: i32,
        range: f32,
    ) -> Result<(u8, Waypoints), GenieError> {
        self.find_with(from, to, target_id, range, true, false)
    }

    /// [`HullPather::find`] as slot 0x3e0 calls it (its last argument 1): the quick path alone,
    /// none (3) when it is blocked.
    ///
    /// # Errors
    ///
    /// Where the pather meets what the slice does not model.
    pub fn find_quick(
        &mut self,
        from: Point,
        to: Point,
        target_id: i32,
        range: f32,
    ) -> Result<(u8, Waypoints), GenieError> {
        self.find_with(from, to, target_id, range, false, true)
    }

    /// `FindShortPath` with its argument 11 (`keep`) and its argument 15 below 2 (`quick_only`
    /// for 1). Its callers pass 0 (slot 0x3e8, `FUN_140f9e540`) or 1 (slot 0x3e0) there, never 2
    /// or more, which would skip the quick path. Its arguments 12 to 14 are the pather's player
    /// and include list; its path is the one returned.
    pub(super) fn find_with(
        &mut self,
        from: Point,
        to: Point,
        target_id: i32,
        range: f32,
        keep: bool,
        quick_only: bool,
    ) -> Result<(u8, Waypoints), GenieError> {
        let (result, path, clears) =
            self.find_unclear(from, to, target_id, range, keep, quick_only)?;
        if clears {
            self.world.clear_tile_records();
            self.world.set_manager_buffer(None);
        }
        Ok((result, path))
    }

    /// [`HullPather::find_with`]'s search, and whether its exit removes the manager's temporary
    /// tile records (`FUN_1410561f0`) and resets its buffer (`FUN_1410562d0` with 0): every exit
    /// after the quick path's start does but the quick-only one (its caller, `FUN_1411506e0`,
    /// removes the records and leaves the buffer); the off-map and already-there exits come
    /// before the start set the buffer.
    #[allow(clippy::too_many_lines)]
    pub(super) fn find_unclear(
        &mut self,
        from: Point,
        to: Point,
        target_id: i32,
        range: f32,
        mut keep: bool,
        quick_only: bool,
    ) -> Result<(u8, Waypoints, bool), GenieError> {
        let r = self.radius;
        let [w, h] = self.world.map;
        let mut path = Waypoints::default();
        if !keep {
            self.area = [-1, -1, -1, -1];
        }
        self.bounds = [r, r, w as f32 - r, h as f32 - r];
        let [_, _, bx1, by1] = self.bounds;
        // each bound by `COMISS`/`JA` (0x140f8569f..0x140f85701): an unordered coordinate passes
        if from[0] < r || bx1 < from[0] || from[1] < r || by1 < from[1] {
            return Ok((4, path, false));
        }
        if (to[0] < r || bx1 < to[0] || to[1] < r || by1 < to[1])
            && !self.edge_goal(to, target_id)?
        {
            return Ok((4, path, false));
        }
        self.start = from;
        self.goal = to;
        // the target counts only where it stands on the goal
        let mut target_id = target_id;
        let target = if target_id < 0 {
            None
        } else {
            match self.world.get_known(target_id)? {
                Some(p) if p.position[0] == to[0] && p.position[1] == to[1] => Some(Target {
                    at: [p.position[0], p.position[1]],
                    radius: p.radius,
                    speed: p.speed,
                }),
                Some(_) => {
                    target_id = -1;
                    None
                }
                None => None,
            }
        };
        let straight =
            sqrt((from[1] - to[1]) * (from[1] - to[1]) + (from[0] - to[0]) * (from[0] - to[0]));
        let gap = target
            .as_ref()
            .map_or(straight - r, |t| self.target_gap(t, from));
        if (from[0] == to[0] && from[1] == to[1]) || gap <= range {
            return Ok((0, path, false));
        }
        self.buffer = (0.0 < r).then_some(r * HALF);
        // the pather's buffer becomes the manager's (FUN_1410562d0), its temporary tile records
        // go (FUN_1410561f0), and the pather's ignore list becomes the manager's
        // (FUN_1410562e0 with +0xc0..+0xd0)
        self.world.set_manager_buffer(self.buffer);
        self.world.clear_tile_records();
        self.world.set_manager_ignore(&self.ignore);
        // the quick path, to the goal moved by the range or short of the target
        let quick = if range == 0.0
            || target
                .as_ref()
                .is_some_and(|t| t.at[0] != to[0] && t.at[1] != to[1])
        {
            let mut q = to;
            if let Some(t) = target.as_ref()
                && t.at[0] == to[0]
                && t.at[1] == to[1]
            {
                let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
                let d = dy * dy + dx * dx;
                if t.speed != 0.0 || d <= NEAR_TARGET {
                    let len = sqrt(d);
                    q = [
                        q[0] - (dx / len) * (r + t.radius[0]),
                        q[1] - (dy / len) * (r + t.radius[1]),
                    ];
                }
            }
            q
        } else {
            let v = [to[0] - from[0], to[1] - from[1], 0.0];
            let len = sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
            let (ux, uy) = (v[0] / len, v[1] / len);
            [ux * range + to[0], uy * range + to[1]]
        };
        let quick_target = (target_id != -1).then_some(target_id);
        if self.quick_path(from, quick, quick_target, &mut path)? {
            return Ok((1, path, true));
        }
        if quick_only {
            return Ok((3, path, false));
        }
        // the area, unless kept: the walk's tiles and two more each way, on the map (`MINSS`,
        // `MAXSS` of the start's and the goal's, `CVTTSS2SI`, 0x140f85f61..0x140f85ff5)
        if !keep || self.area[0] == -1 {
            let tile = |v: f32, d: i32| cvttss2si(v).wrapping_add(d);
            let x0 = tile(minss(from[0], to[0]), -2);
            let y0 = tile(minss(from[1], to[1]), -2);
            let x1 = tile(maxss(from[0], to[0]), 2);
            let y1 = tile(maxss(from[1], to[1]), 2);
            self.area = [
                x0.max(0),
                y0.max(0),
                if w <= x1 { w - 1 } else { x1 },
                if h <= y1 { h - 1 } else { y1 },
            ];
            keep = false;
        }
        // no target on the goal: the first record with an object that holds the goal is the
        // target, its id still -1 (`FUN_141051900` at 0x140f85fd9)
        let target = match target {
            Some(t) => Some(t),
            None => self.goal_record(to)?,
        };
        let [ax0, ay0, ax1, ay1] = self.area;
        self.world.check_known_records([
            (ax0 - 5) as f32,
            (ay0 - 5) as f32,
            (ax1 + 6) as f32,
            (ay1 + 6) as f32,
        ])?;
        self.empty(0);
        let mut nearest_d =
            sqrt((from[1] - to[1]) * (from[1] - to[1]) + (from[0] - to[0]) * (from[0] - to[0]));
        let mut found = false;
        let mut entrance: i32 = -1;
        let mut iterations = 0;
        let mut growths = 0;
        let mut within = false;
        let mut entered_box = false;
        let mut best_node: Option<usize> = None;
        let mut start_hull = -1;
        let mut goal_hull = -1;
        'outer: loop {
            if ITERATION_CAP <= iterations || GROWTHS <= growths {
                break;
            }
            self.close_all(0);
            if !keep && growths == 0 {
                self.build_hulls()?;
            }
            let root = self.add_unrefused(0, true, None, from, Some(to), -1, -1);
            start_hull = self.hull_of(from);
            goal_hull = self.hull_of(to);
            within = false;
            if 0 <= start_hull {
                let sh = start_hull as usize;
                if start_hull == goal_hull {
                    path.points.clear();
                    let mut count = 0;
                    let walk = self.walk_out(
                        sh,
                        from,
                        Some(to),
                        target,
                        range,
                        false,
                        std::slice::from_mut(&mut path),
                        1,
                        &mut count,
                    )?;
                    if walk.result == 0 {
                        // 2 for a target whose id is below 1, the goal query's among them
                        let partial = target.is_some() && target_id < 1;
                        return Ok((if partial { 2 } else { 1 }, path, true));
                    }
                    if walk.result == 3 || !walk.left_boxes {
                        return Ok((3, path, true));
                    }
                    within = walk.result == 1;
                }
                let mut exits = std::mem::take(&mut self.exits);
                for e in &mut exits {
                    e.points.clear();
                }
                let mut count = 0;
                let walk = self.walk_out(
                    sh, from, None, None, range, true, &mut exits, PATHS, &mut count,
                );
                self.exits = exits;
                // the walk's result is not read: its paths are
                let _ = walk?;
                if count < 1 {
                    return Ok((3, path, true));
                }
                self.close(0, root);
                for k in 0..count {
                    let points = self.exits[k].points.clone();
                    let n = points.len();
                    if n <= 1 {
                        continue;
                    }
                    // the way out's points in turn, its last open (none refused: closed, or
                    // without a hull point)
                    let mut node = root;
                    for &p in &points[1..n - 1] {
                        node =
                            self.add_unrefused(0, false, Some(node), p, Some(to), start_hull, -1);
                    }
                    let last = points[n - 1];
                    let _ = self.add_unrefused(0, true, Some(node), last, Some(to), start_hull, -1);
                }
            }
            entered_box = false;
            if 0 <= goal_hull {
                let gh = goal_hull as usize;
                let mut entrances = std::mem::take(&mut self.entrances);
                for e in &mut entrances {
                    e.points.clear();
                }
                let n = self.hulls[gh].points.len();
                self.hulls[gh].entrances = vec![-1; n];
                let mut count = 0;
                let mut sides = [0usize; PATHS];
                let walk = self.walk_in(
                    gh,
                    to,
                    from,
                    target,
                    &mut entrances,
                    &mut sides,
                    PATHS,
                    &mut count,
                );
                self.entrances = entrances;
                entered_box = walk?.result == 5;
                self.join_entrances(gh, count, &sides);
            }
            // the search
            loop {
                if ITERATION_CAP <= iterations {
                    continue 'outer;
                }
                iterations += 1;
                let Some(best) = self.nodes[0].next else {
                    break 'outer;
                };
                let mut n = Some(best);
                let mut off = false;
                while let Some(i) = n {
                    if !self.on_map(self.nodes[i].at) {
                        off = true;
                        break;
                    }
                    n = self.nodes[i].next;
                }
                if off {
                    self.close(0, best);
                    continue;
                }
                if !self.in_area(self.nodes[best].at) {
                    if !self.grow_area()? {
                        break 'outer;
                    }
                    growths += 1;
                    continue 'outer;
                }
                self.close(0, best);
                let at = self.nodes[best].at;
                // within range of the target's box, or without one of the goal: the radius and the
                // range, squared, NUDGE more (`COMISS`/`JNC` at 0x140f86cff: a NaN is not)
                let reached = target.as_ref().map_or_else(
                    || {
                        let d =
                            (at[1] - to[1]) * (at[1] - to[1]) + (at[0] - to[0]) * (at[0] - to[0]);
                        d <= (range + r) * (range + r) + NUDGE
                    },
                    |t| self.target_gap(t, at) <= range + NUDGE,
                );
                if reached {
                    found = target.is_none() || target_id >= 0;
                    best_node = Some(best);
                    break 'outer;
                }
                if self.nodes[best].to_goal < nearest_d {
                    nearest_d = self.nodes[best].to_goal;
                    best_node = Some(best);
                }
                let vertex = self.nodes[best].vertex;
                let mut crossing: Option<(usize, Point, usize)> = None;
                let mut crossing_d = MOST;
                for (i, hull) in self.hulls.iter().enumerate() {
                    if let Some((p, edge)) = hull.intersection(vertex < 0, at, to) {
                        let d = (p[1] - at[1]) * (p[1] - at[1]) + (p[0] - at[0]) * (p[0] - at[0]);
                        if d < crossing_d {
                            crossing = Some((i, p, edge));
                            crossing_d = d;
                        }
                    }
                }
                let Some((hit, p, edge)) = crossing else {
                    // no hull between the node and the goal
                    if !self.blocked(at, to)? {
                        best_node =
                            self.add_node(0, false, Some(best), to, Some(to), -1, -1, -1, false);
                        found = true;
                        break 'outer;
                    }
                    if 0 <= goal_hull && goal_hull == self.nodes[best].hull && 0 <= vertex {
                        let entrances = &self.hulls[goal_hull as usize].entrances;
                        if let Some(&e) = entrances.get(vertex as usize)
                            && (vertex as usize) < self.hulls[goal_hull as usize].points.len()
                            && 0 <= e
                        {
                            found = true;
                            best_node = Some(best);
                            entrance = e;
                            break 'outer;
                        }
                    }
                    continue;
                };
                if start_hull == -1 && goal_hull == -1 && self.blocked(at, p)? {
                    continue;
                }
                if hit as i32 == self.nodes[best].hull && 0 <= vertex {
                    continue;
                }
                let node = self.add_unrefused(0, false, Some(best), p, Some(to), hit as i32, -1);
                if self.nodes[node].to_goal < nearest_d {
                    nearest_d = self.nodes[node].to_goal;
                    best_node = Some(node);
                }
                let np = self.hulls[hit].points.len();
                let mut back = if edge >= 1 { edge } else { np } as i32 - 1;
                let mut fwd = edge;
                let (mut a, mut b) = (Some(node), Some(node));
                for _ in 0..np / 2 {
                    if let Some(x) = a {
                        let q = self.hulls[hit].points[back as usize];
                        a = self.add_node(
                            0,
                            true,
                            Some(x),
                            q,
                            Some(to),
                            hit as i32,
                            back,
                            -1,
                            false,
                        );
                    }
                    if let Some(x) = b {
                        let q = self.hulls[hit].points[fwd];
                        b = self.add_node(
                            0,
                            true,
                            Some(x),
                            q,
                            Some(to),
                            hit as i32,
                            fwd as i32,
                            -1,
                            false,
                        );
                    }
                    back -= 1;
                    if back < 0 {
                        back = np as i32 - 1;
                    }
                    fwd = if fwd + 1 < np { fwd + 1 } else { 0 };
                }
                if np % 2 == 1 {
                    let (qa, qb) = (
                        self.hulls[hit].points[back as usize],
                        self.hulls[hit].points[fwd],
                    );
                    match (a, b) {
                        (None, None) => {}
                        (Some(x), y)
                            if y.is_none_or(|y| self.nodes[x].cost < self.nodes[y].cost) =>
                        {
                            let _ = self.add_node(
                                0,
                                true,
                                Some(x),
                                qa,
                                Some(to),
                                hit as i32,
                                back,
                                -1,
                                false,
                            );
                        }
                        (_, Some(y)) => {
                            let _ = self.add_node(
                                0,
                                true,
                                Some(y),
                                qb,
                                Some(to),
                                hit as i32,
                                fwd as i32,
                                -1,
                                false,
                            );
                        }
                        (Some(_), None) => unreachable!("taken by the arm above"),
                    }
                }
            }
        }
        let _ = (start_hull, goal_hull);
        // the end: the path to the node found, or to the one nearest the goal
        let Some(end) = best_node else {
            if !within {
                return Ok((3, path, true));
            }
            return Ok((self.partway(&mut path, from, to, range), path, true));
        };
        if !found && within {
            let last = *path.points.last().unwrap_or(&[f32::NAN, f32::NAN]);
            let e = self.nodes[end].at;
            let d_path =
                (last[1] - to[1]) * (last[1] - to[1]) + (last[0] - to[0]) * (last[0] - to[0]);
            let d_node = (e[1] - to[1]) * (e[1] - to[1]) + (e[0] - to[0]) * (e[0] - to[0]);
            // a node nearer the goal than the walk's end is the end (`COMISS`/`JNC` to the
            // partway end, 0x140f86cb6: an unordered distance takes the node)
            if !below_or_unordered(d_node, d_path) {
                return Ok((self.partway(&mut path, from, to, range), path, true));
            }
        }
        path.points.clear();
        self.build_path(&mut path, end)?;
        if found {
            if entrance < 0 {
                return Ok((1, path, true));
            }
            let e = self.entrances[entrance as usize].points.clone();
            for j in (0..e.len().saturating_sub(1)).rev() {
                path.push(e[j]);
            }
            if !entered_box {
                return Ok((1, path, true));
            }
            let k = path.points.len() - 1;
            let hits = self.hits(path.points[k], path.points[k - 1])?;
            if hits.len() < 2 && (hits.len() != 1 || hits[0].id == target_id) {
                return Ok((1, path, true));
            }
            let _ = path.points.remove(k);
        }
        Ok((self.partway(&mut path, from, to, range), path, true))
    }
}
