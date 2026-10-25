//! Where the world's objects are, as movement reads them: each live object's position, radii and
//! class, and the obstruction record the game keeps for it in its obstruction manager
//! (`DAT_143f0a088`, queried by `FUN_141051900`).
//!
//! The game files its records in a grid of levels ([`Grid`]); the slice keeps them by object id
//! and asks the grid only which records a query visits: a record filed in a cell outside the
//! query's tiles is not found, even when its grown box meets the query. The grid also orders a
//! query's hits, but the slice stops a walker at its first hit ([`Spatial::walk_hits`]), so it
//! needs no order yet.
//!
//! [`Tiles`] holds what the game keeps per tile: the obstruction manager's word (terrain class,
//! static records' marks) and the counts of records and objects whose centre is on the tile.

use crate::registry::Record;
use crate::unit_ai::{Found, GenieError, Pick, Search};
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
pub type Reach<'a> = dyn FnMut(i32, f32) -> Result<bool, GenieError> + 'a;

/// `bestUnitToAttack`'s world ([`crate::best_target::Battlefield`]): the unit AI's picker for its
/// reads and tests, the search's reachability for its paths (slot 0x3d0).
struct AttackField<'p, 'r> {
    picker: &'p mut dyn crate::unit_ai::AttackPicker,
    reach: &'p mut Reach<'r>,
}

impl crate::best_target::Battlefield for AttackField<'_, '_> {
    fn thing(&mut self, id: i32) -> Result<Option<crate::best_target::Thing>, GenieError> {
        self.picker.thing(id)
    }

    fn can_attack(&mut self, id: i32) -> Result<bool, GenieError> {
        self.picker.can_attack(id)
    }

    fn attacks_neutral(&mut self, class: i16) -> Result<bool, GenieError> {
        self.picker.attacks_neutral(class)
    }

    fn spares_friends(&mut self, id: i32) -> Result<bool, GenieError> {
        self.picker.spares_friends(id)
    }

    fn shuns(&mut self, id: i32) -> Result<bool, GenieError> {
        self.picker.shuns(id)
    }

    fn candidate(
        &mut self,
        id: i32,
        distance: f32,
        category: i32,
    ) -> Result<crate::best_target::Candidate, GenieError> {
        self.picker.candidate(id, distance, category)
    }

    fn armored(&mut self, id: i32) -> Result<bool, GenieError> {
        self.picker.armored(id)
    }

    fn weapon_range(&mut self, id: i32) -> Result<f32, GenieError> {
        self.picker.weapon_range(id)
    }

    fn engages(&mut self, id: i32) -> Result<bool, GenieError> {
        self.picker.engages(id)
    }

    fn quick_path(&mut self, id: i32, range: f32) -> Result<bool, GenieError> {
        self.picker.quick_path(id, range)
    }

    fn path(&mut self, id: i32, range: f32) -> Result<bool, GenieError> {
        (self.reach)(id, range)
    }
}

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
    /// The relation passed over (argument 9, `FUN_140c32b50`: 0 friends, 1 neutrals, 2
    /// enemies, 3 itself; −1 none).
    spared: i32,
    /// Argument 8: only objects the unit reaches count (slot 0x3d0).
    reach: bool,
}

/// `ClosestObject`'s count (`FUN_140c488b0`, `DAT_143e6f92c`): the owners from 1 to 8 of the
/// records the lists read, every record marking its owner (`record+8`) before any test; Gaia
/// is not counted.
fn players_seen(found: &[Vec<Record>; 5]) -> u32 {
    let mut seen = [false; 9];
    for record in found.iter().flatten() {
        if let Some(s) = seen.get_mut(usize::from(record.owner)) {
            *s = true;
        }
    }
    seen[1..].iter().map(|&s| u32::from(s)).sum()
}

/// The pastures, whose workers and livestock `ClosestObject` counts (`FUN_140c488b0`, raw
/// `0x6801`, `0x6805`).
const PASTURES: [i16; 2] = [1893, 1897];
/// The fish trap (raw `0x6163`), which only the fishing ships gather from (`FUN_1411945a0`).
const FISH_TRAP: i16 = 199;
/// The fishing ships' masters (`FUN_1411945a0`: 13, 2356, 2171).
const TRAPPERS: [i16; 3] = [13, 2356, 2171];
/// The villagers' class, which gathers from farms (`FUN_141195750`).
const FARMER_CLASS: i16 = 4;

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
    /// Slot 0x2a8 (`FUN_141163100`): it waits to move (`+0x7f9`) and its top action is in a state
    /// other than 6 and 7.
    pub waits: bool,
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
    /// Its unit group (`mGroupID`, `+0x6d0`), none for −1: a walker of the same group ranks it by
    /// the group's member order.
    pub group: Option<i32>,
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
    /// `FUN_141056460`'s first test: a master of obstruction kind 0 or 12, or of class 0, gets
    /// neither a record nor a mesh shape.
    #[must_use]
    pub const fn recorded(self) -> bool {
        self.kind != 0 && self.kind != 12 && self.class != 0
    }

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

/// The room of the hit list the obstruction queries' callers give (`local_1758`, `local_148`:
/// `0x19`), which the edge records (`FUN_141057290`) also respect.
const QUERY_HITS: usize = 0x19;

/// The most objects a tile's list holds (`FUN_140f5f0a0`'s `count < 0xfa`).
const MOST_ON_TILE: usize = 0xfa;

/// The obstruction manager's grid (`FUN_141054660`): its top cell is the largest power of two up
/// to twice the map's longer side (at most 4096 tiles), each level halves the cells' side
/// (level `k`'s cells are `top >> k` tiles wide, its cell shift `levels - k + 2`), down to cells
/// of 8 tiles. Records are filed in one cell each ([`Spatial::filed`]); a query visits on every
/// level the cells its tile range covers (`FUN_141051900`, `FUN_141052350`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Grid {
    /// How many levels (`+0x1ac`).
    levels: i32,
}

impl Grid {
    fn new([w, h]: [i32; 2]) -> Self {
        let longer = w.max(h);
        let mut top = 0x1000;
        while longer * 2 < top {
            top >>= 1;
        }
        let mut levels = 0;
        while 7 < top {
            levels += 1;
            top >>= 1;
        }
        Self { levels }
    }

    /// The level table (`+0x1bc`): the finest level whose cells hold two tiles whose coordinates
    /// XOR, shifted right by 3, to `v` (the levels less its bit length, at least −1).
    fn level(self, v: i32) -> i32 {
        let bits = 32 - v.cast_unsigned().leading_zeros() as i32;
        (self.levels - 1 - bits).max(-1)
    }

    /// Level `level`'s cell shift (`+0x2c4`).
    const fn shift(self, level: i32) -> i32 {
        self.levels - level + 2
    }
}

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

    /// The object goes last on tile `i` (`FUN_140f5f0a0`). An empty tile's list takes it and
    /// the tall count becomes 1 for a `tall` object (else it stays, 0 on an empty tile); a tile
    /// of [`MOST_ON_TILE`] objects or more takes none and keeps its count; else the object goes
    /// last and the tall count is redone over the list, each object by `tall` (its master's
    /// height radius above 0 now).
    fn append(&mut self, i: usize, id: i32, tall: impl Fn(i32) -> bool) {
        let n = self.lists[i].len();
        if n == 0 {
            self.lists[i].push(id);
            if tall(id) {
                self.tall[i] = 1;
            }
            return;
        }
        if MOST_ON_TILE <= n {
            return;
        }
        self.lists[i].push(id);
        self.recount(i, tall);
    }

    /// The object leaves tile `i` (`FUN_140f5f560`): one not on the list changes nothing; else
    /// the last object takes its slot and the tall count is redone over the list.
    fn take_off(&mut self, i: usize, id: i32, tall: impl Fn(i32) -> bool) {
        let Some(k) = self.lists[i].iter().position(|&o| o == id) else {
            return;
        };
        let _ = self.lists[i].swap_remove(k);
        self.recount(i, tall);
    }

    /// Tile `i`'s tall count over its list, each object by `tall` (a byte, as the game counts).
    fn recount(&mut self, i: usize, tall: impl Fn(i32) -> bool) {
        let count = self.lists[i].iter().filter(|&&o| tall(o)).count();
        self.tall[i] = count as u8;
    }

    /// Marks (`bits`) on tile (`x`, `y`)'s word, for a test's map.
    #[cfg(test)]
    pub(crate) fn mark_for_test(&mut self, x: i32, y: i32, bits: u64) {
        let i = self.index(x, y).expect("a tile on the map");
        self.words[i] |= bits;
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

    /// A static record of object `me` leaves (`FUN_141055980`): tile by tile, row by row, over
    /// the tiles it covers, a tile its player marked keeps only its terrain class; but at a tile
    /// where another object (its centre on the tile) holds a static record (`static_record`:
    /// kind 0, 1, 2, 11 or 13), the rest stays as it is and the planners hear nothing (false).
    fn unmark(&mut self, p: &Placed, me: i32, static_record: impl Fn(i32) -> bool) -> bool {
        let (xs, ys) = self.covered(p);
        let own = 1u64 << p.player;
        for y in ys {
            for x in xs.clone() {
                let i = self.index(x, y).expect("covered tiles are on the map");
                if self.lists[i].iter().any(|&o| o != me && static_record(o)) {
                    return false;
                }
                if self.words[i] >> 23 == own {
                    self.words[i] &= 0xff;
                }
            }
        }
        true
    }
}

/// The world's objects by id, the map's size in tiles and what the game keeps per tile.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spatial {
    /// The map's size in tiles.
    pub map: [i32; 2],
    /// The obstruction manager's ignore list (`+0x2c` set, `+0x30` count, `+0x38` ids) as the
    /// last `FUN_1410562e0` left it: the queries that set it before they ask
    /// ([`Spatial::set_manager_ignore`]) leave it to the ones that do not (the motion step's
    /// query of a level's waypoint). The game keeps a pointer to the caller's list; the slice
    /// keeps the ids it held then.
    manager_ignore: std::cell::RefCell<Vec<i32>>,
    /// The obstruction manager's temporary tile records (`FUN_1410510c0`: kind 3, the tile's box,
    /// id [`tile_record_id`], on the list `+0x70` with the count `+0x78`, bit 0x80 in the tile's
    /// word), as (y, x): [`Spatial::put_tile_records`] puts them, and they stay for every query
    /// until [`Spatial::clear_tile_records`] removes them all.
    tile_records: std::cell::RefCell<std::collections::BTreeSet<(i32, i32)>>,
    /// The obstruction manager's buffer (`+0x57` set, `+0x58` the radius its queries grow the
    /// records by) as the last `FUN_1410562d0` left it, None when not set: the routines that
    /// set it reset it at their end but `FindShortPath`'s quick-only exit, and the queries that
    /// set none read what is left ([`Spatial::manager_grow`]).
    manager_buffer: std::cell::Cell<Option<f32>>,
    objects: BTreeMap<i32, Placed>,
    /// The objects inside others (`+0x2f0`): off the tiles' lists and the obstruction manager,
    /// without a record, at their container's x and y; a lookup by id still finds them.
    inside: BTreeMap<i32, Placed>,
    tiles: Tiles,
    /// The tile rectangles (x0, y0, x1, y1) whose static marks changed, for the planners
    /// ([`Spatial::take_changes`]).
    changed: Vec<[i16; 4]>,
    /// The foundations whose construction started this update, whose clearing of the desolid
    /// foundations they overlap (`FUN_1411cd0d0`) the update runs after the builder's
    /// ([`Spatial::take_started`]).
    started: Vec<i32>,
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
    /// Each player's scout map (`player+0x1370`, `mIsScoutMapEnabled` `+0x1368`), once made.
    scout_maps: BTreeMap<usize, crate::explore::InfluenceMap>,
    /// Each annex's main building (`+0xbb8`), which the walk's avoidance takes for the annex.
    annexes: BTreeMap<i32, i32>,
    /// `relation[i]` of each player (`player+0x260`), for the walk's query's allies.
    pub relations: Vec<Vec<u8>>,
    /// What each player's queries make of the other players' objects (`player+0x278`, the
    /// save's `unit_diplomacy`): 1 its own, 0 Gaia's, 2 an ally's, 3 a neutral's, 4 an enemy's.
    /// Loaded, not derived: the slice runs no diplomacy change (command 0x76), whose handler
    /// would write it with `relation`.
    pub unit_diplomacy: Vec<Vec<i32>>,
    /// Each player's masters with a task of action type 0x99, which a building's slot 0x4f0
    /// reads (`FUN_1411d4980` -> `FUN_1415c60b0`): a drop-off walker counts such a building
    /// among the workers near it.
    pub task_0x99: Vec<Vec<i16>>,
    /// What the players see, when the slice holds it: the objects' sight follows their tiles
    /// ([`Spatial::set_position`]).
    pub sight: Option<Visibility>,
    /// Each player's vision masks, visible (`+0x2a8`) and explored (`+0x2a0`), as the world's
    /// rebuild sets them ([`crate::vision`]); a player without them sees with its own bits.
    pub vision: Vec<[u64; 2]>,
    /// The map's terrain, when the slice holds it.
    pub terrain: Option<crate::terrain::Terrain>,
    /// The unit AIs' search cache ([`SearchCache`]).
    search_cache: std::cell::RefCell<SearchCache>,
    /// The obstruction manager's grid, from the map's size.
    grid: Grid,
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

/// [`Spatial::closest_object`]'s per-object checks: livestock that is moving, and the objects a
/// class-10 searcher passes.
type ClosestChecks<'a> = (
    &'a dyn Fn(i32) -> bool,
    &'a dyn Fn(&Placed) -> Result<bool, GenieError>,
    &'a FarmLook<'a>,
);

/// What `ClosestObject` reads of a farm it found: its workers (`+0x644`) and the pasture it is in
/// (`+0x790`, `parent_pasture_id`, −1 for none); None for an object the caller does not hold.
pub type FarmLook<'a> = dyn Fn(i32) -> Option<(i32, i32)> + 'a;

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
            inside: BTreeMap::new(),
            tiles,
            changed: Vec::new(),
            started: Vec::new(),
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
    fn unmark(&mut self, p: &Placed, id: i32) {
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

    /// A foundation's construction started ([`crate::build::Construction`]'s start).
    pub fn started(&mut self, id: i32) {
        self.started.push(id);
    }

    /// The foundations whose construction started since the last call.
    pub fn take_started(&mut self) -> Vec<i32> {
        std::mem::take(&mut self.started)
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
        let word = v.unified[(y * v.width + x) as usize] & crate::registry::MASK;
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

    fn check_known_of(
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
    fn touchable(&self, id: i32, rect: [f32; 4]) -> bool {
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
    fn frozen(&self) -> impl Iterator<Item = (i32, &Placed)> + '_ {
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
        if let Some(i) = self.tiles.tile_of(placed.position) {
            let objects = &self.objects;
            self.tiles.append(i, id, |o| Self::is_tall(objects, o));
        }
    }

    /// Whether listed object `id` counts as tall (its master's height radius above 0, which
    /// `FUN_140f5f0a0` and `FUN_140f5f560` read): a ghost, whose master the slice does not hold,
    /// counts as not tall.
    fn is_tall(objects: &BTreeMap<i32, Placed>, id: i32) -> bool {
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

    /// The object `id` comes out of the object it was inside ([`Spatial::go_inside`]): its record,
    /// which the caller puts on the map again.
    pub fn come_out(&mut self, id: i32) -> Option<Placed> {
        self.inside.remove(&id)
    }

    /// The object `id` leaves the world.
    pub fn remove(&mut self, id: i32) {
        let _ = self.inside.remove(&id);
        if let Some(gone) = self.objects.remove(&id) {
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
            || self.objects.iter().any(|(id, p)| {
                // kind 10 is filed as 2
                let kind = p.obstruction.map(|o| if o.kind == 10 { 2 } else { o.kind });
                matches!(kind, Some(2 | 11 | 13))
                    && !ignore.contains(id)
                    && test.meets(p.bounds())
                    && self.visits(p, kind.unwrap_or(0), test.query)
            })
    }

    /// Whether a temporary tile record ([`Spatial::put_tile_records`]) of a cell the walk's
    /// query visits, not in `ignore`, meets the walk ([`WalkTest::meets`]). The records are filed
    /// on the level of the table's first entry (`+0x1bc`), in the cell of their tile.
    fn tile_records_meet(&self, test: &WalkTest, ignore: &[i32]) -> bool {
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
        use crate::checkpoint::cvttss2si;
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

    /// Each obstruction record: the object's id, the record's kind and box.
    pub fn records(&self) -> impl Iterator<Item = (i32, u8, [f32; 4])> + '_ {
        self.objects
            .iter()
            .filter_map(|(&id, p)| Some((id, p.obstruction?.kind, p.bounds())))
    }

    /// Each obstruction record's grid cell ([`Spatial::filed`]): the object's id, the level and
    /// the cell's x and y.
    pub fn record_cells(&self) -> impl Iterator<Item = (i32, (i32, i32, i32))> + '_ {
        self.objects
            .iter()
            .filter_map(|(&id, p)| Some((id, self.filed(p, p.obstruction?.kind))))
    }

    /// The grid cell an object's record of kind `kind` is filed in (`FUN_141056460`): level and
    /// cell x, y. Its box's tiles, held on the map; a box of kind 2 (10), 11 or 13 whose centre
    /// sits on a half tile ends 0.001 short on both axes. The level is the finest whose cell holds
    /// both corners' tiles, the cell the one of the low corner's tile.
    fn filed(&self, p: &Placed, kind: u8) -> (i32, i32, i32) {
        let [x0, y0, mut x1, mut y1] = p.bounds();
        let [x, y, _] = p.position;
        if matches!(kind, 2 | 10 | 11 | 13) && x % 0.5 < 0.001 && y % 0.5 < 0.001 {
            x1 -= 0.001;
            y1 -= 0.001;
        }
        let [w, h] = self.map;
        let tile = |v: f32, n: i32| (v as i32).clamp(0, n - 1);
        let (tx0, ty0, tx1, ty1) = (tile(x0, w), tile(y0, h), tile(x1, w), tile(y1, h));
        let level = self
            .grid
            .level((tx1 ^ tx0) >> 3)
            .min(self.grid.level((ty1 ^ ty0) >> 3));
        let shift = self.grid.shift(level);
        (level, tx0 >> shift, ty0 >> shift)
    }

    /// Whether a query whose rectangle, grown as the query grows it, is `rect` (`x0, y0, x1, y1`)
    /// visits the cell an object's record of kind `kind` is filed in ([`Spatial::filed`]): the
    /// query's tiles run from the low corner's (at least 0) to the high corner's (at most the
    /// map's size), and on each level it visits the cells they cover.
    fn visits(&self, p: &Placed, kind: u8, rect: [f32; 4]) -> bool {
        let (level, cx, cy) = self.filed(p, kind);
        self.visits_cell(level, cx, cy, rect)
    }

    /// Whether a query whose grown rectangle is `rect` visits cell (`cx`, `cy`) of `level`
    /// ([`Spatial::visits`]).
    fn visits_cell(&self, level: i32, cx: i32, cy: i32, rect: [f32; 4]) -> bool {
        let shift = self.grid.shift(level);
        let [w, h] = self.map;
        let (qx0, qy0) = ((rect[0] as i32).max(0), (rect[1] as i32).max(0));
        let (qx1, qy1) = ((rect[2] as i32).min(w), (rect[3] as i32).min(h));
        qx0 >> shift <= cx && cx <= qx1 >> shift && qy0 >> shift <= cy && cy <= qy1 >> shift
    }

    /// The object moved: its record moves with it, and it changes tiles when its centre crosses
    /// a tile's edge (`RGE_Static_Object::new_position`, `0x1411a58b0`).
    /// Player `player`'s scout map, once made (`player+0x1370`).
    #[must_use]
    pub fn scout_map(&self, player: usize) -> Option<&crate::explore::InfluenceMap> {
        self.scout_maps.get(&player)
    }

    /// Player `player`'s scout map becomes `map` (the old one goes).
    pub fn set_scout_map(&mut self, player: usize, map: crate::explore::InfluenceMap) {
        let _ = self.scout_maps.insert(player, map);
    }

    /// Tile (`x`, `y`) of player `player`'s scout map, if it has one, takes 1 (`FUN_140f4d5e0`).
    pub fn scout(&mut self, player: usize, x: i32, y: i32) {
        if let Some(m) = self.scout_maps.get_mut(&player) {
            m.set(x, y, 1);
        }
    }

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

    /// The walk's view of the moving object `id`'s last human order time (`+0x650`) as the field
    /// changes; nothing for an object without one.
    pub fn set_human_order(&mut self, id: i32, human_order: u32) {
        if let Some(g) = self.gaits.get_mut(&id) {
            g.human_order = human_order;
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

    /// The obstruction query's ignore list for `ids` (−1 for none), as `FUN_14114e8a0` adds each
    /// object: its id, then a building's annexes (`+0xbc0`..`+0xbd8`, in id order).
    #[must_use]
    pub fn query_ignore(&self, ids: &[i32]) -> Vec<i32> {
        let mut ignore = Vec::new();
        for &id in ids.iter().filter(|&&id| id != -1) {
            ignore.push(id);
            ignore.extend(self.annexes_of(id));
        }
        ignore
    }

    /// The annexes of the building `main`, in id order.
    pub fn annexes_of(&self, main: i32) -> Vec<i32> {
        self.annexes
            .iter()
            .filter(|&(_, &m)| m == main)
            .map(|(&a, _)| a)
            .collect()
    }

    /// The main building of the annex `id`, if it is one.
    pub fn annex_of(&self, id: i32) -> Option<i32> {
        self.annexes.get(&id).copied()
    }

    /// `FUN_141663dc0` with the ally flag ([`Spatial::allied`]), for the walk's avoidance.
    pub fn allied_with(&self, from: usize, to: usize) -> bool {
        self.allied(from, to)
    }

    /// The players `from` counts as allies ([`Spatial::allied_with`]), one bit each.
    pub fn allies_of(&self, from: usize) -> u64 {
        (0..64usize)
            .filter(|&to| self.allied(from, to))
            .fold(0, |bits, to| bits | (1 << to))
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

    /// `FUN_1410562e0` on the obstruction manager: its ignore list becomes `ids` (none for an
    /// empty list), until the next call.
    pub fn set_manager_ignore(&self, ids: &[i32]) {
        let mut list = self.manager_ignore.borrow_mut();
        list.clear();
        list.extend_from_slice(ids);
    }

    /// The obstruction manager's ignore list as the last `FUN_1410562e0` left it
    /// ([`Spatial::set_manager_ignore`]).
    #[must_use]
    pub fn manager_ignore(&self) -> Vec<i32> {
        self.manager_ignore.borrow().clone()
    }

    /// `FUN_1410562d0` on the obstruction manager: its buffer becomes `buffer` (None: not set),
    /// until the next call.
    pub fn set_manager_buffer(&self, buffer: Option<f32>) {
        self.manager_buffer.set(buffer);
    }

    /// The obstruction manager's buffer as the last `FUN_1410562d0` left it
    /// ([`Spatial::set_manager_buffer`]).
    #[must_use]
    pub const fn manager_buffer(&self) -> Option<f32> {
        self.manager_buffer.get()
    }

    /// What the queries that set no buffer grow the records by: the buffer the last
    /// `FUN_1410562d0` left, 0 when not set (the `MoveTo`'s target check `FUN_140aa2a70`, the
    /// motion step's waypoint query, the formation's lines `FUN_14103a1f0`, the group's clear
    /// spot `FUN_141007630`).
    #[must_use]
    pub fn manager_grow(&self) -> f32 {
        self.manager_buffer.get().unwrap_or(0.0)
    }

    /// The walk's obstruction query (`FUN_14114cc60` -> `FUN_141051900` with flags `0x90`, each
    /// cell's records by `FUN_141051e00`): the records of a tested kind (kind 10 filed as 2),
    /// except the `ignore`d objects', in the cells the query visits ([`Spatial::visits`], `rect`
    /// grown by `grow`), whose box, grown by `grow` (the walker's radius), meets the step's
    /// rectangle `rect` and whose grown box meets the step `from` -> `to`
    /// (`FUN_1410512f0`; a circle's, kind 5, `FUN_141051630`, alike). A moving object of
    /// `player` or an ally grows by `grow × ally_factor` instead (`+0x84`, the walker's master's
    /// `min_collision_size_multiplier`). The ids hit, in id order; the game's are in its grid's,
    /// which no caller reads yet. A rectangle that, grown, leaves the map meets the map edge's
    /// records first ([`Spatial::edge_records`], `FUN_141057290`). Both its callers
    /// (`FUN_14114cc60`, the gate's `FUN_1411d0570`) set the manager's ignore list to `ignore`
    /// first ([`Spatial::set_manager_ignore`]).
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
    ) -> Result<Vec<i32>, GenieError> {
        self.set_manager_ignore(ignore);
        // its callers reset the buffer after the query (FUN_1410562d0 with 0)
        self.set_manager_buffer(None);
        let [w, h] = self.map;
        let walk = [from[0], from[1], to[0], to[1]];
        let query = [
            rect[0] - grow,
            rect[1] - grow,
            grow + rect[2],
            grow + rect[3],
        ];
        // a query beyond the map's edge meets the edge's records first (FUN_141057290)
        let mut hits: Vec<i32> =
            if query[0] < 0.0 || query[1] < 0.0 || (w as f32) < query[2] || (h as f32) < query[3] {
                self.edge_records(query, QUERY_HITS)
                    .iter()
                    .map(|e| e.id)
                    .collect()
            } else {
                Vec::new()
            };
        // a box that misses the step's rectangle even grown by the larger of the two grows cannot
        // be hit (a smaller grow's box lies inside it): skip it before the grid's filing test,
        // which costs the most. Without a player a moving object's test fails, so then all go
        // through it
        let reach = grow.max(ally_factor * grow);
        let skip = |p: &Placed| {
            let [x0, y0, x1, y1] = p.bounds();
            !(rect[0] <= reach + x1
                && x0 - reach <= rect[2]
                && rect[1] <= reach + y1
                && y0 - reach <= rect[3])
        };
        for (&id, p) in &self.objects {
            let Some(o) = p.obstruction else {
                continue;
            };
            if player.is_some() && skip(p) {
                continue;
            }
            let kind = if o.kind == 10 { 2 } else { o.kind };
            if !TESTED_KINDS.contains(&kind) || ignore.contains(&id) || !self.visits(p, kind, query)
            {
                continue;
            }
            let g = if self.gaits.get(&id).is_some_and(|g| g.moving) {
                let player = player.ok_or(GenieError::unsupported("a walker without a place"))?;
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
                && record_meets_segment(kind, [x0, y0, x1, y1], g, walk)
            {
                hits.push(id);
            }
        }
        Ok(hits)
    }

    /// `FUN_1411570c0` (the caller holds the ends inside the map): the manager's ignore list
    /// becomes `ignore` (`FUN_1410562e0`), with `mask` the temporary records of the tiles of the
    /// line's box grown by `grow` that the mask lacks go in ([`Spatial::put_tile_records`]), the
    /// records the line meets ([`Spatial::line_hits`]), then the buffer the routine set is reset
    /// (`FUN_1410562d0` with 0) and, with `mask`, the tile records go (`FUN_1410561f0`).
    ///
    /// # Errors
    ///
    /// As [`Spatial::line_hits`] (the records go all the same).
    pub fn line_query(
        &self,
        from: [f32; 2],
        to: [f32; 2],
        grow: f32,
        mask: Option<u8>,
        ignore: &[i32],
    ) -> Result<Vec<i32>, GenieError> {
        self.set_manager_ignore(ignore);
        if let Some(mask) = mask {
            self.put_tile_records(from, to, grow, mask);
        }
        let hits = self.line_hits(from, to, grow, true, ignore);
        self.set_manager_buffer(None);
        if mask.is_some() {
            self.clear_tile_records();
        }
        hits
    }

    /// `FUN_1411570c0`'s query (`FUN_141051900` with flags 0x10, any player): the records the
    /// line `from` -> `to` meets, each box grown by `grow` (the manager's buffer,
    /// `FUN_1410562d0`), but the `ignore`d objects' (`FUN_1410562e0`), at most 25 (the query
    /// stops there). First the map edge's records when the grown rectangle leaves the map
    /// ([`Spatial::edge_records`]); with `tiles` the temporary tile records still put in the
    /// cells the query visits ([`Spatial::put_tile_records`], kind 3, the tile's box), in the
    /// tiles' row order; then the objects' records of a tested kind (kind 10 filed as 2) in the
    /// cells the query visits whose grown box meets the line's rectangle and the line
    /// (`FUN_1410512f0`; a circle's alike), in id order. Ids: an object's, a tile record's
    /// ([`tile_record_id`]) or an edge record's.
    ///
    /// # Errors
    ///
    /// A record whose place the slice does not know near the line.
    pub fn line_hits(
        &self,
        from: [f32; 2],
        to: [f32; 2],
        grow: f32,
        tiles: bool,
        ignore: &[i32],
    ) -> Result<Vec<i32>, GenieError> {
        const MOST_HITS: usize = 25;
        let rect = [
            from[0].min(to[0]),
            from[1].min(to[1]),
            from[0].max(to[0]),
            from[1].max(to[1]),
        ];
        let query = [
            rect[0] - grow,
            rect[1] - grow,
            grow + rect[2],
            grow + rect[3],
        ];
        let [w, h] = self.map;
        let mut hits: Vec<i32> = Vec::new();
        if query[0] < 0.0 || query[1] < 0.0 || (w as f32) < query[2] || (h as f32) < query[3] {
            hits.extend(self.edge_records(query, QUERY_HITS).iter().map(|e| e.id));
            if MOST_HITS <= hits.len() {
                hits.truncate(MOST_HITS);
                return Ok(hits);
            }
        }
        self.check_known_records(query)?;
        let walk = [from[0], from[1], to[0], to[1]];
        let meets = |kind: u8, bx: [f32; 4]| {
            let grown = [bx[0] - grow, bx[1] - grow, grow + bx[2], grow + bx[3]];
            rect[0] <= grown[2]
                && grown[0] <= rect[2]
                && rect[1] <= grown[3]
                && grown[1] <= rect[3]
                && record_meets_segment(kind, bx, grow, walk)
        };
        if tiles {
            let level = self.grid.level(0);
            let shift = self.grid.shift(level);
            for &(y, x) in self.tile_records.borrow().iter() {
                let id = tile_record_id(x, y);
                let bx = [x as f32, y as f32, x as f32 + 1.0, y as f32 + 1.0];
                if self.visits_cell(level, x >> shift, y >> shift, query)
                    && !ignore.contains(&id)
                    && meets(3, bx)
                {
                    hits.push(id);
                    if MOST_HITS <= hits.len() {
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
            if !TESTED_KINDS.contains(&kind) || ignore.contains(&id) || !self.visits(p, kind, query)
            {
                continue;
            }
            if !meets(kind, p.bounds()) {
                continue;
            }
            hits.push(id);
            if MOST_HITS <= hits.len() {
                break;
            }
        }
        Ok(hits)
    }

    /// `FormationTransform`'s line query (`FUN_141051900` with flags 0x40, any player, the
    /// segment test `FUN_141051e00`) with the buffer `grow` the formation finds
    /// ([`Spatial::manager_grow`]): whether the line `from` -> `to` meets a record, the
    /// `ignore`d objects' and the moving objects' (flag 0x40, [`Gait::moving`]; the game's list
    /// of moving objects still tested holds only ignored ones here) left out. The records: the
    /// map edge's where the line's rectangle, grown by `grow`, leaves the map
    /// ([`Spatial::edge_records`]), the temporary tile records still put
    /// ([`Spatial::put_tile_records`]: the formation's over its box, kind 3, the tile's box) and
    /// the objects' of a tested kind (kind 10 filed as 2), in the cells the query visits; a
    /// record counts when its box, grown by `grow`, meets the line's rectangle and the line
    /// (`FUN_1410512f0`; a circle's alike).
    ///
    /// # Errors
    ///
    /// A record whose place the slice does not know near the line.
    pub fn still_line_hit(
        &self,
        from: [f32; 2],
        to: [f32; 2],
        grow: f32,
        ignore: &[i32],
    ) -> Result<bool, GenieError> {
        let rect = [
            if to[0] <= from[0] { to[0] } else { from[0] },
            if to[1] <= from[1] { to[1] } else { from[1] },
            if from[0] <= to[0] { to[0] } else { from[0] },
            if from[1] <= to[1] { to[1] } else { from[1] },
        ];
        let query = [
            rect[0] - grow,
            rect[1] - grow,
            grow + rect[2],
            grow + rect[3],
        ];
        let [w, h] = self.map;
        if (query[0] < 0.0 || query[1] < 0.0 || (w as f32) < query[2] || (h as f32) < query[3])
            && !self.edge_records(query, QUERY_HITS).is_empty()
        {
            return Ok(true);
        }
        self.check_known_records(query)?;
        let walk = [from[0], from[1], to[0], to[1]];
        let meets = |kind: u8, bx: [f32; 4]| {
            let grown = [bx[0] - grow, bx[1] - grow, grow + bx[2], grow + bx[3]];
            rect[0] <= grown[2]
                && grown[0] <= rect[2]
                && rect[1] <= grown[3]
                && grown[1] <= rect[3]
                && record_meets_segment(kind, bx, grow, walk)
        };
        let level = self.grid.level(0);
        let shift = self.grid.shift(level);
        let tile = self.tile_records.borrow().iter().any(|&(y, x)| {
            self.visits_cell(level, x >> shift, y >> shift, query)
                && !ignore.contains(&tile_record_id(x, y))
                && meets(3, [x as f32, y as f32, x as f32 + 1.0, y as f32 + 1.0])
        });
        if tile {
            return Ok(true);
        }
        Ok(self.objects.iter().any(|(&id, p)| {
            let Some(o) = p.obstruction else {
                return false;
            };
            let kind = if o.kind == 10 { 2 } else { o.kind };
            TESTED_KINDS.contains(&kind)
                && !ignore.contains(&id)
                && !self.gaits.get(&id).is_some_and(|g| g.moving)
                && self.visits(p, kind, query)
                && meets(kind, p.bounds())
        }))
    }

    /// The object's new place (`new_position`, `0x1411a58b0`): its tile's counts and, on a new
    /// tile, its sight follow.
    pub fn set_position(&mut self, id: i32, position: [f32; 3]) {
        if let Some(p) = self.objects.get_mut(&id) {
            let from = p.position;
            p.position = position;
            let (old, new) = (self.tiles.tile_of(from), self.tiles.tile_of(position));
            if old != new {
                let objects = &self.objects;
                if let Some(i) = old {
                    self.tiles.take_off(i, id, |o| Self::is_tall(objects, o));
                }
                if let Some(i) = new {
                    self.tiles.append(i, id, |o| Self::is_tall(objects, o));
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
    pub fn check_sight(&self, player: usize, rect: [f32; 4]) -> Result<(), GenieError> {
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
            return Err(GenieError::unsupported(
                "a stopped object's sight may reach the search",
            ));
        }
        Ok(())
    }

    /// A unit AI's search ([`Search`]) for object `me` of `player` (`FUN_140c63e20`): the query
    /// over the player's registry ([`crate::registry::Registry::visible_units`]) unless the
    /// [`SearchCache`] holds near the place (`fresh` false: the last answer stands); then its
    /// [`Pick`], an attack by `attack`'s tests of the objects found and its choice among them. A query that reads nothing leaves the last answer too (and so is not fresh). A
    /// query near a frozen object, or a last answer the slice has not got, fails. `shuns`
    /// answers for a found object whether a class-10 searcher passes it ([`Spatial::closest_object`]).
    #[allow(clippy::too_many_arguments)]
    pub fn search(
        &self,
        me: i32,
        player: usize,
        search: &Search<'_>,
        attack: &mut dyn crate::unit_ai::AttackPicker,
        reach: &mut Reach<'_>,
        moving: &dyn Fn(i32) -> bool,
        shuns: &dyn Fn(&Placed) -> Result<bool, GenieError>,
        farm: &FarmLook<'_>,
    ) -> Result<Found, GenieError> {
        let v = self.sight.as_ref().ok_or(GenieError::unsupported(
            "a search without the players' sight",
        ))?;
        let fresh = self.search_query(player, search)?;
        let cache = self.search_cache.borrow();
        let found = cache.last.as_ref().ok_or(GenieError::unsupported(
            "a search that reads the registry's last answer (before any query)",
        ))?;
        self.pick_found(
            v,
            (me, player),
            search,
            found,
            fresh,
            attack,
            reach,
            moving,
            shuns,
            farm,
        )
    }

    /// What player `player`'s queries make of the first `players` players' objects
    /// ([`Spatial::unit_diplomacy`], `player+0x278`).
    ///
    /// # Errors
    ///
    /// A player whose kinds the slice did not load.
    pub fn query_kinds(&self, player: usize, players: usize) -> Result<Vec<u32>, GenieError> {
        let kinds = self
            .unit_diplomacy
            .get(player)
            .ok_or(GenieError::unsupported(
                "a search of a player without its unit diplomacy",
            ))?;
        Ok(kinds.iter().take(players).map(|&k| k as u32).collect())
    }

    /// The query of a unit AI's search (`FUN_140c63e20`'s first part): over the player's
    /// registry unless the [`SearchCache`] holds near the place; true when it queried afresh. The
    /// lists it leaves are [`Spatial::searched`]'s.
    pub fn search_query(&self, player: usize, search: &Search<'_>) -> Result<bool, GenieError> {
        let v = self.sight.as_ref().ok_or(GenieError::unsupported(
            "a search without the players' sight",
        ))?;
        let r = v.registry.as_ref().ok_or(GenieError::unsupported(
            "a search without the objects-in-sight registry",
        ))?;
        let relations = self.query_kinds(player, r.players)?;
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
        Ok(fresh)
    }

    /// The lists the last search left (`DAT_143f69178`, by relation kind; 4 the enemies), as
    /// [`Spatial::search_query`] left them.
    pub fn searched(&self) -> Option<[Vec<Record>; 5]> {
        self.search_cache.borrow().last.clone()
    }

    /// The search's pick of what it found ([`Spatial::search`]).
    #[allow(clippy::too_many_arguments)]
    fn pick_found(
        &self,
        v: &Visibility,
        (me, player): (i32, usize),
        search: &Search<'_>,
        found: &[Vec<Record>; 5],
        fresh: bool,
        attack: &mut dyn crate::unit_ai::AttackPicker,
        reach: &mut Reach<'_>,
        moving: &dyn Fn(i32) -> bool,
        shuns: &dyn Fn(&Placed) -> Result<bool, GenieError>,
        farm: &FarmLook<'_>,
    ) -> Result<Found, GenieError> {
        match search.pick {
            Pick::Closest {
                group,
                relation,
                state,
                master,
                workers,
                spared,
                reach: reaching,
            } => {
                let filter = Filter {
                    group,
                    relation,
                    state,
                    master,
                    workers,
                    spared,
                    reach: reaching,
                };
                let checks = (moving, shuns, farm);
                let (id, distance) =
                    self.closest_object(v, me, player, found, filter, reach, checks)?;
                Ok(Found {
                    id,
                    enemies: !found[4].is_empty(),
                    position: self.objects.get(&id).map(|p| p.position),
                    fresh,
                    players: players_seen(found),
                    distance: crate::checkpoint::cvttss2si(distance),
                    retries: [None; 3],
                })
            }
            Pick::Look => Ok(Found {
                id: -1,
                enemies: !found[4].is_empty(),
                position: None,
                fresh,
                players: 0,
                distance: -1,
                retries: [None; 3],
            }),
            Pick::Attack { ask, .. } => {
                let choice = Self::best_unit_to_attack(found, &ask, search.memory, attack, reach)?;
                let id = choice.target;
                let mut retries = [None; 3];
                for (slot, r) in retries.iter_mut().zip(choice.retries) {
                    *slot = Some(r);
                }
                Ok(Found {
                    id,
                    enemies: !found[4].is_empty(),
                    position: self.objects.get(&id).map(|p| p.position),
                    fresh,
                    players: 0,
                    distance: -1,
                    retries,
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
    /// (`moving`: slot 0x288 not −1 and slot 0x530 107, "Sheep is moving"). A searcher of class
    /// 10 passes the objects `shuns` names (of master 440 or a master with object flag 0x10; the
    /// game's `+0x6b4` of 1, not met, would let it take them). Not modelled: a workers cap
    /// (argument 6; the objects' `+0x644`), the unbuilt farm test of a search for state 0, the
    /// pastures and farms (class 49). Nothing found answers −1.
    #[allow(clippy::too_many_arguments)]
    fn closest_object(
        &self,
        v: &Visibility,
        me: i32,
        player: usize,
        found: &[Vec<Record>; 5],
        filter: Filter,
        reach: &mut Reach<'_>,
        (moving, shuns, farm): ClosestChecks<'_>,
    ) -> Result<(i32, f32), GenieError> {
        let class_10 = self.objects.get(&me).is_some_and(|m| m.class == 10);
        let farmer = self
            .objects
            .get(&me)
            .is_some_and(|m| m.class == FARMER_CLASS);
        let trapper = self
            .objects
            .get(&me)
            .is_some_and(|m| TRAPPERS.contains(&m.master));
        // FUN_140c32b50: the relation argument 9 passes over, as a list
        let spared = match filter.spared {
            0 => Some(2),
            1 => Some(3),
            2 => Some(4),
            3 => Some(1),
            _ => None,
        };
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
                if 7 <= seer.state || !related || spared == Some(kind) {
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
                    return Err(GenieError::unsupported(
                        "ClosestObject's workers cap (+0x644)",
                    ));
                }
                if filter.state == 0 {
                    return Err(GenieError::unsupported("ClosestObject's unbuilt farm test"));
                }
                // a farm (class 49): not a pasture's livestock (`+0x790`); a pasture is not
                // modelled; without a master asked for, one without workers that the unit
                // gathers from (a fish trap for a fishing ship, any other for a villager)
                if filter.group == PASTURE_CLASS {
                    let (workers, parent) = farm(record.id).ok_or(GenieError::unsupported(
                        "a found farm the slice does not hold",
                    ))?;
                    if parent != -1 {
                        continue;
                    }
                    if PASTURES.contains(&p.master) {
                        return Err(GenieError::unsupported(
                            "ClosestObject over pastures (their workers and livestock)",
                        ));
                    }
                    if filter.master == -1 {
                        if 1 <= workers {
                            continue;
                        }
                        if !trapper && p.master == FISH_TRAP {
                            continue;
                        }
                        if !farmer && p.master != FISH_TRAP {
                            continue;
                        }
                    }
                }
                if filter.group == LIVESTOCK_CLASS
                    && ((p.player != player && seer.state == 2) || moving(record.id))
                {
                    continue;
                }
                if class_10 && shuns(p)? {
                    continue;
                }
                if filter.reach && !reach(record.id, CLOSEST_RANGE)? {
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
        // the distance starts at `DAT_14330ec20` (−1) without a find
        Ok(best.unwrap_or((-1, -1.0)))
    }

    /// `bestUnitToAttack` (`FUN_140c44680`, [`crate::best_target::best_unit_to_attack`]) over
    /// the relation lists its search `found` (each record's id in order), with the unit AI's
    /// `memory`: `attack` reads the unit and the objects, the paths (slot 0x3d0) are `reach`'s.
    ///
    /// # Errors
    ///
    /// What the reads and tests do not model.
    fn best_unit_to_attack(
        found: &[Vec<Record>; 5],
        ask: &crate::unit_ai::AttackAsk,
        memory: crate::unit_ai::Memory<'_>,
        attack: &mut dyn crate::unit_ai::AttackPicker,
        reach: &mut Reach<'_>,
    ) -> Result<crate::best_target::Choice, GenieError> {
        let me = attack.searcher(memory)?;
        let lists: [Vec<i32>; 5] = std::array::from_fn(|k| found[k].iter().map(|r| r.id).collect());
        let mut field = AttackField {
            picker: attack,
            reach,
        };
        crate::best_target::best_unit_to_attack(&me, &ask.ask(), &lists, &mut field)
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
    ) -> Result<Option<usize>, GenieError> {
        const KEPT: [i16; 16] = [1, 60, 42, 11, 14, 30, 58, 3, 52, 27, 39, 49, 37, 9, 10, 38];
        // a world without the players' sight (the unit tests') holds nobody near
        let Some(v) = self.sight.as_ref() else {
            return Ok(None);
        };
        // an animal inside another object (in this update, its Artifact's Enter got it in)
        // searches about its record there
        let p = self
            .objects
            .get(&me)
            .or_else(|| self.inside.get(&me))
            .ok_or(GenieError::unsupported(
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
            return Err(GenieError::unsupported(
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
                return Err(GenieError::unsupported(
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

    /// An ownership change's records ([`Visibility::records_owner`]), which come first.
    pub fn records_owner(&mut self, id: i32, new: usize) -> Result<(), GenieError> {
        self.sight
            .as_mut()
            .map_or(Ok(()), |v| v.records_owner(id, new))
    }

    /// An ownership change's world part (`FUN_14120f800`, once the new owner has the master):
    /// the object's place takes the new owner, and an object that `sees` moves its sight to the
    /// new owner's map with `los` ([`Visibility::owner_changed`]).
    pub fn change_owner(
        &mut self,
        id: i32,
        new: usize,
        los: f32,
        sees: bool,
    ) -> Result<(), GenieError> {
        let Some(p) = self.objects.get_mut(&id) else {
            return Err(GenieError::unsupported(
                "an ownership change of an object the world does not hold",
            ));
        };
        p.player = new;
        let (position, radius) = (p.position, p.radius);
        if let Some(v) = self.sight.as_mut() {
            v.owner_changed(id, position, radius, (new, los), sees);
        }
        Ok(())
    }

    /// The vision masks of player `player` (`+0x2a8`, `+0x2a0`; [`crate::vision`]): the world's
    /// rebuild's, else its own bits and the ones no tile has.
    #[must_use]
    pub fn vision(&self, player: usize) -> [u64; 2] {
        self.vision
            .get(player)
            .copied()
            .unwrap_or_else(|| crate::vision::own_masks(player, false))
    }

    /// The map revealed to player `player` (`FUN_141639860` -> `FUN_1412847b0` with
    /// `0x4000000040000000`): every word of the unified map takes the ended bits 30 and 62, and
    /// the player's masks (`+0x2a8`, `+0x2a0`) too.
    pub fn reveal_to(&mut self, player: usize) {
        use crate::vision::{ENDED, ENDED_EXPLORED};
        self.reveal_map(ENDED | ENDED_EXPLORED);
        if let Some(m) = self.vision.get_mut(player) {
            m[0] |= ENDED;
            m[1] |= ENDED_EXPLORED;
        }
    }

    /// The unified map's word (`+0x5e30`) at tile (`x`, `y`), none off the map.
    fn unified_word(v: &Visibility, x: i32, y: i32) -> Option<u64> {
        if x < 0 || y < 0 || v.width <= x || v.height <= y {
            return None;
        }
        Some(v.unified[(y * v.width + x) as usize])
    }

    /// `FUN_141663500`, the player's tile test: what player `player` knows of tile (`x`, `y`).
    /// 0xf when it sees the tile (its visible mask `+0x2a8` meets the unified map's word under
    /// [`crate::registry::MASK`]), or when the word is revealed to all (bit 29,
    /// [`crate::vision::NO_TILE`]) and the player explored it (its explored mask `+0x2a0` meets
    /// the word under the mask) or the word is explored by all (bit 61); else 0x80 when it
    /// explored it or the word is explored by all; else 0, also off the map. None without the
    /// players' sight.
    #[must_use]
    pub fn tile_sight(&self, player: usize, x: i32, y: i32) -> Option<u8> {
        use crate::registry::MASK;
        use crate::vision::{NO_TILE, NO_TILE_EXPLORED};
        let Some(word) = Self::unified_word(self.sight.as_ref()?, x, y) else {
            return Some(0);
        };
        let [seen, explored] = self.vision(player);
        let explored = explored & word & MASK != 0;
        let all_explored = word & NO_TILE_EXPLORED != 0;
        let revealed = word & NO_TILE != 0;
        if seen & word & MASK != 0 || (revealed && (explored || all_explored)) {
            return Some(0xf);
        }
        Some(if explored || all_explored { 0x80 } else { 0 })
    }

    /// A building's slot 0x1e0 (`FUN_1411d3280`) for `player`, as far as the world holds it:
    /// the player's tile test of its place (`FUN_141193840` -> `FUN_1416635e0`, its coordinates
    /// truncated, [`Spatial::tile_sight`]); short of 0xf, 0xf when the building (type 50 and up)
    /// follows its footprint's UMV entry (`+0x904`) and its visibility mask (`+0x8f8`, the
    /// registry's) meets the player's visible or explored mask (`player+0x2a8`, `+0x2a0`). None
    /// when neither: a ghost of it on its tile that the player is not among the blind of decides
    /// then (0xf), which the world does not hold. Without the players' sight: 0.
    #[must_use]
    pub fn building_sight(&self, id: i32, position: [f32; 3], player: usize) -> Option<u8> {
        use crate::checkpoint::cvttss2si;
        let Some(v) = self.sight.as_ref() else {
            return Some(0);
        };
        let (x, y) = (cvttss2si(position[0]), cvttss2si(position[1]));
        if self.tile_sight(player, x, y) == Some(0xf) {
            return Some(0xf);
        }
        let umv = v.seer(id).is_some_and(|s| s.umv != 0);
        let mask = v
            .registry
            .as_ref()
            .and_then(|r| r.entries.get(&id))
            .map_or(0, |e| e.mask);
        let [visible, explored] = self.vision(player);
        if umv && (visible & mask != 0 || explored & mask != 0) {
            return Some(0xf);
        }
        None
    }

    /// `FUN_141285350`, the visible map's tile test (`InfluenceMap` and the scout map read it):
    /// 0xf when player `player`'s visible mask (`+0x2a8`) meets the unified map's word at tile
    /// (`x`, `y`) under [`crate::registry::MASK`], else 0x80 when its explored mask (`+0x2a0`)
    /// does, else 0, also off the map (the visible map's own size, `+0x20`, `+0x24`, the map's).
    /// The revealed bits 29 and 61 lie outside the mask. None without the players' sight.
    #[must_use]
    pub fn scout_sight(&self, player: usize, x: i32, y: i32) -> Option<u8> {
        let Some(word) = Self::unified_word(self.sight.as_ref()?, x, y) else {
            return Some(0);
        };
        let word = word & crate::registry::MASK;
        let [seen, explored] = self.vision(player);
        Some(if seen & word != 0 {
            0xf
        } else if explored & word != 0 {
            0x80
        } else {
            0
        })
    }

    /// `FUN_140c68a00`, a unit's tile test (`FUN_141285280` reads the unified map's word, its
    /// player the unit's `+0x138`): 0xf when player `player`'s visible mask (`+0x2a8`) meets the
    /// word at tile (`x`, `y`) under [`crate::registry::MASK`], else 0x80 when its explored mask
    /// (`+0x2a0`) does, else 0. The game reads the word unchecked; its three callers
    /// (`FUN_140ab79e0`, `FUN_140c567c0`, `FUN_140c26f60`) pass an object's tile, which the
    /// position setter holds on the map, so off the map is 0 here. None without the players'
    /// sight.
    #[must_use]
    pub fn visibility(&self, player: usize, x: i32, y: i32) -> Option<u8> {
        let Some(word) = Self::unified_word(self.sight.as_ref()?, x, y) else {
            return Some(0);
        };
        let word = word & crate::registry::MASK;
        let [seen, explored] = self.vision(player);
        Some(if seen & word != 0 {
            0xf
        } else if explored & word != 0 {
            0x80
        } else {
            0
        })
    }

    /// `FUN_1412847b0`: every word of the unified map (`+0x5e30`, width × height) takes the bits
    /// of `mask` (the end of the game `0x4000000040000000`, a revealed map `0x2000000020000000`,
    /// both `0x6000000060000000`). Nothing without the players' sight.
    pub fn reveal_map(&mut self, mask: u64) {
        if let Some(v) = self.sight.as_mut() {
            for w in &mut v.unified {
                *w |= mask;
            }
        }
    }

    /// Whether player `player` sees tile (`x`, `y`): [`Spatial::tile_sight`] is 0xf. False
    /// without the players' sight.
    pub fn sees_tile(&self, player: usize, x: i32, y: i32) -> bool {
        self.tile_sight(player, x, y) == Some(0xf)
    }

    /// [`Spatial::tile_sight`] is not 0: player `player` sees tile (`x`, `y`) or explored it.
    /// Unknown without the players' sight.
    pub fn explored(&self, player: usize, x: i32, y: i32) -> Result<bool, GenieError> {
        let answer = self
            .tile_sight(player, x, y)
            .ok_or(GenieError::unsupported(
                "a test of explored tiles without the players' sight",
            ))?;
        Ok(answer != 0)
    }

    /// [`Spatial::visibility`] is 0xf: player `player` sees tile (`x`, `y`) now. False without
    /// the players' sight.
    pub fn visible(&self, player: usize, x: i32, y: i32) -> bool {
        self.visibility(player, x, y) == Some(0xf)
    }

    /// `RGE_Combat_Object::update`'s sight ([`Visibility::combat_sight`]); an annex reads its
    /// main building's place and UMV entry. The changed mask of an object others take over by
    /// seeing it is the answer.
    pub fn combat_sight(&mut self, id: i32) -> Result<Option<u64>, GenieError> {
        let (Some(v), Some(p)) = (self.sight.as_mut(), self.objects.get(&id)) else {
            return Ok(None);
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
    ) -> Result<(), GenieError> {
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
    pub fn take_sight_unsupported(&mut self) -> Option<GenieError> {
        self.sight.as_mut().and_then(Visibility::take_unsupported)
    }

    /// A player's ghost `id` of an object it saw (a doppelganger, type 25): it is on its tile's
    /// list, and movement sees nothing else of it. The slice keeps the ghosts of the save and
    /// makes none. The tile's tall count is redone ([`Tiles`]' append, `FUN_140f5f0a0`); the
    /// game counts the ghost by its master's height radius, which the slice does not hold, so
    /// the ghost counts as not tall.
    pub fn insert_ghost(&mut self, id: i32, position: [f32; 3]) {
        if let Some(i) = self.tiles.tile_of(position) {
            let objects = &self.objects;
            self.tiles.append(i, id, |o| Self::is_tall(objects, o));
        }
    }

    /// A ghost leaves its tile's list (its last object takes its place) and the tile's tall count
    /// is redone (`FUN_140f5f560`).
    pub fn remove_ghost(&mut self, id: i32, position: [f32; 3]) {
        if let Some(i) = self.tiles.tile_of(position) {
            let objects = &self.objects;
            self.tiles.take_off(i, id, |o| Self::is_tall(objects, o));
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

    /// `FUN_141185690`: tile (`x`, `y`) lies on the obstruction manager's map, its terrain class
    /// is in `mask` and no static record blocks it.
    #[must_use]
    pub fn standable(&self, x: i32, y: i32, mask: u8) -> bool {
        self.tiles
            .word(x, y)
            .is_some_and(|w| w & (u64::from(mask ^ 0xff) | BLOCKED) == 0)
    }

    /// Whether unit `me` (its x radius `radius_x`) with terrain mask `mask` can stand on tile
    /// (`x`, `y`) (`FUN_14114d460`): the tile is on the map, its class is in the mask and no
    /// static record blocks it; on a tile with two tall objects or more, the obstruction
    /// manager's point query at the tile's corner (`FUN_141051900` with flags 0x20, the records
    /// grown by half the x radius, `FUN_1410562d0`, the unit itself ignored, `FUN_1410562e0`)
    /// finds none ([`Spatial::point_clear`]).
    ///
    /// # Panics
    ///
    /// Never: the tall-object grid covers the map as the word grid does.
    pub fn passable(
        &self,
        x: i32,
        y: i32,
        mask: u8,
        me: i32,
        radius_x: f32,
    ) -> Result<bool, GenieError> {
        let Some(word) = self.tiles.word(x, y) else {
            return Ok(false);
        };
        if word & (u64::from(mask ^ 0xff) | BLOCKED) != 0 {
            return Ok(false);
        }
        if 1 < self.tiles.tall(x, y).expect("a tile with a word") {
            return self.point_clear(me, [x as f32, y as f32], radius_x * 0.5, mask);
        }
        Ok(true)
    }

    /// The tile beside `site`'s footprint a unit at `from` with terrain mask `mask` goes to
    /// (`FUN_1411566b0`, closest): [`Spatial::spot_off`] with the site's place and radii.
    ///
    /// # Errors
    ///
    /// As [`Spatial::spot_off`].
    pub fn spot_beside(
        &self,
        from: [f32; 3],
        me: (i32, f32),
        mask: u8,
        site: &Placed,
    ) -> Result<Option<[f32; 2]>, GenieError> {
        self.spot_off(from, me, mask, (site.position, site.radius), false)
    }

    /// `FUN_1411566b0`: the tile beside a footprint at `at` of radii `radius` a unit at `from`
    /// with terrain mask `mask` goes to: a flood over passable tiles from the footprint's own
    /// (`(int)(x - rx)` to `(int)(x + rx - 0.001)`, columns first), at most `(int)(rx + ry) + 1`
    /// tiles from its centre; of the tiles found outside the footprint, the first empty one, else
    /// by the 3D distance of its centre (at the unit's height) from the unit the closest, or with
    /// `farthest` (`param_7` 1) the farthest, later ones replacing it in the flood's order once
    /// they beat it (an empty tile found, only empty ones). Returns its centre.
    ///
    /// # Errors
    ///
    /// A tile whose objects the slice does not know.
    pub fn spot_off(
        &self,
        from: [f32; 3],
        (me, radius_x): (i32, f32),
        mask: u8,
        (at, radius): ([f32; 3], [f32; 2]),
        farthest: bool,
    ) -> Result<Option<[f32; 2]>, GenieError> {
        /// The footprint's last tile stops short of its edge by this (`DAT_142eb1e9c`).
        const EDGE: f32 = crate::image::spatial::FOOTPRINT_EDGE;
        /// A tile's centre (`DAT_142e7f384`).
        const CENTRE: f32 = crate::image::spatial::TILE_CENTRE;
        /// The closest search's first best (`DAT_14330ebf4`).
        const NO_SPOT: f32 = crate::image::spatial::NO_SPOT;
        let [tx, ty, _] = at;
        let [rx, ry] = radius;
        let size = (((ry + rx) as i32) + 2).max(1) * 2;
        let half = size / 2;
        let (cx, cy) = (tx as i32, ty as i32);
        let (fx, fy) = ((cx - half) as f32, (cy - half) as f32);
        self.check_known([fx, fy, fx + size as f32, fy + size as f32])?;
        let (ox, oy) = (half - cx, half - cy);
        let (x0, y0) = ((tx - rx) as i32, (ty - ry) as i32);
        let (x1, y1) = (((rx + tx) - EDGE) as i32, ((ry + ty) - EDGE) as i32);
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
                    if !visited[at] && self.passable(nx, ny, mask, me, radius_x)? {
                        visited[at] = true;
                        list.push((nx, ny));
                    }
                }
            }
            i += 1;
        }
        let mut best = if farthest { 0.0 } else { NO_SPOT };
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
            let centre = [x as f32 + CENTRE, from[2], y as f32 + CENTRE];
            let d = [
                from[0] - centre[0],
                from[2] - centre[1],
                from[1] - centre[2],
            ];
            let distance = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            let beats = if farthest {
                best < distance
            } else {
                distance < best
            };
            if (empty && !was_empty) || beats {
                spot = Some([centre[0], centre[2]]);
                best = distance;
            }
        }
        Ok(spot)
    }

    /// The objects whose centre is on `site`'s footprint, edges included, and that the game counts
    /// as occupying it (`FUN_14123bb20`): objects with all three radii, other than the site,
    /// unless the site may be built over them (`can_be_built_on` 1); for a site whose master
    /// does not obstruct (`FUN_1411205c0`: obstruction type 0 or 12, `obstructs` false) only
    /// those that do not move (speed 0, slot 0xe8).
    pub fn footprint_occupants(
        &self,
        site_id: i32,
        site: &Placed,
        obstructs: bool,
    ) -> Result<Vec<i32>, GenieError> {
        let [sx, sy, _] = site.position;
        let [rx, ry] = site.radius;
        self.check_known([sx - rx, sy - ry, sx + rx, sy + ry])?;
        Ok(self
            .objects
            .iter()
            .filter(|&(&id, o)| {
                id != site_id
                    && o.buildable_on != 1
                    && (obstructs || o.speed <= 0.0)
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
    pub fn footprint_clearing(&self, site_id: i32, site: &Placed) -> Result<Vec<i32>, GenieError> {
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
    /// The edges cast by `CVTTSS2SI` (a NaN or huge edge gives `i32::MIN`: the loops then do
    /// not run), the tile's players the low 32 bits of `word >> 23` against `1 << (player &
    /// 0x1f)` in 32 bits.
    pub fn foreign_marks(&self, site: &Placed) -> bool {
        use crate::checkpoint::cvttss2si;
        let [sx, sy, _] = site.position;
        let [rx, ry] = site.radius;
        let (w, h) = (self.tiles.width, self.tiles.height);
        let (x0, y0) = (cvttss2si(sx - rx).max(0), cvttss2si(sy - ry).max(0));
        let x1 = cvttss2si((sx + rx) - 0.001).min(w - 1);
        let y1 = cvttss2si((sy + ry) - 0.001).min(h - 1);
        let own = 1u32 << (site.player as u32 & 0x1f);
        (y0..=y1).any(|y| {
            (x0..=x1).any(|x| {
                let players = (self.tiles.word(x, y).unwrap_or(0) >> 23) as u32;
                players != 0 && players != own
            })
        })
    }

    /// Slot 0x278 of unit `me` (`FUN_141158720`): whether it could stand at `point`. The point's
    /// tile is on the obstruction manager's map and its terrain class in the unit's `mask` (the
    /// tile's word, `DAT_143f0a088 + 0x98`; a static record's own bits do not count here), and no
    /// obstruction record but the unit's own holds the point within its box grown by `grow` (the
    /// unit's radius): `FUN_141051900` with flags 0x20 in the cells it visits
    /// ([`Spatial::visits`]), whose point test `FUN_141052160` takes records of every kind, edges
    /// included. A grown point beyond the map's edge meets the edge's
    /// records ([`Spatial::edge_records`]) and is never clear.
    pub fn point_clear(
        &self,
        me: i32,
        point: [f32; 2],
        grow: f32,
        mask: u8,
    ) -> Result<bool, GenieError> {
        let [x, y] = point;
        let (tx, ty) = (x as i32, y as i32);
        let Some(word) = self.tiles.word(tx, ty) else {
            return Ok(false);
        };
        if word & u64::from(mask ^ 0xff) != 0 {
            return Ok(false);
        }
        // the unit alone the manager's ignore list (FUN_1410562e0 with DAT_143f49fc8); the
        // buffer it set for the query it resets after (FUN_1410562d0 with 0)
        self.set_manager_ignore(&[me]);
        self.set_manager_buffer(None);
        Ok(!self.point_hit(point, grow, &[me], false)?)
    }

    /// Slot 0x270 of unit `me` (`FUN_141158940`): whether it could stand at `point` among the
    /// objects of `ignore` (`me` itself for an empty list): the point's tile is on the
    /// obstruction manager's map and its terrain class in the unit's `mask` (its restriction's,
    /// `FUN_141054630`), and no record but the ignored ones holds the point within its box grown
    /// by `grow` (the unit's radius times the caller's factor), the query of
    /// [`Spatial::point_hit`] with 25 hits.
    ///
    /// # Errors
    ///
    /// As [`Spatial::point_hit`].
    pub fn point_clear_among(
        &self,
        me: i32,
        point: [f32; 2],
        grow: f32,
        mask: u8,
        ignore: &[i32],
    ) -> Result<bool, GenieError> {
        use crate::checkpoint::cvttss2si;
        // CVTTSS2SI: a NaN or huge coordinate is i32::MIN, off the map
        let Some(word) = self.tiles.word(cvttss2si(point[0]), cvttss2si(point[1])) else {
            return Ok(false);
        };
        if word & u64::from(mask ^ 0xff) != 0 {
            return Ok(false);
        }
        let ignore = if ignore.is_empty() { &[me][..] } else { ignore };
        self.set_manager_ignore(ignore);
        // the buffer it set for the query it resets after (FUN_1410562d0 with 0)
        self.set_manager_buffer(None);
        Ok(!self.point_hit(point, grow, ignore, false)?)
    }

    /// The obstruction manager's point query (`FUN_141051900` with flags 0x20, any player, the
    /// point test `FUN_141052160`) at `point` with the manager's buffer `grow` (`+0x57`, `+0x58`)
    /// and ignore list `ignore` (`+0x2c`, `FUN_1410562e0`): whether a record holds the point
    /// within its box grown by `grow`, edges included, of the records of every kind in the cells
    /// the query visits ([`Spatial::visits`]), the temporary tile records still put among them
    /// ([`Spatial::put_tile_records`]); with `still` (flag 0x40) an object that moves (the
    /// record's `+3` bit 0x10, [`Gait::moving`]) is left out. A grown point beyond the map's edge
    /// meets the edge's records ([`Spatial::edge_records`]).
    ///
    /// # Errors
    ///
    /// A record whose place the slice does not know near the point.
    pub fn point_hit(
        &self,
        point: [f32; 2],
        grow: f32,
        ignore: &[i32],
        still: bool,
    ) -> Result<bool, GenieError> {
        let [x, y] = point;
        let [w, h] = self.map;
        let rect = [x - grow, y - grow, grow + x, grow + y];
        if (rect[0] < 0.0 || rect[1] < 0.0 || (w as f32) < rect[2] || (h as f32) < rect[3])
            && !self.edge_records(rect, QUERY_HITS).is_empty()
        {
            return Ok(true);
        }
        let level = self.grid.level(0);
        let shift = self.grid.shift(level);
        let tile = self.tile_records.borrow().iter().any(|&(ty, tx)| {
            let [x0, y0, x1, y1] = [tx as f32, ty as f32, tx as f32 + 1.0, ty as f32 + 1.0];
            self.visits_cell(level, tx >> shift, ty >> shift, rect)
                && !ignore.contains(&tile_record_id(tx, ty))
                && x <= x1 + grow
                && x0 - grow <= x
                && y <= grow + y1
                && y0 - grow <= y
        });
        if tile {
            return Ok(true);
        }
        self.object_holds(point, grow, ignore, still)
    }

    /// The objects' records of [`Spatial::point_hit`]'s query (`FUN_141051900` with flags 0x20,
    /// the point test `FUN_141052160`): whether a record with an object (`+8`), not one of
    /// `ignore`, holds `point` within its box grown by `grow`, edges included (with `still` a
    /// moving object's is left out); the edge's and the tiles' records have no object.
    ///
    /// # Errors
    ///
    /// A record whose place the slice does not know near the point.
    pub fn object_holds(
        &self,
        point: [f32; 2],
        grow: f32,
        ignore: &[i32],
        still: bool,
    ) -> Result<bool, GenieError> {
        let [x, y] = point;
        let rect = [x - grow, y - grow, grow + x, grow + y];
        self.check_known_records(rect)?;
        Ok(self.objects.iter().any(|(id, p)| {
            let Some(o) = p.obstruction.filter(|_| !ignore.contains(id)) else {
                return false;
            };
            if still && self.gaits.get(id).is_some_and(|g| g.moving) {
                return false;
            }
            if !self.visits(p, o.kind, rect) {
                return false;
            }
            let [x0, y0, x1, y1] = p.bounds();
            x <= x1 + grow && x0 - grow <= x && y <= grow + y1 && y0 - grow <= y
        }))
    }

    /// Whether `FUN_141052b90` over the tile under `point` alone puts a temporary record on it
    /// (`FUN_1410510c0`): the tile is on the map and its word holds neither a terrain class of
    /// `mask` nor a temporary record's bit 0x80 (`mask & 0x7f | 0x80`).
    #[must_use]
    pub fn makes_tile_record(&self, point: [f32; 2], mask: u8) -> bool {
        let (x, y) = (point[0] as i32, point[1] as i32);
        self.tiles
            .word(x, y)
            .is_some_and(|w| w & u64::from(mask & 0x7f | 0x80) == 0)
            && !self.tile_records.borrow().contains(&(y, x))
    }

    /// The records a move query could report (`FUN_14114cc60` -> `FUN_141051900`): those of a
    /// tested kind in the cells the query visits ([`Spatial::visits`]) whose box, grown by
    /// `grow`, meets `rect` (`x0, y0, x1, y1`, edges included), except the `ignore`d objects'. The game then tests each candidate against the move's
    /// segment and shrinks moving allies' boxes (master `+0x780`, at most 1); every hit is a
    /// candidate, so no candidate means no hit.
    pub fn obstruction_candidates(&self, rect: [f32; 4], grow: f32, ignore: &[i32]) -> Vec<i32> {
        let query = [
            rect[0] - grow,
            rect[1] - grow,
            grow + rect[2],
            grow + rect[3],
        ];
        self.objects
            .iter()
            .filter(|(id, p)| {
                p.obstruction.is_some_and(|o| {
                    TESTED_KINDS.contains(&o.kind) && self.visits(p, o.kind, query)
                }) && !ignore.contains(id)
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
    /// tested), after `FUN_141052b90` put the temporary records (kind 3, the tile's box) of the
    /// walk's box, grown by `buffer`, that the unit's `mask` lacks ([`Spatial::put_tile_records`];
    /// they stay with those of the pather's earlier walks until `FindShortPath` removes them,
    /// [`Spatial::clear_tile_records`]): the records whose box, grown by `buffer`, meets
    /// the walk `a` -> `b` (`FUN_1410512f0`, a circle's `FUN_141051630` alike), whose cell the
    /// query visits ([`Spatial::visits`]) and whose id is not in `ignore` (the manager's list
    /// `+0x2c`, the pather's). An object that moves (its record's
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
    ) -> Result<Vec<Hit>, GenieError> {
        const MOST_HITS: usize = 25;
        let lo_x = if b[0] <= a[0] { b[0] } else { a[0] };
        let hi_x = if a[0] <= b[0] { b[0] } else { a[0] };
        let lo_y = if b[1] <= a[1] { b[1] } else { a[1] };
        let hi_y = if a[1] <= b[1] { b[1] } else { a[1] };
        let g = buffer.unwrap_or(0.0);
        self.put_tile_records(a, b, g, mask);
        let [w, h] = self.map;
        let grown = [lo_x - g, lo_y - g, g + hi_x, g + hi_y];
        let mut hits = Vec::new();
        if grown[0] < 0.0 || grown[1] < 0.0 || (w as f32) < grown[2] || (h as f32) < grown[3] {
            hits = self.edge_records(grown, QUERY_HITS);
            if first {
                return Ok(hits);
            }
        }
        self.check_known_records(grown)?;
        let walk = [a[0], a[1], b[0], b[1]];
        let meets = |kind: u8, bx: [f32; 4]| {
            let r = [bx[0] - g, bx[1] - g, g + bx[2], g + bx[3]];
            lo_x <= r[2]
                && r[0] <= hi_x
                && lo_y <= r[3]
                && r[1] <= hi_y
                && record_meets_segment(kind, bx, g, walk)
        };
        let level = self.grid.level(0);
        let shift = self.grid.shift(level);
        for &(y, x) in self.tile_records.borrow().iter() {
            let id = tile_record_id(x, y);
            let bx = [x as f32, y as f32, x as f32 + 1.0, y as f32 + 1.0];
            if self.visits_cell(level, x >> shift, y >> shift, grown)
                && !ignore.contains(&id)
                && meets(3, bx)
            {
                hits.push(Hit { id, bounds: bx });
                if first || MOST_HITS <= hits.len() {
                    return Ok(hits);
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
                || !self.visits(p, kind, grown)
            {
                continue;
            }
            let bx = p.bounds();
            if !meets(kind, bx) {
                continue;
            }
            if kind == 11 {
                return Err(GenieError::unsupported(
                    "a pather query meeting a record of kind 11 (its own radii)",
                ));
            }
            if p.class == GATE_CLASS {
                return Err(GenieError::unsupported(
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
    ) -> Result<bool, GenieError> {
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
    ) -> Result<Vec<Hit>, GenieError> {
        self.pather_query(a, b, buffer, mask, ignore, false)
    }
}

impl Spatial {
    /// The map edge's records a query rectangle `rect` (`x0, y0, x1, y1`) that leaves the map meets
    /// (`FUN_141057290`, which `FUN_141051900` lists first, before any test of kind, flags or
    /// ignore list): for each edge it crosses (left, top, right, bottom), the manager's record for
    /// that edge (`+0xb0`, `+0xb8`, `+0xc0`, `+0xc8`), set to the tile box where the rectangle's
    /// diagonal `(x0, y0)` -> `(x1, y1)` crosses the edge's line, id `x << 12 | y | 0xff000000`.
    /// The list takes at most `cap` records (its `*param_6`): the routine returns once the list
    /// holds that many, after any edge. The terms are the game's, each with its zero factor
    /// (`0 × dy0`, `(w − w) × dy`, ...), so an infinite or NaN operand gives NaN as there; each
    /// crossing is cast by `CVTTSS2SI` into 32 bits (a NaN or huge one is `i32::MIN`).
    #[must_use]
    // the game's (w - w) and (h - h) factors: 0, but NaN times an infinite or NaN term
    #[allow(clippy::eq_op)]
    pub fn edge_records(&self, rect: [f32; 4], cap: usize) -> Vec<Hit> {
        use crate::checkpoint::cvttss2si;
        let [x0, y0, x1, y1] = rect;
        // the manager's map size as floats (+0xa8, +0xac)
        let (w, h) = (self.map[0] as f32, self.map[1] as f32);
        let (dx, dy) = (x1 - x0, y1 - y0);
        let (dy0, dx0) = (y0 - 0.0, x0 - 0.0);
        let at = |t: f32| {
            let x = cvttss2si(dx * t + x0);
            let y = cvttss2si(dy * t + y0);
            Hit {
                id: ((x << 12) | y | 0xff00_0000_u32 as i32),
                bounds: [x as f32, y as f32, x as f32 + 1.0, y as f32 + 1.0],
            }
        };
        let mut hits = Vec::new();
        let mut edges: [Option<f32>; 4] = [None; 4];
        if x0 < 0.0 {
            let hh = h - 0.0;
            edges[0] = Some((dy0 * 0.0 - hh * dx0) / (dx * hh - dy * 0.0));
        }
        if y0 < 0.0 {
            let hh = h - 0.0;
            edges[1] = Some((hh * dy0 - dx0 * 0.0) / (dx * 0.0 - hh * dy));
        }
        if w < x1 {
            let hh = h - 0.0;
            edges[2] = Some(((w - w) * dy0 - (x0 - w) * hh) / (hh * dx - (w - w) * dy));
        }
        if h < y1 {
            let ww = w - 0.0;
            edges[3] = Some(((y0 - h) * ww - (h - h) * dx0) / ((h - h) * dx - ww * dy));
        }
        for t in edges.into_iter().flatten() {
            hits.push(at(t));
            if cap <= hits.len() {
                break;
            }
        }
        hits
    }
}

/// The class of a gate, which a player's allies may pass (slot 0x678, `FUN_1411d4a30`).
pub(crate) const GATE_CLASS: i16 = 39;

/// `FUN_1410576a0`, Sedgewick's ccw: 1 when `p0`, `p1`, `p2` turn counter-clockwise (or lie in
/// a line with `p2` beyond `p1`), -1 clockwise (or `p0` between), 0 when `p2` lies on the segment.
fn ccw(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2]) -> i32 {
    let (dx1, dy1) = (p1[0] - p0[0], p1[1] - p0[1]);
    let (dx2, dy2) = (p2[0] - p0[0], p2[1] - p0[1]);
    // every test is COMISS/JA (0x1410576ee..0x14105772e): an unordered one falls through, so a
    // NaN reaches the last answer, 0
    if dy2 * dx1 > dx2 * dy1 {
        return 1;
    }
    if dx2 * dy1 > dy2 * dx1 || 0.0 > dx2 * dx1 || 0.0 > dy2 * dy1 {
        return -1;
    }
    if dy2 * dy2 + dx2 * dx2 > dy1 * dy1 + dx1 * dx1 {
        return 1;
    }
    0
}

/// The smoothing's segment test for a unit (`FUN_140f7dbb0`, [`crate::tile_path::SegmentTest`]):
/// each walk puts its temporary tile records ([`Spatial::put_tile_records`]) and is tested
/// ([`Spatial::segment_meets`]) against them and those of the pass's earlier walks; the pass's
/// end removes them ([`Spatial::clear_tile_records`]).
#[derive(Debug)]
pub struct SmoothingWalls<'a> {
    world: &'a Spatial,
    radius: f32,
    mask: u8,
    ignore: &'a [i32],
    /// Whether a walk was tested.
    pub ran: bool,
}

impl<'a> SmoothingWalls<'a> {
    /// The test of a unit `radius` wide (the manager's buffer) whose terrain mask is `mask`,
    /// with the manager's ignore list `ignore`.
    #[must_use]
    pub const fn new(world: &'a Spatial, radius: f32, mask: u8, ignore: &'a [i32]) -> Self {
        Self {
            world,
            radius,
            mask,
            ignore,
            ran: false,
        }
    }
}

impl<E> crate::tile_path::SegmentTest<E> for SmoothingWalls<'_> {
    fn blocked(&mut self, a: [f32; 2], b: [f32; 2]) -> Result<bool, E> {
        self.ran = true;
        // the pass's buffer, the unit's radius (FUN_1410562d0 with 1)
        self.world.set_manager_buffer(Some(self.radius));
        self.world.put_tile_records(a, b, self.radius, self.mask);
        Ok(self.world.segment_meets(a, b, self.radius, self.ignore))
    }

    fn pass_ends(&mut self) {
        self.world.set_manager_buffer(None);
        self.world.clear_tile_records();
    }
}

/// A straight walk's test against records (`FUN_141052350`, `FUN_141052800`): the walk's box
/// (its ends' lower and upper x and y) and the query's box, grown by the manager's buffer.
struct WalkTest {
    walk: [f32; 4],
    lo: [f32; 2],
    hi: [f32; 2],
    grow: f32,
    query: [f32; 4],
}

impl WalkTest {
    /// The walk from `a` to `b` of a unit `grow` wide; None when its grown box leaves a map of
    /// `map` tiles (the query then counts as met).
    fn new(a: [f32; 2], b: [f32; 2], grow: f32, [w, h]: [i32; 2]) -> Option<Self> {
        let lo = [
            if b[0] <= a[0] { b[0] } else { a[0] },
            if b[1] <= a[1] { b[1] } else { a[1] },
        ];
        let hi = [
            if a[0] <= b[0] { b[0] } else { a[0] },
            if a[1] <= b[1] { b[1] } else { a[1] },
        ];
        let query = [lo[0] - grow, lo[1] - grow, grow + hi[0], grow + hi[1]];
        if query[0] < 0.0 || query[1] < 0.0 || (w as f32) < query[2] || (h as f32) < query[3] {
            return None;
        }
        Some(Self {
            walk: [a[0], a[1], b[0], b[1]],
            lo,
            hi,
            grow,
            query,
        })
    }

    /// Whether a record's box `bx`, grown by the buffer, meets the walk's box (edges included)
    /// and segment (`FUN_141052e10`).
    fn meets(&self, bx: [f32; 4]) -> bool {
        let g = self.grow;
        let grown = [bx[0] - g, bx[1] - g, g + bx[2], g + bx[3]];
        self.lo[0] <= grown[2]
            && grown[0] <= self.hi[0]
            && self.lo[1] <= grown[3]
            && grown[1] <= self.hi[1]
            && box_meets_segment(bx, g, self.walk)
    }
}

/// `FUN_141054ec0`: whether segments `p1`–`p2` and `p3`–`p4` meet.
fn segments_meet(p1: [f32; 2], p2: [f32; 2], p3: [f32; 2], p4: [f32; 2]) -> bool {
    (ccw(p1, p2, p4) * ccw(p1, p2, p3)) as f32 <= 0.0
        && (ccw(p3, p4, p2) * ccw(p3, p4, p1)) as f32 <= 0.0
}

/// `FUN_141052e10` (the segment test's) and `FUN_1410512f0` (the walk test's) with the box grown
/// by `r` (the manager's `+0x57`/`+0x58`): the walk `[x0, y0, x1, y1]` crosses one of the grown
/// box's edges or starts or ends inside it, the box ordered (low and high edges) for the inside
/// tests.
fn box_meets_segment([bx0, by0, bx1, by1]: [f32; 4], r: f32, walk: [f32; 4]) -> bool {
    let lo_x = if bx1 < bx0 { bx1 } else { bx0 };
    let hi_x = if bx0 < bx1 { bx1 } else { bx0 };
    let lo_y = if by1 < by0 { by1 } else { by0 };
    let hi_y = if by0 < by1 { by1 } else { by0 };
    segment_meets_box([bx0, by0, bx1, by1], r, walk, [lo_x, lo_y, hi_x, hi_y])
}

/// A record's test against the walk by its kind (`FUN_141051e00`, `FUN_141052800`): a circle's
/// (kind 5, `FUN_141051630` and `FUN_141053130`) reads its box as it stands for the inside tests;
/// the others' (kinds 2, 3, 11, 13) order it ([`box_meets_segment`]).
fn record_meets_segment(kind: u8, b: [f32; 4], r: f32, walk: [f32; 4]) -> bool {
    if kind == 5 {
        segment_meets_box(b, r, walk, b)
    } else {
        box_meets_segment(b, r, walk)
    }
}

/// The four edges of box `[bx0, by0, bx1, by1]` grown by `r`, each tested against the walk
/// (`FUN_141052ca0`: `FUN_141054ec0`), then the inside tests of the walk's ends against `inside`
/// grown by `r`. `FUN_141052ca0` also lists each crossing (record, id, point) at the manager's
/// `+0x40` while its `+0x54` is set; `FUN_141051900` sets `+0x54` only for a list at `+0x40`
/// with a cap `+0x50` of at least 1, and nothing gives the manager one: the only stores to
/// `+0x40`, `+0x48` and `+0x50` in its code (0x14104f000..0x141060000) are its constructor's
/// and `FUN_1410560b0`'s zeros, and none of the routines that address it through
/// `DAT_143f0a088` writes them. So the listing never runs, and the edges' order (the
/// `FUN_1410512f0` tests all four, `FUN_141052e10` stops at the first crossing) does not show.
fn segment_meets_box(
    [bx0, by0, bx1, by1]: [f32; 4],
    r: f32,
    walk: [f32; 4],
    [lo_x, lo_y, hi_x, hi_y]: [f32; 4],
) -> bool {
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
    // COMISS/JC (0x141052fb8..0x141052ff9, 0x1410530dc..0x141053119; 0x1410514ad..0x141051620,
    // 0x14105189e..0x1410518f2): a NaN coordinate is out
    let outside = |p: [f32; 2]| {
        crate::fmath::below_or_unordered(p[0], lo_x - r)
            || crate::fmath::below_or_unordered(r + hi_x, p[0])
            || crate::fmath::below_or_unordered(p[1], lo_y - r)
            || crate::fmath::below_or_unordered(r + hi_y, p[1])
    };
    !(outside(start) && outside(end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_takes_its_kinds_of_players_from_the_save() {
        // player 2's +0x278 as loaded: itself 1, Gaia 0, player 1 an ally (2) whatever its
        // diplomacy byte (2 here, which no derivation knew)
        let mut s = Spatial::new([10, 10]);
        s.relations = vec![vec![1, 4, 4], vec![1, 0, 4], vec![1, 2, 0]];
        s.unit_diplomacy = vec![vec![1, 4, 4], vec![0, 1, 4], vec![0, 2, 1]];
        assert_eq!(s.query_kinds(2, 3), Ok(vec![0, 2, 1]));
        assert_eq!(s.query_kinds(2, 2), Ok(vec![0, 2]));
        assert!(s.query_kinds(3, 3).is_err());
    }

    /// `FUN_1410576a0`: counter-clockwise 1, clockwise -1, on the segment 0, beyond its end 1;
    /// a NaN falls through every COMISS/JA to 0.
    #[test]
    fn ccw_answers_as_the_game_s_compares() {
        assert_eq!(ccw([0.0, 0.0], [1.0, 0.0], [1.0, 1.0]), 1);
        assert_eq!(ccw([0.0, 0.0], [1.0, 0.0], [1.0, -1.0]), -1);
        assert_eq!(ccw([0.0, 0.0], [2.0, 0.0], [1.0, 0.0]), 0);
        assert_eq!(ccw([0.0, 0.0], [1.0, 0.0], [2.0, 0.0]), 1);
        assert_eq!(ccw([0.0, 0.0], [1.0, 0.0], [-1.0, 0.0]), -1);
        assert_eq!(ccw([0.0, 0.0], [1.0, 0.0], [f32::NAN, 1.0]), 0);
    }

    /// `FUN_141052e10`: a walk from a NaN point meets the box's first side already (ccw answers
    /// 0 for a NaN, so the segments meet); a walk inside it meets it; one past it does not.
    #[test]
    fn a_walk_meets_a_box_by_its_sides_or_from_inside() {
        let b = [0.0, 0.0, 1.0, 1.0];
        assert!(box_meets_segment(b, 0.0, [f32::NAN, 0.5, 5.0, 5.0]));
        assert!(box_meets_segment(b, 0.0, [0.5, 0.5, 0.6, 0.6]));
        assert!(!box_meets_segment(b, 0.0, [3.0, 3.0, 5.0, 5.0]));
    }

    /// `FUN_141051630` (and `FUN_141053130`): a circle record's box is read as it stands, so a
    /// box whose x1 lies below its x0 holds no walk inside it; `FUN_1410512f0` orders it first.
    #[test]
    fn a_circle_record_s_box_is_read_unordered_and_another_s_ordered() {
        let b = [2.0, 0.0, 1.0, 1.0];
        let inside = [1.4, 0.4, 1.6, 0.6];
        assert!(record_meets_segment(2, b, 0.0, inside));
        assert!(!record_meets_segment(5, b, 0.0, inside));
        assert!(record_meets_segment(5, [1.0, 0.0, 2.0, 1.0], 0.0, inside));
        // grown, the edges and the inside tests take the grow on the box as it stands: the
        // edges then run at x 1.25 and 1.75, the inside tests hold nothing
        assert!(!record_meets_segment(5, b, 0.25, inside));
        assert!(record_meets_segment(5, b, 0.25, [1.0, 0.5, 1.5, 0.5]));
        assert!(record_meets_segment(2, b, 0.25, inside));
    }

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
        s.unit_diplomacy = vec![vec![1, 4, 4], vec![0, 1, 4], vec![0, 4, 1]];
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
            memory: crate::unit_ai::Memory::default(),
        };
        let mut never = crate::unit_ai::NoAttack;
        let mut reach = |_: i32, _: f32| Ok(true);
        let still = |_: i32| false;
        let none = |_: &Placed| Ok(false);
        let no_farm = |_: i32| None;
        let mut search = |s: &Spatial, at| {
            let f = s
                .search(
                    9,
                    2,
                    &look(at),
                    &mut never,
                    &mut reach,
                    &still,
                    &none,
                    &no_farm,
                )
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
        assert_eq!(
            s.edge_records([0.0, 0.0, 120.0, 120.0], 25),
            Vec::<Hit>::new()
        );
        // the diagonal (-0.3, 50) -> (0.3, 50.6) crosses x 0 at y 50.3: the left edge's tile (0, 50)
        let left = s.edge_records([-0.3, 50.0, 0.3, 50.6], 25);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id as u32, 0xff00_0000 | 0x32);
        assert_eq!(left[0].bounds, [0.0, 50.0, 1.0, 51.0]);
        // past the bottom right corner: (119.5, 119.7) -> (120.5, 120.7) crosses x 120 at y 120.2
        // and y 120 at x 119.8
        let corner = s.edge_records([119.5, 119.7, 120.5, 120.7], 25);
        let ids: Vec<u32> = corner.iter().map(|h| h.id as u32).collect();
        assert_eq!(ids, [0xff07_8078, 0xff07_7078]);
        // a list with room for one stops after the first edge
        assert_eq!(s.edge_records([119.5, 119.7, 120.5, 120.7], 1), corner[..1]);
        // an infinite end: the crossing's terms give NaN, which CVTTSS2SI makes i32::MIN
        let far = s.edge_records([-0.5, 10.0, f32::INFINITY, 10.5], 25);
        assert_eq!(far.len(), 2);
        assert_eq!(far[0].id as u32, 0xff00_000a);
        assert_eq!(
            far[0].bounds,
            [-2_147_483_648.0, 10.0, -2_147_483_648.0, 11.0]
        );
        // a unit of radius 0.3 cannot stand 0.2 from the edge
        assert!(!s.point_clear(1, [0.2, 50.0], 0.3, 0xff).unwrap());
        assert!(s.point_clear(1, [0.4, 50.0], 0.3, 0xff).unwrap());
    }

    /// `FormationTransform`'s line query: a standing villager on the line is hit, but not when
    /// ignored or moving; a tile the mask lacks is hit once the formation put its record (its
    /// box, [`Spatial::put_tile_records`]); a line leaving the map meets the edge; the buffer
    /// the formation finds grows the records.
    #[test]
    fn a_still_line_meets_standing_records_and_the_temporary_tiles() {
        let mut classes = vec![1u8; 100];
        classes[2 * 10 + 7] = 2;
        let mut s = Spatial::with_tiles([10, 10], Tiles::new(10, 10, classes));
        s.insert(7, villager(4.0, 5.0));
        let line = |s: &Spatial, ignore: &[i32]| {
            s.still_line_hit([1.0, 5.0], [8.0, 5.0], 0.0, ignore)
                .unwrap()
        };
        assert!(line(&s, &[]));
        assert!(!line(&s, &[7]));
        s.set_gait(
            7,
            Some(Gait {
                moving: true,
                ..Gait::default()
            }),
        );
        assert!(!line(&s, &[]));
        // the class-2 tile (7, 2) once its record is put, for a mask that lacks its class
        let across = || s.still_line_hit([7.5, 1.0], [7.5, 4.0], 0.0, &[]).unwrap();
        assert!(!across());
        s.put_tile_records([6.0, 1.0], [8.0, 3.0], 0.0, 3);
        assert!(!across());
        s.put_tile_records([6.0, 1.0], [8.0, 3.0], 0.0, 1);
        assert!(across());
        s.clear_tile_records();
        assert!(!across());
        assert!(s.still_line_hit([1.0, 5.0], [10.5, 5.0], 0.0, &[]).unwrap());
        // a line 0.3 beside the standing villager (radius 0.2): met only with a buffer of 0.2
        let mut t = Spatial::with_tiles([10, 10], Tiles::new(10, 10, vec![1; 100]));
        t.insert(8, villager(4.0, 5.0));
        assert!(!t.still_line_hit([1.0, 5.3], [8.0, 5.3], 0.0, &[]).unwrap());
        assert!(t.still_line_hit([1.0, 5.3], [8.0, 5.3], 0.2, &[]).unwrap());
    }

    /// `FUN_141158940`: a villager's record holds (4, 5.5) for anyone but the ignored, and an
    /// empty list ignores the unit itself; a tile of a class outside the mask, or off the map,
    /// is never clear.
    #[test]
    fn a_point_is_clear_among_the_ignored_objects() {
        let mut classes = vec![1u8; 100];
        classes[5 * 10 + 4] = 2;
        let mut s = Spatial::with_tiles([10, 10], Tiles::new(10, 10, classes));
        s.insert(7, villager(4.0, 6.5));
        assert!(!s.point_clear_among(1, [4.0, 6.5], 0.1, 1, &[]).unwrap());
        assert!(s.point_clear_among(1, [4.0, 6.5], 0.1, 1, &[3, 7]).unwrap());
        assert!(s.point_clear_among(7, [4.0, 6.5], 0.1, 1, &[]).unwrap());
        assert!(!s.point_clear_among(7, [4.0, 6.5], 0.1, 1, &[3]).unwrap());
        // the class-2 tile (4, 5) and a point off the map
        assert!(!s.point_clear_among(7, [4.5, 5.5], 0.1, 1, &[]).unwrap());
        assert!(s.point_clear_among(7, [4.5, 5.5], 0.1, 3, &[]).unwrap());
        assert!(!s.point_clear_among(7, [-0.5, 5.5], 0.1, 3, &[]).unwrap());
        // CVTTSS2SI: a NaN coordinate is i32::MIN, off the map, not tile 0
        assert!(
            !s.point_clear_among(7, [f32::NAN, 5.5], 0.1, 1, &[])
                .unwrap()
        );
    }

    /// `FUN_1411570c0`'s query with its tile records: on a 10 by 10 map of terrain class 1 with
    /// a tile of class 2 at (5, 5), the line (2, 5.5) -> (8, 5.5) of a unit whose mask holds only
    /// class 1 meets the tile's temporary record and the villager on its way; without the mask
    /// and with the villager ignored it meets nothing; a line whose grown rectangle leaves the
    /// map meets the map edge's record first.
    #[test]
    fn a_line_meets_the_records_and_the_tiles_its_mask_lacks() {
        let mut classes = vec![1u8; 100];
        classes[5 * 10 + 5] = 2;
        let mut s = Spatial::with_tiles([10, 10], Tiles::new(10, 10, classes));
        s.insert(7, villager(4.0, 5.5));
        s.set_manager_buffer(Some(0.2));
        let hits = s
            .line_query([2.0, 5.5], [8.0, 5.5], 0.2, Some(1), &[])
            .unwrap();
        assert_eq!(hits, [tile_record_id(5, 5), 7]);
        // the routine sets the ignore list, resets the buffer and removes its records
        assert_eq!(s.manager_ignore(), Vec::<i32>::new());
        assert_eq!(s.manager_buffer(), None);
        assert_eq!(s.tile_records(), Vec::<[i32; 2]>::new());
        let none = s
            .line_query([2.0, 5.5], [8.0, 5.5], 0.2, None, &[7])
            .unwrap();
        assert_eq!(none, Vec::<i32>::new());
        assert_eq!(s.manager_ignore(), [7]);
        // a record still put counts without the routine's own; the pather's test leaves them out
        s.put_tile_records([5.5, 5.5], [5.5, 5.5], 0.0, 1);
        let kept = s
            .line_query([2.0, 5.5], [8.0, 5.5], 0.2, None, &[7])
            .unwrap();
        assert_eq!(kept, [tile_record_id(5, 5)]);
        let movers = s
            .line_hits([2.0, 5.5], [8.0, 5.5], 0.2, false, &[7])
            .unwrap();
        assert_eq!(movers, Vec::<i32>::new());
        s.clear_tile_records();
        let edge = s
            .line_query([0.1, 5.5], [3.0, 5.5], 0.2, None, &[7])
            .unwrap();
        assert_eq!(edge.len(), 1);
        assert_eq!(edge[0] as u32 & 0xff00_0000, 0xff00_0000);
    }

    /// `FUN_1410562d0`'s buffer stays with the obstruction manager until the next call: the
    /// stand tests and the walk's query set one and reset it after; a point query counts the
    /// temporary tile records still put (the group's clear spot leaves one, `FUN_141007630`).
    #[test]
    fn the_manager_keeps_its_buffer_and_point_queries_meet_tile_records() {
        let mut classes = vec![1u8; 100];
        classes[5 * 10 + 4] = 2;
        let s = Spatial::with_tiles([10, 10], Tiles::new(10, 10, classes));
        assert_eq!((s.manager_buffer(), s.manager_grow()), (None, 0.0));
        s.set_manager_buffer(Some(0.1));
        assert_eq!(s.manager_grow(), 0.1);
        assert!(s.point_clear(1, [6.5, 6.5], 0.2, 1).unwrap());
        assert_eq!(s.manager_buffer(), None);
        s.set_manager_buffer(Some(0.1));
        let _ = s
            .walk_hits(
                [2.0, 2.0],
                [3.0, 2.0],
                [2.0, 2.0, 3.0, 2.0],
                0.2,
                1.0,
                None,
                &[],
            )
            .unwrap();
        assert_eq!(s.manager_buffer(), None);
        // a point query near tile (4, 5) meets its record once it is put, within the grow
        assert!(!s.point_hit([3.95, 5.5], 0.1, &[], true).unwrap());
        s.put_tile_records([4.5, 5.5], [4.5, 5.5], 0.0, 1);
        assert!(s.point_hit([3.95, 5.5], 0.1, &[], true).unwrap());
        assert!(!s.point_hit([3.85, 5.5], 0.1, &[], true).unwrap());
        assert!(
            !s.point_hit([3.95, 5.5], 0.1, &[tile_record_id(4, 5)], true)
                .unwrap()
        );
        // a stand test counts it too
        assert!(!s.point_clear_among(1, [3.95, 5.5], 0.1, 1, &[]).unwrap());
        s.clear_tile_records();
        assert!(s.point_clear_among(1, [3.95, 5.5], 0.1, 1, &[]).unwrap());
    }

    /// The walk's query beyond the map's edge meets the edge's record first (`FUN_141057290`),
    /// then the objects as before.
    #[test]
    fn a_walk_past_the_maps_edge_meets_its_record_first() {
        let mut s = Spatial::new([10, 10]);
        s.insert(7, villager(1.0, 5.5));
        let (from, to) = ([0.1, 5.5], [2.0, 5.5]);
        let rect = [0.1, 5.5, 2.0, 5.5];
        let hits = s.walk_hits(from, to, rect, 0.2, 1.0, Some(2), &[]).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0] as u32 & 0xff00_0000, 0xff00_0000);
        assert_eq!(hits[1], 7);
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
    fn a_record_filed_in_a_cell_the_query_does_not_visit_is_not_found() {
        // records/425906646's update 553: the smoothing of 27272's plan (radius 0.25) tests
        // (32.25, 154.59375) -> (32.5, 151.5); bush 13594's box (31, 154)-(32, 155) grown by 0.25
        // touches the start, but it is filed in level 5's cell (3, 19) (tiles 24..31) and the
        // query's tiles are 32..32
        let mut s = Spatial::with_tiles([220, 220], Tiles::new(220, 220, vec![1; 220 * 220]));
        assert_eq!(s.grid, Grid { levels: 6 });
        let mut bush = villager(31.5, 154.5);
        bush.radius = [0.5, 0.5];
        bush.player = 0;
        bush.obstruction = Some(Obstruction { kind: 2, class: 1 });
        s.insert(13594, bush);
        assert_eq!(s.filed(&bush, 2), (5, 3, 19));
        let (from, to) = ([32.25, 154.59375], [32.5, 151.5]);
        assert!(!s.segment_blocked(from, to, 0.25, 0xff, &[]));
        // a start a hair to the left visits tile 31's cell: the box blocks
        assert!(s.segment_blocked([32.2, 154.59375], to, 0.25, 0xff, &[]));
        // off a half tile, the box's far edge is its tile: (31.6 ± 0.5) spans tiles 31 and 32,
        // which share only level 2's cells of 64 tiles
        bush.position = [31.6, 154.5, 0.0];
        assert_eq!(s.filed(&bush, 2), (2, 0, 2));
    }

    #[test]
    fn a_record_leaving_a_tile_another_static_record_stands_on_leaves_its_marks() {
        // records/425906646: Gaia's state-8 bush 0 shares tile (140, 3) with bush 808; its
        // removal (FUN_141055980) stops at the tile, which stays marked, and the planners hear
        // nothing
        let mut s = Spatial::with_tiles([220, 220], Tiles::new(220, 220, vec![1; 220 * 220]));
        let mut bush = villager(140.5, 3.5);
        bush.radius = [0.5, 0.5];
        bush.player = 0;
        bush.obstruction = Some(Obstruction { kind: 2, class: 1 });
        s.insert(808, bush);
        s.insert(0, bush);
        let _ = s.take_changes();
        s.remove(0);
        assert!(s.tiles().word(140, 3).is_some_and(|w| w & BLOCKED != 0));
        assert_eq!(s.take_changes(), Vec::<[i16; 4]>::new());
        // alone, it clears its tile
        s.remove(808);
        assert_eq!(s.tiles().word(140, 3), Some(1));
        assert_eq!(s.take_changes(), vec![[140, 3, 140, 3]]);
    }

    /// `FUN_141055b40` by the master's obstruction type: type 12 keeps all (false); type 1 clears
    /// its tiles and keeps its record (true); a type 2 foundation without its record still clears
    /// its player's marks (false); a unit (type 5) drops its record (true).
    #[test]
    fn a_record_goes_by_its_master_s_obstruction_type() {
        let mut s = Spatial::with_tiles([20, 20], Tiles::new(20, 20, vec![1; 400]));
        let mut rock = villager(5.5, 5.5);
        rock.radius = [0.5, 0.5];
        rock.obstruction_type = 1;
        rock.obstruction = Some(Obstruction { kind: 1, class: 1 });
        s.insert(1, rock);
        assert!(s.tiles().word(5, 5).is_some_and(|w| w & BLOCKED != 0));
        assert!(s.dropped_record(1));
        assert_eq!(s.tiles().word(5, 5), Some(1));
        assert_eq!(s.get(1).and_then(|p| p.obstruction), rock.obstruction);
        let mut other = rock;
        other.obstruction_type = 12;
        other.position = [8.5, 8.5, 0.0];
        s.insert(2, other);
        assert!(!s.dropped_record(2));
        assert!(s.tiles().word(8, 8).is_some_and(|w| w & BLOCKED != 0));
        // a foundation of player 2 without its record on a tile player 2 marked
        let mut wall = rock;
        wall.obstruction_type = 2;
        wall.obstruction = Some(Obstruction { kind: 2, class: 1 });
        wall.position = [12.5, 12.5, 0.0];
        s.insert(3, wall);
        let mut site = wall;
        site.obstruction = None;
        s.insert(4, site);
        s.objects.get_mut(&3).unwrap().obstruction = None;
        assert!(!s.dropped_record(4));
        assert_eq!(s.tiles().word(12, 12), Some(1));
        let unit = villager(15.5, 15.5);
        let mut unit5 = unit;
        unit5.obstruction_type = 5;
        s.insert(5, unit5);
        assert!(s.dropped_record(5));
        assert_eq!(s.get(5).and_then(|p| p.obstruction), None);
        assert!(!s.dropped_record(5));
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
        assert_eq!(s.tile_place(9173), 2);
        // the leaving object's slot goes to the last
        s.set_position(9171, [2.1, 1.0, 0.0]);
        assert_eq!(s.tiles().list(1, 1), [9173, 10029]);
        assert_eq!((s.tile_place(9173), s.tile_place(9171)), (0, 0));
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

    /// `FUN_140f5f0a0`: a tile of 250 objects takes no more and keeps its tall count;
    /// `FUN_140f5f560`: an object not on the list leaves nothing changed (no recount).
    #[test]
    fn a_full_tile_takes_no_object_and_one_not_on_it_leaves_its_count() {
        let mut s = Spatial::with_tiles([4, 4], Tiles::new(4, 4, vec![4; 16]));
        for id in 0..250 {
            s.insert(id, villager(1.5, 1.5));
        }
        assert_eq!(s.tiles().objects(1, 1), Some(250));
        assert_eq!(s.tiles().tall(1, 1), Some(250));
        // object 0 is no longer tall; the game does not redo the count
        s.set_tall(0, false);
        s.insert(250, villager(1.5, 1.5));
        assert_eq!(s.tiles().objects(1, 1), Some(250));
        assert!(!s.tiles().list(1, 1).contains(&250));
        assert_eq!(s.tiles().tall(1, 1), Some(250));
        // 250 leaves a tile whose list never held it: no recount
        s.remove(250);
        assert_eq!(s.tiles().tall(1, 1), Some(250));
        // one that is on it leaves: the count is redone over the list, 0 not tall
        s.remove(249);
        assert_eq!(s.tiles().tall(1, 1), Some(248));
    }

    /// A ghost enters and leaves a tile's list as any object does (`FUN_140f5f0a0`,
    /// `FUN_140f5f560`): the tile's tall count is redone over the list.
    #[test]
    fn a_ghost_entering_or_leaving_a_tile_redoes_its_tall_count() {
        let mut s = Spatial::with_tiles([4, 4], Tiles::new(4, 4, vec![4; 16]));
        s.insert(1, villager(1.5, 1.5));
        s.insert(2, villager(1.5, 1.5));
        s.set_tall(1, false);
        assert_eq!(s.tiles().tall(1, 1), Some(2));
        s.insert_ghost(-5, [1.5, 1.5, 0.0]);
        assert_eq!(s.tiles().list(1, 1), [1, 2, -5]);
        assert_eq!(s.tiles().tall(1, 1), Some(1));
        s.set_tall(2, false);
        s.remove_ghost(-5, [1.5, 1.5, 0.0]);
        assert_eq!(s.tiles().tall(1, 1), Some(0));
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
        assert_eq!(s.passable(29, 89, 6, 1, 0.2), Ok(true));
        assert_eq!(s.passable(29, 89, 1, 1, 0.2), Ok(false));
        assert_eq!(s.passable(-1, 89, 6, 1, 0.2), Ok(false));
        let mut tree = villager(29.5, 90.5);
        tree.radius = [0.5, 0.5];
        tree.obstruction = Some(Obstruction { kind: 2, class: 1 });
        s.insert(48, tree);
        assert_eq!(s.passable(29, 90, 6, 1, 0.2), Ok(false));
    }

    /// A player who lost while the game goes on sees the map revealed: its masks take the ended
    /// bits (and the unified map's every word, here without the players' sight).
    /// `FUN_1412847b0`: the mask goes into every word of the unified map; `FUN_141639860` (the
    /// end of the game for a player) also gives the player's masks the ended bits.
    #[test]
    fn revealing_the_map_sets_the_mask_in_every_word() {
        use crate::vision::{ENDED, ENDED_EXPLORED, NO_TILE, NO_TILE_EXPLORED};
        let mut s = sighted(vec![0, 1 << 1, 1 << 33]);
        s.reveal_map(NO_TILE | NO_TILE_EXPLORED);
        let words = &s.sight.as_ref().unwrap().unified;
        assert_eq!(
            *words,
            [0, 1 << 1, 1 << 33].map(|w: u64| w | NO_TILE | NO_TILE_EXPLORED)
        );
        let mut s = sighted(vec![0, 0]);
        s.reveal_to(1);
        assert!(
            s.sight
                .as_ref()
                .unwrap()
                .unified
                .iter()
                .all(|&w| w == ENDED | ENDED_EXPLORED)
        );
        // player 1 lost: its masks take the ended bits; the tile tests mask them out
        // (registry::MASK holds neither bit 30 nor 62)
        assert_eq!(s.vision[1], crate::vision::own_masks(1, true));
        assert_eq!(s.tile_sight(1, 0, 0), Some(0));
    }

    #[test]
    fn a_revealed_map_gives_the_player_the_ended_bits() {
        let mut s = Spatial {
            vision: vec![
                crate::vision::own_masks(0, false),
                crate::vision::own_masks(1, false),
            ],
            ..Spatial::default()
        };
        s.reveal_to(1);
        assert_eq!(s.vision[1], crate::vision::own_masks(1, true));
        assert_eq!(s.vision[0], crate::vision::own_masks(0, false));
    }

    /// A map of two tiles holding `words`, which players 0 and 1 see with their own masks.
    fn sighted(words: Vec<u64>) -> Spatial {
        let n = words.len() as i32;
        Spatial {
            sight: Some(Visibility::new(
                n,
                1,
                Vec::new(),
                words,
                Vec::new(),
                0,
                Vec::new(),
            )),
            vision: vec![
                crate::vision::own_masks(0, false),
                crate::vision::own_masks(1, false),
            ],
            ..Spatial::default()
        }
    }

    /// `FUN_141663500`: 0xf seen, 0x80 explored, 0 neither; a word revealed to all (bit 29 with
    /// an explored hit or bit 61) is seen, one explored for all (bit 61) explored.
    #[test]
    fn a_player_s_tile_test_answers_seen_explored_or_neither_and_reads_the_revealed_bits() {
        use crate::vision::{NO_TILE, NO_TILE_EXPLORED};
        let (seen, explored) = (1 << 1, 1 << 33);
        let s = sighted(vec![
            seen,
            explored,
            NO_TILE_EXPLORED,
            NO_TILE | NO_TILE_EXPLORED,
            NO_TILE,
            NO_TILE | explored,
        ]);
        let answers = |p: usize| (0..7).map(|x| s.tile_sight(p, x, 0)).collect::<Vec<_>>();
        let some = |v: [u8; 7]| v.map(Some).to_vec();
        assert_eq!(answers(1), some([0xf, 0x80, 0x80, 0xf, 0, 0xf, 0]));
        assert_eq!(answers(0), some([0, 0, 0x80, 0xf, 0, 0, 0]));
        assert_eq!(s.tile_sight(1, -1, 0), Some(0));
        assert_eq!(s.tile_sight(1, 0, 1), Some(0));
        assert_eq!(Spatial::default().tile_sight(1, 0, 0), None);
        // sees_tile is its 0xf, explored any answer
        assert!(s.sees_tile(0, 3, 0) && !s.sees_tile(0, 2, 0));
        assert_eq!(s.explored(0, 2, 0), Ok(true));
        assert_eq!(s.explored(0, 4, 0), Ok(false));
    }

    /// `FUN_141285350` and `FUN_140c68a00`: the word under the registry's mask only, so the
    /// revealed bits 29 and 61 count for nothing.
    #[test]
    fn the_visible_map_s_tile_test_reads_only_the_players_bits() {
        use crate::vision::{NO_TILE, NO_TILE_EXPLORED};
        let (seen, explored) = (1 << 1, 1 << 33);
        let s = sighted(vec![seen, explored, NO_TILE | NO_TILE_EXPLORED, 0]);
        let scout = (0..5).map(|x| s.scout_sight(1, x, 0)).collect::<Vec<_>>();
        assert_eq!(scout, [0xf, 0x80, 0, 0, 0].map(Some).to_vec());
        let seen_now = (0..4).map(|x| s.visibility(1, x, 0)).collect::<Vec<_>>();
        assert_eq!(seen_now, [0xf, 0x80, 0, 0].map(Some).to_vec());
        assert!(s.visible(1, 0, 0) && !s.visible(1, 1, 0));
        assert_eq!(Spatial::default().scout_sight(1, 0, 0), None);
        assert_eq!(Spatial::default().visibility(1, 0, 0), None);
    }

    #[test]
    fn the_free_spot_beside_a_footprint_is_the_closest_empty_tile_around_it() {
        // the game sends 9175 to (29.5, 89.5): (30, 89) is as close but holds 9175 itself
        let (s, house) = site();
        assert_eq!(
            s.spot_beside([30.0, 89.0, 0.0], (1, 0.2), 6, &house),
            Ok(Some([29.5, 89.5]))
        );
    }

    #[test]
    fn the_footprint_s_occupants_are_the_objects_whose_centre_is_on_it() {
        let (mut s, house) = site();
        assert_eq!(s.footprint_occupants(10028, &house, true), Ok(vec![9175]));
        s.insert(9177, villager(28.0, 89.0));
        assert_eq!(s.footprint_occupants(10028, &house, true), Ok(vec![9175]));
        assert_eq!(s.footprint_clearing(10028, &house), Ok(vec![]));
        let mut flower = villager(30.5, 87.5);
        flower.buildable_on = 1;
        s.insert(77, flower);
        assert_eq!(s.footprint_clearing(10028, &house), Ok(vec![77]));
        assert!(!s.foreign_marks(&house));
    }

    /// `FUN_1411d4b80` reads a tile's players as the low 32 bits of `word >> 23` and casts the
    /// footprint's edges by `CVTTSS2SI`: a NaN place casts to `i32::MIN`, so no tile is read.
    #[test]
    fn foreign_marks_read_the_players_in_32_bits_and_skip_a_nan_place() {
        let (mut s, mut house) = site();
        // player 2 of 30.0, 88.0: tile (29, 87) marked by player 3, then a bit past 32 players
        s.tiles.mark_for_test(29, 87, 0x80_0000 << 3);
        assert!(s.foreign_marks(&house));
        let (mut t, _) = site();
        t.tiles.mark_for_test(29, 87, 1 << 60);
        assert!(!t.foreign_marks(&house));
        // a NaN place reads nothing, not tile 0 (here marked by player 3)
        s.tiles.mark_for_test(0, 0, 0x80_0000 << 3);
        house.position = [f32::NAN, f32::NAN, 0.0];
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

    /// The old short pather's walks within one `FindShortPath` meet the tile records of its
    /// earlier walks: the walk from (6.5, 6.5) with a buffer of 0.5 puts no record on tile
    /// (5, 5), but touches the record an earlier walk put there, until the records are cleared.
    #[test]
    fn a_pather_walk_meets_the_tile_records_of_earlier_walks() {
        let mut classes = vec![1; 64 * 64];
        classes[5 * 64 + 5] = 0;
        let s = Spatial::with_tiles([64, 64], Tiles::new(64, 64, classes));
        let (a, b) = ([6.5, 6.5], [7.5, 7.5]);
        assert!(!s.pather_blocked(a, b, Some(0.5), 1, &[]).unwrap());
        s.clear_tile_records();
        assert!(
            s.pather_blocked([5.5, 4.0], [5.5, 7.0], Some(0.5), 1, &[])
                .unwrap()
        );
        assert_eq!(s.tile_records(), vec![[5, 5]]);
        let hits = s.pather_hits(a, b, Some(0.5), 1, &[]).unwrap();
        assert_eq!(
            hits,
            [Hit {
                id: tile_record_id(5, 5),
                bounds: [5.0, 5.0, 6.0, 6.0]
            }]
        );
        // the pather's ignore list holds tiles by their record's id
        assert!(
            !s.pather_blocked(a, b, Some(0.5), 1, &[tile_record_id(5, 5)])
                .unwrap()
        );
        s.clear_tile_records();
        assert!(!s.pather_blocked(a, b, Some(0.5), 1, &[]).unwrap());
    }

    /// 426327574's transport at update 309: the smoothing's walk (77.5, 90.5) -> (76.5, 91.5)
    /// puts tile (78, 90)'s record, and the next walk from (77.5, 91.5) touches its grown box.
    #[test]
    fn a_tile_record_blocks_later_walks_until_the_records_are_cleared() {
        let mut classes = vec![1; 128 * 128];
        classes[90 * 128 + 78] = 0;
        classes[87 * 128 + 78] = 0;
        let s = Spatial::with_tiles([128, 128], Tiles::new(128, 128, classes));
        let (first, second) = (([77.5, 90.5], [76.5, 91.5]), ([77.5, 91.5], [76.5, 92.5]));
        // alone, the second walk's tiles stop at y = 91 and its test removes its records
        assert!(!s.segment_blocked(second.0, second.1, 0.5, 1, &[]));
        assert_eq!(s.tile_records(), Vec::<[i32; 2]>::new());
        s.put_tile_records(first.0, first.1, 0.5, 1);
        assert_eq!(s.tile_records(), vec![[78, 90]]);
        assert!(s.segment_meets(second.0, second.1, 0.5, &[]));
        // the record's id in the ignore list leaves it out
        assert!(!s.segment_meets(second.0, second.1, 0.5, &[tile_record_id(78, 90)]));
        // a tile that holds a record takes no second
        s.put_tile_records(second.0, [79.0, 89.0], 0.5, 1);
        assert_eq!(s.tile_records(), vec![[78, 90]]);
        s.clear_tile_records();
        assert!(!s.segment_meets(second.0, second.1, 0.5, &[]));
        // tile (78, 87)'s grown box touches (77.5, 88.5), but its cell (y 80..87) is not one the
        // walk from y 88.5 visits (y from 88)
        s.put_tile_records([78.5, 86.5], [78.5, 87.5], 0.0, 1);
        assert_eq!(s.tile_records(), vec![[78, 87]]);
        assert!(!s.segment_meets([77.5, 88.5], [76.5, 89.5], 0.5, &[]));
        assert!(s.segment_meets([77.5, 88.0], [76.5, 89.5], 0.5, &[]));
        s.clear_tile_records();
        assert_eq!(s.tile_records(), Vec::<[i32; 2]>::new());
    }

    #[test]
    fn a_frozen_object_may_be_anywhere_its_speed_could_take_it() {
        let (mut s, house) = site();
        let mut deer = villager(34.0, 88.0);
        deer.speed = 1.0;
        s.insert(9656, deer);
        s.freeze(9656);
        // frozen at once, the deer is where the save has it
        assert!(s.footprint_occupants(10028, &house, true).is_ok());
        assert!(s.get_known(9656).is_ok());
        // its box reaches x 33.8; 3 s later it may stand on the footprint, which ends at 31
        s.grow_unknown(2.5);
        assert!(s.footprint_occupants(10028, &house, true).is_ok());
        s.grow_unknown(0.5);
        let e = s.footprint_occupants(10028, &house, true).unwrap_err();
        assert_eq!(e.context().next(), Some(("object", "9656")));
        let e = s.get_known(9656).unwrap_err();
        // the error names the object and its player
        let context: Vec<_> = e.context().take(2).collect();
        assert_eq!(context, [("object", "9656"), ("player", "2")]);
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

    /// `FUN_1410562e0`'s list stays with the obstruction manager: the walk's query and the stand
    /// tests set it (`FUN_14114cc60`, `FUN_141158720`, `FUN_141158940`), and it holds until the
    /// next set; the objects' part of the point query leaves the edge's records out.
    #[test]
    fn the_manager_keeps_the_last_ignore_list_set() {
        let s = Spatial::new([10, 10]);
        assert_eq!(s.manager_ignore(), Vec::<i32>::new());
        let rect = [2.0, 5.0, 3.0, 5.0];
        let _ = s
            .walk_hits([2.0, 5.0], [3.0, 5.0], rect, 0.2, 1.0, None, &[7, 8])
            .unwrap();
        assert_eq!(s.manager_ignore(), [7, 8]);
        assert!(s.point_clear(3, [4.5, 4.5], 0.1, 0xff).unwrap());
        assert_eq!(s.manager_ignore(), [3]);
        assert!(s.point_clear_among(4, [4.5, 4.5], 0.1, 0xff, &[]).unwrap());
        assert_eq!(s.manager_ignore(), [4]);
        // a point off the map's tiles sets nothing
        assert!(!s.point_clear(5, [-1.0, 4.5], 0.1, 0xff).unwrap());
        assert_eq!(s.manager_ignore(), [4]);
        s.set_manager_ignore(&[]);
        assert_eq!(s.manager_ignore(), Vec::<i32>::new());
        assert!(s.point_hit([0.05, 4.5], 0.1, &[], false).unwrap());
        assert!(!s.object_holds([0.05, 4.5], 0.1, &[], false).unwrap());
    }
}
