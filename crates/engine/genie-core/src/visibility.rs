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
//! Not held: the tile's fog byte (`+0x17`, for the local player's view), the minimap's and the
//! renderer's dirty tiles (player `+0x180`, `FUN_141285c70`'s `+0x1f0`, `FUN_141778750`), the
//! cheat that reveals the map (`DAT_143f68a30`, probed 0), shared sight (`+0x798`, `+0x79c`, 0 in
//! every object probed) and the line-of-sight powerup (`+0xad4`, unsaved, 0 after a load).

use crate::los_tables::{CENTRED, OFFSET};
use crate::registry::{MASK, Registry};
use crate::unit_ai::Unsupported;
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
#[derive(Clone, Debug, PartialEq)]
pub struct VisibleMap {
    /// Per tile, row by row (`+0x18`).
    pub counts: Vec<i16>,
    /// `numberTilesExploredValue` (`+0x28`).
    pub explored: i32,
}

/// A building's sight state (`TRIBE_Building_Object`: its slots 0x1d0 `FUN_1411d0190` and 0x1d8
/// `FUN_1411e6f60`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
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
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extent {
    pub guid: u32,
    pub offsets: Vec<[i32; 2]>,
    /// The offsets' capacity (the entry's second word).
    pub capacity: i32,
}

/// The players' visible maps, the unified map, the map tiles' building links and the objects'
/// sight.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Visibility {
    pub width: i32,
    pub height: i32,
    /// By player index (Gaia 0).
    pub maps: Vec<VisibleMap>,
    /// Map `+0x5e30`, row by row.
    pub unified: Vec<u64>,
    /// Per tile, the `+0x19` link bits and the `+0x1a` byte (a direction to the anchor, or the
    /// anchor's UMV entry).
    links: Vec<(u8, u8)>,
    pub extents: Vec<Extent>,
    /// The UMV list's capacity (`+0x5e04`): it grows by 8.
    pub extents_capacity: i32,
    /// Players whose type is 3: a computer player's map tells its AI of a newly explored tile
    /// (its slot 0x88), which the slice does not model.
    pub computer: Vec<bool>,
    seers: BTreeMap<i32, Seer>,
    /// The objects-in-sight registry, when the slice holds it (the save does not).
    pub registry: Option<Registry>,
    /// The first part of the game the slice met and does not model, for its caller.
    unsupported: Option<Unsupported>,
}

/// `+0x924` as a combat object's setup sets it (`FUN_14121df60`): 1 for Gaia's objects other
/// than its animals, buildings and a few classes (those of 40, 42, 58, 14, 11, 1, 60, 38, 41, 9
/// and 10, and master 826), and for masters 854 and Gaia's 814, which another player takes over
/// by seeing them; 3 for classes 58 and 1 (livestock).
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
        Visibility {
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
            unsupported: None,
        }
    }

    fn index(&self, x: i32, y: i32) -> usize {
        (y * self.width + x) as usize
    }

    /// The first unmodelled case met since the last call.
    pub fn take_unsupported(&mut self) -> Option<Unsupported> {
        self.unsupported.take()
    }

    fn unsupported(&mut self, what: &'static str) {
        self.unsupported.get_or_insert(Unsupported(what));
    }

    pub fn add_seer(&mut self, id: i32, seer: Seer) {
        self.seers.insert(id, seer);
    }

    pub fn remove_seer(&mut self, id: i32) {
        self.seers.remove(&id);
    }

    pub fn seer(&self, id: i32) -> Option<&Seer> {
        self.seers.get(&id)
    }

    pub fn seer_mut(&mut self, id: i32) -> Option<&mut Seer> {
        self.seers.get_mut(&id)
    }

    /// The link bits and byte of tile (`x`, `y`).
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
        self.reveal_rect(player, x0, y0, x1, y1);
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
        self.base_sight_on(id, s, position, radius, square, offset)
    }

    /// `FUN_141192560`.
    fn base_sight_on(
        &mut self,
        id: i32,
        s: Seer,
        position: [f32; 3],
        radius: [f32; 2],
        square: bool,
        offset: bool,
    ) -> i32 {
        let square = square && s.master != CIRCLE_MASTER;
        let los = s.los as i32;
        if los <= 0 {
            return 0;
        }
        let [x, y, _] = position;
        let newly = if !square {
            self.explore(s.player, x as i32, y as i32, los, offset)
        } else {
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
        let los = s.los as i32;
        if los <= 0 {
            return;
        }
        let [x, y, _] = position;
        if !square {
            self.unexplore(s.player, x as i32, y as i32, los, offset);
        } else {
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
            self.sight_on(id, to, radius, false, false);
        }
    }

    /// set_state's sight (`FUN_14119d710`): leaving state 2 for a higher one (or 0), the
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
                self.sight_on(id, position, radius, true, false);
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
        self.sight_on(id, position, new_radius, false, false);
    }

    /// The unified map's word at tile (`x`, `y`) of a position, masked ([`MASK`]).
    fn word_at(&self, x: f32, y: f32) -> u64 {
        self.unified[self.index(x as i32, y as i32)] & MASK
    }

    /// `RGE_Combat_Object::update`'s sight (`0x14107dfb0`, after the actions): the object's mask
    /// is the unified map's word at its tile with those of its UMV entry's tiles on the map; an
    /// annex not yet dead (state below 7) reads its main building's (`main`: its place and UMV
    /// entry), with no test of the map's edge. A changed mask updates the registry with the place
    /// read, the master's radii and class (`Update_Unit_Info`). An object another player takes
    /// over by seeing it (`+0x924` 1) is not modelled.
    #[allow(clippy::too_many_arguments)]
    pub fn combat_sight(
        &mut self,
        id: i32,
        owner: usize,
        at: [f32; 3],
        radius: [f32; 2],
        class: i16,
        main: Option<([f32; 3], u8)>,
    ) -> Result<(), Unsupported> {
        let Some(s) = self.seers.get(&id).copied() else {
            return Ok(());
        };
        if self.registry.is_none() || id < 0 {
            return Ok(());
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
                .ok_or(Unsupported("an object's UMV entry beyond the list"))?;
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
            return Ok(());
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
        let others = mask & 0xffff & !(1 << owner) & !1;
        if s.capture == 1 && others != 0 {
            return Err(Unsupported(
                "an object another player takes over by seeing it",
            ));
        }
        Ok(())
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
    ) -> Result<(), Unsupported> {
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
    ) -> Result<(), Unsupported> {
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
        r.entries.remove(&id);
        Ok(())
    }

    /// An ownership change's sight (`FUN_14120f800`): the object's records keep their place in
    /// every list with the new owner's id; an object in state 2 and not inside another loses its
    /// sight on the old owner's map and gains it, with `los` (a livestock's own master takes the
    /// new owner's master's line of sight), on the new one's.
    pub fn owner_changed(
        &mut self,
        id: i32,
        position: [f32; 3],
        radius: [f32; 2],
        new: usize,
        los: f32,
    ) -> Result<(), Unsupported> {
        let Some(s) = self.seers.get(&id).copied() else {
            return Ok(());
        };
        if let Some(r) = self.registry.as_mut() {
            r.owner_changed(id, new)?;
        }
        if s.state == 2 {
            self.sight_off(id, position, radius, false, false);
        }
        if let Some(s) = self.seers.get_mut(&id) {
            s.player = new;
            s.los = los;
        }
        if s.state == 2 {
            self.sight_on(id, position, radius, false, false);
        }
        Ok(())
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
        self.sight_on(id, position, radius, false, false);
    }

    /// A foundation's construction starts (`FUN_1411e5670`): its build points become a sliver,
    /// its footprint comes into sight and its tiles link again.
    pub fn construction_started(&mut self, id: i32, position: [f32; 3], radius: [f32; 2]) {
        if let Some(b) = self.seers.get_mut(&id).and_then(|s| s.building.as_mut()) {
            b.started = true;
        }
        self.sight_on(id, position, radius, true, false);
        self.link_building(position, radius);
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
        v.sight_on(7, [5.5, 5.5, 0.0], r, false, false);
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
        v.sight_on(9, at, r, false, false);
        // the table of a tile corner: at LOS 2 a 4 by 4 square around the corner
        assert_eq!(seen(&v), 16);
        assert_eq!(v.maps[2].counts[v.index(8, 9)], 1);
        assert_eq!(v.maps[2].counts[v.index(11, 9)], 1);
    }
}
