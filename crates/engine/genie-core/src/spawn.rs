//! A trained unit's spawn: where the make action (`TRIBE_Action_Make_Obj`, `FUN_140ad0240`) puts
//! the unit its building made. `FUN_140ace630` asks the spawn point search (`FUN_1411d12d0`) for a
//! point beside the building, then the master's slot 0x40 makes the object there.
//!
//! The search walks the building's sides in segments, stepping by the unit's size, and tries
//! each spot (`FUN_1411c5b60`) through the master's placement test (slot 0x50, `0x141264db0`):
//! the map's edge, the tiles' terrain restriction, the player's explored tiles and the objects
//! whose boxes overlap the unit's. A spot whose candidate is blocked by an object that stays
//! where it is (a building, a dead tree, a relic, a resting animal ...) ends; another blocker
//! lets the spot try its next candidate.

use crate::lifecycle::{Class, Lifecycle};
use crate::movement::Mover;
use crate::order::Work;
use crate::production::{IN_PRODUCTION, QUEUED, housed, population};
use crate::rng::Stream;
use crate::spatial::{Obstruction, Placed};
use crate::sprite::ActiveSprite;
use crate::turn::{GatherPoint, Turns, Unit};
use crate::unit_ai::{ACTION_COMPLETED, Notify, UnitAi, Unsupported, factory_type};
use crate::visibility::{Seer, capture_kind};

/// A unit smaller than this on both axes (`DAT_142f5d120`) steps by twice its radius plus
/// `GAP` (`DAT_142eb5690`).
const SMALL: f32 = crate::image::spawn::SMALL;
const GAP: f32 = crate::image::spawn::GAP;
/// The share of a side the first segments cover (`DAT_142eb880c`).
const SIDE_SHARE: f32 = crate::image::spawn::SIDE_SHARE;
/// No gather point (`DAT_14330ec20`).
const NONE: f32 = crate::image::spawn::NONE;
/// The placement test's objects lie within this many tiles of the point (`DAT_142eb9430`).
const REACH: f32 = crate::image::spawn::REACH;
/// A tile whose terrain's restriction value is this or less is not passable (`DAT_14330eb78`).
const PASSABLE: f32 = crate::image::spawn::PASSABLE;
/// How far inside its far edges a box's last tile lies (`DAT_142eb1e9c`).
const INSIDE: f32 = crate::image::spawn::INSIDE;

/// A building's master type (`'P'`).
const BUILDING: u8 = 80;
/// A combat unit's master type (`'F'`, `TRIBE_Master_Combat_Object`).
const TRIBE_COMBAT: u8 = 70;
/// The wall class, which the search checks the building's corners for (`FUN_1415417f0`).
pub const WALL: i16 = 27;
/// Classes the placement tests treat apart: a tree placed (15) passes over farms (49); buildings
/// of classes 51 and 54 block though desolid.
const TREE: i16 = 15;
const FARM: i16 = 49;
const CLASS_51: i16 = 51;
const CLASS_54: i16 = 54;
/// The gate class, whose own walls BUILD's placement scan passes over (`FUN_14123aee0`).
const GATE: i16 = 39;
/// The relic's master, which always stays (`FUN_141194e80`).
const RELIC: i16 = 285;
/// The villager class, whose new units count in attribute 248.
const VILLAGER: i16 = 4;
/// The make action's unit-AI action value (`0x26b`), which its constructor sets and its end
/// notifies.
pub(crate) const MAKE_ACTION: i32 = 0x26b;

/// A new moving object's right vector's z (`FUN_1411580a0` reads `DAT_14330ec20`).
pub(crate) const START_RIGHT_Z: f32 = crate::image::spawn::START_RIGHT_Z;

/// The building a unit spawns from, as the search reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Site {
    /// The building's place.
    pub position: [f32; 3],
    /// Its master's radii.
    pub radius: [f32; 2],
    /// Its gather point (`+0xc34`, `+0xc38`), when it has one (`+0xc30`) that is not on itself
    /// (`+0xc40`).
    pub gather: Option<[f32; 2]>,
    /// A wall at its corners (both tiles beside each: left top, right top, right bottom, left
    /// bottom), which closes the sides that meet there.
    pub walls: [bool; 4],
}

/// What the placement test found at a candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tested {
    /// Free: the unit may stand there.
    Free,
    /// Not free; `stays`: the object it reports blocks for good, which ends the spot.
    Blocked {
        /// The object it reports blocks for good, which ends the spot.
        stays: bool,
    },
}

/// One spot (`FUN_1411c5b60`'s argument): its candidates `i` in `0..=count` lie at
/// `(x + dir.x * step.x * i, y + dir.y * step.y * i)`.
struct Spot {
    at: [f32; 2],
    step: [f32; 2],
    dir: [i32; 2],
    count: i32,
}

/// `FUN_1411c5b60`: the first free candidate; a candidate blocked by an object that stays ends
/// the spot.
fn try_spot(
    spot: &Spot,
    test: &mut dyn FnMut([f32; 2]) -> Result<Tested, Unsupported>,
) -> Result<Option<[f32; 2]>, Unsupported> {
    for i in 0..=spot.count {
        let x = spot.dir[0] as f32 * spot.step[0] * i as f32 + spot.at[0];
        let y = spot.dir[1] as f32 * spot.step[1] * i as f32 + spot.at[1];
        match test([x, y])? {
            Tested::Free => return Ok(Some([x, y])),
            Tested::Blocked { stays: true } => return Ok(None),
            Tested::Blocked { stays: false } => {}
        }
    }
    Ok(None)
}

/// `FUN_1411d12d0`: the spawn point beside `site` for a unit of radii `unit`. The segments of
/// the sides facing the gather point come first (the top side's left part, the left side's top
/// part, the top's right part, the left's lower part; then the right side down; then the bottom
/// side), each only where no wall closes its corner; a second pass tries every open side. Without
/// a gather point there is one pass. The search's later rings (cases 11 to 18: the spots on the
/// building's edge and the rings around it, `FUN_1411d0b20`) and its first test under
/// `DAT_143f01833` (`FUN_1411c52f0`, probed 0) are not modelled.
pub fn spawn_point(
    site: &Site,
    unit: [f32; 2],
    test: &mut dyn FnMut([f32; 2]) -> Result<Tested, Unsupported>,
) -> Result<[f32; 2], Unsupported> {
    let [mut sx, mut sy] = unit;
    if sx < SMALL && sy < SMALL {
        sx = sx + sx + GAP;
        sy = sy + sy + GAP;
    }
    let [bx, by] = site.radius;
    let [x, y, _] = site.position;
    let left = x - (bx + sx);
    let right = x + bx + sx;
    let top = y + by + sy;
    let bottom = y - (by + sy);
    let [left_top, right_top, right_bottom, left_bottom] = site.walls;
    let [gx, gy] = site.gather.unwrap_or([NONE, NONE]);
    let dx = (x - gx).abs();
    let dy = (y - gy).abs();
    let (no_x, no_y) = (gx == NONE, gy == NONE);
    // a spot of a horizontal segment tries its point and the next one up (or down); a vertical
    // segment's, the next one out
    let up = |at| Spot {
        at,
        step: [0.0, sy],
        dir: [0, 1],
        count: 1,
    };
    let down = |at| Spot {
        at,
        step: [0.0, sy],
        dir: [0, -1],
        count: 1,
    };
    let out_left = |at| Spot {
        at,
        step: [sx, 0.0],
        dir: [-1, 0],
        count: 1,
    };
    let out_right = |at| Spot {
        at,
        step: [sx, 0.0],
        dir: [1, 0],
        count: 1,
    };
    for pass in 1..=2 {
        let second = pass == 2;
        let mut tries: Vec<Spot> = Vec::new();
        let mut run = |spots: &mut Vec<Spot>| -> Result<Option<[f32; 2]>, Unsupported> {
            for s in spots.drain(..) {
                if let Some(p) = try_spot(&s, test)? {
                    return Ok(Some(p));
                }
            }
            Ok(None)
        };
        if !left_top {
            // (1) the top side's left part
            if no_y || second || ((y < gy && dx <= dy) && (no_x || gx < x)) {
                let mut cx = left;
                while cx < bx * SIDE_SHARE + bx + left {
                    tries.push(up([cx, top]));
                    cx += sx;
                }
                if let Some(p) = run(&mut tries)? {
                    return Ok(p);
                }
            }
            // (2) the left side's top part
            if no_x || second || ((gx < x && dy < dx) && (no_y || y < gy)) {
                let mut cy = top;
                while top - (by * SIDE_SHARE + by) < cy {
                    tries.push(out_left([left, cy]));
                    cy -= sy;
                }
                if let Some(p) = run(&mut tries)? {
                    return Ok(p);
                }
            }
            // (3) the top side's right part
            if !right_top && (no_y || second || (y < gy && dx <= dy)) {
                let mut cx = bx * SIDE_SHARE + bx + left;
                while cx < bx + bx + left + sx + sx {
                    tries.push(up([cx, top]));
                    cx += sx;
                }
                if let Some(p) = run(&mut tries)? {
                    return Ok(p);
                }
            }
            // (4) the left side's lower part
            if !left_bottom && (no_x || second || (gx < x && dy < dx)) {
                let mut cy = top - (by * SIDE_SHARE + by);
                while top - (by + by + sy + sy) < cy {
                    tries.push(out_left([left, cy]));
                    cy -= sy;
                }
                if let Some(p) = run(&mut tries)? {
                    return Ok(p);
                }
            }
        }
        if !right_top && (no_x || second || (x < gx && dy < dx)) {
            // (5) the right side's top half
            if no_y || second || y < gy {
                let mut cy = top;
                while top - by < cy {
                    tries.push(out_right([right, cy]));
                    cy -= sy;
                }
                if let Some(p) = run(&mut tries)? {
                    return Ok(p);
                }
            }
            // (6) its lower half
            if !right_bottom {
                let mut cy = top - by;
                while top - (by + by + sy + sy) < cy {
                    tries.push(out_right([right, cy]));
                    cy -= sy;
                }
                if let Some(p) = run(&mut tries)? {
                    return Ok(p);
                }
            }
        }
        let bottom_closed = left_bottom || (!no_y && !second && (y <= gy || dy < dx));
        if !bottom_closed {
            // (7) the bottom side's left half
            if no_x || second || gx < x {
                let mut cx = left;
                while cx < left + bx {
                    tries.push(down([cx, bottom]));
                    cx += sx;
                }
                if let Some(p) = run(&mut tries)? {
                    return Ok(p);
                }
            }
            // (8) its right half
            if !right_bottom {
                let mut cx = left + bx;
                while cx < bx + bx + sx + sx + left {
                    tries.push(down([cx, bottom]));
                    cx += sx;
                }
                if let Some(p) = run(&mut tries)? {
                    return Ok(p);
                }
            }
        }
        // (9), (10): with the top right and bottom left corners walled, the second pass tries the
        // bottom side from the right and the right side from the bottom
        if !right_bottom && right_top && left_bottom && second {
            let mut cx = right;
            while right - bx < cx {
                tries.push(down([cx, bottom]));
                cx -= sx;
            }
            let mut cy = bottom;
            while cy < bottom + by {
                tries.push(out_right([right, cy]));
                cy += sy;
            }
            if let Some(p) = run(&mut tries)? {
                return Ok(p);
            }
        }
        if no_x {
            break;
        }
    }
    Err(Unsupported(
        "a spawn point beyond the building's sides (the search's later rings)",
    ))
}

/// An object near a candidate, as the placement test and the blocker test read it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Near {
    /// The object.
    pub id: i32,
    /// Its place.
    pub position: [f32; 3],
    /// Its master's radii on the ground and whether its height radius is above 0.
    pub radius: [f32; 2],
    /// Its master's height radius is above 0.
    pub tall: bool,
    /// Its master's type (`+0x8`), class, id, `can_be_built_on` (`+0x5e9`), `obstruction_type`
    /// (`+0x651`) and `enable_auto_gather` (`+0x681`).
    pub kind: u8,
    /// Its master's class.
    pub class: i16,
    /// Its master.
    pub master: i16,
    /// Its master's `can_be_built_on` (`+0x5e9`).
    pub buildable_on: u8,
    /// Its master's `obstruction_type` (`+0x651`).
    pub obstruction_type: u8,
    /// Its master's `enable_auto_gather` (`+0x681`).
    pub auto_gather: bool,
    /// Its state (`+0x640`) and, for a building, its `desolid_flag` (`+0xc60`).
    pub state: u8,
    /// A building's `desolid_flag` (`+0xc60`).
    pub desolid: bool,
    /// Its owner.
    pub player: usize,
    /// Its master moves (speed above 0).
    pub moving: bool,
    /// Its master's `fog_visibility` (`+0x605`).
    pub fog_visibility: u8,
    /// A building's visibility mask (`+0x8f8`, the registry's) when its footprint has a UMV entry
    /// (`+0x904` not 0); None otherwise.
    pub fog_mask: Option<u64>,
    /// It holds an amount (`+0x310` above 0) of food, wood, stone or gold (its attribute,
    /// `+0x308`, through `FUN_140fdd470` below 4: 0 to 3 and the meats 15 to 17).
    pub holds: bool,
    /// Its hit points.
    pub hp: f32,
    /// The terrain of its tile (`+0x2e8`, taken as the tile it stands on) is a forest
    /// ([`crate::terrain::TerrainKind::forest`]).
    pub forest_tile: bool,
}

impl Near {
    /// The blocker tests of `FUN_1411c5b60`: a building (`FUN_1411942c0`), a tree past state 2
    /// (`FUN_141195660`), the relic (`FUN_141194e80`), and `FUN_141194170`'s master with
    /// `enable_auto_gather` or of a class that stays from state 2 on (5, 9, 10, 29, 31, 33, 38,
    /// 40, 41, 58, 61) or from state 3 on (7, 8, 15, 32, 48, 63).
    #[must_use]
    pub const fn stays(&self) -> bool {
        if self.kind == BUILDING || (self.class == TREE && 2 < self.state) || self.master == RELIC {
            return true;
        }
        if self.auto_gather {
            return true;
        }
        match self.class {
            5 | 9 | 10 | 29 | 31 | 33 | 38 | 40 | 41 | 58 | 61 => 1 < self.state,
            7 | 8 | 15 | 32 | 48 | 63 => 2 < self.state,
            _ => false,
        }
    }
}

/// The master a placement test places (its slot 0x50 with the spawn's arguments: by its radii,
/// the terrain checked, the player's explored tiles needed, the elevation checked by its hill
/// mode, the objects checked).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placing<'a> {
    /// The unit's player.
    pub player: usize,
    /// Its master's radii.
    pub radius: [f32; 2],
    /// Its master's height radius is above 0.
    pub tall: bool,
    /// Its master's class.
    pub class: i16,
    /// Its terrain restriction's value per terrain.
    pub restriction: &'a [f32],
    /// `hill_mode` (`+0x604`).
    pub hill_mode: u8,
    /// The placement terrains (`+0x5f2`..`+0x5f8`, [`placement_terrains`]).
    pub placement: [i16; 4],
}

/// What the placement test reads of the map.
pub trait PlaceMap {
    /// The map's size in tiles.
    fn size(&self) -> [i32; 2];
    /// A tile's terrain (the map tile's `+0x9`).
    fn terrain(&self, x: i32, y: i32) -> Result<u8, Unsupported>;
    /// Terrain `t`'s flags (the `.dat`'s `is_water` byte, the map's terrain record `+2`), None
    /// beyond the terrains.
    fn terrain_flags(&self, _t: usize) -> Option<u8> {
        None
    }
    /// How many terrains the map's table holds.
    fn terrain_count(&self) -> usize {
        0
    }
    /// `FUN_141663500` is not 0: the player sees or saw the tile.
    fn explored(&self, player: usize, x: i32, y: i32) -> bool;
    /// The player sees the tile now (its mask `+0x2a8` meets the tile's visible bits).
    fn visible(&self, player: usize, x: i32, y: i32) -> bool;
    /// The player's masks: what it sees (`+0x2a8`) and what it explored (`+0x2a0`).
    fn vision(&self, player: usize) -> [u64; 2];
    /// A tile's elevation (`+0x14`), when the map holds it.
    fn elevation(&self, x: i32, y: i32) -> Option<u8>;
    /// A tile's shape (`+0x8`, [`crate::terrain::Terrain::set_elevations`]), when the map holds
    /// it.
    fn shape(&self, x: i32, y: i32) -> Option<u8>;
    /// Player `a` counts player `b` as an ally (`FUN_141663dc0`): itself; never Gaia; relation 0,
    /// or with `with_4` relation 4 too (the zone mode's relation 1 is not modelled).
    fn allied(&self, a: usize, b: usize, with_4: bool) -> bool;
    /// The objects on tile (`x`, `y`) (their centre there), in the tile list's order; `Err` where
    /// it cannot tell.
    fn objects(&self, x: i32, y: i32) -> Result<Vec<Near>, Unsupported>;
}

/// The tiles within `REACH` of a point, clamped to the map (`(int)` of each bound, then the map's
/// edges), row by row.
fn reach(map: [i32; 2], [x, y]: [f32; 2]) -> impl Iterator<Item = (i32, i32)> {
    let x0 = ((x - REACH) as i32 as i16).max(0);
    let x1 = ((x + REACH) as i32 as i16).min((map[0] - 1) as i16);
    let y0 = ((y - REACH) as i32 as i16).max(0);
    let y1 = ((y + REACH) as i32 as i16).min((map[1] - 1) as i16);
    (y0..=y1).flat_map(move |ty| (x0..=x1).map(move |tx| (i32::from(tx), i32::from(ty))))
}

/// An object a placement test's scan found, with its tile.
type Found = ((i32, i32), Near);

/// The objects whose box overlaps a box of radii `r` at `p` among those `counts` lets through,
/// with their tile, in the scan's order.
fn overlapping(
    map: &dyn PlaceMap,
    p: [f32; 2],
    r: [f32; 2],
    counts: &dyn Fn(&Near) -> Option<bool>,
) -> Result<Vec<Found>, Unsupported> {
    let mut out = Vec::new();
    for (tx, ty) in reach(map.size(), p) {
        for o in map.objects(tx, ty)? {
            let Some(tall) = counts(&o) else { continue };
            if !(0.0 < o.radius[0] && 0.0 < o.radius[1] && tall) {
                continue;
            }
            let dx = (o.position[0] - p[0]).abs();
            let dy = (o.position[1] - p[1]).abs();
            if dx < o.radius[0] + r[0] && dy < o.radius[1] + r[1] {
                out.push(((tx, ty), o));
            }
        }
    }
    Ok(out)
}

/// What BUILD's placement test reads of the building master and the game.
#[derive(Clone, Copy, Debug)]
pub struct BuildSite<'a> {
    /// The player who places it.
    pub player: usize,
    /// A human player (`p6`: the explored test).
    pub human: bool,
    /// The master's clearance radii (`+0x5fc`, `+0x600`; `p9`).
    pub clearance: [f32; 2],
    /// The master's class.
    pub class: i16,
    /// The master's `obstruction_type` is neither 0 nor 12 (`FUN_1411205c0`).
    pub obstructs: bool,
    /// The master's terrain restriction's value per terrain.
    pub restriction: &'a [f32],
    /// `hill_mode` (`+0x604`, `p7`).
    pub hill_mode: u8,
    /// The placement terrains (`+0x5f2`..`+0x5f8`, [`placement_terrains`]).
    pub placement: [i16; 4],
    /// The world's zone mode (`+0x820`, `FUN_1415e8e10`).
    pub zones: bool,
}

/// The hill mode's test of a tile of the box (`FUN_141123f80`'s `p7` part, the master's
/// `hill_mode` `+0x604`): mode 1 takes flat ground and the four side slopes (shapes 0 and 5 to
/// 8), mode 2 flat ground alone, mode 3 a tile of the box's first column or last row within one
/// step of the `base` elevation; the other modes take any. True when the tile fails (code 3).
fn hill_fails(
    mode: u8,
    map: &dyn PlaceMap,
    (tx, ty): (i32, i32),
    (x0, y1): (i32, i32),
    base: Option<u8>,
) -> Result<bool, Unsupported> {
    let shape = || {
        map.shape(tx, ty).ok_or(Unsupported(
            "a hill mode where the tiles' shapes are not known",
        ))
    };
    Ok(match mode {
        1 => !matches!(shape()?, 0 | 5..=8),
        2 => shape()? != 0,
        3 if tx == x0 || ty == y1 => {
            let e = i32::from(map.elevation(tx, ty).ok_or(Unsupported(
                "a hill mode where the elevations are not known",
            ))?);
            let b = i32::from(base.unwrap_or(0));
            e < b - 1 || b + 1 < e
        }
        _ => false,
    })
}

/// `FUN_141123f80`'s placement terrains (`placement`: the master's side terrains `+0x5f2`,
/// `+0x5f4`, then its terrains `+0x5f6`, `+0x5f8`; −1 for none) for a box of tiles `b` (x0, y0,
/// x1, y1) around point `p`: false where they refuse it (the test's code 1). With a terrain,
/// the point's tile must be of one of the two, or share the flag 4 or 8 (the terrains' `.dat`
/// flags, [`PlaceMap::terrain_flags`]) with them, and the box shrinks to that tile. With a side
/// terrain, one of the tiles beside the box (the row below, the column right, the row above and
/// the column left) must be of a side terrain, or of a terrain with the flag 0x40 or 0x10 when a
/// side terrain has it. A terrain of −1 beside one of 0 or more (its flags read past the table)
/// is not modelled.
///
/// # Errors
///
/// Where the map cannot tell.
pub fn placement_terrains(
    placement: [i16; 4],
    map: &dyn PlaceMap,
    p: [f32; 2],
    b: [i32; 4],
) -> Result<bool, Unsupported> {
    let [side1, side2, t1, t2] = placement;
    let flags = |t: i16| {
        usize::try_from(t)
            .ok()
            .and_then(|t| map.terrain_flags(t))
            .ok_or(Unsupported(
                "placement terrains with one of -1 or beyond the terrains",
            ))
    };
    let mut b = b;
    if 0 <= t1 || 0 <= t2 {
        let (x, y) = (p[0] as i32, p[1] as i32);
        let tile = i16::from(map.terrain(x, y)?);
        if tile != t1 && tile != t2 {
            let (f1, f2, ft) = (flags(t1)?, flags(t2)?, flags(tile)?);
            let water = (f1 & 4 != 0 || f2 & 4 != 0) && ft & 4 != 0;
            let shallow = (f1 & 8 != 0 || f2 & 8 != 0) && ft & 8 != 0;
            if !water && !shallow {
                return Ok(false);
            }
        }
        b = [x, y, x, y];
    }
    if side1 < 0 && side2 < 0 {
        return Ok(true);
    }
    let (f1, f2) = (flags(side1)?, flags(side2)?);
    let mut sides: Vec<i16> = [side1, side2].into_iter().filter(|&t| 0 <= t).collect();
    for bit in [0x40, 0x10] {
        if f1 & bit != 0 || f2 & bit != 0 {
            for t in 0..map.terrain_count() {
                let t = i16::try_from(t).unwrap_or(i16::MAX);
                if flags(t)? & bit != 0 && t != side1 && t != side2 {
                    sides.push(t);
                }
            }
        }
    }
    let [x0, y0, x1, y1] = b;
    let [w, h] = map.size();
    let mut beside = Vec::new();
    if 0 < y0 {
        beside.extend((x0..=x1).map(|x| (x, y0 - 1)));
    }
    if x1 + 1 < w {
        beside.extend((y0..=y1).map(|y| (x1 + 1, y)));
    }
    if y1 + 1 < h {
        beside.extend((x0..=x1).rev().map(|x| (x, y1 + 1)));
    }
    if 0 < x0 {
        beside.extend((y0..=y1).rev().map(|y| (x0 - 1, y)));
    }
    for (x, y) in beside {
        if sides.contains(&i16::from(map.terrain(x, y)?)) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// BUILD's placement test (`FUN_140e7a400`'s call of the master's slot 0x50, the building
/// master's `FUN_14123aee0` with BUILD's flags: `p6` human, `p7`, `p8`, `p9` 1, `p10` 0, `p11` 1,
/// no object of its own, `p13` 0, `p14` 1): 0 when the foundation may go to `p`, else the game's
/// code. First the static test (`FUN_141123f80` through `FUN_141264db0`, whose scans are off)
/// over the clearance box: off the map 7; per tile, columns then rows, a terrain the
/// restriction forbids 2 (`DAT_142eb7e5c`'s threshold, the spawn's); a human who explored none of
/// the tiles 5; a hill mode's tile ([`hill_fails`]) 3. Then the building master's scan of the tiles within 8 (rows, then columns; each
/// tile's objects in its list's order): an object the player sees, or on a tile it never
/// explored (the test's explored flag `+0x730` is set by then), or in the fog with a fog
/// visibility, and only its own in the fog without one, that is below state 3 (another type
/// below 7), standing (or the master obstructs, or it is below type 30), not a wall under its
/// own gate, a desolid building only of an ally both ways, not a moving unit of the player, Gaia
/// or a mutual ally, not a felled tree off a forest (at most 1 hit point; `+0x2e8` taken as its
/// own tile), not buildable on, whose box with a
/// height overlaps the clearance box: 6. Not modelled: the placement terrains, hill modes, the
/// zone test (`FUN_1415c6400`), the editor (game mode 4), a fogged object of fog visibility 3,
/// game type 7's test of allies, the
/// outline radii of
/// obstruction types 11 to 13.
pub fn build_placement(
    site: &BuildSite<'_>,
    map: &dyn PlaceMap,
    p: [f32; 2],
) -> Result<u8, Unsupported> {
    let [x, y] = p;
    let [rx, ry] = site.clearance;
    let [w, h] = map.size();
    let (far_x, far_y) = ((x + rx) - INSIDE, (y + ry) - INSIDE);
    if x - rx < 0.0 || w as f32 <= far_x || y - ry < 0.0 || h as f32 <= far_y {
        return Ok(7);
    }
    let mut explored = false;
    let (x0, x1, y0, y1) = ((x - rx) as i32, far_x as i32, (y - ry) as i32, far_y as i32);
    if !placement_terrains(site.placement, map, p, [x0, y0, x1, y1])? {
        return Ok(1);
    }
    // hill mode 3 measures the box's edge against the elevation of its first column's last row
    let base = if site.hill_mode == 3 {
        Some(map.elevation(x0, y1).ok_or(Unsupported(
            "a hill mode where the elevations are not known",
        ))?)
    } else {
        None
    };
    for tx in x0..=x1 {
        for ty in y0..=y1 {
            let t = map.terrain(tx, ty)?;
            let value = site
                .restriction
                .get(usize::from(t))
                .ok_or(Unsupported("a terrain its restriction does not list"))?;
            if *value <= PASSABLE {
                return Ok(2);
            }
            explored = explored || (site.human && map.explored(site.player, tx, ty));
            if hill_fails(site.hill_mode, map, (tx, ty), (x0, y1), base)? {
                return Ok(3);
            }
        }
    }
    if site.human && !explored {
        return Ok(5);
    }
    if (0.0 < rx || 0.0 < ry) && site.zones {
        return Err(Unsupported("BUILD's placement test in a zone mode"));
    }
    // FUN_14123aee0's scan
    let near = |v: f32, size: i32| {
        let low = (v - BUILD_SCAN) as i32 as i16;
        let high = (v + BUILD_SCAN) as i32 as i16;
        (
            i32::from(low.max(0)),
            if i32::from(high) < size {
                i32::from(high)
            } else {
                size - 1
            },
        )
    };
    let ((x0, x1), (y0, y1)) = (near(x, w), near(y, h));
    let player = site.player;
    for ty in y0..=y1 {
        for tx in x0..=x1 {
            let (seen, visible) = (map.explored(player, tx, ty), map.visible(player, tx, ty));
            for o in map.objects(tx, ty)? {
                if !(seen && visible) {
                    let fog = if seen { o.fog_visibility } else { 1 };
                    if fog == 3 {
                        return Err(Unsupported(
                            "a fogged object of fog visibility 3 near a BUILD",
                        ));
                    }
                    // without a fog visibility only the player's own objects count, and a
                    // building (not a farm) whose visibility mask meets both the player's masks
                    if fog == 0 {
                        let [sees, explored] = map.vision(player);
                        let shown = o.class != 49
                            && 50 <= o.kind
                            && o.fog_mask
                                .is_some_and(|m| m & explored != 0 && m & sees != 0);
                        if !shown && (o.player != player || !map.allied(o.player, player, true)) {
                            continue;
                        }
                    }
                } else if o.fog_visibility == 3 {
                    continue;
                }
                let alive = o.state < 3 || (o.kind != BUILDING && o.state < 7);
                let standing = site.obstructs || o.kind < 30 || !o.moving;
                let gate_wall = site.class == GATE && o.class == WALL && o.player == player;
                let foundation = o.kind == BUILDING
                    && !matches!(o.class, CLASS_51 | CLASS_54)
                    && o.desolid
                    && !(map.allied(player, o.player, true) && map.allied(o.player, player, true));
                if !alive || !standing || gate_wall || foundation {
                    continue;
                }
                // a moving unit of the player, of Gaia or of a mutual ally passes (the game type
                // 7's own test of allies is not modelled)
                let passes = 30 <= o.kind
                    && o.state == 2
                    && o.moving
                    && (o.player == player
                        || o.player == 0
                        || (map.allied(player, o.player, false)
                            && map.allied(o.player, player, false)));
                if passes || o.buildable_on == 1 {
                    continue;
                }
                // a felled tree (at most 1 hit point) passes off a forest (`FUN_140f5efb0` on its
                // tile's terrain)
                if o.class == TREE && o.hp <= 1.0 && !o.forest_tile {
                    continue;
                }
                if (11..=13).contains(&o.obstruction_type) {
                    return Err(Unsupported(
                        "an object of obstruction types 11 to 13 near a BUILD (its outline radii)",
                    ));
                }
                // no height: a building's counts as 1, and so does that of an object holding food,
                // wood, stone or gold
                let tall = o.tall || o.kind == BUILDING || o.holds;
                let [orx, ory] = o.radius;
                if 0.0 < orx
                    && 0.0 < ory
                    && tall
                    && (o.position[0] - x).abs() < orx + rx
                    && (o.position[1] - y).abs() < ory + ry
                {
                    return Ok(6);
                }
            }
        }
    }
    Ok(0)
}

/// How far BUILD's placement scan looks around the point (`FUN_14123aee0`, `DAT_142eb9430`).
const BUILD_SCAN: f32 = crate::image::spawn::BUILD_SCAN;

/// The first object of a scan, which the game reports as the blocker. Objects on one tile come
/// in its list's order, which the slice does not hold: several there must agree on staying.
fn first(found: &[Found]) -> Result<Option<Near>, Unsupported> {
    let Some(&(tile, o)) = found.first() else {
        return Ok(None);
    };
    let disagree = found
        .iter()
        .take_while(|(t, _)| *t == tile)
        .any(|(_, other)| other.stays() != o.stays());
    if disagree {
        return Err(Unsupported(
            "objects on one tile that block a spawn point differently (the tile list's order)",
        ));
    }
    Ok(Some(o))
}

/// The master's slot 0x50 (`0x141264db0`) with the spawn's arguments: `FUN_141123f80`'s test
/// (the map's edge, then per tile of the box the terrain restriction and the explored tiles,
/// then the objects overlapping), then its own scan of the objects overlapping, which reports
/// the first one. The editor's case (the game's mode 4), the placement terrains, the hill modes,
/// the zone test (`FUN_1415c6400`, the world's `+0x820` mode 1) and the moving objects' own
/// check (`DAT_143f01833`, probed 0) are not modelled.
pub fn placement(
    me: &Placing<'_>,
    map: &dyn PlaceMap,
    p: [f32; 2],
    zones: bool,
) -> Result<Tested, Unsupported> {
    let [x, y] = p;
    let [rx, ry] = me.radius;
    let [w, h] = map.size();
    let (far_x, far_y) = ((x + rx) - INSIDE, (y + ry) - INSIDE);
    // FUN_141123f80's own result: 0 free; the objects of its scan report the blocker (6)
    let mut base_blocker = None;
    let off_map = x - rx < 0.0 || w as f32 <= far_x || y - ry < 0.0 || h as f32 <= far_y;
    let base_free = if off_map
        || !placement_terrains(
            me.placement,
            map,
            p,
            [(x - rx) as i32, (y - ry) as i32, far_x as i32, far_y as i32],
        )? {
        false
    } else {
        let (x0, y0, x1, y1) = ((x - rx) as i32, (y - ry) as i32, far_x as i32, far_y as i32);
        let mut passable = true;
        let mut seen = false;
        'tiles: for tx in x0..=x1 {
            for ty in y0..=y1 {
                let t = map.terrain(tx, ty)?;
                let value = me
                    .restriction
                    .get(usize::from(t))
                    .ok_or(Unsupported("a terrain its restriction does not list"))?;
                if *value <= PASSABLE {
                    passable = false;
                    break 'tiles;
                }
                seen = seen || map.explored(me.player, tx, ty);
                if me.hill_mode != 0 {
                    return Err(Unsupported("a placement test of a hill mode"));
                }
            }
        }
        if !passable || !seen {
            false
        } else if 0.0 < rx || 0.0 < ry {
            if zones {
                return Err(Unsupported("a placement test in a zone mode"));
            }
            let counts = |o: &Near| {
                let listed =
                    o.state < 7 && o.buildable_on != 1 && (o.kind != BUILDING || !o.desolid);
                let tall = o.tall
                    || (me.class == TREE && o.class == FARM)
                    || (o.kind == BUILDING && o.obstruction_type != 0);
                listed.then_some(tall && me.tall)
            };
            let found = if me.tall {
                overlapping(map, p, me.radius, &counts)?
            } else {
                Vec::new()
            };
            base_blocker = first(&found)?;
            base_blocker.is_none()
        } else {
            true
        }
    };
    // slot 0x50's own scan
    let counts = |o: &Near| {
        let listed = o.state < 7
            && o.buildable_on != 1
            && (o.kind != BUILDING || o.class == CLASS_51 || o.class == CLASS_54 || !o.desolid);
        let tall = o.tall || (o.kind == BUILDING && !matches!(o.obstruction_type, 0 | 12));
        listed.then_some(tall && me.tall)
    };
    let found = if me.tall && (0.0 < rx || 0.0 < ry) {
        overlapping(map, p, me.radius, &counts)?
    } else {
        Vec::new()
    };
    let blocker = first(&found)?.or(base_blocker);
    if base_free && found.is_empty() {
        return Ok(Tested::Free);
    }
    Ok(Tested::Blocked {
        stays: blocker.is_some_and(|b| b.stays()),
    })
}

/// The corners' tile pairs (`FUN_1411d12d0`'s wall checks, left top, right top, right bottom,
/// left bottom) for a unit of radii `unit` beside `site`.
fn corner_tiles(position: [f32; 3], radius: [f32; 2], unit: [f32; 2]) -> [[(i32, i32); 2]; 4] {
    let [mut sx, mut sy] = unit;
    if sx < SMALL && sy < SMALL {
        sx = sx + sx + GAP;
        sy = sy + sy + GAP;
    }
    let [x, y, _] = position;
    let [bx, by] = radius;
    let left = (x - (bx + sx)) as i32;
    let right = (x + bx + sx) as i32;
    let top = (y + by + sy) as i32;
    let bottom = (y - (by + sy)) as i32;
    [
        [(left, top - 1), (left + 1, top)],
        [(right - 1, top), (right, top - 1)],
        [(right, bottom + 1), (right - 1, bottom)],
        [(left + 1, bottom), (left, bottom + 1)],
    ]
}

/// The map as the spawn's placement test reads it: the terrain, the players' sight and the
/// objects with what their units and masters say.
pub(crate) struct WorldMap<'a> {
    pub(crate) turns: &'a Turns,
}

impl PlaceMap for WorldMap<'_> {
    fn terrain_flags(&self, t: usize) -> Option<u8> {
        self.turns
            .spatial
            .terrain
            .as_ref()
            .and_then(|m| m.terrains.get(t))
            .map(|k| k.flags)
    }

    fn terrain_count(&self) -> usize {
        self.turns
            .spatial
            .terrain
            .as_ref()
            .map_or(0, |m| m.terrains.len())
    }

    fn size(&self) -> [i32; 2] {
        self.turns.spatial.map
    }

    fn terrain(&self, x: i32, y: i32) -> Result<u8, Unsupported> {
        let t = self
            .turns
            .spatial
            .terrain
            .as_ref()
            .ok_or(Unsupported("a placement test without the map's terrain"))?;
        t.kind(x, y)
            .ok_or(Unsupported("a placement test off the map's tiles"))
    }

    fn explored(&self, player: usize, x: i32, y: i32) -> bool {
        // without the sight the spawn stops before (Turns::spawn)
        self.turns.spatial.explored(player, x, y).unwrap_or(false)
    }

    fn visible(&self, player: usize, x: i32, y: i32) -> bool {
        self.turns.spatial.visible(player, x, y)
    }

    fn vision(&self, player: usize) -> [u64; 2] {
        self.turns.spatial.vision(player)
    }

    fn elevation(&self, x: i32, y: i32) -> Option<u8> {
        let t = self.turns.spatial.terrain.as_ref()?;
        let [w, h] = self.turns.spatial.map;
        (0 <= x && 0 <= y && x < w && y < h)
            .then(|| t.elevations.get((y * w + x) as usize).copied())
            .flatten()
    }

    fn shape(&self, x: i32, y: i32) -> Option<u8> {
        let t = self.turns.spatial.terrain.as_ref()?;
        let [w, h] = self.turns.spatial.map;
        (0 <= x && 0 <= y && x < w && y < h)
            .then(|| t.shapes.get((y * w + x) as usize).copied())
            .flatten()
    }

    fn allied(&self, a: usize, b: usize, with_4: bool) -> bool {
        a == b
            || (b != 0
                && self.turns.players.get(a).is_some_and(|p| {
                    matches!(p.relations.get(b), Some(0))
                        || (with_4 && p.relations.get(b) == Some(&4))
                }))
    }

    fn objects(&self, x: i32, y: i32) -> Result<Vec<Near>, Unsupported> {
        let t = self.turns;
        t.spatial
            .objects_on(x, y)
            .map(|(id, placed)| {
                let (owner, unit) = t.object(id).ok_or(Unsupported(
                    "an object near a spawn point the slice does not update",
                ))?;
                let master = t.master_of(owner, unit.life.master_id)?;
                Ok(Near {
                    id,
                    position: placed.position,
                    radius: placed.radius,
                    tall: placed.tall,
                    kind: master.kind,
                    class: placed.class,
                    master: placed.master,
                    buildable_on: placed.buildable_on,
                    obstruction_type: placed.obstruction_type,
                    auto_gather: master.auto_gather,
                    state: unit.life.state,
                    desolid: unit.construction.is_some_and(|c| c.desolid),
                    player: owner,
                    moving: master.moving.as_ref().is_some_and(|m| 0.0 < m.speed),
                    fog_visibility: master.fog_visibility,
                    fog_mask: t.spatial.sight.as_ref().and_then(|v| {
                        let umv = v.seer(id).map_or(0, |s| s.umv);
                        (50 <= master.kind && umv != 0).then(|| {
                            v.registry
                                .as_ref()
                                .and_then(|r| r.entry(id))
                                .map_or(0, |e| e.mask)
                        })
                    }),
                    holds: 0.0 < unit.life.amount
                        && matches!(unit.life.attribute_type, 0..=3 | 15..=17),
                    hp: unit.life.hp,
                    forest_tile: t.spatial.terrain.as_ref().is_some_and(|ter| {
                        ter.kind(placed.position[0] as i32, placed.position[1] as i32)
                            .and_then(|k| ter.terrains.get(usize::from(k)))
                            .is_some_and(|k| k.forest)
                    }),
                })
            })
            .collect()
    }
}

/// The attribute that scales research times above 0 (`+0x2b0`: 86).
const RESEARCH_TIME_MODIFIER: i16 = 86;
/// The attribute whose value above 0 gives the world's effect after a research (`+0x868`: 269).
const WORLD_EFFECT: i16 = 269;

impl Turns {
    /// The building update's production step (`FUN_1411e7dc0`, after its combat update) for the
    /// building at `i` of player `p`: with a queue and no make action, a unit's make action comes
    /// (its constructor `FUN_140acb140` gives the building's unit AI action 619 with
    /// `FUN_140c64b50`; the list's add starts it, slot 0x28 `FUN_140acf0a0`: attribute 81 grows by
    /// the unit's population, state 6); then the make action's update (`0x140ad0490`) works. A
    /// unit that is due spawns ([`Turns::spawn`]); its action ends (state 1, its destructor
    /// `FUN_140ace3f0` notifies the building's unit AI 506 with 619 again) and the queue lets it
    /// go (attribute 80 lowers by its population). A building whose population test fails
    /// ([`housed`]) waits without a make action. Not modelled: a computer player's housed
    /// building (it cancels its queue, `FUN_1411cff00`), the computer players' list, a research
    /// in the queue or the work-rate bonuses of `0x140ad0490` (`FUN_140c884c0`'s game mode, the
    /// AI difficulty under `FUN_141659cf0`).
    pub(crate) fn produce(
        &mut self,
        p: usize,
        i: usize,
        dt_seconds: f32,
    ) -> Result<(), Unsupported> {
        let unit = &self.players[p].objects[i];
        let Some(production) = unit.production.as_ref() else {
            return Ok(());
        };
        if 2 < unit.life.state {
            return Err(Unsupported(
                "a dying building's production (its list's teardown)",
            ));
        }
        let Some(front) = production.queue.front() else {
            return Ok(());
        };
        if front.tech != -1 {
            return self.research_step(p, i, dt_seconds);
        }
        let master = front.master;
        let pop = population(&self.master_of(p, master)?.storage);
        if !production.action {
            if housed(&self.players[p].attributes, pop)? {
                // the building waits: its reason (`+0xc02`, 0x20 or 4) and the events 0x7c and
                // 0x7d are the interface's; a computer player's cancels its queue
                if self.players[p].computer {
                    return Err(Unsupported(
                        "a computer player's housed building cancels its queue (FUN_1411cff00)",
                    ));
                }
                return Ok(());
            }
            let unit = &mut self.players[p].objects[i];
            if let Some(ai) = unit.ai.as_mut() {
                ai.set_action(MAKE_ACTION);
            }
            if let Some(production) = unit.production.as_mut() {
                production.action = true;
                // the queue paid for the unit (FUN_1411e7dc0 makes it with 0): `+0x60` set
                production.make_paid = true;
            }
            self.players[p].add_attribute(IN_PRODUCTION, pop)?;
        }
        let due = self.players[p].objects[i]
            .production
            .as_mut()
            .and_then(|production| production.work(dt_seconds).map(|q| q.master));
        let Some(master) = due else {
            return Ok(());
        };
        self.spawn(p, i, master, pop)?;
        let building = &mut self.players[p].objects[i];
        let state = building.life.state;
        let id = building.id;
        if let Some(ai) = building.ai.as_mut() {
            ai.post(
                Notify::own(id, ACTION_COMPLETED, [MAKE_ACTION, 0, 0]),
                state,
            )?;
        }
        if let Some(production) = building.production.as_mut() {
            production.spawned();
        }
        self.players[p].add_attribute(QUEUED, -pop)
    }

    /// The production step for a research at the front of the queue of the building at `i` of
    /// player `p` (`FUN_1411e7dc0`). Without a make action the research's comes
    /// (`TRIBE_Action_Make_Tech`, constructor `FUN_140ad0700`; its effect's first preparation,
    /// `FUN_1413d1530`, loads graphics and marks the masters an upgrade turns into, `+0x330`:
    /// the save does not hold the mark, and its readers, the object search `FUN_1411650e0` and
    /// the masters' slot 0xc0, are not modelled) and
    /// starts (slot 0x28, `0x140ad1d80`): its start test
    /// ([`crate::tech::TechState::start_test`] against
    /// what the queue entry paid) puts the technology in state 2 at the building's master, the
    /// action in state 6; a failed test (its refund, `FUN_1413d15e0`) is not modelled. Then its
    /// update (slot 0x30, `0x140ad2a20`) in state 6: the building's work rate (its powerup and
    /// its master's rate) times dt adds to the progress, 32767 once the technology was
    /// researched 0x100 times, which the technology's work done takes; a repeatable technology
    /// or one of the shared kind turns to state 2 first. The research is done once the progress
    /// reaches the location's time, times the research time modifier (attribute 86, above 0 and
    /// not an age), plus its adjustment, which is not modelled (`FUN_1413d1740`). Not held: the
    /// start time (`+0x74`), the human player's sound; not modelled: a computer player's
    /// handicaps and the game mode `FUN_140c884c0`'s instant research (0 for this recording, as
    /// for the make action of a unit).
    fn research_step(&mut self, p: usize, i: usize, dt_seconds: f32) -> Result<(), Unsupported> {
        use crate::tech::Attributes;
        if self.players[p].computer {
            return Err(Unsupported("a computer player's research (its handicaps)"));
        }
        let owner = &self.players[p];
        let building = &owner.objects[i];
        let production = building
            .production
            .as_ref()
            .expect("the caller's production");
        let front = production.queue.front().expect("the caller's front");
        let t = usize::try_from(front.tech)
            .ok()
            .filter(|&t| t < owner.techs.len())
            .ok_or(Unsupported(
                "a research of a technology the tree has not got",
            ))?;
        let def = self
            .techs
            .get(t)
            .ok_or(Unsupported("a technology the data file has not got"))?;
        let master = building.life.master_id;
        let tech = front.tech;
        let get = |k: i16| owner.get(k);
        let factors = crate::tech::factors(tech, &get)?;
        if !production.action {
            if !owner.techs[t].start_test(def, factors, &front.costs, &get)? {
                return Err(Unsupported(
                    "a research whose start test fails (its refund)",
                ));
            }
            let owner = &mut self.players[p];
            owner.techs[t].state = 2;
            owner.techs[t].building_master = master;
            let production = owner.objects[i].production.as_mut().expect("above");
            production.action = true;
            production.progress = 0.0;
        }
        let owner = &self.players[p];
        let state = &owner.techs[t];
        let repeatable = state.repeatable(def, false);
        let shared = def.kind == crate::tech::SHARED_KIND;
        let time_modifier = owner.get(RESEARCH_TIME_MODIFIER)?;
        let location = state
            .location(master)
            .copied()
            .ok_or(Unsupported("a research at a technology without locations"))?;
        let times = state.times_researched;
        let owner = &mut self.players[p];
        let production = owner.objects[i].production.as_mut().expect("above");
        let gain = production.rate * dt_seconds;
        let state = &mut owner.techs[t];
        if state.state != 2 && (repeatable || shared) {
            state.state = 2;
        }
        production.progress += gain;
        if 0xff < times {
            production.progress = 32767.0;
        }
        state.research_done = production.progress;
        let mut time = f32::from(location.time as i16);
        if !crate::tech::is_age(tech) && 0.0 < time_modifier {
            time *= time_modifier;
        }
        if location.adjustment + time <= production.progress {
            if 0xff < times {
                return Err(Unsupported("a research done 0x100 times (set_state 1)"));
            }
            return self.research_done(p, i, t, master, repeatable);
        }
        Ok(())
    }

    /// The research's end in its update (`0x140ad2a20`) once its progress reaches its time:
    /// `FUN_1413d1740` (with `repeat`, the player's `+0x648` or [`TechState::repeatable`]): a
    /// technology in state 2 (or with `repeat`) whose work done reaches the `.dat`'s time at the
    /// building (−1 without), times the research time modifier (attribute 86, above 0, not an
    /// age), plus the location's adjustment, is done ([`Turns::complete_research`]); else, or
    /// without `repeat`, it is left. Then the action ends (`set_state` 1; the event 100 and the
    /// computer players' slot 0x3a8 are not held), the building's unit AI is told 0x1fa with the
    /// action value 0x26c, and the production step lets the research go from the queue (the
    /// action list deletes it, the entry's count lowers). Attribute 269 above 0 (the world's
    /// effect `+0x128` slot 0x10) is not modelled.
    ///
    /// [`TechState::repeatable`]: crate::tech::TechState::repeatable
    fn research_done(
        &mut self,
        p: usize,
        i: usize,
        t: usize,
        master: i16,
        repeat: bool,
    ) -> Result<(), Unsupported> {
        use crate::tech::Attributes;
        let owner = &self.players[p];
        let state = &owner.techs[t];
        let def = &self.techs[t];
        if state.state == 2 || repeat {
            let mut time = def
                .locations
                .iter()
                .find(|l| l.0 == master)
                .map_or(-1.0, |l| f32::from(l.1));
            let modifier = owner.get(RESEARCH_TIME_MODIFIER)?;
            if !crate::tech::is_age(t as i16) && 0.0 < modifier {
                time *= modifier;
            }
            let adjustment = state.location(master).map_or(0.0, |l| l.adjustment);
            if repeat || time + adjustment <= state.research_done {
                let _ = self.complete_research(p, t, Some(master))?;
            }
        }
        if 0.0 < self.players[p].get(WORLD_EFFECT)? {
            return Err(Unsupported("attribute 269's world effect after a research"));
        }
        let building = &mut self.players[p].objects[i];
        let (id, state) = (building.id, building.life.state);
        if let Some(ai) = building.ai.as_mut() {
            ai.post(
                Notify::own(id, ACTION_COMPLETED, [crate::turn::RESEARCH_ACTION, 0, 0]),
                state,
            )?;
        }
        if let Some(production) = building.production.as_mut() {
            production.spawned();
        }
        Ok(())
    }

    /// `FUN_140ad0240`: the unit of `master` spawns beside the building at `i`. The spawn point
    /// ([`spawn_point`], from `FUN_140ace630`), then the master's slot 0x40 at the building's
    /// height (the ground's then, [`Turns::combat_object`]). Then attribute 81 lowers by its
    /// population, the building notifies its unit AI 506
    /// with 619, and the gather point's work comes (`FUN_140ace9c0`): `+0x7f9` (waiting to move)
    /// set, the work command on the gather point's target at the point (slot 0x128 with 1). A
    /// building without a gather point (`+0xc30` 0) spawns the unit idle, its spot facing nothing;
    /// one whose unit AI holds a command queue (slot 0x218) is not modelled.
    ///
    /// Not held: its facet and angle, its powerup offset, the building's `+0xc1c` group (unsaved), the game's events
    /// (`FUN_140c898a0`), the player's queue counts (`+0x18`, `FUN_14104f330`) and per-master
    /// counts (slot 0x398's `+0x13b0`), the sounds and particles (`FUN_1411a52e0`).
    fn spawn(&mut self, p: usize, i: usize, master: i16, pop: f32) -> Result<(), Unsupported> {
        if self.players[p].computer {
            return Err(Unsupported(
                "a computer player's new unit (its AI's slot 0x398)",
            ));
        }
        if self.spatial.sight.is_none() {
            return Err(Unsupported("a spawn without the players' sight"));
        }
        let building = &self.players[p].objects[i];
        let made = self.master_of(p, master)?.clone();
        if made.recyclable {
            return Err(Unsupported(
                "a new object of a recyclable master (a negative id)",
            ));
        }
        // the building unit AI's command queue (`+0x1f0`, saved as `savedOrder`): without a
        // gather point its first location is the spot's goal (FUN_1411d12d0, slot 0x220)
        let queue = building
            .ai
            .as_ref()
            .map(|a| a.building_orders.clone())
            .unwrap_or_default();
        let gather = building.gather_point;
        let position = building.position;
        let toward = match (gather, queue.first()) {
            (None, Some(first)) => Some([first.point[0], first.point[1]]),
            _ => gather
                .filter(|g| g.target != building.id)
                .map(|g| [g.point[0], g.point[1]]),
        };
        if made.moving.is_none() {
            return Err(Unsupported("a new unit of a master that does not move"));
        }
        let point = self.spot_beside(p, i, &made, toward)?;
        let new = self.combat_object(p, master, &made, [point[0], point[1], position[2]])?;
        let owner = &mut self.players[p];
        owner.add_attribute(IN_PRODUCTION, -pop)?;
        let (building_id, state) = (owner.objects[i].id, owner.objects[i].life.state);
        if let Some(ai) = owner.objects[i].ai.as_mut() {
            ai.post(
                Notify::own(building_id, ACTION_COMPLETED, [MAKE_ACTION, 0, 0]),
                state,
            )?;
        }
        if !queue.is_empty() {
            // FUN_140ace9c0 with a command queue: the building unit AI's slot 0x200 hands it to
            // the new unit (FUN_140c15e50), its gather point's work skipped
            self.hand_queue(p, new, &queue, position[2])?;
        } else if let Some(gather) = gather {
            self.gather(p, new, gather, building_id, true, true)?;
        }
        // FUN_140ace760: the new unit's tasks that spawn more (action type 0x9c)
        let now = self.players[p].objects[new].life.master_id;
        if self
            .master_of(p, now)?
            .tasks
            .iter()
            .any(|t| t.action_type == 0x9c)
        {
            return Err(Unsupported("a new unit's task that spawns more units"));
        }
        Ok(())
    }

    /// `FUN_140c15e50`: each order of a building's command queue goes into the unit AI's queue of
    /// the new unit at `new` (slot 0x30 into slot 0x28: not immediate, behind, priority 100,
    /// ready; its point the order's x and y at the building's height `z`, range 0). An order on
    /// a monastery (master 104) of a unit whose AI already queues one of target type 42 keeps its
    /// kind; on another object the unit may enter (`FUN_14118cd80`: type 70 and up with a
    /// capacity, `FUN_14120eb10` with its room) it would become an enter order (`0x2cd`), not
    /// modelled. The shown point (the game passes stack bytes there) is not held.
    fn hand_queue(
        &mut self,
        p: usize,
        new: usize,
        queue: &[crate::unit_ai::QueuedOrder],
        z: f32,
    ) -> Result<(), Unsupported> {
        /// The monastery, which a unit carrying a relic's order keeps (`FUN_141188e10`).
        const MONASTERY: i16 = 104;
        /// The target type whose queued order keeps a monastery's (`FUN_140c417f0` with 0x2a).
        const RELIC_TYPE: i32 = 0x2a;
        let master = self.players[p].objects[new].life.master_id;
        for o in queue {
            let target = (o.target != -1)
                .then(|| self.object(o.target))
                .flatten()
                .map(|(tp, t)| (tp, t.life.master_id));
            if let Some((tp, tm)) = target {
                let ai = self.players[p].objects[new].ai.as_ref();
                let keeps = tm == MONASTERY
                    && ai.is_some_and(|ai| ai.orders.iter().any(|q| q.target_type == RELIC_TYPE));
                let t_master = self.master_of(tp, tm)?;
                if !keeps
                    && 70 <= t_master.kind
                    && t_master.garrison != 0
                    && self.takes_in(o.target, p, master)?
                {
                    return Err(Unsupported(
                        "a command queue's order on a building the unit may enter (its room, slot 0x198)",
                    ));
                }
            }
            let unit = &mut self.players[p].objects[new];
            let state = unit.life.state;
            if let Some(ai) = unit.ai.as_mut() {
                let order = crate::unit_ai::QueuedOrder {
                    priority: 100,
                    point: [o.point[0], o.point[1], z],
                    range: 0.0,
                    ready: true,
                    ..*o
                };
                let _ = ai.queue_order(state, order, false, false)?;
            }
        }
        Ok(())
    }

    /// The master's slot 0x40 for a combat unit (type 70, `0x141266120`, the constructor
    /// `FUN_1411eae40` with its setups) of `master` for player `p` at `position`: the setups in
    /// order (static `FUN_1411a45a0`, animated `FUN_141072690`, moving `FUN_141161450`, combat
    /// `FUN_14121df60`) with their draws: the facet (Object stream, for a standing sprite with a
    /// random first frame and several facets), the standing sprite ([`ActiveSprite::start`],
    /// Sprite stream), the angle (Object stream, sequence flag 2), the unit AI
    /// ([`UnitAi::constructed`], three AI-stream draws) and the powerup offset (Object stream).
    /// The object takes its place (its height the ground's, by its master's fly mode), its record
    /// and mesh shape, its sight and its storage (flags 1, 2 and 32, `FUN_141193860`); its
    /// player's attributes 98 and 164 (and 246 under [`Turns::unit_cost_246`]) grow by its paid
    /// cost and a villager's (class 4) attribute 248 by one. It joins its player's list last; its
    /// index there.
    pub(crate) fn combat_object(
        &mut self,
        p: usize,
        master: i16,
        made: &crate::masters::Master,
        position: [f32; 3],
    ) -> Result<usize, Unsupported> {
        if self.players[p].computer {
            return Err(Unsupported(
                "a computer player's new unit (its AI's slot 0x398)",
            ));
        }
        if self.spatial.sight.is_none() {
            return Err(Unsupported("a spawn without the players' sight"));
        }
        if made.recyclable {
            return Err(Unsupported(
                "a new object of a recyclable master (a negative id)",
            ));
        }
        let moving = made
            .moving
            .ok_or(Unsupported("a new unit of a master that does not move"))?;
        // the constructor's new_position: the ground's height by the master's fly mode (slot
        // 0x60, FUN_141193a10)
        let position = self.spatial.terrain.as_ref().map_or(position, |t| {
            t.ground(position[0], position[1]).map_or(position, |g| {
                [
                    position[0],
                    position[1],
                    crate::missile::grounded(position[2], g, false, made.fly_mode),
                ]
            })
        });
        let ai_type = factory_type(made.class, master, made.missile)?
            .ok_or(Unsupported("a new combat unit without a unit AI"))?;
        let standing = (0 <= made.standing_sprite)
            .then(|| self.graphics.get(&made.standing_sprite).copied())
            .flatten()
            .ok_or(Unsupported(
                "a new unit's standing sprite the data file does not have",
            ))?;
        let id = self.next_object_id;
        self.next_object_id += 1;
        // the setups' draws
        if standing.flags & 4 != 0 && 1 < standing.facets {
            let _ = self.streams.get(Stream::Object).rand15();
        }
        let sprite = ActiveSprite::start(standing, self.streams.get(Stream::Sprite));
        let facets = crate::facing::SpriteFacets {
            count: standing.facets,
            flag_1: standing.flags & 2 != 0,
        };
        let angle = if crate::facing::draws_angle(Some(facets)) {
            let r = self.streams.get(Stream::Object).rand15();
            crate::facing::drawn_angle(r, facets)
        } else {
            0.0
        };
        let ai = UnitAi::constructed(ai_type, position, self.streams.get(Stream::Ai))?;
        let _ = self.streams.get(Stream::Object).rand15();
        // its place, record, mesh shape and sight (new_position, slot 0x58, and FUN_141054fe0)
        let solid = 0.0 < made.radius[0] && 0.0 < made.radius[1] && made.tall;
        let placed = Placed {
            position,
            radius: made.radius,
            class: made.class,
            player: p,
            master,
            obstruction_type: made.obstruction_type,
            buildable_on: made.buildable_on,
            tall: made.tall,
            speed: moving.speed,
            unknown: None,
            obstruction: (made.obstruction_type != 0 && solid).then_some(Obstruction {
                kind: made.obstruction_type,
                class: made.obstruction_class,
            }),
        };
        self.spatial.insert(id, placed);
        self.pathing.moved(id, &placed, false);
        self.pathing.tiles_changed(self.spatial.take_changes());
        if let Some(v) = self.spatial.sight.as_mut() {
            v.add_seer(
                id,
                Seer {
                    player: p,
                    master,
                    los: made.line_of_sight,
                    state: 2,
                    explored: false,
                    building: None,
                    umv: 0,
                    capture: capture_kind(p, made.class, master),
                    kind: made.kind,
                },
            );
            let _ = v.sight_on(id, position, made.radius, false, false);
        }
        if let Some(e) = self.spatial.take_sight_unsupported() {
            return Err(e);
        }
        let owner = &mut self.players[p];
        for &(kind, amount, flag) in &made.storage {
            if kind >= 0 && matches!(flag, 1 | 2 | 32) {
                owner.add_attribute(kind, amount)?;
            }
        }
        let cost: i32 = made
            .cost
            .iter()
            .filter(|&&(resource, _, paid)| resource >= 0 && paid)
            .map(|&(_, amount, _)| i32::from(amount))
            .sum();
        owner.add_attribute(98, cost as f32)?;
        owner.add_attribute(164, cost as f32)?;
        if self.unit_cost_246 {
            owner.add_attribute(246, cost as f32)?;
        }
        if made.class == VILLAGER {
            owner.add_attribute(248, 1.0)?;
        }
        owner.objects.push(Unit {
            id,
            position,
            life: Lifecycle {
                class: Class::Animated,
                state: 2,
                hp: f32::from(made.hit_points),
                attribute_type: -1,
                amount: 0.0,
                dying_sprite: (0 <= made.death_sprite).then_some(made.death_sprite),
                death_spawn: 0 <= made.death_spawns[0],
                remains: made.death_spawns[1],
                death_facets: None,
                inside: None,
                goto_sleep: false,
                master_id: master,
                master: made.life,
            },
            ai: Some(ai),
            // FUN_1411580a0: facing x (an immediate 1.0), its right -z, not waiting
            mover: Some(Mover {
                forward: [1.0, 0.0, 0.0],
                right: [0.0, 0.0, START_RIGHT_Z],
                velocity: None,
                paths: Vec::new(),
                factor: 1.0,
                stats: moving,
                waiting: 0,
                wait_delays: 0,
                attack_moving: false,
                last_move: self.world_time,
                moved_at: position,
                substitute_count: 0,
                ignore_units: Vec::new(),
                last_turn: 0.0,
                angle,
                move_to_point: None,
                speed_stopped: false,
                pauses: Vec::new(),
                in_the_way: Vec::new(),
                sprite_facets: Some(facets),
            }),
            attackable: true,
            pasture: -1,
            my_pasture: -1,
            combat: true,
            // the TRIBE combat setup (FUN_14121df60): the master's volley fire
            firepower: made.volley_fire,
            sprite: Some(sprite),
            ..Unit::default()
        });
        Ok(owner.objects.len() - 1)
    }

    /// `FUN_1411d12d0` for a unit of `made` beside the building at `i` of player `p`, `toward`
    /// the point the search faces (its gather point's, or the command's): [`spawn_point`] with
    /// the walls at the building's corners (class 27 on both tiles beside each) and the master's
    /// placement test at each candidate ([`placement`]).
    pub(crate) fn spot_beside(
        &self,
        p: usize,
        i: usize,
        made: &crate::masters::Master,
        toward: Option<[f32; 2]>,
    ) -> Result<[f32; 2], Unsupported> {
        let building = &self.players[p].objects[i];
        let site_master = self.master_of(p, building.life.master_id)?;
        let position = building.position;
        let mut walls = [false; 4];
        for (wall, tiles) in
            walls
                .iter_mut()
                .zip(corner_tiles(position, site_master.radius, made.radius))
        {
            for &(x, y) in &tiles {
                let (fx, fy) = (x as f32, y as f32);
                self.spatial.check_known([fx, fy, fx + 1.0, fy + 1.0])?;
            }
            *wall = tiles
                .iter()
                .all(|&(x, y)| self.spatial.object_on_tile(x, y, |o| o.class == WALL));
        }
        let site = Site {
            position,
            radius: site_master.radius,
            gather: toward,
            walls,
        };
        let moving = made.moving.ok_or(Unsupported(
            "a unit beside a building of a master that does not move",
        ))?;
        let restriction = self
            .spatial
            .terrain
            .as_ref()
            .and_then(|t| t.restrictions.get(moving.restriction))
            .ok_or(Unsupported(
                "a unit's terrain restriction the data file does not have",
            ))?;
        let placing = Placing {
            player: p,
            radius: made.radius,
            tall: made.tall,
            class: made.class,
            restriction,
            hill_mode: made.hill_mode,
            placement: made.placement,
        };
        let map = WorldMap { turns: self };
        let spatial = &self.spatial;
        spawn_point(&site, made.radius, &mut |[x, y]| {
            let [rx, ry] = made.radius;
            spatial.check_known([x - rx, y - ry, x + rx, y + ry])?;
            placement(&placing, &map, [x, y], false)
        })
    }

    /// `FUN_1411a4e90`: the remains a dying sprite's end leaves for player `p`: the master's slot
    /// 0x40 makes an object of `master` at the dead object's place `at`, with the setups' draws
    /// (static `FUN_1411a45a0`: its standing sprite, [`ActiveSprite::start`] on the Sprite stream,
    /// then its facet on the Object stream for a sprite with a random first frame and several
    /// facets; moving `FUN_141161450`: its angle on the Object stream, sequence flag 2), its id
    /// (`FUN_14152fd70`), its place, record and mesh shape (slot 0x58), its sight and its storage
    /// (flags 1, 2 and 32 to its owner, `FUN_141193860`; the last entry of flag 0 held,
    /// `FUN_141193710`); it takes the dead object's facet (`FUN_14119d490`, `FUN_14114d780`). It
    /// joins the owner's list and updates later in the same tick. Remains of type 10 (a static
    /// object, `FUN_141126300` -> `FUN_141169580`: no moving setup, its death on the sprite after
    /// it with a random facet), of types 20 to 39 without hit points (with hit points it would
    /// sleep) and of type 70 (a combat unit, [`Turns::combat_object`]: a placeholder villager's
    /// death spawn, masters 1875 and 1876, is a villager) are modelled; not held: its facet,
    /// angle and sprite.
    pub(crate) fn remains(
        &mut self,
        p: usize,
        master: i16,
        at: [f32; 3],
        facet: Option<u8>,
    ) -> Result<(), Unsupported> {
        let made = self.master_of(p, master)?.clone();
        if made.kind == BUILDING {
            return self.new_building(p, &made, at);
        }
        if made.kind == TRIBE_COMBAT {
            let new = self.combat_object(p, master, &made, at)?;
            // a moving dead object's override (FUN_14119d490, FUN_14114d780): the new unit takes
            // its facet's angle
            if let Some(facet) = facet {
                let standing = self.graphics.get(&made.standing_sprite).copied();
                let own = standing.map_or(
                    crate::facing::SpriteFacets {
                        count: 0,
                        flag_1: false,
                    },
                    |g| crate::facing::SpriteFacets {
                        count: g.facets,
                        flag_1: g.flags & 2 != 0,
                    },
                );
                if let Some(m) = self.players[p].objects[new].mover.as_mut() {
                    m.angle = crate::facing::angle_of_facet(facet, own);
                }
            }
            return Ok(());
        }
        if made.kind != 10 && !(20..40).contains(&made.kind) {
            return Err(Unsupported("remains of a type other than 10 and 20 to 39"));
        }
        if 0 < made.hit_points {
            return Err(Unsupported("remains with hit points (the sleeping list)"));
        }
        if made.recyclable {
            return Err(Unsupported(
                "a new object of a recyclable master (a negative id)",
            ));
        }
        let id = self.next_object_id;
        self.next_object_id += 1;
        let standing = (0 <= made.standing_sprite)
            .then(|| self.graphics.get(&made.standing_sprite).copied())
            .flatten();
        if let Some(g) = standing {
            let _ = ActiveSprite::start(g, self.streams.get(Stream::Sprite));
            if g.flags & 4 != 0 && 1 < g.facets {
                let _ = self.streams.get(Stream::Object).rand15();
            }
        }
        let mut angle = 0.0;
        let moving = if 30 <= made.kind {
            // the moving setup's angle draw (FUN_141161450), whose angle the dead object's facet
            // replaces: that facet's angle (FUN_14119d490, FUN_14114d780, with the remains' own
            // sprite's facets)
            if standing.is_some_and(|g| g.flags & 2 != 0 && 1 < g.facets) {
                let _ = self.streams.get(Stream::Object).rand15();
            }
            let facet = facet.ok_or(Unsupported(
                "moving remains of an object without a facet the slice holds",
            ))?;
            let own = standing.map_or(
                crate::facing::SpriteFacets {
                    count: 0,
                    flag_1: false,
                },
                |g| crate::facing::SpriteFacets {
                    count: g.facets,
                    flag_1: g.flags & 2 != 0,
                },
            );
            angle = crate::facing::angle_of_facet(facet, own);
            Some(
                made.moving
                    .ok_or(Unsupported("moving remains without a moving master"))?,
            )
        } else {
            None
        };
        let solid = 0.0 < made.radius[0] && 0.0 < made.radius[1] && made.tall;
        let placed = Placed {
            position: at,
            radius: made.radius,
            class: made.class,
            player: p,
            master,
            obstruction_type: made.obstruction_type,
            buildable_on: made.buildable_on,
            tall: made.tall,
            speed: moving.as_ref().map_or(0.0, |m| m.speed),
            unknown: None,
            obstruction: (made.obstruction_type != 0 && solid).then_some(Obstruction {
                kind: made.obstruction_type,
                class: made.obstruction_class,
            }),
        };
        self.spatial.insert(id, placed);
        self.pathing.moved(id, &placed, false);
        self.pathing.tiles_changed(self.spatial.take_changes());
        if let Some(v) = self.spatial.sight.as_mut() {
            v.add_seer(
                id,
                Seer {
                    player: p,
                    master,
                    los: made.line_of_sight,
                    state: 2,
                    explored: false,
                    building: None,
                    umv: 0,
                    capture: capture_kind(p, made.class, master),
                    kind: made.kind,
                },
            );
            let _ = v.sight_on(id, at, made.radius, false, false);
        }
        if let Some(e) = self.spatial.take_sight_unsupported() {
            return Err(e);
        }
        let owner = &mut self.players[p];
        let mut held = (-1, 0.0);
        for &(kind, amount, flag) in &made.storage {
            if kind >= 0 && matches!(flag, 1 | 2 | 32) {
                owner.add_attribute(kind, amount)?;
            }
            if kind != -1 && flag == 0 {
                held = (kind, amount);
            }
        }
        // a static object (type 10) dies on its sprite after its death (the dying one, else its
        // standing one), which may start on a random facet
        let death_facets = if made.kind == 10 {
            let after = if 0 <= made.death_sprite {
                made.death_sprite
            } else {
                made.standing_sprite
            };
            self.graphics
                .get(&after)
                .filter(|g| g.flags & 4 != 0 && 1 < g.facets)
                .map(|g| g.facets)
        } else {
            None
        };
        owner.objects.push(Unit {
            id,
            position: at,
            life: Lifecycle {
                class: if made.kind == 10 {
                    Class::Static
                } else {
                    Class::Animated
                },
                state: 2,
                hp: f32::from(made.hit_points),
                attribute_type: held.0,
                amount: held.1,
                dying_sprite: (0 <= made.death_sprite).then_some(made.death_sprite),
                death_spawn: 0 <= made.death_spawns[0],
                remains: made.death_spawns[1],
                death_facets,
                inside: None,
                goto_sleep: false,
                master_id: master,
                master: made.life,
            },
            mover: moving.map(|stats| Mover {
                forward: [1.0, 0.0, 0.0],
                right: [0.0, 0.0, START_RIGHT_Z],
                velocity: None,
                paths: Vec::new(),
                factor: 1.0,
                stats,
                waiting: 0,
                wait_delays: 0,
                attack_moving: false,
                last_move: self.world_time,
                moved_at: at,
                substitute_count: 0,
                ignore_units: Vec::new(),
                last_turn: 0.0,
                angle,
                move_to_point: None,
                speed_stopped: false,
                pauses: Vec::new(),
                in_the_way: Vec::new(),
                sprite_facets: standing.map(|g| crate::facing::SpriteFacets {
                    count: g.facets,
                    flag_1: g.flags & 2 != 0,
                }),
            }),
            attackable: true,
            pasture: -1,
            my_pasture: -1,
            ..Unit::default()
        });
        Ok(())
    }

    /// FLARE's objects (`0x140e7bfd0`): with `players` at most the world's players, each player
    /// from 1 below `players` makes a flare (master 274, its slot 0x1c0 `FUN_14162ce00` with the
    /// footprint left alone, then the master's slot 0x40 as [`Turns::remains`] makes it) at
    /// `target`'s place, or at `point` on the ground when `target` is −1 or gone; its workers
    /// (`+0x644`) are 0. Not held: the sender's flare's workers 1 when the local player is among
    /// the recipients (`+0x11`..), a computer player's AI memory of it (`+0x7a8`'s `+0xbc00`),
    /// the sound and the minimap.
    pub(crate) fn flare(
        &mut self,
        target: i32,
        point: [f32; 2],
        players: u8,
    ) -> Result<(), Unsupported> {
        /// The flare's master (`0x112`, FLARE2).
        const FLARE: i16 = 274;
        if self.players.len() < usize::from(players) {
            return Ok(());
        }
        let at = if target == -1 {
            None
        } else {
            self.spatial.get(target).map(|p| p.position)
        };
        let at = at.unwrap_or([point[0], point[1], 0.0]);
        for p in 1..usize::from(players) {
            self.remains(p, FLARE, at, None)?;
        }
        Ok(())
    }

    /// A death spawn or the remains of a building master: its slot 0x40 (`FUN_14123d1a0`) makes a
    /// building at `at` (`FUN_1411ac5c0` with 1, so its init `FUN_1411de5f0`): the combat setup
    /// (`FUN_14121df60`) with its draws, the unit AI's three ([`UnitAi::constructed`]; the
    /// building's) and the powerup offset's (`FUN_1408117b0`, Object stream); a building adds its
    /// paid cost to attribute 247 only with a train location, none here without a cost. Then type
    /// 80 and `set_state(0)`. Without a build time above 0 it keeps its master's hit points and
    /// holds 0 build points (`+0xc50`), so its first update, later in the same tick, completes it
    /// ([`crate::build::Construction::complete`] from the building's update). Its footprint
    /// clearing (`FUN_1415c3620`) finds nothing without radii. It holds a gate's lock, open.
    ///
    /// Not modelled: a build time above 0 (the construction sprite, one hit point and the
    /// desolid footprint, `FUN_1411cf770`), radii, a standing sprite, annexes or an annex master,
    /// classes 51 and 54, masters 50, 70, 276, 357, 1187 and 1188, a paid cost, a computer
    /// player's, a building with sight, the dead object's `+0x6a8`, and its facet
    /// (`FUN_14119d490`). Its owner watches it ([`Turns::watch_started`]).
    fn new_building(
        &mut self,
        p: usize,
        made: &crate::masters::Master,
        at: [f32; 3],
    ) -> Result<(), Unsupported> {
        let master = made.id;
        let building = made.building.ok_or(Unsupported(
            "a new building of a master without its building part",
        ))?;
        if self.players[p].computer {
            return Err(Unsupported(
                "a computer player's new building (its AI's slot 0x398)",
            ));
        }
        if 0 < made.build_time || 0.0 < made.radius[0] || 0.0 < made.radius[1] {
            return Err(Unsupported(
                "a new building with a build time or radii (its construction, its footprint)",
            ));
        }
        if 0 <= made.standing_sprite || 0.0 < made.line_of_sight {
            return Err(Unsupported(
                "a new building with a standing sprite or sight (its setups' draws, its watch)",
            ));
        }
        if building.annexes.iter().any(|&a| 0 <= a) || 0 <= building.head_unit {
            return Err(Unsupported("a new building with annexes or of an annex"));
        }
        if matches!(made.class, 51 | 54) || matches!(master, 50 | 70 | 276 | 357 | 1187 | 1188) {
            return Err(Unsupported(
                "a new building of classes 51, 54 or masters 50, 70, 276, 357, 1187, 1188",
            ));
        }
        if made.recyclable
            || made
                .cost
                .iter()
                .any(|&(r, a, paid)| 0 <= r && paid && a != 0)
        {
            return Err(Unsupported(
                "a new building of a recyclable master or with a paid cost",
            ));
        }
        let ai_type = factory_type(made.class, master, made.missile)?;
        let id = self.next_object_id;
        self.next_object_id += 1;
        let ai = ai_type
            .map(|t| UnitAi::constructed(t, at, self.streams.get(Stream::Ai)))
            .transpose()?;
        // the combat setup's powerup offset (`+0xa54`)
        let _ = self.streams.get(Stream::Object).rand15();
        let placed = Placed {
            position: at,
            radius: made.radius,
            class: made.class,
            player: p,
            master,
            obstruction_type: made.obstruction_type,
            buildable_on: made.buildable_on,
            tall: made.tall,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        };
        self.spatial.insert(id, placed);
        if let Some(v) = self.spatial.sight.as_mut() {
            v.add_seer(
                id,
                Seer {
                    player: p,
                    master,
                    los: made.line_of_sight,
                    state: 0,
                    explored: false,
                    building: None,
                    umv: 0,
                    capture: capture_kind(p, made.class, master),
                    kind: made.kind,
                },
            );
        }
        if let Some(e) = self.spatial.take_sight_unsupported() {
            return Err(e);
        }
        self.players[p].objects.push(Unit {
            id,
            position: at,
            life: Lifecycle {
                class: Class::Animated,
                state: 0,
                hp: f32::from(made.hit_points),
                attribute_type: -1,
                amount: 0.0,
                dying_sprite: (0 <= made.death_sprite).then_some(made.death_sprite),
                death_spawn: 0 <= made.death_spawns[0],
                remains: made.death_spawns[1],
                death_facets: None,
                inside: None,
                goto_sleep: false,
                master_id: master,
                master: made.life,
            },
            ai,
            construction: Some(crate::build::Construction {
                progress: 0.0,
                desolid: false,
                builders: 0,
                hit_points: made.hit_points,
                build_time: made.build_time,
                record: Obstruction {
                    kind: made.obstruction_type,
                    class: made.obstruction_class,
                },
                snow: false,
                sprite_override: false,
                started: false,
            }),
            firepower: made.volley_fire,
            // the building's constructor sets its first update (`mFirstUpdate`)
            gate: Some(crate::turn::Gate {
                first_update: true,
                ..crate::turn::Gate::default()
            }),
            combat: true,
            attackable: true,
            pasture: -1,
            my_pasture: -1,
            ..Unit::default()
        });
        // the constructor's watch (`FUN_1411cc980`, `FUN_141090430`)
        self.watch_started(p, id)
    }

    /// `FUN_140ace9c0` with a gather point without a target, and an unloaded unit's move
    /// (`FUN_1411e7220`): the unit waits to move (slot 0x2a0 with 1) and takes slot 0x128
    /// (`FUN_141211b80`) with no target at the point (z 0; its `+0xa14` `notified`: 1 for a new
    /// unit, whose unit AI's order queue empties, `FUN_140c5e010`; 0 for an unloaded one); slot
    /// 0x7e8 (`FUN_141227190`) runs the work command (`FUN_14106ff70`), which finds no task: a
    /// unit with speed in state 2 notifies its unit AI with the `MoveTo`'s order and action, no
    /// target and the point, its notifies cleared with `+0xa14` ([`UnitAi::notify_work`], slot
    /// 0x828), and moves (slot 0x138, `FUN_141216f10`: a villager is no longer under the town bell,
    /// `+0x9dc`; with `+0xa14` the order queue empties again; `FUN_14106d340`: the notifies
    /// cleared with the world time, then, the unit holding no action, the `MoveTo` to the point,
    /// range 0, installed). The command count (`+0x888`) is not held.
    pub(crate) fn gather_move(
        &mut self,
        p: usize,
        new: usize,
        point: [f32; 3],
        notified: bool,
    ) -> Result<(), Unsupported> {
        let world_time = self.world_time;
        let villager = self
            .master_of(p, self.players[p].objects[new].life.master_id)?
            .class
            == VILLAGER;
        let unit = &mut self.players[p].objects[new];
        if let Some(m) = unit.mover.as_mut() {
            m.waiting = 1;
        }
        if unit.top_action_kind().is_some() {
            return Err(Unsupported(
                "a gather point's move of a unit with an action",
            ));
        }
        // a unit without speed takes nothing; one not in state 2 would notify without moving
        if unit.mover.as_ref().map_or(0.0, |m| m.stats.speed) <= 0.0 {
            return Ok(());
        }
        if unit.life.state != 2 {
            return Err(Unsupported(
                "a gather point's move of a unit not in state 2",
            ));
        }
        if let Some(ai) = unit.ai.as_mut() {
            if notified {
                ai.orders.clear();
            }
            ai.notify_work(
                crate::unit_ai::MOVE_ORDER,
                crate::unit_ai::MOVE_ACTION,
                None,
                point,
                notified,
                self.streams.get(Stream::Ai),
            )?;
            if notified {
                ai.orders.clear();
            }
            ai.clear_notifies(world_time)?;
        }
        if villager {
            unit.town_bell.flag = false;
        }
        unit.move_to = Some(crate::move_to::MoveToAction {
            run: false,
            state: crate::move_to::MoveTo::Start,
            goal: Some(point),
            target: -1,
            range: 0.0,
            wait: crate::move_to::MoveWait::default(),
        });
        Ok(())
    }

    /// `FUN_140ace9c0` with a gather point (`+0xc30` 1) and a building whose unit AI holds no
    /// command queue location (its slot 0x218): the new unit waits to move (slot 0x2a0 with 1,
    /// `+0x7f9`) and takes the work command on the target at the point (slot 0x128: z 0, its
    /// `+0xa14` 1, no carry test, any task). A target that is gone is looked for again
    /// ([`Turns::lost_gather_target`]); one on the building itself (a garrison) and a building
    /// target (slot 0x7e8) are not modelled.
    /// `FUN_140ace9c0`'s search for a gather point's target that is gone: the closest (by its
    /// centre, the first of equals) of Gaia's live objects of the class of Gaia's master the
    /// point names (`FUN_141166670` over `+0x30`), unless it lies farther than the new unit's
    /// line of sight (squared) from the point, measured from its footprint (`FUN_140a847c0`);
    /// then, but for the classes 5, 29 and 33 (the unit AI's slot 0x80, not modelled), the same
    /// over Gaia's sleeping objects (`+0x38`). None found: the unit moves to the point.
    fn lost_gather_target(
        &self,
        p: usize,
        new: usize,
        g: GatherPoint,
    ) -> Result<Option<i32>, Unsupported> {
        let class = self
            .masters
            .first()
            .and_then(|m| m.get(g.target_master))
            .map_or(-1, |m| m.class);
        let los = self
            .master_of(p, self.players[p].objects[new].life.master_id)?
            .line_of_sight;
        let [x, y, _] = g.point;
        let gaia = self
            .players
            .first()
            .ok_or(Unsupported("a world without Gaia"))?;
        let closest = |list: &[Unit]| -> Result<Option<i32>, Unsupported> {
            let mut best: Option<(&Unit, f32)> = None;
            for u in list {
                if self.master_of(0, u.life.master_id)?.class != class {
                    continue;
                }
                let (dx, dy) = (x - u.position[0], y - u.position[1]);
                let d = dy * dy + dx * dx;
                if best.is_none_or(|(_, b)| d < b) {
                    best = Some((u, d));
                }
            }
            let Some((u, _)) = best else {
                return Ok(None);
            };
            let [rx, ry] = self.master_of(0, u.life.master_id)?.radius;
            let gap = |d: f32, r: f32| {
                let g = d.abs() - r;
                if 0.0 <= g { g } else { 0.0 }
            };
            let (gx, gy) = (gap(x - u.position[0], rx), gap(y - u.position[1], ry));
            Ok((gy * gy + gx * gx <= los * los).then_some(u.id))
        };
        if let Some(id) = closest(&gaia.objects)? {
            return Ok(Some(id));
        }
        if matches!(class, 5 | 29 | 33) {
            return Err(Unsupported(
                "a gather point's lost target of class 5, 29 or 33 (the unit AI's slot 0x80)",
            ));
        }
        closest(&gaia.sleeping)
    }

    /// The new unit's walk to the gather point of its building `building` (`FUN_140ace9c0` with a
    /// point or a target): a point is a move; a target that is gone is looked for again
    /// ([`Turns::lost_gather_target`]). The unit waits to move (slot 0x2a0 with 1); on a building
    /// its own task list's garrison finder (filter 3, [`Turns::garrison_task_found`]) decides: a
    /// task found is worked at once (slot 0x7e8 with filter 3, outside slot 0x128: `+0xa14` 1);
    /// else, and for any other target, slot 0x128 works it (`notified`, any task). An unloaded
    /// unit's caller goes to slot 0x128 at once (`finder` clear). A gather point on its own
    /// building (a garrison) is not modelled.
    pub(crate) fn gather(
        &mut self,
        p: usize,
        new: usize,
        g: GatherPoint,
        building: i32,
        notified: bool,
        finder: bool,
    ) -> Result<(), Unsupported> {
        if g.target == -1 {
            return self.gather_move(p, new, [g.point[0], g.point[1], 0.0], notified);
        }
        if g.target == building {
            return Err(Unsupported(
                "a gather point on its own building (a garrison)",
            ));
        }
        let target = if self.object(g.target).is_some() {
            g.target
        } else {
            match self.lost_gather_target(p, new, g)? {
                Some(id) => id,
                None => {
                    return self.gather_move(p, new, [g.point[0], g.point[1], 0.0], notified);
                }
            }
        };
        let (owner, t) = self
            .object(target)
            .ok_or(Unsupported("unreachable: a gather point's target found"))?;
        let on_building = self.master_of(owner, t.life.master_id)?.kind == BUILDING;
        if let Some(m) = self.players[p].objects[new].mover.as_mut() {
            m.waiting = 1;
        }
        let garrison = finder && on_building && self.garrison_task_found(p, new, target)?;
        let work = Work {
            target,
            point: [g.point[0], g.point[1], 0.0],
            carry_test: false,
            filter: if garrison { 3 } else { -1 },
            notified: garrison || notified,
        };
        self.order_work(p, new, &work)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// records/425906646's update 1883: a TC's gather point on a tree that is gone; its new
    /// villager takes the closest tree of Gaia's (by centre) within its sight of the point,
    /// measured from the tree's footprint, else none (it walks to the point).
    #[test]
    fn a_gather_point_whose_tree_is_gone_takes_the_closest_tree_within_sight() {
        let unit = |id, master, x: f32, y: f32| Unit {
            id,
            position: [x, y, 0.0],
            life: Lifecycle {
                master_id: master,
                ..Lifecycle::default()
            },
            ..Unit::default()
        };
        let mut t = Turns::new(
            0,
            1,
            0,
            vec![0, 0],
            vec![
                crate::turn::Player {
                    sleeping: vec![unit(10, 349, 20.5, 20.5), unit(11, 349, 22.5, 20.5)],
                    ..crate::turn::Player::default()
                },
                crate::turn::Player {
                    objects: vec![unit(99, 83, 18.0, 18.0)],
                    ..crate::turn::Player::default()
                },
            ],
        );
        let tree = crate::masters::Master {
            id: 349,
            class: 15,
            radius: [0.5, 0.5],
            ..crate::masters::Master::default()
        };
        let villager = crate::masters::Master {
            id: 83,
            class: 4,
            line_of_sight: 4.0,
            ..crate::masters::Master::default()
        };
        t.masters = vec![
            crate::masters::Masters::new([tree.clone()]),
            crate::masters::Masters::new([villager, tree]),
        ];
        let g = GatherPoint {
            point: [21.0, 21.0, 0.0],
            target: 5,
            target_master: 349,
        };
        assert_eq!(t.lost_gather_target(1, 0, g).unwrap(), Some(10));
        // 7 tiles from the closest tree's footprint, beyond the villager's sight of 4
        let far = GatherPoint {
            point: [30.0, 21.0, 0.0],
            ..g
        };
        assert_eq!(t.lost_gather_target(1, 0, far).unwrap(), None);
    }

    /// MarvinBlue/426048731's TC 9149 at (104, 47), radii 2: its gather point (107.2451,
    /// 46.075863) lies right and below, so the right side's lower half (6) comes first among the
    /// sides the first pass tries; the villager (radius 0.2) steps by 0.5 from (106.5, 47.5).
    /// The sheep 9520 at (106.68, 47.15) blocks the first two candidates and stays (livestock in
    /// state 2), so each ends its spot; (106.5, 46.5) is free.
    /// A map of one tile's shape and elevation for the hill test.
    struct Hill(u8, u8);

    impl PlaceMap for Hill {
        fn size(&self) -> [i32; 2] {
            [10, 10]
        }
        fn terrain(&self, _: i32, _: i32) -> Result<u8, Unsupported> {
            Ok(0)
        }
        fn explored(&self, _: usize, _: i32, _: i32) -> bool {
            true
        }
        fn visible(&self, _: usize, _: i32, _: i32) -> bool {
            true
        }
        fn vision(&self, _: usize) -> [u64; 2] {
            [0; 2]
        }
        fn elevation(&self, _: i32, _: i32) -> Option<u8> {
            Some(self.1)
        }
        fn shape(&self, _: i32, _: i32) -> Option<u8> {
            Some(self.0)
        }
        fn allied(&self, a: usize, b: usize, _: bool) -> bool {
            a == b
        }
        fn objects(&self, _: i32, _: i32) -> Result<Vec<Near>, Unsupported> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn a_hill_mode_takes_flat_ground_side_slopes_or_an_edge_within_one_step() {
        let fails = |mode, shape, elevation, at| {
            hill_fails(mode, &Hill(shape, elevation), at, (2, 5), Some(3)).unwrap()
        };
        // mode 1: flat and the four side slopes (5 to 8) pass, corners and saddles fail
        assert!(!fails(1, 0, 3, (3, 3)) && !fails(1, 5, 3, (3, 3)) && !fails(1, 8, 3, (3, 3)));
        assert!(fails(1, 2, 3, (3, 3)) && fails(1, 9, 3, (3, 3)));
        // mode 2: flat alone
        assert!(!fails(2, 0, 3, (3, 3)) && fails(2, 6, 3, (3, 3)));
        // mode 3: the first column and the last row within one step of the corner's elevation
        assert!(!fails(3, 9, 4, (2, 3)) && fails(3, 0, 5, (2, 3)) && fails(3, 0, 1, (4, 5)));
        assert!(!fails(3, 0, 7, (4, 4)));
        // the other modes take any tile
        assert!(!fails(0, 12, 9, (3, 3)));
    }

    #[test]
    fn the_tc_spawns_its_villager_below_the_sheep_on_its_right() {
        let site = Site {
            position: [104.0, 47.0, 0.0],
            radius: [2.0, 2.0],
            gather: Some([107.2451, 46.075_863]),
            walls: [false; 4],
        };
        let mut tried = Vec::new();
        let p = spawn_point(&site, [0.2, 0.2], &mut |c| {
            tried.push(c);
            Ok(if c[1] > 46.9 {
                Tested::Blocked { stays: true }
            } else {
                Tested::Free
            })
        })
        .unwrap();
        assert_eq!(p, [106.5, 46.5]);
        assert_eq!(tried, [[106.5, 47.5], [106.5, 47.0], [106.5, 46.5]]);
    }

    #[test]
    fn a_blocker_that_moves_lets_the_spot_try_its_next_candidate() {
        let spot = Spot {
            at: [1.0, 1.0],
            step: [0.5, 0.0],
            dir: [1, 0],
            count: 1,
        };
        let mut n = 0;
        let p = try_spot(&spot, &mut |_| {
            n += 1;
            Ok(if n == 1 {
                Tested::Blocked { stays: false }
            } else {
                Tested::Free
            })
        })
        .unwrap();
        assert_eq!(p, Some([1.5, 1.0]));
    }

    fn sheep(state: u8) -> Near {
        Near {
            id: 9520,
            position: [106.68481, 47.14652, 0.0],
            radius: [0.35, 0.35],
            tall: true,
            kind: 70,
            class: 58,
            master: 705,
            buildable_on: 0,
            obstruction_type: 5,
            auto_gather: false,
            state,
            desolid: false,
            player: 0,
            moving: true,
            fog_visibility: 0,
            fog_mask: None,
            holds: false,
            hp: 7.0,
            forest_tile: false,
        }
    }

    #[test]
    fn livestock_stays_alive_a_tree_only_once_dead() {
        assert!(sheep(2).stays());
        assert!(!sheep(1).stays());
        let tree = Near {
            class: TREE,
            kind: 10,
            ..sheep(2)
        };
        assert!(!tree.stays());
        assert!(Near { state: 3, ..tree }.stays());
    }

    /// A map the player explored but does not see now, with its masks.
    struct Fogged(Vec<Near>, [u64; 2]);

    impl PlaceMap for Fogged {
        fn size(&self) -> [i32; 2] {
            [120, 120]
        }
        fn terrain(&self, _: i32, _: i32) -> Result<u8, Unsupported> {
            Ok(0)
        }
        fn explored(&self, _: usize, _: i32, _: i32) -> bool {
            true
        }
        fn visible(&self, _: usize, _: i32, _: i32) -> bool {
            false
        }
        fn vision(&self, _: usize) -> [u64; 2] {
            self.1
        }
        fn elevation(&self, _: i32, _: i32) -> Option<u8> {
            Some(0)
        }
        fn shape(&self, _: i32, _: i32) -> Option<u8> {
            Some(0)
        }
        fn allied(&self, a: usize, b: usize, _: bool) -> bool {
            a == b
        }
        fn objects(&self, x: i32, y: i32) -> Result<Vec<Near>, Unsupported> {
            Ok(self
                .0
                .iter()
                .filter(|o| o.position[0] as i32 == x && o.position[1] as i32 == y)
                .copied()
                .collect())
        }
    }

    /// In the fog, of the objects without a fog visibility only the player's own block a BUILD,
    /// and another player's building whose visibility mask meets both the player's masks.
    #[test]
    fn a_fogged_building_blocks_a_build_by_its_visibility_mask() {
        let house = Near {
            id: 500,
            position: [20.5, 20.5, 0.0],
            radius: [1.0, 1.0],
            kind: 80,
            class: 3,
            master: 70,
            obstruction_type: 2,
            moving: false,
            player: 2,
            state: 2,
            fog_mask: Some((1 << 1) | (1 << 33)),
            ..sheep(2)
        };
        let site = BuildSite {
            player: 1,
            human: true,
            clearance: [1.0, 1.0],
            class: 3,
            obstructs: true,
            restriction: &[1.0],
            hill_mode: 0,
            placement: [-1; 4],
            zones: false,
        };
        let test = |o: Near, masks: [u64; 2]| {
            build_placement(&site, &Fogged(vec![o], masks), [21.0, 21.0]).unwrap()
        };
        // the house's mask meets what player 1 sees and explored: it blocks
        assert_eq!(test(house, [1 << 1, 1 << 33]), 6);
        // not what it sees: another player's house in the fog passes
        assert_eq!(test(house, [1 << 2, 1 << 33]), 0);
        // without a UMV entry neither, but its own does
        let unmasked = Near {
            fog_mask: None,
            ..house
        };
        assert_eq!(test(unmasked, [1 << 1, 1 << 33]), 0);
        assert_eq!(
            test(
                Near {
                    player: 1,
                    ..unmasked
                },
                [0, 0]
            ),
            6
        );
    }

    struct Flat(Vec<Near>);

    impl PlaceMap for Flat {
        fn size(&self) -> [i32; 2] {
            [120, 120]
        }
        fn terrain(&self, _: i32, _: i32) -> Result<u8, Unsupported> {
            Ok(0)
        }
        fn explored(&self, _: usize, _: i32, _: i32) -> bool {
            true
        }
        fn visible(&self, _: usize, _: i32, _: i32) -> bool {
            true
        }
        fn vision(&self, _: usize) -> [u64; 2] {
            [0; 2]
        }
        fn elevation(&self, _: i32, _: i32) -> Option<u8> {
            Some(0)
        }
        fn shape(&self, _: i32, _: i32) -> Option<u8> {
            Some(0)
        }
        fn allied(&self, a: usize, b: usize, _: bool) -> bool {
            a == b
        }
        fn objects(&self, x: i32, y: i32) -> Result<Vec<Near>, Unsupported> {
            Ok(self
                .0
                .iter()
                .filter(|o| o.position[0] as i32 == x && o.position[1] as i32 == y)
                .copied()
                .collect())
        }
    }

    #[test]
    fn the_sheep_blocks_the_villager_where_their_boxes_overlap() {
        let villager = Placing {
            player: 1,
            radius: [0.2, 0.2],
            tall: true,
            class: 4,
            restriction: &[1.0],
            hill_mode: 0,
            placement: [-1; 4],
        };
        let map = Flat(vec![sheep(2)]);
        let at = |x, y| placement(&villager, &map, [x, y], false).unwrap();
        assert_eq!(at(106.5, 47.5), Tested::Blocked { stays: true });
        assert_eq!(at(106.5, 47.0), Tested::Blocked { stays: true });
        assert_eq!(at(106.5, 46.5), Tested::Free);
    }
}
