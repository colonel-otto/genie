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
use crate::production::{IN_PRODUCTION, MakeObj, MakeTech, Making, QUEUED, housed, population};
use crate::rng::Stream;
use crate::spatial::{Obstruction, Placed};
use crate::sprite::{ActiveSprite, Graphic};
use crate::turn::{GatherPoint, Turns, Unit};
use crate::unit_ai::{ACTION_COMPLETED, GenieError, NewObject, Notify, UnitAi, factory_type};
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
pub(crate) const VILLAGER: i16 = 4;
/// The make action's unit-AI action value (`0x26b`), which its constructor sets and its end
/// notifies.
pub(crate) const MAKE_ACTION: i32 = 0x26b;

/// A new moving object's right vector's z (`FUN_1411580a0` reads `DAT_14330ec20`).
pub(crate) const START_RIGHT_Z: f32 = crate::image::spawn::START_RIGHT_Z;
/// The player's attribute a new soldier's unit AI reads: the current age (`player+0x100` at
/// `+0x30`, `FUN_140c12300`).
const CURRENT_AGE: i16 = 6;

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
    test: &mut dyn FnMut([f32; 2]) -> Result<Tested, GenieError>,
) -> Result<Option<[f32; 2]>, GenieError> {
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
    test: &mut dyn FnMut([f32; 2]) -> Result<Tested, GenieError>,
) -> Result<[f32; 2], GenieError> {
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
        let mut run = |spots: &mut Vec<Spot>| -> Result<Option<[f32; 2]>, GenieError> {
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
    Err(GenieError::unsupported(
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

/// A unit to put down, beside a building or ashore, of a master that does not move (no
/// terrain restriction to test its place by).
pub(crate) const NOT_MOVING: GenieError =
    GenieError::unsupported("a unit to put down of a master that does not move");

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
    fn terrain(&self, x: i32, y: i32) -> Result<u8, GenieError>;
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
    fn objects(&self, x: i32, y: i32) -> Result<Vec<Near>, GenieError>;
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
) -> Result<Vec<Found>, GenieError> {
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
    /// The objects the scan passes over (`p12` and, for a building, its annexes, `FUN_14123d090`):
    /// none for BUILD, a reseeding gatherer's old target ([`crate::turn::Turns::reseed`]).
    pub own: &'a [i32],
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
) -> Result<bool, GenieError> {
    let shape = || {
        map.shape(tx, ty).ok_or(GenieError::unsupported(
            "a hill mode where the tiles' shapes are not known",
        ))
    };
    Ok(match mode {
        1 => !matches!(shape()?, 0 | 5..=8),
        2 => shape()? != 0,
        3 if tx == x0 || ty == y1 => {
            let e = i32::from(map.elevation(tx, ty).ok_or(GenieError::unsupported(
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
) -> Result<bool, GenieError> {
    let [side1, side2, t1, t2] = placement;
    let flags = |t: i16| {
        usize::try_from(t)
            .ok()
            .and_then(|t| map.terrain_flags(t))
            .ok_or(GenieError::unsupported(
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
) -> Result<u8, GenieError> {
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
        Some(map.elevation(x0, y1).ok_or(GenieError::unsupported(
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
                .ok_or(GenieError::unsupported(
                    "a terrain its restriction does not list",
                ))?;
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
        return Err(GenieError::unsupported(
            "BUILD's placement test in a zone mode",
        ));
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
                if site.own.contains(&o.id) {
                    continue;
                }
                if !(seen && visible) {
                    let fog = if seen { o.fog_visibility } else { 1 };
                    if fog == 3 {
                        return Err(GenieError::unsupported(
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
                    return Err(GenieError::unsupported(
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

/// The first object of a scan, which the game reports as the blocker: the tiles row by row, each
/// tile's objects in its list's order ([`crate::spatial::Spatial::objects_on`], the game's: the
/// load files them in the save's order, as the game's load does).
fn first(found: &[Found]) -> Option<Near> {
    found.first().map(|&(_, o)| o)
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
) -> Result<Tested, GenieError> {
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
                    .ok_or(GenieError::unsupported(
                        "a terrain its restriction does not list",
                    ))?;
                if *value <= PASSABLE {
                    passable = false;
                    break 'tiles;
                }
                seen = seen || map.explored(me.player, tx, ty);
                if me.hill_mode != 0 {
                    return Err(GenieError::unsupported("a placement test of a hill mode"));
                }
            }
        }
        if !passable || !seen {
            false
        } else if 0.0 < rx || 0.0 < ry {
            if zones {
                return Err(GenieError::unsupported("a placement test in a zone mode"));
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
            base_blocker = first(&found);
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
    let blocker = first(&found).or(base_blocker);
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

    fn terrain(&self, x: i32, y: i32) -> Result<u8, GenieError> {
        let t = self
            .turns
            .spatial
            .terrain
            .as_ref()
            .ok_or(GenieError::unsupported(
                "a placement test without the map's terrain",
            ))?;
        t.kind(x, y).ok_or(GenieError::unsupported(
            "a placement test off the map's tiles",
        ))
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

    fn objects(&self, x: i32, y: i32) -> Result<Vec<Near>, GenieError> {
        let t = self.turns;
        t.spatial
            .objects_on(x, y)
            .map(|(id, placed)| {
                let (owner, unit) = t.object(id).ok_or(GenieError::unsupported(
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

/// The make action's type (`+0x8`, 0x66, `TRIBE_Action_Make_Obj`).
const MAKE_OBJ: i16 = 0x66;
/// The research action's type (0x67, `TRIBE_Action_Make_Tech`).
const MAKE_TECH: i16 = 0x67;

/// The attribute that scales research times above 0 (`+0x2b0`: 86).
const RESEARCH_TIME_MODIFIER: i16 = 86;
/// The attribute naming the world's effect a player gets after each research (`+0x868`: 269).
const WORLD_EFFECT: i16 = 269;

impl Turns {
    /// The building update's production step (`FUN_1411e7dc0`, after its combat update) for the
    /// building at `i` of player `p`. A disabled queue (`+0xbf6`) or an empty one does nothing.
    /// For a player not of type 3 a top action of the building's own (`FUN_141227a00`) that is a
    /// make, a research or of types 0x7b to 0x7d sets `+0xc01` and nothing more happens; else
    /// `+0xc01` clears. Without an action in its production list the first entry's comes: for a
    /// unit the population test first ([`housed`]): a housed building waits (its reason
    /// `+0xc02`, 0x20 or 4, and the events 0x7c and 0x7d are the interface's), a computer
    /// player's cancels its queue ([`Turns::cancel_queue`]); then the make action
    /// ([`MakeObj::for_entry`]; its constructor `FUN_140acb140` gives the building's unit AI the
    /// action 619, `FUN_140c64b50`; the headroom's sound and event 0xa9 are the interface's), or
    /// the research's ([`Turns::make_research`]). The list's add (`FUN_140a977e0`) starts it
    /// for a building in a state below 3 (slot 0x28): a make action of a master the player has
    /// (`FUN_140acf0a0`) adds its population to attribute 81 and takes state 6, one of a master
    /// it lacks stays in state 0 ("Canceling Train #1a"); a building in state 3 or more deletes
    /// it at once ([`Turns::end_production_action`]). Then the list's update (`FUN_140a992b0`,
    /// the first action's slot 0x30; an empty list answers done): the make action's
    /// ([`Turns::make_step`]) or the research's ([`Turns::research_step`]). Done (1) or failed
    /// (2, 3): after a research of the shared kind the technology joins the building's lists
    /// ([`crate::production::Production::note_shared_research`]); the action goes ([`Turns::end_production_action`])
    /// and the first entry lets one go: a unit's population leaves attribute 80
    /// ([`crate::production::Queue::release_first`]).
    pub(crate) fn produce(
        &mut self,
        p: usize,
        i: usize,
        dt_seconds: f32,
    ) -> Result<(), GenieError> {
        let unit = &self.players[p].objects[i];
        let Some(production) = unit.production.as_ref() else {
            return Ok(());
        };
        if !production.queue.enabled || production.queue.count == 0 {
            return Ok(());
        }
        let busy = !self.players[p].computer
            && unit
                .top_action_kind()
                .is_some_and(|k| matches!(k, MAKE_OBJ | MAKE_TECH | 0x7b..=0x7d));
        if let Some(production) = self.players[p].objects[i].production.as_mut() {
            production.queue.waiting = busy;
        }
        if busy {
            return Ok(());
        }
        let production = self.players[p].objects[i]
            .production
            .as_ref()
            .expect("above");
        if production.action.is_none() {
            let first = *production.queue.first().expect("an entry");
            let making = if first.tech == -1 {
                let pop = self
                    .masters
                    .get(p)
                    .and_then(|m| m.get(first.master))
                    .map_or(0.0, |m| population(&m.storage));
                if housed(&self.players[p].attributes, pop)? {
                    if self.players[p].computer {
                        return self.cancel_queue(p, i);
                    }
                    return Ok(());
                }
                if let Some(ai) = self.players[p].objects[i].ai.as_mut() {
                    ai.set_action(MAKE_ACTION);
                }
                Making::Unit(MakeObj::for_entry(&first))
            } else {
                Self::make_research(&first)
            };
            self.start_production_action(p, i, making)?;
        }
        let code = self.update_production_action(p, i, dt_seconds)?;
        if !matches!(code, 1..=3) {
            return Ok(());
        }
        if code == 1
            && let Some(tech) = self.players[p].objects[i]
                .production
                .as_ref()
                .and_then(|q| q.queue.first())
                .map(|e| e.tech)
                .filter(|&t| 0 <= t)
            && self
                .techs
                .get(tech as usize)
                .is_some_and(|d| d.kind == crate::tech::SHARED_KIND)
            && (tech as usize) < self.players[p].techs.len()
            && let Some(production) = self.players[p].objects[i].production.as_mut()
        {
            production.note_shared_research(tech);
        }
        self.end_production_action(p, i)?;
        let Some(first) = self.players[p].objects[i]
            .production
            .as_ref()
            .and_then(|q| q.queue.first().copied())
        else {
            return Ok(());
        };
        if first.tech == -1 {
            let pop = self
                .masters
                .get(p)
                .and_then(|m| m.get(first.master))
                .map_or(0.0, |m| population(&m.storage));
            self.players[p].add_attribute(QUEUED, -pop)?;
        }
        if let Some(production) = self.players[p].objects[i].production.as_mut() {
            production.queue.release_first();
        }
        Ok(())
    }

    /// The list's add (`FUN_140a977e0`) of `making` to the production list of the building at
    /// `i` of player `p`: for a building in a state below 3 it joins the list and starts
    /// (slot 0x28 with 1). A make action (`FUN_140acf0a0`) of a master the player has adds its
    /// population to attribute 81 and takes state 6; one of a master it lacks stays in state 0
    /// ("Canceling Train #1a"). (The building's update makes it paid, so the payment test of an
    /// unpaid one never runs here.) A research (`0x140ad1d80`): its start test
    /// ([`crate::tech::TechState::start_test`] against what the first entry paid) puts the
    /// technology in state 2 at the building's master and the action in state 6, with no work
    /// done; a failed test (its refund, `FUN_1413d15e0`) is not modelled. A building in state 3
    /// or more deletes the action unstarted (its destructor runs, [`Turns::destroy_action`]).
    fn start_production_action(
        &mut self,
        p: usize,
        i: usize,
        making: Making,
    ) -> Result<(), GenieError> {
        use crate::tech::Attributes;
        if 3 <= self.players[p].objects[i].life.state {
            return self.destroy_action(p, i, making);
        }
        let mut making = making;
        match &mut making {
            Making::Unit(make) => {
                if let Some(m) = self.masters.get(p).and_then(|m| m.get(make.master)) {
                    let pop = population(&m.storage);
                    self.players[p].add_attribute(IN_PRODUCTION, pop)?;
                    make.state = MakeObj::MAKING;
                }
            }
            Making::Research(r) => {
                let (t, def) = self.research_tech(p, r.tech)?;
                let owner = &self.players[p];
                let get = |k: i16| owner.get(k);
                let factors = crate::tech::factors(r.tech, &get)?;
                let paid = owner.objects[i]
                    .production
                    .as_ref()
                    .and_then(|q| q.queue.first())
                    .map_or(crate::production::NO_COSTS, |e| e.costs);
                if !owner.techs[t].start_test(def, factors, &paid, &get)? {
                    return Err(GenieError::unsupported(
                        "a research whose start test fails (its refund)",
                    ));
                }
                let master = owner.objects[i].life.master_id;
                let state = &mut self.players[p].techs[t];
                state.state = 2;
                state.building_master = master;
                r.state = MakeObj::MAKING;
                r.progress = 0.0;
            }
        }
        if let Some(production) = self.players[p].objects[i].production.as_mut() {
            production.action = Some(making);
        }
        Ok(())
    }

    /// The production list's update (`FUN_140a992b0`) of the building at `i` of player `p`:
    /// the first action's slot 0x30, or 1 for an empty list.
    fn update_production_action(
        &mut self,
        p: usize,
        i: usize,
        dt_seconds: f32,
    ) -> Result<u8, GenieError> {
        let action = self.players[p].objects[i]
            .production
            .as_ref()
            .and_then(|q| q.action);
        match action {
            None => Ok(1),
            Some(Making::Unit(_)) => self.make_step(p, i, dt_seconds),
            Some(Making::Research(_)) => self.research_step(p, i, dt_seconds),
        }
    }

    /// The make action's update (`0x140ad0490`) at the building at `i` of player `p`: in state
    /// 1 it is done; in state 6 the building's work rate (its powerup, slot 0x6e8, plus its
    /// master's `work_rate`) times dt adds to its progress, and once its train time
    /// ([`MakeObj::time`]) is reached (`COMISS`, `JC`: a NaN waits) its unit spawns
    /// ([`Turns::spawn`], which a computer player's unit does not reach) and it is done; any
    /// other state waits. Not modelled: a computer player's handicaps (`FUN_141659cf0`, the
    /// difficulty's factors on the rate), the game's instant work (`+0xc14`,
    /// `FUN_140c884c0`, 0 as the tick oracle holds it), the spot search's failure (state 10's
    /// timer, `FUN_140ace630`).
    fn make_step(&mut self, p: usize, i: usize, dt_seconds: f32) -> Result<u8, GenieError> {
        let building = &self.players[p].objects[i];
        let Some(Some(Making::Unit(make))) = building.production.as_ref().map(|q| q.action) else {
            unreachable!("the caller's make action");
        };
        match make.state {
            MakeObj::DONE => return Ok(1),
            MakeObj::MAKING => {}
            _ => return Ok(0),
        }
        let site = self.master_of(p, building.life.master_id)?;
        let rate = building.work_rate_powerup + site.work_rate;
        let made = self.masters.get(p).and_then(|m| m.get(make.master));
        let time = f32::from(make.time(made, building.life.master_id));
        let progress = rate * dt_seconds + make.progress;
        if let Some(Some(Making::Unit(m))) = self.players[p].objects[i]
            .production
            .as_mut()
            .map(|q| q.action.as_mut())
        {
            m.progress = progress;
        }
        if crate::fmath::below_or_unordered(progress, time) {
            return Ok(0);
        }
        let pop = made.map_or(0.0, |m| population(&m.storage));
        self.spawn(p, i, make.master, pop)?;
        if let Some(Some(Making::Unit(m))) = self.players[p].objects[i]
            .production
            .as_mut()
            .map(|q| q.action.as_mut())
        {
            m.state = MakeObj::DONE;
        }
        Ok(1)
    }

    /// The production list's first action goes (`FUN_140a98e20`, and `FUN_140a98620` for the
    /// list of one the building's update makes) at the building at `i` of player `p`: its
    /// destructor ([`Turns::destroy_action`]).
    pub(crate) fn end_production_action(&mut self, p: usize, i: usize) -> Result<(), GenieError> {
        let Some(making) = self.players[p].objects[i]
            .production
            .as_mut()
            .and_then(|q| q.action.take())
        else {
            return Ok(());
        };
        self.destroy_action(p, i, making)
    }

    /// A production action's destructor at the building at `i` of player `p`. The make
    /// action's (`FUN_140ace3f0`): in state 6 or 2, while the game is not over, its unit's
    /// population leaves attribute 81 and, made without the queue's payment (`+0x60` clear),
    /// the master's paid costs come back ([`crate::production::refund_master`]); the building's
    /// unit AI is told the make action completed (0x1fa with 0x26b). The research's: its
    /// destructor ([`Turns::research_destroyed`]).
    fn destroy_action(&mut self, p: usize, i: usize, making: Making) -> Result<(), GenieError> {
        let make = match making {
            Making::Unit(make) => make,
            Making::Research(r) => return self.research_destroyed(p, i, &r),
        };
        if matches!(make.state, MakeObj::MAKING | 2) && self.game_state == 0 {
            let made = self.masters.get(p).and_then(|m| m.get(make.master));
            let pop = made.map_or(0.0, |m| population(&m.storage));
            let refund = (!make.paid).then(|| made.cloned()).flatten();
            let owner = &mut self.players[p];
            owner.add_attribute(IN_PRODUCTION, -pop)?;
            if let Some(m) = refund {
                crate::production::refund_master(&m, 1, owner)?;
            }
        }
        let building = &mut self.players[p].objects[i];
        let (id, state) = (building.id, building.life.state);
        if let Some(ai) = building.ai.as_mut() {
            ai.post(
                Notify::own(id, ACTION_COMPLETED, [MAKE_ACTION, 0, 0]),
                state,
            )?;
        }
        Ok(())
    }

    /// The research's action (`TRIBE_Action_Make_Tech`, constructor `FUN_140ad0700` with the
    /// first entry's technology and master and 0, so paid; state 0, nothing done) for player
    /// `p`: its effect's first preparation (`FUN_1413d1530`) loads graphics and marks the
    /// masters an upgrade turns into (`+0x330`: the save does not hold the mark, and its
    /// readers, the object search `FUN_1411650e0` and the masters' slot 0xc0, are not
    /// modelled); the human player's sound is the interface's. Not held: the start time
    /// (`+0x74`).
    const fn make_research(first: &crate::production::Entry) -> Making {
        Making::Research(MakeTech {
            state: 0,
            paid: true,
            tech: first.tech,
            master: first.master as i32,
            progress: 0.0,
        })
    }

    /// The technology `tech` of a research at player `p`'s building: its index in the player's
    /// tree and the `.dat`'s record (RESEARCH queued only technologies of both).
    fn research_tech(
        &self,
        p: usize,
        tech: i16,
    ) -> Result<(usize, &crate::tech::TechDef), GenieError> {
        let t = usize::try_from(tech)
            .ok()
            .filter(|&t| t < self.players[p].techs.len())
            .ok_or(GenieError::unsupported(
                "a research of a technology the tree has not got",
            ))?;
        let def = self.techs.get(t).ok_or(GenieError::unsupported(
            "a technology the data file has not got",
        ))?;
        Ok((t, def))
    }

    /// The research's update (slot 0x30, `0x140ad2a20`) at the building at `i` of player `p`:
    /// in state 1 it is done, in a state other than 6 it waits; in state 6 the building's work rate (its powerup and its master's rate) times dt adds to
    /// the progress, 32767 once the technology was researched 0x100 times, which the
    /// technology's work done takes; a repeatable technology or one of the shared kind turns to
    /// state 2 first. The research is done once the progress reaches the location's time,
    /// times the research time modifier (attribute 86, above 0 and not an age), plus its
    /// adjustment ([`Turns::research_done`]). Not held: the human player's sound; not
    /// modelled: a computer player's handicaps and the game's instant work (`FUN_140c884c0`,
    /// 0 as the tick oracle holds it).
    fn research_step(&mut self, p: usize, i: usize, dt_seconds: f32) -> Result<u8, GenieError> {
        use crate::tech::Attributes;
        if self.players[p].computer {
            return Err(GenieError::unsupported(
                "a computer player's research (its handicaps)",
            ));
        }
        let owner = &self.players[p];
        let building = &owner.objects[i];
        let Some(Some(Making::Research(r))) = building.production.as_ref().map(|q| q.action) else {
            unreachable!("the caller's research");
        };
        match r.state {
            MakeObj::DONE => return Ok(1),
            MakeObj::MAKING => {}
            _ => return Ok(0),
        }
        let (t, def) = self.research_tech(p, r.tech)?;
        let master = building.life.master_id;
        let rate = building.work_rate_powerup + self.master_of(p, master)?.work_rate;
        let state = &owner.techs[t];
        let repeatable = state.repeatable(def, false);
        let shared = def.kind == crate::tech::SHARED_KIND;
        let time_modifier = owner.get(RESEARCH_TIME_MODIFIER)?;
        let location = state
            .location(master)
            .copied()
            .ok_or(GenieError::unsupported(
                "a research at a technology without locations",
            ))?;
        let times = state.times_researched;
        let owner = &mut self.players[p];
        let state = &mut owner.techs[t];
        if state.state != 2 && (repeatable || shared) {
            state.state = 2;
        }
        let mut progress = r.progress + rate * dt_seconds;
        if 0xff < times {
            progress = 32767.0;
        }
        state.research_done = progress;
        if let Some(Some(Making::Research(r))) = owner.objects[i]
            .production
            .as_mut()
            .map(|q| q.action.as_mut())
        {
            r.progress = progress;
        }
        let mut time = f32::from(location.time as i16);
        if !crate::tech::is_age(r.tech) && 0.0 < time_modifier {
            time *= time_modifier;
        }
        if location.adjustment + time <= progress {
            if 0xff < times {
                return Err(GenieError::unsupported(
                    "a research done 0x100 times (set_state 1)",
                ));
            }
            self.research_done(p, i, t, master, repeatable)?;
            return Ok(1);
        }
        Ok(0)
    }

    /// The research's end in its update (`0x140ad2a20`) once its progress reaches its time:
    /// `FUN_1413d1740` (with `repeat`, the player's `+0x648` or [`TechState::repeatable`]): a
    /// technology in state 2 (or with `repeat`) whose work done reaches the `.dat`'s time at the
    /// building (−1 without), times the research time modifier (attribute 86, above 0, not an
    /// age), plus the location's adjustment, is done ([`Turns::complete_research`]); else, or
    /// without `repeat`, it is left. Then the action takes state 1 (the event 100 and the
    /// computer players' slot 0x3a8 are not held) and the building's unit AI is told 0x1fa with
    /// the action value 0x26c. Last, attribute 269 (the Spanish's 300) names an effect of the
    /// world's table ([`Turns::effects`], slot 0x10) the player gets with the building as its
    /// object ([`Turns::do_effect`]).
    ///
    /// [`TechState::repeatable`]: crate::tech::TechState::repeatable
    fn research_done(
        &mut self,
        p: usize,
        i: usize,
        t: usize,
        master: i16,
        repeat: bool,
    ) -> Result<(), GenieError> {
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
        let building = &mut self.players[p].objects[i];
        let (id, state) = (building.id, building.life.state);
        if let Some(Some(Making::Research(r))) =
            building.production.as_mut().map(|q| q.action.as_mut())
        {
            r.state = MakeObj::DONE;
        }
        if let Some(ai) = building.ai.as_mut() {
            ai.post(
                Notify::own(id, ACTION_COMPLETED, [crate::turn::RESEARCH_ACTION, 0, 0]),
                state,
            )?;
        }
        // attribute 269, cut by CVTTSS2SI and its low short above 0, names an effect of the
        // world's table the player gets with the building as its object (its slot 0x10,
        // `TRIBE_Effects::do_effect`, which skips an id the table does not hold)
        let reward = crate::checkpoint::cvttss2si(self.players[p].get(WORLD_EFFECT)?) as i16;
        if 0 < reward
            && let Some(effect) = usize::try_from(reward)
                .ok()
                .and_then(|e| self.effects.get(e))
                .cloned()
        {
            self.do_effect(p, &effect, Some(id))?;
        }
        Ok(())
    }

    /// The research action's destructor (`FUN_140ad0be0`) at the building at `i` of player
    /// `p`. Done (state 1), the human player's sound is the interface's; else, while the game is
    /// not over, a research made unpaid (`+0x60` clear) gives its technology's costs back
    /// ([`crate::production::refund_tech`]) and the building's unit AI is told the research
    /// failed (0x1f9 with 0x26c). Then a technology of the shared kind (`FUN_1413d1020`) is in
    /// state 1 again, neither researched nor queued; another that stacks below its cap (`+0x56`,
    /// 0x100 above 0x7fff; the player's `+0x648`, which would count too, is not held) takes the
    /// action's progress as its work done and, done (the progress at least the `.dat`'s time at
    /// the building times the research time modifier, above 0 and not an age, plus the
    /// location's adjustment; `COMISS`, `SETNC`), counts one more researched and one less
    /// queued, in state 1 while one more stays below the cap and it may be queued again (its
    /// `+0xe` set and its effect no upgrade), else in state 3; not done, state 1.
    fn research_destroyed(&mut self, p: usize, i: usize, r: &MakeTech) -> Result<(), GenieError> {
        use crate::tech::Attributes;
        let (t, _) = self.research_tech(p, r.tech)?;
        let def = &self.techs[t];
        if r.state != MakeObj::DONE && self.game_state == 0 {
            if !r.paid {
                let owner = &mut self.players[p];
                let mut state = std::mem::take(&mut owner.techs[t]);
                let done = crate::production::refund_tech(&mut state, def, r.tech, owner);
                owner.techs[t] = state;
                let _ = done?;
            }
            let building = &mut self.players[p].objects[i];
            let (id, state) = (building.id, building.life.state);
            if let Some(ai) = building.ai.as_mut() {
                ai.post(
                    Notify::own(
                        id,
                        crate::unit_ai::ACTION_FAILED,
                        [crate::turn::RESEARCH_ACTION, 0, 0],
                    ),
                    state,
                )?;
            }
        }
        let owner = &self.players[p];
        let state = &owner.techs[t];
        if def.kind == crate::tech::SHARED_KIND {
            let state = &mut self.players[p].techs[t];
            state.state = 1;
            state.times_researched = 0;
            state.pending = 0;
            return Ok(());
        }
        let cap = if 0x7fff < state.research_cap as u16 {
            0x100
        } else {
            state.research_cap
        };
        if !(state.stack && state.times_researched < cap) {
            return Ok(());
        }
        let once = !state.allow_multiple || def.upgrades();
        let master = owner.objects[i].life.master_id;
        let mut time = def
            .locations
            .iter()
            .find(|l| l.0 == master)
            .map_or(-1.0, |l| f32::from(l.1));
        let adjustment = state.location(master).map_or(0.0, |l| l.adjustment);
        let modifier = owner.get(RESEARCH_TIME_MODIFIER)?;
        if !crate::tech::is_age(r.tech) && 0.0 < modifier {
            time *= modifier;
        }
        let done = adjustment + time <= r.progress;
        let state = &mut self.players[p].techs[t];
        state.research_done = r.progress;
        if done {
            state.times_researched = state.times_researched.wrapping_add(1);
            state.pending = state.pending.wrapping_sub(1);
            state.state = if i32::from(state.times_researched) < i32::from(cap) && !once {
                1
            } else {
                3
            };
        } else {
            state.state = 1;
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
    /// The player's tracker lets its queue place go ([`crate::tracker::Tracker::unplace`], the
    /// counter one less).
    ///
    /// Not held: its facet and angle, its powerup offset, the building's `+0xc1c` group (unsaved), the game's events
    /// (`FUN_140c898a0`), the per-master counts (slot 0x398's `+0x13b0`), the sounds and
    /// particles (`FUN_1411a52e0`).
    fn spawn(&mut self, p: usize, i: usize, master: i16, pop: f32) -> Result<(), GenieError> {
        if self.players[p].computer {
            return Err(GenieError::unsupported(
                "a computer player's new unit (its AI's slot 0x398)",
            ));
        }
        if self.spatial.sight.is_none() {
            return Err(GenieError::unsupported(
                "a spawn without the players' sight",
            ));
        }
        let building = &self.players[p].objects[i];
        let made = self.master_of(p, master)?.clone();
        if made.recyclable {
            return Err(GenieError::unsupported(
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
            return Err(GenieError::unsupported(
                "a new unit of a master that does not move",
            ));
        }
        let point = self.spot_beside(p, i, &made, toward)?;
        let new = self.combat_object(p, master, &made, [point[0], point[1], position[2]])?;
        let class = self.tracker_key(p, master).class;
        let owner = &mut self.players[p];
        owner.add_attribute(IN_PRODUCTION, -pop)?;
        // its queue place goes (FUN_14104fb00 with -1, FUN_14104f330)
        owner.tracker.count_queued(-1);
        owner.tracker.unplace(master, class);
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
            return Err(GenieError::unsupported(
                "a new unit's task that spawns more units",
            ));
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
    ) -> Result<(), GenieError> {
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
                    return Err(GenieError::unsupported(
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

    /// What the unit AI of player `p`'s new object of master `made` (id `master`) at `position`
    /// reads ([`NewObject`]): its master's fields and its owner's type, posture (`+0x71c`) and
    /// current age (attribute 6).
    ///
    /// # Errors
    ///
    /// A player without attribute 6.
    pub(crate) fn new_object(
        &self,
        p: usize,
        master: i16,
        made: &crate::masters::Master,
        position: [f32; 3],
    ) -> Result<NewObject, GenieError> {
        let owner = &self.players[p];
        Ok(NewObject {
            computer: owner.computer,
            posture: owner.posture,
            age: crate::tech::Attributes::get(owner, CURRENT_AGE)?,
            ..NewObject::of(position, master, made)
        })
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
    /// player's attributes 98, 164 and 246 grow by its paid cost (246 while the telemetry's
    /// match runs: `FUN_140982ff0()`'s `+0x921`, which every game's start sets, `FUN_140f1fb20`
    /// with `FUN_140980ac0`, and only its teardown `FUN_140c85fb0` clears, `FUN_140980de0`) and
    /// a villager's (class 4) attribute 248 by one, and its class counts
    /// ([`crate::turn::count_class`]). It joins its player's list last; its
    /// index there.
    /// The static, animated and moving setups' draws (`FUN_1411a45a0`, `FUN_141072690`,
    /// `FUN_141161450`) of a new object whose current sprite is `standing` (slot 0x798: its
    /// master's standing sprite, a building's too, `FUN_1412029f0`): a sprite with a random first
    /// frame and several facets draws its facet (Object stream), the sprite starts (Sprite stream,
    /// [`ActiveSprite::start`]), and one of several facets with sequence flag 2 draws the angle the
    /// object faces (Object stream, "setup" line 0x2c3). Its sprite, its facets and its angle.
    pub(crate) fn setup_draws(
        &mut self,
        standing: Graphic,
    ) -> (ActiveSprite, crate::facing::SpriteFacets, f32) {
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
        (sprite, facets, angle)
    }

    pub(crate) fn combat_object(
        &mut self,
        p: usize,
        master: i16,
        made: &crate::masters::Master,
        position: [f32; 3],
    ) -> Result<usize, GenieError> {
        if self.players[p].computer {
            return Err(GenieError::unsupported(
                "a computer player's new unit (its AI's slot 0x398)",
            ));
        }
        if self.spatial.sight.is_none() {
            return Err(GenieError::unsupported(
                "a spawn without the players' sight",
            ));
        }
        if made.recyclable {
            return Err(GenieError::unsupported(
                "a new object of a recyclable master (a negative id)",
            ));
        }
        let moving = made.moving.ok_or(GenieError::unsupported(
            "a new unit of a master that does not move",
        ))?;
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
        let ai_type = factory_type(
            made.class,
            master,
            made.volley_missile.unwrap_or(-1),
            made.combat_level,
        )
        .ok_or(GenieError::unsupported(
            "a new combat unit without a unit AI",
        ))?;
        let new_object = self.new_object(p, master, made, position)?;
        let standing = (0 <= made.standing_sprite)
            .then(|| self.graphics.get(&made.standing_sprite).copied())
            .flatten()
            .ok_or(GenieError::unsupported(
                "a new unit's standing sprite the data file does not have",
            ))?;
        let id = self.next_object_id;
        self.next_object_id += 1;
        let (sprite, facets, angle) = self.setup_draws(standing);
        let ai = UnitAi::constructed(ai_type, &new_object, self.streams.get(Stream::Ai));
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
        owner.add_attribute(246, cost as f32)?;
        if made.class == VILLAGER {
            owner.add_attribute(248, 1.0)?;
        }
        // the combat setup's class counters (FUN_141226060)
        crate::turn::count_class(&mut owner.attributes, made, 1.0);
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
            ai,
            // FUN_1411580a0: facing x (an immediate 1.0), its right -z, not waiting
            mover: Some(Mover::new_object(
                moving,
                position,
                self.world_time,
                angle,
                Some(facets),
            )),
            attackable: true,
            pasture: -1,
            my_pasture: -1,
            combat: true,
            // the TRIBE combat setup (FUN_14121df60): the master's volley fire
            firepower: made.volley_fire,
            sprite: Some(sprite),
            // the animated object's constructor (FUN_141070cf0): animation speed (+0x7b8) 0
            anim_speed: Some(0.0),
            // livestock's default task, the Artifact, starts once its action list is empty
            default_artifact: made
                .default_task()
                .is_some_and(|t| t.action_type == crate::artifact::ARTIFACT_TYPE),
            ..Unit::default()
        });
        let idx = owner.objects.len() - 1;
        // the static init's add to its player (slot 0x2d0) files it in the tracker
        self.track_add(p, id, master);
        Ok(idx)
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
    ) -> Result<[f32; 2], GenieError> {
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
        let moving = made.moving.ok_or(NOT_MOVING)?;
        let restriction = self
            .spatial
            .terrain
            .as_ref()
            .and_then(|t| t.restrictions.get(moving.restriction))
            .ok_or(GenieError::unsupported(
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
    ) -> Result<(), GenieError> {
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
            return Err(GenieError::unsupported(
                "remains of a type other than 10 and 20 to 39",
            ));
        }
        if 0 < made.hit_points {
            return Err(GenieError::unsupported(
                "remains with hit points (the sleeping list)",
            ));
        }
        if made.recyclable {
            return Err(GenieError::unsupported(
                "a new object of a recyclable master (a negative id)",
            ));
        }
        // its place (slot 0x58, the static new_position): the ground's height by its master's
        // fly mode (slot 0x60, FUN_141193a10)
        let at = self.on_ground(p, master, at);
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
            let facet = facet.ok_or(GenieError::unsupported(
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
            Some(made.moving.ok_or(GenieError::unsupported(
                "moving remains without a moving master",
            ))?)
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
            mover: moving.map(|stats| {
                Mover::new_object(
                    stats,
                    at,
                    self.world_time,
                    angle,
                    standing.map(|g| crate::facing::SpriteFacets {
                        count: g.facets,
                        flag_1: g.flags & 2 != 0,
                    }),
                )
            }),
            attackable: true,
            pasture: -1,
            my_pasture: -1,
            // an animated one's constructor (FUN_141070cf0): animation speed (+0x7b8) 0
            anim_speed: (20 <= made.kind).then_some(0.0),
            ..Unit::default()
        });
        // the static init's add to its player (slot 0x2d0) files it in the tracker
        self.track_add(p, id, master);
        Ok(())
    }

    /// FLARE's objects (`0x140e7bfd0`): with `players` at most the world's players, each player
    /// from 1 below `players` makes a flare (master 274, its slot 0x1c0 `FUN_14162ce00` with the
    /// footprint left alone: the master's slot 0x40 as [`Turns::remains`] makes it, then
    /// [`Turns::made_new_object`]) at
    /// `target`'s place, or at `point` on the ground when `target` is −1 or gone; its workers
    /// (`+0x644`) are 0. Not held: the sender's flare's workers 1 when the local player is among
    /// the recipients (`+0x11`..), a computer player's AI memory of it (`+0x7a8`'s `+0xbc00`),
    /// the sound and the minimap.
    pub(crate) fn flare(
        &mut self,
        target: i32,
        point: [f32; 2],
        players: u8,
    ) -> Result<(), GenieError> {
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
            self.made_new_object(p)?;
        }
        Ok(())
    }

    /// The flare a hit on player `p`'s object makes for it (`FUN_14120c0a0`: the player's slot
    /// 0x1c0 with master 112, `FUN_14162ce00` with the footprint left alone, as FLARE's,
    /// [`Turns::remains`] then [`Turns::made_new_object`]) at the attacker's place `at`; from a
    /// trebuchet (`doubled`) its amount (`+0x310`, which runs down as its time) doubles.
    ///
    /// # Errors
    ///
    /// What the new object does not model.
    pub(crate) fn make_flare(
        &mut self,
        p: usize,
        at: [f32; 3],
        doubled: bool,
    ) -> Result<(), GenieError> {
        /// The flare a hit leaves (`0x70`, FLARE).
        const HIT_FLARE: i16 = 112;
        let before = self.players[p].objects.len();
        self.remains(p, HIT_FLARE, at, None)?;
        self.made_new_object(p)?;
        if doubled && let Some(f) = self.players[p].objects.get_mut(before) {
            f.life.amount += f.life.amount;
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
    ) -> Result<(), GenieError> {
        let master = made.id;
        let building = made.building.ok_or(GenieError::unsupported(
            "a new building of a master without its building part",
        ))?;
        if self.players[p].computer {
            return Err(GenieError::unsupported(
                "a computer player's new building (its AI's slot 0x398)",
            ));
        }
        if 0 < made.build_time || 0.0 < made.radius[0] || 0.0 < made.radius[1] {
            return Err(GenieError::unsupported(
                "a new building with a build time or radii (its construction, its footprint)",
            ));
        }
        if 0 <= made.standing_sprite || 0.0 < made.line_of_sight {
            return Err(GenieError::unsupported(
                "a new building with a standing sprite or sight (its setups' draws, its watch)",
            ));
        }
        if building.annexes.iter().any(|&a| 0 <= a) || 0 <= building.head_unit {
            return Err(GenieError::unsupported(
                "a new building with annexes or of an annex",
            ));
        }
        if matches!(made.class, 51 | 54) || matches!(master, 50 | 70 | 276 | 357 | 1187 | 1188) {
            return Err(GenieError::unsupported(
                "a new building of classes 51, 54 or masters 50, 70, 276, 357, 1187, 1188",
            ));
        }
        if made.recyclable
            || made
                .cost
                .iter()
                .any(|&(r, a, paid)| 0 <= r && paid && a != 0)
        {
            return Err(GenieError::unsupported(
                "a new building of a recyclable master or with a paid cost",
            ));
        }
        let ai_type = factory_type(
            made.class,
            master,
            made.volley_missile.unwrap_or(-1),
            made.combat_level,
        );
        let new_object = self.new_object(p, master, made, at)?;
        let id = self.next_object_id;
        self.next_object_id += 1;
        let ai =
            ai_type.and_then(|t| UnitAi::constructed(t, &new_object, self.streams.get(Stream::Ai)));
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
        // the combat setup's class counters (FUN_141226060)
        crate::turn::count_class(&mut self.players[p].attributes, made, 1.0);
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
            // the moving setup (FUN_1411580a0): a building is a moving object; without a
            // standing sprite it draws no angle
            mover: made
                .moving
                .map(|stats| Mover::new_object(stats, at, self.world_time, 0.0, None)),
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
            // the animated object's constructor (FUN_141070cf0): animation speed (+0x7b8) 0
            anim_speed: Some(0.0),
            ..Unit::default()
        });
        // the static init's add to its player (slot 0x2d0): it waits in the tracker's pending
        // lists until its first update completes it
        self.track_add(p, id, made.id);
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
    ) -> Result<(), GenieError> {
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
            return Err(GenieError::unsupported(
                "a gather point's move of a unit with an action",
            ));
        }
        // a unit without speed takes nothing; one not in state 2 would notify without moving
        if unit.mover.as_ref().map_or(0.0, |m| m.stats.speed) <= 0.0 {
            return Ok(());
        }
        if unit.life.state != 2 {
            return Err(GenieError::unsupported(
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
    ) -> Result<Option<i32>, GenieError> {
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
            .ok_or(GenieError::unsupported("a world without Gaia"))?;
        let closest = |list: &[Unit]| -> Result<Option<i32>, GenieError> {
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
            return Err(GenieError::unsupported(
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
    /// building takes the unit in at once ([`Turns::enter_forced`]) while the building holds
    /// fewer than 10 or fewer than its capacity.
    pub(crate) fn gather(
        &mut self,
        p: usize,
        new: usize,
        g: GatherPoint,
        building: i32,
        notified: bool,
        finder: bool,
    ) -> Result<(), GenieError> {
        if g.target == -1 {
            return self.gather_move(p, new, [g.point[0], g.point[1], 0.0], notified);
        }
        // its own building takes it at once (slot 0x1a0 forced) while it holds fewer than 10 or
        // fewer than its master's capacity (its slot 0x498 against `+0x5d4`); else the building
        // is a target as any other
        if g.target == building
            && let Some((bp, b)) = self.object(building)
        {
            let room = i32::from(self.master_of(bp, b.life.master_id)?.garrison);
            let held = self.garrisoned(building)?;
            if held < 10 || held < room {
                return self.enter_forced(p, new, building);
            }
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
        let (owner, t) = self.object(target).ok_or(GenieError::unreachable(
            "unreachable: a gather point's target found",
        ))?;
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
        fn terrain(&self, _: i32, _: i32) -> Result<u8, GenieError> {
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
        fn objects(&self, _: i32, _: i32) -> Result<Vec<Near>, GenieError> {
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
        fn terrain(&self, _: i32, _: i32) -> Result<u8, GenieError> {
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
        fn objects(&self, x: i32, y: i32) -> Result<Vec<Near>, GenieError> {
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
            own: &[],
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
        fn terrain(&self, _: i32, _: i32) -> Result<u8, GenieError> {
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
        fn objects(&self, x: i32, y: i32) -> Result<Vec<Near>, GenieError> {
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

    /// Two objects on one tile that block the point differently: the first of the tile's list
    /// reports (the game's scan, `FUN_141264db0`), whichever stays.
    #[test]
    fn the_first_object_of_a_tile_s_list_reports_the_block() {
        let villager = Placing {
            player: 1,
            radius: [0.2, 0.2],
            tall: true,
            class: 4,
            restriction: &[1.0],
            hill_mode: 0,
            placement: [-1; 4],
        };
        // a sheep that stays (state 2) and a soldier that does not, on the same tile
        let soldier = Near {
            id: 9521,
            class: 6,
            master: 4,
            ..sheep(2)
        };
        let at = |list: Vec<Near>| placement(&villager, &Flat(list), [106.5, 47.5], false).unwrap();
        assert_eq!(at(vec![sheep(2), soldier]), Tested::Blocked { stays: true });
        assert_eq!(
            at(vec![soldier, sheep(2)]),
            Tested::Blocked { stays: false }
        );
    }
}
