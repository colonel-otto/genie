//! What each player sees: the players' `RGE_Visible_Map`s and the map's `RGE_Unified_Visible_Map`
//! (both saved), which the objects' lines of sight keep.
//!
//! A player's map counts, per tile, the player's objects whose line of sight covers the tile: -1
//! for a tile the player never saw, 0 for one it saw before, more for one it sees now. The unified
//! map holds, per tile, bit `p` while player `p` sees the tile and bit `32 + p` once it saw it. An
//! object's sight comes and goes with its tile (`RGE_Static_Object::new_position`, `0x1411a58b0`),
//! its master (`FUN_1411a6a70`), its death (`FUN_14119d710`) and a building's construction
//! (`FUN_1411e5670`, `FUN_1411dbb70`).
//!
//! The map's tiles also link a building's tiles to its anchor tile (the map tile's `+0x19` and
//! `+0x1a`, `FUN_140f73900`), and the map keeps a list of the buildings' extents (the saved UMV
//! data list, map `+0x5e00`): a player whose sight first reaches a tile of a building whose anchor
//! it never saw sees the building's tiles as explored (`FUN_141285da0`).
//!
//! Each player's tile list (player `+0x180`) holds the tiles its map explored since its update
//! last ended, whose objects that gather by themselves join its visible-resource lists
//! ([`VisibleResources`], player `+0x2f0`).
//!
//! Not held: the tile's fog byte (`+0x17`, for the local player's view), the minimap's and the
//! renderer's dirty tiles (`FUN_141285c70`'s `+0x1f0`, `FUN_141778750`), the
//! cheat that reveals the map (`DAT_143f68a30`, probed 0), shared sight (`+0x798`, `+0x79c`, 0 in
//! every object probed) and the line-of-sight powerup (`+0xad4`, unsaved, 0 after a load).

use crate::GenieError;
use crate::los_tables::{CENTRED, OFFSET};
use crate::registry::{MASK, Registry};
use std::collections::BTreeMap;

/// The map tile's `+0x19` bit of a building's tile that is not its anchor (`FUN_140f73900`).
const LINKED: u8 = 0x20;
/// The map tile's `+0x19` bit of a building's anchor tile, whose `+0x1a` is the building's entry
/// in the UMV list, from 1.
const ANCHOR: u8 = 0x40;

/// `DAT_1439e6800`: a direction nibble's step back to the anchor.
const STEP: [i32; 16] = [0, 1, 2, 3, 4, 5, 6, 7, -8, -7, -6, -5, -4, -3, -2, -1];

/// The cap of a tile's count (`explore_terrain`).
const MOST: i16 = 0x7ffe;

/// The master whose footprint sight is a circle (`FUN_141192560`: id 112).
const CIRCLE_MASTER: i16 = 112;

/// The master whose sight the AI players' market lists follow (`FUN_1411d0190`: id 84).
const MARKET: i16 = 84;

/// `DAT_142eb99cc`: a footprint's high edge, less this, is its last tile.
const EDGE: f32 = crate::image::visibility::EDGE;

/// One player's `RGE_Visible_Map` (player `+0x70`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleMap {
    /// Per tile, row by row (`+0x18`).
    pub counts: Vec<i16>,
    /// `numberTilesExploredValue` (`+0x28`).
    pub explored: i32,
}

/// A building's sight state (`TRIBE_Building_Object`: its slots 0x1d0 `FUN_1411d0190` and 0x1d8
/// `FUN_1411e6f60`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BuildingSight {
    /// `built` (`+0xca1`): the construction completed, and the sight is the line of sight.
    pub built: bool,
    /// `build_pts` (`+0xc50`) is not 0: a foundation's construction started, and its sight is its
    /// footprint.
    pub started: bool,
}

/// What an object's sight reads of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seer {
    /// The object's player.
    pub player: usize,
    /// The master's id (`+0x58`, decoded).
    pub master: i16,
    /// The master's `line_of_sight` (`+0x5d0`); slot 0x210 adds the object's powerup (`+0xad4`).
    pub los: f32,
    /// `object_state` (`+0x640`): only state 2 sees from a new tile.
    pub state: u8,
    /// `+0x684`: the object explored since it was made (unsaved; a load sets it, `FUN_14116a500`,
    /// and a foundation made in the game has it clear until its construction starts). Only a
    /// building's slot 0x1d8 reads it.
    pub explored: bool,
    /// A building's sight, which its construction changes.
    pub building: Option<BuildingSight>,
    /// A building's UMV entry (`+0x904`, from 1; 0 for none): the extra tiles its mask reads.
    pub umv: u8,
    /// `+0x924` (`FUN_14121df60`): 1 for an object another player takes over by seeing it, 3
    /// for livestock.
    pub capture: u8,
    /// The object's type (`+0xf`): 70 a combat unit, 80 a building, below 70 anything else.
    pub kind: u8,
}

/// An entry of the map's UMV data list (map `+0x5e08`, `+0x5e10`): a building's extent around
/// its anchor, packed as bytes (anchor less low x, anchor less low y, high x less anchor, high y
/// less anchor), and the offsets of the tiles the visible-object updator checks for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extent {
    /// The guid of its entry.
    pub guid: u32,
    /// The offsets of the tiles the visible-object updater checks for it.
    pub offsets: Vec<[i32; 2]>,
    /// The offsets' capacity (the entry's second word).
    pub capacity: i32,
}

/// A record of a player's visible-resource lists (`VISIBLE_RESOURCE_REC`, 32 bytes, saved).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VisibleResource {
    /// The object (`+0xc`).
    pub id: i32,
    /// Its distance from the point of the last search that chose it (`+0x10`,
    /// [`VisibleResources::closest`]); 0 when added.
    pub distance: f32,
    /// `+0x14`, HD's `zone`: 0 when added.
    pub zone: u8,
    /// The object's x when added (`+0x18`).
    pub x: f32,
    /// Its y when added (`+0x1c`).
    pub y: f32,
}

/// A player's `Visible_Resource_Manager` (player `+0x2f0`, saved): per resource group (a master's
/// `resource_gather_group`, `+0x683`) the objects of masters that gather by themselves
/// (`enable_auto_gather`, `+0x681`) the player found on the tiles it explored.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VisibleResources {
    /// `VR_ListSize`: each list's capacity, which a full list's next record doubles.
    pub capacities: Vec<i32>,
    /// The lists (`num_visible_resource_lists` of them), `VR_ListUsed` records each.
    pub lists: Vec<Vec<VisibleResource>>,
}

impl VisibleResources {
    /// `AddResource` (`FUN_141288830`) of the object `id` at (`x`, `y`): a manager with lists
    /// puts its record last in list `group`, doubling a full list's capacity. The masters' groups
    /// are 0 to 4 and every manager holds 5 lists; the game writes past its lists for a group out
    /// of range.
    pub fn add(&mut self, id: i32, group: u8, x: f32, y: f32) {
        let g = usize::from(group);
        let (Some(list), Some(capacity)) = (self.lists.get_mut(g), self.capacities.get_mut(g))
        else {
            return;
        };
        if usize::try_from(*capacity).is_ok_and(|c| c == list.len()) {
            *capacity *= 2;
        }
        list.push(VisibleResource {
            id,
            distance: 0.0,
            zone: 0,
            x,
            y,
        });
    }

    /// `Remove_Resource` (`FUN_141289b80`): the first record of the object `id` in list `group`
    /// takes the last record and the list shortens. Its index; 0 for an empty list, -1 when the
    /// object is not in it or the manager has no lists.
    pub fn remove(&mut self, id: i32, group: u8) -> i32 {
        let Some(list) = self.lists.get_mut(usize::from(group)) else {
            return -1;
        };
        if list.is_empty() {
            return 0;
        }
        let Some(i) = list.iter().position(|r| r.id == id) else {
            return -1;
        };
        let _ = list.swap_remove(i);
        i as i32
    }

    /// `Get_Closest_Resource` (`FUN_141289000`): the record of list `group` nearest (`x`, `y`)
    /// (squared, the first of equals) whose object is not in `skip`; its distance becomes the
    /// root. None for a group out of range or an empty list.
    pub fn closest(&mut self, x: f32, y: f32, group: i32, skip: &[i32]) -> Option<usize> {
        let list = usize::try_from(group)
            .ok()
            .and_then(|g| self.lists.get_mut(g))?;
        // COMISS r, p; JBE: a record at or below the point (or NaN) measures p - r
        let gap = |r: f32, p: f32| {
            if crate::fmath::at_most_or_unordered(r, p) {
                p - r
            } else {
                r - p
            }
        };
        let mut best = None;
        let mut nearest = f32::MAX;
        for (i, r) in list.iter().enumerate() {
            let (dx, dy) = (gap(r.x, x), gap(r.y, y));
            let d = dy * dy + dx * dx;
            if d < nearest && !skip.contains(&r.id) {
                best = Some(i);
                nearest = d;
            }
        }
        if let Some(i) = best {
            list[i].distance = crate::fmath::sqrt(nearest);
        }
        best
    }
}

/// The players' visible maps, the unified map, the map tiles' building links and the objects'
/// sight.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Visibility {
    /// The map's width in tiles.
    pub width: i32,
    /// The map's height in tiles.
    pub height: i32,
    /// By player index (Gaia 0).
    pub maps: Vec<VisibleMap>,
    /// Map `+0x5e30`, row by row.
    pub unified: Vec<u64>,
    /// Per tile, the `+0x19` link bits and the `+0x1a` byte (a direction to the anchor, or the
    /// anchor's UMV entry).
    links: Vec<(u8, u8)>,
    /// The UMV list: the entries of the buildings' extents.
    pub extents: Vec<Extent>,
    /// The UMV list's capacity (`+0x5e04`): it grows by 8.
    pub extents_capacity: i32,
    /// Players whose type is 3: a computer player's map tells its AI of a newly explored tile
    /// (its slot 0x88), which the slice does not model.
    pub computer: Vec<bool>,
    seers: BTreeMap<i32, Seer>,
    /// The objects-in-sight registry, when the slice holds it (the save does not).
    pub registry: Option<Registry>,
    /// Each player's tile list (`RGE_Tile_List`, player `+0x180`, not saved): the tiles its map
    /// explored (`FUN_14165d500`) since its last update ended, in order, which the update's
    /// `Process_New_Tiles` reads; its count of new tiles (`+8`) only a computer player's AI
    /// reads.
    pub new_tiles: Vec<Vec<[i32; 2]>>,
    /// Each player's visible-resource lists.
    pub resources: Vec<VisibleResources>,
    /// The first part of the game the slice met and does not model, for its caller.
    unsupported: Option<GenieError>,
}

/// `+0x924` as a combat object's setup sets it (`FUN_14121df60`): 1 for Gaia's objects other
/// than its animals, buildings and a few classes (those of 40, 42, 58, 14, 11, 1, 60, 38, 41, 9
/// and 10, and master 826), and for masters 854 and Gaia's 814, which another player takes over
/// by seeing them; 3 for classes 58 and 1 (livestock).
#[must_use]
pub fn capture_kind(player: usize, class: i16, master: i16) -> u8 {
    const KEPT: [i16; 11] = [40, 42, 58, 14, 11, 1, 60, 38, 41, 9, 10];
    let mut kind = 0;
    if (player == 0 && !KEPT.contains(&class) && master != 826) || master == 854 {
        kind = 1;
    }
    if player == 0 && master == 814 {
        kind = 1;
    }
    if class == 58 || class == 1 {
        kind = 3;
    }
    kind
}

/// `RGE_Combat_Object::update`'s takeover by sight (`0x14107dfb0`, once the registry took a
/// changed `mask` of an object with `+0x924` 1, [`Visibility::combat_sight`]): the players from
/// 1 on, in order, each with its computer flag (`player+0x10 == 3`) and its
/// `mAllowCapturingGaiaUnits` (`+0x670`). Outside the game modes whose descriptor's `+0x9c` is 1
/// (`standard`, `FUN_141302450`) a computer player that does not allow it is passed over; the
/// test `DAT_143e76fc3` guards before (`FUN_140ca6be0` on the AI's table) is never run, the
/// flag being 0 in the image and never written. The first other player than `owner` whose bit
/// is in `mask` ends the loop: it is the answer, which takes the object when it has its master.
#[must_use]
pub fn sight_taker(
    mask: u64,
    owner: usize,
    standard: bool,
    players: &[(bool, bool)],
) -> Option<usize> {
    players
        .iter()
        .enumerate()
        .skip(1)
        .filter(|&(_, &(computer, allows))| standard || !computer || allows)
        .map(|(q, _)| q)
        .find(|&q| q != owner && mask & (1u64 << (q & 63)) != 0)
}

/// The tile of a position as `new_position` files the object (`0x1411a58b0`): the position held
/// on the map (`DAT_142eb1e9c` below its size), the tile held on the map.
fn tile_of(v: f32, size: i32) -> i32 {
    let v = if v < 0.0 { 0.0 } else { v };
    let v = if size as f32 <= v {
        size as f32 - 0.001
    } else {
        v
    };
    (v as i32).clamp(0, size - 1)
}

impl Visibility {
    /// The saved maps of a `width` × `height` map, with the saved UMV list.
    #[must_use]
    pub fn new(
        width: i32,
        height: i32,
        maps: Vec<VisibleMap>,
        unified: Vec<u64>,
        extents: Vec<Extent>,
        extents_capacity: i32,
        computer: Vec<bool>,
    ) -> Self {
        let n = (width.max(0) * height.max(0)) as usize;
        let players = maps.len();
        Self {
            width,
            height,
            maps,
            unified,
            links: vec![(0, 0); n],
            extents,
            extents_capacity,
            computer,
            seers: BTreeMap::new(),
            registry: None,
            new_tiles: vec![Vec::new(); players],
            resources: vec![VisibleResources::default(); players],
            unsupported: None,
        }
    }

    const fn index(&self, x: i32, y: i32) -> usize {
        (y * self.width + x) as usize
    }

    /// The first unmodelled case met since the last call.
    pub const fn take_unsupported(&mut self) -> Option<GenieError> {
        self.unsupported.take()
    }

    fn unsupported(&mut self, what: &'static str) {
        let _ = self
            .unsupported
            .get_or_insert_with(|| GenieError::unsupported(what));
    }

    /// `RGE_Tile_List::add_node` (`FUN_14165d500`) on player `player`'s tile list.
    fn new_tile(&mut self, player: usize, x: i32, y: i32) {
        if let Some(list) = self.new_tiles.get_mut(player) {
            list.push([x, y]);
        }
    }

    /// `RGE_Tile_List::del_list` (`FUN_141661030`) on player `player`'s tile list, after the
    /// player's update (`BaseWorld::update`); the list is never frozen in a game (`+0x10`).
    pub fn clear_new_tiles(&mut self, player: usize) {
        if let Some(list) = self.new_tiles.get_mut(player) {
            list.clear();
        }
    }

    /// `FUN_1411980b0`'s visible resources, for an object of a master that gathers by itself
    /// (`+0x681`) of group `group` at `position`: from player 0 up while players remain to be
    /// found in the explored bits of the unified map at its tile (bits 32 to 56), each player
    /// there (`FUN_14152b700`: one the world has) loses the object from its list
    /// ([`VisibleResources::remove`]).
    pub fn forget_resource(&mut self, id: i32, position: [f32; 3], group: u8) {
        let (x, y) = (position[0] as i32, position[1] as i32);
        if x < 0 || y < 0 || self.width <= x || self.height <= y {
            return;
        }
        let mut explored = (self.unified[self.index(x, y)] & MASK) >> 32;
        let mut p = 0;
        while explored != 0 {
            if explored & 1 != 0 {
                let Some(r) = self.resources.get_mut(p) else {
                    break;
                };
                let _ = r.remove(id, group);
            }
            p += 1;
            explored >>= 1;
        }
    }

    /// The object `id` comes into the players' sight with `seer`.
    pub fn add_seer(&mut self, id: i32, seer: Seer) {
        let _ = self.seers.insert(id, seer);
    }

    /// The object `id` leaves the sight.
    pub fn remove_seer(&mut self, id: i32) {
        let _ = self.seers.remove(&id);
    }

    /// What the object `id`'s sight reads of it.
    #[must_use]
    pub fn seer(&self, id: i32) -> Option<&Seer> {
        self.seers.get(&id)
    }

    /// The same, to change.
    pub fn seer_mut(&mut self, id: i32) -> Option<&mut Seer> {
        self.seers.get_mut(&id)
    }

    /// The link bits and byte of tile (`x`, `y`).
    #[must_use]
    pub fn link(&self, x: i32, y: i32) -> (u8, u8) {
        self.links[self.index(x, y)]
    }

    // The visible map's routines (`RGE_Visible_Map`).

    /// `explore_terrain` (`FUN_141284a40`): the tiles of a circle of `los` (at most 20) around
    /// tile (`x`, `y`), from the table [`OFFSET`] for a building whose centre is on a tile corner,
    /// count one more seer. A tile seen for the first time counts as explored, and its first sight
    /// of a building's tile whose anchor it never saw explores the building's tiles. Returns the
    /// tiles explored.
    pub fn explore(&mut self, player: usize, x: i32, y: i32, los: i32, offset: bool) -> i32 {
        let table = if offset { &OFFSET } else { &CENTRED }[los.clamp(0, 20) as usize];
        let (visible, seen) = (1u64 << player, 1u64 << (32 + player));
        let mut newly = 0;
        for &(dy, dx0, dx1) in table {
            let ty = y + i32::from(dy);
            if ty < 0 || self.height <= ty {
                continue;
            }
            let x0 = (x + i32::from(dx0)).max(0);
            let x1 = (x + i32::from(dx1)).min(self.width - 1);
            for tx in x0..=x1 {
                let i = self.index(tx, ty);
                let mut c = self.maps[player].counts[i].wrapping_add(1);
                if c == 0 {
                    newly += 1;
                    self.maps[player].explored += 1;
                    self.new_tile(player, tx, ty);
                    if self.computer.get(player).copied().unwrap_or(false) {
                        self.unsupported("a computer player's newly explored tile (its slot 0x88)");
                    }
                    if self.links[i].0 & (LINKED | ANCHOR) != 0 {
                        self.reveal_building(player, tx, ty);
                    }
                    c = 1;
                    self.unified[i] |= seen;
                }
                if c == 1 {
                    self.unified[i] |= visible;
                }
                self.maps[player].counts[i] = c.min(MOST);
            }
        }
        newly
    }

    /// `explore_terrain`'s case of a building's tile (`FUN_140f68f50`, `FUN_140f68e30`): when the
    /// building's anchor was never explored, the tile counts one seer and the building's tiles
    /// count as explored.
    fn reveal_building(&mut self, player: usize, x: i32, y: i32) {
        let Some([ax, ay]) = self.anchor_of(x, y) else {
            return;
        };
        if self.maps[player].counts[self.index(ax, ay)] != -1 {
            return;
        }
        let Some([x0, y0, x1, y1]) = self.extent_of(x, y) else {
            return;
        };
        let i = self.index(x, y);
        self.maps[player].counts[i] = 1;
        let _ = self.reveal_rect(player, x0, y0, x1, y1);
    }

    /// `unexplore_terrain` (`FUN_141287170`): the circle's tiles count one seer less (a tile at 0
    /// stays 0); a tile left without seers is no longer visible.
    pub fn unexplore(&mut self, player: usize, x: i32, y: i32, los: i32, offset: bool) {
        let table = if offset { &OFFSET } else { &CENTRED }[los.clamp(0, 20) as usize];
        let hidden = !(1u64 << player);
        for &(dy, dx0, dx1) in table {
            let ty = y + i32::from(dy);
            if ty < 0 || self.height <= ty {
                continue;
            }
            let x0 = (x + i32::from(dx0)).max(0);
            let x1 = (x + i32::from(dx1)).min(self.width - 1);
            for tx in x0..=x1 {
                let i = self.index(tx, ty);
                let c = self.maps[player].counts[i];
                let c = if c != 0 { c.wrapping_sub(1) } else { 0 };
                if c == 0 {
                    self.unified[i] &= hidden;
                }
                self.maps[player].counts[i] = c;
            }
        }
    }

    /// The tile range of a rectangle held on the map, None when it misses the map.
    fn held(&self, x0: i32, y0: i32, x1: i32, y1: i32) -> Option<(i32, i32, i32, i32)> {
        (x0 < self.width && 0 <= x1 && y0 < self.height && 0 <= y1).then(|| {
            (
                x0.max(0),
                y0.max(0),
                x1.min(self.width - 1),
                y1.min(self.height - 1),
            )
        })
    }

    /// `explore_terrain_sq` (`FUN_141284f10`): `explore_terrain` over a rectangle, without the
    /// buildings' case.
    pub fn explore_rect(&mut self, player: usize, x0: i32, y0: i32, x1: i32, y1: i32) -> i32 {
        let Some((x0, y0, x1, y1)) = self.held(x0, y0, x1, y1) else {
            return 0;
        };
        let (visible, seen) = (1u64 << player, 1u64 << (32 + player));
        let mut newly = 0;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = self.index(x, y);
                let mut c = self.maps[player].counts[i].wrapping_add(1);
                if c == 0 {
                    newly += 1;
                    self.maps[player].explored += 1;
                    self.new_tile(player, x, y);
                    c = 1;
                    self.unified[i] |= seen;
                }
                if c == 1 {
                    self.unified[i] |= visible;
                }
                self.maps[player].counts[i] = c.min(MOST);
            }
        }
        newly
    }

    /// `unexplore_terrain_sq` (`FUN_141287480`): `unexplore_terrain` over a rectangle.
    pub fn unexplore_rect(&mut self, player: usize, x0: i32, y0: i32, x1: i32, y1: i32) {
        let Some((x0, y0, x1, y1)) = self.held(x0, y0, x1, y1) else {
            return;
        };
        let hidden = !(1u64 << player);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = self.index(x, y);
                let c = self.maps[player].counts[i];
                let c = if c != 0 { c.wrapping_sub(1) } else { 0 };
                if c == 0 {
                    self.unified[i] &= hidden;
                }
                self.maps[player].counts[i] = c;
            }
        }
    }

    /// `reveal_terrain_sq` (`FUN_141285da0`): the rectangle's never explored tiles count as
    /// explored.
    pub fn reveal_rect(&mut self, player: usize, x0: i32, y0: i32, x1: i32, y1: i32) -> i32 {
        let Some((x0, y0, x1, y1)) = self.held(x0, y0, x1, y1) else {
            return 0;
        };
        let seen = 1u64 << (32 + player);
        let mut newly = 0;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = self.index(x, y);
                if self.maps[player].counts[i] == -1 {
                    self.maps[player].explored += 1;
                    newly += 1;
                    self.new_tile(player, x, y);
                    self.unified[i] |= seen;
                    self.maps[player].counts[i] = 0;
                }
            }
        }
        newly
    }

    // The map's building links (`RGE_Map`).

    /// `FUN_140f68f50`: from a tile, step back along the directions to the first tile that is not
    /// a building's non-anchor tile.
    #[must_use]
    pub fn anchor_of(&self, mut x: i32, mut y: i32) -> Option<[i32; 2]> {
        while 0 <= x && x < self.width && 0 <= y && y < self.height {
            let (bits, byte) = self.links[self.index(x, y)];
            if bits & LINKED == 0 {
                return Some([x, y]);
            }
            if bits & (LINKED | ANCHOR) != LINKED {
                return None;
            }
            let (sx, sy) = (STEP[usize::from(byte & 0xf)], STEP[usize::from(byte >> 4)]);
            x -= sx;
            y -= sy;
            if sx == 0 && sy == 0 {
                return None;
            }
        }
        None
    }

    /// `FUN_140f68e30`: the building's tile rectangle (x0, y0, x1, y1) from one of its tiles,
    /// through its anchor's UMV entry.
    #[must_use]
    pub fn extent_of(&self, mut x: i32, mut y: i32) -> Option<[i32; 4]> {
        loop {
            if x < 0 || self.width <= x || y < 0 || self.height <= y {
                return None;
            }
            let (bits, byte) = self.links[self.index(x, y)];
            match bits & (LINKED | ANCHOR) {
                ANCHOR => {
                    if byte == 0 {
                        return None;
                    }
                    let g = self.extents.get(usize::from(byte) - 1)?.guid.to_le_bytes();
                    let b = |k: usize| i32::from(g[k] as i8);
                    return Some([x - b(0), y - b(1), x + b(2), y + b(3)]);
                }
                LINKED => {}
                _ => return None,
            }
            let (sx, sy) = (STEP[usize::from(byte & 0xf)], STEP[usize::from(byte >> 4)]);
            x -= sx;
            y -= sy;
            if sx == 0 && sy == 0 {
                return None;
            }
        }
    }

    /// `FUN_140f68650`: the UMV entry (from 1) of the extent of rectangle (`x0`, `y0`)..(`x1`,
    /// `y1`) around anchor (`ax`, `ay`), added when the list has none; 0 for a rectangle of one
    /// tile or off the map.
    pub fn extent_entry(&mut self, ax: i32, ay: i32, x0: i32, y0: i32, x1: i32, y1: i32) -> i32 {
        let (lo_x, hi_x) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
        let (lo_y, hi_y) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
        if self.width <= lo_x
            || self.height <= lo_y
            || hi_x < 0
            || hi_y < 0
            || (lo_x == hi_x && lo_y == hi_y)
        {
            return 0;
        }
        let byte = |v: i32| v as u8;
        let guid = u32::from_le_bytes([
            byte(ax - lo_x),
            byte(ay - lo_y),
            byte(hi_x - ax),
            byte(hi_y - ay),
        ]);
        if 0 < self.extents_capacity
            && let Some(k) = self.extents.iter().position(|e| e.guid == guid)
        {
            return k as i32 + 1;
        }
        if self.extents_capacity <= self.extents.len() as i32 || self.extents_capacity == 0 {
            self.extents_capacity += 8;
        }
        let w = hi_x - lo_x + 1;
        let nx = match w {
            1 => 1,
            2 => 2,
            w if 2 < w => (w + 1) >> 1,
            _ => 0,
        };
        let h = hi_y - lo_y + 1;
        let (ny, slots) = match h {
            1 => (1, nx),
            2 => (2, nx * 2),
            h if 2 < h => {
                let ny = (h + 1) >> 1;
                (ny, (nx + ny) * 2 - 4)
            }
            _ => (0, 0),
        };
        let capacity = slots + 1;
        let mut offsets: Vec<[i32; 2]> = Vec::new();
        let push = |offsets: &mut Vec<[i32; 2]>, dx: i32, dy: i32| {
            if (dx != 0 || dy != 0)
                && (offsets.len() as i32) < capacity
                && !offsets.contains(&[dx, dy])
            {
                offsets.push([dx, dy]);
            }
        };
        let (dx0, dx1) = (lo_x - ax, hi_x - ax);
        let (mut dy0, mut dy1) = (lo_y - ay, hi_y - ay);
        push(&mut offsets, dx0, dy0);
        push(&mut offsets, dx1, dy0);
        push(&mut offsets, dx0, dy1);
        push(&mut offsets, dx1, dy1);
        if 2 < nx {
            let (mut a, mut b) = (dx0, dx1);
            for _ in 0..nx - 2 {
                a += 2;
                b -= 2;
                push(&mut offsets, a, dy0);
                if 1 < ny {
                    push(&mut offsets, b, dy1);
                }
            }
        }
        if 2 < ny {
            for _ in 0..ny - 2 {
                dy1 -= 2;
                dy0 += 2;
                push(&mut offsets, dx0, dy0);
                if 1 < nx {
                    push(&mut offsets, dx1, dy1);
                }
            }
        }
        self.extents.push(Extent {
            guid,
            offsets,
            capacity,
        });
        self.extents.len() as i32
    }

    /// `FUN_140f73900`: a building's tiles, `x0`..=`x1` by `y0`..=`y1` (held on the map), link
    /// to its anchor (`ax`, `ay`), which takes the building's UMV entry. A building of one tile
    /// links nothing; nor does an anchor outside the rectangle. Returns the anchor's entry.
    pub fn link_tiles(&mut self, ax: i32, ay: i32, x0: i32, y0: i32, x1: i32, y1: i32) -> i32 {
        if x0 == x1 && y0 == y1 {
            return 0;
        }
        let (lo_x, hi_x) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
        let (lo_y, hi_y) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
        let inside = lo_x <= ax && ax <= hi_x && lo_y <= ay && ay <= hi_y;
        let Some((x0, y0, x1, y1)) = self.held(lo_x, lo_y, hi_x, hi_y).filter(|_| inside) else {
            return 0;
        };
        let nibble = |v: i32| (v.clamp(-8, 7) & 0xf) as u8;
        let mut entry = 0;
        for y in y0..=y1 {
            let dy = y - ay;
            for x in x0..=x1 {
                let dx = x - ax;
                let i = self.index(x, y);
                if dx == 0 && dy == 0 {
                    entry = self.extent_entry(ax, ay, x0, y0, x1, y1);
                    self.links[i] = ((self.links[i].0 & !LINKED) | ANCHOR, entry as u8);
                } else {
                    let byte = nibble(dx) | nibble(dy) << 4;
                    self.links[i] = ((self.links[i].0 & !ANCHOR) | LINKED, byte);
                }
            }
        }
        entry
    }

    /// `FUN_140f65580`: the rectangle's tiles lose their links.
    pub fn unlink_tiles(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        if x0 == x1 && y0 == y1 {
            return;
        }
        let (lo_x, hi_x) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
        let (lo_y, hi_y) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
        let Some((x0, y0, x1, y1)) = self.held(lo_x, lo_y, hi_x, hi_y) else {
            return;
        };
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = self.index(x, y);
                self.links[i] = (self.links[i].0 & !(LINKED | ANCHOR), 0);
            }
        }
    }

    /// A building's footprint as `FUN_1411e6780` links it: from its low edge to its high edge
    /// less 0.01, around the tile of its centre.
    fn footprint(position: [f32; 3], radius: [f32; 2]) -> ([i32; 2], [i32; 4]) {
        let [x, y, _] = position;
        let [rx, ry] = radius;
        (
            [x as i32, y as i32],
            [
                (x - rx) as i32,
                (y - ry) as i32,
                ((x + rx) - EDGE) as i32,
                ((y + ry) - EDGE) as i32,
            ],
        )
    }

    /// A building (not an annex) on its tile links its footprint (`FUN_1411e6780`, after the
    /// base `new_position`); returns its UMV entry (`+0x904`).
    pub fn link_building(&mut self, position: [f32; 3], radius: [f32; 2]) -> i32 {
        let ([ax, ay], [x0, y0, x1, y1]) = Self::footprint(position, radius);
        self.link_tiles(ax, ay, x0, y0, x1, y1)
    }

    /// A building's UMV entry for its footprint (`FUN_1411e6780`'s `FUN_140f68650`), without
    /// linking its tiles.
    pub fn building_entry(&mut self, position: [f32; 3], radius: [f32; 2]) -> i32 {
        let ([ax, ay], [x0, y0, x1, y1]) = Self::footprint(position, radius);
        self.extent_entry(ax, ay, x0, y0, x1, y1)
    }

    /// The building leaves its tiles (`FUN_1411e6780` before a move, `FUN_1411dbb70`).
    pub fn unlink_building(&mut self, position: [f32; 3], radius: [f32; 2]) {
        let (_, [x0, y0, x1, y1]) = Self::footprint(position, radius);
        self.unlink_tiles(x0, y0, x1, y1);
    }

    // An object's sight (its slots 0x1d0 and 0x1d8).

    /// A building's slots pick the table: one whose radii add up to an odd whole number centres
    /// on a tile (the caller's table), any other on a tile corner (`DAT_143f51790`, probed 0,
    /// would pick the centred one).
    fn building_offset(radius: [f32; 2], offset: bool) -> bool {
        if ((radius[0] + radius[1]) as i32) & 1 != 0 {
            offset
        } else {
            true
        }
    }

    /// Slot 0x1d0: the object's sight comes, on its owner's map: a circle of its line of sight, or
    /// its footprint when `square` (`FUN_141192560`; a building's `FUN_1411d0190`). Returns the
    /// tiles explored.
    pub fn sight_on(
        &mut self,
        id: i32,
        position: [f32; 3],
        radius: [f32; 2],
        square: bool,
        offset: bool,
    ) -> i32 {
        let Some(s) = self.seers.get(&id).copied() else {
            return 0;
        };
        self.sight_on_los(id, position, radius, square, s.los as i32, offset)
    }

    /// Slot 0x1d0 with a line of sight of its own (its fourth argument not −1): a building takes
    /// 5 when a unit enters it (`FUN_141212e10`).
    pub fn sight_on_los(
        &mut self,
        id: i32,
        position: [f32; 3],
        radius: [f32; 2],
        square: bool,
        los: i32,
        offset: bool,
    ) -> i32 {
        let Some(s) = self.seers.get(&id).copied() else {
            return 0;
        };
        let (square, offset) = match s.building {
            Some(b) if !b.built => {
                if !b.started {
                    if let Some(s) = self.seers.get_mut(&id) {
                        s.explored = false;
                    }
                    return 0;
                }
                (true, offset)
            }
            Some(_) => {
                if s.master == MARKET && !self.computer.get(s.player).copied().unwrap_or(false) {
                    // its tile goes to the allied computer players' lists (player +0x180)
                    if self.computer.iter().any(|&c| c) {
                        self.unsupported("a market's sight with computer players");
                    }
                }
                (square, Self::building_offset(radius, offset))
            }
            None => (square, offset),
        };
        self.base_sight_on(id, s, position, radius, square, los, offset)
    }

    /// `FUN_141192560`.
    #[allow(clippy::too_many_arguments)]
    fn base_sight_on(
        &mut self,
        id: i32,
        s: Seer,
        position: [f32; 3],
        radius: [f32; 2],
        square: bool,
        los: i32,
        offset: bool,
    ) -> i32 {
        let square = square && s.master != CIRCLE_MASTER;
        if los <= 0 {
            return 0;
        }
        let [x, y, _] = position;
        let newly = if square {
            let [rx, ry] = radius;
            if rx <= 0.0 || ry <= 0.0 {
                return 0;
            }
            self.explore_rect(
                s.player,
                (x - rx) as i32,
                (y - ry) as i32,
                ((x + rx) - EDGE) as i32,
                ((y + ry) - EDGE) as i32,
            )
        } else {
            self.explore(s.player, x as i32, y as i32, los, offset)
        };
        if let Some(s) = self.seers.get_mut(&id) {
            s.explored = true;
        }
        newly
    }

    /// Slot 0x1d8: the object's sight goes (`FUN_1411a6f90`; a building's `FUN_1411e6f60`, which
    /// does nothing for a building that never explored).
    pub fn sight_off(
        &mut self,
        id: i32,
        position: [f32; 3],
        radius: [f32; 2],
        square: bool,
        offset: bool,
    ) {
        let Some(s) = self.seers.get(&id).copied() else {
            return;
        };
        self.sight_off_los(id, position, radius, square, s.los as i32, offset);
    }

    /// Slot 0x1d8 with a line of sight of its own (its fourth argument not −1): a building lets 5
    /// go when a unit leaves it (`FUN_141213120`).
    pub fn sight_off_los(
        &mut self,
        id: i32,
        position: [f32; 3],
        radius: [f32; 2],
        square: bool,
        los: i32,
        offset: bool,
    ) {
        let Some(s) = self.seers.get(&id).copied() else {
            return;
        };
        let offset = match s.building {
            Some(_) if !s.explored => return,
            Some(b) if !b.built => {
                if !b.started {
                    return;
                }
                offset
            }
            Some(_) => Self::building_offset(radius, offset),
            None => offset,
        };
        let square = square && s.master != CIRCLE_MASTER;
        if los <= 0 {
            return;
        }
        let [x, y, _] = position;
        if square {
            let [rx, ry] = radius;
            if rx <= 0.0 || ry <= 0.0 {
                return;
            }
            self.unexplore_rect(
                s.player,
                (x - rx) as i32,
                (y - ry) as i32,
                ((x + rx) - EDGE) as i32,
                ((y + ry) - EDGE) as i32,
            );
        } else {
            self.unexplore(s.player, x as i32, y as i32, los, offset);
        }
    }

    /// `new_position`'s sight (`0x1411a58b0`): when the object's tile changes, its sight leaves
    /// the old tile (whatever its state) and, in state 2, comes to the new one.
    pub fn moved(&mut self, id: i32, from: [f32; 3], to: [f32; 3], radius: [f32; 2]) {
        let tile = |p: [f32; 3]| [tile_of(p[0], self.width), tile_of(p[1], self.height)];
        if tile(from) == tile(to) {
            return;
        }
        self.sight_off(id, from, radius, false, false);
        if self.seers.get(&id).is_some_and(|s| s.state == 2) {
            let _ = self.sight_on(id, to, radius, false, false);
        }
    }

    /// `set_state`'s sight (`FUN_14119d710`): leaving state 2 for a higher one (or 0), the
    /// object's sight shrinks to its footprint, which goes with state 7.
    pub fn state_changed(&mut self, id: i32, position: [f32; 3], radius: [f32; 2], new: u8) {
        let Some(old) = self.seers.get(&id).map(|s| s.state) else {
            return;
        };
        if old == new || (new < old && new != 0) {
            return;
        }
        let master_sees = self.seers.get(&id).is_some_and(|s| 0.0 < s.los);
        if old == 2 && (2 < new || new == 0) {
            self.sight_off(id, position, radius, false, false);
            if master_sees {
                let _ = self.sight_on(id, position, radius, true, false);
            }
        }
        if old < 7 && new == 7 && master_sees {
            self.sight_off(id, position, radius, true, false);
        }
        if let Some(s) = self.seers.get_mut(&id) {
            s.state = new;
        }
    }

    /// A master change in a state below 3 (`FUN_1411a6a70`): the old master's sight goes and the
    /// new one's comes, whatever the state.
    pub fn master_changed(
        &mut self,
        id: i32,
        position: [f32; 3],
        old_radius: [f32; 2],
        new_radius: [f32; 2],
        master: i16,
        los: f32,
    ) {
        self.sight_off(id, position, old_radius, false, false);
        if let Some(s) = self.seers.get_mut(&id) {
            s.master = master;
            s.los = los;
        }
        let _ = self.sight_on(id, position, new_radius, false, false);
    }

    /// The master and line of sight of seer `id` without its sight (a master change inside
    /// another, whose unit has no tile, `+0x2e8` 0: its sight comes with them on its way out).
    pub fn seer_master(&mut self, id: i32, master: i16, los: f32) {
        if let Some(s) = self.seers.get_mut(&id) {
            s.master = master;
            s.los = los;
        }
    }

    /// The unified map's word at tile (`x`, `y`) of a position, masked ([`MASK`]).
    fn word_at(&self, x: f32, y: f32) -> u64 {
        self.unified[self.index(x as i32, y as i32)] & MASK
    }

    /// `RGE_Combat_Object::update`'s sight (`0x14107dfb0`, after the actions): the object's mask
    /// is the unified map's word at its tile with those of its UMV entry's tiles on the map; an
    /// annex not yet dead (state below 7) reads its main building's (`main`: its place and UMV
    /// entry), with no test of the map's edge. A changed mask updates the registry with the place
    /// read, the master's radii and class (`Update_Unit_Info`), and is the answer for an object
    /// another player takes over by seeing it (`+0x924` 1, [`sight_taker`]).
    ///
    /// # Panics
    ///
    /// Never: the registry is checked before it is updated.
    #[allow(clippy::too_many_arguments)]
    pub fn combat_sight(
        &mut self,
        id: i32,
        owner: usize,
        at: [f32; 3],
        radius: [f32; 2],
        class: i16,
        main: Option<([f32; 3], u8)>,
    ) -> Result<Option<u64>, GenieError> {
        let Some(s) = self.seers.get(&id).copied() else {
            return Ok(None);
        };
        if self.registry.is_none() || id < 0 {
            return Ok(None);
        }
        let annex = main.filter(|_| s.state < 7 && s.building.is_some());
        let ([x, y, _], umv, edge) = match annex {
            Some((place, umv)) => (place, umv, false),
            None => (at, s.umv, true),
        };
        let mut mask = self.word_at(x, y);
        if umv != 0 {
            let offsets = self
                .extents
                .get(usize::from(umv) - 1)
                .map(|e| e.offsets.clone())
                .ok_or(GenieError::unsupported(
                    "an object's UMV entry beyond the list",
                ))?;
            for [dx, dy] in offsets {
                let (fx, fy) = (dx as f32 + x, dy as f32 + y);
                let on_map =
                    0.0 <= fx && 0.0 <= fy && fx < self.width as f32 && fy < self.height as f32;
                if on_map || !edge {
                    mask |= self.word_at(fx, fy);
                }
            }
        }
        let registry = self.registry.as_mut().expect("checked above");
        let old = registry.entry(id).map_or(0, |e| e.mask);
        if mask == old {
            return Ok(None);
        }
        registry.entries.entry(id).or_default().mask = mask;
        registry.update_unit(
            id,
            owner as u8,
            [x, y],
            radius,
            class,
            old & 0xffff_ffff,
            mask,
        )?;
        Ok((s.capture == 1).then_some(mask))
    }

    /// The registry's part of a combat object's new place (`FUN_14108c320`): its records follow.
    pub fn registry_moved(&mut self, id: i32, class: i16, from: [f32; 3], to: [f32; 3]) {
        if id < 0 || (from[0] == to[0] && from[1] == to[1]) {
            return;
        }
        if let Some(r) = self.registry.as_mut() {
            r.moved(id, class, [to[0], to[1]]);
        }
    }

    /// A master change's registry part (`FUN_14108c990`): the object leaves every list of its
    /// old class and joins those of its new one, with its mask as it was.
    pub fn registry_master(
        &mut self,
        id: i32,
        owner: usize,
        at: [f32; 3],
        old: ([f32; 2], i16),
        new: ([f32; 2], i16),
    ) -> Result<(), GenieError> {
        let Some(r) = self.registry.as_mut().filter(|_| 0 <= id) else {
            return Ok(());
        };
        let mask = r.entry(id).map_or(0, |e| e.mask);
        let place = [at[0], at[1]];
        r.update_unit(id, owner as u8, place, old.0, old.1, mask & 0xffff_ffff, 0)?;
        r.update_unit(id, owner as u8, place, new.0, new.1, 0, mask)
    }

    /// The combat object's destructor (`FUN_141073060`): it leaves every list.
    pub fn registry_leave(
        &mut self,
        id: i32,
        owner: usize,
        at: [f32; 3],
        radius: [f32; 2],
        class: i16,
    ) -> Result<(), GenieError> {
        let Some(r) = self.registry.as_mut().filter(|_| 0 <= id) else {
            return Ok(());
        };
        let mask = r.entry(id).map_or(0, |e| e.mask);
        r.update_unit(
            id,
            owner as u8,
            [at[0], at[1]],
            radius,
            class,
            mask & 0xffff_ffff,
            0,
        )?;
        let _ = r.entries.remove(&id);
        Ok(())
    }

    /// An ownership change's records (`FUN_14120f800`, first of all and whatever the new owner's
    /// masters): an object with an id (`+0x8` not below 0) writes the new owner's id into each of
    /// its records (`+0x8b0`..`+0x8f0`), which keep their place in every list.
    pub fn records_owner(&mut self, id: i32, new: usize) -> Result<(), GenieError> {
        match self.registry.as_mut() {
            Some(r) if 0 <= id => r.owner_changed(id, new),
            _ => Ok(()),
        }
    }

    /// An ownership change's sight (`FUN_14120f800`, once the new owner has the master): an
    /// object that `sees` (state 2 and not inside another) loses its sight on the old owner's map
    /// (slot 0x1d8 with the old owner) and gains it on the new one's (slot 0x1d0) with `los`, the
    /// line of sight of the master of its own the change gave it (a livestock's takes the new
    /// owner's).
    pub fn owner_changed(
        &mut self,
        id: i32,
        position: [f32; 3],
        radius: [f32; 2],
        (new, los): (usize, f32),
        sees: bool,
    ) {
        if !self.seers.contains_key(&id) {
            return;
        }
        if sees {
            self.sight_off(id, position, radius, false, false);
        }
        if let Some(s) = self.seers.get_mut(&id) {
            s.player = new;
            s.los = los;
        }
        if sees {
            let _ = self.sight_on(id, position, radius, false, false);
        }
    }

    /// A building's construction completes (`FUN_1411dbb70` with 2): once built, the sight of its
    /// footprint goes (slot 0x1d8, square) and that of its line of sight comes (slot 0x1d0), each
    /// on a tile corner when its radii add up to an even number.
    pub fn construction_completed(&mut self, id: i32, position: [f32; 3], radius: [f32; 2]) {
        let Some(b) = self.seers.get_mut(&id).and_then(|s| s.building.as_mut()) else {
            return;
        };
        if b.built {
            return;
        }
        b.built = true;
        self.sight_off(id, position, radius, true, false);
        let _ = self.sight_on(id, position, radius, false, false);
    }

    /// A foundation's construction starts (`FUN_1411e5670`): its build points become a sliver,
    /// its footprint comes into sight and its tiles link again.
    pub fn construction_started(&mut self, id: i32, position: [f32; 3], radius: [f32; 2]) {
        if let Some(b) = self.seers.get_mut(&id).and_then(|s| s.building.as_mut()) {
            b.started = true;
        }
        let _ = self.sight_on(id, position, radius, true, false);
        let _ = self.link_building(position, radius);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world(players: usize) -> Visibility {
        let n = 120 * 120;
        Visibility::new(
            120,
            120,
            vec![
                VisibleMap {
                    counts: vec![-1; n],
                    explored: 0,
                };
                players
            ],
            vec![0; n],
            Vec::new(),
            0,
            vec![false; players],
        )
    }

    fn unit(player: usize, los: f32) -> Seer {
        Seer {
            player,
            master: 83,
            los,
            state: 2,
            explored: true,
            building: None,
            umv: 0,
            capture: 0,
            kind: 70,
        }
    }

    fn resources(lists: usize) -> VisibleResources {
        VisibleResources {
            capacities: vec![2; lists],
            lists: vec![Vec::new(); lists],
        }
    }

    fn rec(id: i32, x: f32, y: f32) -> VisibleResource {
        VisibleResource {
            id,
            x,
            y,
            ..VisibleResource::default()
        }
    }

    /// `RGE_Combat_Object::update`'s takeover by sight: the first player from 1 on but the owner
    /// whose bit the mask holds; outside the standard modes a computer player takes over only
    /// when it allows taking Gaia's units.
    #[test]
    fn the_first_other_player_that_sees_the_object_takes_it_over() {
        let humans = [(false, false); 4];
        assert_eq!(sight_taker(0b0111, 0, true, &humans), Some(1));
        assert_eq!(sight_taker(0b0110, 1, true, &humans), Some(2));
        assert_eq!(sight_taker(0b0001, 1, true, &humans), None);
        assert_eq!(sight_taker(1 << 33 | 0b10, 1, true, &humans), None);
        let ai = [(false, false), (true, false), (false, false), (true, true)];
        assert_eq!(sight_taker(0b1110, 0, false, &ai), Some(2));
        assert_eq!(sight_taker(0b1010, 0, false, &ai), Some(3));
        assert_eq!(sight_taker(0b0010, 0, false, &ai), None);
        assert_eq!(sight_taker(0b0010, 0, true, &ai), Some(1));
    }

    /// The combat object's sight answers a changed mask only for an object others take over by
    /// seeing it (`+0x924` 1), once the registry took it.
    #[test]
    fn a_changed_mask_is_the_answer_for_an_object_taken_over_by_sight() {
        let mut v = world(3);
        v.registry = Some(Registry::new(3, 65));
        let mut gaia = unit(0, 2.0);
        gaia.capture = 1;
        v.add_seer(7, gaia);
        v.add_seer(8, unit(0, 2.0));
        let i = 10 * 120 + 10;
        v.unified[i] = 0b011;
        let at = [10.5, 10.5, 0.0];
        assert_eq!(
            v.combat_sight(7, 0, at, [0.2, 0.2], 6, None),
            Ok(Some(0b011))
        );
        assert_eq!(v.combat_sight(7, 0, at, [0.2, 0.2], 6, None), Ok(None));
        assert_eq!(v.combat_sight(8, 0, at, [0.2, 0.2], 6, None), Ok(None));
        let r = v.registry.as_ref().unwrap();
        assert_eq!(r.entry(8).map(|e| e.mask), Some(0b011));
        assert_eq!(r.lists[1][6].len(), 2);
    }

    /// `FUN_14120f800`: an object with an id writes the new owner into its records in every
    /// list (`+0x8b0`..); one without an id has none to write.
    #[test]
    fn an_ownership_change_writes_the_new_owner_into_the_object_s_records() {
        let mut v = world(3);
        let mut r = Registry::new(3, 65);
        r.entries.entry(7).or_default().mask = 0b11;
        r.update_unit(7, 0, [10.5, 10.5], [0.2, 0.2], 6, 0, 0b11)
            .unwrap();
        v.registry = Some(r);
        v.records_owner(7, 2).unwrap();
        v.records_owner(-5, 2).unwrap();
        let r = v.registry.as_ref().unwrap();
        assert_eq!([r.lists[0][6][0].owner, r.lists[1][6][0].owner], [2, 2]);
        assert_eq!(r.lists[2][6].len(), 0);
    }

    /// `FUN_14120f800`'s sight: an object that sees loses its sight on the old owner's map
    /// (slot 0x1d8) and gains it, with its new master's line of sight, on the new one's (slot
    /// 0x1d0); one that does not see (inside another, or not in state 2) changes owner and line
    /// of sight alone.
    #[test]
    fn an_ownership_change_moves_the_sight_of_an_object_that_sees() {
        let at = [10.5, 10.5, 0.0];
        let mut v = world(3);
        v.add_seer(7, unit(0, 2.0));
        let _ = v.sight_on(7, at, [0.2, 0.2], false, false);
        let tile = |x: i32, y: i32| (y * 120 + x) as usize;
        assert_eq!(v.maps[0].counts[tile(12, 10)], 1);
        v.owner_changed(7, at, [0.2, 0.2], (1, 3.0), true);
        assert_eq!(v.maps[0].counts[tile(12, 10)], 0);
        assert_eq!(v.maps[1].counts[tile(13, 10)], 1);
        assert_eq!(v.maps[1].counts[tile(14, 10)], -1);
        let s = v.seer(7).copied().unwrap();
        assert_eq!((s.player, s.los), (1, 3.0));
        v.owner_changed(7, at, [0.2, 0.2], (2, 4.0), false);
        assert_eq!(v.maps[1].counts[tile(13, 10)], 1);
        assert_eq!(v.maps[2].counts[tile(10, 10)], -1);
        let s = v.seer(7).copied().unwrap();
        assert_eq!((s.player, s.los), (2, 4.0));
    }

    /// `AddResource` (`FUN_141288830`): the record goes last in its group's list at the object's
    /// place, its distance and byte 0; a full list doubles its capacity. A manager without lists
    /// adds nothing.
    #[test]
    fn a_resource_goes_last_in_its_group_s_list_and_a_full_list_doubles() {
        let mut r = resources(5);
        r.add(10, 4, 3.5, 7.5);
        r.add(11, 4, 1.5, 2.5);
        assert_eq!(r.capacities[4], 2);
        r.add(12, 4, 0.5, 0.5);
        assert_eq!(r.capacities, [2, 2, 2, 2, 4]);
        assert_eq!(
            r.lists[4],
            [rec(10, 3.5, 7.5), rec(11, 1.5, 2.5), rec(12, 0.5, 0.5)]
        );
        assert!(r.lists[..4].iter().all(Vec::is_empty));
        let mut none = VisibleResources::default();
        none.add(10, 0, 1.0, 1.0);
        assert_eq!(none, VisibleResources::default());
    }

    /// `Remove_Resource` (`FUN_141289b80`): the first record of the object takes the last one's
    /// record and the list shortens; its index, 0 for an empty list, -1 when not found.
    #[test]
    fn a_removed_resource_s_place_takes_the_last_record() {
        let mut r = resources(5);
        for id in [1, 2, 3] {
            r.add(id, 0, id as f32, 0.0);
        }
        assert_eq!(r.remove(1, 0), 0);
        assert_eq!(r.lists[0], [rec(3, 3.0, 0.0), rec(2, 2.0, 0.0)]);
        assert_eq!(r.remove(2, 0), 1);
        assert_eq!(r.lists[0], [rec(3, 3.0, 0.0)]);
        assert_eq!(r.remove(9, 0), -1);
        assert_eq!(r.remove(9, 1), 0);
        assert_eq!(VisibleResources::default().remove(3, 0), -1);
    }

    /// `Get_Closest_Resource` (`FUN_141289000`): the group's record nearest the point (squared,
    /// the first of equals), past the ids to skip, takes its distance; none for a group out of
    /// range or an empty list.
    #[test]
    fn the_closest_resource_skips_the_ids_given_and_keeps_its_distance() {
        let mut r = resources(5);
        r.add(1, 2, 13.0, 10.0);
        r.add(2, 2, 10.0, 12.0);
        r.add(3, 2, 7.0, 10.0);
        assert_eq!(r.closest(10.0, 10.0, 2, &[]), Some(1));
        assert_eq!(r.lists[2][1].distance, 2.0);
        assert_eq!(r.closest(10.0, 10.0, 2, &[2]), Some(0));
        assert_eq!(r.lists[2][0].distance, 3.0);
        assert_eq!(r.closest(10.0, 10.0, 2, &[2, 1]), Some(2));
        assert_eq!(r.closest(10.0, 10.0, 2, &[1, 2, 3]), None);
        assert_eq!(r.closest(10.0, 10.0, 1, &[]), None);
        assert_eq!(r.closest(10.0, 10.0, 5, &[]), None);
        assert_eq!(r.closest(10.0, 10.0, -1, &[]), None);
    }

    /// The tiles a player's map explores go to its tile list (player `+0x180`, `FUN_14165d500`)
    /// in order: the circle's (`explore_terrain`) and a rectangle's (`explore_terrain_sq`) newly
    /// explored tiles, a revealed rectangle's never explored ones (`reveal_terrain_sq`); the
    /// player's update ends with the list cleared (`FUN_141661030`).
    #[test]
    fn newly_explored_tiles_join_the_player_s_tile_list_until_its_update_ends() {
        let mut v = world(2);
        let _ = v.explore(1, 10, 10, 0, false);
        let _ = v.explore(1, 10, 10, 1, false);
        assert_eq!(
            v.new_tiles[1],
            [
                [10, 10],
                [9, 9],
                [10, 9],
                [11, 9],
                [9, 10],
                [11, 10],
                [9, 11],
                [10, 11],
                [11, 11]
            ]
        );
        let _ = v.explore_rect(1, 11, 9, 12, 9);
        let _ = v.reveal_rect(1, 11, 8, 12, 8);
        assert_eq!(v.new_tiles[1][9..], [[12, 9], [11, 8], [12, 8]]);
        assert_eq!(v.new_tiles[0], Vec::<[i32; 2]>::new());
        v.clear_new_tiles(1);
        assert_eq!(v.new_tiles[1], Vec::<[i32; 2]>::new());
    }

    /// `FUN_1411980b0`'s lists: each player whose map explored the object's tile (its bit 32 + p
    /// of the unified map, from 0 while the player exists) loses the object from its group's list.
    #[test]
    fn a_resource_leaves_the_lists_of_the_players_that_explored_its_tile() {
        let mut v = world(3);
        v.resources = vec![resources(5), resources(5), resources(5)];
        for r in &mut v.resources {
            r.add(7, 1, 20.5, 30.5);
        }
        let _ = v.explore(2, 20, 30, 0, false);
        v.forget_resource(7, [20.5, 30.5, 0.0], 1);
        assert_eq!(v.resources[2].lists[1], []);
        assert_eq!(v.resources[0].lists[1], [rec(7, 20.5, 30.5)]);
        assert_eq!(v.resources[1].lists[1], [rec(7, 20.5, 30.5)]);
    }

    #[test]
    fn a_sight_counts_its_circle_explored_then_visible_and_leaves_it_explored() {
        let mut v = world(3);
        assert_eq!(v.explore(2, 10, 10, 1, false), 9);
        assert_eq!(v.maps[2].explored, 9);
        let i = v.index(10, 10);
        assert_eq!(v.maps[2].counts[i], 1);
        assert_eq!(v.unified[i], 1 << 2 | 1 << 34);
        assert_eq!(v.explore(2, 11, 10, 1, false), 3);
        assert_eq!(v.maps[2].counts[i], 2);
        v.unexplore(2, 10, 10, 1, false);
        assert_eq!(v.maps[2].counts[i], 1);
        v.unexplore(2, 11, 10, 1, false);
        assert_eq!(v.maps[2].counts[i], 0);
        assert_eq!(v.unified[i], 1 << 34, "explored, no longer visible");
        assert_eq!(v.maps[2].explored, 12);
        // a tile at 0 stays 0; one never explored goes to -2
        v.unexplore(2, 10, 10, 0, false);
        assert_eq!(v.maps[2].counts[i], 0);
        v.unexplore(2, 20, 20, 0, false);
        assert_eq!(v.maps[2].counts[v.index(20, 20)], -2);
    }

    #[test]
    fn a_unit_s_sight_moves_with_its_tile_and_shrinks_to_its_footprint_when_it_dies() {
        let mut v = world(3);
        v.add_seer(7, unit(1, 2.0));
        let r = [0.5, 0.5];
        let _ = v.sight_on(7, [5.5, 5.5, 0.0], r, false, false);
        let before = v.maps[1].counts.clone();
        v.moved(7, [5.5, 5.5, 0.0], [5.9, 5.2, 0.0], r);
        assert_eq!(v.maps[1].counts, before, "the same tile");
        v.moved(7, [5.9, 5.2, 0.0], [6.1, 5.2, 0.0], r);
        assert_eq!(v.maps[1].counts[v.index(8, 5)], 1);
        assert_eq!(v.maps[1].counts[v.index(3, 5)], 0);
        v.state_changed(7, [6.1, 5.2, 0.0], r, 3);
        let seen: Vec<_> = (0..120 * 120)
            .filter(|&i| 0 < v.maps[1].counts[i])
            .collect();
        assert_eq!(
            seen,
            vec![v.index(5, 4), v.index(6, 4), v.index(5, 5), v.index(6, 5)]
        );
        v.state_changed(7, [6.1, 5.2, 0.0], r, 7);
        assert!(v.maps[1].counts.iter().all(|&c| c <= 0));
    }

    #[test]
    fn the_extent_list_matches_the_saved_entries_of_a_town_centre_and_a_house() {
        let mut v = world(1);
        assert_eq!(v.extent_entry(29, 94, 27, 92, 30, 95), 1);
        assert_eq!(v.extents[0].guid, 0x0101_0202);
        assert_eq!(
            v.extents[0].offsets,
            vec![[-2, -2], [1, -2], [-2, 1], [1, 1]]
        );
        assert_eq!(v.extent_entry(49, 87, 48, 86, 49, 87), 2);
        assert_eq!(v.extents[1].guid, 257);
        assert_eq!(v.extents[1].offsets, vec![[-1, -1], [0, -1], [-1, 0]]);
        assert_eq!(v.extent_entry(10, 10, 9, 10, 10, 10), 3);
        assert_eq!(v.extents[2].offsets, vec![[-1, 0]]);
        assert_eq!(v.extent_entry(30, 95, 28, 93, 31, 96), 1, "found again");
        assert_eq!(v.extents_capacity, 8);
        // 5 × 5 about its centre: the corners, then a step of 2 along the rows and the columns
        assert_eq!(v.extent_entry(10, 10, 8, 8, 12, 12), 4);
        assert_eq!(v.extents[3].guid, 0x0202_0202);
        assert_eq!(
            v.extents[3].offsets,
            vec![
                [-2, -2],
                [2, -2],
                [-2, 2],
                [2, 2],
                [0, -2],
                [0, 2],
                [-2, 0],
                [2, 0]
            ]
        );
        assert_eq!(v.extents[3].capacity, 9);
        // one tile wide and three high
        assert_eq!(v.extent_entry(10, 10, 10, 9, 10, 11), 5);
        assert_eq!(v.extents[4].offsets, vec![[0, -1], [0, 1]]);
        // off the map, or one tile: none
        assert_eq!(v.extent_entry(200, 10, 200, 10, 201, 11), 0);
        assert_eq!(v.extent_entry(10, 200, 10, 200, 11, 201), 0);
        assert_eq!(v.extent_entry(-5, 10, -5, 10, -4, 11), 0);
        assert_eq!(v.extent_entry(10, -5, 10, -5, 11, -4), 0);
        assert_eq!(v.extent_entry(10, 10, 10, 10, 10, 10), 0);
        // the list grows by 8 once full
        for w in 1..5 {
            let _ = v.extent_entry(60, 60, 60, 60, 60 + w, 61);
        }
        assert_eq!((v.extents.len(), v.extents_capacity), (9, 16));
    }

    #[test]
    fn a_first_sight_of_a_building_s_tile_explores_the_building() {
        let mut v = world(3);
        assert_eq!(v.link_building([29.0, 94.0, 0.0], [2.0, 2.0]), 1);
        assert_eq!(v.link(29, 94), (ANCHOR, 1));
        assert_eq!(v.link(28, 93), (LINKED, 0xff));
        assert_eq!(v.link(30, 95), (LINKED, 0x11));
        assert_eq!(v.anchor_of(27, 92), Some([29, 94]));
        assert_eq!(v.extent_of(30, 92), Some([27, 92, 30, 95]));
        // a sight of LOS 0 on the building's corner tile
        assert_eq!(v.explore(1, 27, 92, 0, false), 1);
        assert_eq!(v.maps[1].counts[v.index(27, 92)], 1);
        assert_eq!(v.maps[1].counts[v.index(30, 95)], 0);
        assert_eq!(v.maps[1].explored, 16);
        v.unlink_building([29.0, 94.0, 0.0], [2.0, 2.0]);
        assert_eq!(v.link(29, 94), (0, 0));
    }

    #[test]
    fn a_building_sees_its_footprint_while_built_then_its_circle_from_a_tile_corner() {
        let mut v = world(3);
        let site = Seer {
            master: 70,
            explored: false,
            state: 0,
            building: Some(BuildingSight::default()),
            ..unit(2, 2.0)
        };
        v.add_seer(9, site);
        let (at, r) = ([10.0, 10.0, 0.0], [1.0, 1.0]);
        assert_eq!(v.sight_on(9, at, r, false, false), 0, "not started");
        v.sight_off(9, at, r, true, false);
        assert!(v.maps[2].counts.iter().all(|&c| c == -1));
        v.construction_started(9, at, r);
        let seen = |v: &Visibility| (0..120 * 120).filter(|&i| 0 < v.maps[2].counts[i]).count();
        assert_eq!(seen(&v), 4);
        v.seer_mut(9).unwrap().building.as_mut().unwrap().built = true;
        v.sight_off(9, at, r, true, false);
        let _ = v.sight_on(9, at, r, false, false);
        // the table of a tile corner: at LOS 2 a 4 by 4 square around the corner
        assert_eq!(seen(&v), 16);
        assert_eq!(v.maps[2].counts[v.index(8, 9)], 1);
        assert_eq!(v.maps[2].counts[v.index(11, 9)], 1);
    }
}
