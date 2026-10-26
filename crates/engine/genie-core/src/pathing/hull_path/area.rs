//! The search's goal and area: the goal's query, a goal at the map's edge, an end partway,
//! the area's growth and the entrances' join.

use crate::GenieError;
use crate::math::fmath::{at_most_or_unordered, below_or_unordered};
use crate::pathing::hull_path::{EPSILON, HullPather, MOST_AREA, PATHS, Point, Target, Waypoints};

impl HullPather<'_> {
    /// `FindShortPath`'s goal query when no target stands on the goal (`FUN_141051900` with
    /// flags 0x20 at `to`, the manager's buffer and ignore list the pather's, its player for the
    /// gates): the records whose box, grown by the buffer, holds the goal, edges included (the
    /// point test `FUN_141052160`, moving ones too), but the ignored ones and a gate the player
    /// may pass (slot 0x678); the first with an object is the target. The goal lies on the map
    /// less the radius, so the grown point meets no edge record, and the temporary tile records
    /// have no object. The records come in the grid's order and each cell's list in the order
    /// the records joined it, which the slice does not hold: the answer is exact for one object
    /// holding the goal or none.
    pub(super) fn goal_record(&self, to: Point) -> Result<Option<Target>, GenieError> {
        let grow = self.buffer.unwrap_or(0.0);
        if !self.world.point_hit(to, grow, &self.ignore, false)? {
            return Ok(None);
        }
        let [x, y] = to;
        let near: Vec<i32> = self
            .world
            .records()
            .filter(|&(id, _, [x0, y0, x1, y1])| {
                !self.ignore.contains(&id)
                    && x <= x1 + grow
                    && x0 - grow <= x
                    && y <= grow + y1
                    && y0 - grow <= y
            })
            .map(|(id, _, _)| id)
            .collect();
        // each one the query lists (the cells it visits): the point test with the others ignored
        let mut held = Vec::new();
        for &id in &near {
            let mut others = self.ignore.clone();
            others.extend(near.iter().filter(|&&o| o != id));
            if !self.world.point_hit(to, grow, &others, false)? {
                continue;
            }
            let Some(p) = self.world.get(id) else {
                continue;
            };
            if !self.passes(id, p, self.player)? {
                held.push(p);
            }
        }
        match held[..] {
            [] => Ok(None),
            [p] => Ok(Some(Target {
                at: [p.position[0], p.position[1]],
                radius: p.radius,
                speed: p.speed,
            })),
            _ => Err(GenieError::unsupported(
                "the old short pather's goal held by two objects or more (the grid's order)",
            )),
        }
    }

    /// `FindShortPath` for a goal off the map less the radius: the target (none: 4) must stand
    /// on it (`UCOMISS`, an unordered coordinate does not), and the goal lie on the map itself
    /// (`FUN_140f89d80` with the bounds the map's: `COMISS`/`JA` each way, an unordered
    /// coordinate on it); then the bounds go back to the map less the radius.
    pub(super) fn edge_goal(&mut self, to: Point, target_id: i32) -> Result<bool, GenieError> {
        if target_id == -1 {
            return Ok(false);
        }
        let Some(t) = self.world.get_known(target_id)? else {
            return Ok(false);
        };
        if t.position[0] != to[0] || t.position[1] != to[1] {
            return Ok(false);
        }
        let r = self.radius;
        let [w, h] = self.world.map;
        self.bounds = [0.0, 0.0, w as f32 - 0.0, h as f32 - 0.0];
        let [bx0, by0, bx1, by1] = self.bounds;
        let on_map = at_most_or_unordered(bx0, to[0])
            && at_most_or_unordered(to[0], bx1)
            && at_most_or_unordered(by0, to[1])
            && at_most_or_unordered(to[1], by1);
        if on_map {
            self.bounds = [r, r, w as f32 - r, h as f32 - r];
        }
        Ok(on_map)
    }

    /// `FindShortPath`'s end for a path partway (2): none when it ends at the start (3); within
    /// range of the goal it is a path (1); one that ends no nearer the goal than the start is none.
    pub(super) fn partway(&self, path: &mut Waypoints, from: Point, to: Point, range: f32) -> u8 {
        let mut result = 2;
        let last = *path.points.last().unwrap_or(&[f32::NAN, f32::NAN]);
        if (last[0] - self.start[0]).abs() < EPSILON && (last[1] - self.start[1]).abs() < EPSILON {
            path.points.clear();
            result = 3;
        }
        let d = (last[1] - to[1]) * (last[1] - to[1]) + (last[0] - to[0]) * (last[0] - to[0]);
        // `COMISS`/`JC` (0x140f876ca): an unordered distance counts as nearer
        let start = (from[1] - to[1]) * (from[1] - to[1]) + (from[0] - to[0]) * (from[0] - to[0]);
        if below_or_unordered(d, start) {
            if d < range * range {
                result = 1;
            }
        } else {
            path.points.clear();
            result = 3;
        }
        result
    }

    /// The area grows by two tiles each way on the map (its new tiles' obstructions, then the
    /// cliffs about it); false when it would hold more than [`MOST_AREA`] tiles.
    pub(super) fn grow_area(&mut self) -> Result<bool, GenieError> {
        let [w, h] = self.world.map;
        let x0 = (self.area[0] - 2).max(0);
        let y0 = (self.area[1] - 2).max(0);
        let x1 = (self.area[2] + 2).min(w - 1);
        let y1 = (self.area[3] + 2).min(h - 1);
        if MOST_AREA < (x1 - x0) * (y1 - y0) {
            return Ok(false);
        }
        let grown = [x0, y0, x1, y1];
        for x in x0..=x1 {
            for y in y0..=y1 {
                self.process_tile(x, y, grown, None)?;
            }
        }
        self.area = grown;
        self.process_cliffs()?;
        Ok(true)
    }

    /// The entrance paths' ends join the goal's hull (`FindShortPath`): each on the hull side it
    /// left by, by side and then by its distance from the side's first point, the later ones
    /// after the earlier; an end on the last side goes last. Each joined point keeps its path.
    pub(super) fn join_entrances(&mut self, hull: usize, count: usize, sides: &[usize; PATHS]) {
        if count == 0 {
            return;
        }
        let mut order: Vec<usize> = (0..count).collect();
        let mut by: Vec<usize> = sides[..count].to_vec();
        let mut d: Vec<f32> = (0..count)
            .map(|k| {
                let last = *self.entrances[k].points.last().unwrap_or(&[0.0, 0.0]);
                let points = &self.hulls[hull].points;
                let i = if by[k] == 0 { points.len() } else { by[k] };
                let p = points[i - 1];
                (last[1] - p[1]) * (last[1] - p[1]) + (last[0] - p[0]) * (last[0] - p[0])
            })
            .collect();
        for a in 0..count {
            for b in a + 1..count {
                if by[b] < by[a] || (by[a] == by[b] && d[b] < d[a]) {
                    by.swap(a, b);
                    d.swap(a, b);
                    order.swap(a, b);
                }
            }
        }
        let mut joined = 0;
        for k in 0..count {
            let e = order[k];
            let last = *self.entrances[e].points.last().unwrap_or(&[0.0, 0.0]);
            let hull = &mut self.hulls[hull];
            let at = if joined + by[k] != 0 {
                joined + by[k]
            } else {
                hull.points.len()
            };
            hull.points.insert(at, last);
            hull.entrances.insert(at, e as i32);
            if by[k] != 0 {
                joined += 1;
            }
        }
    }
}
