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

mod gather_walk;
mod make_action;
mod new_unit;
mod objects;
mod placement_test;
mod research;
mod search;
mod terrains;
mod world_map;

use self::make_action::{RESEARCH_TIME_MODIFIER, WORLD_EFFECT};
pub(crate) use self::placement_test::NOT_MOVING;
pub use self::placement_test::{Near, PlaceMap, Placing, build_placement, placement};
use self::search::corner_tiles;
pub use self::search::spawn_point;
#[cfg(test)]
use self::search::{Spot, try_spot};
use self::terrains::hill_fails;
pub use self::terrains::{BuildSite, placement_terrains};
pub(crate) use self::world_map::WorldMap;
#[cfg(test)]
use crate::objects::lifecycle::Lifecycle;
#[cfg(test)]
use crate::unit_ai::GenieError;
#[cfg(test)]
use crate::world::{GatherPoint, Turns, Unit};

/// A unit smaller than this on both axes (`DAT_142f5d120`) steps by twice its radius plus
/// `GAP` (`DAT_142eb5690`).
const SMALL: f32 = crate::math::image::spawn::SMALL;
const GAP: f32 = crate::math::image::spawn::GAP;
/// The share of a side the first segments cover (`DAT_142eb880c`).
const SIDE_SHARE: f32 = crate::math::image::spawn::SIDE_SHARE;
/// No gather point (`DAT_14330ec20`).
const NONE: f32 = crate::math::image::spawn::NONE;
/// The placement test's objects lie within this many tiles of the point (`DAT_142eb9430`).
const REACH: f32 = crate::math::image::spawn::REACH;
/// A tile whose terrain's restriction value is this or less is not passable (`DAT_14330eb78`).
const PASSABLE: f32 = crate::math::image::spawn::PASSABLE;
/// How far inside its far edges a box's last tile lies (`DAT_142eb1e9c`).
const INSIDE: f32 = crate::math::image::spawn::INSIDE;

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
pub(crate) const START_RIGHT_Z: f32 = crate::math::image::spawn::START_RIGHT_Z;
/// The player's attribute a new soldier's unit AI reads: the current age (`player+0x100` at
/// `+0x30`, `FUN_140c12300`).
const CURRENT_AGE: i16 = 6;

/// How a new plain object ([`Turns::plain_object`]) that moves faces.
///
/// [`Turns::plain_object`]: crate::world::Turns::plain_object
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facing {
    /// Its drawn angle (the moving setup's, `FUN_141141640`).
    Drawn,
    /// The dead object's facet it remains of (`FUN_14119d490`, `FUN_14114d780`), None where the
    /// slice does not hold it.
    Dead(Option<u8>),
}

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

#[cfg(test)]
mod tests;
