//! Where the world's objects are, as movement reads them: each live object's position, radii and
//! class, and the obstruction record the game keeps for it in its obstruction manager
//! (`DAT_143f0a088`, queried by `FUN_141051900`).
//!
//! The game files its records in a five-level grid; the slice keeps them by object id. The grid
//! only orders a query's hits, and the slice stops a walker at its first hit
//! ([`Spatial::walk_hits`]), so it needs no order yet.
//!
//! [`Tiles`] holds what the game keeps per tile: the obstruction manager's word (terrain class,
//! static records' marks) and the counts of records and objects whose centre is on the tile.

use crate::registry::{Record, relation_kind};
use crate::unit_ai::{Found, Pick, Search, Unsupported};
use crate::visibility::Visibility;
use std::collections::BTreeMap;

/// The range of `ClosestObject`'s reachability check (`FUN_140c488b0`, `DAT_142eb5690`).
const CLOSEST_RANGE: f32 = crate::image::spatial::CLOSEST_RANGE;

/// How near (squared) a unit AI's search comes to the last one's place to skip its query
/// (`FUN_140c63e20`).
const SEARCH_REUSE_SQUARED: f32 = crate::image::spatial::SEARCH_REUSE_SQUARED;

/// The pasture and livestock classes `ClosestObject` treats apart.
const PASTURE_CLASS: i32 = 49;
const LIVESTOCK_CLASS: i32 = 58;

/// The caller's reachability check of a found object (slot 0x3d0 within a range).
pub type Reach<'a> = dyn FnMut(i32, f32) -> Result<bool, Unsupported> + 'a;

/// What `ClosestObject` keeps of the objects found (its arguments 2 to 6; each −1 for any): the
/// master class, the relation (0 itself or friends, 1 neutrals, 2 enemies, 3 itself), the state,
/// the master and the cap of workers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Filter {
    group: i32,
    relation: i32,
    state: i32,
    master: i32,
    workers: i32,
}

/// Obstruction kinds the query tests (`FUN_141051e00`): boxes (2, 3, 11, 13) and circles (5).
const TESTED_KINDS: [u8; 5] = [2, 3, 5, 11, 13];

/// What the walk reads of a moving object it meets: whether it moves (its velocity, `+0x868`, is
/// not 0: slot 0x5c8, `FUN_1411583d0`), its last move time (`+0x870`, slot 0x5e8) and its current
/// path level's pause time (`+0x858`'s `+0x34`, slot 0x528), none without a level, and when that
/// level's path was made.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Gait {
    /// It moves: its velocity (`+0x868`) is not 0 (slot 0x5c8).
    pub moving: bool,
    /// Its last move time (`+0x870`, slot 0x5e8).
    pub last_move: u32,
    /// Its current path level's pause time (slot 0x528), none without a level.
    pub pause: Option<f32>,
    /// `+0x650`, `mLastHumanOrderTime`: a met unit's rank against a human walker.
    pub human_order: u32,
    /// When its current path level's path was made (slot 0x528's `+0x18`, the path's `+8`), none
    /// without a level or a path: a drop-off walker's ignore list passes a worker whose path is
    /// this update's.
    pub path_made: Option<u32>,
}

/// One object as movement sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// x, y, z on the map.
    pub position: [f32; 3],
    /// The master's radii on the ground (master `+0x5d8`, `+0x5dc`).
    pub radius: [f32; 2],
    /// The master's class (`+0x410`, decoded): the motion step treats gates (39) apart.
    pub class: i16,
    /// The owning player.
    pub player: usize,
    /// The master's id.
    pub master: i16,
    /// The master's `obstruction_type` (`+0x651`), whether or not the object has its record yet.
    pub obstruction_type: u8,
    /// The master's `can_be_built_on` (`+0x5e9`): a foundation clears such objects off its
    /// footprint.
    pub buildable_on: u8,
    /// The master's height radius (`+0x5e0`) is above 0: the tile counts it as tall.
    pub tall: bool,
    /// The top speed, tiles per second; 0 for an object that does not move.
    pub speed: f32,
    /// How far from `position` the object may be, when its player's update is not modelled: it
    /// grows by its speed every tick ([`Spatial::grow_unknown`]). None for an object the slice
    /// updates.
    pub unknown: Option<f32>,
    /// The obstruction record, if the object has one: a foundation gets its record only when
    /// construction starts.
    pub obstruction: Option<Obstruction>,
}

/// An obstruction record's kind (its byte `+1`, the master's `obstruction_type`, master `+0x651`)
/// and the master's `obstruction_class` (`+0x652`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Obstruction {
    /// The record's kind (`obstruction_type`).
    pub kind: u8,
    /// The master's `obstruction_class`.
    pub class: u8,
}

impl Obstruction {
    /// The kinds that mark the tiles under them (`FUN_141056460`: 0, 1, 2, 11, 13 and 10, which the
    /// game files as 2).
    const fn marks_tiles(self) -> bool {
        self.kind == 10 || (self.kind < 14 && (0x2807u32 >> self.kind) & 1 != 0)
    }
}

impl Placed {
    /// The record's box `x0, y0, x1, y1` (`+0x20`..`+0x2c`): the position plus and minus the radii.
    #[must_use]
    pub fn bounds(&self) -> [f32; 4] {
        let [x, y, _] = self.position;
        let [rx, ry] = self.radius;
        [x - rx, y - ry, x + rx, y + ry]
    }
}

/// The tile bit a static record sets (`0x4000`, in `FUN_141056460`): the tile is blocked.
pub const BLOCKED: u64 = 0x4000;

/// What the game keeps per tile, row by row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tiles {
    width: i32,
    height: i32,
    /// The obstruction manager's word (`DAT_143f0a088 + 0x98`): bits 0 to 7 the terrain's class,
    /// bits 8 to 13 a static record's obstruction class, [`BLOCKED`], bit 15 for class 5, bits 16
    /// to 22 the record's player, bits 23 and up a bit per player that marked the tile.
    words: Vec<u64>,
    /// Tall objects (a height radius above 0) whose centre is on the tile (the map's bytes at
    /// `+0x10`, `FUN_140f5f0a0`); the passability test queries the obstructions of a tile with two.
    tall: Vec<u8>,
    /// The objects whose centre is on the tile, in the tile's order (its list `+0x28`, count
    /// `+0x1b`): an object entering the tile goes last (`FUN_140f5f0a0`), one leaving it gives
    /// its slot to the last (`FUN_140f5f560`).
    lists: Vec<Vec<i32>>,
}

impl Tiles {
    /// A map whose tiles hold `classes` (one terrain class bit per tile, row by row) and nothing
    /// else.
    ///
    /// # Panics
    ///
    /// When `classes` does not hold one class per tile of the map.
    pub fn new(width: i32, height: i32, classes: Vec<u8>) -> Self {
        let n = classes.len();
        assert_eq!(
            n,
            (width.max(0) * height.max(0)) as usize,
            "one class per tile"
        );
        Self {
            width,
            height,
            words: classes.into_iter().map(u64::from).collect(),
            tall: vec![0; n],
            lists: vec![Vec::new(); n],
        }
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        (0 <= x && x < self.width && 0 <= y && y < self.height)
            .then(|| (y * self.width + x) as usize)
    }

    /// Tile (`x`, `y`)'s word: its terrain class and the obstruction manager's marks.
    #[must_use]
    pub fn word(&self, x: i32, y: i32) -> Option<u64> {
        self.index(x, y).map(|i| self.words[i])
    }

    /// How many tall objects stand on tile (`x`, `y`).
    #[must_use]
    pub fn tall(&self, x: i32, y: i32) -> Option<u8> {
        self.index(x, y).map(|i| self.tall[i])
    }

    /// How many objects stand on tile (`x`, `y`).
    #[must_use]
    pub fn objects(&self, x: i32, y: i32) -> Option<u8> {
        self.index(x, y).map(|i| self.lists[i].len() as u8)
    }

    /// The objects on tile (`x`, `y`) in the tile's order; none off the map.
    #[must_use]
    pub fn list(&self, x: i32, y: i32) -> &[i32] {
        self.index(x, y).map_or(&[], |i| &self.lists[i])
    }

    /// The tile of a centre, if it is on the map.
    fn tile_of(&self, position: [f32; 3]) -> Option<usize> {
        self.index(position[0] as i32, position[1] as i32)
    }

    /// The object goes last on tile `i` (`FUN_140f5f0a0`).
    fn append(&mut self, i: usize, id: i32) {
        self.lists[i].push(id);
    }

    /// The object leaves tile `i`: the last object takes its slot (`FUN_140f5f560`).
    fn take_off(&mut self, i: usize, id: i32) {
        if let Some(k) = self.lists[i].iter().position(|&o| o == id) {
            let _ = self.lists[i].swap_remove(k);
        }
    }

    /// A tall object on tile (`x`, `y`), for a test's map.
    #[cfg(test)]
    pub(crate) fn add_tall_for_test(&mut self, x: i32, y: i32) {
        let i = self.index(x, y).expect("a tile on the map");
        self.lists[i].push(i32::MIN);
        self.tall[i] += 1;
    }

    /// The tiles a static record covers (`FUN_141056460`): from the box's low edge to its high
    /// edge less 0.001, held on the map.
    fn covered(
        &self,
        p: &Placed,
    ) -> (std::ops::RangeInclusive<i32>, std::ops::RangeInclusive<i32>) {
        let [x, y, _] = p.position;
        let [rx, ry] = p.radius;
        let hold = |v: i32, size: i32| v.clamp(0, size - 1);
        let xs = hold((x - rx) as i32, self.width)..=hold(((x + rx) - 0.001) as i32, self.width);
        let ys = hold((y - ry) as i32, self.height)..=hold(((y + ry) - 0.001) as i32, self.height);
        (xs, ys)
    }

    /// The covered tiles as a rectangle, as the manager hands it to the planners' callback.
    fn covered_rect(&self, p: &Placed) -> [i16; 4] {
        let (xs, ys) = self.covered(p);
        [*xs.start(), *ys.start(), *xs.end(), *ys.end()].map(|v| v as i16)
    }

    /// A static record marks the tiles it covers that no player marked yet.
    fn mark(&mut self, p: &Placed, o: Obstruction) {
        let player = p.player as u64;
        let mut bits =
            ((player << 8 | u64::from(o.class & 0x3f) | 0x40) << 8) | (0x80_0000 << player);
        if o.class == 5 {
            bits |= 0x8000;
        }
        let (xs, ys) = self.covered(p);
        for y in ys {
            for x in xs.clone() {
                let i = self.index(x, y).expect("covered tiles are on the map");
                if self.words[i] < 0x80_0000 {
                    self.words[i] = (self.words[i] & !0xff_ff00) | bits;
                }
            }
        }
    }

    /// A static record leaves: its tiles keep only their terrain class (`FUN_141055980`).
    fn unmark(&mut self, p: &Placed) {
        let (xs, ys) = self.covered(p);
        for y in ys {
            for x in xs.clone() {
                let i = self.index(x, y).expect("covered tiles are on the map");
                self.words[i] &= 0xff;
            }
        }
    }
}

/// The world's objects by id, the map's size in tiles and what the game keeps per tile.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spatial {
    /// The map's size in tiles.
    pub map: [i32; 2],
    objects: BTreeMap<i32, Placed>,
    tiles: Tiles,
    /// The tile rectangles (x0, y0, x1, y1) whose static marks changed, for the planners
    /// ([`Spatial::take_changes`]).
    changed: Vec<[i16; 4]>,
    /// The seconds since each frozen object froze.
    frozen_for: BTreeMap<i32, f32>,
    /// What a stopped player's units may have done since: the fastest any of the players' unit
    /// masters (type 70) moves and the farthest any acts beyond its footprint (its weapon or work
    /// range), for [`Spatial::check_known`]. 0 when not set.
    pub fastest: f32,
    /// The farthest any of the players' units acts beyond its footprint.
    pub reach: f32,
    /// What the walk reads of the moving objects it meets ([`Gait`]).
    gaits: BTreeMap<i32, Gait>,
    /// Each annex's main building (`+0xbb8`), which the walk's avoidance takes for the annex.
    annexes: BTreeMap<i32, i32>,
    /// `relation[i]` of each player (`player+0x260`), for the walk's query's allies.
    pub relations: Vec<Vec<u8>>,
    /// What the players see, when the slice holds it: the objects' sight follows their tiles
    /// ([`Spatial::set_position`]).
    pub sight: Option<Visibility>,
    /// The map's terrain, when the slice holds it.
    pub terrain: Option<crate::terrain::Terrain>,
    /// The unit AIs' search cache ([`SearchCache`]).
    search_cache: std::cell::RefCell<SearchCache>,
}

/// The unit AIs' search cache (`FUN_140c63e20`): a search within [`SEARCH_REUSE_SQUARED`] (squared)
/// of the last one's place while the cache holds skips its query ("Already searched this update,
/// skipping second time.") and reads the registry's last answer, whichever unit's search made it;
/// a search that queries sets the place and holds the cache. Each unit AI's full update
/// (`0x140c66b10`, at its start) lets it go ([`Spatial::forget_search`]). Not saved: a slice
/// loaded from a save starts without one, as the game's load does.
#[derive(Clone, Debug, Default, PartialEq)]
struct SearchCache {
    holds: bool,
    at: [f32; 2],
    /// The registry's five output lists as the last query left them.
    last: Option<[Vec<Record>; 5]>,
}

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
            objects: BTreeMap::new(),
            tiles,
            changed: Vec::new(),
            frozen_for: BTreeMap::new(),
            fastest: 0.0,
            reach: 0.0,
            gaits: BTreeMap::new(),
            annexes: BTreeMap::new(),
            relations: Vec::new(),
            sight: None,
            terrain: None,
            search_cache: std::cell::RefCell::default(),
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
    }

    /// Whether an object on tile (`x`, `y`) (its centre there) satisfies `pred`.
    pub fn object_on_tile(&self, x: i32, y: i32, pred: impl Fn(&Placed) -> bool) -> bool {
        self.objects
            .values()
            .any(|o| o.position[0] as i32 == x && o.position[1] as i32 == y && pred(o))
    }

    /// The objects whose centre is on tile (`x`, `y`): the map tile's object list (`+0x1b` count,
    /// `+0x28` array), in its order, without the ghosts.
    pub fn objects_on(&self, x: i32, y: i32) -> impl Iterator<Item = (i32, &Placed)> + '_ {
        self.tiles
            .list(x, y)
            .iter()
            .filter_map(|id| self.objects.get(id).map(|o| (*id, o)))
    }

    /// The object's obstruction record goes (`FUN_141055b40`'s record part): a static record's
    /// tiles keep only their terrain class (`FUN_141055980`).
    pub fn drop_record(&mut self, id: i32) {
        let Some(p) = self.objects.get(&id).copied() else {
            return;
        };
        if p.obstruction.is_some_and(Obstruction::marks_tiles) {
            self.tiles.unmark(&p);
            self.changed.push(self.tiles.covered_rect(&p));
        }
        if let Some(o) = self.objects.get_mut(&id) {
            o.obstruction = None;
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

    /// Whether another foundation without its record (a master that obstructs, no record yet)
    /// overlaps `site`'s box (`FUN_1411cd0d0`'s test, without its players' visibility).
    pub fn foundations_overlapping(&self, site_id: i32, site: &Placed) -> bool {
        self.objects.iter().any(|(&id, o)| {
            id != site_id
                && o.obstruction.is_none()
                && o.obstruction_type != 0
                && (o.position[0] - site.position[0]).abs() < o.radius[0] + site.radius[0]
                && (o.position[1] - site.position[1]).abs() < o.radius[1] + site.radius[1]
        })
    }

    /// The tile rectangles whose static marks changed since the last call.
    pub fn take_changes(&mut self) -> Vec<[i16; 4]> {
        std::mem::take(&mut self.changed)
    }

    /// The object `id`'s place, if the world holds it.
    pub fn get(&self, id: i32) -> Option<&Placed> {
        self.objects.get(&id)
    }

    /// An object whose place the slice knows: an object of a player the slice stopped is known
    /// while it cannot have moved.
    pub fn get_known(&self, id: i32) -> Result<Option<&Placed>, Unsupported> {
        match self.objects.get(&id) {
            Some(p) if p.unknown.is_some_and(|u| 0.0 < u) => Err(Unsupported(
                "an object of a stopped player (its place is unknown)",
            )),
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
    pub fn check_known(&self, rect: [f32; 4]) -> Result<(), Unsupported> {
        self.check_known_of(rect, false, None)
    }

    /// [`Spatial::check_known`] for a query over obstruction records and the mesh (the motion
    /// step's and the short pather's): a frozen object without a record (a bird) has no say.
    pub fn check_known_records(&self, rect: [f32; 4]) -> Result<(), Unsupported> {
        self.check_known_of(rect, true, None)
    }

    /// [`Spatial::check_known`] for a query over the registry's lists of `classes`: a frozen
    /// object of another class has no say.
    pub fn check_known_classes(&self, rect: [f32; 4], classes: &[i32]) -> Result<(), Unsupported> {
        self.check_known_of(rect, false, Some(classes))
    }

    fn check_known_of(
        &self,
        rect: [f32; 4],
        records: bool,
        classes: Option<&[i32]>,
    ) -> Result<(), Unsupported> {
        let near = self.objects.iter().any(|(&id, p)| {
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
        if near {
            return Err(Unsupported("an object of a stopped player may be near"));
        }
        Ok(())
    }

    /// Whether a stopped player's unit may have reached the frozen object `id` within `rect`.
    fn touchable(&self, id: i32, rect: [f32; 4]) -> bool {
        self.objects.iter().any(|(&other, q)| {
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

    /// A new object, in the order the game adds them: a static record marks only the tiles no
    /// earlier record marked.
    pub fn insert(&mut self, id: i32, placed: Placed) {
        self.remove(id);
        if let Some(o) = placed.obstruction.filter(|o| o.marks_tiles()) {
            self.tiles.mark(&placed, o);
            self.changed.push(self.tiles.covered_rect(&placed));
        }
        let _ = self.objects.insert(id, placed);
        if let Some(i) = self.tiles.tile_of(placed.position) {
            self.tiles.append(i, id);
            self.recount(i);
        }
    }

    /// Tile `i`'s tall count after its list changed: the tall objects on it (`FUN_140f5f0a0` and
    /// `FUN_140f5f560` count them over the list, each by its master's height radius now; a
    /// ghost is not tall).
    fn recount(&mut self, i: usize) {
        let tall = self.tiles.lists[i]
            .iter()
            .filter(|id| self.objects.get(id).is_some_and(|p| p.tall))
            .count();
        self.tiles.tall[i] = tall as u8;
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
            self.tiles.unmark(&old);
            self.changed.push(self.tiles.covered_rect(&old));
        }
        if let Some(o) = placed.obstruction.filter(|o| o.marks_tiles()) {
            self.tiles.mark(&placed, o);
            self.changed.push(self.tiles.covered_rect(&placed));
        }
        let _ = self.objects.insert(id, placed);
    }

    /// The object leaves the world: its place, its sight's state and its records in the registry
    /// (its destructor, `FUN_141073060`) go.
    pub fn leave(&mut self, id: i32) -> Result<(), Unsupported> {
        if let (Some(v), Some(p)) = (self.sight.as_mut(), self.objects.get(&id)) {
            v.registry_leave(id, p.player, p.position, p.radius, p.class)?;
            v.remove_seer(id);
        }
        self.remove(id);
        Ok(())
    }

    /// The object `id` leaves the world.
    pub fn remove(&mut self, id: i32) {
        if let Some(gone) = self.objects.remove(&id) {
            if let Some(i) = self.tiles.tile_of(gone.position) {
                self.tiles.take_off(i, id);
                self.recount(i);
            }
            if gone.obstruction.is_some_and(Obstruction::marks_tiles) {
                self.tiles.unmark(&gone);
                self.changed.push(self.tiles.covered_rect(&gone));
            }
        }
    }

    /// Whether the straight walk from `a` to `b` (x, y) of a unit `grow` wide meets an obstruction,
    /// as the path smoothing asks (`FUN_141052b90`, then `FUN_141052350` with flags `0x100`): the
    /// tiles its terrain mask lacks ([`Spatial::tiles_block`]) and the static records (kinds 2, 11
    /// and 13) of objects not in `ignore`, each box grown by `grow`, against the walk's segment
    /// (`FUN_141052e10`).
    pub fn segment_blocked(
        &self,
        a: [f32; 2],
        b: [f32; 2],
        grow: f32,
        mask: u8,
        ignore: &[i32],
    ) -> bool {
        if self.tiles_block(a, b, grow, mask) {
            return true;
        }
        let walk = [a[0], a[1], b[0], b[1]];
        let [lo_x, lo_y, hi_x, hi_y] = [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[0].max(b[0]),
            a[1].max(b[1]),
        ];
        self.objects.iter().any(|(id, p)| {
            // kind 10 is filed as 2
            let kind = p.obstruction.map(|o| if o.kind == 10 { 2 } else { o.kind });
            let bx = p.bounds();
            let grown = [bx[0] - grow, bx[1] - grow, grow + bx[2], grow + bx[3]];
            matches!(kind, Some(2 | 11 | 13))
                && !ignore.contains(id)
                && lo_x <= grown[2]
                && grown[0] <= hi_x
                && lo_y <= grown[3]
                && grown[1] <= hi_y
                && box_meets_segment(bx, grow, walk)
        })
    }

    /// The tiles of a straight walk from `a` to `b` of a unit `grow` wide (`FUN_141052b90`, the
    /// temporary tile records of `FUN_1410510c0`): whether a tile of the walk's box whose terrain
    /// class the unit's `mask` lacks, grown by `grow`, meets the walk's segment. A walk whose grown
    /// box leaves the map counts as blocked.
    ///
    /// # Panics
    ///
    /// Never: the rectangle's tiles are held on the map.
    pub fn tiles_block(&self, a: [f32; 2], b: [f32; 2], grow: f32, mask: u8) -> bool {
        let (lo_x, hi_x) = (
            if b[0] <= a[0] { b[0] } else { a[0] },
            if a[0] <= b[0] { b[0] } else { a[0] },
        );
        let (lo_y, hi_y) = (
            if b[1] <= a[1] { b[1] } else { a[1] },
            if a[1] <= b[1] { b[1] } else { a[1] },
        );
        let [w, h] = self.map;
        let tx0 = ((lo_x - grow) as i32).max(0);
        let ty0 = ((lo_y - grow) as i32).max(0);
        let tx1 = if ((hi_x + grow) as i32) < w {
            (hi_x + grow) as i32
        } else {
            w - 1
        };
        let ty1 = if ((hi_y + grow) as i32) < h {
            (hi_y + grow) as i32
        } else {
            h - 1
        };
        if lo_x - grow < 0.0
            || lo_y - grow < 0.0
            || (w as f32) < grow + hi_x
            || (h as f32) < grow + hi_y
        {
            return true;
        }
        let walk = [a[0], a[1], b[0], b[1]];
        let meets = |bx: [f32; 4]| {
            let grown = [bx[0] - grow, bx[1] - grow, grow + bx[2], grow + bx[3]];
            lo_x <= grown[2]
                && grown[0] <= hi_x
                && lo_y <= grown[3]
                && grown[1] <= hi_y
                && box_meets_segment(bx, grow, walk)
        };
        let impassable = u64::from(mask & 0x7f | 0x80);
        for y in ty0..=ty1 {
            for x in tx0..=tx1 {
                let word = self.tiles.word(x, y).expect("a tile on the map");
                if word & impassable == 0
                    && meets([x as f32, y as f32, x as f32 + 1.0, y as f32 + 1.0])
                {
                    return true;
                }
            }
        }
        false
    }

    /// Each obstruction record: the object's id, the record's kind and box.
    pub fn records(&self) -> impl Iterator<Item = (i32, u8, [f32; 4])> + '_ {
        self.objects
            .iter()
            .filter_map(|(&id, p)| Some((id, p.obstruction?.kind, p.bounds())))
    }

    /// The object moved: its record moves with it, and it changes tiles when its centre crosses
    /// a tile's edge (`RGE_Static_Object::new_position`, `0x1411a58b0`).
    /// What the walk reads of a moving object now, None for one without movement state.
    pub fn set_gait(&mut self, id: i32, gait: Option<Gait>) {
        match gait {
            Some(g) => {
                let _ = self.gaits.insert(id, g);
            }
            None => {
                let _ = self.gaits.remove(&id);
            }
        }
    }

    /// What the walk reads of the moving object `id`.
    pub fn gait(&self, id: i32) -> Option<Gait> {
        self.gaits.get(&id).copied()
    }

    /// An annex's main building, or none.
    pub fn set_annex(&mut self, id: i32, main: Option<i32>) {
        match main {
            Some(m) => {
                let _ = self.annexes.insert(id, m);
            }
            None => {
                let _ = self.annexes.remove(&id);
            }
        }
    }

    /// The main building of the annex `id`, if it is one.
    pub fn annex_of(&self, id: i32) -> Option<i32> {
        self.annexes.get(&id).copied()
    }

    /// `FUN_141663dc0` with the ally flag ([`Spatial::allied`]), for the walk's avoidance.
    pub fn allied_with(&self, from: usize, to: usize) -> bool {
        self.allied(from, to)
    }

    /// `FUN_141663dc0` with the ally flag: `to` counts for `from` when it is `from`, or `from`
    /// stands to it in relation 0 or 4; Gaia counts for no one else.
    fn allied(&self, from: usize, to: usize) -> bool {
        if from == to {
            return true;
        }
        if to == 0 {
            return false;
        }
        matches!(
            self.relations.get(from).and_then(|r| r.get(to)),
            Some(0 | 4)
        )
    }

    /// The walk's obstruction query (`FUN_14114cc60` -> `FUN_141051900` with flags `0x90`, each
    /// cell's records by `FUN_141051e00`): the records of a tested kind (kind 10 filed as 2),
    /// except the `ignore`d objects', whose box, grown by `grow` (the walker's radius), meets the
    /// step's rectangle `rect` and whose grown box meets the step `from` -> `to`
    /// (`FUN_1410512f0`; a circle's, kind 5, `FUN_141051630`, alike). A moving object of
    /// `player` or an ally grows by `grow × ally_factor` instead (`+0x84`, the walker's master's
    /// `min_collision_size_multiplier`). The ids hit, in id order; the game's are in its grid's,
    /// which no caller reads yet. A rectangle that, grown, leaves the map adds the map edge's
    /// records (`FUN_141057290`), which is not modelled.
    #[allow(clippy::too_many_arguments)]
    pub fn walk_hits(
        &self,
        from: [f32; 2],
        to: [f32; 2],
        rect: [f32; 4],
        grow: f32,
        ally_factor: f32,
        player: Option<usize>,
        ignore: &[i32],
    ) -> Result<Vec<i32>, Unsupported> {
        let [w, h] = self.map;
        if rect[0] - grow < 0.0
            || rect[1] - grow < 0.0
            || (w as f32) < grow + rect[2]
            || (h as f32) < grow + rect[3]
        {
            return Err(Unsupported(
                "a walk query beyond the map's edge (FUN_141057290)",
            ));
        }
        let walk = [from[0], from[1], to[0], to[1]];
        let mut hits = Vec::new();
        for (&id, p) in &self.objects {
            let Some(o) = p.obstruction else {
                continue;
            };
            let kind = if o.kind == 10 { 2 } else { o.kind };
            if !TESTED_KINDS.contains(&kind) || ignore.contains(&id) {
                continue;
            }
            let g = if self.gaits.get(&id).is_some_and(|g| g.moving) {
                let player = player.ok_or(Unsupported("a walker without a place"))?;
                if self.allied(p.player, player) {
                    ally_factor * grow
                } else {
                    grow
                }
            } else {
                grow
            };
            let [x0, y0, x1, y1] = p.bounds();
            let grown = [x0 - g, y0 - g, g + x1, g + y1];
            if rect[0] <= grown[2]
                && grown[0] <= rect[2]
                && rect[1] <= grown[3]
                && grown[1] <= rect[3]
                && box_meets_segment([x0, y0, x1, y1], g, walk)
            {
                hits.push(id);
            }
        }
        Ok(hits)
    }

    /// The object's new place (`new_position`, `0x1411a58b0`): its tile's counts and, on a new
    /// tile, its sight follow.
    pub fn set_position(&mut self, id: i32, position: [f32; 3]) {
        if let Some(p) = self.objects.get_mut(&id) {
            let from = p.position;
            p.position = position;
            let (old, new) = (self.tiles.tile_of(from), self.tiles.tile_of(position));
            if old != new {
                if let Some(i) = old {
                    self.tiles.take_off(i, id);
                    self.recount(i);
                }
                if let Some(i) = new {
                    self.tiles.append(i, id);
                    self.recount(i);
                }
            }
            let Some(p) = self.objects.get(&id) else {
                return;
            };
            if let Some(v) = self.sight.as_mut() {
                v.moved(id, from, position, p.radius);
                v.registry_moved(id, p.class, from, position);
            }
        }
    }

    /// Fails when a frozen object of `player` that may have moved sees, or saw, near `rect`: the
    /// player's visible map there, and the registry's records the objects' masks keep, may differ
    /// from the game's (its sight, plus a tile, beyond what it may have moved).
    pub fn check_sight(&self, player: usize, rect: [f32; 4]) -> Result<(), Unsupported> {
        let Some(v) = self.sight.as_ref() else {
            return Ok(());
        };
        let near = self.objects.iter().any(|(&id, p)| {
            let Some(u) = p.unknown.filter(|&u| 0.0 < u && p.player == player) else {
                return false;
            };
            let los = v.seer(id).map_or(0, |s| s.los as i32);
            if los <= 0 {
                return false;
            }
            let grow = u + los as f32 + 1.0;
            let [x0, y0, x1, y1] = p.bounds();
            rect[0] <= x1 + grow
                && x0 - grow <= rect[2]
                && rect[1] <= y1 + grow
                && y0 - grow <= rect[3]
        });
        if near {
            return Err(Unsupported("a stopped object's sight may reach the search"));
        }
        Ok(())
    }

    /// A unit AI's search ([`Search`]) for object `me` of `player` (`FUN_140c63e20`): the query
    /// over the player's registry ([`crate::registry::Registry::visible_units`]) unless the
    /// [`SearchCache`] holds near the place (`fresh` false: the last answer stands); then its
    /// [`Pick`], an attack by the unit AI's attack check `can_attack` of an object, its master and
    /// its owner. A query that reads nothing leaves the last answer too (and so is not fresh). A
    /// query near a frozen object, or a last answer the slice has not got, fails.
    pub fn search(
        &self,
        me: i32,
        player: usize,
        search: &Search<'_>,
        can_attack: &dyn Fn(&Placed, u8) -> Result<bool, Unsupported>,
        reach: &mut Reach<'_>,
        moving: &dyn Fn(i32) -> bool,
    ) -> Result<Found, Unsupported> {
        let v = self
            .sight
            .as_ref()
            .ok_or(Unsupported("a search without the players' sight"))?;
        let r = v.registry.as_ref().ok_or(Unsupported(
            "a search without the objects-in-sight registry",
        ))?;
        let own = self
            .relations
            .get(player)
            .ok_or(Unsupported("a search of a player without relations"))?;
        let relations = (0..r.players.min(own.len()))
            .map(|o| relation_kind(player, o, own[o]))
            .collect::<Result<Vec<u32>, _>>()?;
        let [x, y] = search.at;
        let mut fresh = false;
        {
            let mut cache = self.search_cache.borrow_mut();
            let (dx, dy) = (x - cache.at[0], y - cache.at[1]);
            if SEARCH_REUSE_SQUARED < dy * dy + dx * dx || !cache.holds {
                cache.holds = true;
                cache.at = search.at;
                let radius = search.radius.min(1000.0);
                let [rx, ry] = search.size;
                let rect = [
                    (x - (radius + rx)) - 0.001,
                    (y - (radius + ry)) - 0.001,
                    x + (radius + rx) + 0.001,
                    y + (radius + ry) + 0.001,
                ];
                self.check_known_classes(rect, search.classes)?;
                self.check_sight(player, rect)?;
                if let Some(lists) = r.visible_units(
                    player,
                    search.at,
                    search.radius,
                    0,
                    search.classes,
                    &relations,
                    search.size,
                    0xff,
                )? {
                    cache.last = Some(lists);
                    fresh = true;
                }
            }
        }
        let cache = self.search_cache.borrow();
        let found = cache.last.as_ref().ok_or(Unsupported(
            "a search that reads the registry's last answer (before any query)",
        ))?;
        match search.pick {
            Pick::Closest {
                group,
                relation,
                state,
                master,
                workers,
            } => {
                let filter = Filter {
                    group,
                    relation,
                    state,
                    master,
                    workers,
                };
                let id = self.closest_object(v, me, player, found, filter, reach, moving)?;
                Ok(Found {
                    id,
                    enemies: !found[4].is_empty(),
                    position: self.objects.get(&id).map(|p| p.position),
                    fresh,
                })
            }
            Pick::Look => Ok(Found {
                id: -1,
                enemies: !found[4].is_empty(),
                position: None,
                fresh,
            }),
            Pick::Attack { lowest } => {
                let id = self.best_unit_to_attack(v, own, player, found, lowest, can_attack)?;
                Ok(Found {
                    id,
                    enemies: false,
                    position: self.objects.get(&id).map(|p| p.position),
                    fresh,
                })
            }
        }
    }

    /// `ClosestObject` (`FUN_140c488b0`) for object `me` over what its search `found`, its
    /// reachability check on (argument 8, every caller's): each record by relation and order:
    /// itself, an object that is gone and one dying (state 7 and up) are passed, so is one the
    /// [`Filter`] leaves out; then one the unit reaches within `CLOSEST_RANGE` (slot 0x3d0,
    /// `reach`) and nearer than the nearest so far (by the search's distance; a tie goes to the
    /// lower id) is the nearest. Livestock (class 58) counts when the searcher's player owns it or
    /// it is not in state 2, and not while its top action is the Artifact with a point
    /// (`moving`: slot 0x288 not −1 and slot 0x530 107, "Sheep is moving"). Not modelled: a
    /// workers cap (argument 6; the objects' `+0x644`), the unbuilt farm test of a search for
    /// state 0, the pastures and farms (class 49), a searcher of class 10 (with its test of
    /// master 440 and the object flag 0x10) and the game's `+0x6b4`. Nothing found answers −1.
    #[allow(clippy::too_many_arguments)]
    fn closest_object(
        &self,
        v: &Visibility,
        me: i32,
        player: usize,
        found: &[Vec<Record>; 5],
        filter: Filter,
        reach: &mut Reach<'_>,
        moving: &dyn Fn(i32) -> bool,
    ) -> Result<i32, Unsupported> {
        let mut best: Option<(i32, f32)> = None;
        for (kind, records) in found.iter().enumerate() {
            // the relation filter (`param_3`): 0 friends and itself, 1 neutrals, 2 enemies, 3
            // itself, else any
            let related = match filter.relation {
                0 => matches!(kind, 1 | 2),
                1 => kind == 3,
                2 => kind == 4,
                3 => kind == 1,
                _ => true,
            };
            for record in records {
                if record.id == me {
                    continue;
                }
                let (Some(seer), Some(p)) = (v.seer(record.id), self.objects.get(&record.id))
                else {
                    continue;
                };
                if 7 <= seer.state || !related {
                    continue;
                }
                if filter.group != -1 && i32::from(p.class) != filter.group {
                    continue;
                }
                if filter.master != -1 && i32::from(p.master) != filter.master {
                    continue;
                }
                if filter.state != -1 && i32::from(seer.state) != filter.state {
                    continue;
                }
                if filter.workers != -1 {
                    return Err(Unsupported("ClosestObject's workers cap (+0x644)"));
                }
                if filter.state == 0 {
                    return Err(Unsupported("ClosestObject's unbuilt farm test"));
                }
                if filter.group == PASTURE_CLASS {
                    return Err(Unsupported("ClosestObject over pastures or farms"));
                }
                if filter.group == LIVESTOCK_CLASS
                    && ((p.player != player && seer.state == 2) || moving(record.id))
                {
                    continue;
                }
                if self.objects.get(&me).is_some_and(|m| m.class == 10) {
                    return Err(Unsupported("ClosestObject of a class-10 searcher"));
                }
                if !reach(record.id, CLOSEST_RANGE)? {
                    continue;
                }
                match best {
                    Some((id, d)) if d <= record.distance => {
                        if record.distance == d && record.id < id {
                            best = Some((record.id, d));
                        }
                    }
                    _ => best = Some((record.id, record.distance)),
                }
            }
        }
        Ok(best.map_or(-1, |(id, _)| id))
    }

    /// `bestUnitToAttack` (`FUN_140c44680`) for a unit of `player` (diplomacy `own`, its
    /// `player+0x260`) without a current target or extra objects, over what its search `found`
    /// (which sets the alert mode on enemies found: not modelled, nor any enemy record). The
    /// relations from enemies down to `lowest`, each record in order: an object that is gone, dead
    /// (state 3 and up), allied (`FUN_141663d90` with the ally flag) or one the unit AI may not
    /// attack (slot 0xa0, `can_attack` with the object's `+0x924`) is passed; so would be one
    /// without a unit AI (`+0x6b0`) or inside another (`+0x2f0`), or in the unit AI's retarget
    /// entries (not held, the save's: empty in every unit probed). The checks after those (the
    /// classes, the distances, the statistics, the paths) are not modelled. Nothing found answers
    /// −1 (no wall to fall back on).
    fn best_unit_to_attack(
        &self,
        v: &Visibility,
        own: &[u8],
        player: usize,
        found: &[Vec<Record>; 5],
        lowest: usize,
        can_attack: &dyn Fn(&Placed, u8) -> Result<bool, Unsupported>,
    ) -> Result<i32, Unsupported> {
        if !found[4].is_empty() {
            return Err(Unsupported("a search that finds enemies (the alert mode)"));
        }
        for records in found[lowest.min(4)..].iter().rev() {
            for record in records {
                let (Some(seer), Some(p)) = (v.seer(record.id), self.objects.get(&record.id))
                else {
                    continue;
                };
                let owner = usize::from(record.owner);
                if 3 <= seer.state || crate::tasks::allied(own, player, owner, true) {
                    continue;
                }
                if !can_attack(p, seer.capture)? {
                    continue;
                }
                return Err(Unsupported(
                    "bestUnitToAttack's checks of an object it may attack",
                ));
            }
        }
        Ok(-1)
    }

    /// The Artifact action's owner search (`FUN_140aabc40`) for the animal `me`: the objects
    /// whose centre is on a tile within the animal's radius (whole) plus 3 tiles of its own tile,
    /// combat units (type 70) in state 2 of a player that is not out (`+0x89`, 0 for every player
    /// probed), Gaia's only with `+0x924` 0, of no class of 1, 60, 42, 11, 14, 30, 58, 3, 52, 27,
    /// 39, 49, 37, 9, 10 or 38, counted per player with the nearest footprint gap (squared). A
    /// unit of its own owner keeps it, unless that owner's attribute 97 (`attribute_97`, by
    /// player) is above 0 and it is livestock (class 58), which then takes it from nobody; a
    /// player whose attribute 97 is above 0 counts 100 more (the most units then win). One other
    /// player takes it; of several, the nearest (Gaia never). A frozen unit that counts and may
    /// have moved onto the tiles fails it.
    pub fn artifact_owner(
        &self,
        me: i32,
        attribute_97: &[f32],
    ) -> Result<Option<usize>, Unsupported> {
        const KEPT: [i16; 16] = [1, 60, 42, 11, 14, 30, 58, 3, 52, 27, 39, 49, 37, 9, 10, 38];
        // a world without the players' sight (the unit tests') holds nobody near
        let Some(v) = self.sight.as_ref() else {
            return Ok(None);
        };
        let p = self.objects.get(&me).ok_or(Unsupported(
            "an owner search of an object the world does not hold",
        ))?;
        let reach = (p.radius[0] as i32 as i16).wrapping_add(3);
        let [x, y, _] = p.position;
        let (x0, x1) = (
            (x as i32 as i16).wrapping_sub(reach),
            (x as i32 as i16).wrapping_add(reach),
        );
        let (y0, y1) = (
            (y as i32 as i16).wrapping_sub(reach),
            (y as i32 as i16).wrapping_add(reach),
        );
        let (x0, y0) = (i32::from(x0.max(0)), i32::from(y0.max(0)));
        let x1 = i32::from(x1).min(self.map[0] - 1);
        let y1 = i32::from(y1).min(self.map[1] - 1);
        // a frozen unit that could count (of the combat type and a class that counts) may have
        // moved onto the tiles
        let area = [x0 as f32, y0 as f32, (x1 + 1) as f32, (y1 + 1) as f32];
        let unsure = self.objects.iter().any(|(&id, q)| {
            q.unknown.is_some_and(|u| {
                let counts = v.seer(id).is_some_and(|s| s.kind == 70) && !KEPT.contains(&q.class);
                let [bx0, by0, bx1, by1] = q.bounds();
                counts
                    && 0.0 < u
                    && area[0] <= bx1 + u
                    && bx0 - u <= area[2]
                    && area[1] <= by1 + u
                    && by0 - u <= area[3]
            })
        });
        if unsure {
            return Err(Unsupported(
                "an owner search near a stopped unit that counts",
            ));
        }
        let players = self.relations.len().max(attribute_97.len());
        let mut counts = vec![0i32; players];
        let mut nearest = vec![999.0f32; players];
        let mut own_seen = false;
        let mut taker = false;
        let livestock = p.class == 58;
        for (&id, q) in &self.objects {
            let (tx, ty) = (q.position[0] as i32, q.position[1] as i32);
            if !(x0 <= tx && tx <= x1 && y0 <= ty && ty <= y1) {
                continue;
            }
            let Some(s) = v.seer(id) else { continue };
            if s.kind != 70 || s.state != 2 || KEPT.contains(&q.class) {
                continue;
            }
            if q.player == 0 && s.capture != 0 {
                continue;
            }
            let o = q.player;
            if players <= o {
                return Err(Unsupported(
                    "an owner search's unit of a player beyond the list",
                ));
            }
            let strong = livestock && attribute_97.get(o).is_some_and(|&a| 0.0 < a);
            if o == p.player {
                if strong {
                    return Ok(None);
                }
                own_seen = true;
                counts[o] += 1;
            } else {
                counts[o] += 1;
                if strong {
                    taker = true;
                    counts[o] += 100;
                }
            }
            let gap =
                crate::movement::footprint_gap_squared(q.position, q.radius, p.position, p.radius);
            if gap < nearest[o] {
                nearest[o] = gap;
            }
        }
        if own_seen && !taker {
            return Ok(None);
        }
        let takers: Vec<usize> = (0..players).filter(|&o| 0 < counts[o]).collect();
        match takers[..] {
            [] => Ok(None),
            [one] => Ok(Some(one)),
            _ if !taker => {
                let mut best = (999.0f32, None);
                for (o, &d) in nearest.iter().enumerate() {
                    if d < best.0 {
                        best = (d, Some(o));
                    }
                }
                Ok(best.1.filter(|&o| 0 < o))
            }
            _ => {
                let mut best = (0, None);
                for (o, &n) in counts.iter().enumerate() {
                    if best.0 < n {
                        best = (n, Some(o));
                    }
                }
                Ok(best.1)
            }
        }
    }

    /// An ownership change's world part (`FUN_14120f800`): the object's place and records take
    /// the new owner, and its sight moves to the new owner's map ([`Visibility::owner_changed`]).
    pub fn change_owner(&mut self, id: i32, new: usize, los: f32) -> Result<(), Unsupported> {
        let Some(p) = self.objects.get_mut(&id) else {
            return Err(Unsupported(
                "an ownership change of an object the world does not hold",
            ));
        };
        p.player = new;
        let (position, radius) = (p.position, p.radius);
        if let Some(v) = self.sight.as_mut() {
            v.owner_changed(id, position, radius, new, los)?;
        }
        Ok(())
    }

    /// Whether player `player` sees tile (`x`, `y`) (`FUN_141663500` returning 0xf): its bit of
    /// the unified map (its `+0x2a8` mask is its bit and bit 29, which no tile has; bit 61 neither).
    pub fn sees_tile(&self, player: usize, x: i32, y: i32) -> bool {
        let Some(v) = self.sight.as_ref() else {
            return false;
        };
        if x < 0 || y < 0 || v.width <= x || v.height <= y {
            return false;
        }
        v.unified[(y * v.width + x) as usize] & (1u64 << player) != 0
    }

    /// `FUN_141663500` is not 0: player `player` sees tile (`x`, `y`) or saw it (its bit, or its
    /// bit 32 up, of the unified map; the player's masks `+0x2a8` and `+0x2a0` add bits 29 and
    /// 61, which no tile has). Unknown without the players' sight.
    pub fn explored(&self, player: usize, x: i32, y: i32) -> Result<bool, Unsupported> {
        let v = self.sight.as_ref().ok_or(Unsupported(
            "a test of explored tiles without the players' sight",
        ))?;
        if x < 0 || y < 0 || v.width <= x || v.height <= y {
            return Ok(false);
        }
        let bits = (1u64 << player) | (1u64 << (32 + player));
        Ok(v.unified[(y * v.width + x) as usize] & bits != 0)
    }

    /// `FUN_140c68a00` gives 0xf: player `player` sees tile (`x`, `y`) now (its bit of the
    /// unified map; the player's mask `+0x2a8` adds bit 29, which no tile has). False off the map
    /// and without the players' sight.
    pub fn visible(&self, player: usize, x: i32, y: i32) -> bool {
        let Some(v) = self.sight.as_ref() else {
            return false;
        };
        if x < 0 || y < 0 || v.width <= x || v.height <= y {
            return false;
        }
        v.unified[(y * v.width + x) as usize] & (1u64 << player) != 0
    }

    /// `RGE_Combat_Object::update`'s sight ([`Visibility::combat_sight`]); an annex reads its
    /// main building's place and UMV entry.
    pub fn combat_sight(&mut self, id: i32) -> Result<(), Unsupported> {
        let (Some(v), Some(p)) = (self.sight.as_mut(), self.objects.get(&id)) else {
            return Ok(());
        };
        let main = self.annexes.get(&id).and_then(|main| {
            let place = self.objects.get(main)?.position;
            Some((place, v.seer(*main).map_or(0, |s| s.umv)))
        });
        v.combat_sight(id, p.player, p.position, p.radius, p.class, main)
    }

    /// The object's `set_state` to `state` ([`Visibility::state_changed`]).
    pub fn sight_state(&mut self, id: i32, state: u8) {
        if let (Some(v), Some(p)) = (self.sight.as_mut(), self.objects.get(&id)) {
            v.state_changed(id, p.position, p.radius, state);
        }
    }

    /// The object's master changes to `master` with radii `radius`, class `class` and line of
    /// sight `los` (`FUN_14108c990`: its records leave the registry's lists of its old class,
    /// [`Visibility::master_changed`], and join those of the new one); the caller files the
    /// object's new radii and class.
    pub fn sight_master(
        &mut self,
        id: i32,
        radius: [f32; 2],
        class: i16,
        master: i16,
        los: f32,
    ) -> Result<(), Unsupported> {
        if let (Some(v), Some(p)) = (self.sight.as_mut(), self.objects.get(&id)) {
            v.master_changed(id, p.position, p.radius, radius, master, los);
            v.registry_master(
                id,
                p.player,
                p.position,
                (p.radius, p.class),
                (radius, class),
            )?;
        }
        Ok(())
    }

    /// The first part of the players' sight the slice met and does not model.
    pub fn take_sight_unsupported(&mut self) -> Option<Unsupported> {
        self.sight.as_mut().and_then(Visibility::take_unsupported)
    }

    /// A player's ghost `id` of an object it saw (a doppelganger, type 25): it is on its tile's
    /// list, and movement sees nothing else of it. The slice keeps the ghosts of the save and
    /// makes none.
    pub fn insert_ghost(&mut self, id: i32, position: [f32; 3]) {
        if let Some(i) = self.tiles.tile_of(position) {
            self.tiles.append(i, id);
        }
    }

    /// The tiles' lists in the game's order (`x`, `y`, ids), each a tile of two objects or more
    /// (a single object's tile follows from the positions); the order is the game's history,
    /// which no save holds. Returns the tiles whose objects the slice does not hold as the game
    /// does (the same ids, in any order), with the slice's list; those keep the slice's.
    pub fn force_tile_lists(
        &mut self,
        lists: &[(i32, i32, Vec<i32>)],
    ) -> Vec<(i32, i32, Vec<i32>)> {
        let mut differ = Vec::new();
        for (x, y, ids) in lists {
            let Some(i) = self.tiles.index(*x, *y) else {
                differ.push((*x, *y, Vec::new()));
                continue;
            };
            let mut mine = self.tiles.lists[i].clone();
            let mut theirs = ids.clone();
            mine.sort_unstable();
            theirs.sort_unstable();
            if mine == theirs {
                self.tiles.lists[i].clone_from(ids);
            } else {
                differ.push((*x, *y, self.tiles.lists[i].clone()));
            }
        }
        differ
    }

    /// Whether a unit with terrain mask `mask` can stand on tile (`x`, `y`) (`FUN_14114d460`): the
    /// tile is on the map, its class is in the mask and no static record blocks it. A tile with
    /// two tall objects needs the game's point query, which the slice does not model.
    ///
    /// # Panics
    ///
    /// Never: the tall-object grid covers the map as the word grid does.
    pub fn passable(&self, x: i32, y: i32, mask: u8) -> Result<bool, Unsupported> {
        let Some(word) = self.tiles.word(x, y) else {
            return Ok(false);
        };
        if word & (u64::from(mask ^ 0xff) | BLOCKED) != 0 {
            return Ok(false);
        }
        if 1 < self.tiles.tall(x, y).expect("a tile with a word") {
            return Err(Unsupported(
                "a tile with two tall objects (its point query)",
            ));
        }
        Ok(true)
    }

    /// The tile beside `site`'s footprint a unit at `from` with terrain mask `mask` goes to
    /// (`FUN_1411566b0`, closest): a flood over passable tiles from the footprint's own, at most
    /// `(int)(rx + ry) + 1` tiles from its centre; of the tiles found outside the footprint, the
    /// first empty one, else the closest to the unit, nearer ones replacing it in the flood's
    /// order. Returns its centre.
    pub fn spot_beside(
        &self,
        from: [f32; 3],
        mask: u8,
        site: &Placed,
    ) -> Result<Option<[f32; 2]>, Unsupported> {
        let [tx, ty, _] = site.position;
        let [rx, ry] = site.radius;
        let size = (((ry + rx) as i32) + 2).max(1) * 2;
        let half = size / 2;
        let (cx, cy) = (tx as i32, ty as i32);
        let (fx, fy) = ((cx - half) as f32, (cy - half) as f32);
        self.check_known([fx, fy, fx + size as f32, fy + size as f32])?;
        let (ox, oy) = (half - cx, half - cy);
        let (x0, y0) = ((tx - rx) as i32, (ty - ry) as i32);
        let (x1, y1) = (((rx + tx) - 0.001) as i32, ((ry + ty) - 0.001) as i32);
        let mut list: Vec<(i32, i32)> = (x0..=x1)
            .flat_map(|x| (y0..=y1).map(move |y| (x, y)))
            .collect();
        let mut visited = vec![false; (size * size) as usize];
        let reach = half - 1;
        let mut i = 0;
        while i < list.len() {
            let (x, y) = list[i];
            if (cx - x).abs() < reach && (cy - y).abs() < reach {
                for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
                    let at = ((ox + nx) * size + ny + oy) as usize;
                    if !visited[at] && self.passable(nx, ny, mask)? {
                        visited[at] = true;
                        list.push((nx, ny));
                    }
                }
            }
            i += 1;
        }
        let mut best = f32::MAX;
        let mut spot = None;
        let mut empty = false;
        for &(x, y) in &list {
            if x0 <= x && x <= x1 && y0 <= y && y <= y1 {
                continue;
            }
            let was_empty = empty;
            let count = self.tiles.objects(x, y);
            if empty && count.is_some_and(|n| n != 0) {
                continue;
            }
            if count == Some(0) {
                empty = true;
            }
            let centre = [x as f32 + 0.5, from[2], y as f32 + 0.5];
            let d = [
                from[0] - centre[0],
                from[2] - centre[1],
                from[1] - centre[2],
            ];
            let distance = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            if (empty && !was_empty) || distance < best {
                spot = Some([centre[0], centre[2]]);
                best = distance;
            }
        }
        Ok(spot)
    }

    /// The objects whose centre is on `site`'s footprint, edges included, and that the game counts
    /// as occupying it (`FUN_14123bb20` for a site whose master obstructs): objects with all three
    /// radii, other than the site, unless the site may be built over them (`can_be_built_on` 1).
    pub fn footprint_occupants(
        &self,
        site_id: i32,
        site: &Placed,
    ) -> Result<Vec<i32>, Unsupported> {
        let [sx, sy, _] = site.position;
        let [rx, ry] = site.radius;
        self.check_known([sx - rx, sy - ry, sx + rx, sy + ry])?;
        Ok(self
            .objects
            .iter()
            .filter(|&(&id, o)| {
                id != site_id
                    && o.buildable_on != 1
                    && o.tall
                    && o.radius.iter().all(|&r| 0.0 < r)
                    && (o.position[0] - sx).abs() <= rx
                    && (o.position[1] - sy).abs() <= ry
            })
            .map(|(&id, _)| id)
            .collect())
    }

    /// The objects a foundation clears off its footprint when its first builder arrives
    /// (`FUN_1415c3620`): those whose footprint overlaps the site's, of another master, that may
    /// be built over (`can_be_built_on`) or are trees (class 15, which it clears when their hit
    /// points are at most 1).
    pub fn footprint_clearing(&self, site_id: i32, site: &Placed) -> Result<Vec<i32>, Unsupported> {
        let [sx, sy, _] = site.position;
        let [rx, ry] = site.radius;
        self.check_known([sx - rx, sy - ry, sx + rx, sy + ry])?;
        Ok(self
            .objects
            .iter()
            .filter(|&(&id, o)| {
                id != site_id
                    && o.master != site.master
                    && (o.buildable_on != 0 || o.class == 15)
                    && (o.position[0] - sx).abs() < o.radius[0] + rx
                    && (o.position[1] - sy).abs() < o.radius[1] + ry
            })
            .map(|(&id, _)| id)
            .collect())
    }

    /// Whether a static record of another player marks a tile of `site`'s footprint
    /// (`FUN_1411d4b80`).
    pub fn foreign_marks(&self, site: &Placed) -> bool {
        let [sx, sy, _] = site.position;
        let [rx, ry] = site.radius;
        let (w, h) = (self.tiles.width, self.tiles.height);
        let (x0, y0) = (((sx - rx) as i32).max(0), ((sy - ry) as i32).max(0));
        let x1 = (((sx + rx) - 0.001) as i32).min(w - 1);
        let y1 = (((sy + ry) - 0.001) as i32).min(h - 1);
        let own = 1u64 << site.player;
        (y0..=y1).any(|y| {
            (x0..=x1).any(|x| {
                let players = self.tiles.word(x, y).unwrap_or(0) >> 23;
                players != 0 && players != own
            })
        })
    }

    /// Slot 0x278 of unit `me` (`FUN_141158720`): whether it could stand at `point`. The point's
    /// tile is on the obstruction manager's map and its terrain class in the unit's `mask` (the
    /// tile's word, `DAT_143f0a088 + 0x98`; a static record's own bits do not count here), and no
    /// obstruction record but the unit's own holds the point within its box grown by `grow` (the
    /// unit's radius): `FUN_141051900` with flags 0x20, whose point test `FUN_141052160` takes
    /// records of every kind, edges included. A grown point beyond the map's edge meets the edge's
    /// records ([`Spatial::edge_records`]) and is never clear.
    pub fn point_clear(
        &self,
        me: i32,
        point: [f32; 2],
        grow: f32,
        mask: u8,
    ) -> Result<bool, Unsupported> {
        let [x, y] = point;
        let (tx, ty) = (x as i32, y as i32);
        let Some(word) = self.tiles.word(tx, ty) else {
            return Ok(false);
        };
        if word & u64::from(mask ^ 0xff) != 0 {
            return Ok(false);
        }
        let [w, h] = self.map;
        let rect = [x - grow, y - grow, grow + x, grow + y];
        if (rect[0] < 0.0 || rect[1] < 0.0 || (w as f32) < rect[2] || (h as f32) < rect[3])
            && !self.edge_records(rect).is_empty()
        {
            return Ok(false);
        }
        self.check_known_records(rect)?;
        let hit = self.objects.iter().any(|(&id, p)| {
            if id == me || p.obstruction.is_none() {
                return false;
            }
            let [x0, y0, x1, y1] = p.bounds();
            x <= x1 + grow && x0 - grow <= x && y <= grow + y1 && y0 - grow <= y
        });
        Ok(!hit)
    }

    /// The records a move query could report (`FUN_14114cc60` -> `FUN_141051900`): those of a
    /// tested kind whose box, grown by `grow`, meets `rect` (`x0, y0, x1, y1`, edges included),
    /// except the `ignore`d objects'. The game then tests each candidate against the move's
    /// segment and shrinks moving allies' boxes (master `+0x780`, at most 1); every hit is a
    /// candidate, so no candidate means no hit.
    pub fn obstruction_candidates(&self, rect: [f32; 4], grow: f32, ignore: &[i32]) -> Vec<i32> {
        self.objects
            .iter()
            .filter(|(id, p)| {
                p.obstruction
                    .is_some_and(|o| TESTED_KINDS.contains(&o.kind))
                    && !ignore.contains(id)
            })
            .filter(|(_, p)| {
                let [x0, y0, x1, y1] = p.bounds();
                let (x0, y0, x1, y1) = (x0 - grow, y0 - grow, grow + x1, grow + y1);
                rect[0] <= x1 && x0 <= rect[2] && rect[1] <= y1 && y0 <= rect[3]
            })
            .map(|(&id, _)| id)
            .collect()
    }
}

/// A record a pather query met (an entry of `FUN_141051900`'s list): an object's (its id) or a
/// temporary tile record's (`x << 12 | y | 0xf0000000`, `FUN_1410510c0`), with its box
/// (`+0x20`..`+0x2c`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// The record's id (`+0x18`).
    pub id: i32,
    /// Its box x0, y0, x1, y1.
    pub bounds: [f32; 4],
}

/// The id of tile (`x`, `y`)'s temporary record (`FUN_1410510c0`) and of the tile in a pather's
/// ignore list (`FUN_140f89e40`): `x << 12 | y` with the top four bits set.
#[must_use]
pub const fn tile_record_id(x: i32, y: i32) -> i32 {
    (x << 12) | y | (0xf000_0000_u32 as i32)
}

impl Spatial {
    /// The old short pather's line queries (flags 0x40: moving objects left out, every kind
    /// tested), after `FUN_141052b90` put a temporary record (kind 3, the tile's box) on each
    /// tile of the walk's box, grown by `buffer` and held on the map, whose terrain class the
    /// unit's `mask` lacks (`mask & 0x7f | 0x80`): the records whose box, grown by `buffer`, meets
    /// the walk `a` -> `b` (`FUN_1410512f0`, a circle's `FUN_141051630` alike) and whose id is not
    /// in `ignore` (the manager's list `+0x2c`, the pather's). An object that moves (its record's
    /// `+3` bit 0x10, set from slot 0x5c8 at the end of its update, [`Gait::moving`]) is left
    /// out, as is a gate the player may pass (slot 0x678, not modelled). `first` stops at the
    /// first hit (`FUN_141052350`); else at the 25th (`FUN_141051900`). The hits are in the
    /// tiles' row order, then by id; the game's are in its grid's order, which its callers do not
    /// read. A walk whose grown box leaves the map is blocked (`FUN_141052350`) or meets the
    /// map's edge records first ([`Spatial::edge_records`]).
    #[allow(clippy::too_many_arguments)]
    fn pather_query(
        &self,
        a: [f32; 2],
        b: [f32; 2],
        buffer: Option<f32>,
        mask: u8,
        ignore: &[i32],
        first: bool,
    ) -> Result<Vec<Hit>, Unsupported> {
        const MOST_HITS: usize = 25;
        let lo_x = if b[0] <= a[0] { b[0] } else { a[0] };
        let hi_x = if a[0] <= b[0] { b[0] } else { a[0] };
        let lo_y = if b[1] <= a[1] { b[1] } else { a[1] };
        let hi_y = if a[1] <= b[1] { b[1] } else { a[1] };
        let g = buffer.unwrap_or(0.0);
        let [w, h] = self.map;
        let grown = [lo_x - g, lo_y - g, g + hi_x, g + hi_y];
        let mut hits = Vec::new();
        if grown[0] < 0.0 || grown[1] < 0.0 || (w as f32) < grown[2] || (h as f32) < grown[3] {
            hits = self.edge_records(grown);
            if first {
                return Ok(hits);
            }
        }
        self.check_known_records(grown)?;
        let walk = [a[0], a[1], b[0], b[1]];
        let meets = |bx: [f32; 4]| {
            let r = [bx[0] - g, bx[1] - g, g + bx[2], g + bx[3]];
            lo_x <= r[2]
                && r[0] <= hi_x
                && lo_y <= r[3]
                && r[1] <= hi_y
                && box_meets_segment(bx, g, walk)
        };
        let impassable = u64::from(mask & 0x7f | 0x80);
        let tx1 = if (grown[2] as i32) < w {
            grown[2] as i32
        } else {
            w - 1
        };
        let ty1 = if (grown[3] as i32) < h {
            grown[3] as i32
        } else {
            h - 1
        };
        for y in (grown[1] as i32).max(0)..=ty1 {
            for x in (grown[0] as i32).max(0)..=tx1 {
                let Some(word) = self.tiles.word(x, y) else {
                    continue;
                };
                let id = tile_record_id(x, y);
                let bx = [x as f32, y as f32, x as f32 + 1.0, y as f32 + 1.0];
                if word & impassable == 0 && !ignore.contains(&id) && meets(bx) {
                    hits.push(Hit { id, bounds: bx });
                    if first || MOST_HITS <= hits.len() {
                        return Ok(hits);
                    }
                }
            }
        }
        for (&id, p) in &self.objects {
            let Some(o) = p.obstruction else {
                continue;
            };
            let kind = if o.kind == 10 { 2 } else { o.kind };
            if !TESTED_KINDS.contains(&kind)
                || self.gaits.get(&id).is_some_and(|g| g.moving)
                || ignore.contains(&id)
            {
                continue;
            }
            let bx = p.bounds();
            if !meets(bx) {
                continue;
            }
            if kind == 11 {
                return Err(Unsupported(
                    "a pather query meeting a record of kind 11 (its own radii)",
                ));
            }
            if p.class == GATE_CLASS {
                return Err(Unsupported(
                    "a pather query meeting a gate (its players' passage, slot 0x678)",
                ));
            }
            hits.push(Hit { id, bounds: bx });
            if first || MOST_HITS <= hits.len() {
                break;
            }
        }
        Ok(hits)
    }

    /// Whether the old short pather's walk `a` -> `b` meets a record ([`Spatial::pather_query`]:
    /// `FUN_141052b90`, then `FUN_141052350` with flags 0x40).
    pub fn pather_blocked(
        &self,
        a: [f32; 2],
        b: [f32; 2],
        buffer: Option<f32>,
        mask: u8,
        ignore: &[i32],
    ) -> Result<bool, Unsupported> {
        Ok(!self
            .pather_query(a, b, buffer, mask, ignore, true)?
            .is_empty())
    }

    /// The records the old short pather's walk `a` -> `b` meets, at most 25
    /// ([`Spatial::pather_query`]: `FUN_141052b90`, then `FUN_141051900` with flags 0x40).
    pub fn pather_hits(
        &self,
        a: [f32; 2],
        b: [f32; 2],
        buffer: Option<f32>,
        mask: u8,
        ignore: &[i32],
    ) -> Result<Vec<Hit>, Unsupported> {
        self.pather_query(a, b, buffer, mask, ignore, false)
    }
}

impl Spatial {
    /// The map edge's records a query rectangle `rect` (`x0, y0, x1, y1`) that leaves the map meets
    /// (`FUN_141057290`, which `FUN_141051900` lists first, before any test of kind, flags or
    /// ignore list): for each edge it crosses (left, top, right, bottom), the manager's record for
    /// that edge (`+0xb0`, `+0xb8`, `+0xc0`, `+0xc8`), set to the tile box where the rectangle's
    /// diagonal `(x0, y0)` -> `(x1, y1)` crosses the edge's line, id `x << 12 | y | 0xff000000`.
    #[must_use]
    pub fn edge_records(&self, rect: [f32; 4]) -> Vec<Hit> {
        let [x0, y0, x1, y1] = rect;
        // the manager's map size as floats (+0xa8, +0xac)
        let (w, h) = (self.map[0] as f32, self.map[1] as f32);
        let (dx, dy) = (x1 - x0, y1 - y0);
        let (dy0, dx0) = (y0 - 0.0, x0 - 0.0);
        // each edge line through the diagonal (the game's terms with a zero side, 0 × d and
        // (w − w) × d, drop out: they add or take a zero)
        let at = |t: f32| {
            let x = (dx * t + x0) as i32;
            // (uint): a truncation through 64 bits
            let y = (dy * t + y0) as i64 as u32;
            Hit {
                id: ((x << 12) as u32 | y | 0xff00_0000) as i32,
                bounds: [
                    x as f32,
                    y as i32 as f32,
                    x as f32 + 1.0,
                    y as i32 as f32 + 1.0,
                ],
            }
        };
        let mut hits = Vec::new();
        if x0 < 0.0 {
            let hh = h - 0.0;
            hits.push(at(-(hh * dx0) / (dx * hh)));
        }
        if y0 < 0.0 {
            let hh = h - 0.0;
            hits.push(at((hh * dy0) / -(hh * dy)));
        }
        if w < x1 {
            let hh = h - 0.0;
            hits.push(at(-((x0 - w) * hh) / (hh * dx)));
        }
        if h < y1 {
            let ww = w - 0.0;
            hits.push(at(((y0 - h) * ww) / -(ww * dy)));
        }
        hits
    }
}

/// The class of a gate, which a player's allies may pass (slot 0x678, `FUN_1411d4a30`).
const GATE_CLASS: i16 = 39;

/// `FUN_1410576a0`, Sedgewick's ccw: 1 when `p0`, `p1`, `p2` turn counter-clockwise (or lie in
/// a line with `p2` beyond `p1`), -1 clockwise (or `p0` between), 0 when `p2` lies on the segment.
fn ccw(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2]) -> i32 {
    let (dx1, dy1) = (p1[0] - p0[0], p1[1] - p0[1]);
    let (dx2, dy2) = (p2[0] - p0[0], p2[1] - p0[1]);
    // the game's `<=` is false for a NaN too
    if matches!(
        (dy2 * dx1).partial_cmp(&(dx2 * dy1)),
        Some(std::cmp::Ordering::Greater) | None
    ) {
        return 1;
    }
    if dy2 * dx1 < dx2 * dy1 || dx2 * dx1 < 0.0 || dy2 * dy1 < 0.0 {
        return -1;
    }
    if dy2 * dy2 + dx2 * dx2 <= dy1 * dy1 + dx1 * dx1 {
        return 0;
    }
    1
}

/// `FUN_141054ec0`: whether segments `p1`–`p2` and `p3`–`p4` meet.
fn segments_meet(p1: [f32; 2], p2: [f32; 2], p3: [f32; 2], p4: [f32; 2]) -> bool {
    (ccw(p1, p2, p4) * ccw(p1, p2, p3)) as f32 <= 0.0
        && (ccw(p3, p4, p2) * ccw(p3, p4, p1)) as f32 <= 0.0
}

/// `FUN_141052e10` with the box grown by `r`: the walk `[x0, y0, x1, y1]` crosses one of the grown
/// box's edges or starts or ends inside it.
fn box_meets_segment([bx0, by0, bx1, by1]: [f32; 4], r: f32, walk: [f32; 4]) -> bool {
    let lo_x = if bx1 < bx0 { bx1 } else { bx0 };
    let hi_x = if bx0 < bx1 { bx1 } else { bx0 };
    let lo_y = if by1 < by0 { by1 } else { by0 };
    let hi_y = if by0 < by1 { by1 } else { by0 };
    let (start, end) = ([walk[0], walk[1]], [walk[2], walk[3]]);
    let edges = [
        ([bx0 - r, by0 - r], [r + bx1, by0 - r]),
        ([r + bx1, by0 - r], [r + bx1, r + by1]),
        ([r + bx1, r + by1], [bx0 - r, r + by1]),
        ([bx0 - r, r + by1], [bx0 - r, by0 - r]),
    ];
    if edges
        .iter()
        .any(|&(e0, e1)| segments_meet(start, end, e0, e1))
    {
        return true;
    }
    let outside =
        |p: [f32; 2]| p[0] < lo_x - r || r + hi_x < p[0] || p[1] < lo_y - r || r + hi_y < p[1];
    !(outside(start) && outside(end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn villager(x: f32, y: f32) -> Placed {
        Placed {
            position: [x, y, 0.0],
            radius: [0.2, 0.2],
            class: 4,
            player: 2,
            master: 0,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: Some(Obstruction { kind: 5, class: 2 }),
        }
    }

    #[test]
    fn a_search_near_the_last_reads_its_answer_until_a_unit_ai_lets_it_go() {
        use crate::registry::Registry;
        use crate::unit_ai::{Pick, Search};
        let mut s = Spatial::new([120, 120]);
        // player 2 counts player 1 its enemy (3)
        s.relations = vec![vec![0, 3, 3], vec![0, 0, 3], vec![0, 3, 0]];
        let mut r = Registry::new(3, 65);
        // a villager of player 1 at (12, 10), which player 2 sees
        r.update_unit(1, 1, [12.0, 10.0], [0.2, 0.2], 4, 0, 1 << 2)
            .unwrap();
        let mut v = Visibility::new(120, 120, Vec::new(), Vec::new(), Vec::new(), 0, Vec::new());
        v.registry = Some(r);
        s.sight = Some(v);
        let look = |at| Search {
            at,
            radius: 3.0,
            classes: &[4],
            size: [0.2, 0.2],
            pick: Pick::Look,
        };
        let never = |_: &Placed, _: u8| Ok(false);
        let mut reach = |_: i32, _: f32| Ok(true);
        let still = |_: i32| false;
        let mut search = |s: &Spatial, at| {
            let f = s
                .search(9, 2, &look(at), &never, &mut reach, &still)
                .unwrap();
            (f.enemies, f.fresh)
        };
        assert_eq!(search(&s, [10.0, 10.0]), (true, true));
        // 4 tiles on (16 squared, within 25) the query is skipped: the enemy stays found
        assert_eq!(search(&s, [10.0, 14.0]), (true, false));
        // a unit AI's full update lets the cache go: the query runs, and finds nothing there
        s.forget_search();
        assert_eq!(search(&s, [10.0, 14.0]), (false, true));
        // 6 tiles from the last query (36 squared) it runs again
        assert_eq!(search(&s, [10.0, 8.0]), (true, true));
    }

    #[test]
    fn a_rectangle_leaving_the_map_meets_the_edge_records_where_its_diagonal_crosses() {
        let s = Spatial::new([120, 120]);
        assert_eq!(s.edge_records([0.0, 0.0, 120.0, 120.0]), Vec::<Hit>::new());
        // the diagonal (-0.3, 50) -> (0.3, 50.6) crosses x 0 at y 50.3: the left edge's tile (0, 50)
        let left = s.edge_records([-0.3, 50.0, 0.3, 50.6]);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id as u32, 0xff00_0000 | 0x32);
        assert_eq!(left[0].bounds, [0.0, 50.0, 1.0, 51.0]);
        // past the bottom right corner: (119.5, 119.7) -> (120.5, 120.7) crosses x 120 at y 120.2
        // and y 120 at x 119.8
        let corner = s.edge_records([119.5, 119.7, 120.5, 120.7]);
        let ids: Vec<u32> = corner.iter().map(|h| h.id as u32).collect();
        assert_eq!(ids, [0xff07_8078, 0xff07_7078]);
        // a unit of radius 0.3 cannot stand 0.2 from the edge
        assert!(!s.point_clear(1, [0.2, 50.0], 0.3, 0xff).unwrap());
        assert!(s.point_clear(1, [0.4, 50.0], 0.3, 0xff).unwrap());
    }

    #[test]
    fn a_units_box_is_its_position_plus_and_minus_its_radii() {
        // MarvinBlue/426048731 tick 680: 9177's record box in the game's obstruction manager
        assert_eq!(villager(28.0, 89.0).bounds(), [27.8, 88.8, 28.2, 89.2]);
    }

    #[test]
    fn a_query_finds_the_boxes_its_rectangle_meets_but_not_the_ignored_or_unobstructing() {
        let mut s = Spatial::new([120, 120]);
        s.insert(9177, villager(28.0, 89.0));
        s.insert(9175, villager(30.0, 89.0));
        let mut foundation = villager(30.0, 88.0);
        foundation.radius = [1.0, 1.0];
        foundation.obstruction = None;
        foundation.class = 3;
        s.insert(10028, foundation);
        // 9177's first step: from (28, 89) toward (28.0143, 88.9928), grown by its radius
        let step = [28.0, 88.99284, 28.01431, 89.0];
        assert_eq!(
            s.obstruction_candidates(step, 0.2, &[9177]),
            Vec::<i32>::new()
        );
        assert_eq!(s.obstruction_candidates(step, 0.2, &[]), vec![9177]);
        // 9175's box grown by 0.2 starts at x 29.6
        assert_eq!(
            s.obstruction_candidates([29.0, 89.0, 29.59, 89.0], 0.2, &[9177]),
            Vec::<i32>::new()
        );
        assert_eq!(
            s.obstruction_candidates([29.0, 89.0, 29.61, 89.0], 0.2, &[9177]),
            vec![9175]
        );
    }

    #[test]
    fn a_walk_hits_the_boxes_its_step_crosses_and_a_moving_ally_s_box_shrinks() {
        let mut s = Spatial::new([120, 120]);
        s.relations = vec![vec![1, 4, 4], vec![1, 0, 4], vec![1, 4, 0]];
        s.insert(9175, villager(30.0, 89.0));
        let (from, to) = ([29.5, 88.0], [29.5, 88.3]);
        let rect = [29.5, 88.0, 29.5, 88.3];
        // the box (29.8..30.2, 88.8..89.2) grown by 0.25 reaches 88.55, past the step's end
        assert_eq!(
            s.walk_hits(from, to, rect, 0.25, 0.5, Some(2), &[]),
            Ok(vec![])
        );
        // grown by 0.4 it reaches x 29.4: the step crosses it once it gets to y 88.4
        let longer = ([29.5, 88.0], [29.5, 88.5]);
        let rect = [29.5, 88.0, 29.5, 88.5];
        assert_eq!(
            s.walk_hits(longer.0, longer.1, rect, 0.4, 1.0, Some(2), &[]),
            Ok(vec![9175])
        );
        assert_eq!(
            s.walk_hits(longer.0, longer.1, rect, 0.4, 1.0, Some(2), &[9175]),
            Ok(vec![])
        );
        // moving, of the walker's player, it grows by 0.4 × 0.5 only
        s.set_gait(
            9175,
            Some(Gait {
                moving: true,
                ..Gait::default()
            }),
        );
        assert_eq!(
            s.walk_hits(longer.0, longer.1, rect, 0.4, 0.5, Some(2), &[]),
            Ok(vec![])
        );
        // an enemy's moving box grows in full
        s.relations[2][1] = 3;
        assert_eq!(
            s.walk_hits(longer.0, longer.1, rect, 0.4, 0.5, Some(1), &[]),
            Ok(vec![9175])
        );
    }

    /// MarvinBlue/426048731's Town Centre 9157 of player 2 (class 3) and a tree of Gaia (class 1),
    /// with the words the game's obstruction manager holds for their tiles.
    #[test]
    fn a_static_record_marks_its_tiles_with_its_class_and_player() {
        let mut s = Spatial::with_tiles([4, 4], Tiles::new(4, 4, vec![4; 16]));
        let centre = Placed {
            position: [2.0, 2.0, 0.0],
            radius: [1.0, 1.0],
            class: 3,
            player: 2,
            master: 0,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: Some(Obstruction { kind: 2, class: 3 }),
        };
        s.insert(9157, centre);
        let tree = Placed {
            position: [0.5, 0.5, 0.0],
            radius: [0.5, 0.5],
            class: 15,
            player: 0,
            master: 0,
            obstruction_type: 0,
            buildable_on: 0,
            tall: true,
            speed: 0.0,
            unknown: None,
            obstruction: Some(Obstruction { kind: 2, class: 1 }),
        };
        s.insert(48, tree);
        let t = s.tiles();
        assert_eq!(t.word(1, 1), Some(0x202_4304));
        assert_eq!(t.word(2, 2), Some(0x202_4304));
        assert_eq!(t.word(3, 3), Some(4));
        assert_eq!(t.word(0, 0), Some(0x80_4104));
        assert_eq!((t.objects(2, 2), t.tall(2, 2)), (Some(1), Some(1)));
        s.remove(9157);
        assert_eq!(s.tiles().word(1, 1), Some(4));
        assert_eq!(s.tiles().objects(2, 2), Some(0));
    }

    #[test]
    fn a_unit_counts_on_the_tile_its_centre_is_on_and_marks_nothing() {
        let mut s = Spatial::with_tiles([4, 4], Tiles::new(4, 4, vec![4; 16]));
        s.insert(9177, villager(1.9, 1.0));
        assert_eq!(s.tiles().objects(1, 1), Some(1));
        assert_eq!(s.tiles().word(1, 1), Some(4));
        s.set_position(9177, [2.05, 1.0, 0.0]);
        assert_eq!(
            (s.tiles().objects(1, 1), s.tiles().objects(2, 1)),
            (Some(0), Some(1))
        );
        assert_eq!(s.tiles().tall(2, 1), Some(1));
    }

    #[test]
    fn a_tile_s_list_keeps_the_game_s_order() {
        let mut s = Spatial::with_tiles([4, 4], Tiles::new(4, 4, vec![4; 16]));
        for (id, x) in [(9171, 1.1), (10029, 1.5), (9173, 1.9)] {
            s.insert(id, villager(x, 1.0));
        }
        assert_eq!(s.tiles().list(1, 1), [9171, 10029, 9173]);
        // the leaving object's slot goes to the last
        s.set_position(9171, [2.1, 1.0, 0.0]);
        assert_eq!(s.tiles().list(1, 1), [9173, 10029]);
        // a move within the tile and a master change keep the slot
        s.set_position(9173, [1.2, 1.0, 0.0]);
        s.change_master(9173, villager(1.2, 1.0));
        assert_eq!(s.tiles().list(1, 1), [9173, 10029]);
        s.set_position(9171, [1.0, 1.0, 0.0]);
        assert_eq!(s.tiles().list(1, 1), [9173, 10029, 9171]);
        assert_eq!(
            (s.tiles().objects(1, 1), s.tiles().tall(1, 1)),
            (Some(3), Some(3))
        );
        assert_eq!(
            s.force_tile_lists(&[(1, 1, vec![9171, 9173, 10029])]),
            Vec::new()
        );
        assert_eq!(s.tiles().list(1, 1), [9171, 9173, 10029]);
    }

    /// The house foundation 10028 at (30, 88) of MarvinBlue/426048731 with its builder 9175 on its
    /// footprint at (30, 89), on open land (class 4) and a villager's mask (restriction 7: 6).
    fn site() -> (Spatial, Placed) {
        let mut s = Spatial::with_tiles([40, 120], Tiles::new(40, 120, vec![4; 40 * 120]));
        s.insert(9175, villager(30.0, 89.0));
        let mut house = villager(30.0, 88.0);
        house.radius = [1.0, 1.0];
        house.class = 3;
        house.master = 70;
        house.obstruction_type = 2;
        house.obstruction = None;
        s.insert(10028, house);
        (s, house)
    }

    #[test]
    fn a_tile_is_passable_when_its_class_is_in_the_mask_and_nothing_blocks_it() {
        let (mut s, _) = site();
        assert_eq!(s.passable(29, 89, 6), Ok(true));
        assert_eq!(s.passable(29, 89, 1), Ok(false));
        assert_eq!(s.passable(-1, 89, 6), Ok(false));
        let mut tree = villager(29.5, 90.5);
        tree.radius = [0.5, 0.5];
        tree.obstruction = Some(Obstruction { kind: 2, class: 1 });
        s.insert(48, tree);
        assert_eq!(s.passable(29, 90, 6), Ok(false));
    }

    #[test]
    fn the_free_spot_beside_a_footprint_is_the_closest_empty_tile_around_it() {
        // the game sends 9175 to (29.5, 89.5): (30, 89) is as close but holds 9175 itself
        let (s, house) = site();
        assert_eq!(
            s.spot_beside([30.0, 89.0, 0.0], 6, &house),
            Ok(Some([29.5, 89.5]))
        );
    }

    #[test]
    fn the_footprint_s_occupants_are_the_objects_whose_centre_is_on_it() {
        let (mut s, house) = site();
        assert_eq!(s.footprint_occupants(10028, &house), Ok(vec![9175]));
        s.insert(9177, villager(28.0, 89.0));
        assert_eq!(s.footprint_occupants(10028, &house), Ok(vec![9175]));
        assert_eq!(s.footprint_clearing(10028, &house), Ok(vec![]));
        let mut flower = villager(30.5, 87.5);
        flower.buildable_on = 1;
        s.insert(77, flower);
        assert_eq!(s.footprint_clearing(10028, &house), Ok(vec![77]));
        assert!(!s.foreign_marks(&house));
    }

    #[test]
    fn a_walk_meets_a_static_box_in_its_way_but_not_one_beside_it() {
        // tiles of class 1 everywhere: a unit with mask 1 crosses them all
        let mut s = Spatial::with_tiles([16, 16], Tiles::new(16, 16, vec![1; 256]));
        let mut house = villager(8.0, 8.0);
        house.radius = [1.0, 1.0];
        house.obstruction = Some(Obstruction { kind: 2, class: 3 });
        s.insert(7, house);
        assert!(s.segment_blocked([4.0, 8.0], [12.0, 8.0], 0.2, 1, &[]));
        assert!(!s.segment_blocked([4.0, 11.0], [12.0, 11.0], 0.2, 1, &[]));
        // within its radius of the box, and the box ignored
        assert!(s.segment_blocked([4.0, 9.1], [12.0, 9.1], 0.2, 1, &[]));
        assert!(!s.segment_blocked([4.0, 8.0], [12.0, 8.0], 0.2, 1, &[7]));
        // a moving unit's record never blocks the walk
        s.insert(8, villager(6.0, 3.0));
        assert!(!s.segment_blocked([4.0, 3.0], [12.0, 3.0], 0.2, 1, &[]));
    }

    #[test]
    fn a_walk_meets_a_tile_whose_terrain_the_unit_cannot_cross() {
        let mut classes = vec![1; 256];
        classes[5 * 16 + 9] = 2;
        let s = Spatial::with_tiles([16, 16], Tiles::new(16, 16, classes));
        assert!(s.segment_blocked([2.0, 5.5], [14.0, 5.5], 0.2, 1, &[]));
        assert!(!s.segment_blocked([2.0, 5.5], [14.0, 5.5], 0.2, 3, &[]));
        // a walk whose grown box leaves the map
        assert!(s.segment_blocked([0.1, 5.5], [14.0, 5.5], 0.2, 3, &[]));
    }

    #[test]
    fn a_frozen_object_may_be_anywhere_its_speed_could_take_it() {
        let (mut s, house) = site();
        let mut deer = villager(34.0, 88.0);
        deer.speed = 1.0;
        s.insert(9656, deer);
        s.freeze(9656);
        // frozen at once, the deer is where the save has it
        assert!(s.footprint_occupants(10028, &house).is_ok());
        assert!(s.get_known(9656).is_ok());
        // its box reaches x 33.8; 3 s later it may stand on the footprint, which ends at 31
        s.grow_unknown(2.5);
        assert!(s.footprint_occupants(10028, &house).is_ok());
        s.grow_unknown(0.5);
        assert!(s.footprint_occupants(10028, &house).is_err());
        assert!(s.get_known(9656).is_err());
    }

    #[test]
    fn a_frozen_object_that_does_not_move_is_known_unless_a_stopped_unit_may_reach_it() {
        let mut s = Spatial::new([64, 64]);
        let tree = Placed {
            player: 0,
            speed: 0.0,
            ..villager(24.5, 97.5 - 60.0)
        };
        s.insert(9365, tree);
        s.freeze(9365);
        s.reach = 1.0;
        let near = [24.0, 37.0, 24.6, 38.0];
        assert!(s.check_known(near).is_ok());
        // a stopped villager 10 tiles off, 0.8 tiles a second
        let mut walker = villager(34.5, 37.5);
        walker.player = 1;
        walker.speed = 0.8;
        s.insert(9200, walker);
        s.freeze(9200);
        s.grow_unknown(10.0);
        assert!(s.check_known(near).is_ok());
        s.grow_unknown(2.0);
        assert!(s.check_known(near).is_err());
    }
}
