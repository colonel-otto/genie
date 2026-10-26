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

use crate::GenieError;
use crate::vision::registry::Record;
use crate::vision::visibility::Visibility;
use std::collections::BTreeMap;

mod footprint;
mod grid;
mod objects;
mod pather;
mod search;
mod sight;

pub(crate) use self::pather::GATE_CLASS;
pub use self::pather::{Hit, SmoothingWalls, tile_record_id};
use self::pather::{WalkTest, record_meets_segment};
#[cfg(test)]
use self::pather::{box_meets_segment, ccw};

/// The range of `ClosestObject`'s reachability check (`FUN_140c488b0`, `DAT_142eb5690`).
const CLOSEST_RANGE: f32 = crate::math::image::spatial::CLOSEST_RANGE;

/// How near (squared) a unit AI's search comes to the last one's place to skip its query
/// (`FUN_140c63e20`).
const SEARCH_REUSE_SQUARED: f32 = crate::math::image::spatial::SEARCH_REUSE_SQUARED;

/// The pasture and livestock classes `ClosestObject` treats apart.
const PASTURE_CLASS: i32 = 49;
const LIVESTOCK_CLASS: i32 = 58;

/// The caller's reachability check of a found object (slot 0x3d0 within a range).
pub type Reach<'a> = dyn FnMut(i32, f32) -> Result<bool, GenieError> + 'a;

/// `bestUnitToAttack`'s world ([`crate::unit_ai::best_target::Battlefield`]): the unit AI's picker for its
/// reads and tests, the search's reachability for its paths (slot 0x3d0).
struct AttackField<'p, 'r> {
    picker: &'p mut dyn crate::unit_ai::AttackPicker,
    reach: &'p mut Reach<'r>,
}

impl crate::unit_ai::best_target::Battlefield for AttackField<'_, '_> {
    fn thing(&mut self, id: i32) -> Result<Option<crate::unit_ai::best_target::Thing>, GenieError> {
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
    ) -> Result<crate::unit_ai::best_target::Candidate, GenieError> {
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

/// The obstruction manager's grid as a lookup (FD-030-D03): the cell each object's record is
/// filed in ([`Spatial::filed`], as (level, x, y)) and the ids filed in each cell, so a query
/// reads the cells it visits ([`Spatial::visited_records`]) instead of testing every object.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RecordCells {
    cells: std::collections::HashMap<(i32, i32, i32), std::collections::BTreeSet<i32>>,
    filed: std::collections::HashMap<i32, (i32, i32, i32)>,
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
    /// The cell of each object's record ([`RecordCells`]), kept with `objects`
    /// ([`Spatial::refile`]).
    records: RecordCells,
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
    scout_maps: BTreeMap<usize, crate::movement::explore::InfluenceMap>,
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
    pub terrain: Option<crate::map::terrain::Terrain>,
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

#[cfg(test)]
mod tests;
