//! A repairer's Repair action (`TRIBE_Action_Repair`, type 106, vtable `0x142ed7b88`): the
//! action walks to its target (state 4, a `MoveTo` sub-action within the unit's weapon range),
//! faces it (state 6) and repairs it each update (state 7) until the target holds its most hit
//! points or the player cannot pay for more; a target out of reach makes it wait (state 10) or
//! fail (state 13).
//!
//! The work of one update is the unit's work rate times the task's work value times the world's
//! dt, of which each repairer past the first adds half ([`work_amount`]). The target's own repair
//! (a building's slot 0x8c8, `FUN_1411d37d0`; a unit's, `FUN_1412143b0`) pays a share of its
//! master's cost ([`crate::production::pay`]) and heals it (`FUN_141193b30`); the caller runs it
//! ([`RepairView::repair`]), since it reaches other objects and the player's resources.

use crate::move_to::{MoveTo, MoveWait};
use crate::unit_ai::GenieError;

mod action;
mod payment;
pub(crate) mod unit;

pub use self::action::work_amount;
pub use self::payment::{
    BUILDING_REPAIR_COST, Payer, Repaired, UNIT_REPAIR_COST, repair_building, repair_unit,
};
#[cfg(test)]
use crate::move_to::Walk;

/// The Repair action's type, which word 9 adds, and its tasks' action type.
pub const REPAIR: i16 = 0x6a;
/// The repair's action value (`0x26a`), which its notifies to the unit AI carry.
pub const REPAIR_ACTION: i32 = 0x26a;

/// The gap within which a repairer reaches its target (`DAT_142eb5690`).
pub const REACH: f32 = crate::math::image::repair::REACH;
/// What a moving target (`FUN_140fdd080`) or a scenario's game mode adds to the reach
/// (`DAT_142e8c150`).
const MOVING_REACH: f32 = crate::math::image::repair::MOVING_REACH;
/// The hit points below which a target in state 2 is not repaired (`DAT_142e7f384`), and the
/// share each repairer past the first adds to the work (the same constant).
const LEAST_HP: f32 = crate::math::image::repair::LEAST_HP;
const EXTRA_REPAIRER: f32 = crate::math::image::repair::EXTRA_REPAIRER;
/// The wait of state 10 (`0x3f800000`), and the repairer count's first (`DAT_14330eb8c`).
const ONE: f32 = crate::math::image::repair::ONE;

/// The Repair action's states.
const DONE: u8 = 1;
const HALTED: u8 = 2;
const IDLE: u8 = 3;
const MOVE: u8 = 4;
const FACE: u8 = 6;
const WORK: u8 = 7;
const WAIT: u8 = 10;
const WALK_POINT: u8 = 11;
const FAILED: u8 = 13;

/// The ship classes whose repair meets them at the shore (`FUN_140ad9120`: 20, 21, 22, 2, 53).
pub const SHIPS: [i16; 5] = [20, 21, 22, 2, 53];

/// A ship's repair: its meeting point (`FUN_140ad9120`: the unit's slot 0x428 toward the
/// shore, the ship's slot 0x340 there and its command count `+0x888`) is not modelled.
/// The masters a unit of task swap group 1 or 2 (`+0x7b8`) becomes for a repair command (slot
/// 0x378): its player's master table's entries at `+0x4e0` and `+0x6f0`, the repairers 156
/// and 222, when the player has them.
pub const REPAIRERS: [(u8, i16); 2] = [(1, 156), (2, 222)];
/// The repair of a ship (its meeting point, `FUN_140ad9120`), which is not modelled.
pub const SHIP_REPAIR: GenieError =
    GenieError::unsupported("the task finder's repair (106) of a ship (FUN_140ad9120)");

/// What a Repair works with: its task's id and work value (`+0x28`, `work_val1`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RepairTask {
    /// The task's id.
    pub id: i16,
    /// Its work value.
    pub work_value: f32,
}

/// The Repair action of a repairer (`TRIBE_Action_Repair`, type 106).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RepairAction {
    /// The target's id (`+0x30`), −1 for none.
    pub target: i32,
    /// The action state (`+0x18`), which word 9 adds.
    pub state: u8,
    /// Its `MoveTo` sub-action's state.
    pub move_to: Option<MoveTo>,
    /// Its `MoveTo`'s wait after the motion step gave up ([`MoveWait`]).
    pub move_wait: MoveWait,
    /// The point a `MoveTo` to a point walks to (`set_state(11)`).
    pub goal: Option<[f32; 3]>,
    /// `+0x38`..`+0x40`: the target's place as the action last took it (the constructor, states
    /// 4, 6 and 7), or the point the unit was sent to.
    pub point: [f32; 3],
    /// `+0x44`: state 10's wait.
    pub timer: f32,
    /// The task it repairs with (`+0x48`).
    pub task: RepairTask,
    /// The `MoveTo`'s range toward the target: the unit's master's weapon range (`+0x828`).
    pub range: f32,
    /// `+0x6c`: the action counts among its target's repairers (`+0x9e8` of the object
    /// `FUN_140ad9050` gives: the target, an annex's main building).
    pub counted: bool,
    /// `+0x60`: its end posts no notify 0x1fa.
    pub quiet: bool,
}

/// What a Repair's update reads of its target (the object its id names).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Repairee {
    /// Its state (`+0x640`).
    pub state: u8,
    /// Its hit points (`+0x278`).
    pub hp: f32,
    /// Its owner's id (`player+0x718`).
    pub owner: usize,
    /// The repairer's player counts its owner as its own or an ally (`FUN_141663dc0` with the
    /// ally flag).
    pub allied: bool,
    /// Its place and its master's radii.
    pub position: [f32; 3],
    /// Its master's radii.
    pub radius: [f32; 2],
    /// Its master's class.
    pub class: i16,
    /// It moves (`FUN_140fdd080`: a master of type 30 and up with a speed above 0).
    pub moves: bool,
}

/// What a Repair's update reads of the world and runs on it.
pub struct RepairView<'a> {
    /// The target, None when the world holds no object of its id.
    pub target: Option<Repairee>,
    /// The repairer's place and its master's radii.
    pub radius: [f32; 2],
    /// The game mode is one of `FUN_141302540`'s ([`crate::turn::scenario_mode`]).
    pub scenario: bool,
    /// The repairer's work rate: its `mWorkRatePowerup` (`+0xa30`, slot 0x6e8) plus its master's
    /// `work_rate` (`+0x798`), in that order.
    pub rate: f32,
    /// The repairers of the object `FUN_140ad9050` gives (its `+0x9e8`), None without one.
    pub repairers: Option<u8>,
    /// The world's time, which a turn toward the target reads.
    pub now: u32,
    /// The motion step giving up makes the `MoveTo` wait once (a human player's villager), and
    /// the unit AI's stance, which sets the wait ([`MoveWait::give_up`]).
    pub waits: bool,
    /// The unit AI's stance (`+0x1a9`).
    pub stance: u8,
    /// The unit's quick path to the target within [`REACH`] (slot 0x3e0, `canQuickPath`) over
    /// the world's objects.
    pub can_path: &'a mut dyn FnMut(&crate::spatial::Spatial) -> Result<bool, GenieError>,
    /// The target's repair by `amount` (a building's or a combat unit's slot 0x8c8, any other's
    /// slot 0x110): 0 to go on, 1 once it holds its most hit points, 2 when the player cannot pay.
    pub repair: &'a mut dyn FnMut(f32) -> Result<u8, GenieError>,
}

impl std::fmt::Debug for RepairView<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepairView")
            .field("target", &self.target)
            .field("rate", &self.rate)
            .field("repairers", &self.repairers)
            .finish_non_exhaustive()
    }
}

/// What a Repair's update leaves to its unit and player.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepairStep {
    /// The update returned 1 or 3: the action list removes the action, whose destructor
    /// (`FUN_140ad8fd0`) leaves its target's repairers ([`RepairStep::repairers`]).
    pub done: bool,
    /// The action let its target go (slot 0x68 with none, `FUN_140ada470` -> `FUN_140a7a230`:
    /// the target's slot 0x180): the caller's.
    pub released: bool,
    /// The notifies the unit's slot 0x328 posts to its unit AI, each with the repair's action
    /// value ([`REPAIR_ACTION`], 0, 0), in order: 0x1fa its end, 0x202 its search, 0x1f9 its
    /// failure, 0x1fb the player's want of resources.
    pub notifies: Vec<i32>,
    /// The unit takes its master's standing sprite (slot 0x68 with `+0x470`): the caller's.
    pub stands: bool,
    /// What the action did to its target's repairers (`+0x9e8` of `FUN_140ad9050`'s object).
    pub repairers: i8,
    /// The repair took the target to its most hit points, and a target of a type above 69 has
    /// the armor its charge's hits took back (`FUN_1412066a0`: `+0x9e4` zeroed): the caller's.
    pub restored: bool,
}

#[cfg(test)]
mod tests;
