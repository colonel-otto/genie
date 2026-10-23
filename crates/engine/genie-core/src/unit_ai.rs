//! The unit AI of an idle unit on its owner's turn: the timers and the AI-stream draws behind
//! checkpoint word 9.
//!
//! 158041: the full update is `UnitAIModule` slot 4 (`0x140c66b10`); slot 73 (`0x140c567c0`) redraws
//! the retarget timer, slot 72 is the idle check (`processIdle`). Only an idle unit is modelled: no
//! order, no action, no target, no actions in its list. Anything else is [`Unsupported`]. A boar
//! (factory type 4, `TribeElephantUnitAIModule`, slot 4 `0x140c2c010`) wanders around its origin
//! ([`Home`]); its moves go to the unit as an [`AiMove`]. A hunted animal (type 2,
//! `TribeHuntedAnimalUnitAIModule`: deer, foxes) looks for the closest object of the classes it
//! cares about in its player's sight ([`Search`], answered by the caller) and otherwise wanders
//! around its origin ([`Hunted`]).
//!
//! The unit's actions post notifies to its unit AI ([`UnitAi::post`]), which the full update
//! processes ([`Notify`]); orders wait in a queue the AI runs once its unit can take one
//! ([`QueuedOrder`]), on its owner's turn and between turns ([`UnitAi::off_turn`]). What the AI
//! does to its unit comes back as [`Effect`]s, in order.

use crate::image::unit_ai::{ATTACKER_FAR, ATTACKER_TIE};
use crate::rng::Well512;

/// A notify to a unit AI (the save's `temp_notify`, 0x18 bytes): from `caller` to `recipient`, of
/// `kind`, with three parameters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Notify {
    /// The object that posts it.
    pub caller: i32,
    /// The object whose unit AI takes it.
    pub recipient: i32,
    /// The kind (named in [`crate::ai_kinds`]).
    pub kind: i32,
    /// Its three parameters.
    pub p: [i32; 3],
}

impl Notify {
    /// The notify an object posts to itself.
    #[must_use]
    pub const fn own(id: i32, kind: i32, p: [i32; 3]) -> Self {
        Self {
            caller: id,
            recipient: id,
            kind,
            p,
        }
    }

    /// The record as the save holds it.
    #[must_use]
    pub fn from_bytes(b: &[u8; 24]) -> Self {
        let w = |i: usize| i32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        Self {
            caller: w(0),
            recipient: w(4),
            kind: w(8),
            p: [w(12), w(16), w(20)],
        }
    }
}

/// The notify kinds the slice posts or processes, the game's names where its log lines give one
/// ([`crate::ai_kinds`], generated). `0x1fa`: an action completed (p1 the action value).
pub const ACTION_COMPLETED: i32 = crate::ai_kinds::NOTIFY_ACTION_COMPLETED;
/// `0x1fb`: an action's invalidation (p1 the action value).
pub const ACTION_INVALIDATED: i32 = crate::ai_kinds::NOTIFY_ACTION_INVALIDATED;
/// `0x1f9`: an action failed (p1 its action value).
pub const ACTION_FAILED: i32 = crate::ai_kinds::NOTIFY_ACTION_FAILED;
/// `0x1fc`: a `MoveTo`'s target left its point and its path's end did not follow (p1..p3 0).
pub const TARGET_MOVED: i32 = 0x1fc;
/// `0x215` (no log line): the motion step met a unit of its walked target's master that neither
/// player counts an ally (p1 its id).
pub const IN_THE_WAY: i32 = 0x215;
/// `0x200`: "`ActionExecuted`" (the soldier's log line), an attack's swing (p1 600, p2 the target).
pub const ACTION_EXECUTED: i32 = 0x200;
/// `0x202` ("Action %d has been invalidated or requires a search.", the villager's log line): an
/// action asks for another target (p1 its action value, p2 the target class, p3 a drop site).
pub const REQUIRES_SEARCH: i32 = 0x202;
/// `0x20b`: the unit changed hands (p1 the new owner).
pub const OWNER_CHANGED: i32 = 0x20b;
/// `500`: the unit took damage.
pub const DAMAGED: i32 = crate::ai_kinds::NOTIFY_DAMAGED;
/// `699`: a unit is no longer doing an action on this one.
/// Notify 699: an object no longer acts on the unit (its action let the unit go, `FUN_141086e60`).
pub const NO_LONGER_ON_ME: i32 = crate::ai_kinds::NOTIFY_NO_LONGER_ON_ME;

/// `0x2db`: the order an INTERACT queues on its target (slot 0x230 runs it as the unit's slot
/// 0x128 on the target, case `0x2db`/`0x325` of `FUN_140c5c170`, which logs no line to name it).
pub const ORDER_ON_TARGET: i32 = 0x2db;
/// A unit sent off a foundation's footprint (`FUN_14123bb20`): the foundation master's radii
/// times 100 and the foundation ([`UnitAi::sent_off`]).
pub const SENT_OFF: i32 = 0x216;
/// A unit sent off a footprint with a group (`FUN_14123bb20`'s group move).
pub const SENT_OFF_GROUP: i32 = 0x21b;
/// The action value of a unit moving off a footprint (slot 0x148, `FUN_140c4d0d0`), and of one
/// moving off with a group (the base handler's case 0x21b).
pub const MOVE_ASIDE_ACTION: i32 = 0x2bf;
/// The action value of a unit moving off a footprint with a group.
pub const GROUP_ASIDE_ACTION: i32 = 0x2c0;
/// The order on a foundation a villager sent off its footprint takes up again (slot 0x108,
/// `FUN_140c5c170` case 0x2de).
pub const FOUNDATION_ORDER: i32 = 0x2de;
/// [`SENT_OFF`]'s radii are hundredths (`DAT_14330ebcc`).
const RADIUS_SCALE: f32 = crate::image::unit_ai::RADIUS_SCALE;
/// The prey animals' class, whose move off a footprint runs (slot 0x148's seventh argument).
const PREY_CLASS: i16 = 9;
/// The villagers' class.
const VILLAGER_CLASS: i16 = 4;

/// What a notify handler (unit-AI slot 0x238) returns: go on with the next notify.
const GO_ON: u32 = 2;
/// The defend stance's leash, squared (`DAT_142ed6f20`).
const DEFEND_LEASH: f32 = crate::image::unit_ai::DEFEND_LEASH;
/// "Notify processing returned stop or new action, breaking."
const BREAK: u32 = 3;
/// "I'm dead, purging notifyQueue and returning FALSE."
const DEAD: u32 = 4;
/// "`ProcessIdle` produced an action."
const PRODUCED: u32 = 0xd;
/// The boar's unit-AI type (`TribePreditorUnitAIModule`).
const BOAR_AI: u32 = 4;
/// The soldier's factory type (`TribeSoldierUnitAIModule`).
pub(crate) const SOLDIER_AI: u32 = 13;
/// The tower's factory type (`TribeTowerUnitAIModule`, the town centre's too).
const TOWER_AI: u32 = 15;
/// The seconds without a sighting after which enemies found are the owner's news
/// (`DAT_142eb0f4c`).
const ENEMY_SIGHTING_GAP: f32 = crate::image::unit_ai::ENEMY_SIGHTING_GAP;

/// The notify queue's capacity of a unit AI of `ai_type` holding `count` notifies: the
/// factory's (`FUN_141215560`: 5 for the artifact's and the livestock's, types 14 and 19; 10
/// for the others), doubled until they fit.
#[must_use]
pub const fn notify_capacity(ai_type: u32, count: usize) -> usize {
    let mut c = if matches!(ai_type, 14 | 19) { 5 } else { 10 };
    while c < count {
        c *= 2;
    }
    c
}

/// A queued order (the save's `temp_order`, 0x34 bytes), which the order queue (`+0x18` count,
/// `+0x20` array) runs once the unit can take it ([`UnitAi::run_orders`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct QueuedOrder {
    /// `+0x0`: the issuing player.
    pub issuer: i32,
    /// `+0x4`: the order value (`0x2c6` a move, `0x2db` an order on a target).
    pub order: i32,
    /// `+0x8`: below 100 it waits while the unit AI has an action.
    pub priority: i32,
    /// `+0xc`, `+0x10`: the target and its owner (−1 for none; "T", "`TOwner`" in the log line of
    /// `FUN_140c55c90`).
    pub target: i32,
    /// The target's owner.
    pub target_owner: i32,
    /// `+0x14`..`+0x1c`: the point; `+0x20`: the range.
    pub point: [f32; 3],
    /// The range.
    pub range: f32,
    /// `+0x24`, `+0x28`: the point the unit's slot 0x2d8 shows (not held).
    pub shown: [f32; 2],
    /// `+0x2c` (a byte): 0 leaves the order in the queue.
    pub ready: bool,
    /// `+0x30`: the target's type (its master id, −1 for none).
    pub target_type: i32,
}

impl QueuedOrder {
    /// The record as the save holds it (`+0x2d`..`+0x2f` are padding).
    #[must_use]
    pub fn from_bytes(b: &[u8; 52]) -> Self {
        let w = |i: usize| i32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let f = |i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        Self {
            issuer: w(0),
            order: w(4),
            priority: w(8),
            target: w(12),
            target_owner: w(16),
            point: [f(20), f(24), f(28)],
            range: f(32),
            shown: [f(36), f(40)],
            ready: b[44] != 0,
            target_type: w(48),
        }
    }
}

/// What the unit AI does to its unit, in the order it does it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    /// stopAction (the unit's slot 0x3c0, `FUN_14108c160`): its action list goes
    /// (`FUN_140a98620`, each action's teardown) and a unit in state 2 takes its standing sprite
    /// (slot 0x68 with slot 0x798's).
    Stop,
    /// The AI's move (slot 0x130), which the unit takes (its slot 0x340).
    Move(AiMove),
    /// Slot 0x148 (`FUN_140c4d0d0`) forced: the unit's slot 0x340 to the point, which the caller
    /// runs after the update ([`crate::turn::Turns::move_aside`]).
    MoveAside(AiMove),
    /// `FUN_140c41e50`: the unit's last human order time (`+0x650`) is at most this.
    CapHumanOrder(u32),
    /// The queued order on a target (`0x2db`) the AI took: the unit's slot 0x128 on the target,
    /// then the order's priority becomes the AI's (the caller's, `Turns::order_on_target`).
    OrderOnTarget(QueuedOrder),
    /// The full update's tail that reads what the order on a target before it gave the AI
    /// ([`UnitAi::after_order`], the caller's once it ran the order).
    AfterOrder(AfterOrder),
    /// The unit's explore (slot 0x398, `FUN_141084db0`, with its argument 4 set) to the point
    /// (x, y): its Explore takes the point (slot 0x48), else a new Explore to it (`FUN_140a87d30`
    /// at the unit's height) replaces its actions (`FUN_140a98620`, slot 0x808).
    Explore([f32; 2]),
    /// `HuntObject`'s unit part (`FUN_140c4e220`): the unit's `stopAction` (the AI's slot 0xb0 with
    /// 1, slot 0x3c0), then its hunt command on the target (object slot 0x360,
    /// `FUN_1412151f0`, forced; the caller's, `Turns::hunt_command`, after the update).
    HuntObject(i32),
    /// `GatherObject`'s unit part (slot 0xd0, `FUN_140c4dd50`, forced): the unit's gather command
    /// on the target (object slot 0x368, `FUN_141085030`, forced; the caller's,
    /// `Turns::gather_command`, after the update).
    GatherObject(i32),
    /// The AI's move to an object (slot 0x138, `FUN_140c55490`, with its fourth argument 1): the
    /// unit's slot 0x348 (`FUN_141086440`, forced) replaces its actions (`FUN_140a98620`) with a
    /// `MoveTo` to the target within `range` showing the master's running sprite (the walking one
    /// without it) and starts it (slot 0x808).
    MoveToObject {
        /// The target.
        target: i32,
        /// The range within which the `MoveTo` arrives.
        range: f32,
    },
    /// `BuildObject` (the AI's slot 0xf0, `FUN_140c47d80`, with 1) on the foundation: the unit's
    /// slot 0x380 builds it, then the AI's action becomes the build action and its target the
    /// foundation (the caller's, `Turns::build_object`, after the update).
    BuildObject(i32),
}

/// What a full update that took an order on a target ([`Effect::OrderOnTarget`]) leaves for
/// after it: the game runs the order inside the update (slot 0x230), so the retarget check's
/// outcome and the idle part's end read the action, target and order it gave the AI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AfterOrder {
    /// The retarget check's timer ran out (it was redrawn): its outcome is to come.
    pub retargeted: bool,
    /// The unit's group as the update read it ([`Surroundings::group`]).
    pub group: Option<GroupState>,
}

/// The first move among a full update's effects.
#[must_use]
pub fn first_move(effects: &[Effect]) -> Option<AiMove> {
    effects.iter().find_map(|e| match e {
        Effect::Move(m) => Some(*m),
        _ => None,
    })
}

/// The unit-AI fields the idle update reads and writes, with their offsets in 158041.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UnitAi {
    /// The factory's type (`has_ai` in the save), which picks the class.
    pub ai_type: u32,
    /// `+0x3c`, −1 for none.
    pub current_order: i32,
    /// `+0x40`, the current order's priority (−1 for none): a queued order sets it.
    pub order_priority: i32,
    /// `+0x44`, −1 for none.
    pub current_action: i32,
    /// `+0x48`, −1 for none.
    pub current_target: i32,
    /// `+0x4c`, the target's class (−1 for none), and `+0x50`..`+0x58`, its point.
    pub target_type: i32,
    /// `+0x50`..`+0x58`: the target's point.
    pub target_point: [f32; 3],
    /// `+0x5c`, `desiredTargetDistance`: how near the follow order keeps the unit to its target
    /// (the constructor's 2).
    pub desired_distance: f32,
    /// `+0x64`, `+0x68`, `+0x6c`, `+0x70`: the order, action, target and target class before the
    /// current ones.
    pub last_order: i32,
    /// `+0x68`: the action before the current one.
    pub last_action: i32,
    /// `+0x6c`: the target before the current one.
    pub last_target: i32,
    /// `+0x70`: that target's class.
    pub last_target_type: i32,
    /// `+0x160`.
    pub idle_timer: u32,
    /// `+0x164`, the randomised timeout the idle timer is compared with.
    pub adjusted_idle_timeout: u32,
    /// `+0x168`, the base the adjusted timeout is drawn around.
    pub idle_timeout: u32,
    /// `+0x16c`.
    pub secondary_timer: u32,
    /// `+0x170`.
    pub look_around_timer: u32,
    /// `+0x174`.
    pub look_around_timeout: u32,
    /// `+0xbc`, the world time of the last retarget.
    pub last_retarget_time: u32,
    /// `+0x1cc`, `mRandomizedRetargetTimer`: the word-9 input.
    pub retarget_timer: u32,
    /// `+0x1a9`: 1 and 2 add leash checks to slot 73.
    pub stance: u8,
    /// `+0x1d4`, `mFormationType`: the formation the unit's group last took for it, which its
    /// next group's type counts (`FUN_14100b940`).
    pub formation_type: u8,
    /// `+0x1ac`, `+0x1b0`, `state_position`: where a unit with an order last updated, which a
    /// stance's leash measures from.
    pub state_position: [f32; 2],
    /// `+0x1d6`: non-zero skips the retarget check.
    pub retarget_disabled: u8,
    /// `+0x1b8`, `alertModeValue`: the last search found enemies.
    pub alert_mode: bool,
    /// `+0x1b4`, `time_since_enemy_sighting`: seconds since a search last found enemies; each full
    /// update adds the world's dt.
    pub time_since_enemy: f32,
    /// `+0x1c0` holds a patrol path (the save's `hasPatrolPath`).
    pub patrol_path: bool,
    /// `+0x1c8`, `mPatrolCurrentWaypoint`: the patrol path's waypoint the unit walks to.
    pub patrol_waypoint: i32,
    /// `+0x1a8`, `stop_after_target_killed_value`: a move does not let the current action take its
    /// point (`FUN_140c41c10`).
    pub stop_after_kill: bool,
    /// A boar's own fields (factory type 4).
    pub home: Option<Home>,
    /// A hunted animal's own fields (factory type 2).
    pub hunted: Option<Hunted>,
    /// `+0x15c`, `last_update_time_value`: what `FUN_140c5dc10` last cleared the notifies with
    /// (the owner's time on its turn, the world time on a MOVE or an ownership change).
    pub last_update_time: u32,
    /// The order queue, oldest first.
    pub orders: Vec<QueuedOrder>,
    /// A building's or tower's command queue (`+0x1f0`..`+0x1f8`, 0x34-byte orders the
    /// building command queue fills, saved as `savedOrder`), which each unit it trains takes.
    pub building_orders: Vec<QueuedOrder>,
    /// The notify queue (`+0x28` count, `+0x30` array), oldest first.
    pub notifies: Vec<Notify>,
    /// Its capacity (`+0x2c`, not saved): the factory's 10 (5 for the artifact's and the
    /// livestock's, types 14 and 19), doubled by a post that finds it full outside the
    /// processing (`FUN_140c42470`).
    pub notify_capacity: usize,
    /// `+0x38`: the notifies are being processed; a post that finds the queue full then goes to
    /// the buffer (`DAT_143e6f930`), which the processing's end flushes into the queue
    /// (`FUN_140c55270`).
    pub processing: bool,
    /// The buffered notifies (the game's buffer is global; one unit AI processes at a time).
    pub buffered: Vec<Notify>,
    /// This update gave the unit an action of its own (the attack command, from `AttackObject`):
    /// its action list is no longer as [`Surroundings::busy`] read it.
    pub commanded: bool,
    /// `+0xc0`, the attackers (count `+0xc8`): a set of object ids kept sorted
    /// (`FUN_140b43f30`), which a boar's damage notify adds to and the full update prunes; the
    /// save's `numberUnits` and `tempInt`.
    pub attackers: Vec<i32>,
}

/// A hunted animal's part of its unit AI (`TribeHuntedAnimalUnitAIModule`, factory type 2), set
/// when the AI is made (`FUN_140c11920`) and not saved: a loaded game makes it from the loaded
/// unit.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hunted {
    /// `+0x1f0`..`+0x1f8`: the unit's place when its AI was made.
    pub origin: [f32; 3],
    /// `+0x1fc`: the unit's slot 0x7b0 then (`+0xb74`, the work range of its master's last task
    /// of action type 6, `FUN_141206d50`): beyond it the animal goes home.
    pub leash: f32,
}

/// The hunted animal evades by its speed times this (`FUN_140c1eea0`, `DAT_142eb1c98`), and a
/// point at the map's far edge goes this far in (`FUN_140c4d140`, `DAT_14330eb8c`).
const EVADE_REACH: f32 = crate::image::unit_ai::EVADE_REACH;
/// `runAwayFromAttackers` (`FUN_140c5e900`): the run is the unit's speed times this
/// (`DAT_142e8c150`); attackers closer together than `RUN_EPSILON` (`DAT_142eb0c60`) give a random
/// direction, each part `2 r / RUN_DRAW_SCALE - RUN_ONE` (`DAT_142eb69cc`, `DAT_14330eb8c`); a
/// defensive unit runs at most `RUN_LEASH` (`DAT_14330ebb8`) from its state position.
const RUN_REACH: f32 = crate::image::unit_ai::RUN_REACH;
const RUN_EPSILON: f32 = crate::image::unit_ai::RUN_EPSILON;
const RUN_DRAW_SCALE: f32 = crate::image::unit_ai::RUN_DRAW_SCALE;
const RUN_ONE: f32 = crate::image::unit_ai::RUN_ONE;
const RUN_LEASH: f32 = crate::image::unit_ai::RUN_LEASH;
/// `evasiveMoveTo` (`FUN_140c4d140`): a coordinate that is not a number becomes the unit's own
/// plus one, or minus one above this (`DAT_14330ebb8`).
const EVASIVE_NEAR: f32 = crate::image::unit_ai::EVASIVE_NEAR;
/// The class whose attackers make a computer's unit warn its group (`FUN_140c5e900`: 22).
const CLASS_22: i16 = 22;
const EVADE_EDGE: f32 = crate::image::unit_ai::EVADE_EDGE;

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

/// The classes a building cares about (`+0x190`, `TribeBuildingUnitAIModule`'s constructor
/// `FUN_140c11470`, its 39 entries in this order): the soldier's set, 52 moved behind 59.
pub const BUILDING_CARED: [i32; 39] = [
    0, 3, 4, 26, 24, 6, 35, 25, 17, 12, 23, 36, 28, 18, 13, 27, 22, 20, 2, 21, 44, 46, 43, 45, 47,
    50, 51, 53, 54, 55, 56, 57, 19, 59, 52, 49, 39, 42, 61,
];

/// The classes a tower cares about (`+0x190`, `TribeTowerUnitAIModule`'s constructor
/// `FUN_140c12960`): the soldier's and 10.
pub const TOWER_CARED: [i32; 40] = [
    0, 3, 4, 26, 24, 6, 35, 25, 17, 12, 23, 36, 28, 18, 13, 27, 22, 20, 2, 21, 44, 46, 43, 45, 47,
    50, 51, 52, 53, 54, 55, 39, 56, 57, 19, 59, 49, 10, 42, 61,
];

/// A unit AI's search (`FUN_140c63e20`) as the idle checks run it: it queries its player's
/// objects in sight of `classes` within `radius` of `at` for a searcher of radii `size`
/// (`GetVisibleUnits`, mode 0, every relation), and `pick` chooses among them. The caller answers
/// with the object's id, −1 for none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Search<'a> {
    /// Where it searches from.
    pub at: [f32; 2],
    /// How far.
    pub radius: f32,
    /// The object classes it cares about.
    pub classes: &'a [i32],
    /// The searcher's radii.
    pub size: [f32; 2],
    /// What it picks of what it finds.
    pub pick: Pick,
}

/// What a [`Search`] picks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pick {
    /// `ClosestObject` (`FUN_140c488b0`) with its reachability check: the closest object of
    /// master class `group`, of relation `relation` (0 itself or friends, 1 neutrals, 2 enemies,
    /// 3 itself), in state `state`, of master `master` and below `workers` workers (each −1 for
    /// any), reachable ones only.
    Closest {
        /// The master class (−1 any).
        group: i32,
        /// The relation (0 itself or friends, 1 neutrals, 2 enemies, 3 itself; −1 any).
        relation: i32,
        /// The object state (−1 any).
        state: i32,
        /// The master (−1 any).
        master: i32,
        /// The workers cap (−1 none).
        workers: i32,
        /// The relation passed over (argument 9: 0 friends, 1 neutrals, 2 enemies, 3 itself;
        /// −1 none).
        spared: i32,
    },
    /// `bestUnitToAttack` (`FUN_140c44680`) with no current target and no extra objects, as the
    /// idle checks call it: of the relations from enemies down to `lowest` (2 in the scenario
    /// modes for a human player, else 0, Gaia), the object to attack, by the unit AI's attack
    /// check ([`can_attack`]). The search sets the alert mode
    /// (`+0x1b8`) when it finds enemies, and holds a second search from within 5 tiles in the same
    /// update to the first's answer, which the idle checks never ask.
    Attack {
        /// The lowest relation it looks at (0 Gaia, 2 in a scenario).
        lowest: usize,
        /// What the pick reads of the searching unit AI.
        seeker: Seeker,
    },
    /// The search alone (slot 0x50 as a `NoAttack` soldier's idle check calls it): no object, and
    /// whether it found enemies (the alert mode).
    Look,
}

/// What a unit AI's attack check (its slot 0xa0) and `bestUnitToAttack` read of the unit: its
/// owner's id, its state (`+0x640`), what it holds (`+0x308`) and its top action, and its
/// master's id, type, class, `attack_priority` (`+0x92c`), `missile_id` (`+0x83c`), weapon and
/// least ranges (slots 0x230, 0x240), charge type, blast (`+0x82c`, `+0x830`) and line of sight;
/// its place and top speed (slot 0x208).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attacker {
    /// The unit AI's type.
    pub ai_type: u32,
    /// The unit's player.
    pub player: usize,
    /// The owner is a computer player (`FUN_141659cf0`).
    pub computer: bool,
    /// The unit's state.
    pub state: u8,
    /// The kind it holds (`attribute_type_held`, `+0x308`).
    pub held: i16,
    /// Its top action's type, if any.
    pub top_action: Option<i32>,
    /// Its master's id.
    pub master: i16,
    /// Its master's type.
    pub kind: u8,
    /// Its master's class.
    pub class: i16,
    /// Its master's `attack_priority`.
    pub attack_priority: u8,
    /// Its master's missile, −1 for none.
    pub missile: i16,
    /// Its master's weapon range (slot 0x230, `+0x828`).
    pub range: f32,
    /// Its master's least range (slot 0x240, `+0x850`).
    pub min_range: f32,
    /// Its master's charge type: 6 and 7 add to the range against a target (slot 0x2f8).
    pub charge_type: i16,
    /// Its master's `area_effect_range` (`+0x82c`).
    pub area_effect: f32,
    /// Its master's `area_effect_level` (`+0x830`).
    pub blast_level: u8,
    /// Its master's line of sight (`+0x5d0`).
    pub los: f32,
    /// Its place.
    pub position: [f32; 3],
    /// Its master's radii.
    pub radius: [f32; 2],
    /// Its top speed (slot 0x208).
    pub speed: f32,
    /// The game mode is a scenario's.
    pub scenario: bool,
    /// Its firepower (`+0xa00`): the units inside a building add theirs.
    pub firepower: f32,
}

/// What a unit AI's attack check and `bestUnitToAttack` read of an object it found: the owner's
/// id, the master's class, type, id and `combat_level` (`+0x62c`), and the object's `+0x924`,
/// unit AI (`+0x6b0`), container (`+0x2f0`), targeting and attackable flags (`+0x689`,
/// `+0x68a`), `desolid_flag` (`+0xc60`), place and master's radii.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    /// The object's player.
    pub owner: usize,
    /// Its master's class.
    pub class: i16,
    /// Its master's type.
    pub kind: u8,
    /// Its master.
    pub master: i16,
    /// Its master's `combat_level`.
    pub combat_level: u8,
    /// The object's `+0x924`.
    pub capture: u8,
    /// It has a unit AI (`+0x6b0`).
    pub has_ai: bool,
    /// It is inside another (`+0x2f0`).
    pub inside: bool,
    /// `+0x689`, `unit_targeting_flag`.
    pub targeting: bool,
    /// `+0x68a`, `unit_attackable_flag`.
    pub attackable: bool,
    /// `+0xc60`, `desolid_flag`: a foundation no builder has started.
    pub desolid: bool,
    /// Its place.
    pub position: [f32; 3],
    /// Its master's radii.
    pub radius: [f32; 2],
}

/// What `bestUnitToAttack` reads of the unit AI that searches: its stance (`+0x1a9`), state
/// position (`+0x1ac`), order (`+0x3c`), action (`+0x44`), target (`+0x48`), the current and last
/// target's classes (`+0x4c`, `+0x70`) and whether it holds a patrol path (`+0x1c0`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seeker {
    /// `+0x1a9`.
    pub stance: u8,
    /// `+0x1ac`, `+0x1b0`.
    pub state_position: [f32; 2],
    /// `+0x3c`.
    pub order: i32,
    /// `+0x44`.
    pub action: i32,
    /// `+0x48`.
    pub target: i32,
    /// `+0x4c`.
    pub target_type: i32,
    /// `+0x70`.
    pub last_target_type: i32,
    /// `+0x1c0` holds a patrol path.
    pub patrol_path: bool,
}

impl Seeker {
    /// `canMoveToAttack` with no target (slot 0x1d0 with −1, `FUN_140c48420`; the slice holds no
    /// unit groups): with an order, or without one in a stance below 2.
    #[must_use]
    pub const fn can_move(&self) -> bool {
        self.order != -1 || self.stance <= 1
    }
}

/// The classes an attacker of `attack_priority` 3 may attack (`FUN_140fddf10`).
const PRIORITY_3_CLASSES: [i16; 6] = [3, 27, 39, 49, 52, 60];

/// The masters a tower never attacks (`FUN_140c1a760`, raw `0x60ce`, `0x653f`).
const TOWER_SPARED: [i16; 2] = [50, 1187];

/// Slot 0xa0 of `me`'s unit AI with 0, `CanAttack`: whether it may attack `t`: the tower's
/// ([`tower_can_attack`]) or the soldier's ([`soldier_can_attack`]); the other types' are not
/// modelled.
pub fn can_attack(me: &Attacker, seeker: &Seeker, t: &Target) -> Result<bool, Unsupported> {
    match me.ai_type {
        15 => tower_can_attack(me, t),
        13 => soldier_can_attack(me, seeker, t),
        _ => Err(Unsupported(
            "an attack check (slot 0xa0) of a unit-AI type not modelled",
        )),
    }
}

/// The tower's attack check (`FUN_140c1a760`): never for Gaia's own tower nor the classes 1 and
/// 42; Gaia's objects only of the classes 9 and 10, unless they are of the combat type with
/// `+0x924` 0 outside the scenario modes; never a building (master type `'P'`) no builder has
/// started (`desolid_flag`, `+0xc60`), masters 50 and 1187, nor from a tower in a state below 2. Class 10 with an attack
/// priority below 3 only of combat level 4 (`FUN_140fde120`); else when the tower is armed
/// (`FUN_14120ea30`: classes 51 and 54, or a missile, or else a firepower, `+0xa00`, above 1)
/// and its attack priority is not 3 or the class one of [`PRIORITY_3_CLASSES`].
pub fn tower_can_attack(me: &Attacker, t: &Target) -> Result<bool, Unsupported> {
    if me.player == 0 || matches!(t.class, 1 | 42) {
        return Ok(false);
    }
    let gaia_combat = !me.scenario && 70 <= t.kind && t.capture == 0;
    if !gaia_combat && t.owner == 0 && !matches!(t.class, 9 | 10) {
        return Ok(false);
    }
    if (t.kind == 80 && t.desolid) || TOWER_SPARED.contains(&t.master) || me.state <= 1 {
        return Ok(false);
    }
    if t.class == 10 && me.attack_priority < 3 {
        return Ok(t.combat_level == 4);
    }
    if !matches!(me.class, 51 | 54) && me.missile == -1 && me.firepower <= ARMED_FIREPOWER {
        return Ok(false);
    }
    Ok(me.attack_priority != 3 || PRIORITY_3_CLASSES.contains(&t.class))
}

/// An attacker without a missile is armed with a firepower above this (`FUN_14120ea30`,
/// `DAT_14330eb8c`).
const ARMED_FIREPOWER: f32 = crate::image::unit_ai::ARMED_FIREPOWER;
/// A soldier never attacks with this master (raw `0x6258`).
const SOLDIER_NEVER: i16 = 444;
/// The master and class whose packed state (top action 124 or 125) attacks nothing (raw
/// `0x6109`; class 54).
const PACKED_MASTER: i16 = 109;
const PACKED_CLASS: i16 = 54;
/// What a unit carrying it never attacks with (`+0x308`).
const HELD_NO_ATTACK: i16 = 57;
/// The soldier's range against a target grows by this for its stance checks
/// (`DAT_142e7f384`).
const STANCE_RANGE_GROWTH: f32 = crate::image::unit_ai::STANCE_RANGE_GROWTH;
/// And the defend stance's by this more (`DAT_14330ebb8`).
const DEFEND_GROWTH: f32 = crate::image::unit_ai::DEFEND_GROWTH;

/// The squared gap between a point and an object's footprint (`FUN_140a847c0`: per axis the
/// distance less the radius, `MAXSS 0` of it, so a gap that is not a number stays one).
fn point_gap_squared(point: [f32; 2], at: [f32; 3], radius: [f32; 2]) -> f32 {
    let gap = |d: f32, r: f32| crate::fmath::maxss(0.0, d.abs() - r);
    let x = gap(point[0] - at[0], radius[0]);
    let y = gap(point[1] - at[1], radius[1]);
    y * y + x * x
}

impl Attacker {
    /// The weapon range against a target (slot 0x238, `FUN_14108d9b0`): the master's (slot
    /// 0x230); a charge of type 6 or 7 adds to it (slot 0x2f8), which is not modelled.
    ///
    /// # Errors
    ///
    /// A charge of type 6 or 7.
    pub const fn range_against(&self) -> Result<f32, Unsupported> {
        if matches!(self.charge_type, 6 | 7) {
            return Err(Unsupported(
                "an attacker whose charge adds to its range (slot 0x2f8)",
            ));
        }
        Ok(self.range)
    }
}

/// The soldier's attack check (`FUN_140c19a20`): never class 64; a Gaia soldier only objects of
/// the combat type with `+0x924` 0 outside the scenario modes (`FUN_141302540`, those it takes on
/// by sight), and of Gaia's objects only classes 9 and 10 or those; never an object whose
/// targeting flag (`+0x689`) is off. A computer player's tests (its priorities, ranges) are not
/// modelled. Never from class 51, master 444, a packed class 54 or master 109 (top action 124 or
/// 125), or a unit holding kind 57; never the classes 1, 42 and 60, nor a building no builder has
/// started (`+0xc60`); never in `NoAttack`. In `StandGround` (stance 2) only within its range
/// against the target plus 0.5 of its state position (by the squared footprint gap,
/// `FUN_140a847c0`), but on order `0x2bd`; in Defend (1) within 5 more, the patrol orders' and a
/// patrol path's tests not modelled. Then an attack priority of 3 takes only
/// [`PRIORITY_3_CLASSES`].
pub fn soldier_can_attack(me: &Attacker, s: &Seeker, t: &Target) -> Result<bool, Unsupported> {
    if t.class == 64 {
        return Ok(false);
    }
    let gaia_combat = !me.scenario && 70 <= t.kind && t.capture == 0;
    if me.player == 0 && !gaia_combat {
        return Ok(false);
    }
    if t.owner == 0 && !matches!(t.class, 9 | 10) && !gaia_combat {
        return Ok(false);
    }
    if !t.targeting {
        return Ok(false);
    }
    if me.computer {
        return Err(Unsupported("a computer player's soldier's attack check"));
    }
    if me.class == 51 || me.master == SOLDIER_NEVER {
        return Ok(false);
    }
    if (me.class == PACKED_CLASS || me.master == PACKED_MASTER)
        && matches!(me.top_action, Some(124 | 125))
    {
        return Ok(false);
    }
    if me.held == HELD_NO_ATTACK || matches!(t.class, 1 | 42 | 60) {
        return Ok(false);
    }
    if t.kind == 80 && t.desolid {
        return Ok(false);
    }
    if s.stance == 3 {
        return Ok(false);
    }
    let d2 = point_gap_squared(s.state_position, t.position, t.radius);
    let reach = me.range_against()? + STANCE_RANGE_GROWTH;
    match s.stance {
        2 if s.order != 0x2bd && reach * reach < d2 => return Ok(false),
        1 => {
            if matches!(s.order, 0x2c7 | 0x2dd) || s.patrol_path {
                return Err(Unsupported(
                    "a defending soldier's patrol range (slot 0x470, FUN_140f95e10)",
                ));
            }
            let reach = reach + DEFEND_GROWTH;
            if reach * reach < d2 {
                return Ok(false);
            }
        }
        _ => {}
    }
    Ok(me.attack_priority != 3 || PRIORITY_3_CLASSES.contains(&t.class))
}

/// `FUN_14120e4e0` for an attacker of type 70: one of attack priority 3 attacks only
/// [`PRIORITY_3_CLASSES`]; a ranged one (range above 1) with a blast (`+0x82c` above 0) whose
/// `area_effect_level` has neither bit 2 nor 3 (`FUN_1410bf160`, `FUN_1410bfcb0`) scans the tiles
/// of the blast around the target for its own or allied objects (not modelled).
fn spares_friends(me: &Attacker, t: &Target) -> Result<bool, Unsupported> {
    if me.attack_priority == 3 && !PRIORITY_3_CLASSES.contains(&t.class) {
        return Ok(false);
    }
    let ranged = 1.0 < me.range;
    if ranged && me.blast_level & 8 == 0 && 0.0 < me.area_effect && me.blast_level & 4 == 0 {
        return Err(Unsupported(
            "a blast attacker's scan for friends near its target (FUN_14120e4e0)",
        ));
    }
    Ok(true)
}

/// An object in `bestUnitToAttack`'s statistics (`FUN_140c66590`): its id, its footprints' gap
/// to the unit and whether it moves (its slot 0x208 above 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttackStat {
    /// The object.
    pub id: i32,
    /// The gap between its footprint and the unit's (`FUN_14118f680`).
    pub gap: f32,
    /// Its speed (slot 0x208) is above 0.
    pub moving: bool,
}

/// What `bestUnitToAttack` asks of the world while its unit AI runs: the tests of an object it
/// found ([`attack_candidate`] with what the world holds of it) and the choice among those that
/// went into the statistics ([`choose_attack`] with the unit's path tests).
pub trait AttackPicker {
    /// The object `id` of relation list `list` (`None`: the unit AI's current target) goes into
    /// the statistics.
    ///
    /// # Errors
    ///
    /// What the tests do not model.
    fn candidate(&self, id: i32, list: Option<usize>) -> Result<Option<AttackStat>, Unsupported>;

    /// The object to attack among the statistics, −1 for none.
    ///
    /// # Errors
    ///
    /// What the choice does not model.
    fn choose(&mut self, stats: &[AttackStat]) -> Result<i32, Unsupported>;
}

/// An [`AttackPicker`] for searches that pick no attack: a test of an object fails.
#[derive(Clone, Copy, Debug)]
pub struct NoAttack;

impl AttackPicker for NoAttack {
    fn candidate(&self, _: i32, _: Option<usize>) -> Result<Option<AttackStat>, Unsupported> {
        Err(Unsupported(
            "an attack test in a search that picks no attack",
        ))
    }

    fn choose(&mut self, _: &[AttackStat]) -> Result<i32, Unsupported> {
        Err(Unsupported(
            "an attack choice in a search that picks no attack",
        ))
    }
}

/// `bestUnitToAttack`'s choice (`FUN_140c44680` after its statistics) as the idle checks call it
/// (an idle check, path tests on, not in an attack work state): none without statistics. One
/// object needs no scoring (`FUN_140c48150`) nor sort (`FUN_140c41830`, `FUN_140c41b40`): it
/// passes the first test of the first three (it moves, or none of the objects moves) and is the
/// choice when the unit reaches it by the quick path (`quick`, slot 0x3e0, within the unit's range
/// against it); else the path test (slot 0x3d0) follows, whose failure gives the object an
/// ignored retry entry (`FUN_140c42320`), not modelled. Several objects are not modelled.
///
/// # Errors
///
/// What the choice does not model.
pub fn choose_attack(
    stats: &[AttackStat],
    quick: &mut dyn FnMut(i32) -> Result<bool, Unsupported>,
) -> Result<i32, Unsupported> {
    match stats {
        [] => Ok(-1),
        [one] => {
            if quick(one.id)? {
                Ok(one.id)
            } else {
                Err(Unsupported(
                    "bestUnitToAttack's path test after its quick path (slot 0x3d0)",
                ))
            }
        }
        _ => Err(Unsupported(
            "bestUnitToAttack's scoring of several candidates (FUN_140c48150)",
        )),
    }
}

/// The classes slot 0xa8 of the soldier's unit AI (`FUN_140c1ae60`) passes among neutral objects:
/// those of the mask `0x200200800838001c` (2, 3, 4, 19, 20, 21, 27, 39, 49, 61) never.
const NEUTRAL_SPARED: u64 = 0x2002_0080_0838_001c;

/// Slot 0xa8 of the soldier's unit AI (`FUN_140c1ae60`), whether it attacks a neutral object of
/// `class` (taken as unsigned, so a class outside `0..0x3e` passes the mask): never one of
/// [`NEUTRAL_SPARED`]'s classes, and with the unit's master's `attack_priority` 3
/// (`FUN_141202220`) only [`PRIORITY_3_CLASSES`] (`FUN_140fddf10`).
#[must_use]
pub fn soldier_attacks_neutral(attack_priority: u8, class: i16) -> bool {
    let class_bits = i32::from(class) as u32;
    if class_bits < 0x3e && NEUTRAL_SPARED >> class_bits & 1 != 0 {
        return false;
    }
    attack_priority != 3 || PRIORITY_3_CLASSES.contains(&class)
}

/// `bestUnitToAttack`'s (`FUN_140c44680`, as the idle checks call it: no reference distance, an
/// idle check, path checks on, no extra objects, the class filter off) tests of an object it
/// found in relation list `list` (4 enemies down), or of its current target (`None`): whether it
/// goes into the statistics, and its gap. An object without a unit AI (`+0x6b0`), inside another
/// (`+0x2f0`) or one the attack check passes ([`can_attack`]) is left; an attacker of type 70 also
/// asks [`spares_friends`]; a neutral one slot 0xa8 (the soldier's: not [`NEUTRAL_SPARED`] nor,
/// at attack priority 3, outside [`PRIORITY_3_CLASSES`]); a Gaia unit of the combat type taken by
/// sight is left outside the scenario modes, as are an object with its targeting flag off
/// (`+0x689`) and, while the attack action (600) is current, one not attackable (`+0x68a`). A
/// computer player's tests (`FUN_140c41690`, the ship and priority rules) are not modelled. Then
/// the footprints' gap (`FUN_14118f680`) must be at least the least range, within the weapon
/// range unless the unit moves (slot 0x208), and within twice the line of sight
/// (`FUN_140c47f90`, a human player's). Walls (27) and gates (39) count only for a unit that
/// attacks walls (its current or last target one), which is not modelled.
pub fn attack_candidate(
    me: &Attacker,
    s: &Seeker,
    t: &Target,
    list: Option<usize>,
) -> Result<Option<f32>, Unsupported> {
    if me.computer {
        return Err(Unsupported("a computer player's bestUnitToAttack"));
    }
    let gap = crate::movement::footprint_gap(me.position, me.radius, t.position, t.radius);
    let Some(list) = list else {
        // the current target: in the statistics when in range or the unit moves
        let range = me.range_against()?;
        return Ok((gap <= range || 0.0 < me.speed).then_some(gap));
    };
    if !t.has_ai || t.inside || !can_attack(me, s, t)? {
        return Ok(None);
    }
    if me.kind == 70 && !spares_friends(me, t)? {
        return Ok(None);
    }
    if list == 3 {
        if me.ai_type != 13 {
            return Err(Unsupported(
                "a neutral object's test (slot 0xa8) of a unit-AI type not modelled",
            ));
        }
        if !soldier_attacks_neutral(me.attack_priority, t.class) {
            return Ok(None);
        }
    }
    let gaia_unit = !me.scenario && t.owner == 0 && 70 <= t.kind;
    if gaia_unit && t.capture != 0 {
        return Ok(None);
    }
    if !t.targeting || (!t.attackable && s.action == 600) {
        return Ok(None);
    }
    let sight = me.los + me.los;
    let in_range = me.min_range <= gap && (gap <= me.range || me.speed != 0.0);
    if !in_range || !(s.can_move() || gap <= sight) || sight < gap {
        return Ok(None);
    }
    if matches!(t.class, 27 | 39) {
        let walls = matches!(s.target_type, 27 | 39) || matches!(s.last_target_type, 27 | 39);
        if walls {
            return Err(Unsupported(
                "bestUnitToAttack of a unit that attacks walls (the closest wall)",
            ));
        }
        return Ok(None);
    }
    Ok(Some(gap))
}

/// The classes a lion attacks by its own choice (its slot 0xa0's list, `FUN_140c18d50`; the
/// game's `+0x6b4` of 1, not met, adds 13, 18 and 55).
const LION_ATTACKS: [i16; 19] = [
    0, 4, 6, 9, 12, 23, 24, 25, 26, 28, 35, 36, 44, 45, 46, 47, 50, 56, 57,
];

/// The lion's slot 0xa0 (`FUN_140c18d50`) without its `any` flag: by the master's attack
/// priority 1 and 2 anything, 3 the classes of [`PRIORITY_3_CLASSES`], any other the classes
/// of [`LION_ATTACKS`].
fn lion_may_attack(attack_priority: u8, class: i16) -> bool {
    match attack_priority {
        1 | 2 => true,
        3 => PRIORITY_3_CLASSES.contains(&class),
        _ => LION_ATTACKS.contains(&class),
    }
}

/// The civilian's slot 0xa0 (`FUN_140c18b70`) without its `any` flag: never classes 1 and 42;
/// with the unit's master's `attack_priority` 3 (`FUN_141202220`) only [`PRIORITY_3_CLASSES`]
/// (`FUN_140fddf10`).
fn civilian_may_attack(attack_priority: u8, class: i16) -> bool {
    !matches!(class, 1 | 42) && (attack_priority != 3 || PRIORITY_3_CLASSES.contains(&class))
}

/// What [`UnitSide::attack`] answers: the unit attacks the target, and the notifies the command
/// posted to its unit AI (slot 0x328 with the kind and three parameters).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attacked {
    /// The unit attacks the target.
    pub attacks: bool,
    /// The notifies posted, in order.
    pub posted: Vec<(i32, [i32; 3])>,
}

/// The caller's answer to a [`Search`].
/// What the unit AI asks of its unit and the world while it runs, each answered at once (the game
/// calls them from inside the AI): its searches over the objects-in-sight registry
/// ([`Search`]) and the unit's own commands. A closure answers the searches alone.
pub trait UnitSide {
    /// A search (the unit AI's `FUN_140c63e20` with its pick).
    ///
    /// # Errors
    ///
    /// What the search does not model.
    fn closest(&mut self, search: &Search<'_>) -> Result<Found, Unsupported>;

    /// The unit's attack command on object `target` (slot 0x338, `FUN_14120cea0`), forced or
    /// not, while its unit AI stands as `ai`: true when the unit attacks it, with the notifies
    /// the command posts to the unit AI (kind and parameters), which the AI queues.
    ///
    /// # Errors
    ///
    /// What the command does not model.
    fn attack(&mut self, target: i32, force: bool, ai: &UnitAi) -> Result<Attacked, Unsupported>;

    /// The unit's `+0x685`, `underAttackValue`, becomes `value` (`FUN_14119d2d0`).
    fn set_under_attack(&mut self, value: bool);

    /// The unit's stopAction (slot 0x3c0) as the unit AI calls it, ahead of a command that
    /// follows it while the AI processes its notifies (an [`Effect::Stop`] otherwise waits for
    /// the AI's update to end).
    ///
    /// # Errors
    ///
    /// What the stop does not model.
    fn stop(&mut self) -> Result<(), Unsupported>;

    /// `AttackObject`'s test of object `target` (`FUN_140c43720`): the world holds it and the unit
    /// AI's attack check (slot 0xa0, [`can_attack`], as `seeker` stands) passes; then its master's
    /// class and its place.
    ///
    /// # Errors
    ///
    /// What the attack check does not model.
    fn may_attack(
        &mut self,
        target: i32,
        seeker: &Seeker,
    ) -> Result<Option<(i16, [f32; 3])>, Unsupported>;

    /// The explorer's point to explore (the base handler's case 0x202 for the auto scout): the
    /// search around its town centre (`FUN_140c65490`), else its unit AI's (`ai`) slot 0x90 with
    /// a draw of the AI stream (`rng`); None when neither finds one.
    ///
    /// # Errors
    ///
    /// What the searches do not model.
    fn seek_unexplored(
        &mut self,
        ai: &UnitAi,
        rng: &mut Well512,
    ) -> Result<Option<[i32; 2]>, Unsupported>;

    /// The auto scout's last search (`FUN_140c4b130`, [`crate::explore::beyond_sight`]), drawing
    /// from the AI stream (`rng`); None when it finds no point.
    ///
    /// # Errors
    ///
    /// What the search does not model.
    fn seek_beyond_sight(&mut self, rng: &mut Well512) -> Result<Option<[i32; 2]>, Unsupported>;

    /// The player's scout map, made when the auto scout's last search found nothing
    /// (`FUN_140c589c0`'s case 0x202).
    ///
    /// # Errors
    ///
    /// A unit without its player's sight.
    fn make_scout_map(&mut self) -> Result<(), Unsupported>;

    /// The builder's search for another foundation (`FUN_140c1c7c0`): −1 for none.
    ///
    /// # Errors
    ///
    /// What the search does not model.
    fn foundation_search(&mut self) -> Result<i32, Unsupported> {
        Err(Unsupported("a builder's search without the world"))
    }

    /// Slot 0x3d0: whether the unit reaches object `target` within `range`.
    ///
    /// # Errors
    ///
    /// What the reach test does not model.
    fn reaches(&mut self, _target: i32, _range: f32) -> Result<bool, Unsupported> {
        Err(Unsupported("a reach test without the world"))
    }

    /// What the base handler's case 0x216 reads ([`OffFootprint`]) for a unit sent off the
    /// footprint of `site` (its master's radii `radius`).
    ///
    /// # Errors
    ///
    /// What the spot's search does not model.
    fn off_footprint(
        &mut self,
        _site: i32,
        _radius: [f32; 2],
    ) -> Result<OffFootprint, Unsupported> {
        Err(Unsupported("a unit sent off a footprint without the world"))
    }

    /// The unit's slot 0x278: whether it could stand at `point`.
    ///
    /// # Errors
    ///
    /// What the test does not model.
    fn can_stand(&mut self, _point: [f32; 2]) -> Result<bool, Unsupported> {
        Err(Unsupported("a stand test without the world"))
    }

    /// The object `id` as the unit AI reads it ([`UnitSeen`]): None when the world does not
    /// hold it (`FUN_1415417a0`).
    ///
    /// # Errors
    ///
    /// An object the slice does not hold.
    fn seen_unit(&mut self, _id: i32) -> Result<Option<UnitSeen>, Unsupported> {
        Err(Unsupported("an object's look without the world"))
    }

    /// The unit AI's `ClosestResourceObject` (slot 0x80, `FUN_140c49760`) of gather group `kind`
    /// with fewer than `cap` workers (−1 any) from the drop site `site` (−1 none), `param_6` 2 and
    /// no point, passing over the AI's current target `current`: what it found, None for nothing.
    ///
    /// # Errors
    ///
    /// What the search does not model.
    fn closest_resource(
        &mut self,
        _kind: u8,
        _cap: i32,
        _site: i32,
        _current: i32,
    ) -> Result<Option<Gatherable>, Unsupported> {
        Err(Unsupported("a resource search without the world"))
    }
}

/// What the unit AI's `ClosestResourceObject` found ([`UnitSide::closest_resource`]): the object,
/// its place and its master's class.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gatherable {
    /// The object.
    pub id: i32,
    /// Its place.
    pub position: [f32; 3],
    /// Its master's class.
    pub class: i16,
}

/// An object as an order in the queue names it: its id, master, owner and the first two parts
/// of its slot 0x520 vector (its x and its height).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Named {
    /// The object.
    pub id: i32,
    /// Its master.
    pub master: i32,
    /// Its owner.
    pub owner: i32,
    /// Its slot 0x520 vector's x and second part (the height).
    pub vector: [f32; 2],
}

/// What a unit sent off a footprint finds (the base handler's case 0x216).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OffFootprint {
    /// The closest spot beside the footprint (`FUN_1411566b0` with 0), None when there is none
    /// (or no foundation).
    pub spot: Option<[f32; 2]>,
    /// The unit's height.
    pub z: f32,
    /// Its master's class.
    pub class: i16,
    /// Its owner is a computer player (`FUN_141659cf0`).
    pub computer: bool,
    /// Its owner's id.
    pub player: i32,
    /// The foundation, while the world holds it.
    pub site: Option<Named>,
    /// The target of its current action (`FUN_140a986f0`, `FUN_140a78450`).
    pub action_target: Option<Named>,
}

impl<F: FnMut(&Search<'_>) -> Result<Found, Unsupported>> UnitSide for F {
    fn closest(&mut self, search: &Search<'_>) -> Result<Found, Unsupported> {
        self(search)
    }

    fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, Unsupported> {
        Err(Unsupported("a unit AI's attack without its unit"))
    }

    fn set_under_attack(&mut self, _: bool) {}

    fn stop(&mut self) -> Result<(), Unsupported> {
        Err(Unsupported("a unit AI's stop without its unit"))
    }

    fn may_attack(&mut self, _: i32, _: &Seeker) -> Result<Option<(i16, [f32; 3])>, Unsupported> {
        Err(Unsupported("an attack test without the world"))
    }

    fn seek_unexplored(
        &mut self,
        _: &UnitAi,
        _: &mut Well512,
    ) -> Result<Option<[i32; 2]>, Unsupported> {
        Err(Unsupported("an explorer's search without its unit"))
    }

    fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, Unsupported> {
        Err(Unsupported("an explorer's search without its unit"))
    }

    fn make_scout_map(&mut self) -> Result<(), Unsupported> {
        Err(Unsupported("a scout map without the world"))
    }
}

/// What a [`Search`] answers: the object picked (−1 for none) and whether the lists it read hold
/// enemies (the fifth relation list), which sets the alert mode when its query ran (`fresh`; a
/// search that read the last answer leaves the alert mode as it is).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Found {
    /// The object picked, −1 for none.
    pub id: i32,
    /// The lists read hold enemies.
    pub enemies: bool,
    /// The object's place (x, y, z), for the caller's use.
    pub position: Option<[f32; 3]>,
    /// Its query ran (else it read the last answer).
    pub fresh: bool,
}

impl Found {
    /// Nothing found, no enemies.
    pub const NONE: Self = Self {
        id: -1,
        enemies: false,
        position: None,
        fresh: true,
    };
}

/// The builder's flood of `FUN_140c1c7c0` from tile `at` with reach `r`: the start is tested
/// first; then each tile it reached, the first queued first, tries its neighbours left, right,
/// below and above (x − 1, x + 1, y − 1, y + 1) while it lies less than `r − 1` from the start on
/// both axes, and a neighbour not yet reached is tested and queued. The first tile `test` names
/// an object on answers it.
pub fn flood_search(
    at: (i32, i32),
    r: i32,
    mut test: impl FnMut(i32, i32) -> Option<i32>,
) -> Option<i32> {
    let (ux, uy) = at;
    let mut reached = std::collections::HashSet::from([at]);
    if let Some(id) = test(ux, uy) {
        return Some(id);
    }
    let mut queue = vec![at];
    let mut next = 0;
    while let Some(&(qx, qy)) = queue.get(next) {
        next += 1;
        if (ux - qx).abs() >= r - 1 || (uy - qy).abs() >= r - 1 {
            continue;
        }
        for (x, y) in [(qx - 1, qy), (qx + 1, qy), (qx, qy - 1), (qx, qy + 1)] {
            if !reached.insert((x, y)) {
                continue;
            }
            if let Some(id) = test(x, y) {
                return Some(id);
            }
            queue.push((x, y));
        }
    }
    None
}

/// A search that never runs (units without a search).
pub const fn no_search(_: &Search<'_>) -> Result<Found, Unsupported> {
    Err(Unsupported("a unit AI's search the caller does not answer"))
}

/// A boar's part of its unit AI (`TribeElephantUnitAIModule`, factory type 4), which its idle check
/// reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Home {
    /// `+0x1f0`..`+0x1f8`, `origin`: the point it wanders around.
    pub origin: [f32; 3],
    /// `+0x1fc`, `furious`: it does not wander.
    pub furious: bool,
    /// `+0x1fd`: its next idle check sends it home; its slot 0x1f0 (`FUN_140c16d50`) sets it
    /// after a failed Attack ([`crate::attack::Attack::update`]). Not saved; a loaded boar
    /// starts without it.
    pub go_home: bool,
}

/// What an idle check reads of its unit and the game.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Surroundings {
    /// The unit's position (`+0x3c8`, `+0x4c8`, `+0x5a8`).
    pub position: [f32; 3],
    /// Its top speed (slot 0x208, `FUN_141202c70`; 0 for a unit that does not move), the map's
    /// size in tiles and the world's dt in seconds (`world+0x164`).
    pub speed: f32,
    /// The map's size in tiles.
    pub map: [i32; 2],
    /// The world's dt in seconds (`world+0x164`).
    pub dt_seconds: f32,
    /// The game's setting at `+0x680` (`FUN_140ec0cd0`) is 1: boars stay put.
    pub mode_1: bool,
    /// The unit's action list has an action (object slot 0x430, `FUN_14106d0e0`; its slot 0x818,
    /// `FUN_14106d100`, is 0).
    pub busy: bool,
    /// The owner's id (`player+0x718`); Gaia is 0.
    pub player: usize,
    /// The unit's master's class (decoded).
    pub class: i16,
    /// The master's `combat_level` (`+0x62c`).
    pub combat_level: u8,
    /// The master's `attack_priority` (`+0x92c`, `FUN_141202220`).
    pub attack_priority: u8,
    /// The master's radii (`+0x5d8`, `+0x5dc`), line of sight (`+0x5d0`) and search radius
    /// (`+0x794`, the unit's slot 0x2c8).
    pub size: [f32; 2],
    /// The master's line of sight (`+0x5d0`).
    pub los: f32,
    /// The search radius (`+0x794`, the unit's slot 0x2c8).
    pub search_radius: f32,
    /// Slot 0x230 (`FUN_14108d960`): its master's weapon range (`+0x828`).
    pub weapon_range: f32,
    /// The unit's slot 0x2d0 with 0 (`FUN_14106d170`): its top action may be interrupted, or it
    /// has none.
    pub interruptible: bool,
    /// The master has a task of action type 0x99 (`FUN_1415c60b0`; the mule cart's): such a
    /// unit with an action or a target takes no queued order (`FUN_140c4eea0`).
    pub task_0x99: bool,
    /// An object of type 70 or more inside another (`+0x2f0`): it runs no queued order.
    pub garrisoned: bool,
    /// The unit's state (`+0x640`), type (`+0xf`), `+0x924` and master id.
    pub state: u8,
    /// The unit's type (`+0xf`).
    pub kind: u8,
    /// Its master has a missile (`+0x83c` 0 or more, `FUN_1410bfca0`).
    pub ranged: bool,
    /// Its missiles per volley (`+0xa00`, `FUN_1412136d0`), 0 for a unit without volleys.
    pub firepower: f32,
    /// The unit's `+0x924`.
    pub capture: u8,
    /// The unit's master.
    pub master: i16,
    /// The game mode (the save's `mGameMode`, options `+0x34`) is one of `FUN_141302540`'s:
    /// Scenario (3), Campaign (4), King of the Hill (5), Co-op Campaign (15).
    pub scenario: bool,
    /// The master has a task of action type 0x69 (`FUN_1415c60b0`; the healers'), whose idle
    /// soldier looks for an ally to heal (`FUN_140c16f00`).
    pub task_0x69: bool,
    /// The unit AI's current target as the world's object table holds it (`FUN_1415417a0`),
    /// `None` when it does not exist or the unit AI has none.
    pub target: Option<Seen>,
    /// The unit's `+0x685`, `underAttackValue`.
    pub under_attack: bool,
    /// The unit's id (`+8`), which its attackers' unit AIs must target to stay attackers.
    pub id: i32,
    /// The unit's slot 0x1f8 (`FUN_14106a6c0`): its animation speed (`+0x7b8`); None while the
    /// slice does not know it.
    pub anim_speed: Option<f32>,
    /// The unit's slot 0x2b0 (`RGE_Action_Object::actionState`): its top action's state, 0
    /// without one; None for an action the slice does not hold.
    pub action_state: Option<u8>,
    /// The objects the unit AI names besides its target, as the world's object table holds them:
    /// its attackers ([`UnitAi::attackers`]) and the callers of its queued damage notifies. One
    /// the table does not hold is absent.
    pub units: Vec<UnitSeen>,
    /// The unit's group (`+0x6d0`, the world's `FUN_14152ff40`), when the world holds it: what
    /// the full update reads of it ([`GroupState`]).
    pub group: Option<GroupState>,
}

/// The idle part's group test (`0x140c67e8f`): no group, or one that is idle (`FUN_141017ad0`) of
/// a unit that does not move (slot 0x5c8).
fn group_lets_idle(group: Option<GroupState>) -> bool {
    group.is_none_or(|g| g.idle && !g.member_moving)
}

/// What the full update (`0x140c66b10`) reads of a unit's group.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GroupState {
    /// Its current command (`+0x3a8`) is not -1.
    pub commanded: bool,
    /// `IsIdle` (`FUN_141017ad0`, [`crate::group::UnitGroup::is_idle`]).
    pub idle: bool,
    /// It patrols: its top path's flags bit 0 (`FUN_141017c40`) or its flags bit 11
    /// (`FUN_141017c30`).
    pub patrol: bool,
    /// Its motion moves (`FUN_141046600`).
    pub moving: bool,
    /// The unit itself moves (its slot 0x5c8), which the idle part reads with the group.
    pub member_moving: bool,
}

impl Surroundings {
    /// The object `id` among [`Surroundings::units`].
    #[must_use]
    pub fn unit(&self, id: i32) -> Option<&UnitSeen> {
        self.units.iter().find(|u| u.id == id)
    }
}

/// An object a unit AI names, as its attackers' upkeep and `ClosestValidAttacker` read it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UnitSeen {
    /// `+8`.
    pub id: i32,
    /// Its master's class.
    pub class: i16,
    /// Its owner's id (`+0x138`, `player+0x718`); Gaia is 0.
    pub owner: usize,
    /// Its master's `combat_level` (`+0x62c`).
    pub combat_level: u8,
    /// `+0x640`.
    pub state: u8,
    /// Its unit AI's current target (`+0x48`); None without a unit AI.
    pub target: Option<i32>,
    /// The gap between its footprint and the unit's, squared (`FUN_14118f8c0`).
    pub gap_squared: f32,
    /// Its place.
    pub position: [f32; 3],
}

/// An object as a unit AI's target block reads it: its state (`+0x640`) and type (`+0xf`); as
/// the follow order reads it (slot 73): its owner, master's class, place (x, y, z), its master's
/// radii on the ground (`+0x5d8`, `+0x5dc`) and whether the unit's player sees its tile now
/// (`FUN_140c68a00` gives 0xf: the tile's bit of the unified map and the player's mask `+0x2a8`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Seen {
    /// The object's state.
    pub state: u8,
    /// Its master's type.
    pub kind: u8,
    /// Its owner.
    pub owner: usize,
    /// Its master's class.
    pub class: i16,
    /// Its place.
    pub position: [f32; 3],
    /// Its master's radii.
    pub radius: [f32; 2],
    /// Its tile is visible to the viewer.
    pub visible: bool,
    /// Its master's `object_flags` (`+0x654`).
    pub flags: u32,
}

/// The class whose busy units skip the idle check (`-0xe98` raw).
const CLASS_20: i16 = 20;
/// The class of the attackers and targets a villager's damage notify looks at (`-0xea2` raw).
const CLASS_10: i16 = 10;
/// The class whose stop leaves the state position (`-0xe9a` raw, slot 0xb0).
const CLASS_18: i16 = 18;
/// The patrol orders (`0x2c7`, `0x2dd`), whose ends reset the idle and secondary timers.
const PATROL: i32 = 0x2c7;
const PATROL_2: i32 = 0x2dd;
/// Classes whose actions `0x26b`, `0x26c` take another branch of the idle part (`-0xea9`,
/// `-0xe78` raw).
const CLASS_3: i16 = 3;
/// A building's master type (`'P'`, `FUN_1411942c0`).
const BUILDING_KIND: u8 = 80;
/// A building on its make action without a missile and with at most this firepower forgets its
/// target (`DAT_14330eb8c`).
const SINGLE_VOLLEY: f32 = crate::image::unit_ai::SINGLE_VOLLEY;
const CLASS_52: i16 = 52;

/// A move the unit AI gives its unit (unit-AI slot 0x130, `FUN_140c55620`, to the unit's slot
/// 0x340): a `MoveTo` to `point` within `range`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AiMove {
    /// The point.
    pub point: [f32; 3],
    /// The range within which it arrives.
    pub range: f32,
    /// The sixth argument: the move replaces even a `MoveTo` to the same point.
    pub force: bool,
    /// The seventh: the unit walks (its master's walking sprite, `+0x748`), not runs.
    pub walk: bool,
}

/// Why an update was not modelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unsupported(pub &'static str);

/// What slot 73 does for a factory type.
#[derive(Clone, Copy)]
enum Retarget {
    /// The base slot 73: the retarget check.
    Base,
    /// An override that never reaches the check: livestock (19, `0x140c21f20`: 9).
    Never,
    /// The villagers' override (8, `0x140c21da0`, [`UnitAi::civilian_misc`]), which never
    /// reaches the check either.
    Civilian,
}

const fn retarget_kind(ai_type: u32) -> Result<Retarget, Unsupported> {
    match ai_type {
        // villagers (0x140c21da0) and livestock (0x140c21f20) return 9 from slot 73
        8 => Ok(Retarget::Civilian),
        19 => Ok(Retarget::Never),
        // hunted animal (2, slot 72 FUN_140c1eea0), boar (4, slot 72 FUN_140c1e630), lion (5,
        // slot 72 FUN_140c1fb70), transport ship (11, slot 72 0x140c21d20), soldier (13, slot 72
        // FUN_140c20db0), tower (15, the base's slot 72 FUN_140c564c0), building (17, slot 72
        // 0x140c1e600, which does nothing)
        2 | 4 | 5 | 11 | 13 | 15 | 17 => Ok(Retarget::Base),
        _ => Err(Unsupported("unit-AI type not modelled")),
    }
}

/// The base idle timeout (`+0x168`) the constructor sets; the adjusted one is drawn around it.
const IDLE_TIMEOUT: u32 = 3000;
/// The villager class (`-0xea8` raw): a boar attacking one turns furious.
const CLASS_VILLAGER: i16 = 4;
/// The desired target distance (`+0x5c`) the constructor sets.
const DESIRED_DISTANCE: f32 = 2.0;

/// The boar's idle check (`FUN_140c1e630`): it stays within a draw of `0..81` plus `DAT_142eb5bbc`
/// (144) of its origin, squared; within `DAT_142eb0f48` (4) it wanders `-3..=2` tiles each way
/// (`(r % 6) - 3`), once in 7 checks; its moves come within `DAT_142e7f384` (0.5).
const BOAR_LEASH_DRAW: u32 = 81;
const BOAR_LEASH: f32 = 144.0;
const BOAR_HOME: f32 = 4.0;
const BOAR_WANDER: u32 = 7;
const BOAR_STEP: u32 = 6;
const BOAR_OFFSET: i32 = 3;
const AI_MOVE_RANGE: f32 = 0.5;
/// The move order and action values (`0x2c6`, `0x262`), which moves set.
pub const MOVE_ORDER: i32 = crate::ai_kinds::ORDER_MOVEMENT;
/// `0x321`: `AUTO_SCOUT`'s order.
pub const AUTO_SCOUT_ORDER: i32 = crate::ai_kinds::ORDER_AUTO_SCOUT;
/// `0x2c2`: the stop order ("I have a stop order.", after "SO Call 6"), which `AUTO_SCOUT` gives a
/// unit already scouting.
pub const STOP_ORDER: i32 = 0x2c2;
/// `0x2bd`: the auto scout's action value, which the order's case sets after the explore's.
pub const AUTO_SCOUT_ACTION: i32 = 0x2bd;
/// `0x25d`: the explore's action value (slot 0x100).
pub const EXPLORE_ACTION: i32 = crate::explore::EXPLORE_ACTION;
/// The height of the explore's target point (`FUN_140c4d910`, `DAT_14330eb8c`).
const EXPLORE_Z: f32 = crate::image::unit_ai::EXPLORE_Z;
/// The villager's reach test of the foundation its search found (slot 0x3d0 in
/// `FUN_140c22bf0`, `DAT_142eb5690`).
const FOUNDATION_REACH: f32 = crate::image::unit_ai::FOUNDATION_REACH;
/// The move action value (`0x262`).
pub const MOVE_ACTION: i32 = 0x262;
/// The follow order (`0x2c8`, "I have a follow order for Unit #%d" in slot 73) and its action
/// value (`0x264`), which FOLLOW gives.
pub const FOLLOW_ORDER: i32 = 0x2c8;
/// The follow action value (`0x264`).
pub const FOLLOW_ACTION: i32 = 0x264;
/// The defend order (`0x2bd`, "I have a defend order for Unit #%d" in slot 73).
const DEFEND_ORDER: i32 = 0x2bd;
/// The follow order's target counts as moved when it is more than a tile off the AI's target
/// point on either axis (`0x140c567c0`: `DAT_14330ec20`, `DAT_14330eb8c`).
const FOLLOW_SLACK_LOW: f32 = crate::image::unit_ai::FOLLOW_SLACK_LOW;
const FOLLOW_SLACK_HIGH: f32 = crate::image::unit_ai::FOLLOW_SLACK_HIGH;

/// The hunted animal's idle check (`FUN_140c1eea0`): away from its origin by more than
/// `DAT_142eb0f48` (4, squared) it goes home once in 7 checks; otherwise it wanders 1 to 3 tiles
/// each way (`HUNTED_STEP`), in mode 1 once in 28 and by a tile. Its search reaches its line of
/// sight plus `DAT_14330eb8c` (1), or its search radius if that is as far, plus `+0x1e8` (unsaved,
/// 0 in every unit probed).
const HUNTED_HOME: f32 = 4.0;
const HUNTED_STEP: i32 = 3;
const HUNTED_SIGHT_GROWTH: f32 = 1.0;
/// What an idle check lowers the unit's last human order time to (`FUN_140fe0d10`), and below
/// which the moving object's update counts it down.
pub const HUMAN_ORDER_CAP: u32 = 5000;
/// The master whose idle soldier does nothing (`FUN_140c20db0`, raw `0x635e`).
const SOLDIER_IDLE_MASTER: i16 = 706;
/// The lion's idle timeout (`FUN_140c1fb70`, as its constructor `FUN_140c11bd0`).
const LION_IDLE_TIMEOUT: u32 = 6000;
/// The lion's idle timeout once its attack completed (`FUN_140c266f0`).
const LION_ATTACKED_TIMEOUT: u32 = 1000;
/// Outside mode 1 the lion attacks within this share of its line of sight (`FUN_140c1fb70`,
/// `DAT_142e7f384`).
const LION_SIGHT_SHARE: f32 = crate::image::unit_ai::LION_SIGHT_SHARE;
/// Finding no villager outside mode 1 it attacks within its line of sight over this
/// (`FUN_140c1fb70`, `DAT_142eb1c98`).
const LION_SIGHT_DIVISOR: f32 = crate::image::unit_ai::LION_SIGHT_DIVISOR;
/// The class of the king's branch of the hunted animal's idle check (`-0xe71` raw).
const CLASS_KING: i16 = 59;
/// The king's master that the branch passes over (raw `0x635c`).
const KING_UNLED: i16 = 704;

/// The walk back's gap and range (`FUN_140c5e7d0`: `DAT_14330eb8c`, 1.0, whose image role in
/// the unit AI's table is [`RUN_ONE`]'s).
const WALK_BACK: f32 = crate::image::unit_ai::RUN_ONE;

/// The action values the full update resets once the unit's actions ended (`0x140c66b10`, the
/// unit's slot 0x430 false): attack, `0x261`, move, hunt, `0x269`, `0x26b`, `0x26c`, build and
/// `0x2bf`.
const ENDED_ACTIONS: [i32; 9] = [600, 0x261, 0x262, 0x265, 0x269, 0x26b, 0x26c, 0x2be, 0x2bf];
/// The attack action value.
const ATTACK_ACTION: i32 = 600;
/// The attack order (`0x2bc`), which a work command's attack notifies (kind 9).
const ATTACK_ORDER: i32 = 700;
/// A human player's unit on the attack order with a range above this does not retarget
/// (processMisc).
const RETARGET_RANGE: f32 = crate::image::unit_ai::RETARGET_RANGE;
/// The action values that need a live target (the target block's mask `0x4002080d` over the
/// value − `0x259`).
const LIVE_TARGET_ACTIONS: [i32; 6] = [0x259, 0x25b, 0x25c, 0x264, 0x26a, 0x277];

/// The radius of the search `bestUnitToAttack` runs (`FUN_140c63e20`): the larger of the unit's
/// search radius (slot 0x2c8) and its line of sight plus 1, plus `+0x1e8`, which no save holds (0
/// in every unit probed).
fn attack_search_radius(at: &Surroundings) -> f32 {
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
const fn adjusted_idle_timeout(base: u32, ai: &mut Well512) -> u32 {
    ai.rand15() % (base >> 1) + ((base * 3) >> 2)
}

/// The unit-AI type the factory (`FUN_141215a90`) gives a new object of a master of `class`
/// with the volley missile `missile` (`+0x8d4`, −1 for none). The types are those
/// `FUN_141215560`'s switch names for its constructors: masters 1105 (`FUN_140c11fb0`, 0x12) and 1744, 1746
/// (`FUN_140c12040`, 0x10) by their id; then none for classes 5, 7, 8, 11, 14, 15, 16, 30 to 34,
/// 38, 42, 48 and 63; 0xe for class 1; 0xa for 2 and 19; the building's (0x11) for classes 3,
/// 27, 39, 49 and 60 without a volley missile, the tower's (0xf) for those with one and for 52;
/// the civilian's (8) for 4; the hunted animal's (2) for 9 and 59; 0x10 for 13, 51, 54 and 55;
/// 9 for 18, 43 and 53; 0xb for 20; 0xc for 21; 0x13 for 29 and 58; the soldier's (0xd) for any
/// other. Class 10 takes the boar's (4) or the lion's (5) by its master's class 10 and combat
/// level 1 (`FUN_140fde4f0`), which the arguments do not carry.
pub const fn factory_type(
    class: i16,
    master: i16,
    missile: i16,
) -> Result<Option<u32>, Unsupported> {
    match master {
        1105 => return Ok(Some(0x12)),
        1744 | 1746 => return Ok(Some(0x10)),
        _ => {}
    }
    Ok(Some(match class {
        5 | 7 | 8 | 11 | 14 | 15 | 16 | 30..=34 | 38 | 42 | 48 | 63 => return Ok(None),
        1 => 0xe,
        2 | 19 => 0xa,
        3 | 27 | 39 | 49 | 60 if missile < 0 => 17,
        3 | 27 | 39 | 49 | 52 | 60 => TOWER_AI,
        4 => CIVILIAN,
        9 | 59 => 2,
        10 => {
            return Err(Unsupported(
                "a new object of class 10, whose type its master's combat level picks",
            ));
        }
        13 | 51 | 54 | 55 => 0x10,
        18 | 43 | 53 => 9,
        20 => 0xb,
        21 => 0xc,
        29 | 58 => 0x13,
        _ => SOLDIER_AI,
    }))
}

impl UnitAi {
    /// The unit AI a new object gets. `UnitAIModule`'s constructor (`0x140c2e080`) draws three
    /// AI-stream values, in this order: the last retarget time `r % 500`, the adjusted idle timeout
    /// `r % 1500 + 2250` and the retarget timer `750 + r % 500`. The factory (`FUN_141215a90`)
    /// picks the type's constructor; the building's (17, `TribeBuildingUnitAIModule`
    /// `0x140c11470`, whose cared classes are [`BUILDING_CARED`]) and the civilian's (8,
    /// `TribeCivilianUnitAIModule` `0x140c115d0`, [`CIVILIAN_CARED`]) are modelled, both of which
    /// set the stance (`+0x1a9`) to 0 after the base's 3; for any other type nothing is drawn.
    pub const fn constructed(
        ai_type: u32,
        position: [f32; 3],
        ai: &mut Well512,
    ) -> Result<Self, Unsupported> {
        if !matches!(ai_type, 8 | 17) {
            return Err(Unsupported("constructor of a unit-AI type not modelled"));
        }
        let last_retarget_time = ai.rand15() % 500;
        let adjusted_idle_timeout = adjusted_idle_timeout(IDLE_TIMEOUT, ai);
        let retarget_timer = 750 + ai.rand15() % 500;
        Ok(Self {
            ai_type,
            current_order: -1,
            order_priority: -1,
            current_action: -1,
            current_target: -1,
            target_type: -1,
            target_point: [-1.0; 3],
            desired_distance: DESIRED_DISTANCE,
            last_order: -1,
            last_action: -1,
            last_target: -1,
            last_target_type: -1,
            idle_timer: 0,
            adjusted_idle_timeout,
            idle_timeout: IDLE_TIMEOUT,
            secondary_timer: 750,
            look_around_timer: 0,
            look_around_timeout: 1000,
            last_retarget_time,
            retarget_timer,
            // the base's two bytes at `+0x1a8` (0x300), whose stance byte the type's own
            // constructor clears
            stance: 0,
            // the constructor's word at +0x1d4 (0x140c2e080)
            formation_type: 0,
            // the object's place
            state_position: [position[0], position[1]],
            retarget_disabled: 0,
            alert_mode: false,
            time_since_enemy: 0.0,
            patrol_path: false,
            patrol_waypoint: 0,
            stop_after_kill: false,
            home: None,
            hunted: None,
            last_update_time: 0,
            orders: Vec::new(),
            building_orders: Vec::new(),
            notifies: Vec::new(),
            notify_capacity: notify_capacity(ai_type, 0),
            processing: false,
            buffered: Vec::new(),
            commanded: false,
            attackers: Vec::new(),
        })
    }

    /// What an order (INTERACT, MOVE, RETREAT) does to each of its units' AI before it runs
    /// (`FUN_140e6cfc0`): the patrol path goes (`FUN_140c68de0`: `+0x1c0`, its waypoint `+0x1c8`
    /// 0); an adjusted idle timeout of 750 or more is redrawn
    /// (`UpdateAdjustedIdleTimeout`, unit-AI slot 53, `0x140c41e00`), and the secondary timer is
    /// capped at 750.
    pub fn reset_for_order(&mut self, ai: &mut Well512) {
        // FUN_140c68de0: the patrol path goes and its waypoint is 0
        self.patrol_path = false;
        self.patrol_waypoint = 0;
        if self.adjusted_idle_timeout > 749 {
            self.update_adjusted_idle_timeout(ai);
        }
        self.secondary_timer = self.secondary_timer.min(750);
    }

    /// `UpdateAdjustedIdleTimeout` (slot 0x1a8): the base draws around the idle timeout
    /// (`0x140c41e00`); a deer's or fox's (factory type 2, `FUN_140c17170`) is 5/4 of it.
    const fn update_adjusted_idle_timeout(&mut self, ai: &mut Well512) {
        self.adjusted_idle_timeout = if self.ai_type == 2 {
            self.idle_timeout.wrapping_mul(5) >> 2
        } else {
            adjusted_idle_timeout(self.idle_timeout, ai)
        };
    }

    /// The order setter with its history ([`UnitAi::set_order_with`] with 1).
    pub(crate) fn set_order(&mut self, order: i32, ai: &mut Well512) {
        self.set_order_with(order, true, ai);
    }

    /// The order setter (`FUN_140c64b60`): leaving a patrol order (711, 733) redraws an adjusted
    /// idle timeout of exactly 750 (slot 0x1a8) and caps the secondary timer at 750, the
    /// compare signed (`0x2ee < (int)+0x16c`); with `history` (its third argument) the order
    /// before becomes the last (`+0x64`), else the last stays; then the order.
    pub(crate) fn set_order_with(&mut self, order: i32, history: bool, ai: &mut Well512) {
        let leaving = |o| self.current_order == o && order != o;
        if leaving(PATROL) || leaving(PATROL_2) {
            if self.adjusted_idle_timeout == 750 {
                self.update_adjusted_idle_timeout(ai);
            }
            if 750 < self.secondary_timer as i32 {
                self.secondary_timer = 750;
            }
        }
        if history {
            self.last_order = self.current_order;
        }
        self.current_order = order;
    }

    /// The end of a Build whose AI holds queued orders (`0x140ab0370` case 0xdf): no order and no
    /// action (`FUN_140c64b60` with −1 and 1, `FUN_140c64b50` with −1); leaving a patrol order,
    /// whose timers reset, is not modelled.
    pub const fn end_order_and_action(&mut self) -> Result<(), Unsupported> {
        if matches!(self.current_order, PATROL | PATROL_2) {
            return Err(Unsupported("the end of a Build on a patrol order"));
        }
        self.last_order = self.current_order;
        self.current_order = -1;
        self.set_action(-1);
        Ok(())
    }

    /// `FUN_140c64b50`.
    pub(crate) const fn set_action(&mut self, action: i32) {
        self.last_action = self.current_action;
        self.current_action = action;
    }

    /// `FUN_140c64c10`: the target and its class become the last ones (`+0x6c`, `+0x70`), the
    /// target takes `id` and the point (`+0x50`..`+0x58`); the class (`+0x4c`) is the master's
    /// of the object the world holds under `id` (`FUN_1415417a0`; `class`), else it stays.
    pub(crate) const fn target_at(&mut self, id: i32, class: Option<i32>, point: [f32; 3]) {
        self.last_target = self.current_target;
        self.last_target_type = self.target_type;
        self.current_target = id;
        self.target_point = point;
        if let Some(class) = class {
            self.target_type = class;
        }
    }

    /// `FUN_140c64be0`: the target, its class and its point.
    pub(crate) const fn set_target(&mut self, target: i32, class: i32, point: [f32; 3]) {
        self.last_target = self.current_target;
        self.last_target_type = self.target_type;
        self.target_type = class;
        self.target_point = point;
        self.current_target = target;
    }

    /// The work command's notify of its new action (`FUN_14106e4e0`, object slot 0x828): the unit
    /// AI takes the order and action values of the action's type, and the target with its class
    /// (None for a point) and the order's point (`FUN_140c64b60`, `FUN_140c64b50`,
    /// `FUN_140c64be0`); then, when `clears`, `FUN_140c64e50` clears the notifies with time 0
    /// (`FUN_140c5dc10`; for a unit of type 70 or more only with its `+0xa14`, which is 1 but
    /// inside slot 0x128's work command, where INTERACT passes 1 and the queued order on a target
    /// 0). The order history (`FUN_140c42080`) is not held.
    pub fn notify_work(
        &mut self,
        order: i32,
        action: i32,
        target: Option<(i32, i32)>,
        point: [f32; 3],
        clears: bool,
        ai: &mut Well512,
    ) -> Result<(), Unsupported> {
        self.set_order(order, ai);
        self.set_action(action);
        let (id, class) = target.unwrap_or((-1, -1));
        self.set_target(id, class, point);
        if clears {
            self.clear_notifies(0)?;
        }
        Ok(())
    }

    /// MOVE's notify (`FUN_140e67800`): a unit that took the move as a `MoveTo` (`moved`) gets the
    /// move order and action and forgets its target (`FUN_140c4dcb0`); every unit then takes the
    /// target (None for a point) and the order's point on the ground (`FUN_140c64be0`).
    pub fn notify_move(
        &mut self,
        moved: bool,
        target: Option<(i32, i32)>,
        point: [f32; 2],
        ai: &mut Well512,
    ) {
        if moved {
            self.set_order(MOVE_ORDER, ai);
            self.set_action(MOVE_ACTION);
            self.forget_targets();
        }
        let (id, class) = target.unwrap_or((-1, -1));
        self.set_target(id, class, [point[0], point[1], 0.0]);
    }

    /// MOVE's notify to a unit that took its cell of the grid (`FUN_140e67800`): the move order
    /// (`FUN_140c64b60`) and action (`FUN_140c64b50`), then no target and the cell on the ground
    /// (`FUN_140c64be0`); the target it had becomes its last as [`UnitAi::set_target`] does.
    pub fn notify_grid_move(&mut self, cell: [f32; 2], ai: &mut Well512) {
        self.set_order(MOVE_ORDER, ai);
        self.set_action(MOVE_ACTION);
        self.set_target(-1, -1, [cell[0], cell[1], 0.0]);
    }

    /// The AI's move (slot 0x130, `FUN_140c55620`): with `force` or no action value, the unit takes
    /// it (its slot 0x340, which the caller runs), and the AI holds the move action and the point
    /// without a target (`FUN_140c64c10` with −1, which leaves the target class). The game
    /// changes the AI only on slot 0x340's 1: the caller's slot 0x340 is `FUN_1410866e0`, which
    /// always gives 1, but for classes 51 and 54 (`FUN_141216c50`, their packing), which it
    /// does not run.
    const fn move_to(&mut self, m: AiMove) -> Option<AiMove> {
        if !m.force && self.current_action != -1 {
            return None;
        }
        self.set_action(MOVE_ACTION);
        // FUN_140c64c10 with -1: the world holds no object -1
        self.target_at(-1, None, m.point);
        Some(m)
    }

    /// `FUN_140c41e50`: the unit's last human order time drops to `FUN_140fe0d10`'s 5000, not on
    /// orders 701, 711, 712 and 733.
    fn cap_human_order(&self, fx: &mut Vec<Effect>) {
        if !matches!(self.current_order, 0x2bd | 0x2c7 | 0x2c8 | 0x2dd) {
            fx.push(Effect::CapHumanOrder(HUMAN_ORDER_CAP));
        }
    }

    /// The boar's idle check (slot 72, `FUN_140c1e630`): `FUN_140c41e50` first
    /// ([`UnitAi::cap_human_order`]). A furious boar, or any in mode 1, stays. Then one draw of
    /// `0..81` widens its leash: outside it, a boar not yet heading home goes home; inside, once
    /// in 7 checks (a second draw) it goes home from more than 2 tiles away, or wanders up to 3
    /// tiles (two more draws). Its moves keep its height. True when it produced an action (6, not
    /// 5): every way but staying and the 6 in 7 checks inside the leash that do nothing,
    /// including the one heading home already.
    fn boar_idle(&mut self, at: &Surroundings, ai: &mut Well512, fx: &mut Vec<Effect>) -> bool {
        self.cap_human_order(fx);
        let (produced, moved) = self.boar_wander(at, ai);
        fx.extend(moved.map(Effect::Move));
        produced
    }

    /// [`UnitAi::boar_idle`] past its cap: whether it produced an action, and its move.
    fn boar_wander(&mut self, at: &Surroundings, ai: &mut Well512) -> (bool, Option<AiMove>) {
        let Some(home) = self.home else {
            return (false, None);
        };
        if home.furious || at.mode_1 {
            return (false, None);
        }
        let [x, y, z] = at.position;
        let dx = x - home.origin[0];
        let dy = y - home.origin[1];
        let d2 = dy * dy + dx * dx;
        let leash = ai.below(BOAR_LEASH_DRAW) as i32 as f32 + BOAR_LEASH;
        let go_home = AiMove {
            point: [home.origin[0], home.origin[1], z],
            range: AI_MOVE_RANGE,
            force: true,
            walk: true,
        };
        if home.go_home {
            let m = self.move_to(go_home);
            if let Some(h) = self.home.as_mut() {
                h.go_home = false;
            }
            return (true, m);
        }
        // comiss; jbe: unordered counts as inside
        if d2.partial_cmp(&leash) != Some(std::cmp::Ordering::Greater) {
            if !ai.rand15().is_multiple_of(BOAR_WANDER) {
                return (false, None);
            }
            if d2 > BOAR_HOME {
                return (true, self.move_to(go_home));
            }
            let step = |ai: &mut Well512| (ai.rand15() % BOAR_STEP) as i32 - BOAR_OFFSET;
            let nx = step(ai) as f32 + x;
            let ny = step(ai) as f32 + y;
            return (
                true,
                self.move_to(AiMove {
                    point: [nx, ny, z],
                    ..go_home
                }),
            );
        }
        if self.target_point[0] == home.origin[0] || self.target_point[1] == home.origin[1] {
            return (true, None);
        }
        (true, self.move_to(go_home))
    }

    /// The hunted animal's idle check (slot 72, `FUN_140c1eea0`). `FUN_140c41e50` first
    /// ([`UnitAi::cap_human_order`]). A king (class 59) does nothing more (5) unless it is a
    /// computer player's, not of master 704, not inside another and without an order: then its
    /// unit's slot 0x7e8 on `FUN_140c16930`'s object, which is not modelled. Beyond its leash
    /// (squared, from the unit's footprint, `FUN_140a847c0`) it goes home. Then its search: with
    /// nothing found, one draw; once in 7 it goes home from more than 2 tiles away, else once in
    /// 7 (the same draw) it wanders, two more draws; mode 1's herd is not modelled. A found object
    /// it evades ([`UnitAi::evade`]). Its moves keep its height and come within 0.5, forced and
    /// walking. True when it moved (6), false for 5.
    /// `FUN_140c63e20`'s alert after a search with its argument 4 set (`ClosestObject`'s, 1 at every
    /// call probed): enemies found set the alert mode and start the time since the last sighting
    /// again (past 10 seconds it would tell the owner first, slot 0x2e8 with notify 0x210, which
    /// the slice does not hold); none clear the alert mode. A search that read the last answer
    /// leaves it.
    const fn alert(&mut self, found: Found, _dt: f32) {
        if !found.fresh {
            return;
        }
        self.alert_mode = found.enemies;
        if found.enemies {
            self.time_since_enemy = 0.0;
        }
    }

    /// The hunted animal's evade of the object its search found (`FUN_140c1eea0`): from its
    /// place away from the object by three times its speed (slot 0x208; one tile more in mode 1,
    /// not here), at its height, through `evasiveMoveTo` (slot 0x150, [`UnitAi::evasive_move`])
    /// with arguments 5 and 6 1 and 0 (`[RSP+0x20]`, `[RSP+0x28]`): an object on its own point
    /// gives coordinates that are not numbers, which become its own place plus or minus one;
    /// the point is held on the map and the unit moves there. True (6).
    fn evade(
        &mut self,
        at: &Surroundings,
        other: [f32; 3],
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, Unsupported> {
        let [x, y, z] = at.position;
        let reach = at.speed * EVADE_REACH;
        let (dx, dy) = (other[0] - x, other[1] - y);
        let d = crate::fmath::sqrt(dy * dy + dx * dx);
        let point = [x - (dx / d) * reach, y - (dy / d) * reach, z];
        let _ = self.evasive_move(point, (true, false), at, ai, side, fx)?;
        Ok(true)
    }

    fn hunted_idle(
        &mut self,
        at: &Surroundings,
        computer: bool,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, Unsupported> {
        self.cap_human_order(fx);
        if at.class == CLASS_KING {
            let led =
                at.master != KING_UNLED && !at.garrisoned && self.current_order == -1 && computer;
            if led {
                return Err(Unsupported(
                    "a computer player's king's idle check (FUN_140c16930, slot 0x7e8)",
                ));
            }
            return Ok(false);
        }
        self.hunted_wander(at, ai, side, fx)
    }

    /// [`UnitAi::hunted_idle`] for an animal that is not a king: its move, if any.
    fn hunted_wander(
        &mut self,
        at: &Surroundings,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, Unsupported> {
        let h = self
            .hunted
            .ok_or(Unsupported("a hunted animal without its origin"))?;
        let [x, y, z] = at.position;
        let go_home = AiMove {
            point: [h.origin[0], h.origin[1], z],
            range: AI_MOVE_RANGE,
            force: true,
            walk: true,
        };
        let leash = h.leash * h.leash;
        if 0.0 < leash {
            let gap = point_gap_squared([h.origin[0], h.origin[1]], at.position, at.size);
            if leash < gap {
                fx.extend(self.move_to(go_home).map(Effect::Move));
                return Ok(true);
            }
        }
        if at.mode_1 {
            return Err(Unsupported(
                "a hunted animal's idle check in mode 1 (its herd)",
            ));
        }
        let sight = at.los + HUNTED_SIGHT_GROWTH;
        let radius = if sight <= at.search_radius {
            at.search_radius
        } else {
            sight
        };
        let found = side.closest(&Search {
            at: [x, y],
            radius,
            classes: &HUNTED_CARED,
            size: at.size,
            pick: Pick::Closest {
                group: -1,
                relation: -1,
                state: -1,
                master: -1,
                workers: -1,
                spared: -1,
            },
        })?;
        self.alert(found, at.dt_seconds);
        if found.id != -1 {
            let other = found
                .position
                .ok_or(Unsupported("a found object without its place"))?;
            return self.evade(at, other, ai, side, fx);
        }
        let r = ai.rand15();
        if r.is_multiple_of(BOAR_WANDER) {
            let dx = x - h.origin[0];
            let dy = y - h.origin[1];
            if HUNTED_HOME < dy * dy + dx * dx {
                fx.extend(self.move_to(go_home).map(Effect::Move));
                return Ok(true);
            }
            let step = |ai: &mut Well512| {
                let n = (ai.rand15() % (2 * HUNTED_STEP as u32)) as i32;
                if HUNTED_STEP <= n {
                    n - 2 * HUNTED_STEP
                } else {
                    n + 1
                }
            };
            let sx = step(ai) as f32;
            let sy = step(ai) as f32;
            let wander = self.move_to(AiMove {
                point: [x + sx, sy + y, z],
                ..go_home
            });
            fx.extend(wander.map(Effect::Move));
            return Ok(true);
        }
        Ok(false)
    }

    /// The lion's idle check (slot 72, `FUN_140c1fb70`, factory type 5: wolves, bears).
    /// `FUN_140c41e50` first ([`UnitAi::cap_human_order`]); its idle timeout becomes 6000,
    /// redrawn (`FUN_140c41bc0`, slot 0x1a8: one draw). Its search looks for an enemy villager
    /// (class 4) in state 2, then for any enemy in state 2, from the same place (the second reads
    /// the first's answer). A found one within its reach (the footprints' squared gap,
    /// `FUN_14118f8c0`, against the reach squared) it attacks (`AttackObject`, slot 0xb8, not
    /// forced, [`UnitAi::attack_object`]): the reach is its line of sight, outside mode 1 half
    /// of it, or a third of it when it found no villager. The scenario modes' reach (a third by
    /// a setting's `+0x10` of 4) is not modelled. True (6) once it called `AttackObject`,
    /// whatever that did; false (5) when it found nothing within its reach.
    fn lion_idle(
        &mut self,
        at: &Surroundings,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, Unsupported> {
        self.cap_human_order(fx);
        self.idle_timeout = LION_IDLE_TIMEOUT;
        self.update_adjusted_idle_timeout(ai);
        let sight = at.los + HUNTED_SIGHT_GROWTH;
        let radius = if sight <= at.search_radius {
            at.search_radius
        } else {
            sight
        };
        let search = |group| Search {
            at: [at.position[0], at.position[1]],
            radius,
            classes: &LION_CARED,
            size: at.size,
            pick: Pick::Closest {
                group,
                relation: 2,
                state: 2,
                master: -1,
                workers: -1,
                spared: -1,
            },
        };
        let mut reach = if at.mode_1 {
            at.los
        } else {
            at.los * LION_SIGHT_SHARE
        };
        if at.scenario {
            return Err(Unsupported("a lion's reach in the scenario modes"));
        }
        // each search that queries sets the alert mode (`FUN_140c63e20`)
        let mut found = side.closest(&search(4))?;
        self.alert(found, at.dt_seconds);
        if found.id < 0 {
            found = side.closest(&search(-1))?;
            self.alert(found, at.dt_seconds);
            if !at.mode_1 {
                reach = at.los / LION_SIGHT_DIVISOR;
            }
        }
        if found.id == -1 {
            return Ok(false);
        }
        let Some(t) = side.seen_unit(found.id)? else {
            return Ok(false);
        };
        if 0.0 <= reach && reach * reach < t.gap_squared {
            return Ok(false);
        }
        let _ = self.attack_object(found.id, false, false, at, side, fx)?;
        Ok(true)
    }

    /// processIdle, slot 72 (`+0x240`) of each factory type's vtable: the hunted animal's
    /// ([`UnitAi::hunted_idle`]), the boar's ([`UnitAi::boar_idle`]), the lion's
    /// ([`UnitAi::lion_idle`]), the soldier's ([`UnitAi::soldier_idle`]) and the tower's, the
    /// base `FUN_140c564c0` ([`UnitAi::tower_idle`]); the villager's (`FUN_140c1e610`) and the
    /// transport ship's (`0x140c21d20`) only lower the unit's last human order time
    /// ([`UnitAi::cap_human_order`]); the building's (`FUN_140c1e600`) and the livestock's
    /// (`FUN_140c1fea0`) do nothing. True when it produced an action (6), false for 5. Its
    /// second argument (1 from the idle part, else 0) lets an order the soldier's and the base's
    /// slot 0x250 retry give 6, which no modelled type's slot 0x60 allows.
    fn process_idle(
        &mut self,
        at: &Surroundings,
        computer: bool,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, Unsupported> {
        match self.ai_type {
            2 => self.hunted_idle(at, computer, ai, side, fx),
            BOAR_AI => Ok(self.boar_idle(at, ai, fx)),
            5 => self.lion_idle(at, ai, side, fx),
            CIVILIAN | 11 => {
                self.cap_human_order(fx);
                Ok(false)
            }
            SOLDIER_AI => self.soldier_idle(at, computer, side, fx),
            TOWER_AI => self.tower_idle(at, computer, side, fx),
            17 | 19 => Ok(false),
            _ => Err(Unsupported(
                "processIdle (slot 0x240) of a unit-AI type not modelled",
            )),
        }
    }

    /// The soldier's idle check (slot 72, `FUN_140c20db0`, factory type 13). Before its cap
    /// (`FUN_140c41e50`, [`UnitAi::cap_human_order`]), a player that is out (`+0x89` 2; not held,
    /// 0 in every player probed) would stop, and slot 0x250 (`FUN_140c5d9a0`) retries no order,
    /// the soldier's slot 0x60 taking none as retryable. A Gaia unit stops unless it is of the combat type with `+0x924` 0 outside the
    /// scenario modes (`FUN_141302540`); one in `NoAttack` (stance 3) searches and returns to its
    /// state position ([`UnitAi::no_attack_idle`]); master 706 stops. Its target is the target of its top action
    /// within its range (slot 400, slot 0x1e8) with retargeting on (`+0x1d6` 0), else its current
    /// target; neither is modelled, nor a computer's branches (patrol data, priorities, evasion).
    /// Without one, `bestUnitToAttack` from its place ([`Pick::Attack`]): an object found it attacks
    /// (slot 0xb8 forced, [`UnitAi::attack_object`]; then, with retargeting on and off order
    /// 0x2bd, the order goes) and processIdle produced (true); a patrol order's saved patrol data
    /// is not modelled. Else a patrol order stops. The search leaves the alert mode clear, so `+0x1bc`
    /// stays. Then, outside a unit group (`+0x6d0` −1; its test is not held), a unit in Defend or
    /// `StandGround` (stance 1, 2) walks back to its state position ([`UnitAi::walk_back`]),
    /// which leaves the result 5; a patrol path (`+0x1c0`) and the healer's ally search
    /// (`FUN_140c16f00`, for a master with a task of action type 0x69) are not modelled.
    fn soldier_idle(
        &mut self,
        at: &Surroundings,
        computer: bool,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, Unsupported> {
        self.cap_human_order(fx);
        let searches = !at.scenario && 70 <= at.kind && at.capture == 0;
        if at.player == 0 && !searches {
            return Ok(false);
        }
        if self.stance == 3 {
            return self.no_attack_idle(at, computer, side, fx).map(|()| false);
        }
        if at.master == SOLDIER_IDLE_MASTER {
            return Ok(false);
        }
        if self.retarget_disabled == 0 {
            if at.busy {
                return Err(Unsupported(
                    "the top action's target of a soldier's idle check (slot 400)",
                ));
            }
        } else if self.current_target != -1 && at.target.is_some() {
            return Err(Unsupported("the current target of a soldier's idle check"));
        }
        if computer {
            return Err(Unsupported("a computer player's soldier's idle check"));
        }
        let found = side.closest(&Search {
            at: [at.position[0], at.position[1]],
            radius: attack_search_radius(at),
            classes: &SOLDIER_CARED,
            size: at.size,
            pick: Pick::Attack {
                lowest: if at.scenario { 2 } else { 0 },
                seeker: self.seeker(),
            },
        })?;
        self.attack_search_alert(found, computer)?;
        if found.id != -1 {
            if matches!(self.current_order, PATROL | PATROL_2) {
                return Err(Unsupported(
                    "a patrolling soldier's attack from its idle search (FUN_140c63d20)",
                ));
            }
            // slot 0xb8 forced (FUN_140c17550: the base's, then the unit's group, not held);
            // with retargeting on and off order 0x2bd the order goes (FUN_140c64b60 with -1, 0)
            if self.attack_object(found.id, true, false, at, side, fx)? {
                if self.retarget_disabled == 0 && self.current_order != 0x2bd {
                    // FUN_140c64b60 with -1 and 0: the order is no patrol order here (those
                    // returned above), so only the order changes
                    self.current_order = -1;
                }
                return Ok(true);
            }
        }
        if matches!(self.current_order, PATROL | PATROL_2) {
            return Ok(false);
        }
        // "I'm in Defend or StandGround, returning to state pos.": the walk back's result does
        // not make the processIdle's
        if matches!(self.stance, 1 | 2) {
            let _ = self.walk_back(at, fx);
        }
        if self.patrol_path {
            return Err(Unsupported("a soldier's patrol path in its idle check"));
        }
        if at.task_0x69 {
            return Err(Unsupported("a healer's search for an ally to heal"));
        }
        Ok(false)
    }

    /// The soldier's idle check in `NoAttack` (stance 3, `FUN_140c20db0`: "searching and returning
    /// to state pos."): the search alone from its place (slot 0x50, `FUN_140c63e20` with mode 1,
    /// [`Pick::Look`]) leaves the alert mode clear when it finds no enemies; then, but on the
    /// patrol orders, it walks back to its state position ([`UnitAi::walk_back`], its result
    /// unread); last the healer's ally search (`FUN_140c16f00`). Not modelled: enemies found (the
    /// alert mode on, its object `+0x1bc` from slot 0x1d8, and after 10 s without a sighting the
    /// player's notify 0x210, slot 0x2e8), a defend order's search (from its target's objects), a
    /// computer's cared classes (`+0x190` with 10) and the healer's search.
    fn no_attack_idle(
        &mut self,
        at: &Surroundings,
        computer: bool,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        if self.current_order == DEFEND_ORDER {
            return Err(Unsupported(
                "a defend order's search (its target's objects)",
            ));
        }
        if computer {
            return Err(Unsupported("a computer player's soldier's NoAttack search"));
        }
        let found = side.closest(&Search {
            at: [at.position[0], at.position[1]],
            radius: attack_search_radius(at),
            classes: &SOLDIER_CARED,
            size: at.size,
            pick: Pick::Look,
        })?;
        if found.enemies {
            return Err(Unsupported(
                "a NoAttack soldier's search that finds enemies",
            ));
        }
        if found.fresh {
            self.alert_mode = false;
        }
        if !matches!(self.current_order, PATROL | PATROL_2) {
            let _ = self.walk_back(at, fx);
        }
        if at.task_0x69 {
            return Err(Unsupported("a healer's search for an ally to heal"));
        }
        Ok(())
    }

    /// The base unit AI's idle check (slot 72, `FUN_140c564c0`), the tower's (factory type 15):
    /// slot 0x250 first retries no order (the tower's slot 0x60 taking none as retryable), then
    /// the inlined cap of `FUN_140c41e50` ([`UnitAi::cap_human_order`]). In `NoAttack` (stance
    /// 3) it only searches (slot 0x50 with mode 1 from its place, [`Pick::Look`] over its cared
    /// classes, which sets the alert mode) and gives 5; else `bestUnitToAttack` from its place
    /// ([`Pick::Attack`], as [`UnitAi::soldier_idle`]'s), then an object found is attacked
    /// ([`UnitAi::attack_object`] forced, slot 0xb8, which then sets `+0x1d5`, not held). True
    /// when it attacks (processIdle's 6).
    fn tower_idle(
        &mut self,
        at: &Surroundings,
        computer: bool,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, Unsupported> {
        self.cap_human_order(fx);
        if self.stance == 3 {
            let found = side.closest(&Search {
                at: [at.position[0], at.position[1]],
                radius: attack_search_radius(at),
                classes: &TOWER_CARED,
                size: at.size,
                pick: Pick::Look,
            })?;
            self.attack_search_alert(found, computer)?;
            return Ok(false);
        }
        let found = side.closest(&Search {
            at: [at.position[0], at.position[1]],
            radius: attack_search_radius(at),
            classes: &TOWER_CARED,
            size: at.size,
            pick: Pick::Attack {
                lowest: if at.scenario && !computer { 2 } else { 0 },
                seeker: self.seeker(),
            },
        })?;
        self.attack_search_alert(found, computer)?;
        if found.id == -1 {
            return Ok(false);
        }
        // slot 0xb8 with 1 (`+0x1d5` then 1, not held)
        self.attack_object(found.id, true, false, at, side, fx)
    }

    /// What `bestUnitToAttack` reads of this unit AI ([`Seeker`]).
    const fn seeker(&self) -> Seeker {
        Seeker {
            stance: self.stance,
            state_position: self.state_position,
            order: self.current_order,
            action: self.current_action,
            target: self.current_target,
            target_type: self.target_type,
            last_target_type: self.last_target_type,
            patrol_path: self.patrol_path,
        }
    }

    /// The alert mode after `bestUnitToAttack`'s search (`FUN_140c63e20` with its fourth argument
    /// 1): a query that ran sets it to whether it found enemies (`+0x1b8`); with enemies, after
    /// more than 10 s without a sighting (`+0x1b4`) the owner hears of it (its slot 0x2e8 with
    /// `0x210`, `FUN_14162dab0`: no case for it, only the reset of the old short pather's hulled
    /// area, and a computer player's AI's notify, not modelled), and the time restarts.
    fn attack_search_alert(&mut self, found: Found, computer: bool) -> Result<(), Unsupported> {
        if !found.fresh {
            return Ok(());
        }
        self.alert_mode = found.enemies;
        if found.enemies {
            if ENEMY_SIGHTING_GAP < self.time_since_enemy && computer {
                return Err(Unsupported(
                    "a computer player's notify of enemies sighted (0x210, slot 0x3c8)",
                ));
            }
            self.time_since_enemy = 0.0;
        }
        Ok(())
    }

    const fn is_idle(&self) -> bool {
        self.current_order == -1 && self.current_action == -1 && self.current_target == -1
    }

    /// The full update (slot 4, `0x140c66b10`; the boar's `0x140c2c010` adds its tail):
    /// `dt` is the time the owner accumulated since its last turn (`world+0x18c[player]`),
    /// `computer` the owner's type test (`player+0x10 == 3`). In the game's order: the target
    /// block ([`UnitAi::target_block`]), the state position of a unit with an order, the timers, the notifies ([`UnitAi::post`]: each through
    /// slot 0x238, then `FUN_140c5dc10`), the reset of an action value whose actions ended, the
    /// order check of a unit no longer busy, the order queue ([`UnitAi::run_orders`]), slot 73's
    /// retarget check, the idle part (slot 72 once the idle timer passes its timeout), the
    /// look-around check. What it does to the unit comes back as [`Effect`]s, which the caller
    /// applies in order. A unit in a group ([`Surroundings::group`]) keeps its state position
    /// while the group has a command, runs its order queue only while the group is idle, its idle
    /// check only while the group is idle and the unit still, and takes the patrol branches while
    /// the group patrols. The retarget entries and the attackers list (the save's, empty in every
    /// unit probed) are not modelled.
    pub fn update(
        &mut self,
        dt: u32,
        world_time: u32,
        computer: bool,
        at: &Surroundings,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
    ) -> Result<Vec<Effect>, Unsupported> {
        let retarget = retarget_kind(self.ai_type)?;
        self.commanded = false;
        // the full update's start: the time since the last enemy sighting grows by the world's dt
        self.time_since_enemy += at.dt_seconds;
        let mut fx = Vec::new();
        self.target_block(at, computer, ai, side, &mut fx)?;
        self.keep_attackers(at, side);
        self.check_busy_order(retarget, at.busy && fx.is_empty())?;
        // a unit with an order, or in a group with a command (`+0x3a8`), keeps its place as its
        // state position (0x140c674c9)
        if self.current_order != -1 || at.group.is_some_and(|g| g.commanded) {
            self.state_position = [at.position[0], at.position[1]];
        }
        self.secondary_timer = self.secondary_timer.saturating_sub(dt);
        self.look_around_timer = self.look_around_timer.wrapping_add(dt);
        self.idle_timer = self.idle_timer.wrapping_add(dt);
        let result = self.process_notifies(dt, at, computer, ai, side, &mut fx)?;
        if result == DEAD {
            self.boar_tail(at, computer, ai, side, &mut fx)?;
            return Ok(fx);
        }
        self.end_notifies(dt, at, &mut fx)?;
        // slot 0x430 again: a stop emptied the action list, an attack command filled it
        let stopped = fx.contains(&Effect::Stop);
        let busy = (at.busy && !stopped) || self.commanded;
        if !busy {
            self.reset_ended_action(at);
            if self.current_order != -1 {
                self.check_order(at)?;
            }
        }
        let interruptible = at.interruptible || stopped;
        if self.current_action == -1
            && (!busy || interruptible)
            && result != 0xd
            && self.can_order(at, false)
            && at.group.is_none_or(|g| g.idle)
            && !at.garrisoned
        {
            self.run_orders(at, &mut fx)?;
        }
        let ordered = fx.iter().any(|e| matches!(e, Effect::OrderOnTarget(_)));
        let mut retargeted = false;
        match retarget {
            Retarget::Base => {
                retargeted = self.retarget_check(world_time, computer, at, ai, ordered, &mut fx)?;
            }
            // with an order on a target taken, the game's slot 73 reads what the order's work
            // gave the AI, which the caller runs after the update: the check is not run then
            Retarget::Civilian if !ordered => self.civilian_misc(at, &mut fx)?,
            Retarget::Civilian | Retarget::Never => {}
        }
        if ordered {
            self.idle_part_before_order(busy, at)?;
        } else {
            self.idle_part(busy, at, computer, ai, side, &mut fx)?;
        }
        self.look_around(computer, ai)?;
        self.boar_tail(at, computer, ai, side, &mut fx)?;
        if ordered {
            fx.push(Effect::AfterOrder(AfterOrder {
                retargeted,
                group: at.group,
            }));
        }
        Ok(fx)
    }

    /// The full update's tail after the order on a target it took ([`AfterOrder`]), with what the
    /// order gave the AI: processMisc's order part on a defend or follow order is not modelled;
    /// a retarget check whose timer ran out decides on the order's target (`target`: its owner
    /// and class) and the unit's weapon range ([`UnitAi::retarget_outcome`]); then the idle
    /// part's end ([`UnitAi::idle_part`] past its slot 72): an action or a target keeps the idle
    /// timer of a player above Gaia at 0.
    ///
    /// # Errors
    ///
    /// What the tail does not model.
    pub fn after_order(
        &mut self,
        after: AfterOrder,
        player: usize,
        class: i16,
        weapon_range: f32,
        target: Option<(usize, i16)>,
        computer: bool,
    ) -> Result<(), Unsupported> {
        let order = match (self.current_order, self.current_action) {
            (-1, -1) => -1,
            (-1, action) => action + 100,
            (order, _) => order,
        };
        if matches!(order, DEFEND_ORDER | FOLLOW_ORDER) {
            return Err(Unsupported(
                "processMisc's defend or follow order after an order on a target",
            ));
        }
        if after.retargeted {
            self.retarget_outcome(target, weapon_range, computer, None)?;
        }
        self.idle_part_end(class, player, after.group)
    }

    /// The boar's slot 4 (`0x140c2c010`) after the base, however that ended: its second idle
    /// check runs only for a timer the base's left past the timeout (a unit with an action), and
    /// a boar without a target calms down.
    fn boar_tail(
        &mut self,
        at: &Surroundings,
        computer: bool,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        if let Some(home) = self.home
            && !home.furious
            && self.adjusted_idle_timeout < self.idle_timer
        {
            self.idle_timer = 0;
            let _ = self.process_idle(at, computer, ai, side, fx)?;
        }
        self.calm_down();
        Ok(())
    }

    /// The target block at the full update's start (`0x140c66b10`): a unit inside another
    /// (`+0x2f0`) drops its target, as does one whose target no longer exists (an attack action's
    /// idle timer then takes its timeout; `FUN_140c5e1e0` finds no target left). A dead target (a
    /// state above 2, not a building) of a live unit ends an action that needs a live one
    /// ([`LIVE_TARGET_ACTIONS`]), an attack whose top action is no longer in its state 2 (slot
    /// 0x2b0: the swing on the kill) or any action with `+0x1a8` set: `+0x1a8` clears; an attack
    /// looks for its next target ([`UnitAi::next_after_kill`]); but for a hunt or `0x2be`, any
    /// other action loses its target and stops (slot 0xb0 with 1). The range test of a target on
    /// a patrol order or in a patrolling group (`FUN_140c41690`) answers 0 for a player that is
    /// not a computer; a computer's (the ship and priority rules), a computer's priority by the
    /// target's distance and action `0x2be`'s check (`FUN_140c40f10`) are not modelled.
    fn target_block(
        &mut self,
        at: &Surroundings,
        computer: bool,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        if self.current_target != -1 && at.garrisoned {
            self.current_target = -1;
        }
        if self.current_target == -1 {
            return Ok(());
        }
        let action = self.current_action;
        match at.target {
            None => {
                self.current_target = -1;
                if action == ATTACK_ACTION {
                    self.idle_timer = self.idle_timeout;
                }
            }
            Some(t) => {
                if at.state == 2 && 2 < t.state && t.kind != 80 {
                    let attack_over = action == ATTACK_ACTION
                        && at.action_state.ok_or(Unsupported(
                            "an attack whose target died, its top action not held",
                        ))? != 2;
                    if self.stop_after_kill || LIVE_TARGET_ACTIONS.contains(&action) || attack_over
                    {
                        self.stop_after_kill = false;
                        if action == ATTACK_ACTION {
                            self.next_after_kill(at, computer, ai, side, fx)?;
                        } else if action != HUNT_ACTION && action != BUILD_ORDER {
                            self.remove_target();
                            self.stop_current(true, at, fx)?;
                        }
                    }
                }
                // on a patrol order, or in a group that patrols (`FUN_141017c40`,
                // `FUN_141017c30`), FUN_140c41690 tests the target's range; for a player that
                // is not a computer (FUN_141659cf0) it answers 0 at once
                if computer
                    && (matches!(self.current_order, PATROL | PATROL_2)
                        || at.group.is_some_and(|g| g.patrol))
                {
                    return Err(Unsupported("the patrol range of a unit AI's target"));
                }
                // order 0x262, which is MOVE's action value
                if computer
                    && (0..100).contains(&self.order_priority)
                    && self.current_order == 0x262
                {
                    return Err(Unsupported(
                        "a computer's order priority by its target's distance",
                    ));
                }
            }
        }
        if self.current_action == BUILD_ORDER {
            return Err(Unsupported("action 0x2be's target check"));
        }
        Ok(())
    }

    /// The villager's case 0x202 on its hunt (`FUN_140c22bf0`, action 0x265): the class to hunt is
    /// the notify's (its second parameter), else the AI's target class, else its last; with none
    /// the target goes and the action stops (slot 0xb0 with 1; slot 0x48 hands the notify to the
    /// owner, not held). Of a class the civilian's slot 0x1b0 gives no kind, `ClosestObject`
    /// (every relation, below one worker for class 49) over the civilian's search
    /// ([`civilian_search_radius`]) names the next target, which it hunts (`HuntObject`, slot 0xc8
    /// with 1, [`UnitAi::hunt_object`]) and the processing breaks; finding none, a human player's
    /// villager does nothing more. The search by kind (slot 0x80, `ClosestResourceObject` with the
    /// notify's drop site), a farm found (class 49) and a computer player's villager are not
    /// modelled.
    fn hunter_search(
        &mut self,
        n: Notify,
        at: &Surroundings,
        computer: bool,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, Unsupported> {
        let mut class = n.p[1];
        if class == -1 {
            class = self.target_type;
        }
        if class == -1 {
            class = self.last_target_type;
        }
        if class == -1 {
            self.remove_target();
            self.stop_current(true, at, fx)?;
            return Ok(GO_ON);
        }
        if civilian_kind(class) != -1 {
            return Err(Unsupported(
                "a hunter's search by kind (ClosestResourceObject, slot 0x80)",
            ));
        }
        let found = side.closest(&Search {
            at: [at.position[0], at.position[1]],
            radius: civilian_search_radius(at.los, at.search_radius),
            classes: &CIVILIAN_CARED,
            size: at.size,
            pick: Pick::Closest {
                group: class,
                relation: -1,
                state: -1,
                master: -1,
                workers: if class == 49 { 1 } else { -1 },
                spared: -1,
            },
        })?;
        self.alert(found, at.dt_seconds);
        if found.id == -1 {
            if computer {
                return Err(Unsupported("a computer player's hunter that found nothing"));
            }
            return Ok(GO_ON);
        }
        if class == 49 {
            return Err(Unsupported("a hunter's search finding a farm"));
        }
        let point = found
            .position
            .ok_or(Unsupported("a found object without its place"))?;
        self.hunt_object(found.id, class, point, at, fx)?;
        Ok(BREAK)
    }

    /// The villager's case 0x202 on its gather (`FUN_140c22bf0`, action 0x261): the class to
    /// gather is the AI's target class, else its last, else the notify's (its second parameter);
    /// with none the target goes and the action stops (slot 0xb0 with 1; slot 0x48 hands the
    /// notify to the owner, not held). The civilian's slot 0x1b0 names its gather group: by it
    /// `ClosestResourceObject` (slot 0x80, [`UnitSide::closest_resource`]) from the notify's drop
    /// site (its third parameter), below 4 workers for stone and gold (groups 3 and 4); a class
    /// without a group takes `ClosestObject` (every relation, any workers) over the civilian's
    /// search ([`civilian_search_radius`]). What it finds it gathers (`GatherObject`, slot 0xd0
    /// with 1, [`UnitAi::gather_object`]) and the processing breaks; finding nothing, a human
    /// player's villager does nothing more, a computer player's stops as above. For a farm (the
    /// notify's class 49) `ClosestObject`'s ninth argument is 0: friends' farms pass.
    fn gatherer_search(
        &mut self,
        n: Notify,
        at: &Surroundings,
        computer: bool,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, Unsupported> {
        let mut class = self.target_type;
        if class == -1 {
            class = self.last_target_type;
        }
        if class == -1 {
            class = n.p[1];
        }
        if class == -1 {
            self.remove_target();
            self.stop_current(true, at, fx)?;
            return Ok(GO_ON);
        }
        let kind = civilian_kind(class);
        let found = if kind == -1 {
            // "Calling closest object for a farm that's alive": the friends' pass over
            let spared = if n.p[1] == i32::from(crate::gather::FARM_CLASS) {
                0
            } else {
                -1
            };
            let found = side.closest(&Search {
                at: [at.position[0], at.position[1]],
                radius: civilian_search_radius(at.los, at.search_radius),
                classes: &CIVILIAN_CARED,
                size: at.size,
                pick: Pick::Closest {
                    group: class,
                    relation: -1,
                    state: -1,
                    master: -1,
                    workers: -1,
                    spared,
                },
            })?;
            self.alert(found, at.dt_seconds);
            match found.position {
                Some(position) if found.id != -1 => Some(Gatherable {
                    id: found.id,
                    position,
                    class: class as i16,
                }),
                None if found.id != -1 => {
                    return Err(Unsupported("a found object without its place"));
                }
                _ => None,
            }
        } else {
            let cap = if matches!(kind, 3 | 4) { 4 } else { -1 };
            side.closest_resource(kind as u8, cap, n.p[2], self.current_target)?
        };
        let Some(found) = found else {
            if computer {
                self.remove_target();
                self.stop_current(true, at, fx)?;
            }
            return Ok(GO_ON);
        };
        self.gather_object(found, fx);
        Ok(BREAK)
    }

    /// `GatherObject` (slot 0xd0, `FUN_140c4dd50`) forced on what the search found: the unit's
    /// gather command runs on it ([`Effect::GatherObject`]); taken as done (the caller stops on a
    /// failure): the gather action value (`0x261`) becomes current, the last the one before, the
    /// target with its class and place (the last ones kept), and the AI counts as commanded.
    fn gather_object(&mut self, found: Gatherable, fx: &mut Vec<Effect>) {
        fx.push(Effect::GatherObject(found.id));
        self.set_action(GATHER_ACTION);
        self.set_target(found.id, i32::from(found.class), found.position);
        self.commanded = true;
    }

    /// The target block's attack whose target died (`0x140c66b10`): processIdle (slot 0x240 with
    /// 0, [`UnitAi::process_idle`]) may find the next thing to do (6); with nothing (5) the AI
    /// moves to its old target's point within the unit's weapon range (slot 0x130, forced,
    /// running: it always takes it). Then the order ends: it becomes the last order, the order and
    /// its priority −1. A patrol order's part (orders 711, 733) is not modelled.
    fn next_after_kill(
        &mut self,
        at: &Surroundings,
        computer: bool,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        let produced = self.process_idle(at, computer, ai, side, fx)?;
        if !produced {
            let m = self.move_to(AiMove {
                point: self.target_point,
                range: at.weapon_range,
                force: true,
                walk: false,
            });
            fx.extend(m.map(Effect::Move));
        }
        if matches!(self.current_order, PATROL | PATROL_2) {
            return Err(Unsupported("a patrol's attack whose target died"));
        }
        self.last_order = self.current_order;
        self.current_order = -1;
        self.order_priority = -1;
        Ok(())
    }

    /// The units the update models after its target block ([`UnitAi::target_block`]): a target
    /// only on a villager's build, gather, hunt or garrison (702/602, 709/609, 713/613, 717/617), its attack back
    /// as a hunt (700/613, [`UnitAi::hunt_object`]), a hunt without an order (−1/613: `HuntObject`
    /// after a stop that ended the order; the update has no case for the order), on the follow
    /// order (712 with its action 612, the move's 610 or none), a builder or hunter sent off a
    /// footprint (702/703, 713/703), a building on MOVE's order with a target (710 and any
    /// action value), on a boar's or lion's own attack (−1/600) or the attack order
    /// (700/600), or without an order and an action (a hunt ended by its
    /// queued orders); busy (`busy`, the actions left), only those or a unit on MOVE's order
    /// (710/610: without a target, or an animal's own wander; with one, the work command's move to
    /// an object), and for the types that never retarget also one whose move ended (710 with no
    /// action value); the attacks only for the types that do.
    fn check_busy_order(&self, retarget: Retarget, busy: bool) -> Result<(), Unsupported> {
        let order = (self.current_order, self.current_action);
        let working = matches!(
            order,
            (BUILD_ORDER, BUILD_ACTION)
                | (HUNT_ORDER | ATTACK_ORDER | -1, HUNT_ACTION)
                | (crate::ai_kinds::ORDER_GATHER, GATHER_ACTION)
                | (crate::garrison::ENTER_ORDER, crate::garrison::ENTER_ACTION)
                | (FOLLOW_ORDER, FOLLOW_ACTION | MOVE_ACTION | -1)
                // sent off a footprint (`sent_off`): the order and target kept
                | (BUILD_ORDER | HUNT_ORDER, MOVE_ASIDE_ACTION)
        );
        // a move, or a move off a footprint (slot 0x148's 0x2bf, which keeps the order)
        let moving = (matches!(order, (MOVE_ORDER | -1, MOVE_ACTION))
            || order.1 == MOVE_ASIDE_ACTION)
            && self.current_target == -1;
        // a move whose action ended (the action value reset) while the default action runs
        let moved = order == (MOVE_ORDER, -1) && self.current_target == -1;
        let scouting = order == (AUTO_SCOUT_ORDER, AUTO_SCOUT_ACTION) && self.current_target == -1;
        let following = order.0 == FOLLOW_ORDER;
        // a boar's own attack on its attacker or a lion's on its prey ([`UnitAi::attack_object`]),
        // or a work command's
        // attack (the attack order and action, notify kind 9); a building's make action that
        // started during it takes the action value (619, its constructor `FUN_140acb140`) while
        // the Attack goes on
        let attacking = (matches!(self.ai_type, BOAR_AI | 5) && order == (-1, ATTACK_ACTION))
            || order == (ATTACK_ORDER, ATTACK_ACTION)
            || order == (ATTACK_ORDER, crate::spawn::MAKE_ACTION);
        // a move to an object (the work command's without a task, slot 0x138); a building a
        // MOVE took with a target keeps the order while it trains or researches
        let moving_to =
            order == (MOVE_ORDER, MOVE_ACTION) || (self.ai_type == 17 && order.0 == MOVE_ORDER);
        // without an order and an action the target is a stale one: the order queue's next order
        // takes over
        if self.current_target != -1 && !working && !attacking && !moving_to && order != (-1, -1) {
            return Err(Unsupported("a unit AI's target the slice does not model"));
        }
        if !busy || self.is_idle() {
            return Ok(());
        }
        match retarget {
            Retarget::Never | Retarget::Civilian if working || moving || moving_to || moved => {
                Ok(())
            }
            // a move to an object: the retarget check passes over its target unless it is a
            // building, wall or trebuchet, whose search is not modelled
            Retarget::Base if moving || moving_to || scouting || following || attacking => Ok(()),
            _ => Err(Unsupported(
                "a busy unit AI on an order the slice does not model",
            )),
        }
    }

    /// The notify processing of the full update: while notifies are queued and the last handler
    /// said go on, each runs through slot 0x238 ([`UnitAi::handle`]); a pass that went through
    /// them all ends with `FUN_140c5dc10`; 3 and 0xd break off. Returns the last handler's
    /// result (2 without notifies). Notifies posted during the pass (`FUN_140c55270`'s buffer)
    /// do not occur in the slice.
    fn process_notifies(
        &mut self,
        dt: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, Unsupported> {
        let mut result = GO_ON;
        while !self.notifies.is_empty() && result == GO_ON {
            self.processing = true;
            let mut i = 0;
            while i < self.notifies.len() {
                let n = self.notifies[i];
                result = self.handle(n, dt, at, computer, ai_rng, side, fx)?;
                if result == DEAD {
                    // "I'm dead, purging notifyQueue and returning FALSE.": the full update ends
                    self.processing = false;
                    self.flush_buffered();
                    self.end_notifies(dt, at, fx)?;
                    return Ok(DEAD);
                }
                if result == BREAK || result == PRODUCED {
                    break;
                }
                i += 1;
            }
            if result == GO_ON {
                self.end_notifies(dt, at, fx)?;
            }
            self.processing = false;
            self.flush_buffered();
        }
        Ok(result)
    }

    /// `FUN_140c5dc10`: a completion of the current action (with no order) stops it
    /// (`FUN_140c5e1e0`, slot 0xb0 with 1); then the notifies go and `+0x15c` takes `time`.
    pub fn end_notifies(
        &mut self,
        time: u32,
        at: &Surroundings,
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        let completed = self.notifies.iter().any(|n| {
            n.kind == ACTION_COMPLETED && self.current_order == -1 && self.current_action == n.p[0]
        });
        if completed {
            self.remove_target();
            self.stop_current(true, at, fx)?;
        }
        self.last_update_time = time;
        self.notifies.clear();
        Ok(())
    }

    /// `FUN_140c5dc10` from outside the full update (a MOVE, an ownership change: `time` is the
    /// world time), where a completion it would stop on is not modelled.
    pub fn clear_notifies(&mut self, time: u32) -> Result<(), Unsupported> {
        let mut fx = Vec::new();
        self.end_notifies(time, &Surroundings::default(), &mut fx)?;
        if !fx.is_empty() {
            return Err(Unsupported(
                "a completion stopping the action outside the full update",
            ));
        }
        Ok(())
    }

    /// The unit's notify (its slot 0x328, `FUN_141086a10`) to its unit AI: a unit in state 2 (or
    /// state 0 with a damage notify, which slot 0x38 refuses there) passes it to the AI's slot
    /// 0x38 ([`UnitAi::accept`]). A notify the AI does not take (any but 699) goes to the
    /// owner's slot 0x2e8 (`FUN_14162dab0`, the human player's alerts), which the slice does not
    /// hold.
    pub fn post(&mut self, n: Notify, unit_state: u8) -> Result<(), Unsupported> {
        let _ = self.accept(n, unit_state);
        Ok(())
    }

    /// The unit AI's slot 0x38 (`FUN_140c556d0`, every type's): outside state 2 it takes nothing
    /// (0). A notify 699 is taken only from one of its attackers (a binary search of `+0xc0`),
    /// an action's invalidation only of the current action, and a movement completion
    /// (`0x1fa` with `0x262`) while the AI has an action other than a move (`0x262`, `0x268`,
    /// `0x2bf`, `0x2c0`) is ignored, answering 1 without joining the queue; any other joins the
    /// queue (`FUN_140c42470`, whose 0 on a failed allocation cannot happen) and answers 1.
    pub fn accept(&mut self, n: Notify, unit_state: u8) -> bool {
        if unit_state != 2 {
            return false;
        }
        let action = self.current_action;
        let taken = match n.kind {
            // "That unit is not attacking me, so I'm ignoring it."
            NO_LONGER_ON_ME => self.attackers.binary_search(&n.caller).is_ok(),
            ACTION_INVALIDATED => n.p[0] == action,
            ACTION_COMPLETED => {
                // ignored (and handled) for a non-movement action
                if n.p[0] == MOVE_ACTION && !matches!(action, 0x262 | 0x268 | 0x2bf | 0x2c0) {
                    return true;
                }
                true
            }
            _ => true,
        };
        if taken {
            self.append(n);
        }
        taken
    }

    /// `FUN_140c42470`: a notify joins the queue; a full queue doubles its capacity, but while
    /// it is processed the notify goes to the buffer instead.
    fn append(&mut self, n: Notify) {
        if self.notify_capacity <= self.notifies.len() {
            if self.processing {
                self.buffered.push(n);
                return;
            }
            self.notify_capacity *= 2;
        }
        self.notifies.push(n);
    }

    /// `FUN_140c55270`: the buffered notifies join the queue, in order.
    fn flush_buffered(&mut self) {
        for n in std::mem::take(&mut self.buffered) {
            self.append(n);
        }
    }

    /// The hunted animal's damage notify (`FUN_140c26420`, case 500, "I took some damage from
    /// Unit #%d"): at no hit points left (`p[1]` below 1) it gives up: the notifies end
    /// (`FUN_140c5dc10`) and it returns 4. Else the attacker joins its attackers
    /// (`FUN_140b43f30`), the unit is under attack (`FUN_14119d2d0` with 1) and it runs from its
    /// attackers ([`UnitAi::run_away`]), unconditionally unless its action is 0x25f or 0x268 or
    /// its class 59; it returns 3. An animal that is not Gaia's first alerts its owner (slot
    /// 0x2e8 with 0x201): a human player's alert list, not held; a computer player's AI, not
    /// modelled.
    #[allow(clippy::too_many_arguments)]
    fn hunted_damaged(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, Unsupported> {
        if n.p[1] < 1 {
            self.end_notifies(time, at, fx)?;
            return Ok(DEAD);
        }
        // an animal that is not Gaia's alerts its owner (slot 0x2e8 with 0x201,
        // `FUN_14162dab0`): a human player's alert list and the old pather's hulled-area reset,
        // not held; a computer player's AI hears it (slot 0x3c8)
        if at.player != 0 && computer {
            return Err(Unsupported(
                "a computer player's hunted animal under attack (its AI hears 0x201)",
            ));
        }
        let _ = self.add_attacker(n.caller);
        side.set_under_attack(true);
        let uncon = !(matches!(self.current_action, 0x25f | 0x268) || at.class == CLASS_KING);
        let _ = self.run_away(uncon, at, computer, ai_rng, side, fx)?;
        Ok(BREAK)
    }

    /// `runAwayFromAttackers` (`FUN_140c5e900`) with no unit to run from but its attackers: a
    /// unit in stand ground or no attack (stances 2 and 3) of a player other than Gaia stays, and
    /// so does one with an action value unless `uncon`. Else it runs away from the sum of the
    /// vectors to its attackers the world holds (a random one when they cancel out: two draws of
    /// the AI stream), by twice its speed, a defensive unit of a player at most 5 tiles from its
    /// state position, through `evasiveMoveTo` with both its flags ([`UnitAi::evasive_move`]).
    /// A computer's unit attacked by class 22 (its group's runs) and a computer's transport
    /// ship are not modelled; neither are unit groups (`+0x6d0`). True when the unit moves.
    fn run_away(
        &mut self,
        uncon: bool,
        at: &Surroundings,
        computer: bool,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, Unsupported> {
        if matches!(self.stance, 2 | 3) && at.player != 0 {
            return Ok(false);
        }
        if !uncon && self.current_action != -1 {
            return Ok(false);
        }
        let [x, y, z] = at.position;
        let (mut vx, mut vy) = (0.0f32, 0.0f32);
        let mut class_22 = false;
        for &id in &self.attackers {
            let Some(u) = at.unit(id) else {
                continue;
            };
            vx += u.position[0] - x;
            vy += u.position[1] - y;
            class_22 |= u.class == CLASS_22;
        }
        if computer && class_22 && 0.0 < at.speed {
            return Err(Unsupported(
                "a computer's unit running from class 22 (its group's runs)",
            ));
        }
        if computer && at.class == 20 {
            return Err(Unsupported("a computer's transport ship running away"));
        }
        let reach = at.speed * RUN_REACH;
        let mut d = crate::fmath::sqrt(vy * vy + vx * vx);
        if d < RUN_EPSILON {
            let r = ai.rand15() as i32 as f32;
            vx = (r + r) / RUN_DRAW_SCALE - RUN_ONE;
            let r = ai.rand15() as i32 as f32;
            vy = (r + r) / RUN_DRAW_SCALE - RUN_ONE;
            d = crate::fmath::sqrt(vy * vy + vx * vx);
        }
        let mut ex = x - (vx / d) * reach;
        let mut ey = y - (vy / d) * reach;
        if at.player != 0 && self.stance == 1 {
            let gx = self.state_position[0] - ex;
            let gy = self.state_position[1] - ey;
            if RUN_LEASH < crate::fmath::sqrt(gy * gy + gx * gx) {
                ex = self.state_position[0] - (vx / d) * RUN_LEASH;
                ey = self.state_position[1] - (vy / d) * RUN_LEASH;
            }
        }
        self.evasive_move([ex, ey, z], (true, true), at, ai, side, fx)
    }

    /// `evasiveMoveTo` (the unit AI's slot 0x150, `FUN_140c4d140`) to `point` with its flags
    /// `(uncon, tries)`: without `uncon` a unit with an action value stays. A coordinate that is
    /// not a number becomes the unit's own plus one (minus one above 5); the point is held on the
    /// map (below 0 to 0, at the far edge one tile in). With `tries`, a point the unit could not
    /// stand at (its slot 0x278) moves by up to 3 on each axis, five times at most, each move
    /// four draws of the AI stream (the x sign, its amount, the y sign, its amount), held on the
    /// map again. Then the unit's move there (slot 0x340, forced, running, range 0): the AI holds
    /// the move action and the point without a target. True when it moves.
    fn evasive_move(
        &mut self,
        point: [f32; 3],
        (uncon, tries): (bool, bool),
        at: &Surroundings,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, Unsupported> {
        if !uncon && self.current_action != -1 {
            return Ok(false);
        }
        let near = |p: f32, own: f32| {
            if !p.is_nan() {
                p
            } else if crate::fmath::at_most_or_unordered(own, EVASIVE_NEAR) {
                // COMISS own, 5.0; JBE
                own + RUN_ONE
            } else {
                own - RUN_ONE
            }
        };
        let held = |v: f32, size: i32| {
            if v < 0.0 {
                0.0
            } else if size as f32 <= v {
                size as f32 - EVADE_EDGE
            } else {
                v
            }
        };
        let mut x = held(near(point[0], at.position[0]), at.map[0]);
        let mut y = held(near(point[1], at.position[1]), at.map[1]);
        if tries {
            for _ in 0..5 {
                if side.can_stand([x, y])? {
                    break;
                }
                let step = |ai: &mut Well512, v: f32| {
                    let up = ai.rand15() & 1 != 0;
                    let by = (ai.rand15() & 3) as f32;
                    if up { v + by } else { v - by }
                };
                x = step(ai, x);
                y = step(ai, y);
                x = held(x, at.map[0]);
                y = held(y, at.map[1]);
            }
        }
        let m = self.move_to(AiMove {
            point: [x, y, point[2]],
            range: 0.0,
            force: true,
            walk: false,
        });
        fx.extend(m.map(Effect::Move));
        // the move (slot 0x340) gives the unit its MoveTo at once: the update's later checks see
        // it busy
        self.commanded = true;
        Ok(true)
    }

    /// The boar's damage notify (`FUN_140c25770`, case 500): at no hit points left
    /// (`p[1]` below 1) it gives up: the notifies end (`FUN_140c5dc10`) and it returns 4. Else
    /// the attacker joins its attackers (`FUN_140b43f30`), the unit is under attack
    /// (`FUN_14119d2d0` with 1) and it attacks its closest valid attacker
    /// ([`UnitAi::closest_valid_attacker`]) through `AttackObject` (slot 0xb8,
    /// [`UnitAi::attack_object`]): not forced while its action is the attack (600), which then
    /// fails, so a boar keeps attacking the unit it took first; else forced, the furious flag
    /// (`+0x1fc`) becomes whether that attacker is a villager and the idle timer (`+0x160`)
    /// restarts. With an attacker it returns 3, else 2.
    fn boar_damaged(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, Unsupported> {
        if n.p[1] < 1 {
            self.end_notifies(time, at, fx)?;
            return Ok(DEAD);
        }
        let _ = self.add_attacker(n.caller);
        side.set_under_attack(true);
        let target = self.closest_valid_attacker(at, ai_rng);
        let Some(attacker) = at.unit(target) else {
            return Ok(GO_ON);
        };
        let villager = attacker.class == CLASS_VILLAGER;
        let furious = if self.current_action == ATTACK_ACTION {
            let _ = self.attack_object(target, false, false, at, side, fx)?;
            true
        } else {
            let _ = self.attack_object(target, true, false, at, side, fx)?;
            self.idle_timer = 0;
            villager
        };
        let home = self
            .home
            .as_mut()
            .ok_or(Unsupported("a boar's unit AI without its own part"))?;
        home.furious = furious;
        Ok(BREAK)
    }

    /// The villager's damage notify (`FUN_140c22bf0`, case 500, "I took some damage from Unit
    /// #%d"): the unit's group (`+0x6d0`, `FUN_140c55c00`) would hear of it, and the slice holds
    /// none. At no hit points left (`p[1]` below 1) it gives up: the owner's alert (slot 0x2e8
    /// with `0x209`, the human player's), the notifies end (`FUN_140c5dc10`) and it returns 4.
    /// An attacker the world does not hold ends it, as does a player who is out (`+0x89` 2; not
    /// held, 0 in every player probed). A computer's unit AI (slot 0x50) is not modelled. Then the
    /// owner's alert (slot 0x2e8 with `0x201`); the attacker joins its attackers
    /// (`FUN_140b43f30`), and a new one that is not its target nor a class-10 attacker of
    /// combat level 1 (`FUN_140fde4f0`) makes it call for help (`AskForHelp`, `FUN_140c42ce0`),
    /// which is not modelled; the unit is under attack (`FUN_14119d2d0` with 1). It ignores the
    /// damage ("I have an attack/hunt/repair order") on a move or enter order while it animates
    /// (slot 0x1f8 above 0), the attack or the repair order; on the hunt, gather and boar luring
    /// orders with its top action in state 12 (slot 0x2b0), a target of class 10 the world holds
    /// and a Gaia attacker; or on those orders from a class-10 attacker of combat level 1 with its
    /// top action in state 4, 8, 12 or 15 (`0x9110`). A Gaia attacker it attacks back: with an
    /// order not on the attacker, its current order goes back into the queue at once
    /// ([`UnitAi::requeue_current`]) and the order becomes the attack (700, the last order kept)
    /// at priority 100; then the civilian's `AttackObject` (slot 0xb8, `FUN_140c17370`, forced):
    /// `HuntObject` ([`UnitAi::hunt_object`]) on classes 9, 10 and 15, else `AttackObject`; then
    /// `+0x1a8` is set and it breaks off. Its evade from another player's attacker is not
    /// modelled.
    #[allow(clippy::too_many_arguments)]
    fn villager_damaged(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, Unsupported> {
        if n.p[1] < 1 {
            self.end_notifies(time, at, fx)?;
            return Ok(DEAD);
        }
        let Some(attacker) = at.unit(n.caller).copied() else {
            return Ok(GO_ON);
        };
        if computer {
            return Err(Unsupported(
                "a computer villager's damage notify (its slot 0x50)",
            ));
        }
        let added = self.add_attacker(n.caller);
        let fierce = attacker.class == CLASS_10 && attacker.combat_level == 1;
        if added && self.current_target != n.caller && !fierce {
            return Err(Unsupported(
                "a villager's call for help (AskForHelp, FUN_140c42ce0)",
            ));
        }
        side.set_under_attack(true);
        let order = self.current_order;
        let state = at.action_state.ok_or(Unsupported(
            "a villager's damage notify with an action the slice does not hold",
        ))?;
        let gathers = matches!(
            order,
            crate::ai_kinds::ORDER_HUNT
                | crate::ai_kinds::ORDER_GATHER
                | crate::ai_kinds::ORDER_AUTO_BOAR_LURING
        );
        let moves = matches!(
            order,
            crate::ai_kinds::ORDER_MOVEMENT | crate::ai_kinds::ORDER_ENTER
        ) && 0.0
            < at.anim_speed.ok_or(Unsupported(
                "a moving villager's damage notify without its animation speed",
            ))?;
        let ignores = moves
            || order == crate::ai_kinds::ORDER_ATTACK
            || (gathers
                && state == 12
                && at.target.is_some_and(|t| t.class == CLASS_10)
                && attacker.owner == 0)
            || order == crate::ai_kinds::ORDER_REPAIR
            || (fierce && gathers && state < 16 && (0x9110u32 >> state) & 1 != 0);
        if ignores {
            return Ok(GO_ON);
        }
        if attacker.owner != 0 {
            return Err(Unsupported("a villager evading its attackers"));
        }
        // "I have a GAIA attacker": with an order not on the attacker, the current order goes back
        // into the queue at once (slot 0x30) and the order becomes the attack (700,
        // `FUN_140c64b60` with 0: the last order kept) at priority 100
        if self.current_order != -1 && n.caller != self.current_target {
            self.requeue_current(at, fx)?;
            self.set_order_with(crate::ai_kinds::ORDER_ATTACK, false, ai_rng);
            self.order_priority = 100;
        }
        // slot 0xb8 forced, the civilian's
        let _ = self.civilian_attack(n.caller, at, side, fx)?;
        self.stop_after_kill = true;
        Ok(BREAK)
    }

    /// The building's damage notify (`FUN_140c22610`, case 500): at no hit points left it gives
    /// up ("I'm dead, I give up."): the owner's alert (slot 0x2e8 with 0x209), the notifies end
    /// (`FUN_140c5dc10`) and it returns 4. Else the owner hears 0x201, as does each other owner
    /// of the units inside it (`+0x2f8`); for a human player those are the alert list and the
    /// old pather's hulled-area reset (`FUN_14162dab0`), not held, and a computer player's AI
    /// hears them (slot 0x3c8), which is not modelled. A new attacker (`FUN_140bf4cb0`) that is
    /// not its target makes a human player's building ask for help (`AskForHelp`,
    /// `FUN_140c42ce0`), not modelled. Then the attacker joins the set (`FUN_140b43f30`), the
    /// unit is under attack (`FUN_14119d2d0` with 1) and it returns 2.
    fn building_damaged(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, Unsupported> {
        let new = self.attackers.binary_search(&n.caller).is_err();
        let asks = n.p[1] >= 1 && new && self.current_target != n.caller;
        if computer || asks {
            return Err(Unsupported(
                "a building's damage notify (its computer owner's AI, AskForHelp)",
            ));
        }
        if n.p[1] < 1 {
            self.end_notifies(time, at, fx)?;
            return Ok(DEAD);
        }
        let _ = self.add_attacker(n.caller);
        side.set_under_attack(true);
        Ok(GO_ON)
    }

    /// `ClosestValidAttacker` (`FUN_140c158d0`): over the attackers in their order, those the
    /// world holds but of classes 3, 13, 52 and 54, the one with the least squared gap
    /// (`FUN_14118f8c0`); one within `DAT_142ef9004` (0.0625) of the best so far takes over on a
    /// coin from the Ai stream (`rand15`, odd). −1 for none.
    fn closest_valid_attacker(&self, at: &Surroundings, ai_rng: &mut Well512) -> i32 {
        let mut best = (ATTACKER_FAR, -1);
        for &id in &self.attackers {
            let Some(u) = at.unit(id) else {
                continue;
            };
            if matches!(u.class, 3 | 13 | 52 | 54) {
                continue;
            }
            let d = u.gap_squared;
            if best.0 - ATTACKER_TIE <= d
                && (best.0 + ATTACKER_TIE <= d || ai_rng.rand15() & 1 == 0)
            {
                continue;
            }
            best = (d, id);
        }
        best.1
    }

    /// `HuntObject` (slot 0xc8, `FUN_140c4e220`) forced on `target` (its master's `class` and its
    /// `point`): the current action stops (slot 0xb0 with 1) and the unit's hunt command runs on the
    /// target ([`Effect::HuntObject`], which carries the stop); taken as done (the caller stops on
    /// a failure): the hunt action value (`0x265`) becomes current, the stopped one the last, the
    /// target with its class and point (the last ones kept), and the AI counts as commanded.
    fn hunt_object(
        &mut self,
        target: i32,
        class: i32,
        point: [f32; 3],
        at: &Surroundings,
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        self.stop_current(true, at, fx)?;
        // the unit's stopAction goes with its hunt command, which runs it first
        if fx.pop() != Some(Effect::Stop) {
            return Err(Unsupported(
                "a HuntObject whose stop the slice did not make",
            ));
        }
        fx.push(Effect::HuntObject(target));
        self.set_action(HUNT_ACTION);
        self.set_target(target, class, point);
        self.commanded = true;
        Ok(())
    }

    /// Slot 0x30 with the current order, immediate and in front (`FUN_140c55c90` as the
    /// civilian's damage notify calls it: the issuer the unit, the order, target and its class,
    /// owner −1, the target point, the range `+0x5c`, the priority `+0x40`, the point shown,
    /// ready): a unit above state 2 or without an order takes none; else
    /// [`UnitAi::order_now`] with the unit's id, so the current order goes in front of the queue
    /// and the given one, the same record, in front of it.
    fn requeue_current(
        &mut self,
        at: &Surroundings,
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        if 2 < at.state || self.current_order == -1 {
            return Ok(());
        }
        let record = QueuedOrder {
            issuer: at.id,
            order: self.current_order,
            priority: self.order_priority,
            target: self.current_target,
            target_owner: -1,
            point: self.target_point,
            range: self.desired_distance,
            shown: [self.target_point[0], self.target_point[1]],
            ready: true,
            target_type: self.target_type,
        };
        let place = ([at.position[0], at.position[1]], at.class);
        self.order_now(record, Some(at.id), Some(place), Some(at), fx)
    }

    /// `FUN_140c55c90`'s immediate branch ("Adding as an immediate order."), for a unit in state
    /// 2 or below and an order other than −1. With `keep_from` (inFront 1, the unit's id) a
    /// current order goes in front of the queue first ("Keeping old order, putting it onto the
    /// queue.", [`UnitAi::insert_order`]): the issuer the unit, the current order, priority
    /// (`+0x40`), target and its class, owner −1, the target point, the range `+0x5c`, and the
    /// new order's shown point and ready flag. Then the order goes in front. But for an attack
    /// roundup (`0x2d9`, "so I'm not stopping the object") the target goes (`FUN_140c5e1e0`) and
    /// the action stops (slot 0xb0 with 1, which ends an order of its action and reads the
    /// unit's `place` for it). The notifies end (`FUN_140c5dc10` with 0; outside the full update
    /// without `at`, where a completion it would stop on is not modelled), the queue's first
    /// order runs at once (slot 0x230, [`UnitAi::process_order`]) and, unless it was not ready,
    /// leaves the queue (`FUN_140c5e280`).
    fn order_now(
        &mut self,
        order: QueuedOrder,
        keep_from: Option<i32>,
        place: Option<([f32; 2], i16)>,
        at: Option<&Surroundings>,
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        if let Some(unit) = keep_from
            && self.current_order != -1
        {
            let kept = QueuedOrder {
                issuer: unit,
                order: self.current_order,
                priority: self.order_priority,
                target: self.current_target,
                target_owner: -1,
                point: self.target_point,
                range: self.desired_distance,
                shown: order.shown,
                ready: order.ready,
                target_type: self.target_type,
            };
            self.insert_order(kept, true);
        }
        self.insert_order(order, true);
        if order.order != crate::ai_kinds::ORDER_ATTACK_ROUNDUP {
            self.remove_target();
            self.stop_with(true, place, fx)?;
        }
        match at {
            Some(at) => self.end_notifies(0, at, fx)?,
            None => self.clear_notifies(0)?,
        }
        let front = self.orders[0];
        if self.process_order(&front, at, fx)? != 0xe {
            let _ = self.orders.remove(0);
        }
        Ok(())
    }

    /// `FUN_140c42640`: an order joins the queue, in front of the first when `in_front` and the
    /// queue holds one, else after the last. A full queue doubles its capacity first, its new
    /// records filled with a stop order (`0x2c2`) to nothing (Vec's own growth here); a failed
    /// allocation, the routine's only other way out, cannot happen (`operator new` throws).
    pub(crate) fn insert_order(&mut self, order: QueuedOrder, in_front: bool) {
        if in_front && !self.orders.is_empty() {
            self.orders.insert(0, order);
        } else {
            self.orders.push(order);
        }
    }

    /// `AttackObject` (slot 0xb8, `FUN_140c43720`): `+0x1d5` clears; not `force`d, an AI with an
    /// action fails (false). The target must be in the world; its slot 0xa0 with `any` must let it
    /// attack the target: the boar's, the transport ship's and the livestock's
    /// (`FUN_140c48400`) anything, the building's (`FUN_140c18b60`) nothing, the lion's
    /// ([`lion_may_attack`]), the civilian's ([`civilian_may_attack`]), the soldier's and the
    /// tower's ([`UnitSide::may_attack`]; the hunted animal's, `FUN_140c18d00`, reads its action
    /// list, slot 0x538, and is not modelled); then the unit's attack command
    /// ([`UnitSide::attack`]), on
    /// success the action is
    /// the attack (600, the last action keeping the old) and the target its point and class
    /// (`FUN_140c64c10`). The slice holds no unit groups (`+0x6d0`), whose members would attack
    /// too. A stop the processing made before (slot 0x3c0, a pending [`Effect::Stop`]) emptied
    /// the action list before the command fills it: it runs first ([`UnitSide::stop`]).
    fn attack_object(
        &mut self,
        target: i32,
        force: bool,
        any: bool,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, Unsupported> {
        if !force && self.current_action != -1 {
            return Ok(false);
        }
        let (class, position) = if matches!(self.ai_type, 13 | 15) {
            match side.may_attack(target, &self.seeker())? {
                Some(t) => t,
                None => return Ok(false),
            }
        } else {
            let t = match at.unit(target) {
                Some(t) => *t,
                None => match side.seen_unit(target)? {
                    Some(t) => t,
                    None => return Ok(false),
                },
            };
            let may = match self.ai_type {
                // FUN_140c48400: the boar's, the transport ship's and the livestock's
                4 | 11 | 19 => true,
                5 => any || lion_may_attack(at.attack_priority, t.class),
                CIVILIAN => civilian_may_attack(at.attack_priority, t.class),
                // FUN_140c18b60
                17 => false,
                _ => {
                    return Err(Unsupported(
                        "AttackObject's slot 0xa0 of a unit AI type not modelled",
                    ));
                }
            };
            if !may {
                return Ok(false);
            }
            (t.class, t.position)
        };
        if fx.contains(&Effect::Stop) {
            if fx.iter().any(|e| *e != Effect::Stop) {
                return Err(Unsupported(
                    "an attack command after a stop with other effects pending",
                ));
            }
            fx.clear();
            side.stop()?;
        }
        let attacked = side.attack(target, force, self)?;
        // the unit's posts (slot 0x328 -> the AI's slot 0x38) while the AI processes its
        // notifies: into its queue, or the buffer when it is full
        for (kind, p) in attacked.posted {
            self.post(Notify::own(at.id, kind, p), at.state)?;
        }
        if !attacked.attacks {
            return Ok(false);
        }
        self.commanded = true;
        self.set_action(ATTACK_ACTION);
        self.target_at(target, Some(i32::from(class)), position);
        Ok(true)
    }

    /// The civilian's `AttackObject` (slot 0xb8, `FUN_140c17370`) with its force flag 1, as every
    /// caller in the slice passes it (the damage notify and the unit in the way): an object the
    /// world does not hold (`FUN_1415417a0`) gets nothing (false); an animal or a tree (classes 9,
    /// 10 and 15) is hunted (slot 0xc8, [`UnitAi::hunt_object`]); any other goes to the base
    /// `AttackObject` with `any` 0 and the group flag 1 ([`UnitAi::attack_object`]). True when
    /// it hunts or attacks.
    fn civilian_attack(
        &mut self,
        target: i32,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, Unsupported> {
        let t = match at.unit(target) {
            Some(t) => *t,
            None => match side.seen_unit(target)? {
                Some(t) => t,
                None => return Ok(false),
            },
        };
        if matches!(t.class, 9 | 10 | 15) {
            self.hunt_object(target, i32::from(t.class), t.position, at, fx)?;
            return Ok(true);
        }
        self.attack_object(target, true, false, at, side, fx)
    }

    /// The attackers' upkeep of the full update (`0x140c6741b`): with none, a unit under attack
    /// no longer is (`FUN_14119d2d0` with 0); else the first attacker that the world does not
    /// hold, that has no unit AI, is dead (state above 2) or whose unit AI's target is another
    /// leaves the set (one an update).
    fn keep_attackers(&mut self, at: &Surroundings, side: &mut dyn UnitSide) {
        if self.attackers.is_empty() {
            if at.under_attack {
                side.set_under_attack(false);
            }
            return;
        }
        let gone = self.attackers.iter().position(|&id| {
            at.unit(id)
                .is_none_or(|u| 2 < u.state || u.target != Some(at.id))
        });
        if let Some(i) = gone {
            let _ = self.attackers.remove(i);
        }
    }

    /// Slot 0x238 per factory type: each type's handler, most of which pass the notify on to the
    /// base handler ([`UnitAi::base_notify`]).
    #[allow(clippy::too_many_arguments)]
    fn handle(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, Unsupported> {
        match self.ai_type {
            // TribeHuntedAnimalUnitAIModule (FUN_140c26420): its damage notify evades
            2 => match n.kind {
                DAMAGED => self.hunted_damaged(n, time, at, computer, ai_rng, side, fx),
                0x20f => Ok(GO_ON),
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the boar (FUN_140c25770): its damage notify attacks; a completion while it moves
            // off a footprint (0x2bf) makes its idle timeout the adjusted one plus 1, drops the
            // target (FUN_140c5e1e0) and sends it home on its next idle check (slot 0x1f0,
            // FUN_140c16d50: +0x1fd), then goes to the base handler
            4 => match n.kind {
                DAMAGED => self.boar_damaged(n, time, at, ai_rng, side, fx),
                ACTION_COMPLETED => {
                    if self.current_action == MOVE_ASIDE_ACTION {
                        self.idle_timeout = self.adjusted_idle_timeout.wrapping_add(1);
                        self.remove_target();
                        if let Some(home) = self.home.as_mut() {
                            home.go_home = true;
                        }
                    }
                    self.base_notify(n, time, at, computer, ai_rng, side, fx)
                }
                0x20f => Ok(GO_ON),
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the lion (FUN_140c266f0); both its cases go on to the base handler
            5 => match n.kind {
                ACTION_COMPLETED => {
                    if n.p[0] == ATTACK_ACTION {
                        // FUN_140c41bc0 with 1000: the idle timeout, then slot 0x1a8
                        self.idle_timeout = LION_ATTACKED_TIMEOUT;
                        self.update_adjusted_idle_timeout(ai_rng);
                        self.remove_target();
                        self.stop_current(true, at, fx)?;
                    }
                    self.base_notify(n, time, at, computer, ai_rng, side, fx)
                }
                TARGET_MOVED => {
                    // "Target is still visible, tracking.": a target the world holds within the
                    // lion's line of sight (FUN_14118f8c0 against its sight squared: COMISS, JC
                    // past)
                    let gap = at.target.filter(|_| self.current_target != -1).map(|t| {
                        crate::movement::footprint_gap_squared(
                            at.position,
                            at.size,
                            t.position,
                            t.radius,
                        )
                    });
                    // a target within its sight whose master has no flag 0x10 (`+0x654`) it
                    // attacks again (slot 0xb8: the target, forced, not any, its group) and the
                    // processing breaks
                    let flags = at.target.map_or(0, |t| t.flags);
                    if let Some(gap) = gap
                        && !crate::fmath::below_or_unordered(at.los * at.los, gap)
                        && flags & 0x10 == 0
                    {
                        let _ =
                            self.attack_object(self.current_target, true, false, at, side, fx)?;
                        return Ok(BREAK);
                    }
                    // "Target is invalid or no longer visible, stopping pursuit.": the target
                    // goes, the action stops (slot 0xb0 with 1) and slot 0x48 hands the notify
                    // to the owner (not held)
                    self.remove_target();
                    self.stop_current(true, at, fx)?;
                    self.base_notify(n, time, at, computer, ai_rng, side, fx)
                }
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the villager (FUN_140c22bf0; the notifies it has no case for go to the base handler):
            // a completion or failure of its action stops it, and slot 0x48
            // passes the notify to its owner's slot 0x2e8 (not held); a failed build (0x202, its
            // Build waiting without work, action 0x25a) looks for another foundation to build
            // (FUN_140c1c7c0, when the player's +0x613 is 0: unsaved, 0 in this game): one the
            // unit reaches within 0.1 (slot 0x3d0) it builds (slot 0xf0 with 1) and the processing
            // breaks; finding none, or not reaching it, a human player's villager stops (slot
            // 0x3c0) and the processing breaks
            8 => match n.kind {
                0x202 if self.current_action == HUNT_ACTION => {
                    self.hunter_search(n, at, computer, side, fx)
                }
                0x202 if self.current_action == GATHER_ACTION => {
                    self.gatherer_search(n, at, computer, side, fx)
                }
                0x202 if self.current_action == BUILD_ACTION => {
                    let found = side.foundation_search()?;
                    if found != -1 && side.reaches(found, FOUNDATION_REACH)? {
                        fx.push(Effect::BuildObject(found));
                        return Ok(BREAK);
                    }
                    if computer {
                        return Err(Unsupported("a computer builder without a foundation"));
                    }
                    fx.push(Effect::Stop);
                    Ok(BREAK)
                }
                // a completion or a failure of its action (0x1fa, 0x1f9) stops it
                ACTION_COMPLETED | ACTION_FAILED => {
                    if self.current_action == n.p[0] {
                        self.remove_target();
                        self.stop_current(true, at, fx)?;
                    }
                    Ok(GO_ON)
                }
                ACTION_EXECUTED if computer => {
                    Err(Unsupported("a computer villager's executed action"))
                }
                ACTION_EXECUTED => self.base_notify(n, time, at, computer, ai_rng, side, fx),
                DAMAGED => self.villager_damaged(n, time, at, computer, ai_rng, side, fx),
                // its own cases the slice does not model; any other is the base handler's
                ACTION_INVALIDATED
                | TARGET_MOVED
                | 0x202
                | 0x20f
                | 0x217..=0x21a
                | 0x21c
                | 0x21d => Err(Unsupported("a villager's notify the slice does not model")),
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the soldier (FUN_140c28bb0): its own cases are the computer's, a backing-up and a
            // healer's search
            13 => match n.kind {
                0x1fe => Err(Unsupported("a soldier's target under its least range")),
                ACTION_EXECUTED | 0x20f if computer => {
                    Err(Unsupported("a computer soldier's notify"))
                }
                0x202 if self.current_action == 0x25b => {
                    Err(Unsupported("a healer's search notify"))
                }
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the tower (FUN_140c299f0)
            15 => match n.kind {
                // "I'm dead, I give up.": the owner's alert (slot 0x2e8 with 0x209: a human
                // player's alert list and the old pather's hulled-area reset, not held; a
                // computer player's AI hears it), the notifies end and it gives up. Alive, the
                // other owners of the units inside it hear 0x201 and the base handler's damage
                // case follows, which is not modelled
                DAMAGED if n.p[1] < 1 && !computer => {
                    self.end_notifies(time, at, fx)?;
                    Ok(DEAD)
                }
                DAMAGED => Err(Unsupported(
                    "a tower's damage notify (its computer owner's AI, the base handler's case)",
                )),
                // "I am a defensive building and my current target has been destroyed, purging
                // attack order.": a target gone or dead (above state 2) is forgotten
                // (`FUN_140c5e1e0`) and the current action stops (slot 0xb0 with 1)
                ACTION_COMPLETED if n.p[0] == 600 && self.current_order == ATTACK_ORDER => {
                    let gone = self.current_target == -1 || at.target.is_none_or(|t| 2 < t.state);
                    if gone {
                        self.remove_target();
                        self.stop_current(true, at, fx)?;
                    }
                    Ok(GO_ON)
                }
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the building (FUN_140c22610): most notifies end there
            17 => match n.kind {
                DAMAGED => self.building_damaged(n, time, at, computer, side, fx),
                // "Unit #%d is attacking me": it joins the attackers and the unit is under attack
                600 => {
                    let _ = self.add_attacker(n.caller);
                    side.set_under_attack(true);
                    Ok(GO_ON)
                }
                // "is no longer doing an action on me.": one in the set (FUN_140bf4cb0) leaves it
                NO_LONGER_ON_ME => {
                    if let Ok(i) = self.attackers.binary_search(&n.caller) {
                        let _ = self.attackers.remove(i);
                    }
                    Ok(GO_ON)
                }
                0x202 | 0x216 | 0x21b => self.base_notify(n, time, at, computer, ai_rng, side, fx),
                _ => Ok(GO_ON),
            },
            // livestock (FUN_140c26ac0): a unit sent off a footprint (0x216, with the new
            // sending-off `DAT_1439e60e2` 1) is the base handler's; any notify but 0x202 drops
            // the action value (FUN_140c64b50) and breaks off
            19 => match n.kind {
                SENT_OFF => self.base_notify(n, time, at, computer, ai_rng, side, fx),
                0x202 => Err(Unsupported(
                    "a livestock's notify 0x202 (its settings' test)",
                )),
                _ => {
                    self.set_action(-1);
                    Ok(BREAK)
                }
            },
            _ => Err(Unsupported(
                "the notify handler of a unit-AI type not modelled",
            )),
        }
    }

    /// The base handler (`FUN_140c589c0`) for the kinds the slice posts: a completed action
    /// ([`UnitAi::action_completed`]), an ownership change (stopAction, `FUN_140c5e1e0`, slot 0xb0
    /// with 1, `FUN_140c5dc10` with the handler's time, the order queue emptied), the kinds it
    /// ignores; anything else is not modelled.
    #[allow(clippy::too_many_arguments)]
    fn base_notify(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, Unsupported> {
        match n.kind {
            ACTION_COMPLETED => self.action_completed(n, at, fx),
            ACTION_FAILED => self.action_failed(n, at, computer, ai_rng, side, fx),
            ACTION_INVALIDATED => self.action_invalidated(n, at, computer, ai_rng, side, fx),
            crate::explore::WANTS_POINT if self.explorer() => {
                self.seek_unexplored(at, computer, ai_rng, side, fx)
            }
            OWNER_CHANGED => {
                fx.push(Effect::Stop);
                self.remove_target();
                self.stop_current(true, at, fx)?;
                self.end_notifies(time, at, fx)?;
                self.orders.clear();
                Ok(GO_ON)
            }
            // a MoveTo's moved target: a patrol path's processIdle (slot 0x240 with 1)
            TARGET_MOVED if self.patrol_path => {
                Err(Unsupported("a moved target's processIdle on a patrol path"))
            }
            // a moved target, then the kinds "I'm ignoring that." and the action values
            TARGET_MOVED
            | 0x1f6
            | 0x1f7
            | 0x1f8
            | 0x1ff
            | ACTION_EXECUTED
            | 0x203
            | 0x259..=0x262
            | 0x265..=0x269
            | 0x2be => Ok(GO_ON),
            // readyToAttack's target out of reach (0x1fd): a unit on the attack order that cannot
            // move (FUN_140fdd080: a master of type 30 and up with speed) gives the order up
            0x1fd => {
                if self.current_order == ATTACK_ORDER && !(29 < at.kind && 0.0 < at.speed) {
                    self.current_order = -1;
                }
                Ok(GO_ON)
            }
            IN_THE_WAY => self.in_the_way(n, at, side, fx),
            SENT_OFF => self.sent_off(n, side, fx),
            // a unit of a group sent off a footprint: the action value 0x2c0
            SENT_OFF_GROUP => {
                self.set_action(GROUP_ASIDE_ACTION);
                Ok(GO_ON)
            }
            // "Unit #%d is no longer doing an action on me.": an attacker leaves the set
            NO_LONGER_ON_ME => {
                if let Ok(i) = self.attackers.binary_search(&n.caller) {
                    let _ = self.attackers.remove(i);
                }
                Ok(GO_ON)
            }
            _ => Err(Unsupported("a notify kind the base handler does not model")),
        }
    }

    /// The base handler's case 0x215 ([`IN_THE_WAY`]): a unit not of a boar's master (class 10
    /// of combat level 1, `FUN_140fde4f0`) attacks the unit in its way (`AttackObject`, slot
    /// 0xb8, forced, its slot 0xa0 unforced: the civilian's [`UnitAi::civilian_attack`], any
    /// other type's [`UnitAi::attack_object`]) when the world's table
    /// holds it, retargeting is on (`+0x1d6` 0), its stance (`+0x1a9`) is 0 and it has no order;
    /// the other unit's group (`+0x6d0`), which would let it attack with one, the slice does not
    /// hold. Processing goes on.
    fn in_the_way(
        &mut self,
        n: Notify,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, Unsupported> {
        let boar = at.class == 10 && at.combat_level == 1;
        if !boar
            && at.unit(n.p[0]).is_some()
            && self.retarget_disabled == 0
            && self.current_order == -1
            && self.stance == 0
        {
            // slot 0xb8 forced: the civilian's own, every other type's the base's
            let _ = if self.ai_type == CIVILIAN {
                self.civilian_attack(n.p[0], at, side, fx)?
            } else {
                self.attack_object(n.p[0], true, false, at, side, fx)?
            };
        }
        Ok(GO_ON)
    }

    /// The base handler's case 0x216 ([`SENT_OFF`]): the closest spot beside the foundation's
    /// footprint (`FUN_1411566b0` with the notify's radii, 0) takes the unit's move off it (slot
    /// 0x148 forced, range 0, walking unless it is prey: the unit's slot 0x340, then the action
    /// value 0x2bf, the old one the last). A human player's villager whose current action has a
    /// target then queues the foundation's order in front (`FUN_140c42640` with 1: order 0x2de,
    /// priority 100, ready) and the work on that target behind (0x2db); the orders' points are
    /// the objects' slot 0x520 vectors' first two parts (x and the height) and 0, as the game
    /// passes them. Processing goes on.
    fn sent_off(
        &mut self,
        n: Notify,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, Unsupported> {
        let radius = [n.p[0] as f32 / RADIUS_SCALE, n.p[1] as f32 / RADIUS_SCALE];
        let off = side.off_footprint(n.p[2], radius)?;
        if let Some([x, y]) = off.spot {
            let _ = self.move_aside(
                AiMove {
                    point: [x, y, off.z],
                    range: 0.0,
                    force: true,
                    walk: off.class != PREY_CLASS,
                },
                fx,
            );
        }
        if !off.computer
            && off.class == VILLAGER_CLASS
            && let (Some(site), Some(target)) = (off.site, off.action_target)
        {
            let order = |o: &Named, order: i32| QueuedOrder {
                issuer: off.player,
                order,
                priority: 100,
                target: o.id,
                target_owner: o.owner,
                point: [o.vector[0], o.vector[1], 0.0],
                range: 0.0,
                shown: o.vector,
                ready: true,
                target_type: o.master,
            };
            self.insert_order(order(&site, FOUNDATION_ORDER), true);
            self.insert_order(order(&target, ORDER_ON_TARGET), false);
        }
        Ok(GO_ON)
    }

    /// Slot 0x148 (`FUN_140c4d0d0`), the move off a footprint: with its force flag (`m.force`)
    /// or without an action value, the unit's slot 0x340 takes the move with the same
    /// arguments ([`Effect::MoveAside`], which the caller runs after the update); on its 1 the
    /// action value becomes [`MOVE_ASIDE_ACTION`], the old one the last, and the target stays.
    /// The caller's slot 0x340 is `FUN_1410866e0`, which always gives 1, but for classes 51 and
    /// 54 (`FUN_141216c50`, their packing), which it does not run. True when it moves.
    fn move_aside(&mut self, m: AiMove, fx: &mut Vec<Effect>) -> bool {
        if !m.force && self.current_action != -1 {
            return false;
        }
        fx.push(Effect::MoveAside(m));
        self.set_action(MOVE_ASIDE_ACTION);
        true
    }

    /// The action values of an explorer (the base handler's case 0x202): 605, 623 and 630, and
    /// the auto scout's `0x2bd`.
    const fn explorer(&self) -> bool {
        let a = self.current_action;
        let k = a.wrapping_sub(EXPLORE_ACTION) as u32;
        (k < 0x1a && (0x204_0001u32 >> (k & 0x1f)) & 1 != 0) || a == AUTO_SCOUT_ACTION
    }

    /// The base handler's case 0x202 for an explorer ("I'm an explorer, looking for the closest
    /// undiscovered tile."): on the auto scout's action, with the player's scout map never made
    /// (`player+0x1370`), the search around the town centre (`FUN_140c65490`), else the unit AI's
    /// slot 0x90 (`FUN_140c4bd40`), both from its unit ([`UnitSide::seek_unexplored`]); then slot
    /// 0x100 with 1 to the point ([`UnitAi::explore_at`]), the explore's action value becomes the
    /// last and the action value comes back; processing breaks. "Couldn't find any tiles.": on
    /// the auto scout order its last search ([`UnitSide::seek_beyond_sight`]) sends the auto scout
    /// to its point the same way (slot 0x100 with 1; the last action stays) and processing breaks;
    /// with no point left the player's scout map is made from the tiles it sees and the notify
    /// posted to the owner (nothing for a human player), [`UnitSide::make_scout_map`]; with the
    /// map made the search about the town centre is skipped and the last search reads the map.
    /// On the scout order (`0x2da`) the player's slot 0x2e8
    /// with `0x211` is not modelled; on any other the unit AI forgets its target
    /// (`FUN_140c5e1e0`) and stops (slot 0xb0 with 1; slot 0x48 hands the notify to the owner,
    /// not held; the interface's flag a last auto scout order sets is not the sim's) and
    /// processing goes on. The other explorers' search is not modelled.
    fn seek_unexplored(
        &mut self,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, Unsupported> {
        let action = self.current_action;
        if action != AUTO_SCOUT_ACTION {
            return Err(Unsupported(
                "an explorer's search for the closest undiscovered tile (slot 0x90)",
            ));
        }
        if let Some([x, y]) = side.seek_unexplored(self, ai_rng)? {
            let _ = self.explore_at(x, y, true, fx);
            self.last_action = self.current_action;
            self.current_action = action;
            return Ok(BREAK);
        }
        match self.current_order {
            crate::ai_kinds::ORDER_SCOUT => Err(Unsupported(
                "a scout that finds no undiscovered tile (the player's slot 0x2e8 with 0x211)",
            )),
            AUTO_SCOUT_ORDER => {
                let Some([x, y]) = side.seek_beyond_sight(ai_rng)? else {
                    // the player's scout map, from the tiles it sees; on the auto scout's action
                    // the AI's slot 0x40 hands the notify to the owner (slot 0x2e8 with 0x202,
                    // FUN_14162dab0: no case for it but the old short pather's hulled area, a
                    // computer player's AI would hear it); processing breaks
                    side.make_scout_map()?;
                    if computer && self.current_action == AUTO_SCOUT_ACTION {
                        return Err(Unsupported(
                            "a computer player's AI told its scout found nothing (0x202)",
                        ));
                    }
                    return Ok(BREAK);
                };
                if self.current_action == AUTO_SCOUT_ACTION {
                    let _ = self.explore_at(x, y, true, fx);
                }
                self.current_action = action;
                Ok(BREAK)
            }
            _ => {
                self.remove_target();
                self.stop_current(true, at, fx)?;
                Ok(GO_ON)
            }
        }
    }

    /// The base handler's case `0x1f9` ("Action %d has failed.", `FUN_140c589c0`): with an order
    /// the state position takes the unit's place first; on the scout order (`0x2da`) the
    /// player's slot 0x2e8 with `0x211` is not modelled. A failed attack (600) on the attack order
    /// (700) or with the attack action current runs processIdle (slot 0x240 with 0,
    /// [`UnitAi::process_idle`]): an action it produced breaks off (0xd), none makes a human
    /// player's unit forget its target and stop (slot 0xb0 with 1). Without an
    /// order, the failure of the current action forgets the target (`FUN_140c5e1e0`) and stops
    /// it (slot 0xb0 with 1; slot 0x48 hands the notify to the owner, not held); with one, so
    /// does the failure of the order's own action (action + 100 = order); on order `0x2d4` a
    /// failed `0x262` forgets the target and ends the order too (slot 0xb0 with 0), on `0x2bd`
    /// the action value goes (the last action takes it); any other failed attack forgets the
    /// target and stops; the rest go on.
    fn action_failed(
        &mut self,
        n: Notify,
        at: &Surroundings,
        computer: bool,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, Unsupported> {
        let (order, action, p0) = (self.current_order, self.current_action, n.p[0]);
        if order != -1 {
            self.state_position = [at.position[0], at.position[1]];
            if order == crate::ai_kinds::ORDER_SCOUT {
                return Err(Unsupported(
                    "a failed action on the scout order (the player's slot 0x2e8 with 0x211)",
                ));
            }
        }
        if p0 == 600 && (order == 700 || action == 600) {
            let produced = self.process_idle(at, computer, ai, side, fx)?;
            if produced {
                return Ok(PRODUCED);
            }
            if computer {
                return Err(Unsupported(
                    "a computer player's failed attack without a new action",
                ));
            }
            self.remove_target();
            self.stop_current(true, at, fx)?;
            return Ok(GO_ON);
        }
        let own = if order == -1 {
            action == p0
        } else {
            action + 100 == order
        };
        if own {
            self.remove_target();
            self.stop_current(true, at, fx)?;
            return Ok(GO_ON);
        }
        match (order, p0) {
            (0x2d4, 0x262) => {
                self.remove_target();
                self.stop_current(false, at, fx)?;
                return Ok(GO_ON);
            }
            (0x2bd, 0x262) => {
                self.set_action(-1);
                return Ok(GO_ON);
            }
            _ => {}
        }
        if p0 == 600 {
            self.remove_target();
            self.stop_current(true, at, fx)?;
        }
        Ok(GO_ON)
    }

    /// The base handler's case `0x1fb` ("Action %d has been invalidated.", `FUN_140c589c0`): an
    /// attack (600) invalidated on the attack order (700) or with `0x202` retargets: with
    /// retargeting off (`+0x1d6`) a target still in the world breaks off (not modelled), else
    /// processIdle (slot 0x240 with 0, [`UnitAi::process_idle`]; the building's handler never
    /// passes the notify on). Unless it is 5 the notify goes on; with 5 a human player's unit
    /// forgets its target (`FUN_140c5e1e0`) and stops (slot 0xb0 with 1); a computer player's
    /// tests (slot 0x130) are not modelled. Without an order, the invalidation of the current
    /// action forgets the target and stops it (slot 0xb0 with 1; then slot 0x48 hands the notify
    /// to the owner, not held); with one, so does the invalidation of the order's own action
    /// (action + 100 = order); order `0x2d4` and `0x2bd` on action `0x262` are not modelled; any
    /// other invalidated attack forgets the target and stops; the rest go on.
    #[allow(clippy::too_many_arguments)]
    fn action_invalidated(
        &mut self,
        n: Notify,
        at: &Surroundings,
        computer: bool,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, Unsupported> {
        let (order, action, p0) = (self.current_order, self.current_action, n.p[0]);
        if (order == 700 || n.p[2] == 0x202) && p0 == 600 {
            if self.retarget_disabled != 0 {
                return Err(Unsupported(
                    "an invalidated attack with retargeting off (+0x1d6)",
                ));
            }
            let produced = self.process_idle(at, computer, ai, side, fx)?;
            if produced {
                return Ok(GO_ON);
            }
            if computer {
                return Err(Unsupported(
                    "a computer player's invalidated attack (slot 0x130)",
                ));
            }
            self.remove_target();
            self.stop_current(true, at, fx)?;
            return Ok(GO_ON);
        }
        let own = if order == -1 {
            action == p0
        } else {
            if matches!(order, 0x2d4 | 0x2bd) && p0 == 0x262 {
                return Err(Unsupported(
                    "an invalidated action 0x262 on order 0x2d4 or 0x2bd",
                ));
            }
            action + 100 == order
        };
        if own || p0 == 600 {
            self.remove_target();
            self.stop_current(true, at, fx)?;
        }
        Ok(GO_ON)
    }

    /// The base handler's case `0x1fa` (`FUN_140c589c0`): without an order, a completion of the
    /// current action (or of an attack) stops it. With one, the state position takes the unit's
    /// place; the completion of the order's own action (action + 100 = order) stops it, and so
    /// do an attack's; order 724 and 701 end their move, 712 a heal's. The attack order 700's
    /// own case (processIdle with 0) is not modelled.
    fn action_completed(
        &mut self,
        n: Notify,
        at: &Surroundings,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, Unsupported> {
        let p1 = n.p[0];
        let order = self.current_order;
        let action = self.current_action;
        if order == -1 {
            if action == p1 || p1 == 600 {
                self.remove_target();
                self.stop_current(true, at, fx)?;
            }
            return Ok(GO_ON);
        }
        self.state_position = [at.position[0], at.position[1]];
        if order == 700 && p1 == 600 {
            return Err(Unsupported("a completed attack on the attack order"));
        }
        if action == p1 && action + 100 == order {
            self.remove_target();
            self.stop_current(true, at, fx)?;
            return Ok(GO_ON);
        }
        match (order, p1) {
            (0x2d4, MOVE_ACTION) => {
                self.remove_target();
                self.stop_current(false, at, fx)?;
                return Ok(GO_ON);
            }
            (0x2bd, MOVE_ACTION) => {
                self.set_action(-1);
                return Ok(GO_ON);
            }
            (0x2c8, MOVE_ACTION) if action == 0x2bf => {
                self.set_action(-1);
                return Ok(GO_ON);
            }
            _ => {}
        }
        if p1 == 600 {
            self.remove_target();
            self.stop_current(true, at, fx)?;
        }
        Ok(GO_ON)
    }

    /// `FUN_140b43f30` on the attackers (`+0xc0`, count `+0xc8`): a binary search of the sorted
    /// set; an id not in it goes into its place (the array grows by `capacity * 103 / 64 + 1`,
    /// Vec's own growth here) and the call answers true, one already in it answers false.
    pub(crate) fn add_attacker(&mut self, id: i32) -> bool {
        match self.attackers.binary_search(&id) {
            Ok(_) => false,
            Err(i) => {
                self.attackers.insert(i, id);
                true
            }
        }
    }

    /// `FUN_140c5e1e0`: the target becomes the last one, and none (its point −1).
    pub(crate) const fn remove_target(&mut self) {
        if self.current_target != -1 {
            self.last_target = self.current_target;
            self.last_target_type = self.target_type;
            self.current_target = -1;
            self.target_type = -1;
            self.target_point = [-1.0; 3];
        }
    }

    /// Slot 0xb0 (`FUN_140c64ed0`), stopping the current action: it becomes the last action; with
    /// `keep_order` false or an action of the order (action + 100 = order) the order ends too
    /// (its priority with it), the state position taking the unit's place but for class 18. Then
    /// the unit's stopAction (slot 0x3c0), no action, and `+0x1a8` cleared. A patrol order (711,
    /// 733) would end too, with its timers' reset, which is not modelled.
    fn stop_current(
        &mut self,
        keep_order: bool,
        at: &Surroundings,
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        self.stop_with(
            keep_order,
            Some(([at.position[0], at.position[1]], at.class)),
            fx,
        )
    }

    /// [`UnitAi::stop_current`] with the unit's position and master's class (`place`), which only
    /// an order the stop ends reads (None: the caller knows it does not end one).
    pub(crate) fn stop_with(
        &mut self,
        keep_order: bool,
        place: Option<([f32; 2], i16)>,
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        let action = self.current_action;
        let order = self.current_order;
        if matches!(order, PATROL | PATROL_2) {
            return Err(Unsupported("a stop on a patrol order"));
        }
        self.last_action = action;
        if !keep_order || action + 100 == order {
            self.last_order = order;
            self.current_order = -1;
            self.order_priority = -1;
            let (position, class) = place.ok_or(Unsupported(
                "a stop ending an order without the unit's place",
            ))?;
            if class != CLASS_18 {
                self.state_position = position;
            }
        }
        fx.push(Effect::Stop);
        self.current_action = -1;
        self.stop_after_kill = false;
        Ok(())
    }

    /// `FUN_140c4dcb0` ("Forgetting current target"): the last target (`+0x6c`, `+0x70`) and the
    /// current one (`+0x48`, `+0x4c`) go, the target point with them. Its only other line is the
    /// debug log behind `DAT_143f0900d`, which changes no state.
    pub(crate) const fn forget_targets(&mut self) {
        self.last_target = -1;
        self.last_target_type = -1;
        self.current_target = -1;
        self.target_type = -1;
        self.target_point = [-1.0; 3];
    }

    /// `FUN_140c5e7d0`, the walk back to the state position: a unit more than a tile from it
    /// (the squared footprint gap, `FUN_140a847c0`, against [`WALK_BACK`]; `COMISS` then `JNC`,
    /// so a gap that is not a number walks) moves there at its own height within [`WALK_BACK`],
    /// forced and running (the unit's slot 0x340, which the caller runs: it takes the move but
    /// for classes 51 and 54, whose slot `FUN_141216c50` the caller does not model). On the
    /// move the action value becomes the move's (`0x262`, the old one the last); the target
    /// stays. The move gives the unit its `MoveTo` at once: the update's later checks see it
    /// busy. True when it walks.
    fn walk_back(&mut self, at: &Surroundings, fx: &mut Vec<Effect>) -> bool {
        let gap = point_gap_squared(self.state_position, at.position, at.size);
        if !crate::fmath::below_or_unordered(WALK_BACK, gap) {
            return false;
        }
        let [x, y] = self.state_position;
        fx.push(Effect::Move(AiMove {
            point: [x, y, at.position[2]],
            range: WALK_BACK,
            force: true,
            walk: false,
        }));
        self.set_action(MOVE_ACTION);
        self.commanded = true;
        true
    }

    /// The reset at the full update once the unit's actions ended (the unit's slot 0x430 false):
    /// an action value of [`ENDED_ACTIONS`] goes; `0x2bf` of stance 2 also sets the state
    /// position.
    fn reset_ended_action(&mut self, at: &Surroundings) {
        if ENDED_ACTIONS.contains(&self.current_action) {
            if self.current_action == 0x2bf && self.stance == 2 {
                self.state_position = [at.position[0], at.position[1]];
            }
            self.current_action = -1;
        }
    }

    /// The order check of a unit that is not busy: an order of these values the unit can give up
    /// (`FUN_140c4eea0` with 0) ends, its target removed (`FUN_140c5e1e0`); its priority stays.
    const fn check_order(&mut self, at: &Surroundings) -> Result<(), Unsupported> {
        let ends = matches!(
            self.current_order,
            0x269 | 700 | 0x2c5 | 0x2c6 | 0x2c9 | 0x2cf | 0x2d0 | 0x2dd | 0x322
        );
        if ends && self.can_order(at, false) {
            if self.current_order == PATROL_2 {
                return Err(Unsupported("the end of a patrol order 733"));
            }
            self.remove_target();
            self.current_order = -1;
        }
        Ok(())
    }

    /// `FUN_140c4eea0`: a unit takes orders unless its master has a task of action type 0x99
    /// (`FUN_1415c60b0`) and its target exists in the world (`FUN_1415417a0` on `+0x48`) or it
    /// has an action value (`+0x44`). With `task_only` (its second argument, which the full
    /// update and `FUN_140c68980` pass as 0 and only the slot 4 of `FUN_140c2bd20` as 1) a unit
    /// that passes the test takes orders only with that task.
    const fn can_order(&self, at: &Surroundings, task_only: bool) -> bool {
        if at.task_0x99 && (at.target.is_some() || self.current_action != -1) {
            return false;
        }
        !task_only || at.task_0x99
    }

    /// The unit AI between its owner's turns (`FUN_140c68980`, from `RGE_Combat_Object::update`
    /// for a unit of a type below 70 or not inside another): with no notifies queued, a unit
    /// without an action value or with an action it may interrupt (slot 0x2d0) runs its order
    /// queue. Its effects, as the full update's.
    pub fn off_turn(&mut self, at: &Surroundings) -> Result<Vec<Effect>, Unsupported> {
        let mut fx = Vec::new();
        if !self.notifies.is_empty() || self.orders.is_empty() {
            return Ok(fx);
        }
        if (self.current_action != -1 || at.busy) && !at.interruptible {
            return Ok(fx);
        }
        if self.can_order(at, false) {
            self.run_orders(at, &mut fx)?;
        }
        Ok(fx)
    }

    /// Slot 0x28 (`FUN_140c55c90`, every type's; slot 0x30 jumps to it), an order for the
    /// queue: a unit in a state above 2 takes none (false), an order of kind −1 neither (true);
    /// an order that is not immediate goes into the queue (`FUN_140c42640`: before the first
    /// queued order when `in_front`, else after the last). An immediate one, which keeps or drops
    /// the current order and runs the queue at once, is not modelled.
    pub fn queue_order(
        &mut self,
        unit_state: u8,
        order: QueuedOrder,
        immediate: bool,
        in_front: bool,
    ) -> Result<bool, Unsupported> {
        if 2 < unit_state {
            return Ok(false);
        }
        if order.order == -1 {
            return Ok(true);
        }
        if immediate {
            // the immediate branch is [`UnitAi::order_now`], which reads the unit's id and place
            // and gives effects; [`UnitAi::take_order_now`] is its entry
            return Err(Unsupported("an immediate order for the unit AI's queue"));
        }
        self.insert_order(order, in_front);
        Ok(true)
    }

    /// `FUN_140c4dfa0`: an order in the queue is not ready (its `+0x2c` is 0).
    #[must_use]
    pub fn has_unready_order(&self) -> bool {
        self.orders.iter().any(|o| !o.ready)
    }

    /// The order queue (`FUN_140c5d790`): while the AI has an action, an order below priority
    /// 100 waits; each order runs through slot 0x230 ([`UnitAi::process_order`]): one that is not
    /// ready (0xe) stays and the next is tried, any other result removes it, and 1 (taken) ends
    /// the queue's turn.
    pub fn run_orders(
        &mut self,
        at: &Surroundings,
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        let mut k = 0;
        while k < self.orders.len() {
            if self.current_action != -1 && self.orders[k].priority < 100 {
                return Ok(());
            }
            let order = self.orders[k];
            let r = self.process_order(&order, Some(at), fx)?;
            if r == 0xe {
                k += 1;
                continue;
            }
            // the game removes the order only while k is inside the queue: slot 0x230 may have
            // shrunk it (FUN_140c5de60)
            if k < self.orders.len() {
                let _ = self.orders.remove(k);
            }
            if r == 1 {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Slot 0x230 (`FUN_140c5c170`, every type's): an order not ready returns 0xe. A move
    /// (`0x2c6`) is the AI's move to its point within its range, forced and walking (slot 0x130),
    /// and the unit's slot 0x2d8 shows its points (`+0x7c0`..`+0x7cc`, not held); once taken
    /// (1) the order and its priority become the AI's (leaving a patrol order caps the secondary
    /// timer). An order on a target (`0x2db`) goes to the unit ([`Effect::OrderOnTarget`]) and is
    /// taken; a hunt order (`0x2c9`) runs `HuntObject` ([`UnitAi::hunt_object`]) and is taken. Other
    /// orders are not modelled.
    fn process_order(
        &mut self,
        o: &QueuedOrder,
        at: Option<&Surroundings>,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, Unsupported> {
        if !o.ready {
            return Ok(0xe);
        }
        if o.order == AUTO_SCOUT_ORDER {
            // case 0x321: slot 0x100 with 1 to the order's point, then the auto scout's action
            // value, the explore's the last; the tail takes the order
            let taken = self.explore_at(o.point[0] as i32, o.point[1] as i32, true, fx);
            self.last_action = std::mem::replace(&mut self.current_action, AUTO_SCOUT_ACTION);
            if !taken {
                return Ok(0);
            }
            return self.take_order(o);
        }
        if o.order == HUNT_ORDER {
            // case 0x2c9: HuntObject (slot 0xc8) forced on the target; the tail takes the order
            let at = at.ok_or(Unsupported("a hunt order run outside the unit AI's update"))?;
            let (class, point) = if o.target == self.current_target {
                (self.target_type, self.target_point)
            } else if let Some(t) = at.target.filter(|_| o.target == self.last_target) {
                // the target the AI held when its update began (an immediate requeue forgot it)
                (i32::from(t.class), t.position)
            } else if let Some(u) = at.unit(o.target) {
                (i32::from(u.class), u.position)
            } else {
                return Err(Unsupported(
                    "a hunt order on an object the unit AI does not name",
                ));
            };
            self.hunt_object(o.target, class, point, at, fx)?;
            return self.take_order(o);
        }
        if o.order == STOP_ORDER {
            // case 0x2c2: the target goes and the action stops (slot 0xb0 with 1); the order and
            // its priority end
            self.remove_target();
            self.stop_with(true, None, fx)?;
            if matches!(self.current_order, PATROL | PATROL_2) {
                return Err(Unsupported("a stop order after a patrol order"));
            }
            self.current_order = -1;
            self.order_priority = -1;
            return Ok(1);
        }
        if o.order == BUILD_ORDER {
            // case 0x2be: slot 0xf0 (`FUN_140c47d80`) with 1 builds on the target, and the tail
            // makes the order the AI's with its priority (the caller runs both,
            // [`crate::turn::Turns::build_order`])
            fx.push(Effect::OrderOnTarget(*o));
            return Ok(1);
        }
        if o.order == ORDER_ON_TARGET {
            // case 0x2db: the unit's slot 0x128 on the target, then the tail keeps the order the
            // work gave the AI and takes the order's priority (the caller runs both); a patrol
            // order before cannot be left there (the work's order is the record's)
            fx.push(Effect::OrderOnTarget(*o));
            return Ok(1);
        }
        if o.order != MOVE_ORDER {
            return Err(Unsupported(
                "a queued order other than a move or on a target",
            ));
        }
        let Some(m) = self.move_to(AiMove {
            point: o.point,
            range: o.range,
            force: true,
            walk: true,
        }) else {
            return Ok(0);
        };
        if matches!(self.current_order, PATROL | PATROL_2) {
            return Err(Unsupported("a queued order after a patrol order"));
        }
        fx.push(Effect::Move(m));
        self.take_order(o)
    }

    /// The order's tail (`LAB_140c5d611`): the order and its priority become the AI's; leaving a
    /// patrol order (which caps the secondary timer) is not modelled.
    const fn take_order(&mut self, o: &QueuedOrder) -> Result<u32, Unsupported> {
        if matches!(self.current_order, PATROL | PATROL_2) {
            return Err(Unsupported("a queued order after a patrol order"));
        }
        self.current_order = o.order;
        self.order_priority = o.priority;
        Ok(1)
    }

    /// Slot 0x100 (`FUN_140c4d910`), with `again` (its argument 4) or without an action value:
    /// the unit's explore to the point (slot 0x398, [`Effect::Explore`], which takes it); the AI
    /// then holds the explore's action value and the point without a target (`FUN_140c64c10`
    /// with −1, which leaves the target class), at height [`EXPLORE_Z`].
    fn explore_at(&mut self, x: i32, y: i32, again: bool, fx: &mut Vec<Effect>) -> bool {
        if !again && self.current_action != -1 {
            return false;
        }
        let point = [x as f32, y as f32];
        fx.push(Effect::Explore(point));
        self.last_action = std::mem::replace(&mut self.current_action, EXPLORE_ACTION);
        self.target_at(-1, None, [point[0], point[1], EXPLORE_Z]);
        true
    }

    /// Slot 0x28 (`FUN_140c55c90`) with Immediate set and inFront clear: for a unit in state 2 or
    /// below and an order (not −1), [`UnitAi::order_now`] without keeping the current order.
    /// `place` is the unit's position (x, y) and master's class, for the stop.
    pub fn take_order_now(
        &mut self,
        unit_state: u8,
        order: QueuedOrder,
        place: ([f32; 2], i16),
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        if 2 < unit_state || order.order == -1 {
            return Ok(());
        }
        self.order_now(order, None, Some(place), None, fx)
    }

    /// The idle part of the full update: a unit without an action value (and without a target
    /// or not busy) runs slot 72 once its idle timer passes its timeout, unless it is a busy unit
    /// of class 20; otherwise an action or a target keeps the idle timer of a player above Gaia
    /// at 0. Patrol orders, class 3's and 52's actions `0x26b`/`0x26c` and action `0x2c0` take
    /// branches the slice does not model.
    fn idle_part(
        &mut self,
        busy: bool,
        at: &Surroundings,
        computer: bool,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        let class_20 = busy && at.class == CLASS_20;
        if !class_20
            && self.adjusted_idle_timeout < self.idle_timer
            && self.current_action == -1
            && (self.current_target == -1 || !busy)
            && group_lets_idle(at.group)
        {
            // slot 0x240 with 1; the idle part reads no result
            self.idle_timer = 0;
            let _ = self.process_idle(at, computer, ai, side, fx)?;
            return Ok(());
        }
        if !self.patrols(at.group)
            && matches!(at.class, CLASS_3 | CLASS_52)
            && matches!(self.current_action, 0x26b | 0x26c)
        {
            return self.making_idle_part(at, computer, ai, side, fx);
        }
        self.idle_part_end(at.class, at.player, at.group)
    }

    /// The idle part's patrol branch (`0x140c67f0b`): a patrol order (`0x2c7`, `0x2dd`), or a
    /// group that patrols ([`GroupState::patrol`]) off order `0x326`.
    fn patrols(&self, group: Option<GroupState>) -> bool {
        matches!(self.current_order, PATROL | PATROL_2)
            || group.is_some_and(|g| g.patrol && self.current_order != 0x326)
    }

    /// The idle part past slot 72's test of a unit of class 3 or 52 on the make action (`0x26b`,
    /// `0x26c`): a building (`FUN_1411942c0`; `FUN_1411206a0` rules out classes 51 and 54) whose
    /// master has no missile (`+0x83c` below 0, `FUN_1410bfca0`) and whose firepower (`+0xa00`)
    /// is at most 1 (`DAT_14330eb8c`) forgets its target (`FUN_140c5e1e0`). With no target and its
    /// idle timer past its timeout, the timer restarts and processIdle runs (slot 0x240 with 1,
    /// [`UnitAi::process_idle`]); the idle timer is not reset otherwise.
    fn making_idle_part(
        &mut self,
        at: &Surroundings,
        computer: bool,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        if self.current_target != -1
            && at.kind == BUILDING_KIND
            && !at.ranged
            && at.firepower <= SINGLE_VOLLEY
        {
            self.remove_target();
        }
        if self.idle_timer <= self.adjusted_idle_timeout || self.current_target != -1 {
            return Ok(());
        }
        self.idle_timer = 0;
        self.process_idle(at, computer, ai, side, fx).map(|_| ())
    }

    /// [`UnitAi::idle_part`] when the update took an order on a target, which in the game ran
    /// before it: its end waits for the order ([`UnitAi::after_order`]); a timer past its timeout
    /// would run slot 72 if the order gave the AI no action, which is not modelled.
    fn idle_part_before_order(&self, busy: bool, at: &Surroundings) -> Result<(), Unsupported> {
        let class_20 = busy && at.class == CLASS_20;
        if !class_20
            && self.adjusted_idle_timeout < self.idle_timer
            && (self.current_target == -1 || !busy)
            && group_lets_idle(at.group)
        {
            return Err(Unsupported(
                "an idle part past its timeout after an order on a target",
            ));
        }
        Ok(())
    }

    /// The idle part past slot 72: the patrol branch ([`UnitAi::patrols`]) and class 3's and
    /// 52's actions `0x26b`/`0x26c` (but in [`UnitAi::idle_part`], [`UnitAi::making_idle_part`])
    /// are not modelled. Action `0x2c0` (a member moved aside) ends, the last action taking it,
    /// unless the unit's group moves (`FUN_141046600`); an action or a target keeps the idle timer
    /// of a player above Gaia at 0.
    fn idle_part_end(
        &mut self,
        class: i16,
        player: usize,
        group: Option<GroupState>,
    ) -> Result<(), Unsupported> {
        if self.patrols(group) {
            return Err(Unsupported("a patrol order's idle part"));
        }
        if matches!(class, CLASS_3 | CLASS_52) && matches!(self.current_action, 0x26b | 0x26c) {
            return Err(Unsupported("a unit of class 3 or 52 on action 0x26b/0x26c"));
        }
        if self.current_action == GROUP_ASIDE_ACTION {
            if group.is_none_or(|g| !g.moving) {
                self.last_action = GROUP_ASIDE_ACTION;
                self.current_action = -1;
            }
            return Ok(());
        }
        if (self.current_action != -1 || self.current_target != -1) && 0 < player {
            self.idle_timer = 0;
        }
        Ok(())
    }

    /// Slot 73 (`0x140c567c0`, processMisc). Its order part first, by the order (an action's
    /// value plus 100 without one): the follow order's ([`UnitAi::follow`]); the defend order's
    /// is not modelled. Then the leash checks (not on the orders 0x2c7, 0x2dd and 0x323): stance
    /// 1's (Defend) stops a unit more than 5 tiles (`DEFEND_LEASH`, squared) from its state
    /// position, forgets its target and walks back ([`UnitAi::walk_back`]), then goes on; with
    /// patrol information (`+0x1c0`, `FUN_140f95e10`) it is not modelled. Stance 2's (stand
    /// ground) stops a unit with an action (`+0x44`, not `0x2bf` or `0x2c0`) that strayed from its
    /// state position by more than its range plus `DAT_142eb880c` (0.25), forgets its target and
    /// walks back (10); a unit without an order but with an action of its own can stray. It is
    /// not modelled: that 0.25 has no role in the unit AI's image table. Then the retarget check:
    /// with retargeting on (`+0x1d6` 0), a timer used up is redrawn (`750 + r % 500`); a human
    /// player's unit of a range above 3 on the attack order 700 stops there ("I have a range >
    /// 3.0 with an attack order, so I don't retarget."), as does a unit with a target of Gaia's
    /// ("I have a GAIA target, so I can't retarget.") or without a target (or one gone), and
    /// one with a player's target off the attack order but for a building or wall target
    /// (classes 3, 27,
    /// 39, 49, 52) or a trebuchet's (51, 54), whose search for a better target (slot 0x538 with
    /// 9, then slot 0x520) is not modelled; nor is the attack order's search.
    ///
    /// With an order on a target taken (`ordered`), whose action, target and order the game's
    /// check reads, the order part and the leash read the AI before it and the outcome after the
    /// redraw is left for [`UnitAi::after_order`]. True when the timer ran out.
    fn retarget_check(
        &mut self,
        world_time: u32,
        computer: bool,
        at: &Surroundings,
        ai: &mut Well512,
        ordered: bool,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, Unsupported> {
        let order = match (self.current_order, self.current_action) {
            (-1, -1) => -1,
            (-1, action) => action + 100,
            (order, _) => order,
        };
        if order == DEFEND_ORDER {
            return Err(Unsupported("a defend order's processMisc"));
        }
        if order == FOLLOW_ORDER && self.follow(at, fx)? {
            return Ok(false);
        }
        // the defend stance's leash (unless on orders 0x2c7, 0x2dd and 0x323; a unit group's
        // flags would skip it too, and the slice holds no unit groups): a unit more than 5
        // tiles (DAT_142ed6f20, squared) from its state position, measured from its footprint
        // (FUN_140a847c0), stops (slot 0xb0 with 1), forgets its target and walks back
        if self.stance == 1 && !matches!(self.current_order, 0x2c7 | 0x2dd | 0x323) {
            if self.patrol_path {
                return Err(Unsupported(
                    "a defending unit's leash with patrol information",
                ));
            }
            // "We're too far out, returning.": then on to the retarget check
            let gap = point_gap_squared(self.state_position, at.position, at.size);
            if DEFEND_LEASH < gap {
                self.stop_current(true, at, fx)?;
                self.remove_target();
                let _ = self.walk_back(at, fx);
            }
        }
        if self.retarget_disabled != 0 {
            return Ok(false);
        }
        if world_time.wrapping_sub(self.last_retarget_time) < self.retarget_timer {
            return Ok(false);
        }
        self.last_retarget_time = world_time;
        self.retarget_timer = 750 + ai.rand15() % 500;
        if !ordered {
            let target = at.target.map(|t| (t.owner, t.class));
            self.retarget_outcome(target, at.weapon_range, computer, at.action_state)?;
        }
        Ok(true)
    }

    /// The retarget check after its timer's redraw: a human player's unit of a range above 3 on
    /// the attack order stops there, as does a unit with a Gaia target or without one (or one
    /// gone; `target` its owner and class as the world holds it), and one with a player's target
    /// off the attack order but for a building, wall or trebuchet target. Then a unit in stand
    /// ground or no attack (stance above 1) with an order stops, and one whose top action is in
    /// an attack's work state (6 or 7, slot 0x2b0: `action_state`, None when not held) without a
    /// building target. The search for a better target that follows (slot 0x538 with 9, then
    /// `bestUnitToAttack` from the target's slot 0x520 place) is not modelled.
    fn retarget_outcome(
        &self,
        target: Option<(usize, i16)>,
        weapon_range: f32,
        computer: bool,
        action_state: Option<u8>,
    ) -> Result<(), Unsupported> {
        let attack_order = self.current_order == ATTACK_ORDER;
        if attack_order && RETARGET_RANGE < weapon_range && !computer {
            return Ok(());
        }
        let mut search = false;
        if self.current_target != -1 {
            match target {
                Some((0, _)) => return Ok(()),
                Some((_, 3 | 27 | 39 | 49 | 52 | 51 | 54)) => {
                    if attack_order && !computer {
                        return Ok(());
                    }
                    search = true;
                }
                // a target gone from the world counts as none (FUN_1415417a0 finds nothing)
                Some(_) | None => {}
            }
        }
        if !attack_order && !search {
            return Ok(());
        }
        // slot 0x538 with 9 (an Attack in the action list) comes first; either way a unit in
        // stand ground or no attack with an order goes no further
        if 1 < self.stance && self.current_order != -1 {
            return Ok(());
        }
        // "I have an game attack action without an order ... inAttackWorkState"
        match action_state {
            Some(6 | 7) if !search => return Ok(()),
            Some(_) => {}
            None => {
                return Err(Unsupported(
                    "a retarget check of an action the slice does not hold (slot 0x2b0)",
                ));
            }
        }
        Err(Unsupported(
            "a unit AI's search for a better target (slot 0x538 with 9, then slot 0x520)",
        ))
    }

    /// The civilian's slot 73 (`FUN_140c21da0`, processMisc): on the attack order (700) without
    /// a target (`+0x48` −1) the action stops and the order ends (slot 0xb0 with 0). A computer
    /// player's villager whose player's `+0x622` is 1 also stops on the attack order or action
    /// when its target, a player's object, is beyond its range plus 1 (`FUN_141193e30`), which
    /// is not modelled: `+0x622` is not held. It returns 9 (no retarget).
    fn civilian_misc(
        &mut self,
        at: &Surroundings,
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        if self.current_order == ATTACK_ORDER && self.current_target == -1 {
            self.stop_current(false, at, fx)?;
        }
        Ok(())
    }

    /// processMisc's follow order (`0x140c567c0`, "I have a follow order for Unit #%d"): a
    /// target gone or dying (above state 2) or on a tile the unit's player does not see now ends
    /// it ("Target is gone or dying, stopping.", "He's NOT on a visible tile, stopping."): the
    /// defend target `+0x60` (not held) and the target go (`FUN_140c5e1e0`), the current action
    /// stops (slot 0xb0 with 1) and the order is −1. On action `0x2bf` or `0x2c0` the target
    /// point becomes (−1, −1). Otherwise a target more than a tile off the AI's target point on
    /// either axis gives the point its place (x, y, z); when the footprints' squared gap
    /// (`FUN_14118f8c0`) then exceeds the desired distance squared, the AI moves to it
    /// ([`UnitAi::move_to_object`] within the desired distance). True when the check ends there.
    fn follow(&mut self, at: &Surroundings, fx: &mut Vec<Effect>) -> Result<bool, Unsupported> {
        let target = match at.target {
            Some(t) if t.state <= 2 && t.visible => t,
            _ => {
                self.remove_target();
                self.stop_current(true, at, fx)?;
                self.current_order = -1;
                return Ok(true);
            }
        };
        if matches!(self.current_action, 0x2bf | 0x2c0) {
            self.target_point[0] = -1.0;
            self.target_point[1] = -1.0;
            return Ok(false);
        }
        let dx = target.position[0] - self.target_point[0];
        let dy = target.position[1] - self.target_point[1];
        let moved = dx < FOLLOW_SLACK_LOW
            || FOLLOW_SLACK_HIGH < dx
            || dy < FOLLOW_SLACK_LOW
            || FOLLOW_SLACK_HIGH < dy;
        if !moved {
            return Ok(false);
        }
        self.target_point = target.position;
        let gap = crate::movement::footprint_gap_squared(
            at.position,
            at.size,
            target.position,
            target.radius,
        );
        let range = self.desired_distance;
        if range * range < gap {
            self.move_to_object(self.current_target, target, range, fx);
            return Ok(true);
        }
        Ok(false)
    }

    /// The AI's move to an object (slot 0x138, `FUN_140c55490`, its fourth argument 1 as its only
    /// caller, the follow order, passes it): the unit's slot 0x348 ([`Effect::MoveToObject`])
    /// takes a target the world holds, and the AI the move action and the target with its class
    /// and point (`FUN_140c64c10`; the target and class it had become the last). The routine's
    /// branch for a target the world does not hold after slot 0x348 cannot run: slot 0x348
    /// (`FUN_141086440`) answers 0 for such a target, and the routine then changes nothing.
    fn move_to_object(&mut self, id: i32, target: Seen, range: f32, fx: &mut Vec<Effect>) {
        fx.push(Effect::MoveToObject { target: id, range });
        self.set_action(MOVE_ACTION);
        self.target_at(id, Some(i32::from(target.class)), target.position);
    }

    /// The boar's end of its full update: without a target it is no longer furious.
    fn calm_down(&mut self) {
        let calm = self.current_target == -1;
        if let Some(home) = self.home.as_mut().filter(|_| calm) {
            home.furious = false;
        }
    }

    /// The look-around check at the end of the full update (`LAB_140c6809c`): a timer past its
    /// timeout restarts with a timeout drawn around 5 s.
    const fn look_around(&mut self, computer: bool, ai: &mut Well512) -> Result<(), Unsupported> {
        if self.look_around_timer > self.look_around_timeout {
            // the human branch; `FUN_14152b470` sends players allied with a computer elsewhere
            if computer {
                return Err(Unsupported("look-around timer of a computer player"));
            }
            self.look_around_timer = 0;
            self.look_around_timeout = 4500 + ai.rand15() % 1000;
        }
        Ok(())
    }
}

/// The build order (`0x2be`) and the build action value (`0x25a`) of a villager's unit AI.
pub const BUILD_ORDER: i32 = 702;
/// The build action value (`0x25a`).
pub const BUILD_ACTION: i32 = 602;
/// The hunt order (`0x2c9`) and action value (`0x265`) of a villager's unit AI.
pub const HUNT_ORDER: i32 = 713;
/// The hunt action value (`0x265`).
pub const HUNT_ACTION: i32 = 613;
/// The gather action value (`0x261`), which a Gather's notifies carry.
pub const GATHER_ACTION: i32 = 0x261;

#[cfg(test)]
mod tests {
    use super::*;

    impl UnitAi {
        /// The full update of an idle unit (no order, action or target), with its move.
        fn update_idle(
            &mut self,
            dt: u32,
            world_time: u32,
            computer: bool,
            at: &Surroundings,
            ai: &mut Well512,
            side: &mut dyn UnitSide,
        ) -> Result<Option<AiMove>, Unsupported> {
            if !self.is_idle() {
                return Err(Unsupported("unit is not idle"));
            }
            self.update(dt, world_time, computer, at, ai, side)
                .map(|fx| first_move(&fx))
        }
    }

    fn idle(ai_type: u32) -> UnitAi {
        UnitAi {
            ai_type,
            current_order: -1,
            current_action: -1,
            current_target: -1,
            adjusted_idle_timeout: 3000,
            look_around_timeout: 1000,
            retarget_timer: 900,
            ..UnitAi::default()
        }
    }

    fn rng() -> Well512 {
        let mut r = Well512::default();
        r.set_seed(0);
        r
    }

    /// A unit side that records the attack commands, the under-attack flag and the stops (as
    /// many attack commands as came before each).
    #[derive(Default)]
    struct Recorder {
        attacks: Vec<(i32, bool)>,
        under_attack: Option<bool>,
        stops: Vec<usize>,
    }

    /// The builder's flood: of two foundations two tiles off, the one left of the unit comes before
    /// the one above it (the neighbours go left, right, below, above); one at the edge of the reach
    /// (`r − 1` tiles) is tested but its tile is not spread from, one beyond is never reached.
    #[test]
    fn the_builder_s_flood_tests_left_right_below_above() {
        let at = (50, 50);
        let found = flood_search(at, 8, |x, y| match (x, y) {
            (48, 50) => Some(1),
            (50, 52) => Some(2),
            _ => None,
        });
        assert_eq!(found, Some(1));
        let mut tested = Vec::new();
        assert_eq!(
            flood_search(at, 3, |x, y| {
                tested.push((x, y));
                None
            }),
            None
        );
        // r = 3: tiles less than 2 from the start spread: the 3x3 square, then its neighbours
        assert_eq!(tested.len(), 1 + 4 + 4 + 12);
        assert!(tested.contains(&(52, 50)) && !tested.contains(&(53, 50)));
        assert_eq!(
            &tested[..5],
            &[(50, 50), (49, 50), (51, 50), (50, 49), (50, 51)]
        );
    }

    impl UnitSide for Recorder {
        fn closest(&mut self, _: &Search<'_>) -> Result<Found, Unsupported> {
            Err(Unsupported("no search here"))
        }

        fn attack(
            &mut self,
            target: i32,
            force: bool,
            _: &UnitAi,
        ) -> Result<Attacked, Unsupported> {
            self.attacks.push((target, force));
            Ok(Attacked {
                attacks: true,
                posted: Vec::new(),
            })
        }

        fn set_under_attack(&mut self, value: bool) {
            self.under_attack = Some(value);
        }

        fn stop(&mut self) -> Result<(), Unsupported> {
            self.stops.push(self.attacks.len());
            Ok(())
        }

        fn may_attack(
            &mut self,
            _: i32,
            _: &Seeker,
        ) -> Result<Option<(i16, [f32; 3])>, Unsupported> {
            Err(Unsupported("no world here"))
        }

        fn seek_unexplored(
            &mut self,
            _: &UnitAi,
            _: &mut Well512,
        ) -> Result<Option<[i32; 2]>, Unsupported> {
            Err(Unsupported("no search here"))
        }

        fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, Unsupported> {
            Err(Unsupported("no search here"))
        }

        fn make_scout_map(&mut self) -> Result<(), Unsupported> {
            Err(Unsupported("no world here"))
        }
    }

    /// A unit sent off a footprint: what [`UnitSide::off_footprint`] answers.
    struct OffSide(OffFootprint, Vec<(i32, [f32; 2])>);

    impl UnitSide for OffSide {
        fn closest(&mut self, _: &Search<'_>) -> Result<Found, Unsupported> {
            Err(Unsupported("no search here"))
        }
        fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, Unsupported> {
            Err(Unsupported("no attack here"))
        }
        fn set_under_attack(&mut self, _: bool) {}
        fn stop(&mut self) -> Result<(), Unsupported> {
            Ok(())
        }
        fn may_attack(
            &mut self,
            _: i32,
            _: &Seeker,
        ) -> Result<Option<(i16, [f32; 3])>, Unsupported> {
            Err(Unsupported("no world here"))
        }
        fn seek_unexplored(
            &mut self,
            _: &UnitAi,
            _: &mut Well512,
        ) -> Result<Option<[i32; 2]>, Unsupported> {
            Err(Unsupported("no search here"))
        }
        fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, Unsupported> {
            Err(Unsupported("no search here"))
        }
        fn make_scout_map(&mut self) -> Result<(), Unsupported> {
            Err(Unsupported("no world here"))
        }
        fn off_footprint(
            &mut self,
            site: i32,
            radius: [f32; 2],
        ) -> Result<OffFootprint, Unsupported> {
            self.1.push((site, radius));
            Ok(self.0)
        }
    }

    /// Notify 0x216 (`FUN_14123bb20`'s, the base handler's case): the radii come back from the
    /// hundredths, the unit walks to the spot (action 0x2bf, the old one the last); a human
    /// villager whose action has a target queues the foundation's order in front and that work
    /// behind, the points the vectors' x and height.
    #[test]
    fn a_unit_sent_off_a_footprint_walks_to_the_spot_and_comes_back_to_its_work() {
        let site = Named {
            id: 10028,
            master: 70,
            owner: 1,
            vector: [30.0, 2.0],
        };
        let tree = Named {
            id: 600,
            master: 349,
            owner: 0,
            vector: [33.5, 2.0],
        };
        let off = OffFootprint {
            spot: Some([29.5, 89.5]),
            z: 2.0,
            class: VILLAGER_CLASS,
            computer: false,
            player: 1,
            site: Some(site),
            action_target: Some(tree),
        };
        let mut ai = UnitAi {
            current_action: GATHER_ACTION,
            orders: vec![QueuedOrder {
                order: MOVE_ORDER,
                ..QueuedOrder::default()
            }],
            ..UnitAi::default()
        };
        let mut side = OffSide(off, Vec::new());
        let mut fx = Vec::new();
        let n = Notify::own(9175, SENT_OFF, [100, 150, 10028]);
        assert_eq!(ai.sent_off(n, &mut side, &mut fx), Ok(GO_ON));
        assert_eq!(side.1, vec![(10028, [1.0, 1.5])]);
        assert_eq!(
            fx,
            vec![Effect::MoveAside(AiMove {
                point: [29.5, 89.5, 2.0],
                range: 0.0,
                force: true,
                walk: true,
            })]
        );
        assert_eq!(
            (ai.current_action, ai.last_action),
            (MOVE_ASIDE_ACTION, GATHER_ACTION)
        );
        let orders: Vec<_> = ai
            .orders
            .iter()
            .map(|o| (o.order, o.target, o.point, o.priority, o.ready))
            .collect();
        assert_eq!(
            orders,
            vec![
                (FOUNDATION_ORDER, 10028, [30.0, 2.0, 0.0], 100, true),
                (MOVE_ORDER, 0, [0.0; 3], 0, false),
                (ORDER_ON_TARGET, 600, [33.5, 2.0, 0.0], 100, true),
            ]
        );
    }

    /// What [`UnitSide::closest_resource`] answers, and the searches asked (group, workers cap,
    /// drop site, current target).
    struct ResourceSide(Option<Gatherable>, Vec<(u8, i32, i32, i32)>);

    impl UnitSide for ResourceSide {
        fn closest(&mut self, _: &Search<'_>) -> Result<Found, Unsupported> {
            Err(Unsupported("no search here"))
        }
        fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, Unsupported> {
            Err(Unsupported("no attack here"))
        }
        fn set_under_attack(&mut self, _: bool) {}
        fn stop(&mut self) -> Result<(), Unsupported> {
            Ok(())
        }
        fn may_attack(
            &mut self,
            _: i32,
            _: &Seeker,
        ) -> Result<Option<(i16, [f32; 3])>, Unsupported> {
            Err(Unsupported("no world here"))
        }
        fn seek_unexplored(
            &mut self,
            _: &UnitAi,
            _: &mut Well512,
        ) -> Result<Option<[i32; 2]>, Unsupported> {
            Err(Unsupported("no search here"))
        }
        fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, Unsupported> {
            Err(Unsupported("no search here"))
        }
        fn make_scout_map(&mut self) -> Result<(), Unsupported> {
            Err(Unsupported("no world here"))
        }
        fn closest_resource(
            &mut self,
            kind: u8,
            cap: i32,
            site: i32,
            current: i32,
        ) -> Result<Option<Gatherable>, Unsupported> {
            self.1.push((kind, cap, site, current));
            Ok(self.0)
        }
    }

    /// The villager's notify 0x202 on its gather (the 4v4's forager 31237 after update 10822):
    /// its target class (7, a bush: gather group 1) names the search from the notify's drop
    /// site, any workers; what it finds it gathers, the AI's target and action taken.
    #[test]
    fn a_gatherer_asked_for_another_target_gathers_what_its_search_finds() {
        let bush = Gatherable {
            id: 28200,
            position: [111.5, 16.5, 0.0],
            class: 7,
        };
        let mut ai = UnitAi {
            ai_type: CIVILIAN,
            current_order: crate::ai_kinds::ORDER_GATHER,
            current_action: GATHER_ACTION,
            current_target: 28202,
            target_type: 7,
            ..UnitAi::default()
        };
        let mut side = ResourceSide(Some(bush), Vec::new());
        let mut fx = Vec::new();
        let at = Surroundings::default();
        let n = Notify::own(31237, REQUIRES_SEARCH, [GATHER_ACTION, 7, 31247]);
        assert_eq!(
            ai.gatherer_search(n, &at, false, &mut side, &mut fx),
            Ok(BREAK)
        );
        assert_eq!(side.1, vec![(1, -1, 31247, 28202)]);
        assert_eq!(fx, vec![Effect::GatherObject(28200)]);
        assert_eq!(
            (ai.current_target, ai.last_target, ai.target_point),
            (28200, 28202, [111.5, 16.5, 0.0])
        );
        // gold is searched below 4 workers; finding nothing a human's villager stays as it is
        let mut ai = UnitAi {
            ai_type: CIVILIAN,
            current_action: GATHER_ACTION,
            target_type: 32,
            ..UnitAi::default()
        };
        let mut side = ResourceSide(None, Vec::new());
        let mut fx = Vec::new();
        let n = Notify::own(31237, REQUIRES_SEARCH, [GATHER_ACTION, 32, 0]);
        assert_eq!(
            ai.gatherer_search(n, &at, false, &mut side, &mut fx),
            Ok(GO_ON)
        );
        assert_eq!((side.1[0].0, side.1[0].1), (4, 4));
        assert_eq!(fx, Vec::new());
        assert_eq!(ai.current_action, GATHER_ACTION);
    }

    fn hit_boar() -> UnitAi {
        UnitAi {
            home: Some(Home::default()),
            ..idle(4)
        }
    }

    fn attacker(id: i32, class: i16, gap_squared: f32) -> UnitSeen {
        UnitSeen {
            id,
            class,
            state: 2,
            target: Some(9729),
            gap_squared,
            position: [100.0, 40.0, 0.0],
            ..UnitSeen::default()
        }
    }

    /// MarvinBlue/426048731's boar 9729 in update 4132: the hunter 10049's arrow makes it attack
    /// the hunter, forced (it was only wandering), furious at a villager; a second attacker's hit
    /// then adds to its attackers, but its `AttackObject` is not forced and fails: it keeps the
    /// first.
    #[test]
    fn a_boar_attacks_its_first_attacker_and_keeps_it() {
        let mut u = hit_boar();
        let at = Surroundings {
            id: 9729,
            units: vec![attacker(10049, 4, 9.0), attacker(10050, 4, 1.0)],
            ..Surroundings::default()
        };
        let hit = |caller| Notify {
            caller,
            recipient: 9729,
            kind: DAMAGED,
            p: [caller, 72, 75],
        };
        let mut side = Recorder::default();
        let mut fx = Vec::new();
        let r = u.boar_damaged(hit(10049), 0, &at, &mut rng(), &mut side, &mut fx);
        assert_eq!(r, Ok(BREAK));
        assert_eq!(
            (u.attackers.clone(), side.attacks.clone()),
            (vec![10049], vec![(10049, true)])
        );
        assert_eq!((u.current_action, u.current_target), (ATTACK_ACTION, 10049));
        assert_eq!(
            (side.under_attack, u.home.map(|h| h.furious)),
            (Some(true), Some(true))
        );
        // the nearer second attacker is the closest now, but the attack is not forced
        let _ = u.boar_damaged(hit(10050), 0, &at, &mut rng(), &mut side, &mut fx);
        assert_eq!(u.attackers, vec![10049, 10050]);
        assert_eq!(side.attacks, vec![(10049, true)]);
        assert_eq!(u.current_target, 10049);
    }

    /// The 4v4's boar 29825 in update 4599: its Attack on the villager 31126 completed (a stop,
    /// slot 0x3c0) and the villager 27240's hit makes it attack again, forced. The stop empties
    /// the action list before the attack command fills it, so it runs first, not after the AI.
    #[test]
    fn a_stop_before_an_attack_command_runs_first() {
        let mut u = UnitAi {
            current_action: ATTACK_ACTION,
            current_target: 31126,
            ..hit_boar()
        };
        let at = Surroundings {
            id: 29825,
            units: vec![attacker(27240, 4, 1.0)],
            ..Surroundings::default()
        };
        let mut side = Recorder::default();
        let mut fx = Vec::new();
        let done = Notify::own(29825, ACTION_COMPLETED, [600, 31126, 0]);
        let r = u.handle(done, 0, &at, false, &mut rng(), &mut side, &mut fx);
        assert_eq!((r, fx.clone()), (Ok(GO_ON), vec![Effect::Stop]));
        let hit = Notify {
            caller: 27240,
            recipient: 29825,
            kind: DAMAGED,
            p: [27240, 3, 33],
        };
        let _ = u.handle(hit, 0, &at, false, &mut rng(), &mut side, &mut fx);
        assert_eq!((side.stops, side.attacks), (vec![0], vec![(27240, true)]));
        assert_eq!(fx, vec![]);
        assert_eq!((u.current_action, u.current_target), (ATTACK_ACTION, 27240));
    }

    /// The 4v4's boar 29823 in update 2917 walks to its villager and meets another of the same
    /// master: a boar (class 10 of combat level 1) lets it be; a wolf (combat level 4) with no
    /// order attacks it, forced, unless its stance is not aggressive.
    #[test]
    fn a_unit_in_the_way_is_attacked_by_a_wolf_not_by_a_boar() {
        let at = |combat_level| Surroundings {
            id: 29823,
            class: CLASS_10,
            combat_level,
            units: vec![attacker(29001, 4, 1.0)],
            ..Surroundings::default()
        };
        let n = Notify::own(29823, IN_THE_WAY, [29001, -1, -1]);
        let mut side = Recorder::default();
        let mut fx = Vec::new();
        let mut boar = UnitAi {
            current_action: ATTACK_ACTION,
            current_target: 29002,
            ..hit_boar()
        };
        let r = boar.handle(n, 0, &at(1), false, &mut rng(), &mut side, &mut fx);
        assert_eq!((r, side.attacks.clone()), (Ok(GO_ON), vec![]));
        let mut wolf = UnitAi {
            current_action: ATTACK_ACTION,
            current_target: 29002,
            ..idle(5)
        };
        let r = wolf.handle(n, 0, &at(4), false, &mut rng(), &mut side, &mut fx);
        assert_eq!((r, side.attacks.clone()), (Ok(GO_ON), vec![(29001, true)]));
        assert_eq!(wolf.current_target, 29001);
        let mut defensive = UnitAi {
            stance: 1,
            ..idle(5)
        };
        let mut side = Recorder::default();
        let _ = defensive.handle(n, 0, &at(4), false, &mut rng(), &mut side, &mut fx);
        assert_eq!(side.attacks, []);
    }

    /// A lion's world: the answers of its two searches (a villager, then any enemy), the object
    /// it then reads, and the attack commands it gave.
    struct LionSide {
        found: [i32; 2],
        searches: usize,
        seen: UnitSeen,
        attacks: Vec<(i32, bool)>,
    }

    impl UnitSide for LionSide {
        fn closest(&mut self, _: &Search<'_>) -> Result<Found, Unsupported> {
            let id = self.found[self.searches];
            self.searches += 1;
            Ok(Found {
                id,
                enemies: id != -1,
                position: Some([0.0; 3]),
                fresh: true,
            })
        }
        fn attack(
            &mut self,
            target: i32,
            force: bool,
            _: &UnitAi,
        ) -> Result<Attacked, Unsupported> {
            self.attacks.push((target, force));
            Ok(Attacked {
                attacks: true,
                posted: Vec::new(),
            })
        }
        fn set_under_attack(&mut self, _: bool) {}
        fn stop(&mut self) -> Result<(), Unsupported> {
            Ok(())
        }
        fn may_attack(
            &mut self,
            _: i32,
            _: &Seeker,
        ) -> Result<Option<(i16, [f32; 3])>, Unsupported> {
            Err(Unsupported("no world here"))
        }
        fn seek_unexplored(
            &mut self,
            _: &UnitAi,
            _: &mut Well512,
        ) -> Result<Option<[i32; 2]>, Unsupported> {
            Err(Unsupported("no search here"))
        }
        fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, Unsupported> {
            Err(Unsupported("no search here"))
        }
        fn make_scout_map(&mut self) -> Result<(), Unsupported> {
            Err(Unsupported("no world here"))
        }
        fn seen_unit(&mut self, id: i32) -> Result<Option<UnitSeen>, Unsupported> {
            Ok((id == self.seen.id).then_some(self.seen))
        }
    }

    /// The lion's idle check (`FUN_140c1fb70`): a villager it finds within half its line of
    /// sight (outside mode 1) it attacks, not forced; finding none, any enemy within a third.
    #[test]
    fn an_idle_lion_attacks_what_it_finds_within_its_reach() {
        let run = |found: [i32; 2], gap_squared: f32| {
            let mut lion = idle(5);
            let at = Surroundings {
                los: 4.0,
                search_radius: 1.0,
                ..Surroundings::default()
            };
            let mut side = LionSide {
                found,
                searches: 0,
                seen: UnitSeen {
                    id: found[0].max(found[1]),
                    class: 4,
                    owner: 1,
                    state: 2,
                    gap_squared,
                    ..UnitSeen::default()
                },
                attacks: Vec::new(),
            };
            let mut fx = Vec::new();
            let produced = lion.lion_idle(&at, &mut rng(), &mut side, &mut fx).unwrap();
            // FUN_140c41e50 first
            assert_eq!(fx, vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)]);
            (
                produced,
                side.attacks,
                lion.current_action,
                lion.current_target,
            )
        };
        // a villager: the reach is 2 (half of 4); slot 0xb8 then gives 6 whatever it did
        assert_eq!(
            run([700, -1], 3.9),
            (true, vec![(700, false)], ATTACK_ACTION, 700)
        );
        assert_eq!(run([700, -1], 4.1), (false, vec![], -1, -1));
        // any enemy: a third of 4
        assert_eq!(
            run([-1, 701], 1.7),
            (true, vec![(701, false)], ATTACK_ACTION, 701)
        );
        assert_eq!(run([-1, 701], 1.8), (false, vec![], -1, -1));
    }

    /// A soldier's world: what its `bestUnitToAttack` finds and the attack commands it gave.
    struct SoldierSide {
        found: i32,
        attacks: Vec<(i32, bool)>,
    }

    impl UnitSide for SoldierSide {
        fn closest(&mut self, _: &Search<'_>) -> Result<Found, Unsupported> {
            Ok(Found {
                id: self.found,
                enemies: self.found != -1,
                position: Some([10.0, 11.0, 0.0]),
                fresh: true,
            })
        }
        fn attack(
            &mut self,
            target: i32,
            force: bool,
            _: &UnitAi,
        ) -> Result<Attacked, Unsupported> {
            self.attacks.push((target, force));
            Ok(Attacked {
                attacks: true,
                posted: Vec::new(),
            })
        }
        fn set_under_attack(&mut self, _: bool) {}
        fn stop(&mut self) -> Result<(), Unsupported> {
            Ok(())
        }
        fn may_attack(
            &mut self,
            _: i32,
            _: &Seeker,
        ) -> Result<Option<(i16, [f32; 3])>, Unsupported> {
            Ok(Some((58, [10.0, 11.0, 0.0])))
        }
        fn seek_unexplored(
            &mut self,
            _: &UnitAi,
            _: &mut Well512,
        ) -> Result<Option<[i32; 2]>, Unsupported> {
            Err(Unsupported("no search here"))
        }
        fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, Unsupported> {
            Err(Unsupported("no search here"))
        }
        fn make_scout_map(&mut self) -> Result<(), Unsupported> {
            Err(Unsupported("no world here"))
        }
    }

    /// A scout (master 448) in stand ground whose sheep died (census v4's blocker): its
    /// processIdle with 0 (`FUN_140c20db0`) attacks the next sheep `bestUnitToAttack` finds,
    /// forced, and its order goes; finding none, it moves to the dead sheep's point.
    #[test]
    fn a_scout_whose_sheep_died_attacks_the_next_or_walks_to_its_point() {
        let run = |found: i32| {
            let mut scout = UnitAi {
                current_order: ATTACK_ORDER,
                current_action: ATTACK_ACTION,
                current_target: 25954,
                target_type: 58,
                target_point: [5.0, 6.0, 0.0],
                stance: 2,
                ..idle(13)
            };
            let at = Surroundings {
                player: 1,
                kind: 70,
                master: 448,
                class: 47,
                weapon_range: 0.0,
                action_state: Some(0),
                ..Surroundings::default()
            };
            let mut side = SoldierSide {
                found,
                attacks: Vec::new(),
            };
            let mut fx = Vec::new();
            scout
                .next_after_kill(&at, false, &mut rng(), &mut side, &mut fx)
                .unwrap();
            (scout, side.attacks, fx)
        };
        let (scout, attacks, _) = run(26000);
        assert_eq!(attacks, vec![(26000, true)]);
        assert_eq!(
            (
                scout.current_action,
                scout.current_target,
                scout.current_order
            ),
            (ATTACK_ACTION, 26000, -1)
        );
        assert_eq!((scout.last_order, scout.order_priority), (-1, -1));
        let (scout, attacks, fx) = run(-1);
        assert_eq!(attacks, vec![]);
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::Move(m) if m.point == [5.0, 6.0, 0.0]))
        );
        assert_eq!((scout.current_order, scout.last_order), (-1, ATTACK_ORDER));
    }

    /// processMisc's retarget check past its timer (`0x140c567c0`): a scout on the attack order
    /// against an enemy's sheep looks no further in stand ground, nor while it swings (its Attack
    /// in state 6 or 7); otherwise the search for a better target follows.
    #[test]
    fn a_retarget_check_stops_in_stand_ground_and_while_attacking() {
        let scout = |stance: u8| UnitAi {
            current_order: ATTACK_ORDER,
            current_action: ATTACK_ACTION,
            current_target: 25954,
            stance,
            ..idle(13)
        };
        let sheep = Some((1, 58));
        assert_eq!(
            scout(2).retarget_outcome(sheep, 0.0, false, Some(4)),
            Ok(())
        );
        assert_eq!(
            scout(0).retarget_outcome(sheep, 0.0, false, Some(6)),
            Ok(())
        );
        assert!(
            scout(0)
                .retarget_outcome(sheep, 0.0, false, Some(4))
                .is_err()
        );
        // a building target is searched past even while attacking
        assert!(
            scout(0)
                .retarget_outcome(Some((1, 3)), 0.0, true, Some(6))
                .is_err()
        );
    }

    #[test]
    fn a_lion_attacks_by_its_priority_or_its_classes() {
        assert!(lion_may_attack(1, 3));
        assert!(!lion_may_attack(3, 4));
        assert!(lion_may_attack(3, 3));
        assert!(lion_may_attack(0, 4));
        assert!(!lion_may_attack(0, 13));
    }

    /// MarvinBlue/426048731's villager 10041 in update 4839, hunting the boar 9731 (its Hunt in
    /// state 12) that hits it: the boar is its target, so it calls nobody for help; it is under
    /// attack, and the hunt order with a class-10 target and a Gaia attacker ignores the damage.
    #[test]
    fn a_hunter_hit_by_its_boar_ignores_the_damage() {
        let mut u = UnitAi {
            current_order: crate::ai_kinds::ORDER_HUNT,
            current_action: 613,
            current_target: 9731,
            ..idle(8)
        };
        let boar = UnitSeen {
            owner: 0,
            ..attacker(9731, CLASS_10, 0.5)
        };
        let at = Surroundings {
            id: 10041,
            action_state: Some(12),
            target: Some(Seen {
                class: CLASS_10,
                ..Seen::default()
            }),
            units: vec![boar],
            ..Surroundings::default()
        };
        let hit = Notify {
            caller: 9731,
            recipient: 10041,
            kind: DAMAGED,
            p: [9731, 34, 40],
        };
        let mut side = Recorder::default();
        let mut fx = Vec::new();
        let r = u.villager_damaged(hit, 0, &at, false, &mut rng(), &mut side, &mut fx);
        assert_eq!(r, Ok(GO_ON));
        assert_eq!(
            (u.attackers.clone(), side.under_attack),
            (vec![9731], Some(true))
        );
        assert_eq!(
            (u.current_order, u.current_target),
            (crate::ai_kinds::ORDER_HUNT, 9731)
        );
        // another player's attacker that is not its target would make it call for help
        let enemy = UnitSeen {
            owner: 1,
            ..attacker(9800, 4, 1.0)
        };
        let at = Surroundings {
            units: vec![enemy],
            ..at
        };
        let hit = Notify {
            caller: 9800,
            ..hit
        };
        assert!(
            u.villager_damaged(hit, 0, &at, false, &mut rng(), &mut side, &mut fx)
                .is_err()
        );
    }

    #[test]
    fn closest_valid_attacker_skips_classes_and_flips_a_coin_on_a_tie() {
        let mut u = hit_boar();
        u.attackers = vec![1, 2, 3];
        let at = Surroundings {
            units: vec![
                attacker(1, 13, 0.5),
                attacker(2, 4, 4.0),
                attacker(3, 4, 4.03),
            ],
            ..Surroundings::default()
        };
        // 1 is of class 13; 3 lies within 0.0625 of 2: the coin decides
        let mut r = rng();
        let mut coin = rng();
        let picked = u.closest_valid_attacker(&at, &mut r);
        let expected = if coin.rand15() & 1 == 0 { 2 } else { 3 };
        assert_eq!(picked, expected);
    }

    #[test]
    fn an_attacker_that_no_longer_targets_the_unit_leaves_one_an_update() {
        let mut u = hit_boar();
        u.attackers = vec![10049, 10050];
        let mut walked_off = attacker(10049, 4, 1.0);
        walked_off.target = Some(-1);
        let mut dead = attacker(10050, 4, 1.0);
        dead.state = 3;
        let at = Surroundings {
            id: 9729,
            under_attack: true,
            units: vec![walked_off, dead],
            ..Surroundings::default()
        };
        let mut side = Recorder::default();
        u.keep_attackers(&at, &mut side);
        assert_eq!(u.attackers, vec![10050]);
        u.keep_attackers(&at, &mut side);
        assert_eq!(u.attackers, Vec::<i32>::new());
        assert_eq!(side.under_attack, None);
        u.keep_attackers(&at, &mut side);
        assert_eq!(side.under_attack, Some(false));
    }

    /// MarvinBlue/426048731's boar 9729 in updates 4157 to 4159: the hunter 10049 moves off, its
    /// Hunt's teardown lets the boar go (notify 699 with the hunter's class); the boar's AI takes
    /// it, as 10049 is one of its attackers, and its base handler drops the attacker. A 699 from
    /// a unit that is not attacking it is ignored.
    #[test]
    fn a_released_attacker_leaves_the_set_through_notify_699() {
        let mut u = hit_boar();
        u.attackers = vec![10049];
        let released = |caller| Notify {
            caller,
            recipient: 9729,
            kind: NO_LONGER_ON_ME,
            p: [4, 0, 0],
        };
        u.post(released(10050), 2).unwrap();
        assert_eq!(u.notifies, Vec::new());
        u.post(released(10049), 2).unwrap();
        assert_eq!(u.notifies, vec![released(10049)]);
        let r = u.base_notify(
            released(10049),
            0,
            &Surroundings::default(),
            false,
            &mut rng(),
            &mut Recorder::default(),
            &mut Vec::new(),
        );
        assert_eq!((r, u.attackers.clone()), (Ok(GO_ON), Vec::new()));
    }

    /// A builder on its build order (702, action 602): MarvinBlue/426048731's 9175 in update 681,
    /// whose look-around timer runs out and is redrawn from the AI stream.
    #[test]
    fn a_builder_s_idle_timer_stays_0_and_its_look_around_is_redrawn() {
        let mut u = UnitAi {
            current_order: BUILD_ORDER,
            current_action: BUILD_ACTION,
            current_target: 10028,
            idle_timer: 120,
            secondary_timer: 10,
            look_around_timer: 990,
            ..idle(8)
        };
        let mut r = rng();
        let at = Surroundings {
            position: [29.5, 89.5, 0.0],
            busy: true,
            player: 2,
            class: 4,
            ..Surroundings::default()
        };
        let _ = u.update(50, 0, false, &at, &mut r, &mut no_search).unwrap();
        assert_eq!(
            (u.idle_timer, u.secondary_timer, u.look_around_timer),
            (0, 0, 0)
        );
        assert_eq!(u.look_around_timeout, 4500 + 7241 % 1000);
        assert_eq!(r.draws, 1);
        // the state position follows a unit with an order
        assert_eq!(u.state_position, [29.5, 89.5]);
        // another order is not modelled
        u.current_order = 700;
        assert!(u.update(50, 0, false, &at, &mut r, &mut no_search).is_err());
    }

    #[test]
    fn a_work_notify_sets_the_order_and_redraws_a_timeout_left_by_order_711() {
        let mut u = UnitAi {
            current_order: BUILD_ORDER,
            current_action: BUILD_ACTION,
            current_target: 10028,
            adjusted_idle_timeout: 750,
            idle_timeout: 3000,
            secondary_timer: 900,
            ..idle(8)
        };
        let mut r = rng();
        u.notify_work(
            HUNT_ORDER,
            HUNT_ACTION,
            Some((9365, 15)),
            [24.5, 97.5, 0.0],
            true,
            &mut r,
        )
        .unwrap();
        assert_eq!(
            (u.current_order, u.current_action, u.current_target, r.draws),
            (HUNT_ORDER, HUNT_ACTION, 9365, 0)
        );
        u.current_order = 711;
        u.notify_work(
            HUNT_ORDER,
            HUNT_ACTION,
            Some((9365, 15)),
            [24.5, 97.5, 0.0],
            true,
            &mut r,
        )
        .unwrap();
        assert_eq!((r.draws, u.secondary_timer), (1, 750));
        assert_ne!(u.adjusted_idle_timeout, 750);
    }

    #[test]
    fn a_defending_unit_beyond_five_tiles_of_its_state_position_stops_to_walk_back() {
        // records/425906646's update 1404: player 8's unit on the defend stance (1); within 5
        // tiles of its state position (from its footprint) it goes on to the retarget check
        let defender = |x: f32| {
            let u = UnitAi {
                stance: 1,
                current_order: -1,
                current_action: 0x25a,
                current_target: 4321,
                state_position: [10.0, 10.0],
                retarget_timer: u32::MAX,
                ..idle(13)
            };
            let at = Surroundings {
                position: [x, 10.0, 0.0],
                size: [0.5, 0.5],
                ..Surroundings::default()
            };
            (u, at)
        };
        let (mut u, at) = defender(15.4);
        let mut fx = Vec::new();
        let mut r = rng();
        assert!(
            u.retarget_check(0, false, &at, &mut r, false, &mut fx)
                .is_ok()
        );
        assert_eq!(fx, Vec::<Effect>::new());
        // 5.1 tiles off its footprint: it stops, forgets its target and walks back to its state
        // position (FUN_140c5e7d0), then goes on to the retarget check (its timer not used up)
        let (mut u, at) = defender(15.6);
        assert_eq!(
            u.retarget_check(0, false, &at, &mut r, false, &mut fx),
            Ok(false)
        );
        assert_eq!(
            fx,
            vec![
                Effect::Stop,
                Effect::Move(AiMove {
                    point: [10.0, 10.0, 0.0],
                    range: 1.0,
                    force: true,
                    walk: false,
                })
            ]
        );
        assert_eq!((u.current_action, u.current_target), (MOVE_ACTION, -1));
        assert_eq!(u.last_target, 4321);
    }

    #[test]
    fn retarget_waits_for_the_timer() {
        let mut u = idle(17);
        let mut r = rng();
        let _ = u
            .update_idle(
                50,
                899,
                false,
                &Surroundings::default(),
                &mut r,
                &mut no_search,
            )
            .unwrap();
        assert_eq!((u.retarget_timer, r.draws), (900, 0));
        let _ = u
            .update_idle(
                50,
                900,
                false,
                &Surroundings::default(),
                &mut r,
                &mut no_search,
            )
            .unwrap();
        // the first rand15 after seed 0 is 7241
        assert_eq!(
            (u.last_retarget_time, u.retarget_timer),
            (900, 750 + 7241 % 500)
        );
    }

    #[test]
    fn a_last_retarget_ahead_of_the_world_time_wraps_and_retargets() {
        let mut u = UnitAi {
            last_retarget_time: 400,
            ..idle(15)
        };
        let mut r = rng();
        let _ = u
            .update_idle(
                50,
                100,
                false,
                &Surroundings::default(),
                &mut r,
                &mut no_search,
            )
            .unwrap();
        assert_eq!((u.last_retarget_time, r.draws), (100, 1));
    }

    #[test]
    fn villagers_and_livestock_never_retarget() {
        for t in [8, 19] {
            let mut u = idle(t);
            let mut r = rng();
            let _ = u
                .update_idle(
                    50,
                    5000,
                    false,
                    &Surroundings::default(),
                    &mut r,
                    &mut no_search,
                )
                .unwrap();
            assert_eq!((u.retarget_timer, r.draws), (900, 0), "type {t}");
        }
    }

    #[test]
    fn the_look_around_timeout_is_redrawn_once_exceeded() {
        let mut u = UnitAi {
            retarget_disabled: 1,
            ..idle(8)
        };
        let mut r = rng();
        let _ = u
            .update_idle(
                1000,
                0,
                false,
                &Surroundings::default(),
                &mut r,
                &mut no_search,
            )
            .unwrap();
        assert_eq!((u.look_around_timer, r.draws), (1000, 0));
        let _ = u
            .update_idle(
                1,
                0,
                false,
                &Surroundings::default(),
                &mut r,
                &mut no_search,
            )
            .unwrap();
        assert_eq!(
            (u.look_around_timer, u.look_around_timeout),
            (0, 4500 + 7241 % 1000)
        );
    }

    #[test]
    fn retarget_draws_before_look_around() {
        let mut u = UnitAi {
            look_around_timer: 1000,
            ..idle(13)
        };
        let mut r = rng();
        let _ = u
            .update_idle(
                1,
                900,
                false,
                &Surroundings::default(),
                &mut r,
                &mut no_search,
            )
            .unwrap();
        assert_eq!(u.retarget_timer, 750 + 7241 % 500);
        assert_eq!(u.look_around_timeout, 4500 + 25269 % 1000);
    }

    #[test]
    fn the_idle_timer_restarts_after_its_timeout_and_the_secondary_timer_stops_at_0() {
        let mut u = UnitAi {
            secondary_timer: 750,
            ..idle(8)
        };
        let mut r = rng();
        let _ = u
            .update_idle(
                3001,
                0,
                false,
                &Surroundings::default(),
                &mut r,
                &mut no_search,
            )
            .unwrap();
        assert_eq!((u.idle_timer, u.secondary_timer), (0, 0));
    }

    #[test]
    fn non_idle_units_and_unknown_types_are_unsupported() {
        let mut r = rng();
        let mut busy = UnitAi {
            current_action: 600,
            ..idle(17)
        };
        assert!(
            busy.update_idle(
                1,
                0,
                false,
                &Surroundings::default(),
                &mut r,
                &mut no_search
            )
            .is_err()
        );
        // a type the slice does not model
        assert!(
            idle(99)
                .update_idle(
                    1,
                    0,
                    false,
                    &Surroundings::default(),
                    &mut r,
                    &mut no_search
                )
                .is_err()
        );
    }

    #[test]
    fn a_new_building_unit_ai_draws_its_three_timers() {
        // UnitAIModule's constructor: last retarget r % 500, adjusted idle timeout r % 1500 + 2250,
        // retarget timer 750 + r % 500, in that order
        let mut r = rng();
        let ai = UnitAi::constructed(17, [30.0, 88.0, 0.0], &mut r).unwrap();
        let mut expected = rng();
        let (a, b, c) = (expected.rand15(), expected.rand15(), expected.rand15());
        assert_eq!(r.draws, 3);
        assert_eq!(
            ai,
            UnitAi {
                ai_type: 17,
                current_order: -1,
                current_action: -1,
                current_target: -1,
                target_type: -1,
                target_point: [-1.0; 3],
                desired_distance: 2.0,
                last_order: -1,
                last_action: -1,
                last_target: -1,
                last_target_type: -1,
                idle_timer: 0,
                adjusted_idle_timeout: b % 1500 + 2250,
                idle_timeout: 3000,
                secondary_timer: 750,
                look_around_timer: 0,
                look_around_timeout: 1000,
                last_retarget_time: a % 500,
                retarget_timer: 750 + c % 500,
                stance: 0,
                formation_type: 0,
                state_position: [30.0, 88.0],
                retarget_disabled: 0,
                alert_mode: false,
                time_since_enemy: 0.0,
                patrol_path: false,
                patrol_waypoint: 0,
                stop_after_kill: false,
                home: None,
                hunted: None,
                order_priority: -1,
                last_update_time: 0,
                orders: Vec::new(),
                building_orders: Vec::new(),
                notifies: Vec::new(),
                notify_capacity: 10,
                processing: false,
                buffered: Vec::new(),
                commanded: false,
                attackers: Vec::new(),
            }
        );
    }

    #[test]
    fn a_unit_ai_class_without_a_modelled_constructor_draws_nothing() {
        let mut r = rng();
        assert!(UnitAi::constructed(13, [0.0; 3], &mut r).is_err());
        assert_eq!(r.draws, 0);
    }

    #[test]
    fn an_order_redraws_an_adjusted_idle_timeout_of_750_or_more_and_caps_the_secondary_timer() {
        // FUN_140e6cfc0 for each unit of an INTERACT, MOVE or RETREAT
        let mut u = UnitAi {
            idle_timeout: 3000,
            adjusted_idle_timeout: 2600,
            secondary_timer: 900,
            patrol_path: true,
            patrol_waypoint: 3,
            ..idle(13)
        };
        let mut r = rng();
        u.reset_for_order(&mut r);
        assert_eq!(
            (u.adjusted_idle_timeout, u.secondary_timer),
            (7241 % 1500 + 2250, 750)
        );
        // FUN_140c68de0 first: the patrol path and its waypoint go
        assert_eq!((u.patrol_path, u.patrol_waypoint), (false, 0));
        let mut low = UnitAi {
            idle_timeout: 3000,
            adjusted_idle_timeout: 749,
            ..idle(13)
        };
        let mut r = rng();
        low.reset_for_order(&mut r);
        assert_eq!((low.adjusted_idle_timeout, r.draws), (749, 0));
    }

    /// MarvinBlue/426048731's boar 9732 (type 4) at its origin (23.5, 105.5), as saved after 680.
    fn boar() -> UnitAi {
        UnitAi {
            idle_timer: 3000,
            adjusted_idle_timeout: 3005,
            idle_timeout: 3000,
            look_around_timeout: 5426,
            retarget_timer: 1205,
            target_type: -1,
            target_point: [-1.0; 3],
            last_order: -1,
            last_action: -1,
            last_target: -1,
            last_target_type: -1,
            stance: 0,
            home: Some(Home {
                origin: [23.5, 105.5, 0.0],
                ..Home::default()
            }),
            ..idle(4)
        }
    }

    fn at(position: [f32; 3]) -> Surroundings {
        Surroundings {
            position,
            class: 11,
            ..Surroundings::default()
        }
    }

    /// A seed whose idle check (a scaled draw, then rand15) wanders, or does not.
    #[allow(clippy::maybe_infinite_iter)] // the search ends at the first seed that fits
    fn seed_for(wanders: bool) -> Well512 {
        (0..)
            .map(|seed| {
                let mut r = Well512::default();
                r.set_seed(seed);
                r
            })
            .find(|r| {
                let mut e = r.clone();
                let _ = e.below(81);
                (e.rand15() % 7 == 0) == wanders
            })
            .unwrap()
    }

    #[test]
    fn a_boar_at_home_draws_its_leash_and_mostly_stays() {
        // updates 766, 910 and 1039: processIdle draws `0..81` and a rand15, which is not a
        // multiple of 7; nothing moves
        let mut u = boar();
        let mut r = seed_for(false);
        let moved = u.update_idle(
            10,
            0,
            false,
            &at([23.5, 105.5, 0.0]),
            &mut r,
            &mut no_search,
        );
        assert_eq!(moved, Ok(None));
        assert_eq!((u.idle_timer, u.current_action, r.draws), (0, -1, 2));
    }

    #[test]
    fn a_boar_wanders_up_to_three_tiles_once_in_seven() {
        let mut u = boar();
        let mut r = seed_for(true);
        let mut e = r.clone();
        let _ = e.below(81);
        let _ = e.rand15();
        let (dx, dy) = (e.rand15() % 6, e.rand15() % 6);
        let moved = u
            .update_idle(
                10,
                0,
                false,
                &at([23.5, 105.5, 0.0]),
                &mut r,
                &mut no_search,
            )
            .unwrap()
            .unwrap();
        let point = [dx as f32 - 3.0 + 23.5, dy as f32 - 3.0 + 105.5, 0.0];
        assert_eq!(
            moved,
            AiMove {
                point,
                range: 0.5,
                force: true,
                walk: true,
            }
        );
        // slot 0x130: the move action, no target, the point; the last action is the old one
        assert_eq!(
            (
                u.current_action,
                u.last_action,
                u.current_target,
                u.target_point
            ),
            (0x262, -1, -1, point)
        );
        assert_eq!(r.draws, 4);
    }

    #[test]
    fn a_boar_away_from_home_goes_back_and_a_furious_one_stays() {
        // more than 2 tiles from its origin and once in 7: home, at its own height
        let mut u = boar();
        let mut r = seed_for(true);
        let moved = u
            .update_idle(
                10,
                0,
                false,
                &at([26.0, 105.5, 0.5]),
                &mut r,
                &mut no_search,
            )
            .unwrap()
            .unwrap();
        assert_eq!(moved.point, [23.5, 105.5, 0.5]);
        assert_eq!(r.draws, 2);
        // beyond the leash (more than 144 + 80, squared) it heads home unless it already does
        let mut far = boar();
        let mut r = seed_for(false);
        let moved = far.update_idle(
            10,
            0,
            false,
            &at([43.5, 105.5, 0.0]),
            &mut r,
            &mut no_search,
        );
        assert_eq!(moved.unwrap().map(|m| m.point), Some([23.5, 105.5, 0.0]));
        assert_eq!(r.draws, 1);
        let mut heading = UnitAi {
            target_point: [23.5, 0.0, 0.0],
            ..boar()
        };
        let mut r = seed_for(false);
        let moved = heading.update_idle(
            10,
            0,
            false,
            &at([43.5, 105.5, 0.0]),
            &mut r,
            &mut no_search,
        );
        assert_eq!((moved, heading.current_action), (Ok(None), -1));
        // furious, or in mode 1, it does not draw at all; without a target it calms down after
        for (furious, mode_1) in [(true, false), (false, true)] {
            let mut u = boar();
            u.home.as_mut().unwrap().furious = furious;
            let mut r = rng();
            let place = Surroundings {
                mode_1,
                ..at([23.5, 105.5, 0.0])
            };
            assert_eq!(
                u.update_idle(10, 0, false, &place, &mut r, &mut no_search),
                Ok(None)
            );
            assert_eq!((r.draws, u.home.unwrap().furious), (0, false));
        }
    }

    #[test]
    fn a_deer_s_adjusted_idle_timeout_is_five_quarters_of_its_idle_timeout() {
        // FUN_140c17170, the deer's and fox's slot 0x1a8: no draw
        let mut u = UnitAi {
            idle_timeout: 4000,
            adjusted_idle_timeout: 3000,
            ..idle(2)
        };
        let mut r = rng();
        u.reset_for_order(&mut r);
        assert_eq!((u.adjusted_idle_timeout, r.draws), (5000, 0));
    }

    #[test]
    fn a_move_forgets_the_target_and_an_order_keeps_its_history() {
        // MOVE of 9185 in 1044: order 710, action 610, no target, the point on the ground
        let mut u = UnitAi {
            current_target: 9365,
            target_type: 15,
            ..idle(13)
        };
        u.notify_move(true, None, [21.947_916, 92.489_586], &mut rng());
        assert_eq!(
            (
                u.current_order,
                u.current_action,
                u.current_target,
                u.target_type
            ),
            (0x2c6, 0x262, -1, -1)
        );
        assert_eq!(
            (
                u.last_order,
                u.last_action,
                u.last_target,
                u.last_target_type
            ),
            (-1, -1, -1, -1)
        );
        assert_eq!(u.target_point, [21.947_916, 92.489_586, 0.0]);
    }

    #[test]
    fn an_idle_transport_ship_retargets_like_a_soldier() {
        // type 11, TribeTransportShipUnitAIModule: base slots 4 and 73, slot 72 0x140c21d20 draws
        // nothing
        let mut u = idle(11);
        let mut r = rng();
        let _ = u
            .update_idle(
                50,
                900,
                false,
                &Surroundings::default(),
                &mut r,
                &mut no_search,
            )
            .unwrap();
        assert_eq!(
            (u.last_retarget_time, u.retarget_timer),
            (900, 750 + 7241 % 500)
        );
    }

    fn deer() -> UnitAi {
        UnitAi {
            idle_timer: 5000,
            adjusted_idle_timeout: 5000,
            idle_timeout: 4000,
            look_around_timeout: 5424,
            retarget_timer: 1212,
            target_type: -1,
            target_point: [-1.0; 3],
            last_order: -1,
            last_action: -1,
            last_target: -1,
            last_target_type: -1,
            hunted: Some(Hunted {
                origin: [31.5, 106.5, 0.0],
                leash: 0.0,
            }),
            ..idle(2)
        }
    }

    fn deer_at(position: [f32; 3]) -> Surroundings {
        Surroundings {
            position,
            class: 9,
            size: [0.2, 0.2],
            los: 2.0,
            search_radius: 1.0,
            ..Surroundings::default()
        }
    }

    /// A stream whose next rand15 is a multiple of 7 or not.
    #[allow(clippy::maybe_infinite_iter)] // the search ends at the first seed that fits
    fn wander_seed(wanders: bool) -> Well512 {
        (0..)
            .map(|seed| {
                let mut r = Well512::default();
                r.set_seed(seed);
                r
            })
            .find(|r| (r.clone().rand15() % 7 == 0) == wanders)
            .unwrap()
    }

    #[test]
    fn a_deer_that_sees_nothing_stays_six_times_in_seven_and_otherwise_wanders() {
        let mut searched = Vec::new();
        let mut nothing = |s: &Search<'_>| {
            searched.push((s.at, s.radius, s.classes.len(), s.size));
            Ok(Found::NONE)
        };
        let mut u = deer();
        let mut r = wander_seed(false);
        let place = deer_at([31.5, 106.5, 0.0]);
        let moved = u
            .update_idle(10, 0, false, &place, &mut r, &mut nothing)
            .unwrap();
        assert_eq!(moved, None);
        // the search reaches the line of sight plus 1, the search radius being shorter
        assert_eq!(searched, vec![([31.5, 106.5], 3.0, 28, [0.2, 0.2])]);
        assert_eq!(r.draws, 1);

        let mut u = deer();
        let mut r = wander_seed(true);
        let mut e = r.clone();
        let _ = e.rand15();
        let step = |n: u32| if 3 <= n { n as i32 - 6 } else { n as i32 + 1 };
        let (dx, dy) = (step(e.rand15() % 6), step(e.rand15() % 6));
        let moved = u
            .update_idle(10, 0, false, &place, &mut r, &mut |_: &Search<'_>| {
                Ok(Found::NONE)
            })
            .unwrap()
            .unwrap();
        assert_eq!(moved.point, [31.5 + dx as f32, 106.5 + dy as f32, 0.0]);
        assert!(dx != 0 && dy != 0 && dx.abs() <= 3 && dy.abs() <= 3);
        assert_eq!(r.draws, 3);
    }

    #[test]
    fn a_deer_away_from_home_goes_back_and_one_beyond_its_leash_at_once() {
        let mut u = deer();
        let mut r = wander_seed(true);
        let away = deer_at([34.5, 106.5, 0.5]);
        let moved = u
            .update_idle(10, 0, false, &away, &mut r, &mut |_: &Search<'_>| {
                Ok(Found::NONE)
            })
            .unwrap()
            .unwrap();
        assert_eq!((moved.point, r.draws), ([31.5, 106.5, 0.5], 1));
        // a fox's leash of 2.5: 3 tiles off (2.8 from its footprint) it goes home without a
        // search or a draw
        let mut u = deer();
        u.hunted.as_mut().unwrap().leash = 2.5;
        let mut r = wander_seed(false);
        let moved = u
            .update_idle(10, 0, false, &away, &mut r, &mut no_search)
            .unwrap()
            .unwrap();
        assert_eq!((moved.point, r.draws), ([31.5, 106.5, 0.5], 0));
    }

    /// A deer that sees an object runs from it by three times its speed (slot 0x150), forced
    /// and running, and holds the move action and the point; an enemy among what it saw sets its
    /// alert mode and starts its sighting time again (deer 9655 at update 2626).
    #[test]
    fn a_deer_that_sees_an_object_evades_it() {
        let mut u = UnitAi {
            time_since_enemy: 12.0,
            ..deer()
        };
        let mut r = wander_seed(false);
        let place = Surroundings {
            speed: 1.2,
            map: [120, 120],
            ..deer_at([31.5, 106.5, 0.0])
        };
        let enemy = |_: &Search<'_>| {
            Ok(Found {
                id: 9183,
                enemies: true,
                position: Some([33.5, 106.5, 0.0]),
                fresh: true,
            })
        };
        let moved = u
            .update_idle(10, 0, false, &place, &mut r, &mut enemy.clone())
            .unwrap()
            .unwrap();
        assert_eq!(
            (moved.point, moved.range, moved.force, moved.walk),
            ([31.5 - 1.2 * 3.0, 106.5, 0.0], 0.0, true, false)
        );
        assert_eq!(
            (u.current_action, u.target_point),
            (MOVE_ACTION, moved.point)
        );
        assert_eq!((u.alert_mode, u.time_since_enemy), (true, 0.0));
    }

    #[test]
    fn an_animal_whose_wander_ended_drops_its_move_action_and_turns_idle() {
        let mut u = UnitAi {
            current_action: 0x262,
            idle_timer: 10,
            ..deer()
        };
        let mut r = wander_seed(false);
        let place = deer_at([31.5, 106.5, 0.0]);
        let moved = u
            .update(10, 0, false, &place, &mut r, &mut no_search)
            .unwrap();
        assert_eq!((moved, u.current_action, u.idle_timer), (vec![], -1, 20));
        // while it walks, its timers run and nothing else happens
        let mut u = UnitAi {
            current_action: 0x262,
            ..deer()
        };
        let walking = Surroundings {
            busy: true,
            ..place
        };
        let _ = u
            .update(10, 0, false, &walking, &mut r, &mut no_search)
            .unwrap();
        assert_eq!(
            (u.current_action, u.idle_timer, u.state_position),
            (0x262, 5010, [0.0; 2])
        );
    }

    /// MarvinBlue/426048731's scout 9183 (soldier AI, type 13) as saved after 821: on its MOVE
    /// order with five queued moves, its `MoveTo` about to arrive.
    fn scout() -> UnitAi {
        let order = |x: f32, y: f32| QueuedOrder {
            issuer: 1,
            order: MOVE_ORDER,
            priority: 100,
            target: -1,
            target_owner: -1,
            point: [x, y, 0.0],
            range: 0.0,
            shown: [x, y],
            ready: true,
            target_type: -1,
        };
        UnitAi {
            current_order: MOVE_ORDER,
            order_priority: -1,
            current_action: MOVE_ACTION,
            target_type: -1,
            target_point: [93.583_336, 57.791_668, 0.0],
            last_order: MOVE_ORDER,
            last_action: MOVE_ACTION,
            last_target: -1,
            last_target_type: -1,
            stance: 2,
            orders: vec![order(89.09375, 60.677_082), order(91.822_914, 52.489_582)],
            ..idle(13)
        }
    }

    fn scout_at(interruptible: bool, busy: bool) -> Surroundings {
        Surroundings {
            position: [93.5, 58.25, 0.0],
            busy,
            player: 1,
            class: 12,
            interruptible,
            ..Surroundings::default()
        }
    }

    #[test]
    fn between_turns_an_arrived_move_runs_the_next_queued_order() {
        // update 822: the MoveTo arrived (state 1, which slot 0x2d0 lets go), so FUN_140c68980
        // runs the queue: the first move becomes the AI's (order 710 at priority 100, action 610,
        // its point) and the unit's, forced and walking; the order leaves the queue
        let mut u = scout();
        let fx = u.off_turn(&scout_at(true, true)).unwrap();
        assert_eq!(
            fx,
            vec![Effect::Move(AiMove {
                point: [89.09375, 60.677_082, 0.0],
                range: 0.0,
                force: true,
                walk: true,
            })]
        );
        assert_eq!(
            (u.current_order, u.order_priority, u.current_action),
            (MOVE_ORDER, 100, MOVE_ACTION)
        );
        assert_eq!(u.target_point, [89.09375, 60.677_082, 0.0]);
        assert_eq!(u.orders.len(), 1);
        assert_eq!(u.orders[0].point, [91.822_914, 52.489_582, 0.0]);
    }

    #[test]
    fn between_turns_a_walking_unit_or_one_with_notifies_keeps_its_queue() {
        let mut walking = scout();
        assert_eq!(walking.off_turn(&scout_at(false, true)), Ok(vec![]));
        assert_eq!(walking.orders.len(), 2);
        let mut told = UnitAi {
            notifies: vec![Notify::own(9183, ACTION_COMPLETED, [MOVE_ACTION, 0, 0])],
            ..scout()
        };
        assert_eq!(told.off_turn(&scout_at(true, true)), Ok(vec![]));
        assert_eq!(told.orders.len(), 2);
        // an order not ready stays and the next one runs
        let mut u = scout();
        u.orders[0].ready = false;
        let _ = u.off_turn(&scout_at(true, true)).unwrap();
        assert_eq!(u.orders.len(), 1);
        assert_eq!(u.orders[0].point, [89.09375, 60.677_082, 0.0]);
    }

    #[test]
    fn a_completed_move_on_its_order_ends_the_order_and_stops_the_unit() {
        // the base handler's 0x1fa: 610 + 100 = 710, so slot 0xb0 ends the order too; the
        // notifies go with the owner's time; the queue then runs (the unit is no longer busy)
        let mut u = UnitAi {
            orders: vec![],
            notifies: vec![Notify::own(9183, ACTION_COMPLETED, [MOVE_ACTION, 0, 0])],
            ..scout()
        };
        let mut r = rng();
        let fx = u
            .update(
                60,
                900,
                false,
                &scout_at(true, false),
                &mut r,
                &mut no_search,
            )
            .unwrap();
        assert_eq!(fx, vec![Effect::Stop]);
        assert_eq!(
            (u.current_order, u.order_priority, u.current_action),
            (-1, -1, -1)
        );
        assert_eq!((u.last_order, u.last_action), (MOVE_ORDER, MOVE_ACTION));
        assert_eq!(u.state_position, [93.5, 58.25]);
        assert_eq!((u.notifies.len(), u.last_update_time), (0, 60));
    }

    #[test]
    fn a_failed_action_stops_like_a_completed_one_and_an_order_s_own_failure_ends_it() {
        // records/425906646's update 1234: a Gaia animal's own MoveTo failed (state 13, notify
        // 0x1f9); without an order the failure of the current action stops it
        let mut u = UnitAi {
            current_order: -1,
            current_action: MOVE_ACTION,
            current_target: 31234,
            ..UnitAi::default()
        };
        let mut fx = Vec::new();
        let n = Notify::own(9183, ACTION_FAILED, [MOVE_ACTION, 0, 0]);
        assert_eq!(
            u.action_failed(
                n,
                &at([10.0, 10.0, 0.0]),
                false,
                &mut rng(),
                &mut Recorder::default(),
                &mut fx
            )
            .unwrap(),
            GO_ON
        );
        assert_eq!((u.current_action, u.last_action), (-1, MOVE_ACTION));
        assert_eq!((u.current_target, u.last_target), (-1, 31234));
        // on its order (610 + 100) the failure ends the order too and sets the state position
        let mut u = UnitAi {
            current_order: MOVE_ORDER,
            current_action: MOVE_ACTION,
            ..UnitAi::default()
        };
        let mut fx = Vec::new();
        let _ = u
            .action_failed(
                n,
                &at([10.0, 12.0, 0.0]),
                false,
                &mut rng(),
                &mut Recorder::default(),
                &mut fx,
            )
            .unwrap();
        assert_eq!((u.current_order, u.current_action), (-1, -1));
        assert_eq!(u.state_position, [10.0, 12.0]);
        // another action's failure goes on; a failed attack on the attack order of a unit AI
        // but a boar's is not modelled
        let mut u = UnitAi {
            current_order: -1,
            current_action: 0x25a,
            ..UnitAi::default()
        };
        let _ = u
            .action_failed(
                n,
                &at([10.0, 12.0, 0.0]),
                false,
                &mut rng(),
                &mut Recorder::default(),
                &mut fx,
            )
            .unwrap();
        assert_eq!(u.current_action, 0x25a);
        let mut u = UnitAi {
            current_order: 700,
            ..UnitAi::default()
        };
        let attack = Notify::own(9183, ACTION_FAILED, [600, 0, 0]);
        assert!(
            u.action_failed(
                attack,
                &at([10.0, 12.0, 0.0]),
                false,
                &mut rng(),
                &mut Recorder::default(),
                &mut fx
            )
            .is_err()
        );
    }

    /// records/425906646's wolf 29827 (Gaia, a boar's unit AI) in update 3891: its Attack's target
    /// went out of sight, which sent it home (`+0x1fd`); the failure's processIdle (slot 0x240)
    /// walks it there and breaks off. A boar that stays forgets its target and stops.
    #[test]
    fn a_boar_s_failed_attack_runs_its_idle_check() {
        let home = |go_home, furious| Home {
            origin: [171.5, 51.5, 0.0],
            furious,
            go_home,
        };
        let boar = |h| UnitAi {
            ai_type: BOAR_AI,
            current_order: -1,
            current_action: 600,
            current_target: 31110,
            home: Some(h),
            ..UnitAi::default()
        };
        let attack = Notify::own(29827, ACTION_FAILED, [600, 0, 0]);
        let mut u = boar(home(true, false));
        let mut fx = Vec::new();
        let r = u.action_failed(
            attack,
            &at([169.5, 50.6, 0.0]),
            false,
            &mut rng(),
            &mut Recorder::default(),
            &mut fx,
        );
        assert_eq!(r, Ok(PRODUCED));
        assert_eq!(first_move(&fx).map(|m| m.point), Some([171.5, 51.5, 0.0]));
        assert_eq!((u.current_action, u.last_action), (MOVE_ACTION, 600));
        assert!(u.home.is_some_and(|h| !h.go_home));
        let mut u = boar(home(false, true));
        let mut fx = Vec::new();
        let r = u.action_failed(
            attack,
            &at([169.5, 50.6, 0.0]),
            false,
            &mut rng(),
            &mut Recorder::default(),
            &mut fx,
        );
        assert_eq!(r, Ok(GO_ON));
        assert_eq!((u.current_target, u.last_target), (-1, 31110));
        assert!(first_move(&fx).is_none());
    }

    #[test]
    fn an_animal_s_completed_wander_becomes_its_last_action() {
        // the fox 9851 in 718: its MoveTo's completion through the hunted animal's handler to
        // the base: no order, its action completed
        let mut u = UnitAi {
            current_action: MOVE_ACTION,
            idle_timer: 0,
            notifies: vec![Notify::own(9851, ACTION_COMPLETED, [MOVE_ACTION, 0, 0])],
            ..deer()
        };
        let mut r = wander_seed(false);
        let fx = u
            .update(
                60,
                0,
                false,
                &deer_at([31.5, 106.5, 0.0]),
                &mut r,
                &mut no_search,
            )
            .unwrap();
        assert_eq!(fx, vec![Effect::Stop]);
        assert_eq!((u.current_action, u.last_action), (-1, MOVE_ACTION));
    }

    #[test]
    fn a_movement_completion_reaches_only_a_moving_unit_ai() {
        let completed = Notify::own(9181, ACTION_COMPLETED, [MOVE_ACTION, 0, 0]);
        let mut builder = UnitAi {
            current_order: BUILD_ORDER,
            current_action: BUILD_ACTION,
            ..idle(8)
        };
        builder.post(completed, 2).unwrap();
        assert_eq!(builder.notifies, Vec::<Notify>::new());
        let mut wanderer = UnitAi {
            current_action: MOVE_ACTION,
            ..deer()
        };
        wanderer.post(completed, 2).unwrap();
        assert_eq!(wanderer.notifies, vec![completed]);
        // a unit not in state 2 tells its AI nothing
        let mut dying = UnitAi {
            current_action: MOVE_ACTION,
            ..deer()
        };
        dying.post(completed, 3).unwrap();
        assert_eq!(dying.notifies, Vec::<Notify>::new());
    }

    #[test]
    fn a_livestock_s_ownership_notify_drops_its_action_value_and_breaks_off() {
        // FUN_140c26ac0: the notify ends the pass; the update's FUN_140c5dc10 clears the queue
        let mut u = UnitAi {
            current_action: MOVE_ACTION,
            notifies: vec![
                Notify::own(9519, OWNER_CHANGED, [2, 0, 0]),
                Notify::own(9519, ACTION_COMPLETED, [MOVE_ACTION, 0, 0]),
            ],
            ..idle(19)
        };
        let mut r = rng();
        let fx = u
            .update(
                60,
                0,
                false,
                &Surroundings::default(),
                &mut r,
                &mut no_search,
            )
            .unwrap();
        assert_eq!(fx, vec![]);
        assert_eq!((u.current_action, u.last_action), (-1, MOVE_ACTION));
        assert_eq!((u.notifies.len(), u.last_update_time), (0, 60));
    }

    /// Slot 0x28: a unit past state 2 takes no order; one in front goes before the first queued
    /// order, any other after the last; an order not ready is what `FUN_140c4dfa0` looks for.
    #[test]
    fn an_order_for_the_queue_goes_after_the_last_or_before_the_first() {
        let order = |target, ready| QueuedOrder {
            target,
            order: ORDER_ON_TARGET,
            ready,
            ..QueuedOrder::default()
        };
        let mut ai = UnitAi::default();
        assert_eq!(ai.queue_order(3, order(1, true), false, false), Ok(false));
        assert_eq!(ai.orders, Vec::<QueuedOrder>::new());
        for (target, front) in [(1, true), (2, false), (3, true)] {
            assert_eq!(
                ai.queue_order(2, order(target, true), false, front),
                Ok(true)
            );
        }
        let targets: Vec<i32> = ai.orders.iter().map(|o| o.target).collect();
        assert_eq!(targets, [3, 1, 2]);
        assert!(!ai.has_unready_order());
        let _ = ai.queue_order(2, order(4, false), false, false).unwrap();
        assert!(ai.has_unready_order());
        assert!(ai.queue_order(2, order(5, true), true, false).is_err());
    }

    #[test]
    fn a_saved_order_and_notify_read_as_the_game_wrote_them() {
        let mut order = [0u8; 52];
        for (i, w) in [1i32, 710, 100, -1, -1].iter().enumerate() {
            order[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
        }
        for (i, f) in [91.822_914_f32, 52.489_582, 0.0, 0.0, 91.822_914, 52.489_582]
            .iter()
            .enumerate()
        {
            order[20 + 4 * i..24 + 4 * i].copy_from_slice(&f.to_le_bytes());
        }
        order[44] = 1;
        order[48..52].copy_from_slice(&(-1i32).to_le_bytes());
        let o = QueuedOrder::from_bytes(&order);
        assert_eq!(
            (
                o.issuer,
                o.order,
                o.priority,
                o.point,
                o.range,
                o.ready,
                o.target_type
            ),
            (1, 710, 100, [91.822_914, 52.489_582, 0.0], 0.0, true, -1)
        );
        let mut notify = [0u8; 24];
        for (i, w) in [9851i32, 9851, 506, 610, 0, 0].iter().enumerate() {
            notify[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
        }
        assert_eq!(
            Notify::from_bytes(&notify),
            Notify::own(9851, ACTION_COMPLETED, [MOVE_ACTION, 0, 0])
        );
    }

    /// MarvinBlue/426048731's builders 9175 and 9177 in update 1497: their Builds ended (no
    /// actions left) on their build order (702/602) with the house 10028 as their target.
    fn builder_done() -> UnitAi {
        UnitAi {
            current_order: BUILD_ORDER,
            current_action: BUILD_ACTION,
            current_target: 10028,
            target_type: 3,
            target_point: [30.0, 88.0, 0.0],
            idle_timeout: 3000,
            ..idle(8)
        }
    }

    #[test]
    fn a_making_building_runs_process_idle_only_without_a_target() {
        // a town centre (type 15, class 3) on its make action: with a target, or within its idle
        // timeout, the idle part does nothing and keeps the timer
        let tc = |target| UnitAi {
            ai_type: 15,
            current_order: ATTACK_ORDER,
            current_action: crate::spawn::MAKE_ACTION,
            current_target: target,
            idle_timer: 4000,
            adjusted_idle_timeout: 1080,
            ..UnitAi::default()
        };
        let at = Surroundings {
            class: CLASS_3,
            kind: BUILDING_KIND,
            ranged: true,
            busy: true,
            ..Surroundings::default()
        };
        let run = |u: &mut UnitAi, at: &Surroundings| {
            u.idle_part(
                true,
                at,
                false,
                &mut Well512::default(),
                &mut Recorder::default(),
                &mut Vec::new(),
            )
        };
        let mut u = tc(29826);
        run(&mut u, &at).unwrap();
        assert_eq!(u.idle_timer, 4000);
        let mut u = UnitAi {
            idle_timer: 900,
            ..tc(-1)
        };
        run(&mut u, &at).unwrap();
        assert_eq!(u.idle_timer, 900);
        // past it without a target the timer restarts and processIdle runs: the tower's search
        let mut u = tc(-1);
        assert_eq!(run(&mut u, &at), Err(Unsupported("no search here")));
        assert_eq!(u.idle_timer, 0);
        // a building without a missile and a single volley forgets its target, then searches
        let single = Surroundings {
            ranged: false,
            firepower: 1.0,
            ..at
        };
        let mut u = tc(29826);
        assert_eq!(run(&mut u, &single), Err(Unsupported("no search here")));
        assert_eq!((u.current_target, u.last_target), (-1, 29826));
        // with units inside it shoots more and keeps it
        let manned = Surroundings {
            firepower: 3.0,
            ..single
        };
        let mut u = tc(29826);
        run(&mut u, &manned).unwrap();
        assert_eq!((u.current_target, u.idle_timer), (29826, 4000));
    }

    #[test]
    fn a_building_s_attack_order_with_its_make_action_is_modelled() {
        // 6091, p6's town centre 27171: on the attack order against the boar 29826, the
        // villager it starts making takes the action value (619) while its Attack goes on
        let making = UnitAi {
            ai_type: 15,
            current_order: ATTACK_ORDER,
            current_action: crate::spawn::MAKE_ACTION,
            current_target: 29826,
            ..UnitAi::default()
        };
        assert!(making.check_busy_order(Retarget::Base, true).is_ok());
    }

    #[test]
    fn a_lion_s_own_attack_a_moved_building_and_a_worker_moved_off_a_footprint_are_modelled() {
        // a wolf's attack from its idle search (lion_idle): no order, the attack action, its
        // target; as the boar's
        let wolf = UnitAi {
            ai_type: 5,
            current_order: -1,
            current_action: ATTACK_ACTION,
            current_target: 1597,
            ..UnitAi::default()
        };
        assert!(wolf.check_busy_order(Retarget::Base, true).is_ok());
        // a building a MOVE took with a target (its unit AI's order 710), with no action value
        // or its own research's (620)
        for action in [-1, 620] {
            let building = UnitAi {
                ai_type: 17,
                current_order: MOVE_ORDER,
                current_action: action,
                current_target: 13764,
                ..UnitAi::default()
            };
            assert!(building.check_busy_order(Retarget::Base, false).is_ok());
        }
        // a builder and a hunter sent off a footprint (sent_off: the action value 0x2bf, the
        // order and the target kept)
        for order in [BUILD_ORDER, HUNT_ORDER] {
            let aside = UnitAi {
                current_order: order,
                current_action: MOVE_ASIDE_ACTION,
                ..builder_done()
            };
            assert!(aside.check_busy_order(Retarget::Civilian, true).is_ok());
        }
    }

    #[test]
    fn a_hunt_without_an_order_after_hunt_object_is_modelled() {
        // 5622, p5 31126: the villager's HuntObject after its stop ended order 713 leaves order
        // -1 with the hunt action and its target; the full update has no case for the order
        let hunting = UnitAi {
            current_order: -1,
            current_action: HUNT_ACTION,
            ..builder_done()
        };
        assert!(hunting.check_busy_order(Retarget::Never, true).is_ok());
        // another action without an order and with a target is not modelled
        let other = UnitAi {
            current_order: -1,
            current_action: 0x25b,
            ..builder_done()
        };
        assert!(other.check_busy_order(Retarget::Never, true).is_err());
    }

    #[test]
    fn the_target_block_keeps_a_standing_target_and_drops_a_gone_one() {
        // 1497: the house exists in state 2, which keeps it
        let standing = Surroundings {
            state: 2,
            target: Some(Seen {
                state: 2,
                kind: 80,
                ..Seen::default()
            }),
            ..Surroundings::default()
        };
        let mut u = builder_done();
        let mut fx = Vec::new();
        u.target_block(
            &standing,
            false,
            &mut Well512::default(),
            &mut Recorder::default(),
            &mut fx,
        )
        .unwrap();
        assert_eq!((u.current_target, &fx[..]), (10028, &[][..]));
        // a target no longer in the world goes (its class and point stay); an attack action's
        // idle timer then takes its timeout
        let gone = Surroundings {
            state: 2,
            ..Surroundings::default()
        };
        let mut u = UnitAi {
            current_action: ATTACK_ACTION,
            ..builder_done()
        };
        u.target_block(
            &gone,
            false,
            &mut Well512::default(),
            &mut Recorder::default(),
            &mut fx,
        )
        .unwrap();
        assert_eq!((u.current_target, u.target_type), (-1, 3));
        assert_eq!(u.idle_timer, 3000);
        // inside another, a unit drops its target before the lookup
        let inside = Surroundings {
            garrisoned: true,
            ..standing
        };
        let mut u = builder_done();
        u.target_block(
            &inside,
            false,
            &mut Well512::default(),
            &mut Recorder::default(),
            &mut fx,
        )
        .unwrap();
        assert_eq!(u.current_target, -1);
    }

    #[test]
    fn a_dead_target_stops_an_action_that_needs_a_live_one() {
        let dead = Surroundings {
            state: 2,
            target: Some(Seen {
                state: 3,
                kind: 70,
                ..Seen::default()
            }),
            ..Surroundings::default()
        };
        // 0x25b needs a live target: the target goes and the action stops
        let mut u = UnitAi {
            current_order: -1,
            current_action: 0x25b,
            ..builder_done()
        };
        let mut fx = Vec::new();
        u.target_block(
            &dead,
            false,
            &mut Well512::default(),
            &mut Recorder::default(),
            &mut fx,
        )
        .unwrap();
        assert_eq!(fx, vec![Effect::Stop]);
        assert_eq!((u.current_target, u.last_target), (-1, 10028));
        assert_eq!((u.current_action, u.last_action), (-1, 0x25b));
        // a hunt goes on at its carcass
        let mut u = UnitAi {
            current_order: HUNT_ORDER,
            current_action: HUNT_ACTION,
            ..builder_done()
        };
        let mut fx = Vec::new();
        u.target_block(
            &dead,
            false,
            &mut Well512::default(),
            &mut Recorder::default(),
            &mut fx,
        )
        .unwrap();
        assert_eq!((u.current_target, fx), (10028, vec![]));
        // an attack whose target died waits while its Attack swings on the kill (state 2); past
        // it the villager's processIdle (the cap, then 5) sends it to the dead target's point; an
        // Attack the slice does not hold is not modelled
        let attack = || UnitAi {
            current_action: ATTACK_ACTION,
            ..builder_done()
        };
        let swinging = Surroundings {
            action_state: Some(2),
            ..dead.clone()
        };
        let mut u = attack();
        let mut fx = Vec::new();
        u.target_block(
            &swinging,
            false,
            &mut Well512::default(),
            &mut Recorder::default(),
            &mut fx,
        )
        .unwrap();
        assert_eq!((u.current_target, fx), (10028, vec![]));
        let over = Surroundings {
            action_state: Some(1),
            ..dead.clone()
        };
        let mut u = attack();
        let mut fx = Vec::new();
        u.target_block(
            &over,
            false,
            &mut Well512::default(),
            &mut Recorder::default(),
            &mut fx,
        )
        .unwrap();
        assert_eq!(
            fx,
            vec![
                Effect::CapHumanOrder(HUMAN_ORDER_CAP),
                Effect::Move(AiMove {
                    point: [30.0, 88.0, 0.0],
                    range: 0.0,
                    force: true,
                    walk: false,
                })
            ]
        );
        assert_eq!((u.current_order, u.last_order), (-1, BUILD_ORDER));
        assert!(
            attack()
                .target_block(
                    &dead,
                    false,
                    &mut Well512::default(),
                    &mut Recorder::default(),
                    &mut Vec::new()
                )
                .is_err()
        );
    }

    /// Player 1's scout 9183 following the deer 9655 (FOLLOW at 2604): order 712, action 612,
    /// the target and its place when FOLLOW ran, the constructor's desired distance.
    fn follower(deer: [f32; 3], visible: bool) -> (UnitAi, Surroundings) {
        let u = UnitAi {
            ai_type: 13,
            current_order: FOLLOW_ORDER,
            current_action: FOLLOW_ACTION,
            current_target: 9655,
            target_type: 9,
            target_point: [115.6998, 39.299_213, 0.0],
            desired_distance: 2.0,
            stance: 2,
            ..UnitAi::default()
        };
        let at = Surroundings {
            position: [110.0, 40.0, 0.0],
            size: [0.2, 0.2],
            state: 2,
            busy: true,
            class: 12,
            target: Some(Seen {
                state: 2,
                kind: 70,
                class: 9,
                position: deer,
                radius: [0.2, 0.2],
                visible,
                ..Seen::default()
            }),
            ..Surroundings::default()
        };
        (u, at)
    }

    #[test]
    fn the_follow_order_moves_to_a_target_that_left_its_point() {
        // 2652: the deer is more than a tile off the point in y; beyond 2 the AI moves to it
        let deer = [114.75516, 40.34795, 0.0];
        let (mut u, at) = follower(deer, true);
        let mut fx = Vec::new();
        assert!(u.follow(&at, &mut fx).unwrap());
        assert_eq!(
            fx,
            vec![Effect::MoveToObject {
                target: 9655,
                range: 2.0
            }]
        );
        assert_eq!(
            (u.current_action, u.last_action),
            (MOVE_ACTION, FOLLOW_ACTION)
        );
        assert_eq!(
            (u.current_target, u.last_target, u.target_type),
            (9655, 9655, 9)
        );
        assert_eq!(u.target_point, deer);
        // less than a tile off on both axes it stays
        let (mut u, at) = follower([115.0, 40.2, 0.0], true);
        let mut fx = Vec::new();
        assert!(!u.follow(&at, &mut fx).unwrap());
        assert_eq!(fx, Vec::<Effect>::new());
        assert_eq!(u.target_point, [115.6998, 39.299_213, 0.0]);
    }

    #[test]
    fn the_follow_order_ends_on_a_target_out_of_sight() {
        let (mut u, at) = follower([114.75516, 40.34795, 0.0], false);
        let mut fx = Vec::new();
        assert!(u.follow(&at, &mut fx).unwrap());
        assert_eq!(fx, vec![Effect::Stop]);
        assert_eq!(
            (u.current_order, u.current_action, u.current_target),
            (-1, -1, -1)
        );
        assert_eq!((u.last_target, u.last_action), (9655, FOLLOW_ACTION));
    }

    #[test]
    fn a_moved_target_notify_is_ignored_without_a_patrol_path() {
        let (mut u, at) = follower([115.0, 40.2, 0.0], true);
        let n = Notify::own(9183, TARGET_MOVED, [0, 0, 0]);
        let (mut r, mut side) = (rng(), Recorder::default());
        assert_eq!(
            u.base_notify(n, 0, &at, false, &mut r, &mut side, &mut Vec::new()),
            Ok(GO_ON)
        );
        u.patrol_path = true;
        assert!(
            u.base_notify(n, 0, &at, false, &mut r, &mut side, &mut Vec::new())
                .is_err()
        );
    }

    /// The scout 9185 in update 1518: idle (its MOVE done), stance 2 at its state position.
    fn idle_scout() -> (UnitAi, Surroundings) {
        let u = UnitAi {
            idle_timer: 504,
            adjusted_idle_timeout: 503,
            stance: 2,
            state_position: [21.947_916, 92.489_586],
            ..idle(13)
        };
        let at = Surroundings {
            position: [21.947_916, 92.489_586, 0.0],
            player: 2,
            class: 12,
            size: [0.2, 0.2],
            los: 4.0,
            search_radius: 4.0,
            state: 2,
            kind: 70,
            master: 448,
            ..Surroundings::default()
        };
        (u, at)
    }

    #[test]
    fn an_idle_soldier_caps_its_human_order_time_and_searches_for_something_to_attack() {
        let (mut u, at) = idle_scout();
        let mut searched = Vec::new();
        let mut closest = |s: &Search<'_>| {
            searched.push((s.radius, s.classes.len(), s.pick));
            Ok(Found::NONE)
        };
        let fx = u
            .update(28, 0, false, &at, &mut rng(), &mut closest)
            .unwrap();
        assert_eq!(fx, vec![Effect::CapHumanOrder(5000)]);
        // the search radius: its line of sight plus 1; Gaia's objects too (not a scenario)
        assert!(matches!(
            searched[..],
            [(5.0, 39, Pick::Attack { lowest: 0, .. })]
        ));
        assert_eq!(u.idle_timer, 0);
        // something found would be attacked, which is not modelled
        let (mut u, at) = idle_scout();
        let mut found = |_: &Search<'_>| {
            Ok(Found {
                id: 9731,
                enemies: false,
                position: Some([0.0; 3]),
                fresh: true,
            })
        };
        assert!(u.update(28, 0, false, &at, &mut rng(), &mut found).is_err());
        // away from its state position, a stance-2 soldier walks back ("I'm in Defend or
        // StandGround, returning to state pos.", FUN_140c5e7d0), which its processIdle does not
        // count as an action (5)
        let (mut u, at) = idle_scout();
        u.state_position = [25.0, 92.5];
        let mut none = |_: &Search<'_>| Ok(Found::NONE);
        let fx = u.update(28, 0, false, &at, &mut rng(), &mut none).unwrap();
        assert_eq!(
            fx,
            vec![
                Effect::CapHumanOrder(5000),
                Effect::Move(AiMove {
                    point: [25.0, 92.5, 0.0],
                    range: 1.0,
                    force: true,
                    walk: false,
                })
            ]
        );
        assert_eq!((u.last_action, u.current_action), (-1, MOVE_ACTION));
    }

    #[test]
    fn a_gaia_soldier_that_takes_owners_by_sight_does_not_search() {
        let (mut u, at) = idle_scout();
        let gaia = Surroundings {
            player: 0,
            capture: 1,
            ..at
        };
        let fx = u
            .update(28, 0, false, &gaia, &mut rng(), &mut no_search)
            .unwrap();
        assert_eq!(fx, vec![Effect::CapHumanOrder(5000)]);
    }

    /// Player 2's town centre 9156 in MarvinBlue/426048731's update 1506.
    const TC: Attacker = Attacker {
        ai_type: 15,
        player: 2,
        computer: false,
        state: 2,
        held: -1,
        top_action: None,
        master: 109,
        kind: 80,
        class: 3,
        attack_priority: 0,
        missile: 54,
        range: 6.0,
        min_range: 0.0,
        charge_type: 0,
        area_effect: 0.0,
        blast_level: 0,
        los: 8.0,
        position: [29.0, 94.0, 0.0],
        radius: [2.0, 2.0],
        speed: 0.0,
        scenario: false,
        firepower: 0.0,
    };

    /// Gaia's boar 9731 (master 48, class 10).
    const BOAR: Target = Target {
        owner: 0,
        class: 10,
        kind: 70,
        master: 48,
        combat_level: 0,
        capture: 0,
        has_ai: true,
        inside: false,
        targeting: true,
        attackable: true,
        desolid: false,
        position: [30.0, 98.0, 0.0],
        radius: [0.5, 0.5],
    };

    /// A unit AI without an order or a target, in the aggressive stance.
    const IDLE: Seeker = Seeker {
        stance: 0,
        state_position: [0.0, 0.0],
        order: -1,
        action: -1,
        target: -1,
        target_type: -1,
        last_target_type: -1,
        patrol_path: false,
    };

    #[test]
    fn a_tower_passes_a_boar_but_not_a_wolf() {
        assert_eq!(can_attack(&TC, &IDLE, &BOAR), Ok(false));
        let wolf = Target {
            combat_level: 4,
            ..BOAR
        };
        assert_eq!(can_attack(&TC, &IDLE, &wolf), Ok(true));
        // Gaia's tower attacks nothing; a Gaia object outside classes 9 and 10 that takes owners
        // by sight neither
        assert_eq!(
            can_attack(&Attacker { player: 0, ..TC }, &IDLE, &wolf),
            Ok(false)
        );
        let sheep = Target {
            class: 58,
            capture: 3,
            ..BOAR
        };
        assert_eq!(can_attack(&TC, &IDLE, &sheep), Ok(false));
        // a type whose attack check is not modelled
        assert!(can_attack(&Attacker { ai_type: 5, ..TC }, &IDLE, &BOAR).is_err());
        // a tower without a missile is armed only by the firepower of the units inside it
        let bare = Attacker { missile: -1, ..TC };
        let enemy = Target {
            owner: 1,
            class: 6,
            ..BOAR
        };
        assert_eq!(can_attack(&bare, &IDLE, &enemy), Ok(false));
        let manned = Attacker {
            firepower: 2.0,
            ..bare
        };
        assert_eq!(can_attack(&manned, &IDLE, &enemy), Ok(true));
    }

    /// A scout cavalry (master 448, class 47) of player 1 in `StandGround` at its state position,
    /// as records/425906646's 27272 at update 4268.
    const SCOUT: Attacker = Attacker {
        ai_type: 13,
        player: 1,
        kind: 70,
        class: 47,
        master: 448,
        missile: -1,
        range: 0.0,
        los: 4.0,
        position: [118.5, 183.5, 0.0],
        radius: [0.4, 0.4],
        speed: 1.2,
        ..TC
    };
    const STAND_GROUND: Seeker = Seeker {
        stance: 2,
        state_position: [118.5, 183.5],
        ..IDLE
    };

    #[test]
    fn a_soldier_standing_its_ground_attacks_within_its_range_of_its_state_position() {
        let villager = |x: f32| Target {
            owner: 2,
            class: 4,
            master: 83,
            position: [x, 183.5, 0.0],
            radius: [0.2, 0.2],
            ..BOAR
        };
        // the range against it plus 0.5 from the state position to its footprint
        let reach = |s: &Seeker, x: f32| soldier_can_attack(&SCOUT, s, &villager(x));
        assert_eq!(reach(&STAND_GROUND, 119.2), Ok(true));
        assert_eq!(reach(&STAND_GROUND, 119.3), Ok(false));
        // on order 0x2bd anywhere; in Defend 5 more
        let ordered = Seeker {
            order: 0x2bd,
            ..STAND_GROUND
        };
        assert_eq!(reach(&ordered, 130.0), Ok(true));
        let defend = Seeker {
            stance: 1,
            ..STAND_GROUND
        };
        assert_eq!(reach(&defend, 124.2), Ok(true));
        assert_eq!(reach(&defend, 124.3), Ok(false));
        // never in NoAttack, never an object its targeting flag leaves out, never class 1
        let no_attack = Seeker {
            stance: 3,
            ..STAND_GROUND
        };
        assert_eq!(reach(&no_attack, 119.0), Ok(false));
        let untargeted = Target {
            targeting: false,
            ..villager(119.0)
        };
        assert_eq!(
            soldier_can_attack(&SCOUT, &STAND_GROUND, &untargeted),
            Ok(false)
        );
        let relic = Target {
            class: 1,
            ..villager(119.0)
        };
        assert_eq!(soldier_can_attack(&SCOUT, &STAND_GROUND, &relic), Ok(false));
    }

    #[test]
    fn best_unit_to_attack_takes_an_enemy_within_twice_the_line_of_sight() {
        let aggressive = Seeker {
            stance: 0,
            ..STAND_GROUND
        };
        let enemy = |x: f32| Target {
            owner: 2,
            class: 6,
            master: 74,
            position: [x, 183.5, 0.0],
            radius: [0.2, 0.2],
            ..BOAR
        };
        // the footprints' gap: 0.4 + 0.2 off the centres' distance
        let gap = attack_candidate(&SCOUT, &aggressive, &enemy(126.0), Some(4)).unwrap();
        assert!(gap.is_some_and(|g| (g - 6.9).abs() < 1e-4));
        assert_eq!(
            attack_candidate(&SCOUT, &aggressive, &enemy(127.2), Some(4)),
            Ok(None)
        );
        // one without a unit AI or inside another is none
        let no_ai = Target {
            has_ai: false,
            ..enemy(120.0)
        };
        assert_eq!(
            attack_candidate(&SCOUT, &aggressive, &no_ai, Some(4)),
            Ok(None)
        );
        let inside = Target {
            inside: true,
            ..enemy(120.0)
        };
        assert_eq!(
            attack_candidate(&SCOUT, &aggressive, &inside, Some(4)),
            Ok(None)
        );
        // a wall counts only for a unit that attacks walls
        let wall = Target {
            class: 27,
            ..enemy(120.0)
        };
        assert_eq!(
            attack_candidate(&SCOUT, &aggressive, &wall, Some(4)),
            Ok(None)
        );
        let at_walls = Seeker {
            last_target_type: 27,
            ..aggressive
        };
        assert!(attack_candidate(&SCOUT, &at_walls, &wall, Some(4)).is_err());
    }

    #[test]
    fn an_idle_tower_searches_with_its_classes() {
        let mut u = UnitAi {
            idle_timer: 504,
            adjusted_idle_timeout: 503,
            ..idle(15)
        };
        let at = Surroundings {
            position: [29.0, 94.0, 0.0],
            player: 2,
            size: [2.0, 2.0],
            los: 8.0,
            search_radius: 0.0,
            state: 2,
            kind: 80,
            ..Surroundings::default()
        };
        let mut searched = Vec::new();
        let mut closest = |s: &Search<'_>| {
            searched.push((s.radius, s.classes.to_vec()));
            Ok(Found::NONE)
        };
        let fx = u
            .update(28, 0, false, &at, &mut rng(), &mut closest)
            .unwrap();
        assert_eq!(fx, vec![Effect::CapHumanOrder(5000)]);
        assert_eq!(searched, vec![(9.0, TOWER_CARED.to_vec())]);
    }

    /// `FUN_140c4dcb0`: both targets and their classes go and the point is −1 on all three axes;
    /// the order and action stay. MOVE's notify to a unit that took its `MoveTo` forgets the same
    /// way before it takes the order's target.
    #[test]
    fn forgetting_the_targets_clears_the_current_and_the_last_and_the_point() {
        let mut u = UnitAi {
            current_order: ATTACK_ORDER,
            current_action: ATTACK_ACTION,
            current_target: 12,
            target_type: 4,
            target_point: [3.0, 4.0, 0.5],
            last_target: 11,
            last_target_type: 9,
            ..idle(13)
        };
        u.forget_targets();
        assert_eq!(
            (
                u.current_target,
                u.target_type,
                u.last_target,
                u.last_target_type
            ),
            (-1, -1, -1, -1)
        );
        assert_eq!(u.target_point, [-1.0; 3]);
        assert_eq!(
            (u.current_order, u.current_action),
            (ATTACK_ORDER, ATTACK_ACTION)
        );
        let mut moved = UnitAi {
            current_target: 12,
            target_type: 4,
            last_target: 11,
            last_target_type: 9,
            ..idle(13)
        };
        moved.notify_move(true, None, [5.0, 6.0], &mut rng());
        // the forget, then FUN_140c64be0 with no target: the forgotten -1 becomes the last
        assert_eq!(
            (
                moved.last_target,
                moved.last_target_type,
                moved.current_target
            ),
            (-1, -1, -1)
        );
        assert_eq!(moved.target_point, [5.0, 6.0, 0.0]);
    }

    /// `FUN_140c4eea0`: a master with a task of action type 0x99 refuses orders while its target
    /// exists or it has an action value; with the second argument set a unit that passes takes
    /// orders only with that task, and without it any unit that passes does.
    #[test]
    fn a_unit_takes_orders_by_its_0x99_task_and_with_task_only_needs_it() {
        let mut u = idle(13);
        let plain = Surroundings::default();
        let carter = Surroundings {
            task_0x99: true,
            ..Surroundings::default()
        };
        assert!(u.can_order(&plain, false));
        assert!(!u.can_order(&plain, true));
        assert!(u.can_order(&carter, false));
        assert!(u.can_order(&carter, true));
        u.current_action = GATHER_ACTION;
        assert!(!u.can_order(&carter, false));
        assert!(!u.can_order(&carter, true));
        assert!(u.can_order(&plain, false));
        u.current_action = -1;
        let targeted = Surroundings {
            target: Some(Seen::default()),
            ..carter
        };
        assert!(!u.can_order(&targeted, true));
    }

    /// `FUN_140c1a760` at `LAB_140c1aa59`: an enemy building (master type `'P'`) passes the
    /// tower's check once a builder started it; a foundation still desolid (`+0xc60`) does not.
    /// The flag counts only for a building: a unit that holds it passes.
    #[test]
    fn a_tower_attacks_a_started_building_but_not_a_desolid_foundation() {
        let house = Target {
            owner: 1,
            class: 3,
            kind: 80,
            master: 70,
            ..BOAR
        };
        assert_eq!(tower_can_attack(&TC, &house), Ok(true));
        let foundation = Target {
            desolid: true,
            ..house
        };
        assert_eq!(tower_can_attack(&TC, &foundation), Ok(false));
        let unit = Target {
            owner: 1,
            class: 6,
            desolid: true,
            ..BOAR
        };
        assert_eq!(tower_can_attack(&TC, &unit), Ok(true));
    }

    /// `FUN_140b43f30` on the attackers (`+0xc0`): an id joins the set in its sorted place and
    /// the call answers 1; an id already in it changes nothing and answers 0.
    #[test]
    fn an_attacker_joins_the_sorted_set_once() {
        let mut u = idle(4);
        assert!(u.add_attacker(30));
        assert!(u.add_attacker(10));
        assert!(u.add_attacker(20));
        assert!(!u.add_attacker(10));
        assert!(u.add_attacker(-1));
        assert_eq!(u.attackers, vec![-1, 10, 20, 30]);
    }

    /// The soldier's slot 0xa8 (`FUN_140c1ae60`): a neutral object of a class in the mask
    /// `0x200200800838001c` is passed over; at attack priority 3 only [`PRIORITY_3_CLASSES`]
    /// pass; any other class passes, also one outside the mask's 62 bits.
    #[test]
    fn the_soldier_passes_over_neutrals_of_its_masked_classes() {
        for class in [2, 3, 4, 19, 20, 21, 27, 39, 49, 61] {
            assert!(!soldier_attacks_neutral(0, class), "class {class}");
        }
        for class in [0, 6, 9, 15, 35, 52, 53, 60, 62, 63, -1] {
            assert!(soldier_attacks_neutral(0, class), "class {class}");
        }
        assert!(soldier_attacks_neutral(3, 52));
        assert!(!soldier_attacks_neutral(3, 6));
        // a masked class stays passed over whatever the priority
        assert!(!soldier_attacks_neutral(3, 27));
    }

    /// `FUN_140c42640`: an order goes after the last unless it goes in front of a queue that
    /// holds one.
    #[test]
    fn an_inserted_order_goes_in_front_only_of_a_queue_that_holds_one() {
        let order = |target| QueuedOrder {
            target,
            order: MOVE_ORDER,
            ready: true,
            ..QueuedOrder::default()
        };
        let mut u = idle(13);
        u.insert_order(order(1), true);
        u.insert_order(order(2), false);
        u.insert_order(order(3), true);
        u.insert_order(order(4), false);
        let targets: Vec<i32> = u.orders.iter().map(|o| o.target).collect();
        assert_eq!(targets, [3, 1, 2, 4]);
    }

    /// `FUN_140c55c90` immediate and in front: a builder (702/602 on foundation 50) told to
    /// move at once keeps its build order: the build goes in front of the queue with the move's
    /// shown point and ready flag (issuer the unit, owner −1, its range `+0x5c`), the move in
    /// front of it; the target goes, the action stops and ends the build order (602 + 100), the
    /// move runs (slot 0x230) and leaves the queue; the build waits behind.
    #[test]
    fn an_immediate_order_in_front_keeps_the_current_order_behind_it() {
        let mut u = UnitAi {
            current_order: BUILD_ORDER,
            order_priority: 2,
            current_action: BUILD_ACTION,
            current_target: 50,
            target_type: 3,
            target_point: [20.0, 21.0, 0.0],
            desired_distance: 2.0,
            ..idle(8)
        };
        let order = QueuedOrder {
            issuer: 1,
            order: MOVE_ORDER,
            priority: 7,
            target: -1,
            target_owner: -1,
            point: [10.0, 11.0, 0.0],
            range: 0.5,
            shown: [10.0, 11.0],
            ready: true,
            target_type: -1,
        };
        let mut fx = Vec::new();
        u.order_now(order, Some(9), Some(([5.0, 6.0], 4)), None, &mut fx)
            .unwrap();
        let kept = QueuedOrder {
            issuer: 9,
            order: BUILD_ORDER,
            priority: 2,
            target: 50,
            target_owner: -1,
            point: [20.0, 21.0, 0.0],
            range: 2.0,
            shown: [10.0, 11.0],
            ready: true,
            target_type: 3,
        };
        assert_eq!(u.orders, vec![kept]);
        assert_eq!(
            fx,
            vec![
                Effect::Stop,
                Effect::Move(AiMove {
                    point: [10.0, 11.0, 0.0],
                    range: 0.5,
                    force: true,
                    walk: true,
                })
            ]
        );
        assert_eq!((u.current_order, u.order_priority), (MOVE_ORDER, 7));
        // the stop ended the build order; the move's FUN_140c64c10 made the forgotten -1 the
        // last target
        assert_eq!((u.last_order, u.last_target), (BUILD_ORDER, -1));
        assert_eq!(u.state_position, [5.0, 6.0]);
    }

    /// `FUN_140c55c90` immediate for an attack roundup (0x2d9): "We have an `AttackRoundup`, so
    /// I'm not stopping the object.": the target stays and nothing stops; the notifies still go
    /// and slot 0x230 runs the order in front, whose roundup case (slot 0xc0) is not modelled.
    #[test]
    fn an_immediate_attack_roundup_keeps_the_target_and_the_action() {
        let mut u = UnitAi {
            current_order: ATTACK_ORDER,
            current_action: ATTACK_ACTION,
            current_target: 50,
            notifies: vec![Notify::own(9, DAMAGED, [1, 5, 0])],
            ..idle(13)
        };
        let roundup = QueuedOrder {
            order: crate::ai_kinds::ORDER_ATTACK_ROUNDUP,
            target: 51,
            ready: true,
            ..QueuedOrder::default()
        };
        let mut fx = Vec::new();
        let r = u.take_order_now(2, roundup, ([0.0; 2], 6), &mut fx);
        assert_eq!(
            r,
            Err(Unsupported(
                "a queued order other than a move or on a target"
            ))
        );
        assert_eq!(fx, Vec::new());
        assert_eq!((u.current_target, u.current_action), (50, ATTACK_ACTION));
        assert_eq!(u.notifies, Vec::new());
        assert_eq!(u.orders, vec![roundup]);
    }

    /// `FUN_140c5e7d0`: a unit more than a tile (the squared footprint gap, `FUN_140a847c0`)
    /// from its state position moves there at its height within 1.0, forced and running (the
    /// unit's slot 0x340); the move action value becomes current, the old one the last, and the
    /// target stays. Within a tile nothing happens. A gap that is not a number walks too
    /// (`COMISS 1.0, gap; JNC`).
    #[test]
    fn a_unit_off_its_state_position_walks_back_and_keeps_its_target() {
        let mut u = UnitAi {
            current_action: ATTACK_ACTION,
            current_target: 77,
            state_position: [10.0, 10.0],
            ..idle(13)
        };
        let at = |x: f32| Surroundings {
            position: [x, 10.0, 2.5],
            size: [0.5, 0.5],
            ..Surroundings::default()
        };
        let mut fx = Vec::new();
        // 1.4 off, 0.9 off its footprint: within a tile
        assert!(!u.walk_back(&at(11.4), &mut fx));
        assert_eq!((fx.len(), u.current_action), (0, ATTACK_ACTION));
        assert!(u.walk_back(&at(11.6), &mut fx));
        assert_eq!(
            fx,
            vec![Effect::Move(AiMove {
                point: [10.0, 10.0, 2.5],
                range: 1.0,
                force: true,
                walk: false,
            })]
        );
        assert_eq!(
            (u.last_action, u.current_action),
            (ATTACK_ACTION, MOVE_ACTION)
        );
        assert_eq!(u.current_target, 77);
        assert!(u.commanded);
        let mut lost = UnitAi {
            state_position: [f32::NAN, 10.0],
            ..idle(13)
        };
        assert!(lost.walk_back(&at(10.0), &mut Vec::new()));
    }

    /// `FUN_140a847c0` clamps each axis by `MAXSS 0, gap`: a gap that is not a number stays one.
    #[test]
    fn the_point_gap_keeps_a_gap_that_is_not_a_number() {
        assert_eq!(
            point_gap_squared([3.0, 0.0], [0.0, 0.0, 0.0], [1.0, 1.0]),
            4.0
        );
        assert!(point_gap_squared([f32::NAN, 0.0], [0.0, 0.0, 0.0], [1.0, 1.0]).is_nan());
    }

    /// `FUN_140c20db0` in `NoAttack` ("searching and returning to state pos."): the search alone
    /// (slot 0x50 with mode 1), then off the patrol orders the walk back; processIdle stays 5.
    #[test]
    fn a_no_attack_soldier_searches_then_walks_back_to_its_state_position() {
        let (mut u, at) = idle_scout();
        u.stance = 3;
        u.state_position = [25.0, 92.5];
        let mut picks = Vec::new();
        let mut look = |s: &Search<'_>| {
            picks.push(s.pick);
            Ok(Found::NONE)
        };
        let fx = u.update(28, 0, false, &at, &mut rng(), &mut look).unwrap();
        assert_eq!(picks, vec![Pick::Look]);
        assert_eq!(
            fx,
            vec![
                Effect::CapHumanOrder(5000),
                Effect::Move(AiMove {
                    point: [25.0, 92.5, 0.0],
                    range: 1.0,
                    force: true,
                    walk: false,
                })
            ]
        );
        // on a patrol order it stays
        let (mut u, at) = idle_scout();
        u.stance = 3;
        u.state_position = [25.0, 92.5];
        u.current_order = PATROL;
        let mut fx = Vec::new();
        u.no_attack_idle(&at, false, &mut |_: &Search<'_>| Ok(Found::NONE), &mut fx)
            .unwrap();
        assert_eq!(fx, Vec::new());
    }

    /// The full update (`0x140c66b10`) of a unit in a group the world holds: its state position
    /// follows it while its group has a command (`+0x3a8`); its idle check (slot 0x240) waits
    /// while its group is not idle (`FUN_141017ad0`) or the unit moves (slot 0x5c8); its order
    /// queue waits while the group is not idle; a group that patrols takes the patrol's idle
    /// part, and its target's range test (`FUN_140c41690`) answers 0 for a human player; action
    /// `0x2c0` (moved aside) ends, the last action keeping it, unless the group moves.
    #[test]
    fn a_grouped_unit_s_full_update_reads_its_group() {
        let group = |idle, moving, member_moving| {
            Some(GroupState {
                commanded: true,
                idle,
                patrol: false,
                moving,
                member_moving,
            })
        };
        let ship = || UnitAi {
            idle_timer: 3001,
            adjusted_idle_timeout: 3000,
            ..idle(11)
        };
        let at = |group| Surroundings {
            position: [3.0, 4.0, 0.0],
            player: 1,
            class: 20,
            group,
            ..Surroundings::default()
        };
        let run = |u: &mut UnitAi, at: &Surroundings| {
            u.update(28, 0, false, at, &mut rng(), &mut no_search)
        };
        // an idle group of a still unit: the idle check runs, the state position follows
        let mut u = ship();
        let fx = run(&mut u, &at(group(true, false, false))).unwrap();
        assert_eq!(fx, vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)]);
        assert_eq!(u.state_position, [3.0, 4.0]);
        // a group not idle, or a moving unit: no idle check
        for g in [group(false, true, false), group(true, false, true)] {
            let mut u = ship();
            assert_eq!(run(&mut u, &at(g)), Ok(vec![]));
            assert_eq!(u.idle_timer, 3029);
        }
        // a group without a command: the state position stays
        let mut u = ship();
        let mut g = group(true, false, false);
        g.as_mut().unwrap().commanded = false;
        let _ = run(&mut u, &at(g)).unwrap();
        assert_eq!(u.state_position, [0.0; 2]);
        // the order queue waits for a group that is not idle
        let order = QueuedOrder {
            issuer: 1,
            order: MOVE_ORDER,
            priority: 100,
            target: -1,
            target_owner: -1,
            point: [9.0, 9.0, 0.0],
            range: 0.0,
            shown: [9.0, 9.0],
            ready: true,
            target_type: -1,
        };
        let mut u = UnitAi {
            orders: vec![order],
            ..idle(11)
        };
        let _ = run(&mut u, &at(group(false, true, false))).unwrap();
        assert_eq!(u.orders.len(), 1);
        let _ = run(&mut u, &at(group(true, false, false))).unwrap();
        assert_eq!(u.orders.len(), 0);
        // a patrolling group
        let mut u = idle(11);
        let mut g = group(false, true, false);
        g.as_mut().unwrap().patrol = true;
        assert_eq!(
            run(&mut u, &at(g)),
            Err(Unsupported("a patrol order's idle part"))
        );
        // a target of a unit in a patrolling group: FUN_140c41690 answers 0 for a human player
        let mut g = group(true, false, false);
        g.as_mut().unwrap().patrol = true;
        let seen = Seen {
            state: 2,
            kind: 70,
            owner: 2,
            class: 6,
            position: [5.0, 5.0, 0.0],
            radius: [0.2, 0.2],
            visible: true,
            flags: 0,
        };
        let target_at = Surroundings {
            target: Some(seen),
            state: 2,
            ..at(g)
        };
        let mut u = UnitAi {
            current_target: 5,
            ..idle(11)
        };
        let mut fx = Vec::new();
        assert_eq!(
            u.target_block(&target_at, false, &mut rng(), &mut no_search, &mut fx),
            Ok(())
        );
        assert_eq!(
            u.target_block(&target_at, true, &mut rng(), &mut no_search, &mut fx),
            Err(Unsupported("the patrol range of a unit AI's target"))
        );
        // action 0x2c0 ends unless the group moves
        for (g, after) in [
            (None, -1),
            (group(true, false, false), -1),
            (group(false, true, false), GROUP_ASIDE_ACTION),
        ] {
            let mut u = UnitAi {
                current_action: GROUP_ASIDE_ACTION,
                last_action: 7,
                ..idle(11)
            };
            let _ = run(&mut u, &at(g)).unwrap();
            assert_eq!(u.current_action, after);
            let last = if after == -1 { GROUP_ASIDE_ACTION } else { 7 };
            assert_eq!(u.last_action, last);
        }
    }

    /// The transport ship's processIdle (`0x140c21d20`): `FUN_140c41e50`, then 5. An idle ship
    /// whose idle timer passed its timeout caps its last human order time and does nothing more.
    #[test]
    fn an_idle_transport_ship_s_check_caps_its_human_order_time_and_does_nothing() {
        let mut u = UnitAi {
            idle_timer: 3001,
            adjusted_idle_timeout: 3000,
            ..idle(11)
        };
        let at = Surroundings {
            player: 1,
            class: 20,
            ..Surroundings::default()
        };
        let fx = u
            .update(28, 0, false, &at, &mut rng(), &mut no_search)
            .unwrap();
        assert_eq!(fx, vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)]);
        assert_eq!(u.idle_timer, 0);
        // the building's (FUN_140c1e600) and the livestock's (FUN_140c1fea0) return 5 alone
        for ai_type in [17, 19] {
            let mut u = idle(ai_type);
            let mut fx = Vec::new();
            assert_eq!(
                u.process_idle(&at, false, &mut rng(), &mut no_search, &mut fx),
                Ok(false)
            );
            assert_eq!(fx, Vec::new());
        }
    }

    /// The target block's attack whose target died, for a villager: the civilian's processIdle
    /// (`FUN_140c1e610`: the cap, then 5) produces nothing, so the AI moves to the dead target's
    /// point within its weapon range and the order ends.
    #[test]
    fn a_villager_whose_attack_target_died_walks_to_its_point() {
        let mut u = UnitAi {
            current_order: ATTACK_ORDER,
            order_priority: 100,
            current_action: ATTACK_ACTION,
            current_target: 50,
            target_point: [5.0, 6.0, 0.0],
            ..idle(8)
        };
        let at = Surroundings {
            weapon_range: 0.25,
            ..Surroundings::default()
        };
        let mut fx = Vec::new();
        u.next_after_kill(&at, false, &mut rng(), &mut no_search, &mut fx)
            .unwrap();
        assert_eq!(
            fx,
            vec![
                Effect::CapHumanOrder(HUMAN_ORDER_CAP),
                Effect::Move(AiMove {
                    point: [5.0, 6.0, 0.0],
                    range: 0.25,
                    force: true,
                    walk: false,
                })
            ]
        );
        assert_eq!(
            (u.current_order, u.order_priority, u.last_order),
            (-1, -1, ATTACK_ORDER)
        );
    }

    /// A wolf whose attack target died: its processIdle (`FUN_140c1fb70`) finds another villager
    /// within its reach and calls `AttackObject` unforced, which fails while the attack action
    /// value is still current; the 6 it gives all the same keeps it from walking to the dead
    /// one's point.
    #[test]
    fn a_wolf_whose_attack_target_died_attacks_the_next_villager() {
        let mut u = UnitAi {
            current_action: ATTACK_ACTION,
            current_target: 50,
            target_point: [5.0, 6.0, 0.0],
            ..idle(5)
        };
        let at = Surroundings {
            los: 4.0,
            search_radius: 1.0,
            ..Surroundings::default()
        };
        let mut side = LionSide {
            found: [700, -1],
            searches: 0,
            seen: UnitSeen {
                id: 700,
                class: 4,
                owner: 1,
                state: 2,
                gap_squared: 1.0,
                ..UnitSeen::default()
            },
            attacks: Vec::new(),
        };
        let mut fx = Vec::new();
        u.next_after_kill(&at, false, &mut rng(), &mut side, &mut fx)
            .unwrap();
        assert_eq!(fx, vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)]);
        assert_eq!(side.attacks, Vec::new());
        assert_eq!((u.current_action, u.current_target), (ATTACK_ACTION, 50));
    }

    /// The tower in `NoAttack` (stance 3, `FUN_140c564c0`): after the cap only the search (slot
    /// 0x50 with mode 1, its cared classes), whose alert mode it sets; then 5.
    #[test]
    fn a_no_attack_tower_only_searches() {
        let mut u = UnitAi {
            idle_timer: 504,
            adjusted_idle_timeout: 503,
            stance: 3,
            ..idle(15)
        };
        let at = Surroundings {
            position: [29.0, 94.0, 0.0],
            player: 2,
            size: [2.0, 2.0],
            los: 8.0,
            state: 2,
            kind: 80,
            ..Surroundings::default()
        };
        let mut searched = Vec::new();
        let mut closest = |s: &Search<'_>| {
            searched.push((s.radius, s.classes.len(), s.pick));
            Ok(Found {
                id: -1,
                enemies: true,
                position: None,
                fresh: true,
            })
        };
        let fx = u
            .update(28, 0, false, &at, &mut rng(), &mut closest)
            .unwrap();
        assert_eq!(fx, vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)]);
        assert_eq!(searched, vec![(9.0, TOWER_CARED.len(), Pick::Look)]);
        assert!(u.alert_mode);
    }

    /// The tower's processIdle in the idle part (`FUN_140c564c0` with 1): what
    /// `bestUnitToAttack` found it attacks (slot 0xb8 forced); the idle part reads no result.
    #[test]
    fn an_idle_tower_attacks_what_its_search_found() {
        let mut u = UnitAi {
            idle_timer: 504,
            adjusted_idle_timeout: 503,
            ..idle(15)
        };
        let at = Surroundings {
            position: [29.0, 94.0, 0.0],
            player: 2,
            size: [2.0, 2.0],
            los: 8.0,
            state: 2,
            kind: 80,
            ..Surroundings::default()
        };
        let mut side = SoldierSide {
            found: 9731,
            attacks: Vec::new(),
        };
        let fx = u.update(28, 0, false, &at, &mut rng(), &mut side).unwrap();
        assert_eq!(fx, vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)]);
        assert_eq!(side.attacks, vec![(9731, true)]);
        assert_eq!((u.current_action, u.current_target), (ATTACK_ACTION, 9731));
    }

    /// The hunted animal's processIdle for a king (class 59, `FUN_140c1eea0`): after the cap a
    /// human player's king does nothing (5) and draws nothing; only a computer player's (not of
    /// master 704, not inside another, without an order) has its unit's slot 0x7e8, which is not
    /// modelled.
    #[test]
    fn a_human_player_s_king_s_idle_check_only_caps() {
        let at = Surroundings {
            position: [31.5, 106.5, 0.0],
            player: 1,
            class: CLASS_KING,
            master: 434,
            ..Surroundings::default()
        };
        let mut u = UnitAi {
            hunted: None,
            ..deer()
        };
        let mut r = rng();
        let mut fx = Vec::new();
        assert_eq!(
            u.process_idle(&at, false, &mut r, &mut no_search, &mut fx),
            Ok(false)
        );
        assert_eq!(fx, vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)]);
        assert_eq!(r.draws, 0);
        assert!(
            u.process_idle(&at, true, &mut r, &mut no_search, &mut Vec::new())
                .is_err()
        );
        // a computer player's king of master 704, or one with an order, does nothing either
        let unled = Surroundings {
            master: KING_UNLED,
            ..at.clone()
        };
        assert_eq!(
            u.process_idle(&unled, true, &mut r, &mut no_search, &mut Vec::new()),
            Ok(false)
        );
        u.current_order = MOVE_ORDER;
        assert_eq!(
            u.process_idle(&at, true, &mut r, &mut no_search, &mut Vec::new()),
            Ok(false)
        );
    }

    /// The civilian's `AttackObject` (slot 0xb8, `FUN_140c17370`, forced): an object the world
    /// does not hold gets nothing; an animal or a tree (classes 9, 10, 15) is hunted (slot 0xc8);
    /// any other goes to the base `AttackObject`, whose civilian slot 0xa0 (`FUN_140c18b70`)
    /// refuses classes 1 and 42, and at attack priority 3 the classes outside
    /// [`PRIORITY_3_CLASSES`].
    #[test]
    fn a_villager_hunts_an_animal_and_attacks_another_object() {
        let at = |priority: u8| Surroundings {
            id: 10041,
            state: 2,
            attack_priority: priority,
            units: vec![
                attacker(9731, CLASS_10, 0.5),
                attacker(9800, 6, 1.0),
                attacker(9801, 42, 1.0),
                attacker(9802, 3, 1.0),
            ],
            ..Surroundings::default()
        };
        let mut u = idle(8);
        let mut side = Recorder::default();
        let mut fx = Vec::new();
        let mut nobody = LionSide {
            found: [-1, -1],
            searches: 0,
            seen: UnitSeen::default(),
            attacks: Vec::new(),
        };
        assert_eq!(
            u.civilian_attack(9999, &at(0), &mut nobody, &mut fx),
            Ok(false)
        );
        assert_eq!(
            u.civilian_attack(9731, &at(0), &mut side, &mut fx),
            Ok(true)
        );
        assert_eq!(fx, vec![Effect::HuntObject(9731)]);
        assert_eq!((u.current_action, u.current_target), (HUNT_ACTION, 9731));
        let mut u = idle(8);
        let mut fx = Vec::new();
        assert_eq!(
            u.civilian_attack(9800, &at(0), &mut side, &mut fx),
            Ok(true)
        );
        assert_eq!(side.attacks, vec![(9800, true)]);
        assert_eq!((u.current_action, u.current_target), (ATTACK_ACTION, 9800));
        let mut u = idle(8);
        assert_eq!(
            u.civilian_attack(9801, &at(0), &mut side, &mut fx),
            Ok(false)
        );
        assert_eq!(
            u.civilian_attack(9800, &at(3), &mut side, &mut fx),
            Ok(false)
        );
        assert_eq!(
            u.civilian_attack(9802, &at(3), &mut side, &mut fx),
            Ok(true)
        );
        assert_eq!(side.attacks, vec![(9800, true), (9802, true)]);
    }

    /// Slot 0xa0 of the types whose `AttackObject` is the base's: the transport ship's and the
    /// livestock's (`FUN_140c48400`) pass anything, the building's (`FUN_140c18b60`) nothing.
    #[test]
    fn a_ship_or_livestock_may_attack_anything_and_a_building_nothing() {
        let at = Surroundings {
            id: 10041,
            state: 2,
            units: vec![attacker(9800, 6, 1.0)],
            ..Surroundings::default()
        };
        for (ai_type, attacks) in [(11, true), (19, true), (17, false)] {
            let mut u = idle(ai_type);
            let mut side = Recorder::default();
            let r = u.attack_object(9800, true, false, &at, &mut side, &mut Vec::new());
            assert_eq!(r, Ok(attacks), "type {ai_type}");
        }
    }

    /// A notify through the type's handler (slot 0x238) with a recording side and no world.
    fn handled(
        u: &mut UnitAi,
        n: Notify,
        at: &Surroundings,
        computer: bool,
    ) -> (Result<u32, Unsupported>, Vec<Effect>, Recorder) {
        let mut side = Recorder::default();
        let mut fx = Vec::new();
        let r = u.handle(n, 0, at, computer, &mut rng(), &mut side, &mut fx);
        (r, fx, side)
    }

    /// The boar's handler (`FUN_140c25770`) for the completion of its move off a footprint
    /// (0x1fa on action 0x2bf): its idle timeout becomes the adjusted one plus 1, the target goes
    /// (`FUN_140c5e1e0`), its slot 0x1f0 (`FUN_140c16d50`) sends it home on its next idle check,
    /// then the base handler stops the completed action.
    #[test]
    fn a_boar_whose_move_aside_completed_heads_home_next() {
        let mut u = UnitAi {
            current_action: MOVE_ASIDE_ACTION,
            current_target: 50,
            target_type: 4,
            adjusted_idle_timeout: 3100,
            idle_timeout: 3000,
            ..hit_boar()
        };
        let n = Notify::own(9729, ACTION_COMPLETED, [MOVE_ASIDE_ACTION, 0, 0]);
        let (r, fx, _) = handled(&mut u, n, &Surroundings::default(), false);
        assert_eq!(r, Ok(GO_ON));
        assert_eq!(fx, vec![Effect::Stop]);
        assert_eq!(u.idle_timeout, 3101);
        assert_eq!((u.current_target, u.last_target), (-1, 50));
        assert_eq!(u.current_action, -1);
        assert!(u.home.is_some_and(|h| h.go_home));
    }

    /// The lion's handler (`FUN_140c266f0`): a completed attack (0x1fa with 600) sets its idle
    /// timeout to 1000 and redraws the adjusted one (`FUN_140c41bc0`), drops the target and
    /// stops (slot 0xb0 with 1); the base handler then stops on the attack's completion again.
    #[test]
    fn a_lion_s_completed_attack_resets_its_idle_timeout_and_stops() {
        let mut u = UnitAi {
            current_action: ATTACK_ACTION,
            current_target: 700,
            idle_timeout: 6000,
            ..idle(5)
        };
        let n = Notify::own(800, ACTION_COMPLETED, [ATTACK_ACTION, 0, 0]);
        let mut r = rng();
        let expected = adjusted_idle_timeout(1000, &mut rng());
        let mut fx = Vec::new();
        let result = u.handle(
            n,
            0,
            &Surroundings::default(),
            false,
            &mut r,
            &mut Recorder::default(),
            &mut fx,
        );
        assert_eq!(result, Ok(GO_ON));
        assert_eq!((u.idle_timeout, u.adjusted_idle_timeout), (1000, expected));
        assert_eq!(r.draws, 1);
        assert_eq!(fx, vec![Effect::Stop, Effect::Stop]);
        assert_eq!(
            (u.current_target, u.last_target, u.current_action),
            (-1, 700, -1)
        );
    }

    /// The lion's handler for a moved target (0x1fc): a target the world no longer holds, or
    /// one beyond its line of sight (the footprints' squared gap against its sight squared,
    /// `COMISS` then `JC`), or one whose master has flag 0x10 (`+0x654`), is dropped and the lion
    /// stops before the base handler; one within its sight it attacks again (slot 0xb8 with the
    /// target, forced) and the processing breaks (3).
    #[test]
    fn a_lion_drops_a_moved_target_out_of_its_sight() {
        let lion = || UnitAi {
            current_action: ATTACK_ACTION,
            current_target: 700,
            ..idle(5)
        };
        let n = Notify::own(800, TARGET_MOVED, [0, 0, 0]);
        let mut u = lion();
        let (r, fx, _) = handled(&mut u, n, &Surroundings::default(), false);
        assert_eq!((r, fx), (Ok(GO_ON), vec![Effect::Stop]));
        assert_eq!((u.current_target, u.current_action), (-1, -1));
        // the villager 700 the lion attacks (class 4, which it attacks by its classes)
        let at = |x: f32| Surroundings {
            position: [10.0, 10.0, 0.0],
            size: [0.5, 0.5],
            los: 4.0,
            target: Some(Seen {
                state: 2,
                class: 4,
                position: [x, 10.0, 0.0],
                radius: [0.5, 0.5],
                ..Seen::default()
            }),
            units: vec![UnitSeen {
                id: 700,
                class: 4,
                owner: 1,
                state: 2,
                position: [x, 10.0, 0.0],
                ..UnitSeen::default()
            }],
            ..Surroundings::default()
        };
        // 5 tiles apart, 4 between the footprints: at its sight, attacked again
        let mut u = lion();
        let (r, fx, side) = handled(&mut u, n, &at(15.0), false);
        assert_eq!(
            (r, fx, side.attacks),
            (Ok(BREAK), vec![], vec![(700, true)])
        );
        assert_eq!((u.current_target, u.current_action), (700, ATTACK_ACTION));
        // a master with flag 0x10 is dropped at its sight too
        let mut u = lion();
        let mut flagged = at(15.0);
        flagged.target = flagged.target.map(|t| Seen { flags: 0x10, ..t });
        let (r, fx, side) = handled(&mut u, n, &flagged, false);
        assert_eq!(
            (r, fx, side.attacks),
            (Ok(GO_ON), vec![Effect::Stop], vec![])
        );
        // 6 apart, 5 between: beyond it
        let mut u = lion();
        let (r, fx, _) = handled(&mut u, n, &at(16.0), false);
        assert_eq!((r, fx), (Ok(GO_ON), vec![Effect::Stop]));
        assert_eq!(u.current_target, -1);
    }

    /// The building's handler (`FUN_140c22610`): an attack (600) adds its attacker and puts it
    /// under attack; 699 drops one; damage (500) at no hit points ends the notifies (4), else
    /// adds an attacker already known or its target; a new one makes a human player's building
    /// ask for help (`FUN_140c42ce0`, not modelled), and a computer player's AI hears of it.
    #[test]
    fn a_building_counts_its_attackers() {
        let mut u = idle(17);
        let at = Surroundings::default();
        let (r, _, side) = handled(&mut u, Notify::own(70, 600, [0; 3]), &at, false);
        assert_eq!((r, side.under_attack), (Ok(GO_ON), Some(true)));
        let _ = handled(&mut u, Notify::own(60, 600, [0; 3]), &at, false);
        assert_eq!(u.attackers, vec![60, 70]);
        let (r, _, _) = handled(&mut u, Notify::own(70, NO_LONGER_ON_ME, [0; 3]), &at, false);
        assert_eq!((r, u.attackers.clone()), (Ok(GO_ON), vec![60]));
        let (r, _, _) = handled(&mut u, Notify::own(99, NO_LONGER_ON_ME, [0; 3]), &at, false);
        assert_eq!((r, u.attackers.clone()), (Ok(GO_ON), vec![60]));
        let hit = |caller, hp| Notify::own(caller, DAMAGED, [caller, hp, 0]);
        let (r, _, side) = handled(&mut u, hit(60, 5), &at, false);
        assert_eq!((r, side.under_attack), (Ok(GO_ON), Some(true)));
        u.current_target = 61;
        let (r, _, _) = handled(&mut u, hit(61, 5), &at, false);
        assert_eq!((r, u.attackers.clone()), (Ok(GO_ON), vec![60, 61]));
        assert!(handled(&mut u, hit(62, 5), &at, false).0.is_err());
        assert!(handled(&mut u, hit(60, 5), &at, true).0.is_err());
        u.notifies = vec![hit(60, 0)];
        let (r, _, _) = handled(&mut u, hit(60, 0), &at, false);
        assert_eq!((r, u.notifies.len()), (Ok(DEAD), 0));
    }

    /// The tower's handler (`FUN_140c299f0`) for damage at no hit points: the owner's alert
    /// (0x209, a human player's alert list), the notifies end and it gives up (4).
    #[test]
    fn a_destroyed_tower_gives_up() {
        let mut u = UnitAi {
            notifies: vec![Notify::own(60, DAMAGED, [60, 0, 0])],
            ..idle(15)
        };
        let n = Notify::own(60, DAMAGED, [60, 0, 0]);
        let (r, _, _) = handled(&mut u, n, &Surroundings::default(), false);
        assert_eq!((r, u.notifies.len()), (Ok(DEAD), 0));
        let mut u = idle(15);
        assert!(
            handled(&mut u, n, &Surroundings::default(), true)
                .0
                .is_err()
        );
    }

    /// The hunted animal's evade goes through `evasiveMoveTo` (slot 0x150 with 1 and 0,
    /// `FUN_140c4d140`): an object on its own point gives a point that is not a number, which
    /// becomes its own place minus one on each axis (above 5) before the map's hold.
    #[test]
    fn a_deer_evading_an_object_on_its_own_point_steps_off_by_one() {
        let mut u = deer();
        let mut r = wander_seed(false);
        let place = Surroundings {
            speed: 1.2,
            map: [120, 120],
            ..deer_at([31.5, 106.5, 0.0])
        };
        let on_top = |_: &Search<'_>| {
            Ok(Found {
                id: 9183,
                enemies: false,
                position: Some([31.5, 106.5, 0.0]),
                fresh: true,
            })
        };
        let moved = u
            .update_idle(10, 0, false, &place, &mut r, &mut on_top.clone())
            .unwrap()
            .unwrap();
        assert_eq!(moved.point, [30.5, 105.5, 0.0]);
        let near_edge = Surroundings {
            speed: 1.2,
            map: [120, 120],
            ..deer_at([3.5, 4.5, 0.0])
        };
        let mut u = deer();
        let on_it = |_: &Search<'_>| {
            Ok(Found {
                id: 9183,
                enemies: false,
                position: Some([3.5, 4.5, 0.0]),
                fresh: true,
            })
        };
        let moved = u
            .update_idle(
                10,
                0,
                false,
                &near_edge,
                &mut wander_seed(false),
                &mut on_it.clone(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(moved.point, [4.5, 5.5, 0.0]);
    }

    /// A unit side whose stand test fails a given number of times.
    struct StandSide {
        refusals: usize,
        tested: Vec<[f32; 2]>,
    }

    impl UnitSide for StandSide {
        fn closest(&mut self, _: &Search<'_>) -> Result<Found, Unsupported> {
            Err(Unsupported("no search here"))
        }
        fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, Unsupported> {
            Err(Unsupported("no attack here"))
        }
        fn set_under_attack(&mut self, _: bool) {}
        fn stop(&mut self) -> Result<(), Unsupported> {
            Ok(())
        }
        fn may_attack(
            &mut self,
            _: i32,
            _: &Seeker,
        ) -> Result<Option<(i16, [f32; 3])>, Unsupported> {
            Err(Unsupported("no world here"))
        }
        fn seek_unexplored(
            &mut self,
            _: &UnitAi,
            _: &mut Well512,
        ) -> Result<Option<[i32; 2]>, Unsupported> {
            Err(Unsupported("no search here"))
        }
        fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, Unsupported> {
            Err(Unsupported("no search here"))
        }
        fn make_scout_map(&mut self) -> Result<(), Unsupported> {
            Err(Unsupported("no world here"))
        }
        fn can_stand(&mut self, point: [f32; 2]) -> Result<bool, Unsupported> {
            self.tested.push(point);
            if self.refusals == 0 {
                return Ok(true);
            }
            self.refusals -= 1;
            Ok(false)
        }
    }

    /// `evasiveMoveTo` (`FUN_140c4d140`): without `uncon` a unit with an action stays; with
    /// `tries` a point the unit cannot stand at moves by four draws (x's sign, its amount, y's
    /// sign, its amount: `rand15 & 1`, `rand15 & 3`), held on the map, up to five times; then the
    /// forced running move within 0, the move action value and the point, the AI commanded.
    #[test]
    fn an_evasive_move_steps_off_a_point_the_unit_cannot_stand_at() {
        let at = Surroundings {
            position: [10.0, 20.0, 1.5],
            map: [64, 64],
            ..Surroundings::default()
        };
        let mut busy = UnitAi {
            current_action: ATTACK_ACTION,
            ..idle(2)
        };
        let mut side = StandSide {
            refusals: 0,
            tested: Vec::new(),
        };
        let mut fx = Vec::new();
        assert_eq!(
            busy.evasive_move(
                [12.0, 22.0, 1.5],
                (false, true),
                &at,
                &mut rng(),
                &mut side,
                &mut fx
            ),
            Ok(false)
        );
        assert_eq!((fx.len(), side.tested.len()), (0, 0));
        let mut u = idle(2);
        let mut side = StandSide {
            refusals: 1,
            tested: Vec::new(),
        };
        let mut r = rng();
        let mut expected = rng();
        let step = |r: &mut Well512, v: f32| {
            let up = r.rand15() & 1 != 0;
            let by = (r.rand15() & 3) as f32;
            if up { v + by } else { v - by }
        };
        let ex = step(&mut expected, 12.0);
        let ey = step(&mut expected, 22.0);
        assert_eq!(
            u.evasive_move(
                [12.0, 22.0, 1.5],
                (false, true),
                &at,
                &mut r,
                &mut side,
                &mut fx
            ),
            Ok(true)
        );
        assert_eq!(side.tested, vec![[12.0, 22.0], [ex, ey]]);
        assert_eq!(r.draws, 4);
        assert_eq!(
            fx,
            vec![Effect::Move(AiMove {
                point: [ex, ey, 1.5],
                range: 0.0,
                force: true,
                walk: false,
            })]
        );
        assert_eq!(
            (u.current_action, u.target_point),
            (MOVE_ACTION, [ex, ey, 1.5])
        );
        assert!(u.commanded);
        // a point past the far edge is held a tile in, below 0 at 0
        let mut u = idle(2);
        let mut fx = Vec::new();
        let mut side = StandSide {
            refusals: 0,
            tested: Vec::new(),
        };
        let _ = u.evasive_move(
            [70.0, -3.0, 0.0],
            (true, false),
            &at,
            &mut rng(),
            &mut side,
            &mut fx,
        );
        assert_eq!(first_move(&fx).map(|m| m.point), Some([63.0, 0.0, 0.0]));
    }

    /// Slot 0x148 (`FUN_140c4d0d0`): unforced, a unit AI with an action value takes no move off
    /// a footprint; forced, or without one, the unit's slot 0x340 gets the move
    /// ([`Effect::MoveAside`]) and the action value becomes 0x2bf, the old one the last.
    #[test]
    fn a_move_off_a_footprint_needs_force_or_no_action() {
        let m = |force| AiMove {
            point: [4.0, 5.0, 0.0],
            range: 0.0,
            force,
            walk: true,
        };
        let mut u = UnitAi {
            current_action: GATHER_ACTION,
            current_target: 40,
            ..idle(8)
        };
        let mut fx = Vec::new();
        assert!(!u.move_aside(m(false), &mut fx));
        assert_eq!((fx.len(), u.current_action), (0, GATHER_ACTION));
        assert!(u.move_aside(m(true), &mut fx));
        assert_eq!(fx, vec![Effect::MoveAside(m(true))]);
        assert_eq!(
            (u.last_action, u.current_action, u.current_target),
            (GATHER_ACTION, MOVE_ASIDE_ACTION, 40)
        );
        let mut idle_unit = idle(8);
        let mut fx = Vec::new();
        assert!(idle_unit.move_aside(m(false), &mut fx));
        assert_eq!(fx, vec![Effect::MoveAside(m(false))]);
    }

    /// Slot 0x130 (`FUN_140c55620`): unforced, a unit AI with an action value takes no move;
    /// forced, the action value becomes the move's (the old one the last) and `FUN_140c64c10`
    /// with −1 moves the target to the last and takes the point, leaving the target class.
    #[test]
    fn an_ai_move_needs_force_or_no_action_and_keeps_the_target_class() {
        let m = |force| AiMove {
            point: [4.0, 5.0, 0.5],
            range: 0.5,
            force,
            walk: true,
        };
        let mut u = UnitAi {
            current_action: ATTACK_ACTION,
            current_target: 40,
            target_type: 9,
            last_target: 30,
            last_target_type: 4,
            ..idle(4)
        };
        let before = u.clone();
        assert_eq!(u.move_to(m(false)), None);
        assert_eq!(u, before);
        assert_eq!(u.move_to(m(true)), Some(m(true)));
        assert_eq!(
            (u.last_action, u.current_action),
            (ATTACK_ACTION, MOVE_ACTION)
        );
        assert_eq!(
            (
                u.last_target,
                u.last_target_type,
                u.current_target,
                u.target_type
            ),
            (40, 9, -1, 9)
        );
        assert_eq!(u.target_point, [4.0, 5.0, 0.5]);
    }

    /// `FUN_140c64b60`: leaving a patrol order (711, 733) redraws an adjusted idle timeout of
    /// exactly 750 and caps the secondary timer at 750 by a signed compare (a timer read as
    /// negative stays); with its third argument 0 the last order stays.
    #[test]
    fn the_order_setter_caps_by_a_signed_compare_and_may_keep_the_last_order() {
        let mut u = UnitAi {
            current_order: PATROL,
            last_order: 5,
            adjusted_idle_timeout: 3000,
            secondary_timer: 0x8000_0000,
            ..idle(13)
        };
        u.set_order_with(ATTACK_ORDER, false, &mut rng());
        assert_eq!((u.current_order, u.last_order), (ATTACK_ORDER, 5));
        assert_eq!(u.secondary_timer, 0x8000_0000);
        let mut u = UnitAi {
            current_order: PATROL_2,
            adjusted_idle_timeout: 750,
            idle_timeout: 3000,
            secondary_timer: 900,
            ..idle(13)
        };
        let mut r = rng();
        u.set_order(-1, &mut r);
        assert_eq!((u.current_order, u.last_order), (-1, PATROL_2));
        assert_eq!(u.secondary_timer, 750);
        assert_eq!(r.draws, 1);
    }

    /// The civilian's slot 73 (`FUN_140c21da0`): on the attack order without a target it stops
    /// its action and ends the order (slot 0xb0 with 0: the state position takes its place);
    /// with a target, or on another order, nothing.
    #[test]
    fn a_villager_on_the_attack_order_without_a_target_gives_it_up() {
        let at = Surroundings {
            position: [12.0, 13.0, 0.0],
            class: 4,
            ..Surroundings::default()
        };
        let mut u = UnitAi {
            current_order: ATTACK_ORDER,
            order_priority: 100,
            current_action: ATTACK_ACTION,
            ..idle(8)
        };
        let mut fx = Vec::new();
        u.civilian_misc(&at, &mut fx).unwrap();
        assert_eq!(fx, vec![Effect::Stop]);
        assert_eq!(
            (
                u.current_order,
                u.order_priority,
                u.last_order,
                u.current_action
            ),
            (-1, -1, ATTACK_ORDER, -1)
        );
        assert_eq!(u.state_position, [12.0, 13.0]);
        for (order, target) in [(ATTACK_ORDER, 40), (BUILD_ORDER, -1)] {
            let mut u = UnitAi {
                current_order: order,
                current_target: target,
                ..idle(8)
            };
            let mut fx = Vec::new();
            u.civilian_misc(&at, &mut fx).unwrap();
            assert_eq!((fx.len(), u.current_order), (0, order));
        }
    }

    /// The factory (`FUN_141215a90`): masters 1105, 1744 and 1746 by their id; then by class,
    /// the classes without a unit AI, the civilian, the building (or the tower with a volley
    /// missile, `+0x8d4`, and for class 52), the hunted animal, the soldier as the default and
    /// the rest of the switch; class 10's type its master's combat level picks
    /// (`FUN_140fde4f0`), which the arguments do not carry.
    #[test]
    fn the_factory_picks_the_unit_ai_type_by_master_and_class() {
        assert_eq!(factory_type(6, 1105, -1), Ok(Some(0x12)));
        assert_eq!(factory_type(6, 1744, -1), Ok(Some(0x10)));
        assert_eq!(factory_type(6, 1746, -1), Ok(Some(0x10)));
        let by_class = [
            (1, 0xe),
            (2, 0xa),
            (19, 0xa),
            (4, 8),
            (9, 2),
            (59, 2),
            (13, 0x10),
            (51, 0x10),
            (54, 0x10),
            (55, 0x10),
            (18, 9),
            (43, 9),
            (53, 9),
            (20, 0xb),
            (21, 0xc),
            (29, 0x13),
            (58, 0x13),
            (52, 0xf),
            (6, 0xd),
            (0, 0xd),
            (64, 0xd),
        ];
        for (class, ai_type) in by_class {
            assert_eq!(
                factory_type(class, 83, -1),
                Ok(Some(ai_type)),
                "class {class}"
            );
        }
        for class in [3, 27, 39, 49, 60] {
            assert_eq!(factory_type(class, 70, -1), Ok(Some(17)));
            assert_eq!(factory_type(class, 70, 5), Ok(Some(0xf)));
        }
        for class in [5, 7, 8, 11, 14, 15, 16, 30, 31, 32, 33, 34, 38, 42, 48, 63] {
            assert_eq!(factory_type(class, 83, -1), Ok(None));
        }
        assert!(factory_type(10, 48, -1).is_err());
    }

    /// The building's cared classes (`FUN_140c11470`) are the soldier's in another order.
    #[test]
    fn the_building_cares_about_the_soldier_s_classes_in_its_own_order() {
        let mut building = BUILDING_CARED.to_vec();
        let mut soldier = SOLDIER_CARED.to_vec();
        assert_ne!(building, soldier);
        building.sort_unstable();
        soldier.sort_unstable();
        assert_eq!(building, soldier);
    }

    /// The hunted animal's damage notify (`FUN_140c26420`) for a human player's king (class 59):
    /// the owner's alert (0x201, a human player's alert list, not held), the attacker joins the
    /// set, the unit is under attack and runs from its attackers (slot 0x160; class 59 not
    /// unconditionally, so only without an action value), by twice its speed away from them;
    /// processing breaks off (3). A computer player's AI would hear the alert (not modelled).
    #[test]
    fn a_human_player_s_king_runs_from_its_attacker() {
        let mut u = UnitAi {
            hunted: None,
            ..idle(2)
        };
        let at = Surroundings {
            id: 500,
            position: [20.0, 30.0, 0.0],
            speed: 1.0,
            map: [64, 64],
            player: 1,
            class: CLASS_KING,
            state: 2,
            units: vec![UnitSeen {
                id: 77,
                class: 6,
                owner: 2,
                state: 2,
                target: Some(500),
                position: [22.0, 30.0, 0.0],
                ..UnitSeen::default()
            }],
            ..Surroundings::default()
        };
        let n = Notify::own(77, DAMAGED, [77, 40, 0]);
        let mut side = StandSide {
            refusals: 0,
            tested: Vec::new(),
        };
        let mut fx = Vec::new();
        let r = u.hunted_damaged(n, 0, &at, false, &mut rng(), &mut side, &mut fx);
        assert_eq!(r, Ok(BREAK));
        assert_eq!(u.attackers, vec![77]);
        assert_eq!(
            fx,
            vec![Effect::Move(AiMove {
                point: [18.0, 30.0, 0.0],
                range: 0.0,
                force: true,
                walk: false,
            })]
        );
        let mut u = UnitAi {
            hunted: None,
            ..idle(2)
        };
        let r = u.hunted_damaged(n, 0, &at, true, &mut rng(), &mut side, &mut Vec::new());
        assert!(r.is_err());
    }

    /// `FUN_140c64c10`: the target and its class become the last ones, the id and the point are
    /// taken, and the class only when the world holds the object (else it stays).
    #[test]
    fn a_target_takes_its_class_only_from_an_object_the_world_holds() {
        let mut u = UnitAi {
            current_target: 40,
            target_type: 9,
            ..idle(13)
        };
        u.target_at(41, None, [1.0, 2.0, 3.0]);
        assert_eq!(
            (
                u.last_target,
                u.last_target_type,
                u.current_target,
                u.target_type
            ),
            (40, 9, 41, 9)
        );
        assert_eq!(u.target_point, [1.0, 2.0, 3.0]);
        u.target_at(42, Some(4), [5.0, 6.0, 0.0]);
        assert_eq!(
            (
                u.last_target,
                u.last_target_type,
                u.current_target,
                u.target_type
            ),
            (41, 9, 42, 4)
        );
    }

    /// Slot 0x38 (`FUN_140c556d0`) answers whether it took the notify: 0 outside state 2, for a
    /// 699 from a unit that is not attacking it and for the invalidation of another action; 1
    /// for a movement completion it ignores (not queued) and for one it queues.
    #[test]
    fn the_unit_ai_s_notify_answers_whether_it_took_it() {
        let mut u = UnitAi {
            current_action: GATHER_ACTION,
            attackers: vec![70],
            ..idle(8)
        };
        let n = |caller, kind, p0| Notify::own(caller, kind, [p0, 0, 0]);
        assert!(!u.accept(n(70, DAMAGED, 0), 0));
        assert!(!u.accept(n(71, NO_LONGER_ON_ME, 0), 2));
        assert!(!u.accept(n(70, ACTION_INVALIDATED, ATTACK_ACTION), 2));
        assert!(u.accept(n(70, ACTION_COMPLETED, MOVE_ACTION), 2));
        assert_eq!(u.notifies, Vec::new());
        assert!(u.accept(n(70, NO_LONGER_ON_ME, 0), 2));
        assert!(u.accept(n(70, ACTION_INVALIDATED, GATHER_ACTION), 2));
        assert_eq!(u.notifies.len(), 2);
    }
}
