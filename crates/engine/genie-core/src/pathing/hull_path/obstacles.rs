//! The hulls' building from the area's obstructions: the tiles' boxes, the runs the unit
//! cannot cross, the cliffs; a point's hull and the area's bounds.

use crate::GenieError;
use crate::map::spatial::{Placed, tile_record_id};
use crate::math::fmath::at_most_or_unordered;
use crate::objects::masters::class;
use crate::pathing::hull_path::{HALF, Hull, HullPather, Point, Shape};
use crate::world::slots::CheckedIndex;

impl HullPather<'_> {
    // The hulls.

    /// `MShortPather::addShapeToHulls` (`FUN_140f8af60`): the box `min`..`max` into the hulls:
    /// into the first it overlaps (its corners outside it, or all four when it only touches),
    /// which then takes in each hull it overlaps in turn; a box inside a hull changes nothing;
    /// else a new hull. Returns the hull's index.
    pub(super) fn add_shape_to(
        hulls: &mut Vec<Hull>,
        min: Point,
        max: Point,
    ) -> Result<usize, GenieError> {
        let mut index = 0;
        let mut hull: Option<usize> = None;
        while index < hulls.len() {
            let (overlap, outside) = hulls.checked(index)?.shape_overlap(min, max);
            let corners = [
                [min[0], min[1]],
                [max[0], min[1]],
                [max[0], max[1]],
                [min[0], max[1]],
            ];
            match overlap {
                1 => {
                    hulls.checked_mut(index)?.merge(&corners)?;
                    hull = Some(index);
                }
                2 => {
                    let some: Vec<Point> = corners
                        .iter()
                        .zip(outside)
                        .filter(|&(_, outside)| outside)
                        .map(|(corner, _)| *corner)
                        .collect();
                    hulls.checked_mut(index)?.merge(&some)?;
                    hull = Some(index);
                }
                3 => return Ok(index),
                _ => {
                    index += 1;
                    continue;
                }
            }
            break;
        }
        let Some(mut mine) = hull else {
            hulls.push(Hull {
                points: vec![
                    [max[0], max[1]],
                    [max[0], min[1]],
                    [min[0], min[1]],
                    [min[0], max[1]],
                ],
                entrances: vec![-1; 4],
                min,
                max,
                objects: Vec::new(),
            });
            return Ok(hulls.len() - 1);
        };
        'merging: loop {
            for j in 0..hulls.len() {
                if j == mine {
                    continue;
                }
                let (h, o) = (hulls.checked(mine)?, hulls.checked(j)?);
                if !(h.min[0] <= o.max[0]
                    && h.min[1] <= o.max[1]
                    && o.min[0] <= h.max[0]
                    && o.min[1] <= h.max[1])
                {
                    continue;
                }
                let n = o.points.len();
                let overlaps = h.points.iter().any(|p| o.inside(*p))
                    || o.points.iter().any(|p| h.inside(*p))
                    // the other's sides k = 0..n-1 (`cmp edi, esi; jl` at 0x140f8b2c4, esi = n - 1)
                    || (0..n.saturating_sub(1)).any(|k| {
                        let prev = if k == 0 { n - 1 } else { k - 1 };
                        o.points
                            .get(k)
                            .zip(o.points.get(prev))
                            .is_some_and(|(a, b)| h.intersection(true, *a, *b).is_some())
                    });
                if !overlaps {
                    continue;
                }
                mine = Self::absorb(hulls, mine, j)?;
                continue 'merging;
            }
            return Ok(mine);
        }
    }

    /// Hull `j` merges into hull `mine`: its boxes the other lacks and its points join, and its
    /// slot goes to the last hull. Returns where `mine` now is.
    fn absorb(hulls: &mut Vec<Hull>, mine: usize, j: usize) -> Result<usize, GenieError> {
        let other = hulls.checked(j)?.clone();
        for s in &other.objects {
            let mine_objects = &mut hulls.checked_mut(mine)?.objects;
            if !mine_objects.iter().any(|m| m.id == s.id) {
                mine_objects.push(*s);
            }
        }
        hulls.checked_mut(mine)?.merge(&other.points)?;
        let last = hulls.len() - 1;
        hulls.swap(j, last);
        let _ = hulls.pop();
        Ok(if mine == last { j } else { mine })
    }

    /// `MShortPather::addTileObjects` (`FUN_140f8b460`): the tall objects of tile (`x`, `y`)'s
    /// list (master `+0x5e0` above 0), in its order, but gates `player` may pass (slot 0x678) and,
    /// unless included, those ignored or moving (slot 0x5c8): each box, grown by the radius, into
    /// `hull` (when given and the object is at most a tile wide) or its own place in the hulls,
    /// once per hull.
    pub(super) fn add_tile_objects(
        &mut self,
        x: i32,
        y: i32,
        hull: Option<usize>,
        player: i32,
    ) -> Result<(), GenieError> {
        let world = &*self.world;
        for &id in world.tiles().list(x, y) {
            let Some(p) = world.get(id) else {
                continue;
            };
            if !p.tall || self.passes(id, p, player)? {
                continue;
            }
            // slot 0x5c8 is asked only of an object not ignored
            let left_out = self.ignore.contains(&id) || world.gait(id).is_some_and(|g| g.moving);
            if left_out && !self.include.contains(&id) {
                continue;
            }
            Self::add_box(&mut self.hulls, self.radius, id, p, hull)?;
        }
        Ok(())
    }

    /// Slot 0x678 of object `id` for `player` (the pather's, or -1): only a gate's
    /// (`FUN_1411d4a30`) answers yes, one of the gates the caller gave
    /// ([`HullPather::with_gates`]), and never for player -1 (`processCliffs`'): its ally test
    /// `FUN_141663dc0` takes no number at or above the player count, unsigned.
    pub(super) fn passes(&self, id: i32, p: &Placed, player: i32) -> Result<bool, GenieError> {
        if p.class != class::GATE || player < 0 {
            return Ok(false);
        }
        let gates = self.gates.ok_or(GenieError::unsupported(
            "a gate in the old short pather's area (its players' passage, slot 0x678)",
        ))?;
        Ok(gates.contains(&id))
    }

    /// One object's box into the hulls ([`HullPather::add_tile_objects`]).
    pub(super) fn add_box(
        hulls: &mut Vec<Hull>,
        r: f32,
        id: i32,
        p: &Placed,
        hull: Option<usize>,
    ) -> Result<(), GenieError> {
        let [x, y, _] = p.position;
        let [rx, ry] = p.radius;
        let bounds = [(x - rx) - r, (y - ry) - r, x + rx + r, y + ry + r];
        // `COMISS`/`JA` (0x140f8b772, 0x140f8b7a4): an unordered radius takes the given hull
        let h = match hull {
            Some(h) if at_most_or_unordered(rx, HALF) && at_most_or_unordered(ry, HALF) => h,
            _ => Self::add_shape_to(hulls, [bounds[0], bounds[1]], [bounds[2], bounds[3]])?,
        };
        let objects = &mut hulls.checked_mut(h)?.objects;
        if !objects.iter().any(|o| o.id == id) {
            objects.push(Shape { bounds, id });
        }
        Ok(())
    }

    /// [`HullPather::add_shape_to`] on the pather's hulls.
    pub(super) fn add_shape(&mut self, min: Point, max: Point) -> Result<usize, GenieError> {
        Self::add_shape_to(&mut self.hulls, min, max)
    }

    /// `MShortPather::processTileObstructions` (`FUN_140f8e1d0`): tile (`x`, `y`)'s box when its
    /// terrain is not the unit's (unless ignored), into `hull` or its own place, then its objects,
    /// and those of its building's anchor tile when that lies outside the `area`.
    pub(super) fn process_tile(
        &mut self,
        x: i32,
        y: i32,
        area: [i32; 4],
        hull: Option<usize>,
    ) -> Result<(), GenieError> {
        let [w, h] = self.world.map;
        if x < 0 || y < 0 || w <= x || h <= y {
            return Ok(());
        }
        let word = self.world.tiles().word(x, y).unwrap_or(0);
        if word & u64::from(self.mask ^ 0xff) != 0 {
            let id = tile_record_id(x, y);
            if !self.ignore.contains(&id) {
                let r = self.radius;
                let bounds = [
                    x as f32 - r,
                    y as f32 - r,
                    (x + 1) as f32 + r,
                    (y + 1) as f32 + r,
                ];
                let k = match hull {
                    Some(k) => k,
                    None => self.add_shape([bounds[0], bounds[1]], [bounds[2], bounds[3]])?,
                };
                let objects = &mut self.hulls.checked_mut(k)?.objects;
                if !objects.iter().any(|o| o.id == id) {
                    objects.push(Shape { bounds, id });
                }
            }
        }
        self.add_tile_objects(x, y, hull, self.player)?;
        let sight = self.world.sight.as_ref().ok_or(GenieError::unsupported(
            "the old short pather without the map's building links",
        ))?;
        if sight.link(x, y)?.0 & 0x60 != 0
            && let Some([ax, ay]) = sight.anchor_of(x, y)
            && (ax < area[0] || area[2] < ax || ay < area[1] || area[3] < ay)
        {
            self.add_tile_objects(ax, ay, hull, self.player)?;
        }
        Ok(())
    }

    /// `MShortPather::buildHulls` (`FUN_140f8b8f0`): the area's tiles row by row: a run of tiles
    /// the unit cannot cross (its terrain, or a static record's mark but a class 5 one) is one box
    /// whose tiles' boxes and small objects go into its hull; each other tile's objects go each to
    /// its own place. Then the cliffs about the area.
    pub(super) fn build_hulls(&mut self) -> Result<(), GenieError> {
        self.hulls.clear();
        let [w, h] = self.world.map;
        self.area[0] = self.area[0].max(0);
        self.area[1] = self.area[1].max(0);
        self.area[2] = self.area[2].min(w - 1);
        self.area[3] = self.area[3].min(h - 1);
        let r = self.radius;
        let not_mask = u64::from(self.mask ^ 0xff);
        for y in self.area[1]..=self.area[3] {
            let mut run: Option<i32> = None;
            let mut x = self.area[0];
            while x <= self.area[2] {
                let word = self.world.tiles().word(x, y).unwrap_or(0);
                let open =
                    word & not_mask == 0 && ((word >> 14) & 1 == 0 || word & 0x3f00 == 0x500);
                if open {
                    if let Some(x0) = run.take() {
                        self.add_run(x0, x, y, r)?;
                    }
                    self.process_tile(x, y, self.area, None)?;
                } else if run.is_none() {
                    run = Some(x);
                }
                x += 1;
            }
            if let Some(x0) = run {
                self.add_run(x0, x, y, r)?;
            }
        }
        self.process_cliffs()
    }

    /// A run of tiles `x0`..`x1` of row `y` the unit cannot cross: one box, then its tiles.
    pub(super) fn add_run(&mut self, x0: i32, x1: i32, y: i32, r: f32) -> Result<(), GenieError> {
        let min = [x0 as f32 - r, y as f32 - r];
        let max = [x1 as f32 + r, (y + 1) as f32 + r];
        let hull = self.add_shape(min, max)?;
        for t in x0..x1 {
            self.process_tile(t, y, self.area, Some(hull))?;
        }
        Ok(())
    }

    /// `MShortPather::processCliffs` (`FUN_140f8e020`): the objects of the cliff tiles (a static
    /// record of class 6) within two tiles about the area.
    pub(super) fn process_cliffs(&mut self) -> Result<(), GenieError> {
        let [w, h] = self.world.map;
        let [x0, y0, x1, y1] = self.area;
        let (ex0, ey0) = ((x0 - 2).max(0), (y0 - 2).max(0));
        let (ex1, ey1) = ((x1 + 2).min(w - 1), (y1 + 2).min(h - 1));
        for y in ey0..=ey1 {
            for x in (ex0..x0).chain(x1 + 1..=ex1) {
                self.cliff(x, y)?;
            }
        }
        for y in (ey0..y0).chain(y1 + 1..=ey1) {
            for x in x0..=x1 {
                self.cliff(x, y)?;
            }
        }
        Ok(())
    }

    /// `FUN_140f8a890`: a cliff tile's objects (its word's static class 6), each to its own place
    /// ([`HullPather::add_tile_objects`] with no hull and player -1: no gate passes).
    pub(super) fn cliff(&mut self, x: i32, y: i32) -> Result<(), GenieError> {
        let word = self.world.tiles().word(x, y).unwrap_or(0);
        if word & 0x3f00 == 0x600 {
            self.add_tile_objects(x, y, None, -1)?;
        }
        Ok(())
    }

    /// `MShortPather::checkPointInsideHull` (`FUN_140f8be30`): the first hull holding `p`.
    pub(super) fn hull_of(&self, p: Point) -> i32 {
        self.hulls
            .iter()
            .position(|h| !h.points.is_empty() && h.inside(p))
            .map_or(-1, |i| i as i32)
    }

    /// Whether `p` lies within the area's tiles, `r` from their edge. Each bound skips by
    /// `COMISS`/`JA` (0x140f86a44, 0x140f8d942, 0x140f899d3): an unordered coordinate is inside.
    pub(super) fn in_area(&self, p: Point) -> bool {
        let r = self.radius;
        let [x0, y0, x1, y1] = self.area;
        at_most_or_unordered(x0 as f32 + r, p[0])
            && at_most_or_unordered(p[0], (x1 + 1) as f32 - r)
            && at_most_or_unordered(y0 as f32 + r, p[1])
            && at_most_or_unordered(p[1], (y1 + 1) as f32 - r)
    }

    /// Whether `p` lies on the map, `r` from its edge.
    pub(super) fn on_map(&self, p: Point) -> bool {
        let r = self.radius;
        let [w, h] = self.world.map;
        !(p[0] < r || w as f32 - r < p[0] || p[1] < r || h as f32 - r < p[1])
    }
}
