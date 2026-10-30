//! `FindShortPath` (`FUN_140f85580`): the bounds, the query, the quick path and the area before
//! its rounds, and the path at their end.

use crate::GenieError;
use crate::math::conversions::cvttss2si;
use crate::math::fmath::{below_or_unordered, maxss, minss, sqrt};
use crate::pathing::hull_path::rounds::Rounds;
use crate::pathing::hull_path::{
    HALF, HullPather, HullQuery, NEAR_TARGET, Point, Target, Waypoints,
};
use crate::world::slots::CheckedIndex;

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
    /// before the start set the buffer. The search's rounds are [`HullPather::search`], its end
    /// [`HullPather::path_end`].
    pub(super) fn find_unclear(
        &mut self,
        from: Point,
        to: Point,
        target_id: i32,
        range: f32,
        keep: bool,
        quick_only: bool,
    ) -> Result<(u8, Waypoints, bool), GenieError> {
        let r = self.radius;
        let mut path = Waypoints::default();
        if !keep {
            self.area = [-1, -1, -1, -1];
        }
        if self.off_map(from, to, target_id)? {
            return Ok((4, path, false));
        }
        self.start = from;
        self.goal = to;
        let mut q = self.query(from, to, target_id, range)?;
        if self.already_there(&q) {
            return Ok((0, path, false));
        }
        self.buffer = (0.0 < r).then_some(r * HALF);
        // the pather's buffer becomes the manager's (FUN_1410562d0), its temporary tile records
        // go (FUN_1410561f0), and the pather's ignore list becomes the manager's
        // (FUN_1410562e0 with +0xc0..+0xd0)
        self.world.set_manager_buffer(self.buffer);
        self.world.clear_tile_records();
        self.world.set_manager_ignore(&self.ignore);
        let quick_target = (q.target_id != -1).then_some(q.target_id);
        if self.quick_path(from, self.quick_goal(&q), quick_target, &mut path)? {
            return Ok((1, path, true));
        }
        if quick_only {
            return Ok((3, path, false));
        }
        let keep = self.set_area(from, to, keep);
        // no target on the goal: the first record with an object that holds the goal is the
        // target, its id still -1 (`FUN_141051900` at 0x140f85fd9)
        if q.target.is_none() {
            q.target = self.goal_record(to)?;
        }
        let [ax0, ay0, ax1, ay1] = self.area;
        self.world.check_known_records([
            (ax0 - 5) as f32,
            (ay0 - 5) as f32,
            (ax1 + 6) as f32,
            (ay1 + 6) as f32,
        ])?;
        self.empty(0)?;
        let mut rounds = Rounds::new(sqrt(
            (from.y() - to.y()) * (from.y() - to.y()) + (from.x() - to.x()) * (from.x() - to.x()),
        ));
        if let Some(result) = self.search(&q, keep, &mut rounds, &mut path)? {
            return Ok((result, path, true));
        }
        let result = self.path_end(&q, &rounds, &mut path)?;
        Ok((result, path, true))
    }

    /// `FindShortPath`'s bounds, the map less the radius (`+0x118`), and whether the start or the
    /// goal lies beyond them (result 4): a goal beyond them stays when [`HullPather::edge_goal`]
    /// takes it.
    fn off_map(&mut self, from: Point, to: Point, target_id: i32) -> Result<bool, GenieError> {
        let r = self.radius;
        let [w, h] = self.world.map;
        self.bounds = [r, r, w as f32 - r, h as f32 - r];
        let [_, _, bx1, by1] = self.bounds;
        // each bound by `COMISS`/`JA` (0x140f8569f..0x140f85701): an unordered coordinate passes
        if from.x() < r || bx1 < from.x() || from.y() < r || by1 < from.y() {
            return Ok(true);
        }
        Ok((to.x() < r || bx1 < to.x() || to.y() < r || by1 < to.y())
            && !self.edge_goal(to, target_id)?)
    }

    /// The search's query: the target counts only where it stands on the goal; an object off the
    /// goal makes the id -1.
    fn query(
        &self,
        from: Point,
        to: Point,
        target_id: i32,
        range: f32,
    ) -> Result<HullQuery, GenieError> {
        let mut target_id = target_id;
        let target = if target_id < 0 {
            None
        } else {
            match self.world.get_known(target_id)? {
                Some(p) if p.position.x() == to.x() && p.position.y() == to.y() => Some(Target {
                    at: p.position.ground(),
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
        Ok(HullQuery {
            from,
            to,
            target,
            target_id,
            range,
        })
    }

    /// Whether the unit is there already (result 0): on the goal, or within range of the target's
    /// box or of the goal (the straight walk less the radius).
    fn already_there(&self, q: &HullQuery) -> bool {
        let (from, to) = (q.from, q.to);
        let straight = sqrt(
            (from.y() - to.y()) * (from.y() - to.y()) + (from.x() - to.x()) * (from.x() - to.x()),
        );
        let gap = q
            .target
            .as_ref()
            .map_or(straight - self.radius, |t| self.target_gap(t, from));
        (from.x() == to.x() && from.y() == to.y()) || gap <= q.range
    }

    /// The quick path's goal: without a range (or with a target off the goal) the goal, short of
    /// a target on it by both radii when the target moves or stands near; with a range the goal
    /// moved on by the range along the walk.
    fn quick_goal(&self, q: &HullQuery) -> Point {
        let (from, to, r) = (q.from, q.to, self.radius);
        if q.range != 0.0
            && !q
                .target
                .as_ref()
                .is_some_and(|t| t.at.x() != to.x() && t.at.y() != to.y())
        {
            let v = [to.x() - from.x(), to.y() - from.y(), 0.0];
            let len = sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
            let (ux, uy) = (v[0] / len, v[1] / len);
            return Point::new(ux * q.range + to.x(), uy * q.range + to.y());
        }
        let mut goal = to;
        if let Some(t) = q.target.as_ref()
            && t.at.x() == to.x()
            && t.at.y() == to.y()
        {
            let (dx, dy) = (to.x() - from.x(), to.y() - from.y());
            let d = dy * dy + dx * dx;
            if t.speed != 0.0 || d <= NEAR_TARGET {
                let len = sqrt(d);
                goal = Point::new(
                    goal.x() - (dx / len) * (r + t.radius.x()),
                    goal.y() - (dy / len) * (r + t.radius.y()),
                );
            }
        }
        goal
    }

    /// The area, unless kept: the walk's tiles and two more each way, on the map (`MINSS`,
    /// `MAXSS` of the start's and the goal's, `CVTTSS2SI`, 0x140f85f61..0x140f85ff5). Whether the
    /// search keeps its hulls: a new area builds them.
    fn set_area(&mut self, from: Point, to: Point, keep: bool) -> bool {
        if keep && self.area[0] != -1 {
            return true;
        }
        let [w, h] = self.world.map;
        let tile = |v: f32, d: i32| cvttss2si(v).wrapping_add(d);
        let x0 = tile(minss(from.x(), to.x()), -2);
        let y0 = tile(minss(from.y(), to.y()), -2);
        let x1 = tile(maxss(from.x(), to.x()), 2);
        let y1 = tile(maxss(from.y(), to.y()), 2);
        self.area = [
            x0.max(0),
            y0.max(0),
            if w <= x1 { w - 1 } else { x1 },
            if h <= y1 { h - 1 } else { y1 },
        ];
        false
    }

    /// The search's end: the path to the node found, or to the one nearest the goal; a walk
    /// within the start's hull nearer the goal than that node ends partway along the walk, and
    /// without a node it ends there too, or finds none (3). The result.
    pub(super) fn path_end(
        &mut self,
        q: &HullQuery,
        rounds: &Rounds,
        path: &mut Waypoints,
    ) -> Result<u8, GenieError> {
        let (from, to, range) = (q.from, q.to, q.range);
        let Some(end) = rounds.best_node else {
            if !rounds.within {
                return Ok(3);
            }
            return Ok(self.partway(path, from, to, range));
        };
        if !rounds.found && rounds.within && !self.node_nearer(end, path, to)? {
            return Ok(self.partway(path, from, to, range));
        }
        path.points.clear();
        self.build_path(path, end)?;
        if rounds.found && self.found_end(q, rounds, path)? {
            return Ok(1);
        }
        Ok(self.partway(path, from, to, range))
    }

    /// Whether the node `end` lies nearer the goal than the walk's end, the path's last point (a
    /// path without one is NaN). `COMISS`/`JNC` to the partway end at 0x140f86cb6: an unordered
    /// distance takes the node.
    fn node_nearer(&self, end: usize, path: &Waypoints, to: Point) -> Result<bool, GenieError> {
        let last = *path
            .points
            .last()
            .unwrap_or(&Point::new(f32::NAN, f32::NAN));
        let e = self.nodes.checked(end)?.at;
        let d_path =
            (last.y() - to.y()) * (last.y() - to.y()) + (last.x() - to.x()) * (last.x() - to.x());
        let d_node = (e.y() - to.y()) * (e.y() - to.y()) + (e.x() - to.x()) * (e.x() - to.x());
        Ok(below_or_unordered(d_node, d_path))
    }

    /// The found node's path on into the goal's hull: along its entrance, if any, backwards. A
    /// path whose entrance ends in a box keeps its last leg only when it hits nothing or the
    /// target alone; else the last point goes. Whether the path is found (1); else it ends
    /// partway.
    pub(super) fn found_end(
        &mut self,
        q: &HullQuery,
        rounds: &Rounds,
        path: &mut Waypoints,
    ) -> Result<bool, GenieError> {
        if rounds.entrance < 0 {
            return Ok(true);
        }
        let e = &self.entrances.checked(rounds.entrance as usize)?.points;
        for j in (0..e.len().saturating_sub(1)).rev() {
            path.push(*e.checked(j)?);
        }
        if !rounds.entered_box {
            return Ok(true);
        }
        let k = path.points.len() - 1;
        let hits = self.hits(*path.points.checked(k)?, *path.points.checked(k - 1)?)?;
        if hits.len() < 2 && (hits.len() != 1 || hits.checked(0)?.id == q.target_id) {
            return Ok(true);
        }
        let _ = path.points.remove(k);
        Ok(false)
    }
}
