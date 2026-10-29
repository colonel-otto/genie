//! The objects on the map: their tiles and records, their freezing and the known checks, their
//! moves inside and out, and the segment tests.

use crate::GenieError;
use crate::map::spatial::{
    Grid, Obstruction, Placed, RecordCells, Spatial, Tiles, WalkTest, tile_record_id,
};
use std::collections::BTreeMap;

impl Spatial {
    /// A map without terrain: every tile's class is 0.
    #[must_use]
    pub fn new(map: [i32; 2]) -> Self {
        let n = (map[0].max(0) * map[1].max(0)) as usize;
        Self::with_tiles(map, Tiles::new(map[0], map[1], vec![0; n]))
    }

    /// A map with the given tiles.
    #[must_use]
    pub fn with_tiles(map: [i32; 2], tiles: Tiles) -> Self {
        Self {
            map,
            manager_ignore: std::cell::RefCell::new(Vec::new()),
            tile_records: std::cell::RefCell::default(),
            manager_buffer: std::cell::Cell::new(None),
            objects: BTreeMap::new(),
            records: RecordCells::default(),
            inside: BTreeMap::new(),
            tiles,
            changed: Vec::new(),
            frozen_for: BTreeMap::new(),
            fastest: 0.0,
            reach: 0.0,
            gaits: BTreeMap::new(),
            scout_maps: BTreeMap::new(),
            annexes: BTreeMap::new(),
            relations: Vec::new(),
            unit_diplomacy: Vec::new(),
            task_0x99: Vec::new(),
            sight: None,
            vision: Vec::new(),
            terrain: None,
            search_cache: std::cell::RefCell::default(),
            grid: Grid::new(map),
        }
    }

    /// A unit AI's full update starts (`0x140c66b10`): the search cache lets go.
    pub fn forget_search(&self) {
        self.search_cache.borrow_mut().holds = false;
    }

    /// What the game keeps per tile.
    pub const fn tiles(&self) -> &Tiles {
        &self.tiles
    }

    /// The tiles, for a test's marks.
    #[cfg(test)]
    pub(crate) const fn tiles_mut_for_test(&mut self) -> &mut Tiles {
        &mut self.tiles
    }

    /// An object gets its obstruction record (`FUN_141056460` for a started foundation): a static
    /// one marks its tiles.
    pub fn add_record(&mut self, id: i32, o: Obstruction) {
        let Some(p) = self.objects.get_mut(&id) else {
            return;
        };
        p.obstruction = Some(o);
        if o.marks_tiles() {
            self.tiles.mark(p, o);
            self.changed.push(self.tiles.covered_rect(p));
        }
        self.refile(id);
    }

    /// Whether an object on tile (`x`, `y`) (its centre there) satisfies `pred`.
    pub fn object_on_tile(&self, x: i32, y: i32, pred: impl Fn(&Placed) -> bool) -> bool {
        self.objects
            .values()
            .any(|o| o.position[0] as i32 == x && o.position[1] as i32 == y && pred(o))
    }

    /// Object `id`'s place in its tile's object list ([`Spatial::objects_on`]'s order, ghosts
    /// counted); 0 for an object the world does not hold.
    #[must_use]
    pub fn tile_place(&self, id: i32) -> usize {
        self.objects.get(&id).map_or(0, |o| {
            let list = self.tiles.list(o.position[0] as i32, o.position[1] as i32);
            list.iter().position(|&t| t == id).unwrap_or(0)
        })
    }

    /// The map tile's object list (`+0x1b` count, `+0x28` array) of tile (`x`, `y`) in its
    /// order, the ghosts' ids among them; none off the map.
    #[must_use]
    pub fn tile_list(&self, x: i32, y: i32) -> &[i32] {
        self.tiles.list(x, y)
    }

    /// The objects whose centre is on tile (`x`, `y`): the map tile's object list (`+0x1b` count,
    /// `+0x28` array), in its order, without the ghosts.
    pub fn objects_on(&self, x: i32, y: i32) -> impl Iterator<Item = (i32, &Placed)> + '_ {
        self.tiles
            .list(x, y)
            .iter()
            .filter_map(|id| self.objects.get(id).map(|o| (*id, o)))
    }

    /// The object's obstruction record goes (`FUN_141055b40`'s record part), by its master's
    /// obstruction type (`+0x651`) whether or not it has its record yet: types 0 and 12 have
    /// none (false); types 0, 1, 2, 11, 13 and 10 (as 2) clear their tiles' marks
    /// ([`Spatial::unmark`]), after which type 1 keeps its record filed (true); any other type
    /// drops its record (true), false without one. The mesh shape's removal is the caller's
    /// (`Pathing::removed`).
    pub fn dropped_record(&mut self, id: i32) -> bool {
        let Some(p) = self.objects.get(&id).copied() else {
            return false;
        };
        let kind = p.obstruction_type;
        if kind == 0 || kind == 12 {
            return false;
        }
        let marks = Obstruction { kind, class: 0 }.marks_tiles();
        if marks {
            self.unmark(&p, id);
            if kind == 1 {
                return true;
            }
        }
        match self.objects.get_mut(&id) {
            Some(o) if o.obstruction.is_some() => {
                o.obstruction = None;
                self.refile(id);
                true
            }
            _ => false,
        }
    }

    /// [`Spatial::dropped_record`] for a caller that does not read its answer.
    pub fn drop_record(&mut self, id: i32) {
        let _ = self.dropped_record(id);
    }

    /// `FUN_141055980` for object `id`'s static record ([`Tiles::unmark`]): the planners hear of
    /// the covered tiles unless another object's static record on one of them stopped it.
    pub(super) fn unmark(&mut self, p: &Placed, id: i32) {
        let objects = &self.objects;
        let static_record = |o: i32| {
            objects
                .get(&o)
                .and_then(|q| q.obstruction)
                .is_some_and(|r| matches!(r.kind, 0 | 1 | 2 | 10 | 11 | 13))
        };
        if self.tiles.unmark(p, id, static_record) {
            self.changed.push(self.tiles.covered_rect(p));
        }
    }

    /// The object's master's height radius changed: it counts as tall or not from now on (a
    /// dying unit's own master copy has none, `FUN_14121cc40`); the tiles' counts are not
    /// redone, as the game does not redo them.
    pub fn set_tall(&mut self, id: i32, tall: bool) {
        if let Some(o) = self.objects.get_mut(&id) {
            o.tall = tall;
        }
    }

    /// The objects whose place lies within `x0`..`x1` and `y0`..`y1`, by id.
    pub fn objects_within(
        &self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
    ) -> impl Iterator<Item = (i32, &Placed)> + '_ {
        self.objects
            .iter()
            .filter(move |(_, o)| {
                let [x, y, _] = o.position;
                x0 <= x && x <= x1 && y0 <= y && y <= y1
            })
            .map(|(&id, o)| (id, o))
    }

    /// The tile rectangles whose static marks changed since the last call.
    pub fn take_changes(&mut self) -> Vec<[i16; 4]> {
        std::mem::take(&mut self.changed)
    }

    /// Player `player`'s masks both meet the unified map's bits of tile (`x`, `y`): what it sees
    /// (`+0x2a8`) and what it explored (`+0x2a0`). False off the map and without the players'
    /// sight.
    pub fn sees_and_explored(&self, player: usize, x: i32, y: i32) -> bool {
        let Some(v) = self.sight.as_ref() else {
            return false;
        };
        if x < 0 || y < 0 || v.width <= x || v.height <= y {
            return false;
        }
        let word = v.unified[(y * v.width + x) as usize] & crate::vision::registry::MASK;
        let [seen, explored] = self.vision(player);
        word & seen != 0 && word & explored != 0
    }

    /// The object `id`'s place, if the world holds it.
    pub fn get(&self, id: i32) -> Option<&Placed> {
        self.objects.get(&id).or_else(|| self.inside.get(&id))
    }

    /// An object whose place the slice knows: an object of a player the slice stopped is known
    /// while it cannot have moved.
    pub fn get_known(&self, id: i32) -> Result<Option<&Placed>, GenieError> {
        match self.get(id) {
            Some(p) if p.unknown.is_some_and(|u| 0.0 < u) => Err(GenieError::unsupported(
                "an object of a stopped player (its place is unknown)",
            )
            .with("object", id)
            .with("player", p.player)),
            found => Ok(found),
        }
    }

    /// The slice stops updating the object: from now on its place is known only within its speed
    /// times the time since.
    pub fn freeze(&mut self, id: i32) {
        if let Some(p) = self.objects.get_mut(&id) {
            let _ = p.unknown.get_or_insert(0.0);
            let _ = self.frozen_for.entry(id).or_insert(0.0);
        }
    }

    /// The object's place is the game's again (the oracle's forced replay): it is known.
    pub fn unfreeze(&mut self, id: i32) {
        if let Some(p) = self.objects.get_mut(&id) {
            p.unknown = None;
        }
        let _ = self.frozen_for.remove(&id);
    }

    /// A tick of `dt` seconds passed: every frozen object may have moved its speed times dt.
    pub fn grow_unknown(&mut self, dt: f32) {
        for p in self.objects.values_mut() {
            if let Some(u) = p.unknown.as_mut() {
                *u += p.speed * dt;
            }
        }
        for t in self.frozen_for.values_mut() {
            *t += dt;
        }
    }

    /// Fails when a frozen object may be within `rect` (`x0, y0, x1, y1`) or may have changed
    /// there: the answer of a query over it depends on where that object really is. A frozen
    /// object that does not move stays where it was; it can only have changed when a stopped
    /// player's unit (Gaia's animals do not work on objects) may have reached it: a frozen unit
    /// within what it may have moved, or a unit a stopped player's building may have made since,
    /// at the fastest speed of the players' units, each plus the farthest reach.
    pub fn check_known(&self, rect: [f32; 4]) -> Result<(), GenieError> {
        self.check_known_of(rect, false, None)
    }

    /// [`Spatial::check_known`] for a query over obstruction records and the mesh (the motion
    /// step's and the short pather's): a frozen object without a record (a bird) has no say.
    pub fn check_known_records(&self, rect: [f32; 4]) -> Result<(), GenieError> {
        self.check_known_of(rect, true, None)
    }

    /// [`Spatial::check_known`] for a query over the registry's lists of `classes`: a frozen
    /// object of another class has no say.
    pub fn check_known_classes(&self, rect: [f32; 4], classes: &[i32]) -> Result<(), GenieError> {
        self.check_known_of(rect, false, Some(classes))
    }

    pub(super) fn check_known_of(
        &self,
        rect: [f32; 4],
        records: bool,
        classes: Option<&[i32]>,
    ) -> Result<(), GenieError> {
        // only a frozen object has an unknown place, and each is in `frozen_for`
        let near = self.frozen().find(|&(id, p)| {
            if records && p.obstruction.is_none() {
                return false;
            }
            if classes.is_some_and(|c| !c.contains(&i32::from(p.class))) {
                return false;
            }
            p.unknown.is_some_and(|u| {
                let [x0, y0, x1, y1] = p.bounds();
                let inside = rect[0] <= x1 + u
                    && x0 - u <= rect[2]
                    && rect[1] <= y1 + u
                    && y0 - u <= rect[3];
                inside && (0.0 < u || self.touchable(id, [x0, y0, x1, y1]))
            })
        });
        if let Some((id, p)) = near {
            return Err(
                GenieError::unsupported("an object of a stopped player may be near")
                    .with("object", id)
                    .with("player", p.player)
                    .with(
                        "query",
                        format_args!("({}, {})..({}, {})", rect[0], rect[1], rect[2], rect[3]),
                    ),
            );
        }
        Ok(())
    }

    /// Whether a stopped player's unit may have reached the frozen object `id` within `rect`.
    pub(super) fn touchable(&self, id: i32, rect: [f32; 4]) -> bool {
        self.frozen().any(|(other, q)| {
            let Some(u) = q.unknown.filter(|_| other != id) else {
                return false;
            };
            if q.player == 0 {
                // Gaia's units (animals) do not work on objects
                return false;
            }
            let moved = if 0.0 < q.speed {
                u
            } else {
                self.fastest * self.frozen_for.get(&other).copied().unwrap_or(0.0)
            };
            let grow = moved + self.reach;
            let [x0, y0, x1, y1] = q.bounds();
            rect[0] <= x1 + grow
                && x0 - grow <= rect[2]
                && rect[1] <= y1 + grow
                && y0 - grow <= rect[3]
        })
    }

    /// The frozen objects (those with an unknown place), in id order.
    pub(super) fn frozen(&self) -> impl Iterator<Item = (i32, &Placed)> + '_ {
        self.frozen_for
            .keys()
            .filter_map(|&id| self.objects.get(&id).map(|p| (id, p)))
            .filter(|(_, p)| p.unknown.is_some())
    }

    /// A new object, in the order the game adds them: a static record marks only the tiles no
    /// earlier record marked.
    pub fn insert(&mut self, id: i32, placed: Placed) {
        self.remove(id);
        if placed.unknown.is_some() {
            let _ = self.frozen_for.entry(id).or_insert(0.0);
        }
        if let Some(o) = placed.obstruction.filter(|o| o.marks_tiles()) {
            self.tiles.mark(&placed, o);
            self.changed.push(self.tiles.covered_rect(&placed));
        }
        let _ = self.objects.insert(id, placed);
        self.refile(id);
        if let Some(i) = self.tiles.tile_of(placed.position) {
            let objects = &self.objects;
            self.tiles.append(i, id, |o| Self::is_tall(objects, o));
        }
    }

    /// Whether listed object `id` counts as tall (its master's height radius above 0, which
    /// `FUN_140f5f0a0` and `FUN_140f5f560` read): a ghost, whose master the slice does not hold,
    /// counts as not tall.
    pub(super) fn is_tall(objects: &BTreeMap<i32, Placed>, id: i32) -> bool {
        objects.get(&id).is_some_and(|p| p.tall)
    }

    /// The object's master changed (`FUN_1411a6a70`): it takes the new master's radii, class,
    /// obstruction and speed (`placed`, at its place), its static record's marks follow, and it
    /// keeps its slot in its tile's list and the tile's tall count, which the game does not redo.
    pub fn change_master(&mut self, id: i32, placed: Placed) {
        let Some(old) = self.objects.get(&id).copied() else {
            return self.insert(id, placed);
        };
        if self.tiles.tile_of(old.position) != self.tiles.tile_of(placed.position) {
            return self.insert(id, placed);
        }
        if old.obstruction.is_some_and(Obstruction::marks_tiles) {
            self.unmark(&old, id);
        }
        if let Some(o) = placed.obstruction.filter(|o| o.marks_tiles()) {
            self.tiles.mark(&placed, o);
            self.changed.push(self.tiles.covered_rect(&placed));
        }
        let _ = self.objects.insert(id, placed);
        self.refile(id);
    }

    /// The object leaves the world: its place, its sight's state and its records in the registry
    /// (its destructor, `FUN_141073060`) go.
    pub fn leave(&mut self, id: i32) -> Result<(), GenieError> {
        if let (Some(v), Some(p)) = (self.sight.as_mut(), self.objects.get(&id)) {
            v.registry_leave(id, p.player, p.position, p.radius, p.class)?;
            v.remove_seer(id);
        }
        self.remove(id);
        Ok(())
    }

    /// The object goes inside another at `at` (`FUN_1411921d0`): it leaves its tile's list
    /// (`FUN_140f5f560`) and the obstruction manager (`FUN_141055b40`), and takes its container's
    /// x and y (`FUN_14118c890`).
    pub fn go_inside(&mut self, id: i32, at: [f32; 3]) {
        let Some(mut p) = self.objects.get(&id).copied() else {
            return;
        };
        self.remove(id);
        p.position = [at[0], at[1], p.position[2]];
        p.obstruction = None;
        let _ = self.inside.insert(id, p);
    }

    /// The record of object `id` inside another ([`Spatial::go_inside`]) becomes `placed` (a master
    /// change inside, which touches no tile, list nor record); nothing for an object not inside.
    pub fn set_inside_record(&mut self, id: i32, placed: Placed) {
        if let Some(p) = self.inside.get_mut(&id) {
            *p = placed;
        }
    }

    /// The record of object `id` inside another takes the place `at` (its object's copy of the
    /// container's place); nothing for an object not inside.
    pub fn set_inside_position(&mut self, id: i32, at: [f32; 3]) {
        if let Some(p) = self.inside.get_mut(&id) {
            p.position = at;
        }
    }

    /// The object `id` comes out of the object it was inside ([`Spatial::go_inside`]): its record,
    /// which the caller puts on the map again.
    pub fn come_out(&mut self, id: i32) -> Option<Placed> {
        self.inside.remove(&id)
    }

    /// The object `id` leaves the world.
    pub fn remove(&mut self, id: i32) {
        let _ = self.inside.remove(&id);
        if let Some(gone) = self.objects.remove(&id) {
            self.refile(id);
            if let Some(i) = self.tiles.tile_of(gone.position) {
                let objects = &self.objects;
                self.tiles.take_off(i, id, |o| Self::is_tall(objects, o));
            }
            if gone.obstruction.is_some_and(Obstruction::marks_tiles) {
                self.unmark(&gone, id);
            }
        }
    }

    /// Whether the straight walk from `a` to `b` (x, y) of a unit `grow` wide meets an obstruction,
    /// as the routines that test one walk ask (`FUN_141052b90`, `FUN_141052350` with flags `0x100`,
    /// then `FUN_1410561f0`): the temporary records of the tiles its terrain mask lacks
    /// ([`Spatial::put_tile_records`]), tested ([`Spatial::segment_meets`]) and removed again.
    pub fn segment_blocked(
        &self,
        a: [f32; 2],
        b: [f32; 2],
        grow: f32,
        mask: u8,
        ignore: &[i32],
    ) -> bool {
        self.put_tile_records(a, b, grow, mask);
        let blocked = self.segment_meets(a, b, grow, ignore);
        self.clear_tile_records();
        blocked
    }

    /// `FUN_141052350` with flags `0x100`: whether the straight walk from `a` to `b` (x, y) of a
    /// unit `grow` wide (the manager's buffer) meets a record. A walk whose grown box leaves the
    /// map does. Else the records of the cells the query visits ([`Spatial::visits`], the walk's
    /// box grown by `grow`) whose id is not in `ignore` count when their box, grown by `grow`,
    /// meets the walk's box and segment (`FUN_141052e10`): the temporary tile records
    /// ([`Spatial::put_tile_records`], kind 3) and the objects' static records (kinds 2, 11 and
    /// 13; flag `0x100` passes the kinds of the mask `0x2804` and the records of no object).
    pub fn segment_meets(&self, a: [f32; 2], b: [f32; 2], grow: f32, ignore: &[i32]) -> bool {
        let Some(test) = WalkTest::new(a, b, grow, self.map) else {
            return true;
        };
        self.tile_records_meet(&test, ignore)
            || self.records_in(test.query).any(|(id, p)| {
                // kind 10 is filed as 2
                let kind = p.obstruction.map(|o| if o.kind == 10 { 2 } else { o.kind });
                matches!(kind, Some(2 | 11 | 13))
                    && !ignore.contains(&id)
                    && test.meets(p.bounds())
                    && self.visits(p, kind.unwrap_or(0), test.query)
            })
    }

    /// Whether a temporary tile record ([`Spatial::put_tile_records`]) of a cell the walk's
    /// query visits, not in `ignore`, meets the walk ([`WalkTest::meets`]). The records are filed
    /// on the level of the table's first entry (`+0x1bc`), in the cell of their tile.
    pub(super) fn tile_records_meet(&self, test: &WalkTest, ignore: &[i32]) -> bool {
        let level = self.grid.level(0);
        let shift = self.grid.shift(level);
        self.tile_records.borrow().iter().any(|&(y, x)| {
            self.visits_cell(level, x >> shift, y >> shift, test.query)
                && !ignore.contains(&tile_record_id(x, y))
                && test.meets([x as f32, y as f32, x as f32 + 1.0, y as f32 + 1.0])
        })
    }

    /// Whether a tile its terrain `mask` lacks lies in the way of a straight walk from `a` to
    /// `b` of a unit `grow` wide, as the walk's query asks (`FUN_14114cc60`: `FUN_141052b90`,
    /// its query, then `FUN_1410561f0`): its temporary tile records
    /// ([`Spatial::put_tile_records`]) tested as [`Spatial::segment_meets`] tests them, and
    /// removed again. A walk whose grown box leaves the map counts as blocked.
    pub fn tiles_block(&self, a: [f32; 2], b: [f32; 2], grow: f32, mask: u8) -> bool {
        self.put_tile_records(a, b, grow, mask);
        let blocked =
            WalkTest::new(a, b, grow, self.map).is_none_or(|t| self.tile_records_meet(&t, &[]));
        self.clear_tile_records();
        blocked
    }

    /// `FUN_141052b90`: a temporary record (`FUN_1410510c0`) on each tile of the rectangle of
    /// `a` and `b` (x, y), grown by `grow` (the manager's buffer) and held on the map, whose word
    /// holds neither a terrain class of `mask` nor a temporary record (`mask & 0x7f | 0x80`), row
    /// by row. The records stay until [`Spatial::clear_tile_records`].
    pub fn put_tile_records(&self, a: [f32; 2], b: [f32; 2], grow: f32, mask: u8) {
        use crate::math::conversions::cvttss2si;
        let (lo_x, hi_x) = (
            if b[0] <= a[0] { b[0] } else { a[0] },
            if a[0] <= b[0] { b[0] } else { a[0] },
        );
        let (lo_y, hi_y) = (
            if b[1] <= a[1] { b[1] } else { a[1] },
            if a[1] <= b[1] { b[1] } else { a[1] },
        );
        let [w, h] = self.map;
        let (lo_x, lo_y, hi_x, hi_y) = (
            cvttss2si(lo_x - grow),
            cvttss2si(lo_y - grow),
            cvttss2si(hi_x + grow),
            cvttss2si(hi_y + grow),
        );
        let (x0, y0) = (lo_x.max(0), lo_y.max(0));
        let x1 = if hi_x < w { hi_x } else { w - 1 };
        let y1 = if hi_y < h { hi_y } else { h - 1 };
        let impassable = u64::from(mask & 0x7f);
        let mut records = self.tile_records.borrow_mut();
        for y in y0..=y1 {
            for x in x0..=x1 {
                let Some(word) = self.tiles.word(x, y) else {
                    continue;
                };
                if word & (impassable | 0x80) == 0 {
                    let _ = records.insert((y, x));
                }
            }
        }
    }

    /// `FUN_1410561f0`: every temporary tile record ([`Spatial::put_tile_records`]) goes.
    pub fn clear_tile_records(&self) {
        self.tile_records.borrow_mut().clear();
    }

    /// The temporary tile records ([`Spatial::put_tile_records`]) as (x, y), row by row.
    #[must_use]
    pub fn tile_records(&self) -> Vec<[i32; 2]> {
        self.tile_records
            .borrow()
            .iter()
            .map(|&(y, x)| [x, y])
            .collect()
    }
}
