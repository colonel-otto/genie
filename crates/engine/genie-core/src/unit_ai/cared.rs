//! What each unit-AI type cares for: the classes its searches look for, their radii and its
//! idle timeout.

use crate::GenieError;
use crate::math::rng::Well512;
use crate::unit_ai::{
    BOAR_AI, FISHING_AI, HUNTED_SIGHT_GROWTH, LION_AI, LIVESTOCK_AI, PREDATOR_AI, PRIEST_AI,
    SIEGE_AI, SIEGE_TOWER_AI, SOLDIER_AI, Surroundings, TOWER_AI, TRADE_AI, TRADE_BOAT_CLASS,
};

/// The hunted animal evades by its speed times this (`FUN_140c1eea0`, `DAT_142eb1c98`), and a
/// point at the map's far edge goes this far in (`FUN_140c4d140`, `DAT_14330eb8c`).
pub(super) const EVADE_REACH: f32 = crate::math::image::unit_ai::EVADE_REACH;
/// `runAwayFromAttackers` (`FUN_140c5e900`): the run is the unit's speed times this
/// (`DAT_142e8c150`); attackers closer together than `RUN_EPSILON` (`DAT_142eb0c60`) give a random
/// direction, each part `2 r / RUN_DRAW_SCALE - RUN_ONE` (`DAT_142eb69cc`, `DAT_14330eb8c`); a
/// defensive unit runs at most `RUN_LEASH` (`DAT_14330ebb8`) from its state position.
pub(super) const RUN_REACH: f32 = crate::math::image::unit_ai::RUN_REACH;
pub(super) const RUN_EPSILON: f32 = crate::math::image::unit_ai::RUN_EPSILON;
pub(super) const RUN_DRAW_SCALE: f32 = crate::math::image::unit_ai::RUN_DRAW_SCALE;
pub(super) const RUN_ONE: f32 = crate::math::image::unit_ai::RUN_ONE;
pub(super) const RUN_LEASH: f32 = crate::math::image::unit_ai::RUN_LEASH;
/// `evasiveMoveTo` (`FUN_140c4d140`): a coordinate that is not a number becomes the unit's own
/// plus one, or minus one above this (`DAT_14330ebb8`).
pub(super) const EVASIVE_NEAR: f32 = crate::math::image::unit_ai::EVASIVE_NEAR;
/// The class whose attackers make a computer's unit warn its group (`FUN_140c5e900`: 22).
pub(super) const CLASS_22: i16 = 22;
pub(super) const EVADE_EDGE: f32 = crate::math::image::unit_ai::EVADE_EDGE;

/// The classes a hunted animal cares about (`+0x190`, the constructor's list).
pub const HUNTED_CARED: [i32; 28] = [
    0, 4, 26, 24, 6, 25, 17, 10, 12, 35, 23, 36, 28, 18, 13, 44, 46, 43, 45, 47, 50, 51, 54, 55,
    56, 57, 19, 59,
];

/// The classes a lion cares about (`+0x190`, `FUN_140c11bd0`'s list; the game's `+0x6b4` of 1,
/// not met, adds 47, 18, 13 and 55).
pub const LION_CARED: [i32; 18] = [
    4, 0, 26, 24, 6, 25, 12, 35, 23, 36, 28, 44, 46, 45, 50, 56, 57, 9,
];

/// The classes a predator (`TribePreditorUnitAIModule`, factory type 3) cares about (`+0x190`,
/// `FUN_140c11da0`'s list).
pub const PREDATOR_CARED: [i32; 2] = [58, 4];

/// The classes a priest (factory type 9) cares about (`+0x190`, `FUN_140c11e30`'s 30 entries).
pub const PRIEST_CARED: [i32; 30] = [
    3, 0, 4, 26, 24, 6, 25, 17, 10, 12, 35, 23, 36, 28, 18, 13, 44, 46, 43, 45, 47, 50, 51, 54, 55,
    56, 57, 19, 52, 42,
];

/// The civilian's factory type (`TribeCivilianUnitAIModule`, the villagers').
pub const CIVILIAN: u32 = 8;

/// The classes a civilian cares about (`+0x190`, `TribeCivilianUnitAIModule`'s constructor
/// `FUN_140c115d0`).
pub const CIVILIAN_CARED: [i32; 39] = [
    3, 7, 8, 9, 10, 15, 17, 18, 32, 33, 27, 49, 39, 52, 58, 63, 0, 4, 26, 24, 6, 35, 25, 12, 23,
    36, 28, 22, 44, 46, 45, 47, 50, 52, 55, 56, 57, 19, 61,
];

/// What the civilian's constructor gives the base (`FUN_140c2e080` with 7): `+0x1e8`, which its
/// search adds to its radius (`FUN_140c63e20`).
pub const CIVILIAN_SEARCH_GROWTH: f32 = 7.0;

/// The civilian's search radius (`FUN_140c63e20`): its line of sight plus a tile, at least its
/// search radius, plus [`CIVILIAN_SEARCH_GROWTH`].
#[must_use]
pub fn civilian_search_radius(los: f32, search_radius: f32) -> f32 {
    let sight = los + 1.0;
    let radius = if sight <= search_radius {
        search_radius
    } else {
        sight
    };
    radius + CIVILIAN_SEARCH_GROWTH
}

/// The civilian's slot 0x1b0 (`FUN_140c1c4f0`): the kind of a target class (0 for 15, 1 for 7, 2
/// for 5, 31 and 33, 3 for 8, 4 for 32 and 63), −1 for any other.
#[must_use]
pub const fn civilian_kind(class: i32) -> i8 {
    match class {
        5 | 0x1f | 0x21 => 2,
        7 => 1,
        8 => 3,
        0xf => 0,
        0x20 | 0x3f => 4,
        _ => -1,
    }
}

/// The classes a soldier cares about (`+0x190`, `TribeSoldierUnitAIModule`'s constructor
/// `FUN_140c12300`; a computer player's adds 10).
pub const SOLDIER_CARED: [i32; 39] = [
    0, 3, 4, 26, 24, 6, 35, 25, 17, 12, 23, 36, 28, 18, 13, 27, 22, 20, 2, 21, 44, 46, 43, 45, 47,
    50, 51, 52, 53, 54, 55, 39, 56, 57, 19, 59, 49, 42, 61,
];

/// A computer player's soldier's classes: [`SOLDIER_CARED`], then 10 (`FUN_140c12300`).
pub const SOLDIER_CARED_COMPUTER: [i32; 40] = [
    0, 3, 4, 26, 24, 6, 35, 25, 17, 12, 23, 36, 28, 18, 13, 27, 22, 20, 2, 21, 44, 46, 43, 45, 47,
    50, 51, 52, 53, 54, 55, 39, 56, 57, 19, 59, 49, 42, 61, 10,
];

/// The classes a building cares about (`+0x190`, `TribeBuildingUnitAIModule`'s constructor
/// `FUN_140c11470`, its 39 entries in this order): the soldier's set, 52 moved behind 59.
pub const BUILDING_CARED: [i32; 39] = [
    0, 3, 4, 26, 24, 6, 35, 25, 17, 12, 23, 36, 28, 18, 13, 27, 22, 20, 2, 21, 44, 46, 43, 45, 47,
    50, 51, 53, 54, 55, 56, 57, 19, 59, 52, 49, 39, 42, 61,
];

/// The artifact's factory type (`TribeArtifactUnitAIModule`, class 1's: relic carts, the
/// monument, the torches, the rock church, the emperor in a litter, the treasure chest); its
/// slot 0x1e0 (`FUN_140c16de0`) answers it.
pub const ARTIFACT_AI: u32 = 14;

/// The classes an artifact cares about (`+0x190`, `TribeArtifactUnitAIModule`'s constructor
/// `FUN_140c11340`: the 0x84 bytes it copies from `DAT_142ef90f0` on, then 61).
pub const ARTIFACT_CARED: [i32; 33] = [
    0, 4, 26, 24, 6, 35, 25, 17, 12, 23, 36, 28, 18, 13, 22, 20, 2, 21, 44, 46, 43, 45, 47, 50, 51,
    53, 54, 55, 56, 57, 19, 59, 61,
];

/// The classes a tower cares about (`+0x190`, `TribeTowerUnitAIModule`'s constructor
/// `FUN_140c12960`): the soldier's and 10.
pub const TOWER_CARED: [i32; 40] = [
    0, 3, 4, 26, 24, 6, 35, 25, 17, 12, 23, 36, 28, 18, 13, 27, 22, 20, 2, 21, 44, 46, 43, 45, 47,
    50, 51, 52, 53, 54, 55, 39, 56, 57, 19, 59, 49, 10, 42, 61,
];

/// The classes a siege unit cares about (`+0x190`, `TribeSiegeUnitAIModule`'s constructor
/// `FUN_140c12040`: the soldier's first 28, then 53, 54, 55, 39, 56, 57, 19, 59, 39, 49, 42, 61:
/// 39 twice).
pub const SIEGE_CARED: [i32; 40] = [
    0, 3, 4, 26, 24, 6, 35, 25, 17, 12, 23, 36, 28, 18, 13, 27, 22, 20, 2, 21, 44, 46, 43, 45, 47,
    50, 51, 52, 53, 54, 55, 39, 56, 57, 19, 59, 39, 49, 42, 61,
];

/// The classes a trade boat (class 2) cares about (`+0x190`, `FUN_140c12ab0`: the four of
/// `DAT_142ef9020`, then 21, 53 and 52).
pub const TRADE_SHIP_CARED: [i32; 7] = [3, 22, 20, 2, 21, 53, 52];

/// The classes a trade cart (any other class of the trade ship's type) cares about (`+0x190`,
/// `FUN_140c12ab0`: the four of `DAT_142ef91f0`, then 19).
pub const TRADE_CART_CARED: [i32; 5] = [3, 17, 18, 52, 19];

/// The classes a fishing ship cares about (`+0x190`, `FUN_140c11860`: the four of each of
/// `DAT_142ef9020`, `DAT_142ef9130` and `DAT_142ef9160`, then 49).
pub const FISHING_CARED: [i32; 13] = [3, 22, 20, 2, 21, 5, 31, 33, 53, 52, 27, 39, 49];

/// The classes a unit AI of factory type `ai_type` cares about (its constructor's `+0x190` list),
/// which its searches read: the trade AI's by its unit's `class` (a boat's or a cart's), the
/// soldier's with 10 for a computer player's (`FUN_140c12300` asks the owner when the AI is made;
/// taken as its owner now), the lion's without the four the world's `+0x6b4` of 1 adds.
///
/// # Errors
///
/// Another type, whose list is not held here.
pub(crate) const fn cared_classes(
    ai_type: u32,
    class: i16,
    computer: bool,
) -> Result<&'static [i32], GenieError> {
    match ai_type {
        2 => Ok(&HUNTED_CARED),
        PREDATOR_AI => Ok(&PREDATOR_CARED),
        // the boar's (`FUN_140c11720`), the ranged units' (`FUN_140c11f80`, `FUN_140c11f50`) and
        // the livestock's (`FUN_140c11d70`) constructors give none: the base's (`FUN_140c2e080`
        // with 0) alone
        BOAR_AI | 6 | 7 | LIVESTOCK_AI => Ok(&[]),
        LION_AI => Ok(&LION_CARED),
        PRIEST_AI => Ok(&PRIEST_CARED),
        CIVILIAN => Ok(&CIVILIAN_CARED),
        BUILDING_AI => Ok(&BUILDING_CARED),
        TRANSPORT_AI => Ok(&TRANSPORT_CARED),
        SIEGE_TOWER_AI => Ok(&SIEGE_TOWER_CARED),
        TRADE_AI if class == TRADE_BOAT_CLASS => Ok(&TRADE_SHIP_CARED),
        TRADE_AI => Ok(&TRADE_CART_CARED),
        FISHING_AI => Ok(&FISHING_CARED),
        ARTIFACT_AI => Ok(&ARTIFACT_CARED),
        TOWER_AI => Ok(&TOWER_CARED),
        SOLDIER_AI if computer => Ok(&SOLDIER_CARED_COMPUTER),
        SOLDIER_AI => Ok(&SOLDIER_CARED),
        SIEGE_AI => Ok(&SIEGE_CARED),
        _ => Err(GenieError::unsupported(
            "a retarget search of a unit-AI type whose classes are not held",
        )),
    }
}

/// What a search of a unit AI of `ai_type` covers (`FUN_140c63e20`), its unit of `class`, line
/// of sight `los` and search radius (slot 0x2c8): its type's [`cared_classes`] and its radius
/// ([`search_radius`]), the civilian's and the fishing ship's grown by their `+0x1e8`
/// ([`civilian_search_radius`]).
///
/// # Errors
///
/// A unit AI type whose classes are not held.
pub(crate) fn search_area(
    ai_type: u32,
    class: i16,
    los: f32,
    radius: f32,
    computer: bool,
) -> Result<(&'static [i32], f32), GenieError> {
    let classes = cared_classes(ai_type, class, computer)?;
    let radius = if matches!(ai_type, CIVILIAN | FISHING_AI) {
        civilian_search_radius(los, radius)
    } else {
        search_radius(los, radius)
    };
    Ok((classes, radius))
}

/// The action values that need a live target (the target block's mask `0x4002080d` over the
/// value − `0x259`).
pub(super) const LIVE_TARGET_ACTIONS: [i32; 6] = [0x259, 0x25b, 0x25c, 0x264, 0x26a, 0x277];

/// The radius of the search `bestUnitToAttack` runs (`FUN_140c63e20`): the larger of the unit's
/// search radius (slot 0x2c8) and its line of sight plus 1, plus `+0x1e8`, which no save holds (0
/// in every unit probed).
pub(super) fn attack_search_radius(at: &Surroundings) -> f32 {
    search_radius(at.los, at.search_radius)
}

/// A unit AI's search radius (`FUN_140c63e20`'s, slot 0x2c8 against the line of sight): its
/// master's line of sight plus [`HUNTED_SIGHT_GROWTH`], or its search radius if that is as far.
pub(crate) fn search_radius(los: f32, search_radius: f32) -> f32 {
    let sight = los + HUNTED_SIGHT_GROWTH;
    if sight <= search_radius {
        search_radius
    } else {
        sight
    }
}

/// `UpdateAdjustedIdleTimeout` (`0x140c41e00`): `r % (base / 2) + 3 * base / 4`, one AI draw.
pub(super) fn adjusted_idle_timeout(base: u32, ai: &mut Well512) -> u32 {
    ai.rand15() % (base >> 1) + ((base * 3) >> 2)
}

/// The classes a transport ship cares about (`+0x190`, `TribeTransportShipUnitAIModule`'s
/// constructor `FUN_140c12c30`).
pub const TRANSPORT_CARED: [i32; 10] = [3, 22, 20, 2, 21, 53, 52, 27, 39, 49];

/// The classes a siege tower cares about (`+0x190`, `TribeSiegeTowerUnitAIModule`'s constructor
/// `FUN_140c11fb0`).
pub const SIEGE_TOWER_CARED: [i32; 2] = [27, 39];

/// The building unit AI's factory type (`TribeBuildingUnitAIModule`).
pub(super) const BUILDING_AI: u32 = 17;

/// The transport ship's factory type (`TribeTransportShipUnitAIModule`).
pub(super) const TRANSPORT_AI: u32 = 11;
