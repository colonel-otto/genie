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
/// The wall class, which the search checks the building's corners for (`FUN_1415417f0`).
pub const WALL: i16 = 27;
/// Classes the placement tests treat apart: a tree placed (15) passes over farms (49); buildings
/// of classes 51 and 54 block though desolid.
const TREE: i16 = 15;
const FARM: i16 = 49;
const CLASS_51: i16 = 51;
const CLASS_54: i16 = 54;
/// The relic's master, which always stays (`FUN_141194e80`).
const RELIC: i16 = 285;
/// The villager class, whose new units count in attribute 248.
const VILLAGER: i16 = 4;
/// The make action's unit-AI action value (`0x26b`), which its constructor sets and its end
/// notifies.
const MAKE_ACTION: i32 = 0x26b;

/// A new moving object's right vector's z (`FUN_1411580a0` reads `DAT_14330ec20`).
const START_RIGHT_Z: f32 = crate::image::spawn::START_RIGHT_Z;

/// The building a unit spawns from, as the search reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Site {
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
    Free,
    /// Not free; `stays`: the object it reports blocks for good, which ends the spot.
    Blocked {
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
    let (gx, gy) = site.gather.map_or((NONE, NONE), |[gx, gy]| (gx, gy));
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
    pub id: i32,
    pub position: [f32; 3],
    /// Its master's radii on the ground and whether its height radius is above 0.
    pub radius: [f32; 2],
    pub tall: bool,
    /// Its master's type (`+0x8`), class, id, `can_be_built_on` (`+0x5e9`), `obstruction_type`
    /// (`+0x651`) and `enable_auto_gather` (`+0x681`).
    pub kind: u8,
    pub class: i16,
    pub master: i16,
    pub buildable_on: u8,
    pub obstruction_type: u8,
    pub auto_gather: bool,
    /// Its state (`+0x640`) and, for a building, its `desolid_flag` (`+0xc60`).
    pub state: u8,
    pub desolid: bool,
}

impl Near {
    /// The blocker tests of `FUN_1411c5b60`: a building (`FUN_1411942c0`), a tree past state 2
    /// (`FUN_141195660`), the relic (`FUN_141194e80`), and `FUN_141194170`'s master with
    /// `enable_auto_gather` or of a class that stays from state 2 on (5, 9, 10, 29, 31, 33, 38,
    /// 40, 41, 58, 61) or from state 3 on (7, 8, 15, 32, 48, 63).
    pub fn stays(&self) -> bool {
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
    pub player: usize,
    pub radius: [f32; 2],
    pub tall: bool,
    pub class: i16,
    /// Its terrain restriction's value per terrain.
    pub restriction: &'a [f32],
    /// `hill_mode` (`+0x604`) and whether the placement terrains (`+0x5f2`..`+0x5f8`) name one.
    pub hill_mode: u8,
    pub placement_terrains: bool,
}

/// What the placement test reads of the map.
pub trait PlaceMap {
    fn size(&self) -> [i32; 2];
    /// A tile's terrain (the map tile's `+0x9`).
    fn terrain(&self, x: i32, y: i32) -> Result<u8, Unsupported>;
    /// `FUN_141663500` is not 0: the player sees or saw the tile.
    fn explored(&self, player: usize, x: i32, y: i32) -> bool;
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
    if me.placement_terrains {
        return Err(Unsupported("a placement test with placement terrains"));
    }
    let [x, y] = p;
    let [rx, ry] = me.radius;
    let [w, h] = map.size();
    let (far_x, far_y) = ((x + rx) - INSIDE, (y + ry) - INSIDE);
    // FUN_141123f80's own result: 0 free; the objects of its scan report the blocker (6)
    let mut base_blocker = None;
    let base_free = if x - rx < 0.0 || w as f32 <= far_x || y - ry < 0.0 || h as f32 <= far_y {
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
struct WorldMap<'a> {
    turns: &'a Turns,
}

impl PlaceMap for WorldMap<'_> {
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
                })
            })
            .collect()
    }
}

impl Turns {
    /// The building update's production step (`FUN_1411e7dc0`, after its combat update) for the
    /// building at `i` of player `p`: with a queue and no make action, a unit's make action comes
    /// (its constructor `FUN_140acb140` gives the building's unit AI action 619 with
    /// `FUN_140c64b50`; the list's add starts it, slot 0x28 `FUN_140acf0a0`: attribute 81 grows by
    /// the unit's population, state 6); then the make action's update (`0x140ad0490`) works. A
    /// unit that is due spawns ([`Turns::spawn`]); its action ends (state 1, its destructor
    /// `FUN_140ace3f0` notifies the building's unit AI 506 with 619 again) and the queue lets it
    /// go (attribute 80 lowers by its population). A building whose population test fails
    /// (housed) is not modelled, nor is the computer players' list, a technology in the queue or
    /// the work-rate bonuses of `0x140ad0490` (`FUN_140c884c0`'s game mode, the AI difficulty
    /// under `FUN_141659cf0`).
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
        let master = front.master;
        let pop = population(&self.master_of(p, master)?.storage);
        if !production.action {
            if housed(&self.players[p].attributes, pop)? {
                return Err(Unsupported("a housed building's make action (it waits)"));
            }
            let unit = &mut self.players[p].objects[i];
            if let Some(ai) = unit.ai.as_mut() {
                ai.set_action(MAKE_ACTION);
            }
            if let Some(production) = unit.production.as_mut() {
                production.action = true;
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

    /// `FUN_140ad0240`: the unit of `master` spawns beside the building at `i`. The spawn point
    /// ([`spawn_point`], from `FUN_140ace630`), then the master's slot 0x40 (`0x141266120`): the
    /// combat object's setups in order (static `FUN_1411a45a0`, animated `FUN_141072690`, moving
    /// `FUN_141161450`, combat `FUN_14121df60`) with their draws: the facet (Object stream, for a
    /// standing sprite with a random first frame and several facets), the standing sprite
    /// ([`ActiveSprite::start`], Sprite stream), the angle (Object stream, sequence flag 2), the
    /// unit AI ([`UnitAi::constructed`], three AI-stream draws) and the powerup offset (Object
    /// stream). The object takes its place, its record and mesh shape, its sight and its storage
    /// (flags 1, 2 and 32, `FUN_141193860`); its player's attributes 98 and 164 (and 246 under
    /// [`Turns::unit_cost_246`]) grow by its paid cost and a villager's (class 4) attribute 248 by
    /// one. Then attribute 81 lowers by its population, the building notifies its unit AI 506
    /// with 619, and the gather point's work comes (`FUN_140ace9c0`): `+0x7f9` (waiting to move)
    /// set, the work command on the gather point's target at the point (slot 0x128 with 1).
    ///
    /// Not held: its facet and angle, its sprite (the slice holds sprites from an attack on), its
    /// powerup offset, the building's `+0xc1c` group (unsaved), the game's events
    /// (`FUN_140c898a0`), the player's queue counts (`+0x18`, `FUN_14104f330`) and per-master
    /// counts (slot 0x398's `+0x13b0`), the sounds and particles (`FUN_1411a52e0`). The unit's
    /// height is the building's (the map's elevation is not held).
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
        let site_master = self.master_of(p, building.life.master_id)?;
        let made = self.master_of(p, master)?.clone();
        if made.recyclable {
            return Err(Unsupported(
                "a new object of a recyclable master (a negative id)",
            ));
        }
        let gather = building.gather_point.ok_or(Unsupported(
            "a spawn without a gather point (the unit AI's command queue location)",
        ))?;
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
            gather: (gather.target != building.id).then_some([gather.point[0], gather.point[1]]),
            walls,
        };
        let moving = made
            .moving
            .clone()
            .ok_or(Unsupported("a new unit of a master that does not move"))?;
        let restriction = self
            .spatial
            .terrain
            .as_ref()
            .and_then(|t| t.restrictions.get(moving.restriction))
            .ok_or(Unsupported(
                "a new unit's terrain restriction the data file does not have",
            ))?;
        let placing = Placing {
            player: p,
            radius: made.radius,
            tall: made.tall,
            class: made.class,
            restriction,
            hill_mode: made.hill_mode,
            placement_terrains: made.placement_terrains,
        };
        let point = {
            let map = WorldMap { turns: self };
            let spatial = &self.spatial;
            spawn_point(&site, made.radius, &mut |[x, y]| {
                let [rx, ry] = made.radius;
                spatial.check_known([x - rx, y - ry, x + rx, y + ry])?;
                placement(&placing, &map, [x, y], false)
            })?
        };
        let ai_type = factory_type(made.class, master)?
            .ok_or(Unsupported("a new combat unit without a unit AI"))?;
        let standing = (0 <= made.standing_sprite)
            .then(|| self.graphics.get(&made.standing_sprite).copied())
            .flatten()
            .ok_or(Unsupported(
                "a new unit's standing sprite the data file does not have",
            ))?;
        let id = self.next_object_id;
        self.next_object_id += 1;
        let position = [point[0], point[1], position[2]];
        // the setups' draws
        if standing.flags & 4 != 0 && 1 < standing.facets {
            self.streams.get(Stream::Object).rand15();
        }
        ActiveSprite::start(standing, self.streams.get(Stream::Sprite));
        if standing.flags & 2 != 0 && 1 < standing.facets {
            self.streams.get(Stream::Object).rand15();
        }
        let ai = UnitAi::constructed(ai_type, position, self.streams.get(Stream::Ai))?;
        self.streams.get(Stream::Object).rand15();
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
        self.spatial.insert(id, placed.clone());
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
            v.sight_on(id, position, made.radius, false, false);
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
                dying_sprite: 0 <= made.death_sprite,
                death_spawn: made.death_spawn,
                death_facets: None,
                inside: false,
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
                substitute_count: 0,
                ignore_units: Vec::new(),
            }),
            attackable: true,
            pasture: -1,
            my_pasture: -1,
            combat: true,
            ..Unit::default()
        });
        let new = owner.objects.len() - 1;
        owner.add_attribute(IN_PRODUCTION, -pop)?;
        let (building_id, state) = (owner.objects[i].id, owner.objects[i].life.state);
        if let Some(ai) = owner.objects[i].ai.as_mut() {
            ai.post(
                Notify::own(building_id, ACTION_COMPLETED, [MAKE_ACTION, 0, 0]),
                state,
            )?;
        }
        self.gather(p, new, gather, building_id)?;
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

    /// `FUN_140ace9c0` with a gather point (`+0xc30` 1) and a building whose unit AI holds no
    /// command queue location (its slot 0x218): the new unit waits to move (slot 0x2a0 with 1,
    /// `+0x7f9`) and takes the work command on the target at the point (slot 0x128: z 0, its
    /// `+0xa14` 1, no carry test, any task). A target that is gone (a search for another of its
    /// class), one on the building itself (a garrison), a building target (slot 0x7e8) and a
    /// point without a target are not modelled.
    fn gather(
        &mut self,
        p: usize,
        new: usize,
        g: GatherPoint,
        building: i32,
    ) -> Result<(), Unsupported> {
        if g.target == -1 {
            return Err(Unsupported(
                "a gather point without a target (a move to the point)",
            ));
        }
        if g.target == building {
            return Err(Unsupported(
                "a gather point on its own building (a garrison)",
            ));
        }
        let (owner, target) = self.object(g.target).ok_or(Unsupported(
            "a gather point's target that is gone (a search for another)",
        ))?;
        if self.master_of(owner, target.life.master_id)?.kind == BUILDING {
            return Err(Unsupported("a gather point on a building (slot 0x7e8)"));
        }
        if let Some(m) = self.players[p].objects[new].mover.as_mut() {
            m.waiting = 1;
        }
        let work = Work {
            target: g.target,
            point: [g.point[0], g.point[1], 0.0],
            carry_test: false,
            filter: -1,
            notified: true,
        };
        self.order_work(p, new, &work)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MarvinBlue/426048731's TC 9149 at (104, 47), radii 2: its gather point (107.2451,
    /// 46.075863) lies right and below, so the right side's lower half (6) comes first among the
    /// sides the first pass tries; the villager (radius 0.2) steps by 0.5 from (106.5, 47.5).
    /// The sheep 9520 at (106.68, 47.15) blocks the first two candidates and stays (livestock in
    /// state 2), so each ends its spot; (106.5, 46.5) is free.
    #[test]
    fn the_tc_spawns_its_villager_below_the_sheep_on_its_right() {
        let site = Site {
            position: [104.0, 47.0, 0.0],
            radius: [2.0, 2.0],
            gather: Some([107.2451, 46.075863]),
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
            placement_terrains: false,
        };
        let map = Flat(vec![sheep(2)]);
        let at = |x, y| placement(&villager, &map, [x, y], false).unwrap();
        assert_eq!(at(106.5, 47.5), Tested::Blocked { stays: true });
        assert_eq!(at(106.5, 47.0), Tested::Blocked { stays: true });
        assert_eq!(at(106.5, 46.5), Tested::Free);
    }
}
