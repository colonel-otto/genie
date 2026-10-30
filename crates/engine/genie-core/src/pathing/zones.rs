//! The map's zones (`RGE_Zone_Map`, made by `FUN_141410680`, labelled by `FUN_141413190`): every
//! tile gets the number of the patch of tiles it belongs to, a patch being the tiles a 4-way
//! flood reaches over tiles that are, like its first, passable or not for a terrain restriction
//! (its value for the tile's terrain, map tile `+0x9`, above 0). Patches are numbered in the order
//! of their first tile, row by row; past 511 the numbers start again at 0 (`FUN_141413190`: the
//! 513th patch is 0, the next 1, ...). Each map also caches its zones' tile counts, holds their
//! number and which zones touch (`FUN_141412a80`).
//!
//! The map keeps one zone map per passability pattern (`map+0x5e28`, `FUN_1414138d0`: a map is
//! shared by the restrictions whose values begin with its own signs), made the first time a unit
//! of the pattern asks for its zone (`FUN_1411931c0`, cached on the unit at `+0x698`). The zone
//! maps are saved (`FUN_141414790`, read by `FUN_1414110f0`): a game loaded from a save holds
//! them as they were made, and a map is not made again when the terrain changes later.

use crate::GenieError;
use crate::map::Tile;
use crate::world::slots::CheckedIndex;

/// The number after the last a patch takes; the next patch takes 0 again. A tile not yet
/// reached holds it.
const PATCHES: u16 = 0x200;

/// The zones a map caches a tile count for (`+0x000`, 0x800 bytes of ints).
const COUNTED: usize = 0x200;

/// A terrain restriction the slice's data lacks, whose zone map the slice cannot make.
pub const UNKNOWN_RESTRICTION: GenieError =
    GenieError::unsupported("a restriction the slice's data has not got");

/// The map's zones of a world without its terrain.
pub const NO_TERRAIN: GenieError = GenieError::unsupported("the map's zones without its terrain");

/// A tile's zone off the map (`FUN_141413890`).
const OFF_MAP: u16 = 0;

/// The zones of one passability pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZoneMap {
    width: i32,
    height: i32,
    /// The pattern (`+0x818`, the restriction's values the map was made for): for each terrain,
    /// the value is above 0.
    pub passable: Vec<bool>,
    /// Each tile's zone (`+0x808`, row by row; `+0x810` the rows).
    zones: Vec<u16>,
    /// Each zone's tile count (`+0x000`, `numberTilesInZoneValue`), cached the first time a flood
    /// ends on the zone while its count is below 1 (`FUN_141413bb0`).
    tiles: Vec<i32>,
    /// The zones' number (`+0x838`, `mNumberZones`): the patches the last labelling made, 0x200
    /// once the numbering started again.
    count: i32,
    /// Which zones touch (`+0x840`, `count` x `count` bytes as `FUN_141412a80` built them, row by
    /// the zone of a tile, column by the zone of a tile among its eight neighbours or itself).
    touching: Vec<bool>,
    /// For two zones that do not touch, the first zone that touches both (`+0x848`, `count` x
    /// `count` shorts as `FUN_141412a80` built them). `None` where the game's table holds memory
    /// it never wrote: the allocation is twice the `count * count` bytes it clears.
    through: Vec<Option<u16>>,
}

impl ZoneMap {
    /// The zone map of the map's terrain (`kinds`, row by row) for a restriction whose value per
    /// terrain is `values` (`FUN_141410680`): its tile counts cleared, every tile allocated 0, the
    /// labelling (`FUN_141413190`) and the zones' adjacency (`FUN_141412a80`). A terrain the
    /// restriction has no value for counts as impassable.
    #[must_use]
    pub fn new(width: i32, height: i32, kinds: &[u8], values: &[f32]) -> Self {
        let n = (width.max(0) * height.max(0)) as usize;
        let mut map = Self {
            width,
            height,
            passable: values.iter().map(|&v| 0.0 < v).collect(),
            zones: vec![0; n],
            tiles: vec![0; COUNTED],
            count: 0,
            touching: Vec::new(),
            through: Vec::new(),
        };
        map.label(kinds);
        map.connect();
        map
    }

    /// A zone map as a save holds it (`FUN_141410870`): its tile counts, zones, restriction values
    /// and zone count read, then the zones' adjacency built (`FUN_141412a80`).
    #[must_use]
    pub fn from_saved(
        width: i32,
        height: i32,
        tiles: &[u32],
        zones: &[u16],
        values: &[f32],
        count: u32,
    ) -> Self {
        let mut counts = vec![0; COUNTED];
        for (c, &t) in counts.iter_mut().zip(tiles) {
            *c = t as i32;
        }
        let mut map = Self {
            width,
            height,
            passable: values.iter().map(|&v| 0.0 < v).collect(),
            zones: zones.to_vec(),
            tiles: counts,
            count: count as i32,
            touching: Vec::new(),
            through: Vec::new(),
        };
        map.connect();
        map
    }

    /// Whether the restriction is above 0 on a tile's terrain (`+0x818` indexed by map tile
    /// `+0x9`).
    fn open(&self, kinds: &[u8], i: usize) -> bool {
        kinds
            .get(i)
            .and_then(|&k| self.passable.get(usize::from(k)))
            .copied()
            .unwrap_or(false)
    }

    /// `FUN_141413190`: the zones numbered again from the terrain `kinds`. The game fills and
    /// scans with its outer loop over the width as rows and its inner loop over the height as
    /// columns, the scan guarded by row < height and column < width: on a square map that is
    /// every tile, row by row. On a map higher than it is wide, the fill (row `i` at `i * width`)
    /// misses the last tiles, which keep their value, and only the width x width square starts
    /// patches. On a wider map the game's fill reads rows past its row table; the slice fills
    /// the rows the table has. Each patch's flood (`FUN_141413340`) ends with its zone's tile
    /// count cached (`FUN_141413bb0`); the counts are not cleared, so a count cached before
    /// stays.
    fn label(&mut self, kinds: &[u8]) {
        let (w, h) = (self.width, self.height);
        for i in 0..w {
            for j in 0..h {
                if let Some(z) = self.zones.get_mut((i * w + j) as usize) {
                    *z = PATCHES;
                }
            }
        }
        let mut next: i32 = 0;
        let mut wrapped = false;
        for y in 0..w {
            for x in 0..h {
                if !(y < h && x < w) {
                    continue;
                }
                let start = (y * w + x) as usize;
                let Some(cell) = self.zones.get_mut(start) else {
                    continue;
                };
                if *cell != PATCHES {
                    continue;
                }
                if 0x1ff < next {
                    wrapped = true;
                }
                let zone = if next < i32::from(PATCHES) { next } else { 0 };
                *cell = zone as u16;
                let kind = self.open(kinds, start);
                self.flood(kinds, x, y, kind, zone as u16);
                next = zone + 1;
            }
        }
        self.count = if wrapped { i32::from(PATCHES) } else { next };
    }

    /// `FUN_141413340`: from the tile `(x, y)` the zone goes to every tile not yet reached that is,
    /// like it, passable or not (`kind`), over the left, upper, right and lower neighbours. The
    /// game keeps a FIFO queue of pool nodes (`+0x850`), the slice a stack: both reach the same
    /// 4-connected tiles. Then the zone's tile count (`FUN_141413bb0`).
    fn flood(&mut self, kinds: &[u8], x: i32, y: i32, kind: bool, zone: u16) {
        let (w, h) = (self.width, self.height);
        let mut stack = vec![(x, y)];
        while let Some((x, y)) = stack.pop() {
            for (nx, ny) in [(x - 1, y), (x, y - 1), (x + 1, y), (x, y + 1)] {
                if nx < 0 || ny < 0 || w <= nx || h <= ny {
                    continue;
                }
                let j = (ny * w + nx) as usize;
                let open = self.open(kinds, j) == kind;
                if let Some(cell) = self.zones.get_mut(j)
                    && *cell == PATCHES
                    && open
                {
                    *cell = zone;
                    stack.push((nx, ny));
                }
            }
        }
        let _ = self.count_tiles(zone);
    }

    /// `FUN_141413bb0`: a zone's tile count, counted over the whole map and cached when the cache
    /// holds less than 1.
    fn count_tiles(&mut self, zone: u16) -> i32 {
        let Some(cached) = self.tiles.get_mut(usize::from(zone)) else {
            return 0;
        };
        if *cached < 1 {
            *cached = self.zones.iter().filter(|&&z| z == zone).count() as i32;
        }
        *cached
    }

    /// `FUN_141412a80`: which zones touch, and for two that do not, a zone that touches both.
    /// For each tile of zone `z` (not above the count) and each of its eight neighbours and
    /// itself that lies on the map, `touching[z * count + zone]` is set; the neighbour's row is
    /// tested against the width (the game's quirk; square maps agree), and a row below the map
    /// reads zone 0. Then, for each pair of zones that do not touch, the first zone touching
    /// both. Writes past the tables (a zone at the count) are the game's out-of-bounds stores;
    /// the slice skips them.
    fn connect(&mut self) {
        let (w, h) = (self.width, self.height);
        let n = self.count.max(0);
        let size = (n * n) as usize;
        self.touching = vec![false; size];
        // the clear covers `size` bytes of the `2 * size`: whole entries below `size / 2`
        self.through = (0..size).map(|i| (i < size / 2).then_some(0)).collect();
        for y in 0..h {
            for x in 0..w {
                let z = self
                    .zones
                    .get((y * w + x) as usize)
                    .map_or(0, |&z| i32::from(z));
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let (nx, ny) = (x + dx, y + dy);
                        if !(0 <= nx && nx < w && 0 <= ny && ny < w && z <= n) {
                            continue;
                        }
                        let neighbour = if ny < h {
                            self.zones
                                .get((ny * w + nx) as usize)
                                .map_or(0, |&z| i32::from(z))
                        } else {
                            0
                        };
                        if let Some(t) = self.touching.get_mut((neighbour + z * n) as usize) {
                            *t = true;
                        }
                    }
                }
            }
        }
        let n = n as usize;
        let touches = |a: usize, b: usize| self.touching.get(a * n + b).copied().unwrap_or(false);
        for a in 0..n {
            for b in 0..n {
                if touches(a, b) {
                    continue;
                }
                if let Some(c) = (0..n).find(|&c| touches(a, c) && touches(b, c))
                    && let Some(slot) = self.through.get_mut(a * n + b)
                {
                    *slot = Some(c as u16);
                }
            }
        }
    }

    /// A tile's zone (`FUN_141413890`): 0 off the map.
    #[must_use]
    pub fn zone(&self, x: i32, y: i32) -> u16 {
        if x < 0 || y < 0 || self.width <= x || self.height <= y {
            return OFF_MAP;
        }
        self.zones
            .get((y * self.width + x) as usize)
            .copied()
            .unwrap_or(OFF_MAP)
    }

    /// `FUN_141414c00`: the first tile of zone `zone` within `reach` of tile `at`, column by
    /// column (x), each from its least y: the box `reach` cut to whole tiles (`CVTTSS2SI`) spans
    /// about `at`, kept on the map, and a tile counts when the root of its squared steps from
    /// `at` (an integer sum as a float, `FUN_14099ef70`) is not above `reach` (`COMISS`/`JA`: a
    /// NaN reach takes every tile of the box).
    #[must_use]
    pub fn near(&self, zone: u16, at: Tile, reach: f32) -> Option<Tile> {
        use crate::math::conversions::cvttss2si;
        let (x, y) = (at.x(), at.y());
        let r = cvttss2si(reach);
        // CMOVNS and CMOVLE on the wrapped sums
        let (x0, y0) = (x.wrapping_sub(r).max(0), y.wrapping_sub(r).max(0));
        let x1 = x.wrapping_add(r).min(self.width - 1);
        let y1 = y.wrapping_add(r).min(self.height - 1);
        for tx in x0..=x1 {
            let dx = x.wrapping_sub(tx);
            for ty in y0..=y1 {
                let dy = y.wrapping_sub(ty);
                let steps = dy.wrapping_mul(dy).wrapping_add(dx.wrapping_mul(dx));
                let d = crate::math::fmath::sqrt(steps as f32);
                if crate::math::fmath::at_most_or_unordered(d, reach) && self.zone(tx, ty) == zone {
                    return Some(Tile::new(tx, ty));
                }
            }
        }
        None
    }

    /// The zones' number (`+0x838`).
    #[must_use]
    pub const fn count(&self) -> i32 {
        self.count
    }

    /// A zone's cached tile count (`+0x000`); 0 past the cached zones.
    #[must_use]
    pub fn tiles(&self, zone: u16) -> i32 {
        self.tiles.get(usize::from(zone)).copied().unwrap_or(0)
    }

    /// Whether two zones touch (`FUN_141414f70`): no when either is above the count, else the
    /// table at `a * count + b`. `None` where that is past the table, which the game reads
    /// beyond: a zone at the count, or a count a relabel raised above the one the table was
    /// built for.
    #[must_use]
    pub fn adjacent(&self, a: u16, b: u16) -> Option<bool> {
        let (a, b) = (i32::from(a), i32::from(b));
        if self.count < a || self.count < b {
            return Some(false);
        }
        self.touching.get((self.count * a + b) as usize).copied()
    }

    /// The zone between two that do not touch (`+0x848` at `a * count + b`, no reader in build
    /// 158041 but its builder): `None` where the game's table holds memory it never wrote, and
    /// past the table.
    #[must_use]
    pub fn via(&self, a: u16, b: u16) -> Option<u16> {
        let at = self.count * i32::from(a) + i32::from(b);
        self.through.get(at as usize).copied().flatten()
    }
}

/// The map's zone maps, one per passability pattern, made when first asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Zones {
    maps: Vec<ZoneMap>,
}

impl Zones {
    /// The zone maps a save holds, in its order.
    #[must_use]
    pub const fn from_saved(maps: Vec<ZoneMap>) -> Self {
        Self { maps }
    }

    /// The zone maps, in the order they were made.
    #[must_use]
    pub fn maps(&self) -> &[ZoneMap] {
        &self.maps
    }

    /// The index of the zone map of a restriction whose value per terrain is `values`
    /// (`FUN_1414138d0`): the first map whose own values are no more than the query's and agree
    /// with the query's first ones in their signs (`0 < v`; a map of no values matches any
    /// query), labelled again from the terrain `kinds` when `relabel` is set (`FUN_141413190`,
    /// only the random map's terrain generator asks); else a new map of the terrain `kinds`,
    /// appended.
    pub fn find(
        &mut self,
        width: i32,
        height: i32,
        kinds: &[u8],
        values: &[f32],
        relabel: bool,
    ) -> usize {
        let found = self.maps.iter().position(|m| {
            m.passable.len() <= values.len()
                && m.passable.iter().zip(values).all(|(&p, &v)| p == (0.0 < v))
        });
        if let Some(i) = found {
            if relabel && let Some(map) = self.maps.get_mut(i) {
                map.label(kinds);
            }
            return i;
        }
        self.maps.push(ZoneMap::new(width, height, kinds, values));
        self.maps.len() - 1
    }

    /// The zone map at `index`.
    ///
    /// # Errors
    ///
    /// An index no zone map has.
    pub fn map(&self, index: usize) -> Result<&ZoneMap, GenieError> {
        self.maps.checked(index)
    }

    /// The zone map of a restriction whose value per terrain is `values`, as a unit asks for it
    /// (`FUN_1411931c0`, `FUN_1414138d0` without the relabel).
    ///
    /// # Errors
    ///
    /// A zone map the table lost between its search and its read.
    pub fn of(
        &mut self,
        width: i32,
        height: i32,
        kinds: &[u8],
        values: &[f32],
    ) -> Result<&ZoneMap, GenieError> {
        let i = self.find(width, height, kinds, values, false);
        self.maps.checked(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 3 x 3 map with a wall of terrain 1 (impassable) between two passable patches.
    #[rustfmt::skip]
    const WALL: [u8; 9] = [
        0, 1, 0,
        0, 1, 0,
        0, 1, 1,
    ];

    #[test]
    fn patches_are_numbered_by_their_first_tile_row_by_row() {
        let z = ZoneMap::new(3, 3, &WALL, &[1.0, 0.0]);
        assert_eq!([z.zone(0, 0), z.zone(1, 0), z.zone(2, 0)], [0, 1, 2]);
        assert_eq!([z.zone(0, 2), z.zone(1, 2), z.zone(2, 2)], [0, 1, 1]);
        assert_eq!(z.zone(2, 1), 2);
        assert_eq!(z.zone(-1, 0), OFF_MAP);
        assert_eq!(z.count(), 3);
    }

    #[test]
    fn a_zone_near_a_tile_is_searched_column_by_column_within_the_reach() {
        // a 5 x 5 map: terrain 1 (impassable, zone 1) in column 4 and at (3, 0); the rest zone 0
        #[rustfmt::skip]
        let kinds = [
            0, 0, 0, 1, 1,
            0, 0, 0, 0, 1,
            0, 0, 0, 0, 1,
            0, 0, 0, 0, 1,
            0, 0, 0, 0, 1,
        ];
        let z = ZoneMap::new(5, 5, &kinds, &[1.0, 0.0]);
        assert_eq!([z.zone(3, 0), z.zone(4, 2), z.zone(2, 2)], [1, 1, 0]);
        // from (2, 2) within 2: column 3 comes before column 4, and (3, 0) is sqrt 5 away;
        // column 4's (4, 2) is 2 away
        assert_eq!(z.near(1, Tile::new(2, 2), 2.0), Some(Tile::new(4, 2)));
        // within 2.5 (cut to 2 tiles) (3, 0) counts and its column comes first
        assert_eq!(z.near(1, Tile::new(2, 2), 2.5), Some(Tile::new(3, 0)));
        // the box stays on the map: from (0, 0) within 1 no zone-1 tile
        assert_eq!(z.near(1, Tile::new(0, 0), 1.0), None);
        // the unit's own tile counts, and x before y: (0, 0) itself
        assert_eq!(z.near(0, Tile::new(0, 0), 1.0), Some(Tile::new(0, 0)));
        // a NaN reach is 0 tiles (CVTTSS2SI's i32::MIN wraps the box away) and takes no distance
        // test
        assert_eq!(z.near(0, Tile::new(2, 2), f32::NAN), None);
    }

    #[test]
    fn the_flood_takes_four_neighbours_only() {
        // tiles touching at a corner only are patches of their own, passable or not
        let kinds = [0, 1, 1, 0];
        let z = ZoneMap::new(2, 2, &kinds, &[1.0, 0.0]);
        assert_eq!([z.zone(0, 0), z.zone(1, 0)], [0, 1]);
        assert_eq!([z.zone(0, 1), z.zone(1, 1)], [2, 3]);
    }

    /// A square map of `side` x `side` tiles alternating passable and not: every tile is a patch.
    fn checkerboard(side: i32) -> Vec<u8> {
        (0..side * side)
            .map(|i| ((i % side + i / side) % 2) as u8)
            .collect()
    }

    #[test]
    fn numbers_start_again_after_511_and_the_count_stays_512() {
        // 23 x 23 = 529 patches: the 513th (tile 512, at 6, 22) is 0 again, the next 1
        let z = ZoneMap::new(23, 23, &checkerboard(23), &[1.0, 0.0]);
        assert_eq!([z.zone(5, 22), z.zone(6, 22), z.zone(7, 22)], [511, 0, 1]);
        assert_eq!(z.count(), 512);
    }

    #[test]
    fn each_flood_caches_its_zone_s_tile_count_once() {
        let z = ZoneMap::new(3, 3, &WALL, &[1.0, 0.0]);
        assert_eq!(z.tiles(0), 3);
        assert_eq!(z.tiles(1), 4);
        assert_eq!(z.tiles(2), 2);
        assert_eq!(z.tiles(3), 0);
        // past 511 a zone's second patch finds its count cached by the first: zone 0's tiles
        // are two (tiles 0 and 512), its count stays the first patch's 1
        let z = ZoneMap::new(23, 23, &checkerboard(23), &[1.0, 0.0]);
        assert_eq!(z.tiles(0), 1);
    }

    #[test]
    fn a_map_wider_than_it_is_high_labels_the_square_of_its_height() {
        // 2 wide, 3 high: the game fills and scans rows over the width and columns over the
        // height, so tile (1, 2) keeps the 0 it was allocated with, and (0, 2), impassable beside
        // the passable patch above, is filled but never starts a patch: it keeps 0x200
        let kinds = [0, 0, 0, 0, 1, 0];
        let z = ZoneMap::new(2, 3, &kinds, &[1.0, 0.0]);
        assert_eq!(
            [z.zone(0, 0), z.zone(1, 0), z.zone(0, 1), z.zone(1, 1)],
            [0; 4]
        );
        assert_eq!([z.zone(0, 2), z.zone(1, 2)], [PATCHES, 0]);
        assert_eq!(z.count(), 1);
        // the game's 3 wide, 2 high case writes past its row table; the slice labels the rest
        let z = ZoneMap::new(3, 2, &[0; 6], &[1.0]);
        assert_eq!(z.zone(0, 0), 0);
    }

    #[test]
    fn zones_touching_at_a_tile_or_a_corner_are_adjacent() {
        let z = ZoneMap::new(3, 3, &WALL, &[1.0, 0.0]);
        assert_eq!(z.adjacent(0, 1), Some(true));
        assert_eq!(z.adjacent(1, 2), Some(true));
        assert_eq!(z.adjacent(0, 0), Some(true));
        assert_eq!(z.adjacent(0, 2), Some(false));
        assert_eq!(z.adjacent(2, 0), Some(false));
        // FUN_141414f70: past the count it answers no; at the count it reads past the table
        assert_eq!(z.adjacent(4, 0), Some(false));
        assert_eq!(z.adjacent(3, 0), None);
        let z = ZoneMap::new(2, 2, &[0, 1, 1, 0], &[1.0, 0.0]);
        assert_eq!(z.adjacent(0, 3), Some(true));
        assert_eq!(z.adjacent(1, 2), Some(true));
    }

    #[test]
    fn two_zones_apart_go_through_the_first_zone_touching_both() {
        let z = ZoneMap::new(3, 3, &WALL, &[1.0, 0.0]);
        assert_eq!(z.via(0, 2), Some(1));
        assert_eq!(z.via(2, 0), Some(1));
        // the table's first n * n bytes start 0 (entries 0 to 3 and the low byte of 4), the
        // rest is not initialised: adjacent pairs keep that
        assert_eq!(z.via(0, 0), Some(0));
        assert_eq!(z.via(1, 1), None);
        assert_eq!(z.via(2, 2), None);
    }

    #[test]
    fn a_saved_map_keeps_its_counts_and_builds_its_adjacency() {
        let made = ZoneMap::new(3, 3, &WALL, &[1.0, 0.0]);
        let mut tiles = vec![0u32; 512];
        tiles[..3].copy_from_slice(&[3, 4, 2]);
        let zones: Vec<u16> = (0..9).map(|i| made.zone(i % 3, i / 3)).collect();
        let saved = ZoneMap::from_saved(3, 3, &tiles, &zones, &[1.0, 0.0], 3);
        assert_eq!(saved, made);
    }

    #[test]
    fn restrictions_of_one_pattern_share_their_map() {
        let mut zones = Zones::default();
        let kinds = [0, 1];
        let _ = zones.of(2, 1, &kinds, &[1.0, 0.0]).unwrap();
        let _ = zones.of(2, 1, &kinds, &[0.5, -1.0]).unwrap();
        let _ = zones.of(2, 1, &kinds, &[0.0, 1.0]).unwrap();
        assert_eq!(zones.maps.len(), 2);
    }

    #[test]
    fn a_map_matches_a_query_whose_values_begin_with_its_pattern() {
        let mut zones = Zones::default();
        let kinds = [0, 1, 1, 0];
        assert_eq!(zones.find(2, 2, &kinds, &[1.0, 0.0], false), 0);
        // the map's two values are a prefix of the query's three
        assert_eq!(zones.find(2, 2, &kinds, &[1.0, 0.0, 1.0], false), 0);
        // a query shorter than the map's values does not match it
        assert_eq!(zones.find(2, 2, &kinds, &[1.0], false), 1);
        // a map of no values matches every query
        let mut empty = Zones::default();
        assert_eq!(empty.find(2, 2, &kinds, &[], false), 0);
        assert_eq!(empty.find(2, 2, &kinds, &[0.0, 1.0], false), 0);
    }

    #[test]
    fn a_relabel_numbers_the_terrain_again_and_keeps_the_rest() {
        let mut zones = Zones::default();
        let i = zones.find(3, 3, &WALL, &[1.0, 0.0], false);
        let before = zones.map(i).unwrap().clone();
        // the wall is gone: one patch, the count 1; the tile counts and the adjacency stay
        let i = zones.find(3, 3, &[0; 9], &[1.0, 0.0], true);
        let after = zones.map(i).unwrap();
        assert_eq!(after.zone(2, 2), 0);
        assert_eq!(after.count(), 1);
        assert_eq!(after.tiles(1), before.tiles(1));
        assert_eq!(after.adjacent(0, 0), Some(true));
        // the 3 x 3 table stays: at the new count 1, (0, 1) reads its entry 1, (0, 1) of before
        assert_eq!(after.adjacent(0, 1), Some(true));
        // without the relabel flag the map is not numbered again
        let i = zones.find(3, 3, &WALL, &[1.0, 0.0], false);
        assert_eq!(zones.map(i).unwrap().zone(1, 0), 0);
    }
}
