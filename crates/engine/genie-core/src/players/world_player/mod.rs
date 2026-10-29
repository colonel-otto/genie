//! The world player's own update (`WorldPlayer::update`, `0x14163b100`, slot 0x100) and Gaia's
//! (`WorldPlayerGaia::update`, `0x141681cd0`, its slot 0x100). `BaseWorld::update` runs a
//! player's slot 0x100 after its groups (`0x1415518f0`); both end in `WorldPlayerBase::update`
//! (`0x141677450`: the plans, the objects, `Process_New_Tiles`), which [`World::tick_with`] runs
//! between [`World::update_world_player`] and [`World::world_player_tail`].
//!
//! Gaia's update only adds the update's seconds (`world+0x164`) to its own `update_time`
//! (`+0x1468`); the world player's `mUpdateTime` (`+0x7a4`) of Gaia stays 0. The world player's
//! update adds them to `mUpdateTime` and runs a loop once for each whole second of it
//! ([`World::world_player_second`]): the history, the explored share (attribute 22), the kill
//! ratio (attribute 44), the defeat check of a human or computer player and, while the player
//! plays, its resource trickles (attributes 230 to 233) and its mills' food (attribute 254).
//! After `WorldPlayerBase::update` its scout map takes the tiles its map explored
//! ([`World::world_player_tail`]).
//!
//! Not simulation, not held: the local player's farm and fish trap queues sent as commands once
//! after a load (`+0x1427`, `FUN_140e74d70`, `FUN_140e74e10`: GAME 0x10 and 0x13, which a
//! recording holds; a playback sends none, `FUN_141533c10`), the profiler (`+0x1448`), the nine
//! slots at `+0xeac` (a world time read, `FUN_1415300d0`, whose answer goes unused), the attack
//! alerts' list (`+0x798`, unsaved: `FUN_141612690` posts the spectator API's event,
//! `FUN_141778410`, and the `"?under_attack"` sound and chat; `FUN_1417787d0` reads the spectator
//! API), the statistics of the local or a computer player's income (`FUN_14097daf0`), the
//! resource panel's event (`FUN_140c898a0` 0x80 in the attribute add), the AI's turn and update
//! (`FUN_14154e310` on `world+0x7dc`, unsaved, and the AI's slot 0x28 at `+0x7a8`: a playback
//! runs neither, `FUN_140d2f9c0`'s `+1`). The units of a formation that came out at one gather
//! point (`+0x1428`, `+0x1438` above 1, `FUN_141614170`) are only listed (`FUN_14160c4f0`) under
//! the world's feature flags (`mUseFeatureFlags`, `DAT_143f01830`), 0 in every recording seen.
//!
//! 158041: the explored share `FUN_141285c50`, the human or computer test `FUN_141663f30`, the
//! world player's attribute add (slot 0xf8) `FUN_141616820` and its base's `FUN_14165d040`, the
//! scout map's set `FUN_140f4d5e0`.
//!
//! [`World::tick_with`]: crate::world::World::tick_with
//! [`World::update_world_player`]: crate::world::World::update_world_player
//! [`World::world_player_second`]: crate::world::World::world_player_second
//! [`World::world_player_tail`]: crate::world::World::world_player_tail

use crate::math::fmath::below_or_unordered;

mod counts;
mod income;
mod tech_tree;
mod update;

pub use self::tech_tree::{TechTree, TreeBuilding, TreeQuery, TreeUnit, UniqueUnit};
#[cfg(test)]
use crate::world::World;

/// One second (`DAT_14330eb8c`): the loop's step.
const SECOND: f32 = crate::math::image::world_player::SECOND;
/// One hundred (`DAT_14330ebcc`): attribute 22 is a percentage.
const PERCENT: f32 = crate::math::image::world_player::PERCENT;
/// A minute's seconds (`DAT_14330ebc8`): the trickles and the mills' food are rates a minute.
const MINUTE: f32 = crate::math::image::world_player::MINUTE;
/// The livestock food over which a mill's yield grows by one e-fold (`DAT_142eb0f54`).
const LIVESTOCK_SCALE: f32 = crate::math::image::world_player::LIVESTOCK_SCALE;
/// Until this world time (ms) the scout map takes only the explored tiles the player sees.
const EARLY_SCOUTING_MS: u32 = 5000;

/// Gaia's player type (`+0x10`), whose class is `WorldPlayerGaia`.
const GAIA_TYPE: u8 = 2;
/// The human player's type.
const HUMAN_TYPE: u8 = 1;
/// The computer player's type: its income fills its escrow.
const COMPUTER_TYPE: u8 = 3;
/// The mill's master, whose livestock makes food (`FUN_14104f960` with 0x44).
const MILL: i32 = 68;
/// The king's master, which the regicide rule asks the tracker for (`FUN_14104eea0` with 0x1b2).
const KING: i32 = 434;
/// The town centres' masters (`FUN_141658970`'s and `FUN_141658a30`'s keys 0x6d and 0x1bc).
const TOWN_CENTRES: [i32; 2] = [109, 444];
/// The wonder's master (`0x61b0` raw, `0x14163bb68`).
const WONDER: i16 = 276;
/// The builder's master (`FUN_141625a20`'s 0x76).
const BUILDER: i16 = 118;
/// The town centre a builder may place (`FUN_141625a20`'s 0x26d).
const BUILDABLE_TOWN_CENTRE: i16 = 621;
/// The classes that never keep their player (`0x48000800`: other 11, walls 27, flags 30).
const UNKEPT_LOW: u32 = 0x4800_0800;
/// The classes that never keep their player (`0x140204a000384006`: 1, 2, 14, 19, 20, 21, 37,
/// 39, 42, 49, 58, 60).
const UNKEPT_HIGH: u64 = 0x1402_04a0_0038_4006;
/// The buildings' class (3).
const BUILDING_CLASS: u32 = 3;
/// The towers' class (52).
const TOWER_CLASS: u32 = 52;
/// The packed siege's class (51).
const PACKED_CLASS: u32 = 51;
/// The unpacked siege's class (54).
const UNPACKED_CLASS: u32 = 54;
/// The ages' technologies (Feudal, Castle, Imperial: `0x3284`, `0x3304`, `0x3384` in the
/// player's technologies), whose state −1 puts their units out of the player's tree.
const AGES: [i32; 3] = [101, 102, 103];
/// A technology's state once researched.
const RESEARCHED: i16 = 3;
/// `FUN_140c7e6f0`: the civilisation the full tech tree reads, by the game's tagged value at
/// `+0x680` ([`crate::world::World::mode_680`]): 18 for 1, 59 for 2, else 0.
const fn full_tree_civ(mode_680: i32) -> i32 {
    match mode_680 {
        1 => 0x12,
        2 => 0x3b,
        _ => 0,
    }
}
/// The resource storage that keeps its player (`0x14163b86b`): attribute 11 …
const POPULATION_STORAGE: i16 = 11;
/// … with the store mode 0x20 (`0x14163b89d`).
const ALIVE_STORE_MODE: u8 = 0x20;

/// The player's attributes the update reads and writes.
mod attribute {
    /// Food: the first of the four resources (food, wood, stone, gold: 0 to 3).
    pub const FOOD: usize = 0;
    /// Gold.
    pub const GOLD: usize = 3;
    /// The population.
    pub const POPULATION: usize = 11;
    /// The units owned.
    pub const UNITS_OWNED: usize = 19;
    /// The kills.
    pub const KILLS: usize = 20;
    /// The explored share of the map, in percent.
    pub const EXPLORED: usize = 22;
    /// The kill ratio: kills less losses.
    pub const KILL_RATIO: usize = 44;
    /// The survival to the finish, which a defeat clears.
    pub const SURVIVAL: usize = 45;
    /// The gold counter, which gold with a statistic grows (`FUN_141616820`).
    pub const GOLD_COUNTER: usize = 49;
    /// The first of the gathered totals of food, wood and stone (166 to 168).
    pub const GATHERED: usize = 166;
    /// The gathered total of gold.
    pub const GOLD_GATHERED: usize = 169;
    /// The first of the trickles a minute of food, wood, stone and gold (230 to 233).
    pub const TRICKLES: usize = 230;
    /// The food a minute of the livestock in the player's mills.
    pub const MILL_FOOD: usize = 254;
    /// The town centre is not available (0 lets a builder place one, `FUN_141625a20`).
    pub const NO_TOWN_CENTRE: usize = 48;
    /// The town centres a player may have (`FUN_141625a20`).
    pub const TOWN_CENTRE_LIMIT: usize = 218;
}

/// The timers that keep a player in the game without a living object (the defeat check's first
/// test): each counts at 0 or more; −1 is none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VictoryTimes {
    /// The game's wonder countdown of the player (the game object's `+0x4ec[id]`,
    /// `FUN_140c88300`): the least time left of its wonders' actions (`TRIBE_Action_Wonder`,
    /// `FUN_140c91500`), −1 without one (`FUN_140c90040`).
    pub wonder: f32,
    /// `ruin_held_time` (`+0xdb0`, saved).
    pub ruins: f32,
    /// `artifact_held_time` (`+0xdb4`, saved).
    pub artifacts: f32,
}

/// No timer runs.
impl Default for VictoryTimes {
    fn default() -> Self {
        Self {
            wonder: -1.0,
            ruins: -1.0,
            artifacts: -1.0,
        }
    }
}

impl VictoryTimes {
    /// One of the timers runs (`COMISS` with 0, `JNC`: 0 or more, not NaN): the player stays in
    /// the game.
    #[must_use]
    pub fn running(&self) -> bool {
        [self.wonder, self.ruins, self.artifacts]
            .into_iter()
            .any(|t| !below_or_unordered(t, 0.0))
    }
}

/// Attribute `k` of `attributes`, 0 past them (the game's array holds them all).
fn get(attributes: &[f32], k: usize) -> f32 {
    attributes.get(k).copied().unwrap_or(0.0)
}

/// Attribute `k` of `attributes` takes `v`; none past them (the game's array holds them all).
fn set(attributes: &mut [f32], k: usize, v: f32) {
    if let Some(a) = attributes.get_mut(k) {
        *a = v;
    }
}

#[cfg(test)]
mod tests;
