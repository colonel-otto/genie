//! The unit AI of an idle unit on its owner's turn: the timers and the AI-stream draws behind
//! checkpoint word 9.
//!
//! 158041: the full update is `UnitAIModule` slot 4 (`0x140c66b10`); slot 73 (`0x140c567c0`) redraws
//! the retarget timer, slot 72 is the idle check (`processIdle`). Only an idle unit is modelled: no
//! order, no action, no target, no actions in its list. Anything else is [`GenieError`]. A boar
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

use crate::fmath::Vec3;
use crate::image::unit_ai::{ATTACKER_FAR, ATTACKER_TIE};
use crate::movement::PathAsk;
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
/// `0x203`: an action's drop-off found no drop site (the Build's `set_state(8)`, a gather's).
pub const NO_DROP_SITE: i32 = 0x203;
/// `0x21d`: a human player's builder left a pasture already built (p2 the pasture its stack unit
/// made).
pub const PASTURE_BUILT: i32 = 0x21d;
/// `0x21c`: an action asked to drop its load off carries another resource than its task's (the
/// base action's slot 0x78, `FUN_140a78060`).
pub const OTHER_LOAD: i32 = 0x21c;
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
/// An attack command after effects the unit AI's run left pending that the slice cannot run
/// before it (a hunt command, a move, …): their unit parts would precede the command.
pub(crate) const PENDING_BEFORE_ATTACK: GenieError =
    GenieError::unsupported("an attack command after a stop with other effects pending");
/// `0x20f`: a missile's warning, which a shot whose blast does not spare friends sends its
/// target when a computer player owns it (`FUN_141081020`).
const MISSILE_WARNING: i32 = 0x20f;
/// The damage case's unit too slow to run away (`DAT_142eb0c60`, [`UnitAi::damaged`]).
const STILL: f32 = crate::image::unit_ai::STILL;
/// The damage case's kill time of an object the unit cannot harm (`DAT_142efe41c`, 25000).
const NEVER_KILLED: f32 = crate::image::unit_ai::NEVER_KILLED;
/// The action value of a unit moving off a footprint with a group.
pub const GROUP_ASIDE_ACTION: i32 = 0x2c0;
/// The order on a foundation a villager sent off its footprint takes up again (slot 0x108,
/// `FUN_140c5c170` case 0x2de).
pub const FOUNDATION_ORDER: i32 = 0x2de;
/// The action value of a unit waiting for a foundation (slot 0x108, `FUN_140c68a60`, and the
/// order switch's case 0x2de); the full update's reset of ended actions leaves it.
pub const WAIT_SOLIDIFY_ACTION: i32 = 0x27a;
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
/// What the stand ground stance's leash adds to the unit's range (`DAT_142eb880c`, 0.25, read by
/// `ADDSS` at `0x140c57c6a`).
const STAND_GROUND_SLACK: f32 = crate::image::unit_ai::STAND_GROUND_SLACK;
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
/// The siege tower's factory type (`TribeSiegeTowerUnitAIModule`, master 1105's).
const SIEGE_TOWER_AI: u32 = 0x12;
/// The priest's factory type (`TribePriestUnitAIModule`: classes 18, 43 and 53), whose slot 72,
/// notify handler and attack checks (its conversions and heals, `FUN_140c1feb0`,
/// `FUN_140c26f60`, `FUN_140c19020`, `FUN_140c1af00`) are not modelled.
pub(crate) const PRIEST_AI: u32 = 9;
/// The siege unit's factory type (`TribeSiegeUnitAIModule`: classes 13, 51, 54, 55, masters 1744
/// and 1746).
const SIEGE_AI: u32 = 0x10;
/// The predator's factory type (`TribePreditorUnitAIModule`, `FUN_140c11da0`).
const PREDATOR_AI: u32 = 3;
/// The lion's factory type (`TribeLionUnitAIModule`, `FUN_140c11bd0`).
const LION_AI: u32 = 5;
/// The livestock's factory type (`TribeLivestockUnitAIModule`, `FUN_140c11d70`).
const LIVESTOCK_AI: u32 = 0x13;
/// The trade ship's and the trade cart's factory type (`TribeTradeShipUnitAIModule`, classes 2
/// and 19).
const TRADE_AI: u32 = 0xa;
/// The fishing ship's factory type (`TribeFishingShipUnitAIModule`, class 21).
const FISHING_AI: u32 = 0xc;
/// The trade boat's class (`-0xeaa` raw): its cared classes and its trade partner are not the
/// trade cart's.
const TRADE_BOAT_CLASS: i16 = 2;
/// The trade action value (`0x267`, which slot 0xf8 `FUN_140c651f0` sets).
const TRADE_ACTION: i32 = 0x267;
/// A computer player's ship evades the object its idle check found by its speed times this
/// (`FUN_140c21930`, `FUN_140c1eab0`: `DAT_142ec9eb8`).
const SHIP_EVADE_REACH: f32 = crate::image::unit_ai::SHIP_EVADE_REACH;
/// The classes of the foundations a fishing ship's builder looks for (`FUN_140c25ca0`, in its
/// order): buildings, walls, gates, towers and farms.
const FISHING_BUILDS: [i32; 5] = [3, 27, 39, 52, 49];
/// The fishing ship's builder takes the nearest foundation nearer than this (`FUN_140c25ca0`'s
/// 9999).
const FISHING_BUILD_FAR: i32 = 9999;
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
        /// The target's state as the AI saw it, which decides whether the `MoveTo` holds it
        /// ([`crate::move_to::holds_object`]).
        state: u8,
        /// The range within which the `MoveTo` arrives.
        range: f32,
    },
    /// `BuildObject`'s unit part (the AI's slot 0xf0, `FUN_140c47d80`): the unit's build command
    /// (slot 0x380) on the foundation, which the caller runs after the update in the order the AI
    /// gave it with its hunts, gathers and works (`Turns::build_command`); the AI's part, the
    /// build action and the foundation as its target, is done at once ([`UnitAi::build_object`]).
    BuildObject(i32),
    /// The unit's ownership change (its slot 0x70, `FUN_14120f800` for a combat unit,
    /// `FUN_1411cd860` for a building) to the player, which the artifact's processIdle asks of
    /// it (`FUN_140c1e410`); the unit runs it as it applies the effects.
    ChangeOwner(usize),
    /// The unit's slot 0x7e8 (`FUN_141227190`) on object `target` at `point`, with the
    /// carried-resource test, any task and `+0xa14` 0 (the caller's, `Turns::order_work`, after
    /// the update with the AI's hunts and gathers in their order).
    WorkOn {
        /// The object.
        target: i32,
        /// Its place.
        point: [f32; 3],
    },
    /// The villager's case 0x21c ([`OTHER_LOAD`]) with the drop-site table's building for its load
    /// ([`UnitSide::table_drop_site`]): the unit's slot 0x128 on it at its place, then its unit
    /// AI's order back to the target its top action had before (slot 0x190), if any (the
    /// caller's, `Turns::drop_elsewhere`).
    DropElsewhere {
        /// The drop site.
        site: i32,
        /// The top action's target before the work.
        back: Option<i32>,
    },
    /// `restorePatrolInformation`'s unit part (`FUN_140c5e5b0`): the unit's own waypoints go
    /// (slot 0x450) and become the saved patrol's points (slot 0x448 each), and the unit moves to
    /// its waypoint (slot 0x340: range 0, forced, running).
    RestorePatrol,
}

/// What a full update that took an order on a target ([`Effect::OrderOnTarget`]) leaves for
/// its rest ([`UnitAi::after_order`]): the game runs the order inside the update (slot 0x230),
/// so slot 73 and the idle part read the action, target and order it gave the AI; the idle part
/// reads what the update read of its unit before the order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AfterOrder {
    /// The unit was busy after the notifies (slot 0x430 again, the update's `busy`).
    pub busy: bool,
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

/// A unit AI's saved patrol (`+0x1c0`, a `Path` of its own): a copy of its unit's top path
/// (the unit's slot 0x468, `FUN_141157c50`: `FUN_140f95c00` into a new path) with that path's
/// flags (`+0xc`: bit 0 a loop, bit 1 back and forth, `getPatrolLinear` `FUN_140c4df80`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PatrolPath {
    /// The points (x, height, y) and the length.
    pub path: crate::tile_path::Path,
    /// The path's flags.
    pub flags: u32,
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
    /// `+0x60`, `DefendTarget`: the object the defend order keeps the unit near (−1 for none).
    pub defend_target: i32,
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
    /// `+0x1c0`, the saved patrol (the save's `hasPatrolPath` and its path): what
    /// `savePatrolInformation` (`FUN_140c63d20`) kept of the unit's patrol when an attack took it
    /// off it, which `restorePatrolInformation` (`FUN_140c5e5b0`) walks on.
    pub patrol_path: Option<PatrolPath>,
    /// `+0x1c8`, `mPatrolCurrentWaypoint`: the saved patrol's waypoint the unit walked to.
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
    /// The order history (`+0x78`'s list, the newest at `+0xa8` first, count `+0xb8`; the
    /// save's `ohCount` entries): the orders on a target ([`UnitAi::remember_order`]),
    /// `bestUnitToAttack`'s weights by attack category.
    pub order_history: Vec<OrderRecord>,
    /// The retarget entries (`+0x1d8` count, `+0x1e0` array): targets `bestUnitToAttack` could
    /// not path to, each with the world time until which it ignores them (`FUN_140c42320`); the
    /// full update lets the expired go ([`UnitAi::expire_retries`]).
    pub retries: Vec<(i32, u32)>,
    /// `+0x1d0`, `mBestUnitToAttackThisUpdate`: what the damage notify's `bestUnitToAttack`
    /// found this update ([`UnitAi::damaged`]), −1 before one ran (the constructor's, and the
    /// full update's start); the update's later damage notifies take it.
    pub best_this_update: i32,
}

/// An order on a target in a unit AI's history (`FUN_140c42080`, a node of `+0x78`'s list).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OrderRecord {
    /// The order (`mOrder`).
    pub order: i32,
    /// The action (`mAction`).
    pub action: i32,
    /// The world time it was given (`mTime`).
    pub time: u32,
    /// The unit's place then (slot 0x520, `mPosition`).
    pub position: [f32; 3],
    /// The target (`mTargetID`).
    pub target: i32,
    /// The target's attack category ([`crate::best_target::category`], `mTargetAttackCategory`).
    pub category: i32,
    /// The target's place then (`mTargetPosition`).
    pub target_position: [f32; 3],
}

/// The order history's length (`FUN_140c42080`: a ninth order recycles the oldest).
pub const ORDER_HISTORY: usize = 8;

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
    /// What an attack pick reads of the unit AI's memory.
    pub memory: Memory<'a>,
}

/// `bestUnitToAttack`'s arguments past its point ([`crate::best_target::Ask`] without its
/// list: at most one extra object).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttackAsk {
    /// The reference distance (0: none).
    pub distance: f32,
    /// An idle check.
    pub idle: bool,
    /// The path checks.
    pub paths: bool,
    /// An extra object for the statistics.
    pub extra: Option<i32>,
    /// Any class.
    pub any_class: bool,
    /// The unit is in an attack work state.
    pub work_state: bool,
}

impl AttackAsk {
    /// The idle checks' call: no reference distance, an idle check, path checks on, no extra
    /// object, any class, not in an attack work state.
    pub const IDLE: Self = Self {
        distance: 0.0,
        idle: true,
        paths: true,
        extra: None,
        any_class: true,
        work_state: false,
    };

    /// The full arguments ([`crate::best_target::Ask`]).
    #[must_use]
    pub fn ask(&self) -> crate::best_target::Ask {
        crate::best_target::Ask {
            distance: self.distance,
            idle: self.idle,
            paths: u8::from(self.paths),
            extra: self.extra.into_iter().collect(),
            any_class: self.any_class,
            work_state: self.work_state,
        }
    }
}

/// What `bestUnitToAttack` reads of the searching unit AI's memory: its retarget entries
/// (`+0x1e0`) and its order history (`+0x78`'s list).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Memory<'a> {
    /// The retarget entries.
    pub retries: &'a [(i32, u32)],
    /// The order history, the newest first.
    pub history: &'a [OrderRecord],
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
        /// Argument 8: only objects the unit reaches count (slot 0x3d0); every caller but the
        /// artifact's passes 1.
        reach: bool,
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
        /// `bestUnitToAttack`'s arguments past its point.
        ask: AttackAsk,
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
    /// Its `+0x924` (`FUN_14107cc90`): a Gaia siege unit attacks only with 0.
    pub capture: u8,
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
    /// The master type of the current target as the world holds it ([`Surroundings::target`]),
    /// which the siege unit's attack check reads.
    pub target_kind: Option<u8>,
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

/// Slot 0xa0 of `me`'s unit AI with 0, `CanAttack`: whether it may attack `t`, by type: the
/// hunted animal's (`FUN_140c18d00`) while its action list holds an Attack (type 9) or a type 7
/// (slot 0x538); the predator's ([`predator_may_attack`]), the lion's ([`lion_may_attack`]) and
/// the civilian's ([`civilian_may_attack`]) by attack priority and class; the base's
/// (`FUN_140c48400`: the boar, the ranged units, the ships, the artifact, the siege tower,
/// livestock) always; the building's (`FUN_140c18b60`) never; the tower's
/// ([`tower_can_attack`]), the soldier's ([`soldier_can_attack`]) and the siege unit's. The
/// priest's (`FUN_140c19020`: its conversion test, slot 0x1b8) is not modelled here.
pub fn can_attack(me: &Attacker, seeker: &Seeker, t: &Target) -> Result<bool, GenieError> {
    match me.ai_type {
        2 => Ok(matches!(me.top_action, Some(9 | 7))),
        3 => Ok(predator_may_attack(me.attack_priority, t.class)),
        BOAR_AI | 6 | 7 | TRADE_AI | TRANSPORT_AI | FISHING_AI | ARTIFACT_AI | SIEGE_TOWER_AI
        | 0x13 => Ok(true),
        5 => Ok(lion_may_attack(me.attack_priority, t.class)),
        CIVILIAN => Ok(civilian_may_attack(me.attack_priority, t.class)),
        BUILDING_AI => Ok(false),
        15 => tower_can_attack(me, t),
        13 => soldier_can_attack(me, seeker, t),
        SIEGE_AI => siege_can_attack(me, seeker, t, false),
        _ => Err(GenieError::unsupported(
            "an attack check (slot 0xa0) of a unit-AI type not modelled",
        )),
    }
}

/// The classes a predator attacks by its own choice (its slot 0xa0's list, `FUN_140c18ed0`).
const PREDATOR_ATTACKS: [i16; 18] = [
    0, 4, 6, 12, 23, 24, 25, 26, 28, 35, 36, 44, 45, 46, 47, 50, 56, 57,
];

/// The predator's slot 0xa0 (`FUN_140c18ed0`) without its `any` flag: by the master's attack
/// priority 1 and 2 anything, 3 the classes of [`PRIORITY_3_CLASSES`], any other the classes of
/// [`PREDATOR_ATTACKS`].
fn predator_may_attack(attack_priority: u8, class: i16) -> bool {
    match attack_priority {
        1 | 2 => true,
        3 => PRIORITY_3_CLASSES.contains(&class),
        _ => PREDATOR_ATTACKS.contains(&class),
    }
}

/// The tower's attack check (`FUN_140c1a760`): never for Gaia's own tower nor the classes 1 and
/// 42; Gaia's objects only of the classes 9 and 10, unless they are of the combat type with
/// `+0x924` 0 outside the scenario modes; never a building (master type `'P'`) no builder has
/// started (`desolid_flag`, `+0xc60`), masters 50 and 1187, nor from a tower in a state below 2. Class 10 with an attack
/// priority below 3 only of combat level 4 (`FUN_140fde120`); else when the tower is armed
/// (`FUN_14120ea30`: classes 51 and 54, or a missile, or else a firepower, `+0xa00`, above 1)
/// and its attack priority is not 3 or the class one of [`PRIORITY_3_CLASSES`].
pub fn tower_can_attack(me: &Attacker, t: &Target) -> Result<bool, GenieError> {
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
    pub const fn range_against(&self) -> Result<f32, GenieError> {
        if matches!(self.charge_type, 6 | 7) {
            return Err(GenieError::unsupported(
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
pub fn soldier_can_attack(me: &Attacker, s: &Seeker, t: &Target) -> Result<bool, GenieError> {
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
        return Err(GenieError::unsupported(
            "a computer player's soldier's attack check",
        ));
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
                defend_patrol_range()?;
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
pub(crate) fn spares_friends(me: &Attacker, t: &Target) -> Result<bool, GenieError> {
    if me.attack_priority == 3 && !PRIORITY_3_CLASSES.contains(&t.class) {
        return Ok(false);
    }
    let ranged = 1.0 < me.range;
    if ranged && me.blast_level & 8 == 0 && 0.0 < me.area_effect && me.blast_level & 4 == 0 {
        return Err(GenieError::unsupported(
            "a blast attacker's scan for friends near its target (FUN_14120e4e0)",
        ));
    }
    Ok(true)
}

/// What `bestUnitToAttack` (`FUN_140c44680`, [`crate::best_target::best_unit_to_attack`]) asks
/// of the world while its unit AI runs: the searching unit ([`AttackPicker::searcher`]) and its
/// reads and tests of the objects its search found; the paths (slot 0x3d0) are the search's.
pub trait AttackPicker {
    /// The searching unit as `bestUnitToAttack` reads it, with its AI's `memory`.
    ///
    /// # Errors
    ///
    /// A unit the reads do not model (a computer player's).
    fn searcher(&mut self, memory: Memory<'_>) -> Result<crate::best_target::Searcher, GenieError>;

    /// [`crate::best_target::Battlefield::thing`].
    ///
    /// # Errors
    ///
    /// An object the slice does not hold.
    fn thing(&mut self, id: i32) -> Result<Option<crate::best_target::Thing>, GenieError>;

    /// [`crate::best_target::Battlefield::can_attack`].
    ///
    /// # Errors
    ///
    /// What the attack check does not model.
    fn can_attack(&mut self, id: i32) -> Result<bool, GenieError>;

    /// [`crate::best_target::Battlefield::attacks_neutral`].
    ///
    /// # Errors
    ///
    /// A unit-AI type whose test is not modelled.
    fn attacks_neutral(&mut self, class: i16) -> Result<bool, GenieError>;

    /// [`crate::best_target::Battlefield::spares_friends`].
    ///
    /// # Errors
    ///
    /// What the scan does not model.
    fn spares_friends(&mut self, id: i32) -> Result<bool, GenieError>;

    /// [`crate::best_target::Battlefield::shuns`].
    ///
    /// # Errors
    ///
    /// What the test does not model.
    fn shuns(&mut self, id: i32) -> Result<bool, GenieError>;

    /// [`crate::best_target::Battlefield::candidate`].
    ///
    /// # Errors
    ///
    /// What the reads do not model.
    fn candidate(
        &mut self,
        id: i32,
        distance: f32,
        category: i32,
    ) -> Result<crate::best_target::Candidate, GenieError>;

    /// [`crate::best_target::Battlefield::armored`].
    ///
    /// # Errors
    ///
    /// What the read does not model.
    fn armored(&mut self, id: i32) -> Result<bool, GenieError>;

    /// [`crate::best_target::Battlefield::weapon_range`].
    ///
    /// # Errors
    ///
    /// What the read does not model.
    fn weapon_range(&mut self, id: i32) -> Result<f32, GenieError>;

    /// [`crate::best_target::Battlefield::engages`].
    ///
    /// # Errors
    ///
    /// What the reads do not model.
    fn engages(&mut self, id: i32) -> Result<bool, GenieError>;

    /// [`crate::best_target::Battlefield::quick_path`].
    ///
    /// # Errors
    ///
    /// What the path does not model.
    fn quick_path(&mut self, id: i32, range: f32) -> Result<bool, GenieError>;
}

/// A search that picks no attack, asked about one ([`NoAttack`]).
const NO_ATTACK: GenieError =
    GenieError::unsupported("an attack test in a search that picks no attack");

/// An [`AttackPicker`] for searches that pick no attack: every read fails.
#[derive(Clone, Copy, Debug)]
pub struct NoAttack;

impl AttackPicker for NoAttack {
    fn searcher(&mut self, _: Memory<'_>) -> Result<crate::best_target::Searcher, GenieError> {
        Err(NO_ATTACK)
    }

    fn thing(&mut self, _: i32) -> Result<Option<crate::best_target::Thing>, GenieError> {
        Err(NO_ATTACK)
    }

    fn can_attack(&mut self, _: i32) -> Result<bool, GenieError> {
        Err(NO_ATTACK)
    }

    fn attacks_neutral(&mut self, _: i16) -> Result<bool, GenieError> {
        Err(NO_ATTACK)
    }

    fn spares_friends(&mut self, _: i32) -> Result<bool, GenieError> {
        Err(NO_ATTACK)
    }

    fn shuns(&mut self, _: i32) -> Result<bool, GenieError> {
        Err(NO_ATTACK)
    }

    fn candidate(
        &mut self,
        _: i32,
        _: f32,
        _: i32,
    ) -> Result<crate::best_target::Candidate, GenieError> {
        Err(NO_ATTACK)
    }

    fn armored(&mut self, _: i32) -> Result<bool, GenieError> {
        Err(NO_ATTACK)
    }

    fn weapon_range(&mut self, _: i32) -> Result<f32, GenieError> {
        Err(NO_ATTACK)
    }

    fn engages(&mut self, _: i32) -> Result<bool, GenieError> {
        Err(NO_ATTACK)
    }

    fn quick_path(&mut self, _: i32, _: f32) -> Result<bool, GenieError> {
        Err(NO_ATTACK)
    }
}

/// What a priest's conversion test (`FUN_140c1af00`, the priest's slot 0x1b8) reads of the
/// priest: its master's class and id, its faith (`+0x310`), its owner's attributes and its
/// master's tasks.
#[derive(Clone, Copy, Debug)]
pub struct Converter<'a> {
    /// Its master's class.
    pub class: i16,
    /// Its master's id.
    pub master: i16,
    /// Its faith (`+0x310`, what it holds).
    pub faith: f32,
    /// Its owner's attributes (`player+0x100`, count `+0x90`).
    pub attributes: &'a [f32],
    /// Its master's tasks (`+0x788`).
    pub tasks: &'a [crate::masters::Task],
}

/// What the conversion test reads of the object: its owner, its master's class, id and hero
/// flag (`+0x8d9`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Convertee {
    /// Its owner's id.
    pub owner: usize,
    /// Its master's class.
    pub class: i16,
    /// Its master's id.
    pub master: i16,
    /// Its master's hero flag (`hero_flag`, `+0x8d9`).
    pub hero: u8,
}

/// The faith a priest's conversion asks unless `any` (`DAT_14330ebcc`).
const FULL_FAITH: f32 = crate::image::unit_ai::FULL_FAITH;
/// The town centres a priest never converts (109, 618 to 621), the relic cart's master 159, the
/// farms and fish trap (50, 1187, 199).
const NEVER_CONVERTED: [i16; 9] = [109, 618, 619, 620, 621, 159, 50, 1187, 199];
/// The priest master that converts only boats (53).
const BOAT_CONVERTER: i16 = 53;

/// The priest's slot 0x1b8 (`FUN_140c1af00`, "CanConvert"): whether priest `p` may convert
/// object `t` (None: one the world does not hold), with `any` its faith not asked. Never for
/// class 43, nor without full faith (100, `COMISS`: a NaN fails) unless `any`; never a Gaia
/// object, nor with the owner's attribute 67 at most 0, nor livestock (58); master 53 converts
/// only the boats (2, 20, 21, 22, 53); never the [`NEVER_CONVERTED`] masters, walls (27), gates
/// (39) or flags (30); never a master whose hero flag forbids it (bit 0 or 1 set, but bit 7 with
/// bit 1). Then the priest's convert task (action type 104): its last for any class and master,
/// or the first for the object's master or class; one that names an attribute (`+0x26`) the
/// owner holds at most 0 (or not a number) fails.
#[must_use]
pub fn can_convert(p: &Converter<'_>, t: Option<&Convertee>, any: bool) -> bool {
    if p.class == 43 {
        return false;
    }
    // full faith (`COMISS`: a NaN has none)
    let faithful = FULL_FAITH <= p.faith;
    if !any && !faithful {
        return false;
    }
    let Some(t) = t else {
        return false;
    };
    if t.owner == 0 || p.attributes.get(67).is_some_and(|&a| a <= 0.0) || t.class == 58 {
        return false;
    }
    if p.master == BOAT_CONVERTER {
        return matches!(t.class, 2 | 20 | 21 | 22 | 53);
    }
    if NEVER_CONVERTED.contains(&t.master) || matches!(t.class, 27 | 39 | 30) {
        return false;
    }
    let hero = t.hero;
    let free = hero.trailing_zeros() >= 2 || ((hero as i8) < 0 && hero & 2 != 0);
    if !free {
        return false;
    }
    let mut task = None;
    for k in p.tasks.iter().filter(|k| k.action_type == 0x68) {
        if k.object_group == -1 && k.object_id == -1 {
            task = Some(k);
        }
        if k.object_id == t.master || k.object_group == t.class {
            task = Some(k);
            break;
        }
    }
    if let Some(k) = task
        && let Ok(i) = usize::try_from(k.attribute_type4)
        && let Some(&a) = p.attributes.get(i)
        && crate::fmath::at_most_or_unordered(a, 0.0)
    {
        return false;
    }
    true
}

/// `FUN_140c47f90` for a human player's unit (`me`): the farthest `bestUnitToAttack` looks, twice
/// its master's line of sight. A computer player's (by the game's difficulty and the player's
/// `+0x608`/`+0x60c` in a scenario) is not modelled.
///
/// # Errors
///
/// A computer player's unit.
pub const fn attack_sight(me: &Attacker) -> Result<f32, GenieError> {
    if me.computer {
        return Err(GenieError::unsupported(
            "a computer player's bestUnitToAttack",
        ));
    }
    Ok(me.los + me.los)
}

/// The unit AI's slot 0xa8 (`me`'s type, [`type_attacks_neutral`]) on a neutral object of
/// `class`.
#[must_use]
pub fn attacks_neutral(me: &Attacker, class: i16) -> bool {
    type_attacks_neutral(me.ai_type, me.attack_priority, class)
}

/// Slot 0xa8 of the unit AI of factory type `ai_type` (`FUN_141215560`'s modules), whether it
/// attacks a neutral object of `class`, its unit's master of `attack_priority`: the civilian's
/// (`FUN_140c1acf0`) and the building's (`FUN_140c1ace0`) never; the predator's
/// (`FUN_140c1ad60`) and the lion's (`FUN_140c1ad00`) not classes 13, 22, 52, 54 and 55; the
/// priest's ([`priest_attacks_neutral`]), the soldier's and the tower's (`FUN_140c1aeb0`, the
/// soldier's mask: [`soldier_attacks_neutral`]) and the siege unit's ([`siege_attacks_neutral`]);
/// the others' the base's (`FUN_140c48410`): always. The predator's and the lion's also take
/// attack priority 3 to [`PRIORITY_3_CLASSES`].
#[must_use]
pub fn type_attacks_neutral(ai_type: u32, attack_priority: u8, class: i16) -> bool {
    match ai_type {
        CIVILIAN | BUILDING_AI => false,
        PREDATOR_AI | LION_AI => {
            !matches!(class, 13 | 22 | 52 | 54 | 55)
                && (attack_priority != 3 || PRIORITY_3_CLASSES.contains(&class))
        }
        PRIEST_AI => priest_attacks_neutral(attack_priority, class),
        SOLDIER_AI | TOWER_AI => soldier_attacks_neutral(attack_priority, class),
        SIEGE_AI => siege_attacks_neutral(attack_priority, class),
        _ => true,
    }
}

/// The classes the priest never attacks as neutral objects (`FUN_140c1adc0`'s mask
/// `0x200800838001c` below class 50: 2, 3, 4, 19, 20, 21, 27, 39, 49).
const PRIEST_NEUTRAL_SPARED: u64 = 0x2_0080_0838_001c;

/// Slot 0xa8 of the priest's unit AI (`FUN_140c1adc0`): never a class below 50 of
/// [`PRIEST_NEUTRAL_SPARED`] (`class` taken as unsigned), and with attack priority 3 only
/// [`PRIORITY_3_CLASSES`].
#[must_use]
pub fn priest_attacks_neutral(attack_priority: u8, class: i16) -> bool {
    let class_bits = i32::from(class) as u32;
    if class_bits < 0x32 && PRIEST_NEUTRAL_SPARED >> class_bits & 1 != 0 {
        return false;
    }
    attack_priority != 3 || PRIORITY_3_CLASSES.contains(&class)
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

/// A computer player's soldier's or siege unit's idle check past its target: its target test
/// (`FUN_140c41690`) and its saved patrol's leash (`player+0x624`); not modelled.
const fn computer_idle_check() -> Result<(), GenieError> {
    Err(GenieError::unsupported(
        "a computer player's soldier's idle check",
    ))
}

/// A unit's back-up from a target under its least range (`FUN_140c43de0`, the soldier's and the
/// siege unit's case 0x1fe); not modelled.
const fn back_up() -> Result<u32, GenieError> {
    Err(GenieError::unsupported(
        "a soldier's target under its least range",
    ))
}

/// A defending unit's range on a patrol order or with a patrol path (slot 0x470, `FUN_140f95e10`),
/// which the soldier's and the siege unit's attack checks read; not modelled.
const fn defend_patrol_range() -> Result<(), GenieError> {
    Err(GenieError::unsupported(
        "a defending soldier's patrol range (slot 0x470, FUN_140f95e10)",
    ))
}

/// The classes the siege unit never attacks as neutral objects (`FUN_140c1ae10`'s mask
/// `0x200200800830001c`: 2, 3, 4, 20, 21, 27, 39, 49, 61; the soldier's but class 19).
const SIEGE_NEUTRAL_SPARED: u64 = 0x2002_0080_0830_001c;

/// Slot 0xa8 of the siege unit's AI (`FUN_140c1ae10`): never a class of
/// [`SIEGE_NEUTRAL_SPARED`] (`class` taken as unsigned), and with attack priority 3 only
/// [`PRIORITY_3_CLASSES`].
#[must_use]
pub fn siege_attacks_neutral(attack_priority: u8, class: i16) -> bool {
    let class_bits = i32::from(class) as u32;
    if class_bits < 0x3e && SIEGE_NEUTRAL_SPARED >> class_bits & 1 != 0 {
        return false;
    }
    attack_priority != 3 || PRIORITY_3_CLASSES.contains(&class)
}

/// The rams (masters 35, 422, 548 and 1258, raw `0x60bf`, `0x6242`, `0x62c0`, `0x6586`), which
/// by attack priority 0 attack only [`RAM_TARGETS`].
const RAM_MASTERS: [i16; 4] = [35, 422, 548, 1258];
/// What a ram attacks by its own choice: buildings, siege, walls, gates, farms, the packed
/// units, towers, the unpacked siege and the scorpions.
const RAM_TARGETS: [i16; 9] = [3, 13, 27, 39, 49, 51, 52, 54, 55];
/// What a trebuchet (class 51 or 54) attacks by its own choice: the rams' but siege and
/// scorpions.
const TREBUCHET_TARGETS: [i16; 7] = [3, 27, 39, 49, 51, 52, 54];
/// The siege masters whose current building target decides by attack priority alone (1744,
/// 1746, raw `0x676c`, `0x676e`).
const SIEGE_BUILDING_MASTERS: [i16; 2] = [1744, 1746];
/// A Gaia target lets the siege unit's range grow by this (`DAT_142eb4838`).
const GAIA_TARGET_REACH: f32 = crate::image::unit_ai::GAIA_TARGET_REACH;
/// The packed unit's class (51), which a human player's siege attack check passes over unless
/// asked for any target.
const PACKED_UNIT_CLASS: i16 = 51;

/// The defend stance's leash from a saved patrol's path (`DAT_14330ebb8`, 5).
const DEFEND_PATROL_LEASH: f32 = crate::image::unit_ai::DEFEND_PATROL_LEASH;
/// Stand ground's leash from a saved patrol's path (`DAT_142e7f384`, 0.5).
const STAND_GROUND_PATROL_LEASH: f32 = crate::image::unit_ai::STAND_GROUND_PATROL_LEASH;
/// `FUN_140f95e10`'s first least distance (`DAT_14330ebf4`, the largest f32).
const PATROL_FARTHEST: f32 = crate::image::unit_ai::PATROL_FARTHEST;

/// `FUN_14087e7c0`: the distance from `p` to the line through `a` and `b` (the cross product's
/// length over the direction's, its squares divided before `sqrtf`, in the routine's order).
/// Its flag whether `p` lies between the two (the dot products' signs) is not read here.
fn line_distance(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let [px, py, pz] = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
    let [dx, dy, dz] = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let c1 = py * dz - pz * dy;
    let c3 = px * dy - py * dx;
    let c2 = pz * dx - px * dz;
    let cross = c2 * c2 + c1 * c1 + c3 * c3;
    let length = dy * dy + dx * dx + dz * dz;
    crate::fmath::sqrt(cross / length)
}

/// `FUN_140f95e10`: the least distance from `point` to the lines through a path's consecutive
/// `points` ([`line_distance`]), from [`PATROL_FARTHEST`], each new one kept unless the least so
/// far is at most it (a NaN distance is kept, and the next one replaces it); a path of one point
/// answers [`PATROL_FARTHEST`].
///
/// # Errors
///
/// An empty path, whose loop the game runs past its end.
pub(crate) fn patrol_distance(points: &[Vec3], point: Vec3) -> Result<f32, GenieError> {
    if points.is_empty() {
        return Err(GenieError::unreachable("a saved patrol without points"));
    }
    let mut least = PATROL_FARTHEST;
    for pair in points.windows(2) {
        let d = line_distance(point, pair[0], pair[1]);
        least = if least <= d { least } else { d };
    }
    Ok(least)
}

/// `FUN_14118faf0`: the gap from an object at `at` with radii `radius` to `point`: each axis's
/// distance less the radius, `MAXSS` 0 (an unordered one is 0), then the length (`sqrtf`).
fn footprint_point_gap(at: [f32; 3], radius: [f32; 2], point: [f32; 2]) -> f32 {
    let gx = crate::fmath::maxss((point[0] - at[0]).abs() - radius[0], 0.0);
    let gy = crate::fmath::maxss((point[1] - at[1]).abs() - radius[1], 0.0);
    crate::fmath::sqrt(gy * gy + gx * gx)
}

/// The siege unit's attack check (slot 0xa0, `FUN_140c19190`) with `any` (its third argument):
/// never the classes 1 and 42, nor in `NoAttack`. Its reach is the target's footprint gap to the
/// unit's state position (`FUN_14118faf0`) less the unit's own diagonal (`sqrtf` of its radii
/// squared by `powf`), against its range (slot 0x230), grown by [`GAIA_TARGET_REACH`] for a Gaia
/// target: in `StandGround` (stance 2) but on order `0x2bd` within the range (`COMISS`, `JBE`:
/// an unordered one passes); in Defend (1) within the range (ungrown) plus 5
/// (`DAT_14330ebb8`; `JA` rejects); the patrol orders' and a patrol path's ranges (slot 0x470,
/// `FUN_140f95e10`) are not modelled. Then a Gaia unit only of the combat type with `+0x924` 0
/// outside the scenario modes; never a building no builder has started (`+0xc60`); never from a
/// unit below state 2; not `any`, never from a human player's packed unit (51); never while its
/// top action packs or unpacks (124, 125). Not `any`, by attack priority 0 a ram attacks only
/// [`RAM_TARGETS`] and a trebuchet (classes 51, 54) [`TREBUCHET_TARGETS`]; masters 1744 and 1746
/// with a building as their current target ([`Seeker::target_kind`]) by their priority alone (1
/// or 4 and up); attack priority 3 only [`PRIORITY_3_CLASSES`].
///
/// # Errors
///
/// A defending unit's patrol range.
pub fn siege_can_attack(
    me: &Attacker,
    s: &Seeker,
    t: &Target,
    any: bool,
) -> Result<bool, GenieError> {
    if matches!(t.class, 1 | 42) || s.stance == 3 {
        return Ok(false);
    }
    // `powf` with 2 (`DAT_142e8c150`), y's first
    let diagonal = crate::fmath::sqrt(
        crate::fmath::powf(me.radius[1], 2.0) + crate::fmath::powf(me.radius[0], 2.0),
    );
    let gap = footprint_point_gap(t.position, t.radius, s.state_position) - diagonal;
    let mut range = me.range;
    if t.owner == 0 {
        range += GAIA_TARGET_REACH;
    }
    match s.stance {
        2 if s.order != DEFEND_ORDER && !crate::fmath::at_most_or_unordered(gap, range) => {
            return Ok(false);
        }
        1 => {
            if matches!(s.order, 0x2c7 | 0x2dd) || s.patrol_path {
                defend_patrol_range()?;
            }
            if me.range + DEFEND_GROWTH < gap {
                return Ok(false);
            }
        }
        _ => {}
    }
    let takes_by_sight = !me.scenario && 70 <= me.kind && me.capture == 0;
    if me.player == 0 && !takes_by_sight {
        return Ok(false);
    }
    if (t.kind == 80 && t.desolid) || me.state < 2 {
        return Ok(false);
    }
    if !me.computer && !any && me.class == PACKED_UNIT_CLASS {
        return Ok(false);
    }
    if matches!(me.top_action, Some(124 | 125)) {
        return Ok(false);
    }
    if any {
        return Ok(true);
    }
    let priority = me.attack_priority;
    if priority == 0 {
        if RAM_MASTERS.contains(&me.master) {
            return Ok(RAM_TARGETS.contains(&t.class));
        }
        if matches!(me.class, 51 | 54) {
            return Ok(TREBUCHET_TARGETS.contains(&t.class));
        }
    }
    if SIEGE_BUILDING_MASTERS.contains(&me.master) && s.target != -1 && s.target_kind == Some(80) {
        return Ok(priority == 1 || priority & 0xfc != 0);
    }
    Ok(priority != 3 || PRIORITY_3_CLASSES.contains(&t.class))
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
    fn closest(&mut self, search: &Search<'_>) -> Result<Found, GenieError>;

    /// The unit's attack command on object `target` (slot 0x338, `FUN_14120cea0`), forced or
    /// not, while its unit AI stands as `ai`: true when the unit attacks it, with the notifies
    /// the command posts to the unit AI (kind and parameters), which the AI queues.
    ///
    /// # Errors
    ///
    /// What the command does not model.
    fn attack(&mut self, target: i32, force: bool, ai: &UnitAi) -> Result<Attacked, GenieError>;

    /// The unit's `+0x685`, `underAttackValue`, becomes `value` (`FUN_14119d2d0`).
    fn set_under_attack(&mut self, value: bool);

    /// The unit's stopAction (slot 0x3c0) as the unit AI calls it, ahead of a command that
    /// follows it while the AI processes its notifies (an [`Effect::Stop`] otherwise waits for
    /// the AI's update to end).
    ///
    /// # Errors
    ///
    /// What the stop does not model.
    fn stop(&mut self) -> Result<(), GenieError>;

    /// `FUN_140c41e50`'s cap on the unit's last human order time (`+0x650`, [`Effect::CapHumanOrder`])
    /// run at once, ahead of a stop that runs at once ([`UnitSide::stop`]).
    ///
    /// # Errors
    ///
    /// A side without its unit.
    fn cap_human_order(&mut self, _cap: u32) -> Result<(), GenieError> {
        Err(WITHOUT_UNIT)
    }

    /// `FUN_141658e20` from the unit's place: the drop-site table's nearest building of the
    /// owner for what the unit carries (`+0x308`) from its class; None without one.
    ///
    /// # Errors
    ///
    /// A side without its unit.
    fn table_drop_site(&mut self) -> Result<Option<i32>, GenieError> {
        Err(WITHOUT_UNIT)
    }

    /// The unit's build command (slot 0x380, forced) on `foundation` run at once
    /// ([`Effect::BuildObject`]), ahead of a command that follows it in the same AI run (an
    /// attack command replacing the Build it makes).
    ///
    /// # Errors
    ///
    /// A side without its unit, and what the build command does not model.
    fn build_now(&mut self, _foundation: i32) -> Result<(), GenieError> {
        Err(WITHOUT_UNIT)
    }

    /// `HuntObject`'s unit part run at once ([`Effect::HuntObject`]): its stopAction, then its
    /// hunt command on `target` (slot 0x360, forced), ahead of a command that follows it in the
    /// same AI run; `ai` is the unit AI as it stands.
    ///
    /// # Errors
    ///
    /// A side without its unit, and what the hunt command does not model.
    fn hunt_now(&mut self, _target: i32, _ai: &UnitAi) -> Result<(), GenieError> {
        Err(WITHOUT_UNIT)
    }

    /// The unit's slot 0x340 on a move its unit AI gave ([`Effect::Move`]) run at once, ahead of
    /// a command that follows it in the same AI run.
    ///
    /// # Errors
    ///
    /// A side without its unit, and what the move does not model.
    fn ai_move_now(&mut self, _m: AiMove) -> Result<(), GenieError> {
        Err(WITHOUT_UNIT)
    }

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
    ) -> Result<Option<(i16, [f32; 3])>, GenieError>;

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
    ) -> Result<Option<[i32; 2]>, GenieError>;

    /// The auto scout's last search (`FUN_140c4b130`, [`crate::explore::beyond_sight`]), drawing
    /// from the AI stream (`rng`); None when it finds no point.
    ///
    /// # Errors
    ///
    /// What the search does not model.
    fn seek_beyond_sight(&mut self, rng: &mut Well512) -> Result<Option<[i32; 2]>, GenieError>;

    /// The player's scout map, made when the auto scout's last search found nothing
    /// (`FUN_140c589c0`'s case 0x202).
    ///
    /// # Errors
    ///
    /// A unit without its player's sight.
    fn make_scout_map(&mut self) -> Result<(), GenieError>;

    /// The unit's slot 0x468 (`FUN_141157c50`, `getPatrolInformation`): a copy of its top path
    /// (`FUN_140f960f0`; `FUN_140f95c00` into a new path: its points, its length, its flags) and
    /// the waypoint of its top level (`+0x24`); None without one.
    fn patrol_information(&self) -> Option<(PatrolPath, i32)>;

    /// The builder's search for another foundation (`FUN_140c1c7c0`): −1 for none.
    ///
    /// # Errors
    ///
    /// What the search does not model.
    fn foundation_search(&mut self) -> Result<i32, GenieError> {
        Err(GenieError::unsupported(
            "a builder's search without the world",
        ))
    }

    /// Slot 0x3d0: whether the unit reaches object `target` within `range`.
    ///
    /// # Errors
    ///
    /// What the reach test does not model.
    fn reaches(&mut self, _target: i32, _range: f32) -> Result<bool, GenieError> {
        Err(GenieError::unsupported("a reach test without the world"))
    }

    /// What the base handler's case 0x216 reads ([`OffFootprint`]) for a unit sent off the
    /// footprint of `site` (its master's radii `radius`).
    ///
    /// # Errors
    ///
    /// What the spot's search does not model.
    fn off_footprint(&mut self, _site: i32, _radius: [f32; 2]) -> Result<OffFootprint, GenieError> {
        Err(GenieError::unsupported(
            "a unit sent off a footprint without the world",
        ))
    }

    /// The unit's slot 0x278: whether it could stand at `point`.
    ///
    /// # Errors
    ///
    /// What the test does not model.
    fn can_stand(&mut self, _point: [f32; 2]) -> Result<bool, GenieError> {
        Err(GenieError::unsupported("a stand test without the world"))
    }

    /// `AskForHelp`'s walk (`FUN_140c42ce0`) of the lists its search left of the unit's player's
    /// own objects and its friends' (`DAT_143f69180`, the relation kinds 1 and 2, each in its
    /// order) for helpers against `attacker`: the unit itself, an object without a unit AI or
    /// owner, a dead one (state 3 and up), one of the classes 1 to 3, 18 to 21, 27 and 39
    /// (`0x80083c000e`), the siege tower master 1105 (`FUN_141194f10`), a villager (class 4)
    /// against an attacker that is not Gaia's, one of a player that does not count the unit's
    /// as an ally (`FUN_141663d90` with 1) and one farther than 3 tiles (the squared footprint
    /// gap, `FUN_14118f8c0`, above 9) do not help. These tests read nothing the helper's attack
    /// check (slot 0xa0) between them changes.
    ///
    /// # Errors
    ///
    /// A helper that passes them ([`helper_answers`]).
    fn call_for_help(&mut self, _attacker: i32) -> Result<(), GenieError> {
        Err(GenieError::unsupported("a call for help without the world"))
    }

    /// The object `id` as the unit AI reads it ([`UnitSeen`]): None when the world does not
    /// hold it (`FUN_1415417a0`).
    ///
    /// # Errors
    ///
    /// An object the slice does not hold.
    fn seen_unit(&mut self, _id: i32) -> Result<Option<UnitSeen>, GenieError> {
        Err(GenieError::unsupported(
            "an object's look without the world",
        ))
    }

    /// What a computer player's unit weighs of object `other` (its current target or its
    /// attacker) on a damage notify ([`Duel`], [`UnitAi::damaged`]).
    ///
    /// # Errors
    ///
    /// An object the side does not hold, and a side without the world.
    fn duel(&mut self, _other: i32) -> Result<Duel, GenieError> {
        Err(GenieError::missing_data("a duel without the world"))
    }

    /// The villager's test of its attacker on a damage notify: the attacker shows too little
    /// attack, armor and range to run from, and is fought back as Gaia's are
    /// ([`UnitAi::villager_damaged`]).
    ///
    /// # Errors
    ///
    /// An object the side does not hold, and a side without the world.
    fn harmless(&mut self, _attacker: i32) -> Result<bool, GenieError> {
        Err(GenieError::missing_data(
            "an attacker's shown values without the world",
        ))
    }

    /// The attacker of object `target` nearest to it (its unit AI's slot 0x78, `FUN_140c48610`:
    /// of its attackers in their order, those the world holds, the least footprint gap,
    /// `FUN_14118f680`, the first of a tie), −1 for none; None when `target` holds no unit AI
    /// ([`UnitAi::defend_misc`]).
    ///
    /// # Errors
    ///
    /// A side without the world.
    fn defended_attacker(&mut self, _target: i32) -> Result<Option<i32>, GenieError> {
        Err(GenieError::missing_data(
            "a defended unit's attackers without the world",
        ))
    }

    /// The unit AI's `ClosestResourceObject` (slot 0x80, `FUN_140c49760`) as `search` asks it:
    /// what it found, None for nothing.
    ///
    /// # Errors
    ///
    /// What the search does not model.
    fn closest_resource(
        &mut self,
        _search: ResourceSearch,
    ) -> Result<Option<Gatherable>, GenieError> {
        Err(GenieError::unsupported(
            "a resource search without the world",
        ))
    }

    /// The class of master `master` of player `player` (`FUN_14152fe40`, the player's master
    /// table `+0x28`): None for a player or master the game does not hold.
    ///
    /// # Errors
    ///
    /// A side without the players' masters.
    fn master_class(&mut self, _player: i32, _master: i32) -> Result<Option<i16>, GenieError> {
        Err(GenieError::unsupported(
            "a master's class without the world",
        ))
    }

    /// The unit's slot 0x388 (`FUN_141226fc0`) on object `target`: when the world holds it
    /// (`FUN_1415417a0`) and the unit's slot 0x820 finds a task on it (any action type, with the
    /// master switch), a new `TRIBE_Action_Wait_Solidify` on that task
    /// ([`crate::wait_solidify`]) replaces the unit's actions (`FUN_140a98620`, slot 0x808);
    /// true then.
    ///
    /// # Errors
    ///
    /// What the task search does not model.
    fn wait_solidify(&mut self, _target: i32) -> Result<bool, GenieError> {
        Err(GenieError::unsupported(
            "a wait for a foundation without the world",
        ))
    }

    /// Slot 400 (`FUN_14106d090`) as the full update's tail reads it: the target of the unit's
    /// top action as it stands now (`FUN_140a78450`: an object the world holds), the commands the
    /// unit took inside the update included (its attack command, [`UnitSide::attack`]); None
    /// without one. A side without a unit (the searches' closures, the tests' sides) has no
    /// action list.
    ///
    /// # Errors
    ///
    /// An object the slice does not hold.
    fn top_target_now(&mut self) -> Result<Option<i32>, GenieError> {
        Ok(None)
    }

    /// What the full update's tail tests of object `id`, the target of its unit's top action
    /// ([`TargetFlags`]); None when the world does not hold it. A side without the world (the
    /// searches' closures, the tests' sides) holds no object.
    ///
    /// # Errors
    ///
    /// An object the slice does not hold.
    fn target_flags(&mut self, _id: i32) -> Result<Option<TargetFlags>, GenieError> {
        Ok(None)
    }

    /// A command of the unit's that the unit AI's order slots give ([`UnitCommand`]): true when
    /// the unit takes it (the slot's 1).
    ///
    /// # Errors
    ///
    /// The commands the slice holds no action for.
    fn command(&mut self, _command: UnitCommand) -> Result<bool, GenieError> {
        Err(UNMODELLED_COMMAND)
    }

    /// The soldier's `AttackObject` (`FUN_140c17550`) after the attack, for a unit whose group
    /// the world holds: its future path data goes (slot 0x550, `FUN_14114d6a0`), and a unit
    /// that moves (slot 0x5c8) or speeds up (slot 0x5a0, its velocity data's forward
    /// acceleration above 0) stops (slot 0x5d0).
    ///
    /// # Errors
    ///
    /// What the stop does not model.
    fn group_attack_stop(&mut self) -> Result<(), GenieError> {
        Err(GenieError::unsupported(
            "a soldier's attack in a group without its unit",
        ))
    }
}

/// A unit AI's call on its unit from a side that holds none (a search's closure).
const WITHOUT_UNIT: GenieError =
    GenieError::unsupported("a unit AI's call on its unit from a side without it");

/// A unit command the slice holds no action for ([`UnitSide::command`]).
pub(crate) const UNMODELLED_COMMAND: GenieError = GenieError::unsupported(
    "a unit command the slice does not model (heal, conversion, enter, transport, unload)",
);

/// The unit's commands the unit AI's order slots give ([`UnitSide::command`]), each the unit's
/// slot of `TRIBE_Combat_Object`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UnitCommand {
    /// The heal of an object (slot 0x108, `FUN_141214040`), forced.
    Heal(i32),
    /// The conversion of an object (slot 0x370, `FUN_141210510`), forced.
    Convert(i32),
    /// The repair of an object (slot 0x378, `FUN_141217c00`), forced.
    Repair(i32),
    /// The enter into an object (slot 0x3a0), forced.
    Enter(i32),
    /// The transport to a point (slot 0x3b8, `FUN_14108ce60`), forced.
    Transport([f32; 3]),
    /// The unload at a point at the unit's height (slot 0x3a8, `FUN_14108d050`), or over a wall
    /// (slot 0x3b0, `FUN_14108d560`), forced.
    Unload {
        /// The point.
        to: [f32; 3],
        /// Over a wall.
        over_wall: bool,
    },
}

/// What the full update's tail (`0x140c66b10` after the look-around check) tests of the target of
/// its unit's top action ([`UnitSide::target_flags`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TargetFlags {
    /// Its owner's id (`+0x138`, `player+0x718`); Gaia is 0.
    pub owner: usize,
    /// Its type (`+0xf`).
    pub kind: u8,
    /// `capture_flag` (`+0x924`, `FUN_14107cc90`).
    pub capture: u8,
    /// `+0x790`, `parent_pasture_id`: the pasture it is in, −1 for none.
    pub pasture: i32,
    /// `+0x689`, `mUnitTargetingFlag`, is 0 (`FUN_141188bb0`).
    pub untargetable: bool,
    /// `+0x68a`, `unit_attackable_flag` (`FUN_141188b10`).
    pub attackable: bool,
}

/// What the unit AI asks `ClosestResourceObject` (slot 0x80, `FUN_140c49760`;
/// [`UnitSide::closest_resource`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResourceSearch {
    /// The gather group (`param_2`).
    pub kind: u8,
    /// Fewer workers than this (`param_4`; −1 any).
    pub cap: i32,
    /// The drop site the search starts from (`param_3`; −1 none).
    pub site: i32,
    /// The unit AI's current target (`+0x48`), which the search passes over.
    pub current: i32,
    /// The point the search starts from without a site (`param_7`, `param_8`; (−1, −1) for the
    /// unit's place).
    pub point: [f32; 2],
    /// Below this state the unit finds nothing (`param_6`; its −1 is 0 here).
    pub min_state: u8,
}

/// [`ResourceSearch::point`]'s (−1, −1): the unit's own place (`DAT_14330ec20`).
pub const NO_POINT: [f32; 2] = [-1.0, -1.0];

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

impl<F: FnMut(&Search<'_>) -> Result<Found, GenieError>> UnitSide for F {
    fn closest(&mut self, search: &Search<'_>) -> Result<Found, GenieError> {
        self(search)
    }

    fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
        Err(GenieError::unsupported(
            "a unit AI's attack without its unit",
        ))
    }

    fn set_under_attack(&mut self, _: bool) {}

    fn stop(&mut self) -> Result<(), GenieError> {
        Err(WITHOUT_UNIT)
    }

    fn may_attack(&mut self, _: i32, _: &Seeker) -> Result<Option<(i16, [f32; 3])>, GenieError> {
        Err(GenieError::unsupported("an attack test without the world"))
    }

    fn seek_unexplored(
        &mut self,
        _: &UnitAi,
        _: &mut Well512,
    ) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported(
            "an explorer's search without its unit",
        ))
    }

    fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
        Err(GenieError::unsupported(
            "an explorer's search without its unit",
        ))
    }

    fn make_scout_map(&mut self) -> Result<(), GenieError> {
        Err(GenieError::unsupported("a scout map without the world"))
    }

    /// A side of searches alone stands for no unit with movement state, whose slot 0x468 (the
    /// static object's) holds no path.
    fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
        None
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
    /// `ClosestObject`'s count of the players but Gaia whose objects the lists read hold
    /// (`DAT_143e6f92c`, `numberDifferentPlayerObjectsVisible`); 0 from the other picks, which
    /// do not count.
    pub players: u32,
    /// What `ClosestObject` writes to its seventh argument: the object's search distance,
    /// truncated (`CVTTSS2SI`); −1 (`DAT_14330ec20`) without one and from the other picks.
    pub distance: i32,
    /// The retarget entries `bestUnitToAttack`'s failed paths add (`FUN_140c42320`: a target and
    /// the world time until which it is ignored; at most three), the unit AI's.
    pub retries: [Option<(i32, u32)>; 3],
}

impl Found {
    /// Nothing found, no enemies.
    pub const NONE: Self = Self {
        id: -1,
        enemies: false,
        position: None,
        fresh: true,
        players: 0,
        distance: -1,
        retries: [None; 3],
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
pub const fn no_search(_: &Search<'_>) -> Result<Found, GenieError> {
    Err(GenieError::unsupported(
        "a unit AI's search the caller does not answer",
    ))
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
    /// Its hit points (`+0x278`).
    pub hp: f32,
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
    /// The unit's slot 0x538 with 9 (`FUN_14106d0c0`): its action list holds an Attack (type
    /// 9); read with [`Surroundings::action_state`], which tells whether the slice holds the
    /// list.
    pub attack_listed: bool,
    /// The objects the unit AI names besides its target, as the world's object table holds them:
    /// its attackers ([`UnitAi::attackers`]) and the callers of its queued damage notifies. One
    /// the table does not hold is absent.
    pub units: Vec<UnitSeen>,
    /// The unit's group (`+0x6d0`, the world's `FUN_14152ff40`), when the world holds it: what
    /// the full update reads of it ([`GroupState`]).
    pub group: Option<GroupState>,
    /// Slot 400 (`FUN_14106d090`): its top action's target (`FUN_140a78450`) as the world holds
    /// it; None without one.
    pub top_target: Option<TopTarget>,
    /// Slot 0x240 (`FUN_1410863c0`): its master's least range (`+0x850`).
    pub min_range: f32,
    /// The unit's `+0x7fb`, `mAttackMoving`.
    pub attack_moving: bool,
    /// Where the unit last moved to (slot 0x520: `+0x824`), as the game's vector (x, height,
    /// y); an object without movement state, its place.
    pub moved_at: Vec3,
    /// Its owner's `chance_to_dodge_missiles` (`player+0xe10`, saved): a ship's handler takes a
    /// missile's warning (`0x20f`) as damage unless a draw below it dodges.
    pub dodge_chance: u8,
    /// Its owner's `chance_for_archers_to_maintain_distance` (`player+0xe11`, saved): a computer
    /// player's siege unit backs up after a shot on a draw at least this.
    pub keep_distance_chance: u8,
    /// Its owner's `disableTowerPriority` (`player+0x620`, GAME 0xb's setting 0xe): a computer
    /// player's unit but a siege weapon, unpacked trebuchet or warship ignores a tower's damage
    /// ([`UnitAi::damaged`]).
    pub disable_tower_priority: bool,
    /// An object inside it (`+0x2f8`) belongs to another owner that is a computer player: a
    /// building's or tower's damage alert (0x201) reaches that owner's AI too (its slot 0x2e8,
    /// then slot 0x3c8, [`alert_owner`]).
    pub guest_computer: bool,
}

/// The target of a unit's top action as its unit AI reads it ([`Surroundings::top_target`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TopTarget {
    /// `+8`.
    pub id: i32,
    /// Its type (`+0xf`).
    pub kind: u8,
    /// Its place.
    pub position: [f32; 3],
    /// Its master's radii.
    pub radius: [f32; 2],
}

/// Slot 400 within the unit's range (slot 0x1e8, `FUN_141086330`): its top action's target when
/// the footprints' squared gap (`FUN_14118f8c0`) is at most the weapon range squared and at
/// least the least range squared (`COMISS`/`JC` each: a NaN is not in range).
fn top_target_in_range(at: &Surroundings) -> Option<i32> {
    let t = at.top_target?;
    let gap = crate::movement::footprint_gap_squared(at.position, at.size, t.position, t.radius);
    let in_range = gap <= at.weapon_range * at.weapon_range && at.min_range * at.min_range <= gap;
    in_range.then_some(t.id)
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
    /// Its hit points (`+0x278`).
    pub hp: f32,
    /// Its master's id and type (`+0xf`).
    pub master: i16,
    /// Its type (`+0xf`).
    pub kind: u8,
    /// An annex's main building (`+0xbb8`).
    pub main: Option<i32>,
    /// The unit's owner counts its owner as its own or an ally (`FUN_141663d90` with the ally
    /// flag, [`crate::tasks::allied`]).
    pub allied: bool,
}

/// What a computer player's unit weighs of an object on a damage notify ([`UnitAi::damaged`]):
/// its current target's or its attacker's threat and how long it would take to kill.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Duel {
    /// The object may convert the unit (its unit AI's slot 0x1b8 with the unit and 1: a priest's
    /// [`can_convert`], any other's 0).
    pub converts: bool,
    /// Its attacks' sum (slot 0x228, `FUN_141084070`: its master's attacks above 0).
    pub attacks: f32,
    /// What it deals the unit per hit (the unit's slot 0x220 with it, `FUN_141083f20`).
    pub harms: f32,
    /// What the unit deals it per hit (its slot 0x220 with the unit).
    pub harmed: f32,
    /// Its hit points (`+0x278`).
    pub hp: f32,
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
    /// The viewer's test of its tile (`FUN_1416635e0`, [`crate::spatial::Spatial::tile_sight`]):
    /// 0xf seen, 0x80 explored, else 0.
    pub sight: u8,
}

/// The class whose busy units skip the idle check (`-0xe98` raw).
const CLASS_20: i16 = 20;
/// The class of the attackers and targets a villager's damage notify looks at (`-0xea2` raw).
const CLASS_10: i16 = 10;
/// The class whose stop leaves the state position (`-0xe9a` raw, slot 0xb0).
const CLASS_18: i16 = 18;
/// A patrol's path goes back and forth when its point is at least this far from the unit
/// (`0x140aa2a70`, `DAT_142eb1c98`).
const PATROL_TURN_REACH: f32 = crate::image::unit_ai::PATROL_TURN_REACH;
/// The patrol orders, the patrol's (`0x2c7`) and the attack move's (`0x2dd`), whose ends reset
/// the idle and secondary timers.
pub(crate) const PATROL: i32 = 0x2c7;
pub(crate) const ATTACK_MOVE: i32 = 0x2dd;
/// Classes whose actions `0x26b`, `0x26c` take another branch of the idle part (`-0xea9`,
/// `-0xe78` raw).
const CLASS_3: i16 = 3;
/// A building's master type (`'P'`, `FUN_1411942c0`).
const BUILDING_KIND: u8 = 80;
/// The combat type (`0x46`), from which on the full update's tail reads a target's `+0x924`.
const COMBAT_KIND: u8 = 70;

/// Whether the effect replaces or ends the unit's actions.
const fn changes_actions(e: &Effect) -> bool {
    !matches!(
        e,
        Effect::CapHumanOrder(_) | Effect::AfterOrder(_) | Effect::ChangeOwner(_)
    )
}

/// Slot 400 as the full update's tail reads it: the target of the unit's top action after the
/// [`Effect`]s pending in `fx`, the last that replaces its actions deciding (a stop leaves none,
/// slot 0x3c0; a move to a point, slots 0x340, 0x148, `restorePatrolInformation`, or an explore,
/// slot 0x398, an action without an object; a move to an object, slot 0x348, holds it, as do a
/// hunt, gather and build, slots 0x360, 0x368, 0x380); without one the unit's now
/// ([`UnitSide::top_target_now`]).
fn top_target_after(fx: &[Effect], side: &mut dyn UnitSide) -> Result<Option<i32>, GenieError> {
    for e in fx.iter().rev() {
        match *e {
            Effect::Stop
            | Effect::Move(_)
            | Effect::MoveAside(_)
            | Effect::Explore(_)
            | Effect::RestorePatrol => return Ok(None),
            Effect::MoveToObject { target, .. }
            | Effect::HuntObject(target)
            | Effect::GatherObject(target)
            | Effect::BuildObject(target)
            | Effect::DropElsewhere { site: target, .. }
            | Effect::WorkOn { target, .. } => return Ok(Some(target)),
            Effect::CapHumanOrder(_)
            | Effect::OrderOnTarget(_)
            | Effect::AfterOrder(_)
            | Effect::ChangeOwner(_) => {}
        }
    }
    side.top_target_now()
}

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

/// The engine's error ([`genie_error::GenieError`]), here for the modules' imports.
pub use crate::GenieError;

/// What slot 73 does for a factory type.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Retarget {
    /// The base slot 73: the retarget check.
    Base,
    /// An override that never reaches the check: livestock (19, `0x140c21f20`: 9).
    Never,
    /// The villagers' override (8, `0x140c21da0`, [`UnitAi::civilian_misc`]), which never
    /// reaches the check either.
    Civilian,
    /// The artifact's override (14, `FUN_140c21d40`, [`UnitAi::artifact_misc`]): its
    /// processIdle, no check.
    Artifact,
    /// The priest's override (9, `FUN_140c21f30`): the base's check but on the order on a target
    /// (`0x2db`) and for a computer player's priest, which return 9.
    Priest,
}

const fn retarget_kind(ai_type: u32) -> Result<Retarget, GenieError> {
    match ai_type {
        // villagers (0x140c21da0) and livestock (0x140c21f20) return 9 from slot 73
        8 => Ok(Retarget::Civilian),
        19 => Ok(Retarget::Never),
        ARTIFACT_AI => Ok(Retarget::Artifact),
        PRIEST_AI => Ok(Retarget::Priest),
        // hunted animal (2, slot 72 FUN_140c1eea0), boar (4, slot 72 FUN_140c1e630), lion (5,
        // slot 72 FUN_140c1fb70), trade ship (0xa, slot 72 FUN_140c21930), transport ship (11,
        // slot 72 0x140c21d20), fishing ship (0xc, slot 72 FUN_140c1eab0), soldier (13, slot 72
        // FUN_140c20db0), tower (15, the base's slot 72 FUN_140c564c0), building (17, slot 72
        // 0x140c1e600, which does nothing), siege unit (0x10, slot 72 FUN_140c206b0), siege tower
        // (0x12, slot 72 FUN_140c20690)
        2 | 4 | 5 | TRADE_AI | 11 | FISHING_AI | 13 | 15 | SIEGE_AI | 17 | SIEGE_TOWER_AI => {
            Ok(Retarget::Base)
        }
        _ => Err(GenieError::unsupported("unit-AI type not modelled")),
    }
}

/// The base idle timeout (`+0x168`) the constructor sets; the adjusted one is drawn around it.
const IDLE_TIMEOUT: u32 = 3000;
/// The tower's idle timeout (`FUN_140c12960`'s `FUN_140c41bc0` with 1000).
const TOWER_IDLE_TIMEOUT: u32 = 1000;
/// The artifact's idle timeout (`FUN_140c11340`'s `FUN_140c41bc0` with 5000).
const ARTIFACT_IDLE_TIMEOUT: u32 = 5000;
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

/// A villager's notify, or a part of one, the slice does not model.
const VILLAGER_NOTIFY: GenieError =
    GenieError::unsupported("a villager's notify the slice does not model");

/// The shepherd's action value (`0x278`), whose search the villager's case 0x202 runs.
const SHEPHERD_ACTION: i32 = 0x278;

/// A villager's damage notify that reads its animation speed (slot 0x1f8) while the slice does
/// not know it ([`Surroundings::anim_speed`]).
const NO_ANIM_SPEED: GenieError =
    GenieError::unsupported("a moving villager's damage notify without its animation speed");
/// The follow order (`0x2c8`, "I have a follow order for Unit #%d" in slot 73) and its action
/// value (`0x264`), which FOLLOW gives.
pub const FOLLOW_ORDER: i32 = 0x2c8;
/// The follow action value (`0x264`).
pub const FOLLOW_ACTION: i32 = 0x264;
/// The defend order (`0x2bd`, "I have a defend order for Unit #%d" in slot 73).
pub const DEFEND_ORDER: i32 = 0x2bd;
/// The defend's action value (`0x259`, slots 0x170 and 0x178).
const DEFEND_ACTION: i32 = 0x259;
/// The target point's x that marks it unset (`DAT_14330ec20`, −1), which the defend order fills.
const INVALID_POINT: f32 = crate::image::unit_ai::INVALID_POINT;
/// What the defend order adds to its desired distance before it follows (`DAT_14330eb8c`, 1).
const DEFEND_SLACK: f32 = crate::image::unit_ai::DEFEND_SLACK;
/// The master whose unit takes no defend order (slot 0x230's case 0x2bd: raw `0x611c`).
const DEFENDLESS_MASTER: i16 = 128;
/// The run away order (`0x2c3`, "I have a runAway order").
const RUN_AWAY_ORDER: i32 = 0x2c3;
/// The repair order (`0x2ce`).
const REPAIR_ORDER: i32 = crate::ai_kinds::ORDER_REPAIR;
/// The second value of the order on a target (`0x325`), which shares `0x2db`'s case: the way
/// back to work after a drop-off (`FUN_140c41ab0`).
pub(crate) const ORDER_ON_TARGET_2: i32 = 0x325;
/// The heal's action value (`0x25b`, slot 0xe0).
const HEAL_ACTION: i32 = 0x25b;
/// The conversion's action value (`0x25c`, slot 0xd8).
const CONVERT_ACTION: i32 = 0x25c;
/// The repair's action value (`0x26a`, slot 0xe8).
const REPAIR_ACTION: i32 = 0x26a;
/// The transport's action value (`0x266`, slot 0x128).
const TRANSPORT_ACTION: i32 = 0x266;
/// The unload's action value (`0x26d`, slot 0x118).
const UNLOAD_ACTION: i32 = 0x26d;
/// The unload over a wall's action value (700, slot 0x120).
const UNLOAD_OVER_WALL_ACTION: i32 = 700;
/// Seek and destroy's action value (`0x26e`, slot 0x180).
const SEEK_ACTION: i32 = 0x26e;
/// Explore and destroy's action value (`0x26f`, slot 0x188).
const EXPLORE_DESTROY_ACTION: i32 = 0x26f;
/// The wait order's action value (`0x274`, slot 0x230's case 0x2d8).
const WAIT_ACTION: i32 = 0x274;
/// The scout order's action value (`0x276`, after slot 0x100).
const SCOUT_ACTION: i32 = 0x276;
/// The auto boar luring's action value (`0x2be`, after slot 0xc8).
const BOAR_LURING_ACTION: i32 = 0x2be;
/// The trebuchet's classes (`-0xe79`, `-0xe76` raw: packed and unpacked), whose attack move on
/// a target attacks it.
const TREBUCHET_CLASS: i16 = 51;
const TREBUCHET_PACKED_CLASS: i16 = 54;
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
/// The retarget's reference distance about a current target (`DAT_142eb9430`).
const RETARGET_REACH: f32 = crate::image::unit_ai::RETARGET_REACH;

/// What the retarget's search reads of its checks ([`UnitAi::retarget_outcome`]): the current
/// target is a building's, wall's or trebuchet's (or one off the attack order), and the top
/// action is in an attack's work state (6 or 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RetargetSearch {
    building: bool,
    work_state: bool,
}

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
fn adjusted_idle_timeout(base: u32, ai: &mut Well512) -> u32 {
    ai.rand15() % (base >> 1) + ((base * 3) >> 2)
}

/// The unit-AI type the factory (`FUN_141215a90`) gives a new object of a master of `class`
/// with the volley missile `missile` (`+0x8d4`, −1 for none) and the combat level
/// `combat_level` (`+0x62c`). The types are those `FUN_141215560`'s switch names for its
/// constructors: masters 1105 (`FUN_140c11fb0`, 0x12) and 1744, 1746 (`FUN_140c12040`, 0x10) by
/// their id; then none for classes 5, 7, 8, 11, 14, 15, 16, 30 to 34, 38, 42, 48 and 63; 0xe for
/// class 1; 0xa for 2 and 19; the building's (0x11) for classes 3, 27, 39, 49 and 60 without a
/// volley missile, the tower's (0xf) for those with one and for 52; the civilian's (8) for 4;
/// the hunted animal's (2) for 9 and 59; 0x10 for 13, 51, 54 and 55; 9 for 18, 43 and 53; 0xb for
/// 20; 0xc for 21; 0x13 for 29 and 58; the soldier's (0xd) for any other. Class 10 takes the
/// boar's (4, `FUN_140c11720`) when its master is of class 10 and combat level 1
/// (`FUN_140fde4f0`), else the lion's (5, `FUN_140c11bd0`).
#[must_use]
pub const fn factory_type(class: i16, master: i16, missile: i16, combat_level: u8) -> Option<u32> {
    match master {
        1105 => return Some(0x12),
        1744 | 1746 => return Some(0x10),
        _ => {}
    }
    Some(match class {
        5 | 7 | 8 | 11 | 14 | 15 | 16 | 30..=34 | 38 | 42 | 48 | 63 => return None,
        1 => 0xe,
        2 | 19 => 0xa,
        3 | 27 | 39 | 49 | 60 if missile < 0 => 17,
        3 | 27 | 39 | 49 | 52 | 60 => TOWER_AI,
        4 => CIVILIAN,
        9 | 59 => 2,
        // FUN_140fde4f0: the master's class (`+0x410`, 10 in this case) and its combat level 1
        10 if combat_level == 1 => BOAR_AI,
        10 => 5,
        13 | 51 | 54 | 55 => 0x10,
        18 | 43 | 53 => 9,
        20 => 0xb,
        21 => 0xc,
        29 | 58 => 0x13,
        _ => SOLDIER_AI,
    })
}

/// What a unit AI's constructor reads besides its type (`FUN_141215a90`'s and
/// `FUN_141215560`'s constructors): the new object's place, its master's fields and its
/// owner's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NewObject {
    /// The object's place (`+0x3c8`, `+0x4c8`, `+0x5a8`): the state position, the hunted
    /// animal's and the boar's origin.
    pub position: [f32; 3],
    /// Its master's id (`+0x58`).
    pub master: i16,
    /// Its master's `copy_id` (`+0x158`).
    pub copy_id: i16,
    /// Its master's class (`+0x410`).
    pub class: i16,
    /// Its master's `object_flags` (`+0x654`).
    pub object_flags: u32,
    /// Its master's `hero_flag` (`+0x8d9`).
    pub hero_flag: u8,
    /// The object's slot 0x7b0 (`+0xb74`, which the action object's constructor set before
    /// through slot 0x8a8, `FUN_141206d50`: the work range of its master's last task of action
    /// type 6, else 0): the hunted animal's leash.
    pub hunt_range: f32,
    /// The owner is a computer player (`FUN_141659cf0`: `+0x10` 3).
    pub computer: bool,
    /// The owner's posture (`+0x71c`, its slot 0x90 `FUN_1416224f0`).
    pub posture: i32,
    /// The owner's attribute 6, the current age (`player+0x100` at `+0x30`).
    pub age: f32,
}

impl NewObject {
    /// The object at `position` of master `made` (id `master`), its owner's fields left at a
    /// human player's in the Dark Age with posture 0 ([`NewObject::computer`],
    /// [`NewObject::posture`], [`NewObject::age`] are the caller's).
    #[must_use]
    pub fn of(position: [f32; 3], master: i16, made: &crate::masters::Master) -> Self {
        Self {
            position,
            master,
            copy_id: made.copy_id,
            class: made.class,
            object_flags: made.object_flags,
            hero_flag: made.hero_flag,
            // FUN_141206d50: each task of action type 6 in turn sets `+0xb74`
            hunt_range: made
                .tasks
                .iter()
                .rev()
                .find(|t| t.action_type == HUNT_RANGE_TASK)
                .map_or(0.0, |t| t.work_range),
            computer: false,
            posture: 0,
            age: 0.0,
        }
    }
}

/// The action type of the task whose work range a new object keeps at `+0xb74`
/// (`FUN_141206d50`).
const HUNT_RANGE_TASK: i16 = 6;
/// The hunted animal's idle timeout (`FUN_140c11920`'s `FUN_140c41bc0` with 4000).
const HUNTED_IDLE_TIMEOUT: u32 = 4000;
/// The priest's idle timeout (`FUN_140c11e30`'s `FUN_140c41bc0` with 500).
const PRIEST_IDLE_TIMEOUT: u32 = 500;
/// The soldier's idle timeout (`FUN_140c12300`'s `FUN_140c41bc0`): 500, a computer player's
/// 1500.
const SOLDIER_IDLE_TIMEOUT: u32 = 500;
const COMPUTER_SOLDIER_IDLE_TIMEOUT: u32 = 1500;
/// The siege unit's idle timeout (`FUN_140c12040`'s `FUN_140c41bc0` with 1000).
const SIEGE_IDLE_TIMEOUT: u32 = 1000;
/// The masters whose soldier is defensive (`FUN_140c12300`, raw `0x62ab`, `0x62ac`, `0x64ec`).
const DEFENSIVE_MASTERS: [i16; 3] = [527, 528, 1104];
/// The classes of a human player's soldier in `NoAttack` (`FUN_140c12300`: `-0xe89`, `-0xe6c`
/// raw).
const NO_ATTACK_CLASSES: [i16; 2] = [35, 64];
/// The class whose soldier is in `NoAttack` unless a hero (`FUN_140c12300`: `-0xe6f` raw).
const NO_ATTACK_CLASS: i16 = 61;
/// The master object flag of the soldier that stands its ground in the Dark Age
/// (`FUN_140c12300`: `+0x654` bit 4).
const STAND_GROUND_FLAG: u32 = 0x10;

/// The soldier's stance (`+0x1a9`) its constructor (`FUN_140c12300`) gives a new object: 2
/// (stand ground) to a human player's object of a master with object flag 0x10 that is its own
/// copy (`copy_id`, `+0x158`, its id) while the player's current age (attribute 6) is 0; 1
/// (defensive) to a human player's of masters 527, 528 and 1104; 3 (no attack) to a human
/// player's of classes 35 and 64; then 1 to a hero (`hero_flag` with bit 0 or 3, not both 3
/// and 7); 3 to class 61; else the owner's posture (slot 0x90, `+0x71c`, its low byte).
fn soldier_stance(new: &NewObject) -> u8 {
    // UCOMISS with JP, JNZ: the age equals 0 (a NaN does not)
    if !new.computer
        && new.object_flags & STAND_GROUND_FLAG != 0
        && new.copy_id == new.master
        && new.age == 0.0
    {
        return 2;
    }
    if !new.computer && DEFENSIVE_MASTERS.contains(&new.master) {
        return 1;
    }
    if !new.computer && NO_ATTACK_CLASSES.contains(&new.class) {
        return 3;
    }
    // TEST 9, then a signed byte at least 0 or bit 3 clear
    let hero = new.hero_flag & 9 != 0 && (new.hero_flag & 0x80 == 0 || new.hero_flag & 8 == 0);
    if hero {
        return 1;
    }
    if new.class == NO_ATTACK_CLASS {
        return 3;
    }
    new.posture as u8
}

/// The siege unit's stance (`+0x1a9`) its constructor (`FUN_140c12040`) gives a new object: 0
/// for a computer player's and for classes 51 and 54 (`-0xe79`, `-0xe76` raw); the owner's
/// posture (slot 0x90, its low byte) for class 55 (`-0xe75`); 1 for any other.
const fn siege_stance(new: &NewObject) -> u8 {
    if new.computer || matches!(new.class, 51 | 54) {
        return 0;
    }
    if new.class == 55 {
        return new.posture as u8;
    }
    1
}

/// A ship's draw on a missile's warning (`0x20f`, `processNotify`): `r * 100 / 0x7fff` below
/// its owner's `chance_to_dodge_missiles` ([`Surroundings::dodge_chance`]) dodges it.
fn dodges(at: &Surroundings, ai: &mut Well512) -> bool {
    // IMUL by 100, then IDIV by 0x7fff: a 15-bit draw stays positive
    let r = ai.rand15() as i32;
    r * 100 / 0x7fff < i32::from(at.dodge_chance)
}

/// The owner's slot 0x2e8 with an alert of its unit's (0x201 under attack, 0x209 dead; 0x206
/// and 0x207 from a transport, units left inside or no unload there; `FUN_14162dab0`): a human
/// player's alert list or interface message and the old pather's hulled-area reset, not held;
/// a computer player's AI hears it (slot 0x3c8), which is not modelled.
pub(crate) const fn alert_owner(computer: bool) -> Result<(), GenieError> {
    if computer {
        return Err(GenieError::unsupported(
            "a computer player's AI hearing its unit's alert (slot 0x2e8, then slot 0x3c8)",
        ));
    }
    Ok(())
}

/// `AskForHelp`'s helper that passes every test ([`crate::turn::view::UnitView::helper_free`]):
/// its reach to the attacker (`FUN_14104c220` within its weapon range), its own
/// `bestUnitToAttack` (`FUN_140c44680` from its place) and its `AttackObject` (slot 0xb8, any)
/// on the nearer of that and the attacker, which run as the helper's, are not modelled.
pub(crate) const fn helper_answers() -> Result<(), GenieError> {
    Err(GenieError::unsupported(
        "a helper that answers a call for help (its reach, bestUnitToAttack and AttackObject)",
    ))
}

/// The classes a transport ship cares about (`+0x190`, `TribeTransportShipUnitAIModule`'s
/// constructor `FUN_140c12c30`).
pub const TRANSPORT_CARED: [i32; 10] = [3, 22, 20, 2, 21, 53, 52, 27, 39, 49];

/// The classes a siege tower cares about (`+0x190`, `TribeSiegeTowerUnitAIModule`'s constructor
/// `FUN_140c11fb0`).
pub const SIEGE_TOWER_CARED: [i32; 2] = [27, 39];

/// The building unit AI's factory type (`TribeBuildingUnitAIModule`).
const BUILDING_AI: u32 = 17;

/// The transport ship's factory type (`TribeTransportShipUnitAIModule`).
const TRANSPORT_AI: u32 = 11;

/// The unit AI's slot 0x90 (`closestUndiscoveredTile`, `FUN_140c4bd40`) for an explorer other
/// than the auto scout, then its explore there (slot 0x100); not modelled.
const fn closest_undiscovered_tile() -> Result<(), GenieError> {
    Err(GenieError::unsupported(
        "an explorer's search for the closest undiscovered tile (slot 0x90)",
    ))
}

/// The state of the unit's top action (slot 0x2b0) as [`Surroundings::action_state`] holds it,
/// for an action the slice holds.
const fn held_action_state(state: Option<u8>) -> Result<u8, GenieError> {
    match state {
        Some(s) => Ok(s),
        None => Err(GenieError::unsupported(
            "an action the slice does not hold (its state, slot 0x2b0)",
        )),
    }
}

impl UnitAi {
    /// The unit AI of `ai_type` a new object gets, as `FUN_141215560`'s constructors make it
    /// (the factory `FUN_141215a90` picks among them; the types 3, 6 and 7 only a load makes);
    /// None for any other type, which gets none. `UnitAIModule`'s constructor (`0x140c2e080`)
    /// draws three AI-stream values, in this order: the last retarget time `r % 500`, the
    /// adjusted idle timeout `r % 1500 + 2250` and the retarget timer `750 + r % 500`; it sets
    /// the stance (`+0x1a9`) to 3 and the state position to the object's place. Then each
    /// type's own constructor:
    ///
    /// - 2, the hunted animal (`FUN_140c11920`, [`HUNTED_CARED`]): its idle timeout 4000
    ///   (`FUN_140c41bc0`, whose slot 0x1a8 `FUN_140c17170` makes the adjusted one 5/4 of it
    ///   without a draw), its origin (the object's place) and leash ([`NewObject::hunt_range`]);
    ///   the king's (class 59) stance 0, the others' stays 3.
    /// - 3 (`TribePreditorUnitAIModule`, `FUN_140c11da0`), 0xa (the trade ship's,
    ///   `FUN_140c12ab0`), 0xb (the transport ship's, `FUN_140c12c30`), 0xc (the fishing ship's,
    ///   `FUN_140c11860`, the base's `+0x1e8` 7), 0x11 (the building's, `FUN_140c11470`,
    ///   [`BUILDING_CARED`]), 0x12 (the siege tower's, `FUN_140c11fb0`) and 8 (the civilian's,
    ///   `FUN_140c115d0`, [`CIVILIAN_CARED`], the base's `+0x1e8` 7): stance 0.
    /// - 4, the boar (`FUN_140c11720`): stance 0, its origin the object's place, not furious.
    /// - 5, the lion (`FUN_140c11bd0`, [`LION_CARED`]; the world's `+0x6b4` of 1 adds four
    ///   classes): stance 0, its idle timeout 6000 and a fourth draw (slot 0x1a8,
    ///   `0x140c41e00`).
    /// - 6 and 7 (`TribeRangedUnitAIModule`, `FUN_140c11f80`, `FUN_140c11f50`): the base's.
    /// - 9, the priest (`FUN_140c11e30`): stance 1, its idle timeout 500 and a fourth draw.
    /// - 0xd, the soldier (`FUN_140c12300`, [`SOLDIER_CARED`]; a computer player's adds 10): its
    ///   idle timeout 500 (a computer player's 1500) and a fourth draw, then its stance
    ///   ([`soldier_stance`]).
    /// - 0xe, the artifact (`FUN_140c11340`, [`ARTIFACT_CARED`]): stance 0, its idle timeout
    ///   5000 and a fourth draw.
    /// - 0xf, the tower (`FUN_140c12960`, [`TOWER_CARED`]): stance 0, its idle timeout 1000 and
    ///   a fourth draw.
    /// - 0x10, the siege unit (`FUN_140c12040`): its stance ([`siege_stance`]), its idle timeout
    ///   1000 and a fourth draw.
    /// - 0x13, livestock (`FUN_140c11d70`): stance 1.
    #[must_use]
    pub fn constructed(ai_type: u32, new: &NewObject, ai: &mut Well512) -> Option<Self> {
        if !matches!(ai_type, 2..=0x13) {
            return None;
        }
        let mut made = Self::base_constructed(ai_type, new.position, ai);
        match ai_type {
            2 => {
                made.set_idle_timeout(HUNTED_IDLE_TIMEOUT, ai);
                made.hunted = Some(Hunted {
                    origin: new.position,
                    leash: new.hunt_range,
                });
                if new.class == CLASS_KING {
                    made.stance = 0;
                }
            }
            BOAR_AI => {
                made.stance = 0;
                made.home = Some(Home {
                    origin: new.position,
                    furious: false,
                    go_home: false,
                });
            }
            5 => {
                made.stance = 0;
                made.set_idle_timeout(LION_IDLE_TIMEOUT, ai);
            }
            // the base's stance 3
            6 | 7 => {}
            9 => {
                made.stance = 1;
                made.set_idle_timeout(PRIEST_IDLE_TIMEOUT, ai);
            }
            SOLDIER_AI => {
                let timeout = if new.computer {
                    COMPUTER_SOLDIER_IDLE_TIMEOUT
                } else {
                    SOLDIER_IDLE_TIMEOUT
                };
                made.set_idle_timeout(timeout, ai);
                made.stance = soldier_stance(new);
            }
            ARTIFACT_AI => {
                made.stance = 0;
                made.set_idle_timeout(ARTIFACT_IDLE_TIMEOUT, ai);
            }
            TOWER_AI => {
                made.stance = 0;
                made.set_idle_timeout(TOWER_IDLE_TIMEOUT, ai);
            }
            0x10 => {
                made.stance = siege_stance(new);
                made.set_idle_timeout(SIEGE_IDLE_TIMEOUT, ai);
            }
            0x13 => made.stance = 1,
            // 3, 8, 0xa, 0xb, 0xc, 0x11 and 0x12
            _ => made.stance = 0,
        }
        Some(made)
    }

    /// `UnitAIModule`'s constructor (`0x140c2e080`): its three draws, the base's fields, the
    /// stance (`+0x1a9`) 3 of the two bytes at `+0x1a8` (0x300) and the state position the
    /// object's place.
    fn base_constructed(ai_type: u32, position: [f32; 3], ai: &mut Well512) -> Self {
        let last_retarget_time = ai.rand15() % 500;
        let adjusted_idle_timeout = adjusted_idle_timeout(IDLE_TIMEOUT, ai);
        let retarget_timer = 750 + ai.rand15() % 500;
        Self {
            ai_type,
            current_order: -1,
            order_priority: -1,
            current_action: -1,
            current_target: -1,
            target_type: -1,
            target_point: [-1.0; 3],
            desired_distance: DESIRED_DISTANCE,
            defend_target: -1,
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
            stance: 3,
            // the constructor's word at +0x1d4 (0x140c2e080)
            formation_type: 0,
            // the object's place
            state_position: [position[0], position[1]],
            retarget_disabled: 0,
            alert_mode: false,
            time_since_enemy: 0.0,
            patrol_path: None,
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
            order_history: Vec::new(),
            retries: Vec::new(),
            best_this_update: -1,
        }
    }

    /// `FUN_140c42320` for each retarget entry an attack search's `bestUnitToAttack` added
    /// ([`Found::retries`]): it joins the end of the entries.
    pub fn add_retries(&mut self, found: &Found) {
        self.retries.extend(found.retries.iter().flatten());
    }

    /// The memory `bestUnitToAttack` reads ([`Memory`]).
    #[must_use]
    pub fn memory(&self) -> Memory<'_> {
        Memory {
            retries: &self.retries,
            history: &self.order_history,
        }
    }

    /// `FUN_140c42080` once the world holds the target (the callers' `FUN_1415417a0`; an order on
    /// a point records nothing): the order joins the history first, a ninth recycling the
    /// oldest ([`ORDER_HISTORY`]).
    pub fn remember_order(&mut self, record: OrderRecord) {
        self.order_history.insert(0, record);
        self.order_history.truncate(ORDER_HISTORY);
    }

    /// The full update's start (`0x140c66b10`): its retarget entries by the world's objects
    /// (`seen`, [`UnitSide::seen_unit`]): a target the world no longer holds or below 1 hit point
    /// (`COMISS`, a NaN stays) empties them all; otherwise one whose time (`timeout`) is not past
    /// `now` stays and the others go, in order.
    ///
    /// # Errors
    ///
    /// What the world's lookup does not model.
    pub fn expire_retries(
        &mut self,
        now: u32,
        seen: &mut dyn FnMut(i32) -> Result<Option<UnitSeen>, GenieError>,
    ) -> Result<(), GenieError> {
        let mut i = 0;
        while i < self.retries.len() {
            let (target, timeout) = self.retries[i];
            if seen(target)?.is_none_or(|u| u.hp < 1.0) {
                self.retries.clear();
                break;
            }
            if now < timeout {
                i += 1;
            } else {
                let _ = self.retries.remove(i);
            }
        }
        Ok(())
    }

    /// `FUN_140c41bc0`: the idle timeout (`+0x168`) is `timeout`, then slot 0x1a8 redraws the
    /// adjusted one ([`UnitAi::update_adjusted_idle_timeout`]).
    fn set_idle_timeout(&mut self, timeout: u32, ai: &mut Well512) {
        self.idle_timeout = timeout;
        self.update_adjusted_idle_timeout(ai);
    }

    /// What an order (INTERACT, MOVE, RETREAT) does to each of its units' AI before it runs
    /// (`FUN_140e6cfc0`): the patrol path goes (`FUN_140c68de0`: `+0x1c0`, its waypoint `+0x1c8`
    /// 0); an adjusted idle timeout of 750 or more is redrawn
    /// (`UpdateAdjustedIdleTimeout`, unit-AI slot 53, `0x140c41e00`), and the secondary timer is
    /// capped at 750.
    pub fn reset_for_order(&mut self, ai: &mut Well512) {
        self.zero_patrol();
        if self.adjusted_idle_timeout > 749 {
            self.update_adjusted_idle_timeout(ai);
        }
        self.secondary_timer = self.secondary_timer.min(750);
    }

    /// `zeroPatrolInformation` (`FUN_140c68de0`): the saved patrol goes and its waypoint is 0.
    pub fn zero_patrol(&mut self) {
        self.patrol_path = None;
        self.patrol_waypoint = 0;
    }

    /// `savePatrolInformation` (`FUN_140c63d20`): the saved patrol goes; the unit's slot 0x468
    /// gives the new one and its waypoint (0 without one).
    fn save_patrol(&mut self, side: &dyn UnitSide) {
        let (path, waypoint) = side.patrol_information().unzip();
        self.patrol_path = path;
        self.patrol_waypoint = waypoint.unwrap_or(0);
    }

    /// `restorePatrolInformation` (`FUN_140c5e5b0`): without a saved patrol, false. Else the unit
    /// walks it on ([`Effect::RestorePatrol`]); a unit in no group (the world's
    /// `FUN_14152ff40`) takes the patrol order back, the attack move's (733) when it attack-moves
    /// (`+0x7fb`), else 711 (leaving the other one redraws an adjusted idle timeout of exactly
    /// 750, slot 0x1a8); the secondary timer is 750. True.
    fn restore_patrol(
        &mut self,
        at: &Surroundings,
        ai: &mut Well512,
        fx: &mut Vec<Effect>,
    ) -> bool {
        if self.patrol_path.is_none() {
            return false;
        }
        fx.push(Effect::RestorePatrol);
        if at.group.is_none() {
            let (order, other) = if at.attack_moving {
                (ATTACK_MOVE, PATROL)
            } else {
                (PATROL, ATTACK_MOVE)
            };
            if self.current_order == other && self.adjusted_idle_timeout == 750 {
                self.update_adjusted_idle_timeout(ai);
            }
            self.current_order = order;
        }
        self.secondary_timer = 750;
        true
    }

    /// What a `MoveTo`'s update in state 3 (`0x140aa2a70`) reads of its unit's AI for the path
    /// it asks (`ai`, None for a unit without one), the unit at `at` and the `MoveTo`'s point
    /// `point` (x, y): without an AI a unit in range asks anyway and the attack move stays
    /// (`param_10` 2). With one, on a patrol order (711, 733) the path loops; the saved patrol, if
    /// any, gives the waypoint to walk on from and its back and forth (`getPatrolLinear`) and goes
    /// (`zeroPatrolInformation`); else the path goes back and forth when the point is at least 3
    /// from the unit (`COMISS`, `SETBE`: a NaN distance is). The attack move is on order 733, off
    /// on any other, as it was while a saved patrol is still held.
    #[must_use]
    pub fn path_ask(ai: Option<&mut Self>, at: [f32; 2], point: [f32; 2]) -> PathAsk {
        let Some(ai) = ai else {
            return PathAsk {
                ends_in_range: false,
                ..PathAsk::default()
            };
        };
        let patrolling = matches!(ai.current_order, PATROL | ATTACK_MOVE);
        let mut ask = PathAsk {
            ends_in_range: !patrolling,
            ..PathAsk::default()
        };
        if patrolling {
            ask.patrol = true;
            if let Some(saved) = ai.patrol_path.as_ref() {
                ask.resume = ai.patrol_waypoint;
                ask.back_and_forth = saved.flags & 2 != 0;
                ai.zero_patrol();
            } else {
                let (dx, dy) = (point[0] - at[0], point[1] - at[1]);
                let d = crate::fmath::sqrt(dy * dy + dx * dx);
                ask.back_and_forth = crate::fmath::at_most_or_unordered(PATROL_TURN_REACH, d);
            }
        }
        ask.attack_move = ai
            .patrol_path
            .is_none()
            .then_some(ai.current_order == ATTACK_MOVE);
        ask
    }

    /// `UpdateAdjustedIdleTimeout` (slot 0x1a8): the base draws around the idle timeout
    /// (`0x140c41e00`); a deer's or fox's (factory type 2, `FUN_140c17170`) is 5/4 of it.
    fn update_adjusted_idle_timeout(&mut self, ai: &mut Well512) {
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
        if leaving(PATROL) || leaving(ATTACK_MOVE) {
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
    /// action (`FUN_140c64b60` with −1 and 1, `FUN_140c64b50` with −1); leaving a patrol order
    /// caps the secondary timer ([`UnitAi::cap_patrol_secondary`]); the redraw of an adjusted
    /// idle timeout of exactly 750 (slot 0x1a8, the AI stream's) is not modelled here.
    pub const fn end_order_and_action(&mut self) -> Result<(), GenieError> {
        if self.patrol_redraws() {
            return Err(GenieError::unsupported(
                "the end of a Build on a patrol order",
            ));
        }
        self.cap_patrol_secondary();
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
    ) -> Result<(), GenieError> {
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
    pub(crate) const fn alert(&mut self, found: Found, _dt: f32) {
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
    ) -> Result<bool, GenieError> {
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
    ) -> Result<bool, GenieError> {
        self.cap_human_order(fx);
        if at.class == CLASS_KING {
            let led =
                at.master != KING_UNLED && !at.garrisoned && self.current_order == -1 && computer;
            if led {
                return Err(GenieError::unsupported(
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
    ) -> Result<bool, GenieError> {
        let h = self.hunted.ok_or(GenieError::unsupported(
            "a hunted animal without its origin",
        ))?;
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
            return Err(GenieError::unsupported(
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
                reach: true,
            },
            memory: Memory::default(),
        })?;
        self.alert(found, at.dt_seconds);
        if found.id != -1 {
            let other = found
                .position
                .ok_or(GenieError::unsupported("a found object without its place"))?;
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
    ) -> Result<bool, GenieError> {
        self.cap_human_order(fx);
        self.set_idle_timeout(LION_IDLE_TIMEOUT, ai);
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
                reach: true,
            },
            memory: Memory::default(),
        };
        let mut reach = if at.mode_1 {
            at.los
        } else {
            at.los * LION_SIGHT_SHARE
        };
        if at.scenario {
            return Err(GenieError::unsupported(
                "a lion's reach in the scenario modes",
            ));
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
    ) -> Result<bool, GenieError> {
        match self.ai_type {
            2 => self.hunted_idle(at, computer, ai, side, fx),
            BOAR_AI => Ok(self.boar_idle(at, ai, fx)),
            5 => self.lion_idle(at, ai, side, fx),
            // the civilian's (FUN_140c1e610), the transport ship's (0x140c21d20) and the siege
            // tower's (FUN_140c20690): the cap, then 5
            CIVILIAN | 11 | SIEGE_TOWER_AI => {
                self.cap_human_order(fx);
                Ok(false)
            }
            SOLDIER_AI => self.soldier_idle(at, computer, ai, side, fx),
            TRADE_AI | FISHING_AI => self.ship_idle(at, computer, ai, side, fx),
            SIEGE_AI => self.siege_idle(at, computer, ai, side, fx),
            TOWER_AI => self.tower_idle(at, computer, side, fx),
            ARTIFACT_AI => self.artifact_idle(at, side, fx),
            17 | 19 => Ok(false),
            _ => Err(GenieError::unsupported(
                "processIdle (slot 0x240) of a unit-AI type not modelled",
            )),
        }
    }

    /// The trade ship's and the fishing ship's idle check (slot 72, `FUN_140c21930`,
    /// `FUN_140c1eab0`): the cap ([`UnitAi::cap_human_order`]); a computer player's ship not
    /// inside another looks for the closest object of the classes it cares about
    /// (`ClosestObject` with every relation, state and master, reachable: [`Pick::Closest`]; the
    /// search adds the fishing ship's `+0x1e8`, 7, to its radius) and, finding one the world
    /// holds, moves away from it by its speed times [`SHIP_EVADE_REACH`] along the line between
    /// them (`evasiveMoveTo`, slot 0x150 with both flags, [`UnitAi::evasive_move`]): true (6).
    /// Else false (5).
    fn ship_idle(
        &mut self,
        at: &Surroundings,
        computer: bool,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        self.cap_human_order(fx);
        if !computer || at.garrisoned {
            return Ok(false);
        }
        let (classes, radius): (&[i32], f32) = if self.ai_type == FISHING_AI {
            (
                &FISHING_CARED,
                civilian_search_radius(at.los, at.search_radius),
            )
        } else if at.class == TRADE_BOAT_CLASS {
            (&TRADE_SHIP_CARED, search_radius(at.los, at.search_radius))
        } else {
            (&TRADE_CART_CARED, search_radius(at.los, at.search_radius))
        };
        let found = side.closest(&Search {
            at: [at.position[0], at.position[1]],
            radius,
            classes,
            size: at.size,
            pick: Pick::Closest {
                group: -1,
                relation: -1,
                state: -1,
                master: -1,
                workers: -1,
                spared: -1,
                reach: true,
            },
            memory: Memory::default(),
        })?;
        self.alert(found, at.dt_seconds);
        if found.id == -1 {
            return Ok(false);
        }
        let Some(u) = side.seen_unit(found.id)? else {
            return Ok(false);
        };
        let reach = at.speed * SHIP_EVADE_REACH;
        let [x, y, z] = at.position;
        let dx = u.position[0] - x;
        let dy = u.position[1] - y;
        let d = crate::fmath::sqrt(dy * dy + dx * dx);
        let point = [x - (dx / d) * reach, y - (dy / d) * reach, z];
        let _ = self.evasive_move(point, (true, true), at, ai, side, fx)?;
        Ok(true)
    }

    /// The siege unit's idle check (slot 72, `FUN_140c206b0`, factory type 0x10). Slot 0x250 first
    /// retries no order (the base's slot 0x60 takes none as retryable), then the cap
    /// ([`UnitAi::cap_human_order`]). A Gaia unit stops (5) unless of the combat type with
    /// `+0x924` 0 outside the scenario modes. In `NoAttack` (stance 3) it searches alone (slot 0x50
    /// with mode 1 from its place, [`Pick::Look`] over [`SIEGE_CARED`], [`UnitAi::attack_search_alert`])
    /// and but on the patrol orders walks back to its state position ([`UnitAi::walk_back`]);
    /// 5. Else its target is its top action's within its range (slot 400, slot 0x1e8); with
    /// retargeting off (`+0x1d6`, `FUN_140c4ee90`) and the world holding its current target, a
    /// top target the unit reaches within its range (slot 0x3d0 with slot 0x230) ends the check
    /// (5), else retargeting comes back on (`FUN_140c40ef0` with 0). A computer player's tests
    /// ([`computer_idle_check`]) follow. Without a target `bestUnitToAttack` from its place
    /// ([`Pick::Attack`] over [`SIEGE_CARED`]); a target found it attacks (slot 0xb8 forced, any
    /// 0, [`UnitAi::attack_object`]), a patrol order's patrol saved first: once it attacks, off
    /// the defend order (`0x2bd`) the order goes (`FUN_140c64b60` with −1, 0) and it gives 6; a
    /// failed attack on a patrol order loses the saved patrol. On a patrol order it then gives 5.
    /// Else (the alert mode's object `+0x1bc` taking the base's slot 0x1d8, −1, not held) a human
    /// player's unit in a group gives 5 ("I'm in a formation"), one in Defend or `StandGround`
    /// walks back; last the saved patrol, if any, goes on (`restorePatrolInformation`,
    /// [`UnitAi::restore_patrol`]: 6). Else 5.
    fn siege_idle(
        &mut self,
        at: &Surroundings,
        computer: bool,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        self.cap_human_order(fx);
        let searches = !at.scenario && 70 <= at.kind && at.capture == 0;
        if at.player == 0 && !searches {
            return Ok(false);
        }
        if self.stance == 3 {
            let found = side.closest(&Search {
                at: [at.position[0], at.position[1]],
                radius: attack_search_radius(at),
                classes: &SIEGE_CARED,
                size: at.size,
                pick: Pick::Look,
                memory: Memory::default(),
            })?;
            self.attack_search_alert(found, computer)?;
            if !matches!(self.current_order, PATROL | ATTACK_MOVE) {
                let _ = self.walk_back(at, fx);
            }
            return Ok(false);
        }
        let mut target = top_target_in_range(at).unwrap_or(-1);
        if self.retarget_disabled != 0 && at.target.is_some() {
            // slot 0x3d0 with the top target (−1 answers 0) and the weapon range
            if target != -1 && side.reaches(target, at.weapon_range)? {
                return Ok(false);
            }
            self.retarget_disabled = 0;
        }
        if computer {
            computer_idle_check()?;
        }
        if target == -1 {
            let found = side.closest(&Search {
                at: [at.position[0], at.position[1]],
                radius: attack_search_radius(at),
                classes: &SIEGE_CARED,
                size: at.size,
                pick: Pick::Attack {
                    lowest: if at.scenario { 2 } else { 0 },
                    seeker: self.seeker(at),
                    ask: AttackAsk::IDLE,
                },
                memory: self.memory(),
            })?;
            self.add_retries(&found);
            self.attack_search_alert(found, computer)?;
            target = found.id;
        }
        if target != -1 {
            if matches!(self.current_order, PATROL | ATTACK_MOVE) {
                self.save_patrol(side);
            }
            if self.attack_object(target, true, false, at, side, fx)? {
                if self.current_order != DEFEND_ORDER {
                    self.set_order_with(-1, false, ai);
                }
                return Ok(true);
            }
            if matches!(self.current_order, PATROL | ATTACK_MOVE) {
                self.zero_patrol();
            }
        }
        if matches!(self.current_order, PATROL | ATTACK_MOVE) {
            return Ok(false);
        }
        if !computer {
            if at.group.is_some() {
                return Ok(false);
            }
            if matches!(self.stance, 1 | 2) {
                let _ = self.walk_back(at, fx);
            }
        }
        Ok(self.restore_patrol(at, ai, fx))
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
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
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
        // its target: with retargeting on, its top action's within its range (slot 400, slot
        // 0x1e8); with it off, its current target while the world holds it, else the top
        // action's
        let held =
            (self.retarget_disabled != 0 && at.target.is_some()).then_some(self.current_target);
        let target = held.or_else(|| top_target_in_range(at));
        if computer {
            computer_idle_check()?;
        }
        let target = if let Some(t) = target {
            t
        } else {
            let found = side.closest(&Search {
                at: [at.position[0], at.position[1]],
                radius: attack_search_radius(at),
                classes: &SOLDIER_CARED,
                size: at.size,
                pick: Pick::Attack {
                    lowest: if at.scenario { 2 } else { 0 },
                    seeker: self.seeker(at),
                    ask: AttackAsk::IDLE,
                },
                memory: self.memory(),
            })?;
            self.add_retries(&found);
            self.attack_search_alert(found, computer)?;
            found.id
        };
        if target != -1 {
            // "I need to save the patrol information in case the attack works."
            if matches!(self.current_order, PATROL | ATTACK_MOVE) {
                self.save_patrol(side);
            }
            // slot 0xb8 forced (FUN_140c17550: the base's, then the unit's group);
            // with retargeting on and off order 0x2bd the order goes (FUN_140c64b60 with -1, 0)
            if self.attack_slot(target, false, at, side, fx)? {
                if self.retarget_disabled == 0 && self.current_order != 0x2bd {
                    self.set_order_with(-1, false, ai);
                }
                return Ok(true);
            }
            // "Attack failed, so I'm nuking the saved patrol information."
            if matches!(self.current_order, PATROL | ATTACK_MOVE) {
                self.zero_patrol();
            }
        }
        // "I have a patrol order, so I'm stopping here."
        if matches!(self.current_order, PATROL | ATTACK_MOVE) {
            return Ok(false);
        }
        // "I'm in Defend or StandGround, returning to state pos.": the walk back's result does
        // not make the processIdle's
        if matches!(self.stance, 1 | 2) {
            let _ = self.walk_back(at, fx);
        }
        // the saved patrol, if any, goes on (`restorePatrolInformation`): processIdle produced
        if self.restore_patrol(at, ai, fx) {
            return Ok(true);
        }
        if at.task_0x69 {
            return Err(GenieError::unsupported(
                "a healer's search for an ally to heal",
            ));
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
    ) -> Result<(), GenieError> {
        if self.current_order == DEFEND_ORDER {
            return Err(GenieError::unsupported(
                "a defend order's search (its target's objects)",
            ));
        }
        if computer {
            return Err(GenieError::unsupported(
                "a computer player's soldier's NoAttack search",
            ));
        }
        let found = side.closest(&Search {
            at: [at.position[0], at.position[1]],
            radius: attack_search_radius(at),
            classes: &SOLDIER_CARED,
            size: at.size,
            pick: Pick::Look,
            memory: Memory::default(),
        })?;
        if found.enemies {
            return Err(GenieError::unsupported(
                "a NoAttack soldier's search that finds enemies",
            ));
        }
        if found.fresh {
            self.alert_mode = false;
        }
        if !matches!(self.current_order, PATROL | ATTACK_MOVE) {
            let _ = self.walk_back(at, fx);
        }
        if at.task_0x69 {
            return Err(GenieError::unsupported(
                "a healer's search for an ally to heal",
            ));
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
    ) -> Result<bool, GenieError> {
        self.cap_human_order(fx);
        if self.stance == 3 {
            let found = side.closest(&Search {
                at: [at.position[0], at.position[1]],
                radius: attack_search_radius(at),
                classes: &TOWER_CARED,
                size: at.size,
                pick: Pick::Look,
                memory: Memory::default(),
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
                seeker: self.seeker(at),
                ask: AttackAsk::IDLE,
            },
            memory: self.memory(),
        })?;
        self.add_retries(&found);
        self.attack_search_alert(found, computer)?;
        if found.id == -1 {
            return Ok(false);
        }
        // slot 0xb8 with 1 (`+0x1d5` then 1, not held)
        self.attack_object(found.id, true, false, at, side, fx)
    }

    /// What `bestUnitToAttack` reads of this unit AI ([`Seeker`]), its current target's master
    /// type as the unit's surroundings `at` hold it.
    fn seeker(&self, at: &Surroundings) -> Seeker {
        self.seeker_with(
            at.target
                .filter(|_| self.current_target != -1)
                .map(|t| t.kind),
        )
    }

    /// What the attack tests read of the unit AI ([`Seeker`]), its current target's master type
    /// `target_kind` as the world holds it.
    #[must_use]
    pub(crate) const fn seeker_with(&self, target_kind: Option<u8>) -> Seeker {
        Seeker {
            stance: self.stance,
            state_position: self.state_position,
            order: self.current_order,
            action: self.current_action,
            target: self.current_target,
            target_type: self.target_type,
            last_target_type: self.last_target_type,
            patrol_path: self.patrol_path.is_some(),
            target_kind,
        }
    }

    /// The alert mode after `bestUnitToAttack`'s search (`FUN_140c63e20` with its fourth argument
    /// 1): a query that ran sets it to whether it found enemies (`+0x1b8`); with enemies, after
    /// more than 10 s without a sighting (`+0x1b4`) the owner hears of it (its slot 0x2e8 with
    /// `0x210`, `FUN_14162dab0`: no case for it, only the reset of the old short pather's hulled
    /// area, and a computer player's AI's notify, not modelled), and the time restarts.
    fn attack_search_alert(&mut self, found: Found, computer: bool) -> Result<(), GenieError> {
        if !found.fresh {
            return Ok(());
        }
        self.alert_mode = found.enemies;
        if found.enemies {
            if ENEMY_SIGHTING_GAP < self.time_since_enemy && computer {
                return Err(GenieError::unsupported(
                    "a computer player's notify of enemies sighted (0x210, slot 0x3c8)",
                ));
            }
            self.time_since_enemy = 0.0;
        }
        Ok(())
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
    /// the group patrols. Its start lets the expired retarget entries go
    /// ([`UnitAi::expire_retries`]) and forgets the update's best unit to attack
    /// ([`UnitAi::best_this_update`]).
    pub fn update(
        &mut self,
        dt: u32,
        world_time: u32,
        computer: bool,
        at: &Surroundings,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
    ) -> Result<Vec<Effect>, GenieError> {
        let retarget = retarget_kind(self.ai_type)?;
        self.commanded = false;
        self.best_this_update = -1;
        // the full update's start: the time since the last enemy sighting grows by the world's dt
        self.time_since_enemy += at.dt_seconds;
        let mut fx = Vec::new();
        self.expire_retries(world_time, &mut |id| Self::world_object(id, at, side))?;
        self.target_block(at, computer, ai, side, &mut fx)?;
        self.keep_attackers(at, side);
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
                self.check_order(at, ai);
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
            self.run_orders(at, side, computer, ai, &mut fx)?;
        }
        // an order on a target ran its work here (slot 0x230's case 0x2db, the unit's slot
        // 0x128): the caller gives it, then the rest of the update reads what it gave the AI
        // ([`UnitAi::after_order`])
        if fx.iter().any(|e| matches!(e, Effect::OrderOnTarget(_))) {
            fx.push(Effect::AfterOrder(AfterOrder {
                busy,
                group: at.group,
            }));
            return Ok(fx);
        }
        self.update_tail(
            retarget, world_time, computer, busy, at.group, at, ai, side, &mut fx,
        )?;
        Ok(fx)
    }

    /// The full update's rest after the order on a target it took ([`AfterOrder`]), which the
    /// game runs after the order's work inside the update and the caller once it gave the work:
    /// [`UnitAi::update`]'s tail with what the order gave the AI and the unit, `at` its
    /// surroundings then; the idle part reads the update's own `busy` and group from before the
    /// order.
    ///
    /// # Errors
    ///
    /// What the tail does not model.
    pub fn after_order(
        &mut self,
        after: AfterOrder,
        world_time: u32,
        computer: bool,
        at: &Surroundings,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
    ) -> Result<Vec<Effect>, GenieError> {
        let retarget = retarget_kind(self.ai_type)?;
        let mut fx = Vec::new();
        self.update_tail(
            retarget,
            world_time,
            computer,
            after.busy,
            after.group,
            at,
            ai,
            side,
            &mut fx,
        )?;
        Ok(fx)
    }

    /// The full update after its order queue (`0x140c66b10` from its slot 73 call on): slot 73
    /// (the base's retarget check [`UnitAi::retarget_check`], the villager's
    /// [`UnitAi::civilian_misc`], the artifact's [`UnitAi::artifact_misc`], livestock's
    /// nothing), the idle part ([`UnitAi::idle_part`]), the look-around check and the boar's
    /// tail. `busy` and `group` are what the update read before its order queue.
    #[allow(clippy::too_many_arguments)]
    fn update_tail(
        &mut self,
        retarget: Retarget,
        world_time: u32,
        computer: bool,
        busy: bool,
        group: Option<GroupState>,
        at: &Surroundings,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<(), GenieError> {
        match retarget {
            Retarget::Base => {
                let _ = self.retarget_check(world_time, computer, at, ai, side, fx)?;
            }
            // the priest's checks but on the order on a target and for a computer player's
            Retarget::Priest => {
                if self.current_order != ORDER_ON_TARGET && !computer {
                    let _ = self.retarget_check(world_time, computer, at, ai, side, fx)?;
                }
            }
            Retarget::Civilian => self.civilian_misc(at, fx)?,
            Retarget::Never => {}
            Retarget::Artifact => self.artifact_misc(at, side, fx)?,
        }
        self.idle_part(busy, group, at, computer, ai, side, fx)?;
        self.look_around(computer, ai)?;
        self.top_target_tail(at, side, fx)?;
        self.drop_started_builds(side)?;
        self.boar_tail(at, computer, ai, side, fx)
    }

    /// The full update's tail on the target of its unit's top action (slot 400 at `0x140c684fc`,
    /// after the look-around check), as the action list stands after the update's commands: the
    /// last of the [`Effect`]s pending that replaces the actions decides, else the unit's now
    /// ([`top_target_after`]). A Gaia unit
    /// whose target is a Gaia object of type 70 or above that another player can take more than
    /// once (`+0x924` 2) lets it go ("Tasking GAIA unit to stop attacking an object which can be
    /// captured multiple times"): the target goes (`FUN_140c5e1e0`) and the action stops (slot
    /// 0xb0 with 1). Then a target outside a pasture (`+0x790` −1) that cannot be targeted
    /// (`+0x689` 0), or cannot be attacked (`+0x68a` 0) while the action value is the attack's
    /// (600), does the same ("Tasking unit to stop attacking an object which has no targeting
    /// flag enabled") but on the trade action (`0x267`); it reads the action value the first
    /// stop left. A stop after a command the caller gives only once the update ends (a hunt,
    /// gather or build, [`Effect::HuntObject`], [`Effect::GatherObject`],
    /// [`Effect::BuildObject`]) would reach the unit before the command: not modelled.
    fn top_target_tail(
        &mut self,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<(), GenieError> {
        let id = top_target_after(fx, side)?;
        // a hunt, gather or build the caller gives once the update ends (after the stops)
        let deferred = fx
            .iter()
            .rev()
            .find(|e| changes_actions(e))
            .is_some_and(|e| {
                matches!(
                    e,
                    Effect::HuntObject(_)
                        | Effect::GatherObject(_)
                        | Effect::BuildObject(_)
                        | Effect::WorkOn { .. }
                )
            });
        let Some(t) = id.map(|id| side.target_flags(id)).transpose()?.flatten() else {
            return Ok(());
        };
        let captured_again =
            at.player == 0 && t.owner == 0 && COMBAT_KIND <= t.kind && t.capture == 2;
        let action = if captured_again {
            -1
        } else {
            self.current_action
        };
        let untargeted = t.pasture == -1
            && (t.untargetable || (!t.attackable && action == ATTACK_ACTION))
            && action != TRADE_ACTION;
        if deferred && (captured_again || untargeted) {
            return Err(GenieError::unsupported(
                "a stop of the full update's tail after a command the caller gives later",
            ));
        }
        if captured_again {
            self.remove_target();
            self.stop_current(true, at, fx)?;
        }
        if untargeted {
            self.remove_target();
            self.stop_current(true, at, fx)?;
        }
        Ok(())
    }

    /// The end of the full update (`0x140c686e0`): unless the AI's order is the build order
    /// (`0x2be`), each queued build order, the last first, whose foundation the world no longer
    /// holds or holds in a state other than 0 (`+0x640`) leaves the queue.
    fn drop_started_builds(&mut self, side: &mut dyn UnitSide) -> Result<(), GenieError> {
        if self.current_order == BUILD_ORDER {
            return Ok(());
        }
        for k in (0..self.orders.len()).rev() {
            if self.orders[k].order == BUILD_ORDER
                && side
                    .seen_unit(self.orders[k].target)?
                    .is_none_or(|u| u.state != 0)
            {
                let _ = self.orders.remove(k);
            }
        }
        Ok(())
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
    ) -> Result<(), GenieError> {
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
    ) -> Result<(), GenieError> {
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
                        && at.action_state.ok_or(GenieError::unsupported(
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
                    && (matches!(self.current_order, PATROL | ATTACK_MOVE)
                        || at.group.is_some_and(|g| g.patrol))
                {
                    return Err(GenieError::unsupported(
                        "the patrol range of a unit AI's target",
                    ));
                }
                // order 0x262, which is MOVE's action value
                if computer
                    && (0..100).contains(&self.order_priority)
                    && self.current_order == 0x262
                {
                    return Err(GenieError::unsupported(
                        "a computer's order priority by its target's distance",
                    ));
                }
            }
        }
        if self.current_action == BUILD_ORDER {
            return Err(GenieError::unsupported("action 0x2be's target check"));
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
    /// villager does nothing more. A class the slot names a kind of searches by it (slot 0x80,
    /// `ClosestResourceObject` from the notify's drop site, its third parameter: kind 0 below 2
    /// workers, the others any; [`UnitSide::closest_resource`]). A farm found (class 49) and a
    /// computer player's villager that found nothing are not modelled.
    fn hunter_search(
        &mut self,
        n: Notify,
        at: &Surroundings,
        computer: bool,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
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
        let kind = civilian_kind(class);
        let found = if kind == -1 {
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
                    reach: true,
                },
                memory: Memory::default(),
            })?;
            self.alert(found, at.dt_seconds);
            match found.position {
                Some(position) if found.id != -1 => Some(Gatherable {
                    id: found.id,
                    position,
                    class: class as i16,
                }),
                None if found.id != -1 => {
                    return Err(GenieError::unsupported("a found object without its place"));
                }
                _ => None,
            }
        } else {
            // slot 0x80 from the notify's drop site: kind 0 below 2 workers, any other any
            side.closest_resource(ResourceSearch {
                kind: kind as u8,
                cap: if kind == 0 { 2 } else { -1 },
                site: n.p[2],
                current: self.current_target,
                point: NO_POINT,
                min_state: 2,
            })?
        };
        let Some(found) = found else {
            if computer {
                return Err(GenieError::unsupported(
                    "a computer player's hunter that found nothing",
                ));
            }
            return Ok(GO_ON);
        };
        if class == 49 {
            return Err(GenieError::unsupported("a hunter's search finding a farm"));
        }
        self.hunt_object(found.id, i32::from(found.class), found.position, at, fx)?;
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
    ) -> Result<u32, GenieError> {
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
                    reach: true,
                },
                memory: Memory::default(),
            })?;
            self.alert(found, at.dt_seconds);
            match found.position {
                Some(position) if found.id != -1 => Some(Gatherable {
                    id: found.id,
                    position,
                    class: class as i16,
                }),
                None if found.id != -1 => {
                    return Err(GenieError::unsupported("a found object without its place"));
                }
                _ => None,
            }
        } else {
            let cap = if matches!(kind, 3 | 4) { 4 } else { -1 };
            side.closest_resource(ResourceSearch {
                kind: kind as u8,
                cap,
                site: n.p[2],
                current: self.current_target,
                point: NO_POINT,
                min_state: 2,
            })?
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
    /// its priority −1, a patrol order (711, 733) resetting its timers
    /// ([`UnitAi::leave_patrol_timers`]).
    fn next_after_kill(
        &mut self,
        at: &Surroundings,
        computer: bool,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<(), GenieError> {
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
        self.leave_patrol_timers(ai);
        self.last_order = self.current_order;
        self.current_order = -1;
        self.order_priority = -1;
        Ok(())
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
    ) -> Result<u32, GenieError> {
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
    ) -> Result<(), GenieError> {
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

    /// `FUN_140c5dc10` from outside the full update for a unit at `place` (its place and its
    /// master's class, which a stop ending an order reads): true when a completion stopped the
    /// action (slot 0xb0 with 1), whose stopAction (slot 0x3c0) the caller runs on the unit then.
    ///
    /// # Errors
    ///
    /// What the stop does not model.
    pub fn clear_notifies_at(
        &mut self,
        time: u32,
        place: ([f32; 2], i16),
    ) -> Result<bool, GenieError> {
        let at = Surroundings {
            position: [place.0[0], place.0[1], 0.0],
            class: place.1,
            ..Surroundings::default()
        };
        let mut fx = Vec::new();
        self.end_notifies(time, &at, &mut fx)?;
        Ok(fx.contains(&Effect::Stop))
    }

    /// `FUN_140c5dc10` from outside the full update (a MOVE, an ownership change: `time` is the
    /// world time), where a completion it would stop on is not modelled.
    pub fn clear_notifies(&mut self, time: u32) -> Result<(), GenieError> {
        let mut fx = Vec::new();
        self.end_notifies(time, &Surroundings::default(), &mut fx)?;
        if !fx.is_empty() {
            return Err(GenieError::unsupported(
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
    pub fn post(&mut self, n: Notify, unit_state: u8) -> Result<(), GenieError> {
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
    ) -> Result<u32, GenieError> {
        if n.p[1] < 1 {
            self.end_notifies(time, at, fx)?;
            return Ok(DEAD);
        }
        // an animal that is not Gaia's alerts its owner (slot 0x2e8 with 0x201)
        if at.player != 0 {
            alert_owner(computer)?;
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
    ) -> Result<bool, GenieError> {
        // the siege unit's slot 0x160 (FUN_140c2a8f0): with a top action's target or of class 54
        // it does not run; a computer player's first lets the units inside it go
        // (FUN_141225c40 with −1, −1)
        if self.ai_type == SIEGE_AI {
            if at.top_target.is_some() || at.class == 54 {
                return Ok(false);
            }
            if computer {
                return Err(GenieError::unsupported(
                    "a computer player's siege unit running away (FUN_141225c40)",
                ));
            }
        }
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
            return Err(GenieError::unsupported(
                "a computer's unit running from class 22 (its group's runs)",
            ));
        }
        if computer && at.class == 20 {
            return Err(GenieError::unsupported(
                "a computer's transport ship running away",
            ));
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
    ) -> Result<bool, GenieError> {
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
    ) -> Result<u32, GenieError> {
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
        let home = self.home.as_mut().ok_or(GenieError::unsupported(
            "a boar's unit AI without its own part",
        ))?;
        home.furious = furious;
        Ok(BREAK)
    }

    /// `AskForHelp` (`FUN_140c42ce0`) of a unit attacked by a new `attacker`: without an attacker
    /// the world holds (`FUN_1415417a0`) nothing; else its search from its place (slot 0x50,
    /// `FUN_140c63e20` with mode 1: the classes it cares about, [`UnitAi::help_search`]; a search
    /// that ran sets the alert mode, [`UnitAi::attack_search_alert`]), whose lists of its own and
    /// its friends' objects ([`UnitSide::call_for_help`]) it walks for helpers.
    ///
    /// # Errors
    ///
    /// A helper the slice does not model ([`helper_answers`]), and what the search does not.
    fn ask_for_help(
        &mut self,
        attacker: i32,
        at: &Surroundings,
        computer: bool,
        side: &mut dyn UnitSide,
    ) -> Result<(), GenieError> {
        if side.seen_unit(attacker)?.is_none() {
            return Ok(());
        }
        let (classes, radius) = self.search_area(at, computer)?;
        let found = side.closest(&Search {
            at: [at.position[0], at.position[1]],
            radius,
            classes,
            size: at.size,
            pick: Pick::Look,
            memory: Memory::default(),
        })?;
        self.attack_search_alert(found, computer)?;
        side.call_for_help(attacker)
    }

    /// What a search of the unit AI covers (`FUN_140c63e20`): the classes its constructor gave
    /// it ([`cared_classes`]), within its search radius, the line of sight plus 1 if that is
    /// farther, plus its growth (`+0x1e8`, `FUN_140c2e080`'s fourth argument: 7 for the villager
    /// and the fishing ship, 0 for the others).
    ///
    /// # Errors
    ///
    /// A unit AI type whose classes are not held.
    fn search_area(
        &self,
        at: &Surroundings,
        computer: bool,
    ) -> Result<(&'static [i32], f32), GenieError> {
        search_area(self.ai_type, at.class, at.los, at.search_radius, computer)
    }

    /// The villager's cases 0x1fb, 0x1fc and 0x202 off the attack action (`FUN_140c22bf0`,
    /// "Action %d has been invalidated or requires a search."): on the hunt action the
    /// hunter's search ([`UnitAi::hunter_search`]), on the gather action the gatherer's
    /// ([`UnitAi::gatherer_search`]); on the build action the builder looks for another
    /// foundation (`FUN_140c1c7c0`, when the player's `+0x613` is 0: unsaved, 0 in this game):
    /// one the unit reaches within 0.1 (slot 0x3d0) it builds (slot 0xf0 with 1) and the
    /// processing breaks; finding none, or not reaching it, a human player's villager stops
    /// (slot 0x3c0) and the processing breaks. On the explore action the explorer looks for the
    /// closest undiscovered tile ([`closest_undiscovered_tile`]). The shepherd's search (action
    /// `0x278`: `ClosestObject` for livestock, then slot 600) and the search for the class of
    /// an invalidated gather or hunt (`p[0]` `0x261` or `0x265`, `p[1]` not 10) are not
    /// modelled. Else any notify but 0x202, off the actions `0x278` and `0x26a`, makes the
    /// target go (`FUN_140c5e1e0`) and the action stop (slot 0xb0 with 1; slot 0x48 hands the
    /// notify to the owner, not held); it returns 2.
    fn villager_search(
        &mut self,
        n: Notify,
        at: &Surroundings,
        computer: bool,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        match self.current_action {
            HUNT_ACTION => return self.hunter_search(n, at, computer, side, fx),
            SHEPHERD_ACTION => return Err(VILLAGER_NOTIFY.with("action", SHEPHERD_ACTION)),
            GATHER_ACTION => return self.gatherer_search(n, at, computer, side, fx),
            BUILD_ACTION => {
                let found = side.foundation_search()?;
                if found != -1 && side.reaches(found, FOUNDATION_REACH)? {
                    let _ = self.build_object(found, true, at, side, fx)?;
                    return Ok(BREAK);
                }
                if computer {
                    return Err(GenieError::unsupported(
                        "a computer builder without a foundation",
                    ));
                }
                fx.push(Effect::Stop);
                return Ok(BREAK);
            }
            EXPLORE_ACTION => closest_undiscovered_tile()?,
            _ => {}
        }
        if matches!(n.p[0], GATHER_ACTION | HUNT_ACTION) && n.p[1] != 10 {
            return Err(VILLAGER_NOTIFY.with("notify", format!("{:#x} {:?}", n.kind, n.p)));
        }
        if n.kind != REQUIRES_SEARCH && !matches!(self.current_action, SHEPHERD_ACTION | 0x26a) {
            self.remove_target();
            self.stop_current(true, at, fx)?;
        }
        Ok(GO_ON)
    }

    /// The villager's damage notify (`FUN_140c22bf0`, case 500, "I took some damage from Unit
    /// #%d"): the unit's group (`+0x6d0`, `FUN_140c55c00`) would hear of it, and the slice holds
    /// none. At no hit points left (`p[1]` below 1) it gives up: the owner's alert (slot 0x2e8
    /// with `0x209`, the human player's), the notifies end (`FUN_140c5dc10`) and it returns 4.
    /// An attacker the world does not hold ends it, as does a player who is out (`+0x89` 2; not
    /// held, 0 in every player probed). A computer's unit AI (slot 0x50) is not modelled. Then the
    /// owner's alert (slot 0x2e8 with `0x201`); the attacker joins its attackers
    /// (`FUN_140b43f30`), and a new one that is not its target nor a class-10 attacker of
    /// combat level 1 (`FUN_140fde4f0`) makes it call for help ([`UnitAi::ask_for_help`]); the
    /// unit is under attack (`FUN_14119d2d0` with 1). It ignores the damage ("I have an
    /// attack/hunt/repair order") on a move or enter order while it animates (slot 0x1f8 above
    /// 0), the attack or the repair order; on the hunt, gather and boar luring orders with its
    /// top action in state 12 (slot 0x2b0), a target of class 10 the world holds and a Gaia
    /// attacker; or on those orders from a class-10 attacker of combat level 1 with its top
    /// action in state 4, 8, 12 or 15 (`0x9110`). A Gaia attacker, or another player's the
    /// villager does not fear ([`UnitSide::harmless`]), it attacks back: with an order not on
    /// the attacker, its current order goes back into the queue at once
    /// ([`UnitAi::requeue_current`]) and the order becomes the attack (700, the last order kept)
    /// at priority 100; then the civilian's `AttackObject` (slot 0xb8, `FUN_140c17370`, forced):
    /// `HuntObject` ([`UnitAi::hunt_object`]) on classes 9, 10 and 15, else `AttackObject`; then
    /// `+0x1a8` is set and it breaks off. From any other it evades ("I have %d attackers,
    /// evading them."), but on the move action (`0x262`) at a speed other than 0: its current
    /// order goes to the queue's end ([`UnitAi::queue_current`]), its action stops (slot 0xb0
    /// with 1) and it runs from its attackers ([`UnitAi::run_away`] with 1). Both return 3.
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
    ) -> Result<u32, GenieError> {
        if n.p[1] < 1 {
            self.end_notifies(time, at, fx)?;
            return Ok(DEAD);
        }
        let Some(attacker) = at.unit(n.caller).copied() else {
            return Ok(GO_ON);
        };
        if computer {
            return Err(GenieError::unsupported(
                "a computer villager's damage notify (its slot 0x50)",
            ));
        }
        let added = self.add_attacker(n.caller);
        let fierce = attacker.class == CLASS_10 && attacker.combat_level == 1;
        if added && self.current_target != n.caller && !fierce {
            self.ask_for_help(n.caller, at, computer, side)?;
        }
        side.set_under_attack(true);
        let order = self.current_order;
        let state = held_action_state(at.action_state)?;
        let gathers = matches!(
            order,
            crate::ai_kinds::ORDER_HUNT
                | crate::ai_kinds::ORDER_GATHER
                | crate::ai_kinds::ORDER_AUTO_BOAR_LURING
        );
        let moves = matches!(
            order,
            crate::ai_kinds::ORDER_MOVEMENT | crate::ai_kinds::ORDER_ENTER
        ) && 0.0 < at.anim_speed.ok_or(NO_ANIM_SPEED)?;
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
        if attacker.owner != 0 && !side.harmless(n.caller)? {
            // "I have %d attackers, evading them.", but on the move action at a speed (not 0,
            // UCOMISS: a NaN keeps moving): the current order goes to the queue's end (slot 0x30
            // neither immediate nor in front), the action stops (slot 0xb0 with 1) and it runs
            // from its attackers (slot 0x160 with 1 and no unit)
            if self.current_action == MOVE_ACTION && at.anim_speed.ok_or(NO_ANIM_SPEED)? != 0.0 {
                return Ok(BREAK);
            }
            self.queue_current(at)?;
            self.stop_current(true, at, fx)?;
            let _ = self.run_away(true, at, computer, ai_rng, side, fx)?;
            return Ok(BREAK);
        }
        // "I have a GAIA attacker" (or a harmless one): with an order not on the attacker, the current order goes back
        // into the queue at once (slot 0x30) and the order becomes the attack (700,
        // `FUN_140c64b60` with 0: the last order kept) at priority 100
        if self.current_order != -1 && n.caller != self.current_target {
            self.requeue_current(at, side, computer, ai_rng, fx)?;
            self.set_order_with(crate::ai_kinds::ORDER_ATTACK, false, ai_rng);
            self.order_priority = 100;
        }
        // slot 0xb8 forced, the civilian's
        let _ = self.civilian_attack(n.caller, at, side, fx)?;
        self.stop_after_kill = true;
        Ok(BREAK)
    }

    /// The transport ship's damage notify (`FUN_140c2a580`, cases 500 and a missile's warning
    /// it did not dodge, "I took some damage from Unit #%d"): the unit's group only logs it
    /// (`FUN_140c55c00`, `FUN_141027d60`). At no hit points left (`p[1]` below 1) it gives up
    /// ("I'm dead, I give up."): the owner's alert (slot 0x2e8 with 0x209, [`alert_owner`]),
    /// the notifies end (`FUN_140c5dc10`) and it returns 4. Else the owner's alert (0x201); a
    /// new attacker (`FUN_140bf4cb0`) that is not its target makes a human player's ship call
    /// for help ([`call_for_help`]); the attacker joins the set (`FUN_140b43f30`) and the unit
    /// is under attack (`FUN_14119d2d0` with 1). With an order it ignores the damage ("I have an
    /// order, I'm ignoring this damage notify.", 2); the siege tower's (`FUN_140c28400`) also
    /// when a human player's tower is in a group (`+0x6d0`) that is not idle (`FUN_141017ad0`:
    /// "I have a group that won't let me move and a human owner"). Else, with no action or its
    /// top action in state 2, it runs from its attackers (slot 0x160 with 1,
    /// [`UnitAi::run_away`]) and returns 3.
    #[allow(clippy::too_many_arguments)]
    fn transport_damaged(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        alert_owner(computer)?;
        if n.p[1] < 1 {
            self.end_notifies(time, at, fx)?;
            return Ok(DEAD);
        }
        let new = self.attackers.binary_search(&n.caller).is_err();
        if new && !computer && self.current_target != n.caller {
            self.ask_for_help(n.caller, at, computer, side)?;
        }
        let _ = self.add_attacker(n.caller);
        side.set_under_attack(true);
        if self.current_order != -1 {
            return Ok(GO_ON);
        }
        let held_by_group = at.group.is_some_and(|g| !g.idle) && !computer;
        if self.ai_type == SIEGE_TOWER_AI && held_by_group {
            return Ok(GO_ON);
        }
        if !at.busy || held_action_state(at.action_state)? == 2 {
            let _ = self.run_away(true, at, computer, ai_rng, side, fx)?;
        }
        Ok(BREAK)
    }

    /// The siege unit's test on its executed action (`FUN_140c287e0`, case 0x200, a computer
    /// player's): a weapon range of at least 1 (`COMISS`, `JA`: an unordered one passes), a target
    /// class (`+0x4c`) other than 3, 27, 39, 49 and 4, then a draw (`processNotify`) whose
    /// `r * 100 / 0x7fff` is at least the owner's `chance_for_archers_to_maintain_distance`.
    fn keeps_distance(&self, at: &Surroundings, ai: &mut Well512) -> bool {
        if !crate::fmath::at_most_or_unordered(1.0, at.weapon_range)
            || matches!(self.target_type, 3 | 27 | 39 | 49 | 4)
        {
            return false;
        }
        let r = ai.rand15() as i32;
        i32::from(at.keep_distance_chance) <= r * 100 / 0x7fff
    }

    /// The trade ship's case 0x202 (`FUN_140c29bf0`, "Action %d has failed, been invalidated,
    /// or requires a search."): off the trade action an explorer (0x25d) looks for the closest
    /// undiscovered tile ([`closest_undiscovered_tile`]); otherwise the target goes
    /// (`FUN_140c5e1e0`), the action stops (slot 0xb0 with 1; slot 0x48 hands the notify to the
    /// owner, not held) and it returns 2. On the trade action the trader looks for another
    /// partner, the dock (45) for a trade boat, else the market (84): `ClosestObject` for its
    /// own, then a neutral's, each followed by slot 0xf8's trade command (the unit's slot 0x390
    /// and its Trade action, which the slice does not hold), 6 once one takes.
    fn trader_search(
        &mut self,
        at: &Surroundings,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        if self.current_action == TRADE_ACTION {
            return Err(GenieError::unsupported(
                "a trader's search for another dock or market (slot 0xf8, its Trade action)",
            ));
        }
        if self.current_action == EXPLORE_ACTION {
            closest_undiscovered_tile()?;
        }
        self.remove_target();
        self.stop_current(true, at, fx)?;
        Ok(GO_ON)
    }

    /// The trade ship's damage notify (`FUN_140c29bf0`, cases 500 and a missile's warning it
    /// did not dodge): at no hit points left it gives up (the owner's alert 0x209, the notifies
    /// end, 4). Else the owner's alert (0x201) only with no attackers yet or a computer owner; a
    /// new attacker that is not its target makes a human player's ship call for help
    /// ([`call_for_help`]); the attacker joins the set and the unit is under attack. With an
    /// order it ignores the damage (2), but for a computer player's with `+0x621` set, which
    /// cannot get here (its alert stops the handler first, [`alert_owner`]). Off the move
    /// action (`0x262`) its order goes to the queue's end (slot 0x30, not immediate:
    /// `FUN_140c55c90` takes none of −1) and its action stops (slot 0xb0 with 1); then it runs
    /// from its attackers (slot 0x160 with 1) and returns 3.
    #[allow(clippy::too_many_arguments)]
    fn trade_damaged(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        if n.p[1] < 1 {
            alert_owner(computer)?;
            self.end_notifies(time, at, fx)?;
            return Ok(DEAD);
        }
        if self.attackers.is_empty() || computer {
            alert_owner(computer)?;
        }
        let new = self.attackers.binary_search(&n.caller).is_err();
        if new && !computer && self.current_target != n.caller {
            self.ask_for_help(n.caller, at, computer, side)?;
        }
        let _ = self.add_attacker(n.caller);
        side.set_under_attack(true);
        if self.current_order != -1 {
            return Ok(GO_ON);
        }
        if self.current_action != MOVE_ACTION {
            self.queue_current(at)?;
            self.stop_current(true, at, fx)?;
        }
        let _ = self.run_away(true, at, computer, ai_rng, side, fx)?;
        Ok(BREAK)
    }

    /// The fishing ship's damage notify (`FUN_140c25ca0`, cases 500 and a missile's warning it
    /// did not dodge): as the transport ship's up to its attackers ([`UnitAi::transport_damaged`]);
    /// a human player's ship with an order ignores the damage (2; a computer player's cannot get
    /// here, its alert stops the handler first). Off the move action (`0x262`) its order goes to
    /// the queue's end (slot 0x30, not immediate), its action stops (slot 0xb0 with 1) and it
    /// runs from its attackers (slot 0x160 with 1). It returns 3.
    #[allow(clippy::too_many_arguments)]
    fn fishing_damaged(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        alert_owner(computer)?;
        if n.p[1] < 1 {
            self.end_notifies(time, at, fx)?;
            return Ok(DEAD);
        }
        let new = self.attackers.binary_search(&n.caller).is_err();
        if new && !computer && self.current_target != n.caller {
            self.ask_for_help(n.caller, at, computer, side)?;
        }
        let _ = self.add_attacker(n.caller);
        side.set_under_attack(true);
        if self.current_order != -1 {
            return Ok(GO_ON);
        }
        if self.current_action != MOVE_ACTION {
            self.queue_current(at)?;
            self.stop_current(true, at, fx)?;
            let _ = self.run_away(true, at, computer, ai_rng, side, fx)?;
        }
        Ok(BREAK)
    }

    /// The fishing ship's case 0x202 on its build (`FUN_140c25ca0`, action `0x25a`: "I'm a
    /// builder, looking around for other objects of type %d to build."): `ClosestObject` for its
    /// own foundations (relation 3, state 0, reachable) of each class of [`FISHING_BUILDS`] over
    /// its search ([`FISHING_CARED`], its radius grown by its `+0x1e8`, 7); the nearest by the
    /// distance each answers, below [`FISHING_BUILD_FAR`], that the unit reaches within
    /// `FOUNDATION_REACH` (slot 0x3d0) it builds (slot 0xf0 with 1, [`Effect::BuildObject`]) and
    /// the processing breaks. Else a human player's ship goes on (2); a computer player's posts
    /// itself its build's completion (slot 0x328: `0x1fa` with `0x25a`) first.
    fn fishing_builder_search(
        &mut self,
        at: &Surroundings,
        computer: bool,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        let mut best = (-1, FISHING_BUILD_FAR);
        for class in FISHING_BUILDS {
            let found = side.closest(&Search {
                at: [at.position[0], at.position[1]],
                radius: civilian_search_radius(at.los, at.search_radius),
                classes: &FISHING_CARED,
                size: at.size,
                pick: Pick::Closest {
                    group: class,
                    relation: 3,
                    state: 0,
                    master: -1,
                    workers: -1,
                    spared: -1,
                    reach: true,
                },
                memory: Memory::default(),
            })?;
            self.alert(found, at.dt_seconds);
            if found.id != -1 && found.distance < best.1 {
                best = (found.id, found.distance);
            }
        }
        if best.0 != -1 && side.reaches(best.0, FOUNDATION_REACH)? {
            let _ = self.build_object(best.0, true, at, side, fx)?;
            return Ok(BREAK);
        }
        if computer {
            self.post(
                Notify::own(at.id, ACTION_COMPLETED, [BUILD_ACTION, 0, 0]),
                at.state,
            )?;
        }
        Ok(GO_ON)
    }

    /// Slot 0x30 with the current order, not immediate and not in front (`FUN_140c55c90`, as the
    /// ships' damage notifies call it: the issuer the unit, the order, its target and class,
    /// owner −1, the target point, the range `+0x5c`, the priority `+0x40`, the point shown,
    /// ready): a unit above state 2 or without an order takes none; else the order goes to the
    /// queue's end ([`UnitAi::queue_order`]).
    fn queue_current(&mut self, at: &Surroundings) -> Result<(), GenieError> {
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
        self.queue_order(at.state, record, false, false).map(|_| ())
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
    ) -> Result<u32, GenieError> {
        let new = self.attackers.binary_search(&n.caller).is_err();
        alert_owner(computer)?;
        if n.p[1] >= 1 {
            alert_owner(at.guest_computer)?;
        }
        if n.p[1] >= 1 && new && self.current_target != n.caller {
            self.ask_for_help(n.caller, at, computer, side)?;
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
    ) -> Result<(), GenieError> {
        self.stop_current(true, at, fx)?;
        // the unit's stopAction goes with its hunt command, which runs it first
        if fx.pop() != Some(Effect::Stop) {
            return Err(GenieError::unsupported(
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
        side: &mut dyn UnitSide,
        computer: bool,
        ai: &mut Well512,
        fx: &mut Vec<Effect>,
    ) -> Result<(), GenieError> {
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
        self.order_now(
            record,
            Some(at.id),
            Some(place),
            Some((at, side, computer)),
            ai,
            fx,
        )
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
    /// without `world`, where a completion it would stop on is not modelled), the queue's first
    /// order runs at once (slot 0x230, [`UnitAi::process_order`] with `world`, the surroundings
    /// and the unit) and, unless it was not ready, leaves the queue (`FUN_140c5e280`).
    fn order_now(
        &mut self,
        order: QueuedOrder,
        keep_from: Option<i32>,
        place: Option<([f32; 2], i16)>,
        world: Option<(&Surroundings, &mut dyn UnitSide, bool)>,
        ai: &mut Well512,
        fx: &mut Vec<Effect>,
    ) -> Result<(), GenieError> {
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
        match &world {
            Some((at, _, _)) => self.end_notifies(0, at, fx)?,
            None => self.clear_notifies(0)?,
        }
        let front = self.orders[0];
        if self.process_order(&front, world, place, ai, fx)? != 0xe {
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
    /// action fails (false). The target must pass [`UnitAi::attack_check`] with `any`; then the
    /// unit's attack command ([`UnitAi::attack_command`]), on success the action is the attack
    /// (600, the last action keeping the old) and the target its point and class
    /// (`FUN_140c64c10`). The group flag's attack by the unit's group (behind the byte
    /// `DAT_143f01835`, which nothing writes) cannot run.
    fn attack_object(
        &mut self,
        target: i32,
        force: bool,
        any: bool,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        if !force && self.current_action != -1 {
            return Ok(false);
        }
        let Some((class, position)) = self.attack_check(target, any, at, side)? else {
            return Ok(false);
        };
        if !self.attack_command(target, force, at, side, fx)? {
            return Ok(false);
        }
        self.set_action(ATTACK_ACTION);
        self.target_at(target, Some(i32::from(class)), position);
        Ok(true)
    }

    /// `AttackObject`'s test of `target` (`FUN_140c43720`, and `attackRoundupObject`'s): the world
    /// must hold it and its slot 0xa0 with `any` must let it attack the target: the base's
    /// (`FUN_140c48400`: the boar's, the ranged units', the ships', the artifact's, the siege
    /// tower's and the livestock's) anything, the building's (`FUN_140c18b60`) nothing, the
    /// lion's ([`lion_may_attack`]), the civilian's ([`civilian_may_attack`]), the soldier's, the
    /// tower's and the siege unit's ([`UnitSide::may_attack`]); the hunted animal's
    /// (`FUN_140c18d00`) reads its action list (slot 0x538) and the priest's (`FUN_140c19020`) its
    /// conversions: not modelled. Its master's class and its place then.
    fn attack_check(
        &self,
        target: i32,
        any: bool,
        at: &Surroundings,
        side: &mut dyn UnitSide,
    ) -> Result<Option<(i16, [f32; 3])>, GenieError> {
        if matches!(self.ai_type, 13 | 15 | SIEGE_AI) {
            return side.may_attack(target, &self.seeker(at));
        }
        let Some(t) = Self::world_object(target, at, side)? else {
            return Ok(None);
        };
        let may = match self.ai_type {
            // FUN_140c48400, the base's: the boar's, the ranged units' (6, 7), the trade
            // ship's, the transport ship's, the fishing ship's, the artifact's, the siege
            // tower's and the livestock's
            4 | 6 | 7 | 10 | 11 | 12 | ARTIFACT_AI | SIEGE_TOWER_AI | 19 => true,
            5 => any || lion_may_attack(at.attack_priority, t.class),
            CIVILIAN => civilian_may_attack(at.attack_priority, t.class),
            // FUN_140c18b60
            17 => false,
            _ => {
                return Err(GenieError::unsupported(
                    "AttackObject's slot 0xa0 of a unit AI type not modelled",
                ));
            }
        };
        Ok(may.then_some((t.class, t.position)))
    }

    /// The unit's attack command (slot 0x338, [`UnitSide::attack`]) on `target`, `force`d or not,
    /// as `AttackObject`, `attackRoundupObject` and seek and destroy give it: a stop the
    /// processing made before (slot 0x3c0, a pending [`Effect::Stop`]) emptied the action list
    /// before the command fills it, so it runs first ([`UnitSide::stop`]), with the caps on the
    /// last human order time before it ([`UnitSide::cap_human_order`]) in their order. The notifies the
    /// command posts (the unit's slot 0x328 -> the AI's slot 0x38) go into the queue, or the
    /// buffer while it is full. True when the unit attacks: it then holds an action of its own
    /// ([`UnitAi::commanded`]).
    fn attack_command(
        &mut self,
        target: i32,
        force: bool,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        if fx.contains(&Effect::Stop) {
            if fx.iter().any(|e| {
                !matches!(
                    e,
                    Effect::Stop
                        | Effect::CapHumanOrder(_)
                        | Effect::BuildObject(_)
                        | Effect::HuntObject(_)
                        | Effect::Move(_)
                )
            }) {
                return Err(PENDING_BEFORE_ATTACK.with("effects", format!("{fx:?}")));
            }
            // the effects before it ran before it: in order (a requeued build or hunt order's
            // command too, whose action the attack command replaces)
            for e in std::mem::take(fx) {
                match e {
                    Effect::CapHumanOrder(cap) => side.cap_human_order(cap)?,
                    Effect::BuildObject(foundation) => side.build_now(foundation)?,
                    Effect::HuntObject(quarry) => side.hunt_now(quarry, self)?,
                    Effect::Move(m) => side.ai_move_now(m)?,
                    _ => side.stop()?,
                }
            }
        }
        let attacked = side.attack(target, force, self)?;
        for (kind, p) in attacked.posted {
            self.post(Notify::own(at.id, kind, p), at.state)?;
        }
        if attacked.attacks {
            self.commanded = true;
        }
        Ok(attacked.attacks)
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
    ) -> Result<bool, GenieError> {
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
    ) -> Result<u32, GenieError> {
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
                        self.set_idle_timeout(LION_ATTACKED_TIMEOUT, ai_rng);
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
            // 0x3c0) and the processing breaks. On the attack action (600) an invalidation, a
            // moved target or a search (0x1fb, 0x1fc, 0x202) is the base handler's ("My attack
            // target's moved, just doing normal target moved processing."), but a moved target
            // with `+0x1a8` set: the unit's slot 0x328 posts the attack's failure to the AI
            // (0x1f9 with 600, from and to the unit) and processing goes on
            8 => match n.kind {
                ACTION_INVALIDATED | TARGET_MOVED | REQUIRES_SEARCH
                    if self.current_action == ATTACK_ACTION =>
                {
                    if n.kind == TARGET_MOVED && self.stop_after_kill {
                        let failed = Notify::own(at.id, ACTION_FAILED, [ATTACK_ACTION, 0, 0]);
                        self.post(failed, at.state)?;
                        return Ok(GO_ON);
                    }
                    self.base_notify(n, time, at, computer, ai_rng, side, fx)
                }
                ACTION_INVALIDATED | TARGET_MOVED | REQUIRES_SEARCH => {
                    self.villager_search(n, at, computer, side, fx)
                }
                // a completion or a failure of its action (0x1fa, 0x1f9) stops it
                ACTION_COMPLETED | ACTION_FAILED => {
                    if self.current_action == n.p[0] {
                        self.remove_target();
                        self.stop_current(true, at, fx)?;
                    }
                    Ok(GO_ON)
                }
                ACTION_EXECUTED if computer => Err(GenieError::unsupported(
                    "a computer villager's executed action",
                )),
                ACTION_EXECUTED => self.base_notify(n, time, at, computer, ai_rng, side, fx),
                // a missile's warning it does not dodge (the player's `+0xe10`) is its damage
                MISSILE_WARNING if dodges(at, ai_rng) => Ok(GO_ON),
                DAMAGED | MISSILE_WARNING => {
                    self.villager_damaged(n, time, at, computer, ai_rng, side, fx)
                }
                // off the build or the hunt action, the object its action made (a pasture,
                // `p[1]`) which the world holds: the unit works it at its place (slot 0x7e8,
                // the caller's)
                PASTURE_BUILT => {
                    if matches!(n.p[0], BUILD_ACTION | HUNT_ACTION)
                        && let Some(made) = Self::world_object(n.p[1], at, side)?
                    {
                        fx.push(Effect::WorkOn {
                            target: made.id,
                            point: made.position,
                        });
                    }
                    Ok(GO_ON)
                }
                // a load of another resource asked off (FUN_141658e20 from its place, its slot
                // 0x128 on the site, its slot 0x30 back to its action's target: the caller's)
                OTHER_LOAD => {
                    // slot 0x190 first: its top action's target
                    let back = top_target_after(fx, side)?;
                    if let Some(site) = side.table_drop_site()? {
                        fx.push(Effect::DropElsewhere { site, back });
                    }
                    Ok(GO_ON)
                }
                // its own cases the slice does not model; any other is the base handler's
                0x217..=0x21a => {
                    Err(VILLAGER_NOTIFY.with("notify", format!("{:#x} {:?}", n.kind, n.p)))
                }
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the transport ship (FUN_140c2a580): its damage notify, and a missile's warning it
            // takes as damage unless it dodges; any other notify is the base handler's
            11 => match n.kind {
                MISSILE_WARNING if dodges(at, ai_rng) => Ok(GO_ON),
                DAMAGED | MISSILE_WARNING => {
                    self.transport_damaged(n, time, at, computer, ai_rng, side, fx)
                }
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the trade ship and cart (FUN_140c29bf0): its search when its action needs one, its
            // damage notify, a missile's warning it takes as damage unless it dodges
            TRADE_AI => match n.kind {
                REQUIRES_SEARCH => self.trader_search(at, fx),
                MISSILE_WARNING if dodges(at, ai_rng) => Ok(GO_ON),
                DAMAGED | MISSILE_WARNING => {
                    self.trade_damaged(n, time, at, computer, ai_rng, side, fx)
                }
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the fishing ship (FUN_140c25ca0): a builder's search for more to build, its damage
            // notify, a missile's warning it takes as damage unless it dodges
            FISHING_AI => match n.kind {
                REQUIRES_SEARCH if self.current_action == BUILD_ACTION => {
                    self.fishing_builder_search(at, computer, side, fx)
                }
                MISSILE_WARNING if dodges(at, ai_rng) => Ok(GO_ON),
                DAMAGED | MISSILE_WARNING => {
                    self.fishing_damaged(n, time, at, computer, ai_rng, side, fx)
                }
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the siege unit (FUN_140c287e0): a computer player's packed unit's damage notify; a
            // target under its least range (0x1fe) makes it back up, as does a computer
            // player's ranged unit's executed attack on a drawn chance (0x200); any other notify
            // is the base handler's
            SIEGE_AI => match n.kind {
                DAMAGED if at.class == PACKED_UNIT_CLASS && computer => {
                    if n.p[1] < 1 {
                        alert_owner(computer)?;
                        self.end_notifies(time, at, fx)?;
                        return Ok(DEAD);
                    }
                    // FUN_140c54e30: an attacker the world does not hold ends it
                    if at.unit(n.caller).is_none() {
                        return Ok(GO_ON);
                    }
                    self.stop_current(true, at, fx)?;
                    let _ = self.run_away(true, at, computer, ai_rng, side, fx)?;
                    Ok(BREAK)
                }
                0x1fe => back_up(),
                ACTION_EXECUTED if computer && self.keeps_distance(at, ai_rng) => {
                    if self.current_action == ATTACK_ACTION {
                        back_up()
                    } else {
                        Ok(GO_ON)
                    }
                }
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the siege tower (FUN_140c28400): as the transport ship's, but a human player's
            // tower in a group that is not idle ignores the damage
            SIEGE_TOWER_AI => match n.kind {
                MISSILE_WARNING if dodges(at, ai_rng) => Ok(GO_ON),
                DAMAGED | MISSILE_WARNING => {
                    self.transport_damaged(n, time, at, computer, ai_rng, side, fx)
                }
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the soldier (FUN_140c28bb0): its own cases are the computer's, a backing-up and a
            // healer's search
            13 => match n.kind {
                0x1fe => back_up(),
                ACTION_EXECUTED | 0x20f if computer => {
                    Err(GenieError::unsupported("a computer soldier's notify"))
                }
                0x202 if self.current_action == 0x25b => {
                    Err(GenieError::unsupported("a healer's search notify"))
                }
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the tower (FUN_140c299f0)
            15 => match n.kind {
                // "I'm dead, I give up.": the owner's alert (slot 0x2e8 with 0x209: a human
                // player's alert list and the old pather's hulled-area reset, not held; a
                // computer player's AI hears it), the notifies end and it gives up. Alive, the
                // other owners of the units inside it hear 0x201 and the base handler's damage
                // case follows
                DAMAGED if n.p[1] < 1 => {
                    alert_owner(computer)?;
                    self.end_notifies(time, at, fx)?;
                    Ok(DEAD)
                }
                DAMAGED => {
                    alert_owner(at.guest_computer)?;
                    self.damaged(n, time, at, computer, ai_rng, side, fx)
                }
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
                0x202 => Err(GenieError::unsupported(
                    "a livestock's notify 0x202 (its settings' test)",
                )),
                _ => {
                    self.set_action(-1);
                    Ok(BREAK)
                }
            },
            ARTIFACT_AI => self.artifact_notify(n, time, at, computer, ai_rng, side, fx),
            _ => Err(GenieError::unsupported(
                "the notify handler of a unit-AI type not modelled",
            )),
        }
    }

    /// The artifact's notify handler (slot 0x238, `FUN_140c21f70`): the kinds it lets be (500,
    /// 0x1f6 to 0x1f8, 0x1fd to 0x200, 0x202, 0x203, 0x20b, 0x20f, 0x259 to 0x262, 0x265 to
    /// 0x269, 0x278, 699 and 0x2be) go on; a unit attacking it (600) joins its attackers
    /// (`FUN_140b43f30`) and puts its unit under attack (`FUN_14119d2d0` with 1); a unit sent off
    /// a footprint (0x216, 0x21b) is the base handler's. A completion, failure or invalidation
    /// of its action (without an order its action value, with one the order's action: action +
    /// 100 = order) and a moved target the world no longer holds or that its owner no longer
    /// sees (`FUN_1416635e0` not 0xf) end in its stop, as any other kind does ("Unprocessed
    /// notify"): the target goes (`FUN_140c5e1e0`), the action stops (slot 0xb0 with 1) and slot
    /// 0x48 hands the notify to the owner (its alerts, not held). Each answers 2.
    #[allow(clippy::too_many_arguments)]
    fn artifact_notify(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        let stops = match n.kind {
            500
            | 0x1f6..=0x1f8
            | 0x1fd..=0x200
            | 0x202
            | 0x203
            | OWNER_CHANGED
            | 0x20f
            | 0x259..=0x262
            | 0x265..=0x269
            | 0x278
            | 699
            | 0x2be => false,
            ACTION_FAILED | ACTION_COMPLETED | ACTION_INVALIDATED => {
                if self.current_order == -1 {
                    self.current_action == n.p[0]
                } else {
                    self.current_action + 100 == self.current_order
                }
            }
            // "Target is still visible.": the world holds the target, which its owner sees
            TARGET_MOVED => !at
                .target
                .is_some_and(|t| self.current_target != -1 && t.sight == 0xf),
            SENT_OFF | SENT_OFF_GROUP => {
                return self.base_notify(n, time, at, computer, ai_rng, side, fx);
            }
            600 => {
                let _ = self.add_attacker(n.caller);
                side.set_under_attack(true);
                false
            }
            _ => true,
        };
        if stops {
            self.remove_target();
            self.stop_current(true, at, fx)?;
        }
        Ok(GO_ON)
    }

    /// The artifact's processIdle (slot 72, `FUN_140c1e410`, factory type 14): `ClosestObject`
    /// (`FUN_140c488b0`) of any class it cares about, relation, state, master and workers from
    /// its place, the reach test only for a unit that moves (slot 0x208 above 0), which sets the
    /// alert mode ([`UnitAi::alert`]). While the lists hold the objects of at most one player
    /// but Gaia (`DAT_143e6f92c` below 2, [`Found::players`]), the object found, when the world
    /// holds it and another player owns it, takes the artifact over: the unit's slot 0x70 to its
    /// owner ([`Effect::ChangeOwner`]), and processIdle produced (6, true); else 5.
    fn artifact_idle(
        &mut self,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        let found = side.closest(&Search {
            at: [at.position[0], at.position[1]],
            radius: search_radius(at.los, at.search_radius),
            classes: &ARTIFACT_CARED,
            size: at.size,
            pick: Pick::Closest {
                group: -1,
                relation: -1,
                state: -1,
                master: -1,
                workers: -1,
                spared: -1,
                reach: 0.0 < at.speed,
            },
            memory: Memory::default(),
        })?;
        self.alert(found, at.dt_seconds);
        if found.players < 2
            && found.id != -1
            && let Some(u) = side.seen_unit(found.id)?
            && u.owner != at.player
        {
            fx.push(Effect::ChangeOwner(u.owner));
            return Ok(true);
        }
        Ok(false)
    }

    /// The artifact's slot 73 (`FUN_140c21d40`): its processIdle (slot 0x240 with 1,
    /// [`UnitAi::artifact_idle`]); once another player took it over (6) its target goes
    /// (`FUN_140c5e1e0`), its action stops (slot 0xb0 with 1) and its idle timer is 0 (it
    /// answers 10), else 9.
    fn artifact_misc(
        &mut self,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<(), GenieError> {
        if self.artifact_idle(at, side, fx)? {
            self.remove_target();
            self.stop_current(true, at, fx)?;
            self.idle_timer = 0;
        }
        Ok(())
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
    ) -> Result<u32, GenieError> {
        match n.kind {
            ACTION_COMPLETED => self.action_completed(n, at, computer, ai_rng, side, fx),
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
            // a MoveTo's moved target: with a saved patrol processIdle runs (slot 0x240 with 1),
            // its result unread
            TARGET_MOVED if self.patrol_path.is_some() => {
                let _ = self.process_idle(at, computer, ai_rng, side, fx)?;
                Ok(GO_ON)
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
            DAMAGED | MISSILE_WARNING => self.damaged(n, time, at, computer, ai_rng, side, fx),
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
            _ => Err(
                GenieError::unsupported("a notify kind the base handler does not model")
                    .with("notify", format_args!("{:#x}", n.kind))
                    .with("caller", n.caller),
            ),
        }
    }

    /// The base handler's damage case (`FUN_140c589c0`, cases 500 and a missile's warning, 0x20f:
    /// "I took some damage from Unit #%d"), the attacker `n.caller`:
    ///
    /// 1. The retarget entries go (`+0x1d8` 0). An attacker already in the set (`FUN_140bf4cb0`)
    ///    of a unit whose top action has a target (slot 400) changes nothing (2).
    /// 2. The owner hears the notify (slot 0x48: slot 0x40, `FUN_140c55b80`, its slot 0x2e8) and
    ///    the unit's group only logs it (`FUN_141027d60`); the owner's alert 0x201
    ///    ([`alert_owner`]); the attacker joins the set (`FUN_140b43f30`) and the unit is under
    ///    attack (`FUN_14119d2d0` with 1). At no hit points left ("I'm dead, I give up.") the
    ///    owner's alert 0x209, the notifies end (`FUN_140c5dc10`) and it returns 4.
    /// 3. It ignores (2) an attacker the world no longer holds or of a player its owner counts as
    ///    its own or an ally. A new attacker that is not its target makes a human player's unit
    ///    outside a group that is not idle ask for help (`AskForHelp`, [`UnitAi::ask_for_help`]).
    ///    It ignores (2): its attack on the attacker; a human player's unit's order other than
    ///    the patrols (711, 733), or its group that is not idle; a computer player's unit on the
    ///    explore order (0x2c1) attacked by another player than Gaia.
    /// 4. `bestUnitToAttack` once per update ([`UnitAi::best_this_update`], `+0x1d0`): from the
    ///    current target's place (slot 0x520) while the world holds it, else its own; no
    ///    reference distance, no idle check, no path checks, the attacker as its extra object,
    ///    any class, no attack work state. With none and no target, order or action: a unit that
    ///    does not move (its top speed below [`STILL`]) ignores it (2), another runs from its
    ///    attackers (slot 0x160 with 1, [`UnitAi::run_away`]): 3 when it moves, else 2. Anything
    ///    but the attacker is ignored (2).
    /// 5. Its reach to the attacker within its weapon range (slot 0x3d0 with slot 0x230). Without
    ///    a current target the world holds: reached, it attacks back ([`UnitAi::strike_back`]);
    ///    else off the move action it runs from its attackers; 3 either way. With one: not
    ///    reached, 2; a human player's unit attacks back from a building, wall or tower target
    ///    (classes 3, 27, 52; 3), else 2.
    /// 6. A computer player's unit ignores it (2) on a wonder target (master 276), a tower
    ///    attacked by a tower (class 52), a missile attacker (type 60), and a tower attacker
    ///    under its owner's `disableTowerPriority` but for a siege weapon, unpacked trebuchet or
    ///    warship (classes 13, 54, 22). Then ([`UnitSide::duel`]) a target that cannot convert it
    ///    (its slot 0x1b8 with 1) but cannot harm it either (its attacks' sum, slot 0x228, or its
    ///    damage to the unit, the unit's slot 0x220, exactly 0) makes it ask for help and attack
    ///    back; and then still, as long as the target cannot convert it, an attacker that would
    ///    die sooner (its hit points over the unit's damage to it, [`NEVER_KILLED`] for none:
    ///    `COMISS`, `JA`) or could convert it makes it attack back. It returns 2.
    #[allow(clippy::too_many_arguments)]
    fn damaged(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        let attacker_id = n.caller;
        let known = self.attackers.binary_search(&attacker_id).is_ok();
        self.retries.clear();
        if known && top_target_after(fx, side)?.is_some() {
            return Ok(GO_ON);
        }
        // slot 0x48's notify and the alert 0x201 reach the same owner
        alert_owner(computer)?;
        let _ = self.add_attacker(attacker_id);
        side.set_under_attack(true);
        if n.p[1] < 1 {
            self.end_notifies(time, at, fx)?;
            return Ok(DEAD);
        }
        let Some(attacker) = Self::world_object(attacker_id, at, side)? else {
            return Ok(GO_ON);
        };
        if attacker.allied {
            return Ok(GO_ON);
        }
        // `IsIdle` (FUN_141017ad0) of the unit's group, read at the handler's start
        let held_by_group = at.group.is_some_and(|g| !g.idle);
        if !known && !computer && self.current_target != attacker_id && !held_by_group {
            self.ask_for_help(attacker_id, at, computer, side)?;
        }
        if self.current_action == ATTACK_ACTION && self.current_target == attacker_id {
            return Ok(GO_ON);
        }
        if !computer && !matches!(self.current_order, -1 | PATROL | ATTACK_MOVE) {
            return Ok(GO_ON);
        }
        if !computer && held_by_group {
            return Ok(GO_ON);
        }
        if computer && self.current_order == crate::ai_kinds::ORDER_EXPLORE && attacker.owner != 0 {
            return Ok(GO_ON);
        }
        let current = if self.current_target == -1 {
            None
        } else {
            Self::world_object(self.current_target, at, side)?
        };
        if self.best_this_update == -1 {
            let from = current.map_or(at.position, |t| t.position);
            let (classes, radius) = self.search_area(at, computer)?;
            let found = side.closest(&Search {
                at: [from[0], from[1]],
                radius,
                classes,
                size: at.size,
                pick: Pick::Attack {
                    lowest: if at.scenario && !computer { 2 } else { 0 },
                    seeker: self.seeker(at),
                    ask: AttackAsk {
                        distance: 0.0,
                        idle: false,
                        paths: false,
                        extra: Some(attacker_id),
                        any_class: true,
                        work_state: false,
                    },
                },
                memory: self.memory(),
            })?;
            self.add_retries(&found);
            self.attack_search_alert(found, computer)?;
            self.best_this_update = found.id;
        }
        if self.best_this_update == -1
            && self.current_target == -1
            && self.current_order == -1
            && self.current_action == -1
        {
            // "BUTA returned -1 and I'm idle without a target": COMISS, JBE (a NaN runs)
            if at.speed < STILL {
                return Ok(GO_ON);
            }
            let ran = self.run_away(true, at, computer, ai_rng, side, fx)?;
            return Ok(if ran { BREAK } else { GO_ON });
        }
        if self.best_this_update != attacker_id {
            return Ok(GO_ON);
        }
        let reaches = side.reaches(attacker_id, at.weapon_range)?;
        let Some(target) = current else {
            if reaches {
                return self.strike_back(attacker_id, at, ai_rng, side, fx);
            }
            if self.current_action != MOVE_ACTION {
                let _ = self.run_away(true, at, computer, ai_rng, side, fx)?;
            }
            return Ok(BREAK);
        };
        if !reaches {
            return Ok(GO_ON);
        }
        if !computer {
            if matches!(target.class, 3 | 27 | 52) {
                return self.strike_back(attacker_id, at, ai_rng, side, fx);
            }
            return Ok(GO_ON);
        }
        self.computer_weighs(&attacker, &target, at, ai_rng, side, fx)
    }

    /// The damage case's end for a computer player's unit ([`UnitAi::damaged`]'s step 6) that
    /// reaches `attacker` while the world holds its current `target`.
    fn computer_weighs(
        &mut self,
        attacker: &UnitSeen,
        target: &UnitSeen,
        at: &Surroundings,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        let attacker_id = attacker.id;
        if target.master == crate::turn::WONDER
            || (target.class == 52 && attacker.class == 52)
            || attacker.kind == 60
            || (attacker.class == 52
                && !matches!(at.class, 13 | 54 | 22)
                && at.disable_tower_priority)
        {
            return Ok(GO_ON);
        }
        let t = side.duel(target.id)?;
        // "my current target cannot damage me": UCOMISS, JP, JNZ (exactly 0)
        if (t.attacks == 0.0 || t.harms == 0.0) && !t.converts {
            if matches!(self.current_order, PATROL | ATTACK_MOVE) {
                self.save_patrol(side);
            }
            self.ask_for_help(attacker_id, at, true, side)?;
            if self.attack_slot(attacker_id, true, at, side, fx)? {
                self.leave_patrol_timers(ai_rng);
                self.current_order = -1;
            }
        }
        let target_time = if 0.0 < t.harmed {
            t.hp / t.harmed
        } else {
            NEVER_KILLED
        };
        let a = side.duel(attacker_id)?;
        let attacker_time = if 0.0 < a.harmed {
            a.hp / a.harmed
        } else {
            NEVER_KILLED
        };
        // "my current target will take longer to kill"
        if !t.converts && (attacker_time < target_time || a.converts) {
            let _ = self.strike_back(attacker_id, at, ai_rng, side, fx)?;
        }
        Ok(GO_ON)
    }

    /// The damage case's attack back on `attacker` (`FUN_140c589c0`): a unit on a patrol order
    /// (711, 733) saves its patrol first (`savePatrolInformation`, [`UnitAi::save_patrol`]);
    /// `AttackObject` (slot 0xb8, any: [`UnitAi::attack_slot`]); when it takes, a patrol order
    /// lets its timers go ([`UnitAi::leave_patrol_timers`]) and the order is −1. It answers 3.
    fn strike_back(
        &mut self,
        attacker: i32,
        at: &Surroundings,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        if matches!(self.current_order, PATROL | ATTACK_MOVE) {
            self.save_patrol(side);
        }
        if self.attack_slot(attacker, true, at, side, fx)? {
            self.leave_patrol_timers(ai_rng);
            self.current_order = -1;
        }
        Ok(BREAK)
    }

    /// The base handler's case 0x215 ([`IN_THE_WAY`]): a unit not of a boar's master (class 10
    /// of combat level 1, `FUN_140fde4f0`) attacks the unit in its way (`AttackObject`, slot
    /// 0xb8, forced, its slot 0xa0 unforced: [`UnitAi::attack_slot`]) when the world's table
    /// holds it, retargeting is on (`+0x1d6` 0), its stance (`+0x1a9`) is 0 and it has no order;
    /// the other unit's group (`+0x6d0`), which would let it attack with one, the slice does not
    /// hold. Processing goes on.
    fn in_the_way(
        &mut self,
        n: Notify,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        let boar = at.class == 10 && at.combat_level == 1;
        if !boar
            && at.unit(n.p[0]).is_some()
            && self.retarget_disabled == 0
            && self.current_order == -1
            && self.stance == 0
        {
            // slot 0xb8 forced, by the type
            let _ = self.attack_slot(n.p[0], false, at, side, fx)?;
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
    ) -> Result<u32, GenieError> {
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
    /// 54 (`FUN_141216c50`, their packing), which it does not run. The move gives the unit its
    /// `MoveTo` at once: the update's later checks see it busy. True when it moves.
    fn move_aside(&mut self, m: AiMove, fx: &mut Vec<Effect>) -> bool {
        if !m.force && self.current_action != -1 {
            return false;
        }
        fx.push(Effect::MoveAside(m));
        self.set_action(MOVE_ASIDE_ACTION);
        self.commanded = true;
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
    ) -> Result<u32, GenieError> {
        let action = self.current_action;
        if action != AUTO_SCOUT_ACTION {
            closest_undiscovered_tile()?;
        }
        if let Some([x, y]) = side.seek_unexplored(self, ai_rng)? {
            let _ = self.explore_at(x, y, true, fx);
            self.last_action = self.current_action;
            self.current_action = action;
            return Ok(BREAK);
        }
        match self.current_order {
            crate::ai_kinds::ORDER_SCOUT => Err(GenieError::unsupported(
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
                        return Err(GenieError::unsupported(
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
    ) -> Result<u32, GenieError> {
        let (order, action, p0) = (self.current_order, self.current_action, n.p[0]);
        if order != -1 {
            self.state_position = [at.position[0], at.position[1]];
            if order == crate::ai_kinds::ORDER_SCOUT {
                return Err(GenieError::unsupported(
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
                return Err(GenieError::unsupported(
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
    ) -> Result<u32, GenieError> {
        let (order, action, p0) = (self.current_order, self.current_action, n.p[0]);
        if (order == 700 || n.p[2] == 0x202) && p0 == 600 {
            if self.retarget_disabled != 0 {
                return Err(GenieError::unsupported(
                    "an invalidated attack with retargeting off (+0x1d6)",
                ));
            }
            let produced = self.process_idle(at, computer, ai, side, fx)?;
            if produced {
                return Ok(GO_ON);
            }
            if computer {
                return Err(GenieError::unsupported(
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
                return Err(GenieError::unsupported(
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

    /// The base handler's case `0x1fa` (`FUN_140c589c0`): with an order, or in a group with a
    /// command (`+0x3a8`), the state position takes the unit's place. Without an order, a
    /// completion of the current action (or of an attack) stops it. On the attack order 700 an
    /// attack's completion ("I have an attack order and my attack action has completed.") runs
    /// processIdle (slot 0x240 with 0): when it produced an action the processing breaks (0xd),
    /// else the target goes and the action stops, ending the order (slot 0xb0 with 0). With
    /// another order, the completion of the order's own action (action + 100 = order) stops it,
    /// and so do an attack's; order 724 and 701 end their move, 712 a heal's.
    #[allow(clippy::too_many_arguments)]
    fn action_completed(
        &mut self,
        n: Notify,
        at: &Surroundings,
        computer: bool,
        ai_rng: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        let p1 = n.p[0];
        let order = self.current_order;
        let action = self.current_action;
        if order != -1 || at.group.is_some_and(|g| g.commanded) {
            self.state_position = [at.position[0], at.position[1]];
        }
        if order == -1 {
            if action == p1 || p1 == 600 {
                self.remove_target();
                self.stop_current(true, at, fx)?;
            }
            return Ok(GO_ON);
        }
        if order == 700 && p1 == 600 {
            if self.process_idle(at, computer, ai_rng, side, fx)? {
                return Ok(PRODUCED);
            }
            self.remove_target();
            self.stop_current(false, at, fx)?;
            return Ok(GO_ON);
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
    /// 733) ends however `keep_order` stands, its secondary timer capped; the redraw of an
    /// adjusted idle timeout of exactly 750 (the AI stream's) is not modelled here.
    fn stop_current(
        &mut self,
        keep_order: bool,
        at: &Surroundings,
        fx: &mut Vec<Effect>,
    ) -> Result<(), GenieError> {
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
    ) -> Result<(), GenieError> {
        let action = self.current_action;
        let order = self.current_order;
        if self.patrol_redraws() {
            return Err(GenieError::unsupported("a stop on a patrol order"));
        }
        self.last_action = action;
        if !keep_order || action + 100 == order || matches!(order, PATROL | ATTACK_MOVE) {
            self.cap_patrol_secondary();
            self.last_order = order;
            self.current_order = -1;
            self.order_priority = -1;
            let (position, class) = place.ok_or(GenieError::unsupported(
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
    /// (`FUN_140c4eea0` with 0) ends, its target removed (`FUN_140c5e1e0`), the patrol order 733
    /// resetting its timers ([`UnitAi::leave_patrol_timers`]); its priority and the last order
    /// stay. The patrol order 711 is not among them.
    fn check_order(&mut self, at: &Surroundings, ai: &mut Well512) {
        let ends = matches!(
            self.current_order,
            0x269 | 700 | 0x2c5 | 0x2c6 | 0x2c9 | 0x2cf | 0x2d0 | 0x2dd | 0x322
        );
        if ends && self.can_order(at, false) {
            self.remove_target();
            self.leave_patrol_timers(ai);
            self.current_order = -1;
        }
    }

    /// On a patrol order (711, 733) with an adjusted idle timeout of exactly 750: leaving the
    /// order redraws it (slot 0x1a8), which draws from the AI stream.
    const fn patrol_redraws(&self) -> bool {
        matches!(self.current_order, PATROL | ATTACK_MOVE) && self.adjusted_idle_timeout == 750
    }

    /// The rest of leaving a patrol order (711, 733) once the redraw is not due
    /// ([`UnitAi::patrol_redraws`]): the secondary timer is capped at 750 (`0x2ee < (int)+0x16c`).
    const fn cap_patrol_secondary(&mut self) {
        if matches!(self.current_order, PATROL | ATTACK_MOVE) && 750 < self.secondary_timer as i32 {
            self.secondary_timer = 750;
        }
    }

    /// What leaving a patrol order (711, 733) does to the timers wherever the game sets the
    /// order by hand: an adjusted idle timeout of exactly 750 is redrawn (slot 0x1a8) and the
    /// secondary timer is capped at 750 (`0x2ee < (int)+0x16c`).
    fn leave_patrol_timers(&mut self, ai: &mut Well512) {
        if matches!(self.current_order, PATROL | ATTACK_MOVE) {
            if self.adjusted_idle_timeout == 750 {
                self.update_adjusted_idle_timeout(ai);
            }
            if 750 < self.secondary_timer as i32 {
                self.secondary_timer = 750;
            }
        }
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
    /// queue. Its effects, as the full update's; `side` answers what the orders ask of the unit,
    /// `computer` is its owner's type and `ai` the AI stream.
    ///
    /// # Errors
    ///
    /// What an order's slot does not model.
    pub fn off_turn(
        &mut self,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        computer: bool,
        ai: &mut Well512,
    ) -> Result<Vec<Effect>, GenieError> {
        let mut fx = Vec::new();
        if !self.notifies.is_empty() || self.orders.is_empty() {
            return Ok(fx);
        }
        if (self.current_action != -1 || at.busy) && !at.interruptible {
            return Ok(fx);
        }
        if self.can_order(at, false) {
            self.run_orders(at, side, computer, ai, &mut fx)?;
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
    ) -> Result<bool, GenieError> {
        if 2 < unit_state {
            return Ok(false);
        }
        if order.order == -1 {
            return Ok(true);
        }
        if immediate {
            // the immediate branch is [`UnitAi::order_now`], which reads the unit's id and place
            // and gives effects; [`UnitAi::take_order_now`] is its entry
            return Err(GenieError::unsupported(
                "an immediate order for the unit AI's queue",
            ));
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
    /// 100 waits; each order runs through slot 0x230 ([`UnitAi::process_order`] with the unit's
    /// `at`, `side` and owner's type `computer`, the AI stream `ai`): one that is not ready (0xe)
    /// stays and the next is tried, any other result removes it, and 1 (taken) ends the queue's
    /// turn.
    ///
    /// # Errors
    ///
    /// What an order's slot does not model.
    pub fn run_orders(
        &mut self,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        computer: bool,
        ai: &mut Well512,
        fx: &mut Vec<Effect>,
    ) -> Result<(), GenieError> {
        let mut k = 0;
        while k < self.orders.len() {
            if self.current_action != -1 && self.orders[k].priority < 100 {
                return Ok(());
            }
            let order = self.orders[k];
            let r = self.process_order(&order, Some((at, &mut *side, computer)), None, ai, fx)?;
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

    /// Slot 0x230 (`FUN_140c5c170`, every type's) on the order `o`: one not ready returns 0xe.
    /// Each case runs its slot, forced (its force argument 1); a slot that does not take the
    /// order returns 0 (the queue drops it), one that does goes to the tail
    /// ([`UnitAi::order_tail`]), which makes it the AI's order with its priority, and 1:
    /// - 700, the attack: `AttackObject` by the type ([`UnitAi::attack_slot`], any target, the
    ///   group flag set).
    /// - `0x2bd`, the defend: a unit of master 128 takes none; without a target slot 0x178 at the
    ///   order's point ([`UnitAi::defend_point`]), else slot 0x170 on the target
    ///   ([`UnitAi::defend_object`]), within the order's range.
    /// - `0x2be`, the build: slot 0xf0 on the foundation and the tail, which the caller runs
    ///   ([`Effect::OrderOnTarget`], `Turns::build_order`).
    /// - `0x2bf`, `0x2c0`, `0x2ce`, the heal, conversion and repair: slots 0xe0, 0xd8 and 0xe8
    ///   ([`UnitAi::work_object`]) through the unit's commands ([`UnitCommand`]).
    /// - `0x2c1`, the explore, `0x2da`, the scout, and `0x321`, the auto scout: slot 0x100 to the
    ///   order's point cut to tiles (`CVTTSS2SI`, [`UnitAi::explore_at`]); whatever it gave, the
    ///   scout then holds the action value `0x276` and the auto scout `0x2bd`, the one before the
    ///   last.
    /// - `0x2c2`, the stop: the target goes (`FUN_140c5e1e0`), the action stops (slot 0xb0 with
    ///   1), and the tail ends the order.
    /// - `0x2c3`, run away: slot 0x160 with 1 and no unit ([`UnitAi::run_away`]).
    /// - `0x2c5`, the gather: slot 0xd0 ([`UnitAi::gather_order`]).
    /// - `0x2c6`, the move, and `0x2d4`, the group gather: slot 0x130 to the order's point within
    ///   its range, forced and walking (a move's unit then shows its points, slot 0x2d8, not
    ///   held).
    /// - `0x2c7`, the patrol ("I don't know what to do with patrol orders"), and an order no case
    ///   names: 0.
    /// - `0x2c8`, the follow: slot 0x168 on the target within the order's range
    ///   ([`UnitAi::follow_object`]).
    /// - `0x2c9`, the hunt, and `0x322`, the auto boar luring: slot 0xc8 ([`UnitAi::hunt_order`]);
    ///   whatever it gave, the luring then holds the action value `0x2be`, the one before the
    ///   last.
    /// - `0x2ca`, the transport: slot 0x128 to the order's point ([`UnitAi::transport_to`]).
    /// - `0x2cd`, the enter: slot 0x110 on the target ([`UnitAi::enter_object`]).
    /// - `0x2d1` and 800, the unloads: slots 0x118 and 0x120 at the order's point
    ///   ([`UnitAi::unload_at`]).
    /// - `0x2d2`, seek and destroy: slot 0x180 on the target at the order's point cut to tiles
    ///   ([`UnitAi::seek_and_destroy`]).
    /// - `0x2d3`, explore and destroy: slot 0x188 to the order's point cut to tiles
    ///   ([`UnitAi::explore_and_destroy`]).
    /// - `0x2d8`, the wait: the action value `0x274` and the order `0x2d8` (leaving a patrol
    ///   order as the tail does).
    /// - `0x2d9`, the attack roundup: slot 0xc0 on the target ([`UnitAi::attack_roundup`]); the
    ///   tail makes the order 700.
    /// - `0x2db` and `0x325`, the order on a target ([`UnitAi::order_on_target`]).
    /// - `0x2dd`, the attack move: a unit of class 51 or 54 with a target runs slot 0xb8 on it (the
    ///   tail makes the order 700), any other slot 0x130 to the order's point.
    /// - `0x2de`, the foundation's: slot 0x108 on the target
    ///   ([`UnitAi::wait_for_foundation`]); whatever it gave, the action value `0x27a`, the one
    ///   before the last.
    ///
    /// `world` is the unit's surroundings, its side and its owner's type, which the full update
    /// and the queue between turns pass; [`UnitAi::take_order_now`] passes none but the unit's
    /// `place` (its x and y and its master's class), as its own order (`AUTO_SCOUT`'s stop or
    /// auto scout order) is the one that runs.
    pub(crate) fn process_order(
        &mut self,
        o: &QueuedOrder,
        world: Option<(&Surroundings, &mut dyn UnitSide, bool)>,
        place: Option<([f32; 2], i16)>,
        ai: &mut Well512,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        if !o.ready {
            return Ok(0xe);
        }
        let place = world
            .as_ref()
            .map(|(at, _, _)| ([at.position[0], at.position[1]], at.class))
            .or(place);
        let class = place.map_or(-1, |(_, class)| class);
        let outside =
            GenieError::unsupported("an order on an object run outside the unit AI's update");
        let tile = |v: f32| crate::checkpoint::cvttss2si(v);
        let mv = |o: &QueuedOrder| AiMove {
            point: o.point,
            range: o.range,
            force: true,
            walk: true,
        };
        // `bVar2`: the attack move's attack, whose tail makes the order 700
        let mut attack_move = false;
        let taken = match o.order {
            ATTACK_ORDER => {
                let (at, side, _) = world.ok_or(outside)?;
                self.attack_slot(o.target, true, at, side, fx)?
            }
            DEFEND_ORDER => {
                let (at, side, _) = world.ok_or(outside)?;
                if at.master == DEFENDLESS_MASTER {
                    return Ok(0);
                }
                if o.target == -1 {
                    self.defend_point([o.point[0], o.point[1]], o.range, true, at)
                } else {
                    self.defend_object(o.target, o.range, true, at, side)?
                }
            }
            BUILD_ORDER => {
                // slot 0xf0 (`FUN_140c47d80`) with 1 builds on the target; the tail makes the
                // order the AI's with its priority
                let (at, side, _) = world.ok_or(outside)?;
                self.build_object(o.target, true, at, side, fx)?
            }
            crate::ai_kinds::ORDER_HEAL | crate::ai_kinds::ORDER_CONVERT | REPAIR_ORDER => {
                let (at, side, _) = world.ok_or(outside)?;
                let (action, command) = match o.order {
                    crate::ai_kinds::ORDER_HEAL => (HEAL_ACTION, UnitCommand::Heal(o.target)),
                    crate::ai_kinds::ORDER_CONVERT => {
                        (CONVERT_ACTION, UnitCommand::Convert(o.target))
                    }
                    _ => (REPAIR_ACTION, UnitCommand::Repair(o.target)),
                };
                self.work_object(o.target, true, action, command, at, side)?
            }
            crate::ai_kinds::ORDER_EXPLORE => {
                self.explore_at(tile(o.point[0]), tile(o.point[1]), true, fx)
            }
            STOP_ORDER => {
                self.remove_target();
                self.stop_with(true, place, fx)?;
                true
            }
            RUN_AWAY_ORDER => {
                let (at, side, computer) = world.ok_or(outside)?;
                self.run_away(true, at, computer, ai, side, fx)?
            }
            crate::ai_kinds::ORDER_GATHER => {
                let (at, side, _) = world.ok_or(outside)?;
                self.gather_order(o.target, at, side, fx)?
            }
            MOVE_ORDER | crate::ai_kinds::ORDER_GROUP_GATHER => {
                let m = self.move_to(mv(o));
                fx.extend(m.map(Effect::Move));
                m.is_some()
            }
            FOLLOW_ORDER => {
                let (at, side, _) = world.ok_or(outside)?;
                self.follow_object(o.target, o.range, true, at, side, fx)?
            }
            HUNT_ORDER | crate::ai_kinds::ORDER_AUTO_BOAR_LURING => {
                let (at, side, _) = world.ok_or(outside)?;
                let hunts = self.hunt_order(o.target, at, side, fx)?;
                if o.order == crate::ai_kinds::ORDER_AUTO_BOAR_LURING {
                    self.set_action(BOAR_LURING_ACTION);
                }
                hunts
            }
            crate::ai_kinds::ORDER_TRANSPORT => {
                let (_, side, _) = world.ok_or(outside)?;
                self.transport_to(o.point, true, side)?
            }
            crate::garrison::ENTER_ORDER => {
                let (at, side, _) = world.ok_or(outside)?;
                self.enter_object(o.target, true, at, side)?
            }
            crate::ai_kinds::ORDER_UNLOAD | crate::ai_kinds::ORDER_UNLOAD_OVER_WALL => {
                let (at, side, _) = world.ok_or(outside)?;
                let over_wall = o.order == crate::ai_kinds::ORDER_UNLOAD_OVER_WALL;
                self.unload_at([o.point[0], o.point[1]], true, over_wall, at, side)?
            }
            crate::ai_kinds::ORDER_SEEK_AND_DESTROY => {
                let (at, side, _) = world.ok_or(outside)?;
                let tiles = [tile(o.point[0]), tile(o.point[1])];
                self.seek_and_destroy(o.target, tiles, true, at, side, fx)?
            }
            crate::ai_kinds::ORDER_EXPLORE_AND_DESTROY => {
                self.explore_and_destroy([tile(o.point[0]), tile(o.point[1])], true, fx)
            }
            crate::ai_kinds::ORDER_WAIT => {
                // case 0x2d8: the wait's action value, leaving a patrol order, then the order
                // itself; the tail finds it the AI's already
                self.current_action = WAIT_ACTION;
                self.leave_patrol_timers(ai);
                self.current_order = crate::ai_kinds::ORDER_WAIT;
                true
            }
            crate::ai_kinds::ORDER_ATTACK_ROUNDUP => {
                let (at, side, _) = world.ok_or(outside)?;
                self.attack_roundup(o.target, at, side, fx)?
            }
            crate::ai_kinds::ORDER_SCOUT => {
                let explored = self.explore_at(tile(o.point[0]), tile(o.point[1]), true, fx);
                self.set_action(SCOUT_ACTION);
                explored
            }
            ORDER_ON_TARGET | ORDER_ON_TARGET_2 => {
                let (at, side, _) = world.ok_or(outside)?;
                return self.order_on_target(o, class, at, side, ai, fx);
            }
            ATTACK_MOVE => {
                let (at, side, _) = world.ok_or(outside)?;
                if o.target != -1 && matches!(at.class, TREBUCHET_CLASS | TREBUCHET_PACKED_CLASS) {
                    attack_move = true;
                    self.attack_slot(o.target, true, at, side, fx)?
                } else {
                    let m = self.move_to(mv(o));
                    fx.extend(m.map(Effect::Move));
                    m.is_some()
                }
            }
            FOUNDATION_ORDER => {
                let (_, side, _) = world.ok_or(outside)?;
                let waits = self.wait_for_foundation(o.target, true, side)?;
                self.set_action(WAIT_SOLIDIFY_ACTION);
                waits
            }
            AUTO_SCOUT_ORDER => {
                let explored = self.explore_at(tile(o.point[0]), tile(o.point[1]), true, fx);
                self.set_action(AUTO_SCOUT_ACTION);
                explored
            }
            // the patrol order (0x2c7) and the orders the switch has no case for
            _ => false,
        };
        if !taken {
            return Ok(0);
        }
        Ok(self.order_tail(o.order, o.priority, class, attack_move, ai))
    }

    /// The order's tail (`LAB_140c5d432`) once its case took order value `order` of priority
    /// `priority`: the stop (`0x2c2`) leaves a patrol order ([`UnitAi::leave_patrol_timers`]) and
    /// ends the order and its priority (−1). The attack roundup (`0x2d9`) and the attack move's
    /// attack (`attack_move`) leave a patrol order and become the attack order (700); the attack
    /// order of a unit of class 18 (`class`, its master's) leaves a patrol order, its target goes
    /// (`+0x48` −1, its class and point kept) and the order is −1. Any other becomes the order,
    /// leaving a patrol order for another (`LAB_140c5d611`). The priority is the order's but
    /// for the stop. Returns 1.
    pub(crate) fn order_tail(
        &mut self,
        order: i32,
        priority: i32,
        class: i16,
        attack_move: bool,
        ai: &mut Well512,
    ) -> u32 {
        if order == STOP_ORDER {
            self.leave_patrol_timers(ai);
            self.current_order = -1;
            self.order_priority = -1;
            return 1;
        }
        let new = if order == crate::ai_kinds::ORDER_ATTACK_ROUNDUP || attack_move {
            self.leave_patrol_timers(ai);
            ATTACK_ORDER
        } else if order == ATTACK_ORDER && class == CLASS_18 {
            self.leave_patrol_timers(ai);
            self.current_target = -1;
            -1
        } else {
            if self.current_order != order {
                self.leave_patrol_timers(ai);
            }
            order
        };
        self.current_order = new;
        self.order_priority = priority;
        1
    }

    /// The object `id` as the world holds it (`FUN_1415417a0`): the unit itself (`at`, its own
    /// target of no one), among the objects the update names ([`Surroundings::unit`]), else as
    /// the unit's side finds it ([`UnitSide::seen_unit`]); None when the world does not hold it.
    fn world_object(
        id: i32,
        at: &Surroundings,
        side: &mut dyn UnitSide,
    ) -> Result<Option<UnitSeen>, GenieError> {
        if id == at.id {
            return Ok(Some(UnitSeen {
                id,
                class: at.class,
                owner: at.player,
                combat_level: at.combat_level,
                state: at.state,
                target: None,
                gap_squared: crate::movement::footprint_gap_squared(
                    at.position,
                    at.size,
                    at.position,
                    at.size,
                ),
                position: at.position,
                hp: at.hp,
                master: at.master,
                kind: at.kind,
                main: None,
                allied: true,
            }));
        }
        at.unit(id)
            .map_or_else(|| side.seen_unit(id), |u| Ok(Some(*u)))
    }

    /// `AttackObject` (slot 0xb8) forced, by the unit AI's type, on `target`, `any` target or
    /// not: the civilian's (`FUN_140c17370`, [`UnitAi::civilian_attack`]: an animal or a tree it
    /// hunts, any other it attacks, not `any`); the soldier's (`FUN_140c17550`) the base's, then
    /// a member of a group the world holds stops moving ([`UnitSide::group_attack_stop`]); the
    /// others' the base's ([`UnitAi::attack_object`]). The priest's (`FUN_140c17490`) asks its
    /// slot 0x1b8 (`FUN_140c1af00`) whether to convert the target first: not modelled, the base's
    /// stops at the priest's slot 0xa0, which is not either.
    fn attack_slot(
        &mut self,
        target: i32,
        any: bool,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        match self.ai_type {
            CIVILIAN => self.civilian_attack(target, at, side, fx),
            SOLDIER_AI => {
                if !self.attack_object(target, true, any, at, side, fx)? {
                    return Ok(false);
                }
                if at.group.is_some() {
                    side.group_attack_stop()?;
                }
                Ok(true)
            }
            _ => self.attack_object(target, true, any, at, side, fx),
        }
    }

    /// `attackRoundupObject` (slot 0xc0, `FUN_140c43aa0`) on `target`: the AI's own target
    /// ("I'm already attacking that target") or one the world does not hold takes nothing; then
    /// slot 0xa0 not `any` ([`UnitAi::attack_check`]) must let it attack the target; with an
    /// action value the unit must reach the target within its range (slot 0x3d0,
    /// [`UnitSide::reaches`]: "can't path to new target"). Then the unit's attack command, forced
    /// ([`UnitAi::attack_command`]), and on success the action is the attack (600, the last action
    /// keeping the old) and the target its class and place (`FUN_140c64c10`). Unlike
    /// `AttackObject`, `+0x1d5` stays.
    fn attack_roundup(
        &mut self,
        target: i32,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        if self.current_target == target {
            return Ok(false);
        }
        let Some((class, position)) = self.attack_check(target, false, at, side)? else {
            return Ok(false);
        };
        if self.current_action != -1 && !side.reaches(target, at.weapon_range)? {
            return Ok(false);
        }
        if !self.attack_command(target, true, at, side, fx)? {
            return Ok(false);
        }
        self.set_action(ATTACK_ACTION);
        self.target_at(target, Some(i32::from(class)), position);
        Ok(true)
    }

    /// Slot 0x170 (`FUN_140c4ccc0`), the defend of object `target` within `range`, `force`d or
    /// without an action value: on an object the world holds, the action value is the defend's
    /// (`0x259`, the last keeping the old), the target the object at its place
    /// (`FUN_140c64c10`), the desired distance `range` and the defend target the object.
    fn defend_object(
        &mut self,
        target: i32,
        range: f32,
        force: bool,
        at: &Surroundings,
        side: &mut dyn UnitSide,
    ) -> Result<bool, GenieError> {
        if !force && self.current_action != -1 {
            return Ok(false);
        }
        let Some(t) = Self::world_object(target, at, side)? else {
            return Ok(false);
        };
        self.set_action(DEFEND_ACTION);
        self.target_at(target, Some(i32::from(t.class)), t.position);
        self.desired_distance = range;
        self.defend_target = target;
        Ok(true)
    }

    /// Slot 0x178 (`FUN_140c4ce30`), the defend of `point` within `range`, `force`d or without an
    /// action value: the defend's action value (`0x259`, the last keeping the old), no target at
    /// the point at the unit's height (`FUN_140c64c10` with −1), the desired distance `range`
    /// and no defend target.
    const fn defend_point(
        &mut self,
        point: [f32; 2],
        range: f32,
        force: bool,
        at: &Surroundings,
    ) -> bool {
        if !force && self.current_action != -1 {
            return false;
        }
        self.set_action(DEFEND_ACTION);
        self.target_at(-1, None, [point[0], point[1], at.position[2]]);
        self.desired_distance = range;
        self.defend_target = -1;
        true
    }

    /// `BuildObject` (slot 0xf0, `FUN_140c47d80`) on foundation `target`, `force`d or off the
    /// build action value (`0x25a`): the unit's build command (slot 0x380), which the caller runs
    /// after the update in its order ([`Effect::BuildObject`]; one that does not take is not
    /// modelled, `Turns::build_command`), then the build action value (the last keeping the old)
    /// and the foundation as the target at its place with its class as the world holds it, else
    /// class −1 at (0, 0, 0) (the target and class before becoming the last). True when it builds.
    ///
    /// # Errors
    ///
    /// A foundation the slice cannot read.
    pub(crate) fn build_object(
        &mut self,
        target: i32,
        force: bool,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        if !force && self.current_action == BUILD_ACTION {
            return Ok(false);
        }
        fx.push(Effect::BuildObject(target));
        self.set_action(BUILD_ACTION);
        match Self::world_object(target, at, side)? {
            Some(t) => self.set_target(target, i32::from(t.class), t.position),
            None => self.set_target(target, -1, [0.0; 3]),
        }
        Ok(true)
    }

    /// The work slots on object `target` (heal 0xe0, `FUN_140c4e010`; conversion 0xd8,
    /// `FUN_140c4c9f0`; repair 0xe8, `FUN_140c5e3a0`), `force`d or off their own action value
    /// `action` (`0x25b`, `0x25c`, `0x26a`): the unit's `command` ([`UnitSide::command`]: slots
    /// 0x108, 0x370, 0x378), on its 1 the action value `action` (the last keeping the old) and the
    /// target with its class and place as the world then holds it, else class −1 at (0, 0, 0)
    /// (the target and class before becoming the last).
    fn work_object(
        &mut self,
        target: i32,
        force: bool,
        action: i32,
        command: UnitCommand,
        at: &Surroundings,
        side: &mut dyn UnitSide,
    ) -> Result<bool, GenieError> {
        if !force && self.current_action == action {
            return Ok(false);
        }
        if !side.command(command)? {
            return Ok(false);
        }
        self.set_action(action);
        match Self::world_object(target, at, side)? {
            Some(t) => self.set_target(target, i32::from(t.class), t.position),
            None => self.set_target(target, -1, [0.0; 3]),
        }
        Ok(true)
    }

    /// `GatherObject` (slot 0xd0, `FUN_140c4dd50`) forced on the order's `target`: the unit's
    /// gather command (slot 0x368, `FUN_141085030`) takes no object the world does not hold (0);
    /// on one it holds it runs after the update ([`Effect::GatherObject`], which fails without a
    /// task: the caller stops then) and the AI holds the gather ([`UnitAi::gather_object`]).
    fn gather_order(
        &mut self,
        target: i32,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        let Some(t) = Self::world_object(target, at, side)? else {
            return Ok(false);
        };
        self.gather_object(
            Gatherable {
                id: target,
                position: t.position,
                class: t.class,
            },
            fx,
        );
        Ok(true)
    }

    /// `HuntObject` (slot 0xc8, `FUN_140c4e220`) forced on the order's `target`: the action stops
    /// first (slot 0xb0 with 1); the unit's hunt command (slot 0x360, `FUN_1412151f0`) takes no
    /// object the world does not hold (0, the stop made); on one it holds,
    /// [`UnitAi::hunt_object`].
    fn hunt_order(
        &mut self,
        target: i32,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        let Some(t) = Self::world_object(target, at, side)? else {
            self.stop_current(true, at, fx)?;
            return Ok(false);
        };
        self.hunt_object(target, i32::from(t.class), t.position, at, fx)?;
        Ok(true)
    }

    /// Slot 0x168 (`FUN_140c4dab0`, "GOT A FOLLOW ORDER"), the follow of object `target` within
    /// `range`, `force`d or without an action value: on an object the world holds, the follow's
    /// action value (`0x264`, the last keeping the old), the target at its place
    /// (`FUN_140c64c10`) and the desired distance `range`; when the footprints' squared gap
    /// (`FUN_14118f8c0`) exceeds `range` squared (`COMISS`/`JBE`: not a number stays), the AI
    /// moves to it (slot 0x138, [`UnitAi::move_to_object`]).
    fn follow_object(
        &mut self,
        target: i32,
        range: f32,
        force: bool,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        if !force && self.current_action != -1 {
            return Ok(false);
        }
        let Some(t) = Self::world_object(target, at, side)? else {
            return Ok(false);
        };
        self.set_action(FOLLOW_ACTION);
        self.target_at(target, Some(i32::from(t.class)), t.position);
        self.desired_distance = range;
        if range * range < t.gap_squared {
            let seen = Seen {
                state: t.state,
                class: t.class,
                position: t.position,
                ..Seen::default()
            };
            self.move_to_object(target, (seen.state, seen.class, seen.position), range, fx);
        }
        Ok(true)
    }

    /// Slot 0x110, the enter into object `target`, `force`d or without an action value (the
    /// base's `FUN_140c4d050`; the civilian's, `FUN_140c1c680`, first takes nothing the world
    /// does not hold): the unit's enter command (slot 0x3a0, [`UnitCommand::Enter`]), on its 1 the
    /// enter's action value (`0x269`, the last keeping the old) and the target at (−1, −1, −1)
    /// (`FUN_140c64c10`: its class when the world holds it).
    fn enter_object(
        &mut self,
        target: i32,
        force: bool,
        at: &Surroundings,
        side: &mut dyn UnitSide,
    ) -> Result<bool, GenieError> {
        let held = Self::world_object(target, at, side)?;
        if self.ai_type == CIVILIAN && held.is_none() {
            return Ok(false);
        }
        if !force && self.current_action != -1 {
            return Ok(false);
        }
        if !side.command(UnitCommand::Enter(target))? {
            return Ok(false);
        }
        self.set_action(crate::garrison::ENTER_ACTION);
        self.target_at(target, held.map(|t| i32::from(t.class)), [-1.0; 3]);
        Ok(true)
    }

    /// Slot 0x128 (`FUN_140c65400`), the transport to `point`, `force`d or without an action
    /// value: the unit's transport command (slot 0x3b8, [`UnitCommand::Transport`]), on its 1
    /// the transport's action value (`0x266`, the last keeping the old) and no target at the
    /// point (`FUN_140c64c10` with −1).
    fn transport_to(
        &mut self,
        point: [f32; 3],
        force: bool,
        side: &mut dyn UnitSide,
    ) -> Result<bool, GenieError> {
        if !force && self.current_action != -1 {
            return Ok(false);
        }
        if !side.command(UnitCommand::Transport(point))? {
            return Ok(false);
        }
        self.set_action(TRANSPORT_ACTION);
        self.target_at(-1, None, point);
        Ok(true)
    }

    /// Slots 0x118 (`FUN_140c660d0`) and, `over_wall`, 0x120 (`FUN_140c66330`): the unload at
    /// `point`, `force`d or without an action value: the unit's unload command at the point at
    /// its own height (slots 0x3a8 and 0x3b0, [`UnitCommand::Unload`]), on its 1 the unload's
    /// action value (`0x26d`; over a wall 700) with the last keeping the old, and the unit
    /// itself as the target, with its class and place.
    fn unload_at(
        &mut self,
        point: [f32; 2],
        force: bool,
        over_wall: bool,
        at: &Surroundings,
        side: &mut dyn UnitSide,
    ) -> Result<bool, GenieError> {
        if !force && self.current_action != -1 {
            return Ok(false);
        }
        let to = [point[0], point[1], at.position[2]];
        if !side.command(UnitCommand::Unload { to, over_wall })? {
            return Ok(false);
        }
        self.set_action(if over_wall {
            UNLOAD_OVER_WALL_ACTION
        } else {
            UNLOAD_ACTION
        });
        self.set_target(at.id, i32::from(at.class), at.position);
        Ok(true)
    }

    /// Slot 0x180 (`FUN_140c64ab0`), seek and destroy object `target` from the tile `tile`,
    /// `force`d or without an action value: the unit's attack command on the target (slot 0x338,
    /// [`UnitAi::attack_command`]), on its 1 the seek's action value (`0x26e`, the last keeping
    /// the old) and the target at the tile (as floats) at height 1 (`FUN_140c64c10`).
    fn seek_and_destroy(
        &mut self,
        target: i32,
        tile: [i32; 2],
        force: bool,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        if !force && self.current_action != -1 {
            return Ok(false);
        }
        if !self.attack_command(target, force, at, side, fx)? {
            return Ok(false);
        }
        self.set_action(SEEK_ACTION);
        let class = Self::world_object(target, at, side)?.map(|t| i32::from(t.class));
        self.target_at(target, class, [tile[0] as f32, tile[1] as f32, EXPLORE_Z]);
        Ok(true)
    }

    /// Slot 0x188 (`FUN_140c4d9b0`), explore and destroy to the tile `tile`, `force`d or without
    /// an action value: the unit's explore there (slot 0x398, [`Effect::Explore`], which takes
    /// it), then the action value `0x26f` (the last keeping the old) and no target at the tile
    /// (as floats) at height 1.
    fn explore_and_destroy(&mut self, tile: [i32; 2], force: bool, fx: &mut Vec<Effect>) -> bool {
        if !force && self.current_action != -1 {
            return false;
        }
        let point = [tile[0] as f32, tile[1] as f32];
        fx.push(Effect::Explore(point));
        self.set_action(EXPLORE_DESTROY_ACTION);
        self.target_at(-1, None, [point[0], point[1], EXPLORE_Z]);
        true
    }

    /// Slot 0x230's case for the order on a target (`0x2db`, `0x325`): on a target the world holds,
    /// the unit's slot 0x128 on it at its place, then the tail with the order value the AI holds
    /// after it ("the order the work gave"), both the caller's ([`Effect::OrderOnTarget`],
    /// `Turns::order_on_target`). On a gone target: without its type (`+0x30` −1), the move to
    /// the order's point within its range (slot 0x130, forced and walking) and, taken, the tail
    /// with the order on a target. With its type (the class of that master of the target's
    /// owner `+0x10`, `FUN_14152fe40`, [`UnitSide::master_class`]; without an owner the type is
    /// the class), the unit AI's slot 0x1b0 gives a kind of resource
    /// ([`UnitAi::resource_kind`]) and slot 0x88 the closest object of it to the order's shown
    /// point (`+0x24`, `+0x28`), which the unit works (slot 0x7e8, [`Effect::WorkOn`]); either
    /// way the order is not taken (0).
    fn order_on_target(
        &mut self,
        o: &QueuedOrder,
        class: i16,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        ai: &mut Well512,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        if Self::world_object(o.target, at, side)?.is_some() {
            fx.push(Effect::OrderOnTarget(*o));
            return Ok(1);
        }
        if o.target_type != -1 {
            // the class of the target's type: its owner's master's, else the type itself
            let class = if o.target_owner == -1 {
                o.target_type
            } else {
                match side.master_class(o.target_owner, o.target_type)? {
                    Some(class) => i32::from(class),
                    None => return Ok(0),
                }
            };
            let kind = self.resource_kind(class);
            if kind == -1 {
                return Ok(0);
            }
            // slot 0x88 (`FUN_140c4b0f0`): slot 0x80 from the order's shown point, without a
            // drop site, wood (0) for fewer than 2 workers, at any state
            let found = side.closest_resource(ResourceSearch {
                kind: kind as u8,
                cap: if kind == 0 { 2 } else { -1 },
                site: -1,
                current: self.current_target,
                point: o.shown,
                min_state: 0,
            })?;
            // the world holds what the search found: the move after a found object it does not
            // hold cannot run
            if let Some(g) = found {
                fx.push(Effect::WorkOn {
                    target: g.id,
                    point: g.position,
                });
            }
            return Ok(0);
        }
        let Some(m) = self.move_to(AiMove {
            point: o.point,
            range: o.range,
            force: true,
            walk: true,
        }) else {
            return Ok(0);
        };
        fx.push(Effect::Move(m));
        Ok(self.order_tail(o.order, o.priority, class, false, ai))
    }

    /// Slot 0x1b0 by the unit AI's type: the kind of resource of a target class. The civilian's
    /// (`FUN_140c1c4f0`) and the building's (`FUN_140c1c460`, the same switch,
    /// [`civilian_kind`]); the fishing ship's (`FUN_140c1c580`): 2 for classes 5, 31 and 33, 4 for
    /// 63; the base's (`FUN_140c4cc00`) none. −1 for none (the byte 0xff).
    const fn resource_kind(&self, class: i32) -> i8 {
        match self.ai_type {
            CIVILIAN | 17 => civilian_kind(class),
            FISHING_AI => match class {
                5 | 0x1f | 0x21 => 2,
                0x3f => 4,
                _ => -1,
            },
            _ => -1,
        }
    }

    /// Slot 0x108 (`FUN_140c68a60`), with `force` (its argument 3) or without an action value:
    /// the unit's slot 0x388 on the foundation `target` ([`UnitSide::wait_solidify`]); on its 1
    /// the action value becomes [`WAIT_SOLIDIFY_ACTION`], the old one the last. True when it
    /// waits.
    fn wait_for_foundation(
        &mut self,
        target: i32,
        force: bool,
        side: &mut dyn UnitSide,
    ) -> Result<bool, GenieError> {
        if !force && self.current_action != -1 {
            return Ok(false);
        }
        if !side.wait_solidify(target)? {
            return Ok(false);
        }
        self.set_action(WAIT_SOLIDIFY_ACTION);
        Ok(true)
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
    /// `place` is the unit's position (x, y) and master's class, for the stop; `ai` the AI
    /// stream, for the order's tail.
    ///
    /// # Errors
    ///
    /// An order whose slot reads the unit's surroundings, which this entry does not have.
    pub fn take_order_now(
        &mut self,
        unit_state: u8,
        order: QueuedOrder,
        place: ([f32; 2], i16),
        ai: &mut Well512,
        fx: &mut Vec<Effect>,
    ) -> Result<(), GenieError> {
        if 2 < unit_state || order.order == -1 {
            return Ok(());
        }
        self.order_now(order, None, Some(place), None, ai, fx)
    }

    /// Slot 0x28 (`FUN_140c55c90`) with Immediate set and inFront clear, from outside the unit
    /// AI's update with what it reads of its unit and the world (`world`: the surroundings, the
    /// side, the owner's computer flag): for a unit in state 2 or below and an order (not −1),
    /// [`UnitAi::order_now`] without keeping the current order, the order run at once with
    /// `world` ([`UnitAi::process_order`]). `ai` is the AI stream.
    ///
    /// # Errors
    ///
    /// What the order's run does not model.
    pub fn take_order_now_with(
        &mut self,
        order: QueuedOrder,
        (at, side, computer): (&Surroundings, &mut dyn UnitSide, bool),
        ai: &mut Well512,
        fx: &mut Vec<Effect>,
    ) -> Result<(), GenieError> {
        if 2 < at.state || order.order == -1 {
            return Ok(());
        }
        let place = ([at.position[0], at.position[1]], at.class);
        self.order_now(order, None, Some(place), Some((at, side, computer)), ai, fx)
    }

    /// The idle part of the full update (`0x140c66b10` after slot 73): a unit without an action
    /// value (and without a target or not busy) runs slot 72 once its idle timer passes its
    /// timeout, unless it is a busy unit of class 20 or its group ([`group_lets_idle`]) holds it;
    /// else the patrol branch ([`UnitAi::patrol_idle_part`]), class 3's and 52's on actions
    /// `0x26b`/`0x26c` ([`UnitAi::making_idle_part`]) or the end ([`UnitAi::idle_part_end`]).
    /// `busy` and `group` are what the update read before its order queue.
    #[allow(clippy::too_many_arguments)]
    fn idle_part(
        &mut self,
        busy: bool,
        group: Option<GroupState>,
        at: &Surroundings,
        computer: bool,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<(), GenieError> {
        let class_20 = busy && at.class == CLASS_20;
        if !class_20
            && self.adjusted_idle_timeout < self.idle_timer
            && self.current_action == -1
            && (self.current_target == -1 || !busy)
            && group_lets_idle(group)
        {
            // slot 0x240 with 1; the idle part reads no result
            self.idle_timer = 0;
            let _ = self.process_idle(at, computer, ai, side, fx)?;
            return Ok(());
        }
        if self.patrols(group) {
            return self.patrol_idle_part(at, computer, ai, side, fx);
        }
        if matches!(at.class, CLASS_3 | CLASS_52) && matches!(self.current_action, 0x26b | 0x26c) {
            return self.making_idle_part(at, computer, ai, side, fx);
        }
        self.idle_part_end(at.class, at.player, group)
    }

    /// The idle part's patrol branch (`0x140c67f0b`): "I have a patrol order, checking to see if
    /// my secondary timer (%d) has expired." While it runs nothing more happens. Expired, it
    /// restarts at 750 (`PATROL_ATTACK_DELAY`; an adjusted idle timeout of exactly 750 is redrawn,
    /// slot 0x1a8) and processIdle runs (slot 0x240 with 1, [`UnitAi::process_idle`]) unless the
    /// unit's top action holds a building as its target (slot 400). The branch on the byte
    /// `DAT_143f01835` cannot run: nothing writes it.
    fn patrol_idle_part(
        &mut self,
        at: &Surroundings,
        computer: bool,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<(), GenieError> {
        if self.secondary_timer != 0 {
            return Ok(());
        }
        self.secondary_timer = 750;
        if self.adjusted_idle_timeout == 750 {
            self.update_adjusted_idle_timeout(ai);
        }
        if at.top_target.is_some_and(|t| t.kind == BUILDING_KIND) {
            return Ok(());
        }
        let _ = self.process_idle(at, computer, ai, side, fx)?;
        Ok(())
    }

    /// The idle part's patrol branch (`0x140c67f0b`): a patrol order (`0x2c7`, `0x2dd`), or a
    /// group that patrols ([`GroupState::patrol`]) off order `0x326`.
    fn patrols(&self, group: Option<GroupState>) -> bool {
        matches!(self.current_order, PATROL | ATTACK_MOVE)
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
    ) -> Result<(), GenieError> {
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
    ) -> Result<(), GenieError> {
        if self.patrols(group) {
            return Err(GenieError::unsupported("a patrol order's idle part"));
        }
        if matches!(class, CLASS_3 | CLASS_52) && matches!(self.current_action, 0x26b | 0x26c) {
            return Err(GenieError::unsupported(
                "a unit of class 3 or 52 on action 0x26b/0x26c",
            ));
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
    /// value plus 100 without one): the follow order's ([`UnitAi::follow`]) and the defend
    /// order's ([`UnitAi::defend_misc`]). Then the leash checks (not on the orders 0x2c7, 0x2dd and 0x323, nor in a
    /// group that patrols, [`GroupState::patrol`]): stance 1's (Defend) stops a unit more than 5
    /// tiles (`DEFEND_LEASH`, squared) from its state position, forgets its target and walks
    /// back ([`UnitAi::walk_back`]), then goes on. Stance 2's (stand ground) stops a unit with an
    /// action value (`+0x44`, not `0x2bf` or `0x2c0`) whose squared gap to its state position
    /// exceeds its range plus [`STAND_GROUND_SLACK`] (`COMISS`/`JBE`: a gap that is not a number
    /// stays), forgets its target and walks back, and slot 73 ends there (10); a unit without an
    /// order but with an action of its own can stray. With patrol information (`+0x1c0`,
    /// `FUN_140f95e10`) either leash is not modelled. Then the retarget check:
    /// with retargeting on (`+0x1d6` 0), a timer used up is redrawn (`750 + r % 500`); a human
    /// player's unit of a range above 3 on the attack order 700 stops there ("I have a range >
    /// 3.0 with an attack order, so I don't retarget."), as does a unit with a target of Gaia's
    /// ("I have a GAIA target, so I can't retarget.") or without a target (or one gone), and
    /// one with a player's target off the attack order but for a building or wall target
    /// (classes 3, 27,
    /// 39, 49, 52) or a trebuchet's (51, 54), whose search for a better target (slot 0x538 with
    /// 9, then slot 0x520) is not modelled; nor is the attack order's search. True when the
    /// timer ran out.
    fn retarget_check(
        &mut self,
        world_time: u32,
        computer: bool,
        at: &Surroundings,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        let order = match (self.current_order, self.current_action) {
            (-1, -1) => -1,
            (-1, action) => action + 100,
            (order, _) => order,
        };
        if order == DEFEND_ORDER && self.defend_misc(at, computer, ai, side, fx)? {
            return Ok(false);
        }
        if order == FOLLOW_ORDER && self.follow(at, fx)? {
            return Ok(false);
        }
        // the leashes (unless on orders 0x2c7, 0x2dd and 0x323, or in a group that patrols:
        // FUN_141017c30, FUN_141017c40), measured from the unit's footprint (FUN_140a847c0)
        let leashed = matches!(self.stance, 1 | 2)
            && !matches!(self.current_order, 0x2c7 | 0x2dd | 0x323)
            && !at.group.is_some_and(|g| g.patrol);
        // with patrol information (+0x1c0) the leash is the distance from where the unit last
        // moved to (slot 0x520) to the patrol's path (FUN_140f95e10): beyond it the unit stops
        // (slot 0xb0 with 1), forgets its target and walks the patrol on
        // (restorePatrolInformation): "That's too far out, returning to patrol." (10); within
        // it the defend stance ends slot 73 (9), stand ground goes on to the retarget check
        if let Some(saved) = self.patrol_path.as_ref().filter(|_| leashed) {
            let leash = if self.stance == 1 {
                DEFEND_PATROL_LEASH
            } else {
                STAND_GROUND_PATROL_LEASH
            };
            if leash < patrol_distance(&saved.path.points, at.moved_at)? {
                self.stop_current(true, at, fx)?;
                self.remove_target();
                let _ = self.restore_patrol(at, ai, fx);
                return Ok(false);
            }
            if self.stance == 1 {
                return Ok(false);
            }
        }
        if leashed && self.stance == 1 && self.patrol_path.is_none() {
            // the defend stance's: a unit more than 5 tiles (DAT_142ed6f20, squared) from its
            // state position stops (slot 0xb0 with 1), forgets its target and walks back ("We're
            // too far out, returning."), then on to the retarget check
            let gap = point_gap_squared(self.state_position, at.position, at.size);
            if DEFEND_LEASH < gap {
                self.stop_current(true, at, fx)?;
                self.remove_target();
                let _ = self.walk_back(at, fx);
            }
        }
        if leashed
            && self.stance == 2
            && self.patrol_path.is_none()
            && !matches!(self.current_action, 0x2bf | 0x2c0)
        {
            // stand ground's: the squared gap against the range plus 0.25 (ADDSS, then COMISS
            // and JBE); a unit with an action value stops, forgets its target and walks back,
            // and slot 73 returns 10 ("We're too far out, returning.", "SO Call 40.")
            let gap = point_gap_squared(self.state_position, at.position, at.size);
            if at.weapon_range + STAND_GROUND_SLACK < gap && self.current_action != -1 {
                self.stop_current(true, at, fx)?;
                self.remove_target();
                let _ = self.walk_back(at, fx);
                return Ok(false);
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
        let target = at.target.map(|t| (t.owner, t.class));
        let listed = at.action_state.map(|_| at.attack_listed);
        if let Some(search) =
            self.retarget_outcome(target, at.weapon_range, computer, at.action_state, listed)?
        {
            let _ = self.retarget_search(search, computer, at, side, fx)?;
        }
        Ok(true)
    }

    /// The retarget's search for a better target (`0x140c567c0` past its checks):
    /// `bestUnitToAttack` from the current target's place (slot 0x520) within 8
    /// (`DAT_142eb9430`), else from the unit's own without a reference distance; an idle check
    /// with path checks, any class without a target or with a building's, in the attack work
    /// state the checks read (none without a target). Its retarget entries join the AI's, its
    /// search sets the alert mode. Then, unless a computer player's unit in stance 0 keeps a
    /// building target: on the attack order a found object whose attack category differs from
    /// a live current target's (state 2) ends it (9). Without a building target (the attack
    /// order, so) nothing more happens (9). With one: a found building but a trebuchet's
    /// classes (51, 54), unless the current target is a farm (49) and it is none, keeps the
    /// current target (10); a found annex names its main building. A found object other than the
    /// current target is attacked (slot 0xb8, not any: [`UnitAi::attack_slot`], 10).
    fn retarget_search(
        &mut self,
        search: RetargetSearch,
        computer: bool,
        at: &Surroundings,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, GenieError> {
        let current = if self.current_target == -1 {
            None
        } else {
            Self::world_object(self.current_target, at, side)?
        };
        let (from, distance, any_class, work_state) =
            current.map_or((at.position, 0.0, true, false), |t| {
                (
                    t.position,
                    RETARGET_REACH,
                    search.building,
                    search.work_state,
                )
            });
        let (classes, radius) = self.search_area(at, computer)?;
        let found = side.closest(&Search {
            at: [from[0], from[1]],
            radius,
            classes,
            size: at.size,
            pick: Pick::Attack {
                lowest: if at.scenario && !computer { 2 } else { 0 },
                seeker: self.seeker(at),
                ask: AttackAsk {
                    distance,
                    idle: true,
                    paths: true,
                    extra: None,
                    any_class,
                    work_state,
                },
            },
            memory: self.memory(),
        })?;
        self.add_retries(&found);
        self.attack_search_alert(found, computer)?;
        let keeps = search.building && computer && self.stance == 0;
        let new = if found.id == -1 {
            None
        } else {
            Self::world_object(found.id, at, side)?
        };
        if !keeps
            && self.current_order == ATTACK_ORDER
            && let (Some(n), Some(c)) = (new, current)
            && c.state == 2
            && crate::best_target::category(n.master, n.class)
                != crate::best_target::category(c.master, c.class)
        {
            return Ok(9);
        }
        if !search.building {
            return Ok(9);
        }
        let mut id = found.id;
        if let Some(n) = new
            && id != -1
        {
            if n.kind == 80 {
                let farm_rule = current.is_none_or(|c| c.class != 49) || n.class == 49;
                if !matches!(n.class, 51 | 54) && farm_rule {
                    return Ok(10);
                }
            }
            if 80 <= n.kind
                && let Some(main) = n.main
            {
                id = main;
            }
        }
        if id == -1 || id == self.current_target {
            return Ok(9);
        }
        let _ = self.attack_slot(id, false, at, side, fx)?;
        Ok(10)
    }

    /// The retarget check after its timer's redraw: a human player's unit of a range above 3 on
    /// the attack order stops there, as does a unit with a Gaia target or without one (or one
    /// gone; `target` its owner and class as the world holds it), and one with a player's target
    /// off the attack order but for a building, wall or trebuchet target. Then a unit without an
    /// Attack in its action list (slot 0x538 with 9: `attack_listed`, None when the slice does
    /// not hold the list) stops, as does a unit in stand ground or no attack (stance above 1)
    /// with an order, and one whose top action is in an attack's work state (6 or 7, slot 0x2b0:
    /// `action_state`, None when not held) without a building target. What the search for a
    /// better target reads ([`UnitAi::retarget_search`]), or None.
    fn retarget_outcome(
        &self,
        target: Option<(usize, i16)>,
        weapon_range: f32,
        computer: bool,
        action_state: Option<u8>,
        attack_listed: Option<bool>,
    ) -> Result<Option<RetargetSearch>, GenieError> {
        let attack_order = self.current_order == ATTACK_ORDER;
        if attack_order && RETARGET_RANGE < weapon_range && !computer {
            return Ok(None);
        }
        let mut search = false;
        if self.current_target != -1 {
            match target {
                Some((0, _)) => return Ok(None),
                Some((_, 3 | 27 | 39 | 49 | 52 | 51 | 54)) => {
                    if attack_order && !computer {
                        return Ok(None);
                    }
                    search = true;
                }
                // a target gone from the world counts as none (FUN_1415417a0 finds nothing)
                Some(_) | None => {}
            }
        }
        if !attack_order && !search {
            return Ok(None);
        }
        // slot 0x538 with 9: without an Attack in the action list, no search (an action list
        // the slice does not hold goes on to the state's test below, which stops)
        if attack_listed == Some(false) {
            return Ok(None);
        }
        // a unit in stand ground or no attack with an order goes no further
        if 1 < self.stance && self.current_order != -1 {
            return Ok(None);
        }
        // "I have an game attack action without an order ... inAttackWorkState"
        if matches!(held_action_state(action_state)?, 6 | 7) && !search {
            return Ok(None);
        }
        let work_state = matches!(held_action_state(action_state)?, 6 | 7);
        Ok(Some(RetargetSearch {
            building: search,
            work_state,
        }))
    }

    /// The civilian's slot 73 (`FUN_140c21da0`, processMisc): on the attack order (700) without
    /// a target (`+0x48` −1) the action stops and the order ends (slot 0xb0 with 0). A computer
    /// player's villager whose player's `+0x622` is 1 also stops on the attack order or action
    /// when its target, a player's object, is beyond its range plus 1 (`FUN_141193e30`), which
    /// is not modelled: `+0x622` is not held. It returns 9 (no retarget).
    fn civilian_misc(&mut self, at: &Surroundings, fx: &mut Vec<Effect>) -> Result<(), GenieError> {
        if self.current_order == ATTACK_ORDER && self.current_target == -1 {
            self.stop_current(false, at, fx)?;
        }
        Ok(())
    }

    /// processMisc's defend order (`0x140c567c0`, "I have a defend order for Unit #%d"). A defend
    /// target (`+0x60`) the world does not hold, or not the owner's own or an ally's
    /// (`FUN_141663d90` with the ally flag), ends it ("Unit #%d is invalid, stopping."); one held,
    /// it first gives a target point at (−1, x) its place (`UCOMISS`: not a NaN) and ends the
    /// order once dead (above state 2) but for Gaia's ("Unit #%d is dead, stopping."). The end:
    /// the defend target and the target go (`FUN_140c5e1e0`), the current action stops (slot 0xb0
    /// with 1), a patrol order leaves its timers ([`UnitAi::leave_patrol_timers`]) and the order
    /// is −1. Else, but on the attack action (600), the evacuation (`0x2bf`) or a move toward the
    /// defend target (`0x262` with the unit's top action's target, slot 0x280, the defend
    /// target): with a defend target the target point takes its place (x, y, z) and the distance
    /// is the footprints' squared gap (`FUN_14118f8c0`), without one the squared gap from the
    /// unit's footprint to the point (`FUN_140a847c0`); beyond the desired distance plus 1,
    /// squared (`ADDSS`, `MULSS`, `COMISS` with `JBE`: a NaN stays), the unit moves to the
    /// target (slot 0x138, [`UnitAi::move_to_object`], within the desired distance) or to the
    /// point (slot 0x130 forced, running, within the desired distance). Within it, a defend
    /// target with a unit AI names the attacker of its nearest to it (its AI's slot 0x78,
    /// [`UnitSide::defended_attacker`]), which the unit attacks (slot 0xb8 forced, not any:
    /// [`UnitAi::attack_slot`]); with none, its processIdle (slot 0x240 with 0,
    /// [`UnitAi::process_idle`]) and an action it produced end slot 73. True when slot 73 ends
    /// here (10, or 9 after the order ended).
    fn defend_misc(
        &mut self,
        at: &Surroundings,
        computer: bool,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<bool, GenieError> {
        let target = self.defend_target;
        let mut defended = None;
        if target != -1 {
            let held = Self::world_object(target, at, side)?.filter(|t| t.allied);
            let ends = match held {
                None => true,
                Some(t) => {
                    if self.target_point[0] == INVALID_POINT {
                        self.target_point[0] = t.position[0];
                        self.target_point[1] = t.position[1];
                    }
                    2 < t.state && t.owner != 0
                }
            };
            if ends {
                self.defend_target = -1;
                self.remove_target();
                self.stop_current(true, at, fx)?;
                self.leave_patrol_timers(ai);
                self.current_order = -1;
                return Ok(true);
            }
            defended = held;
        }
        let toward_it = self.current_action == MOVE_ACTION
            && at.top_target.map_or(-1, |t| t.id) == self.defend_target;
        if matches!(self.current_action, ATTACK_ACTION | MOVE_ASIDE_ACTION) || toward_it {
            return Ok(false);
        }
        let gap = if let Some(t) = defended {
            self.target_point = t.position;
            t.gap_squared
        } else {
            let [x, y, _] = self.target_point;
            point_gap_squared([x, y], at.position, at.size)
        };
        let reach = self.desired_distance + DEFEND_SLACK;
        if reach * reach < gap {
            if let Some(t) = defended {
                let range = self.desired_distance;
                self.move_to_object(target, (t.state, t.class, t.position), range, fx);
            } else {
                let m = self.move_to(AiMove {
                    point: self.target_point,
                    range: self.desired_distance,
                    force: true,
                    walk: false,
                });
                fx.extend(m.map(Effect::Move));
            }
            return Ok(true);
        }
        if defended.is_some()
            && let Some(attacker) = side.defended_attacker(target)?
        {
            if attacker != -1 {
                let _ = self.attack_slot(attacker, true, at, side, fx)?;
                return Ok(true);
            }
            return self.process_idle(at, computer, ai, side, fx);
        }
        Ok(false)
    }

    /// processMisc's follow order (`0x140c567c0`, "I have a follow order for Unit #%d"): a
    /// target gone or dying (above state 2) or on a tile the unit's player does not see now ends
    /// it ("Target is gone or dying, stopping.", "He's NOT on a visible tile, stopping."): the
    /// defend target `+0x60` and the target go (`FUN_140c5e1e0`), the current action
    /// stops (slot 0xb0 with 1) and the order is −1. On action `0x2bf` or `0x2c0` the target
    /// point becomes (−1, −1). Otherwise a target more than a tile off the AI's target point on
    /// either axis gives the point its place (x, y, z); when the footprints' squared gap
    /// (`FUN_14118f8c0`) then exceeds the desired distance squared, the AI moves to it
    /// ([`UnitAi::move_to_object`] within the desired distance). True when the check ends there.
    fn follow(&mut self, at: &Surroundings, fx: &mut Vec<Effect>) -> Result<bool, GenieError> {
        let target = match at.target {
            Some(t) if t.state <= 2 && t.visible => t,
            _ => {
                self.defend_target = -1;
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
            let held = (target.state, target.class, target.position);
            self.move_to_object(self.current_target, held, range, fx);
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
    fn move_to_object(
        &mut self,
        id: i32,
        (state, class, position): (u8, i16, [f32; 3]),
        range: f32,
        fx: &mut Vec<Effect>,
    ) {
        fx.push(Effect::MoveToObject {
            target: id,
            state,
            range,
        });
        self.set_action(MOVE_ACTION);
        self.target_at(id, Some(i32::from(class)), position);
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
    fn look_around(&mut self, computer: bool, ai: &mut Well512) -> Result<(), GenieError> {
        if self.look_around_timer > self.look_around_timeout {
            // the human branch; `FUN_14152b470` sends players allied with a computer elsewhere
            if computer {
                return Err(GenieError::unsupported(
                    "look-around timer of a computer player",
                ));
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

    /// A side whose searches find nothing (`Found::NONE`).
    fn finds_nothing() -> impl FnMut(&Search<'_>) -> Result<Found, GenieError> {
        |_| Ok(Found::NONE)
    }

    impl UnitAi {
        const fn is_idle(&self) -> bool {
            self.current_order == -1 && self.current_action == -1 && self.current_target == -1
        }

        /// The full update of an idle unit (no order, action or target), with its move.
        fn update_idle(
            &mut self,
            dt: u32,
            world_time: u32,
            computer: bool,
            at: &Surroundings,
            ai: &mut Well512,
            side: &mut dyn UnitSide,
        ) -> Result<Option<AiMove>, GenieError> {
            if !self.is_idle() {
                return Err(GenieError::unsupported("unit is not idle"));
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
            best_this_update: -1,
            ..UnitAi::default()
        }
    }

    fn rng() -> Well512 {
        let mut r = Well512::default();
        r.set_seed(0);
        r
    }

    /// A unit side that records the attack commands, the under-attack flag, the stops (as
    /// many attack commands as came before each) and the caps on the last human order time (with
    /// as many stops before each).
    #[derive(Default)]
    struct Recorder {
        attacks: Vec<(i32, bool)>,
        under_attack: Option<bool>,
        stops: Vec<usize>,
        caps: Vec<(u32, usize)>,
        /// The drop-site table's building it answers, and the top action's target.
        drop_site: Option<i32>,
        top: Option<i32>,
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
        fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
            None
        }
        fn table_drop_site(&mut self) -> Result<Option<i32>, GenieError> {
            Ok(self.drop_site)
        }
        fn top_target_now(&mut self) -> Result<Option<i32>, GenieError> {
            Ok(self.top)
        }
        fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }

        fn attack(&mut self, target: i32, force: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
            self.attacks.push((target, force));
            Ok(Attacked {
                attacks: true,
                posted: Vec::new(),
            })
        }

        fn set_under_attack(&mut self, value: bool) {
            self.under_attack = Some(value);
        }

        fn stop(&mut self) -> Result<(), GenieError> {
            self.stops.push(self.attacks.len());
            Ok(())
        }

        fn cap_human_order(&mut self, cap: u32) -> Result<(), GenieError> {
            self.caps.push((cap, self.stops.len()));
            Ok(())
        }

        fn may_attack(
            &mut self,
            _: i32,
            _: &Seeker,
        ) -> Result<Option<(i16, [f32; 3])>, GenieError> {
            Err(GenieError::unsupported("no world here"))
        }

        fn seek_unexplored(
            &mut self,
            _: &UnitAi,
            _: &mut Well512,
        ) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }

        fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }

        fn make_scout_map(&mut self) -> Result<(), GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
    }

    /// A unit that calls for help: whether the world holds the attacker, the searches it ran
    /// (classes and radius) and the attackers its walk got.
    struct HelpSide {
        holds: bool,
        searches: Vec<(Vec<i32>, f32)>,
        walks: Vec<i32>,
    }

    impl UnitSide for HelpSide {
        fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
            None
        }
        fn closest(&mut self, search: &Search<'_>) -> Result<Found, GenieError> {
            self.searches.push((search.classes.to_vec(), search.radius));
            Ok(Found {
                id: -1,
                enemies: true,
                position: None,
                fresh: true,
                players: 0,
                distance: 0,
                retries: [None; 3],
            })
        }
        fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
            Err(GenieError::unsupported("no attack here"))
        }
        fn set_under_attack(&mut self, _: bool) {}
        fn stop(&mut self) -> Result<(), GenieError> {
            Ok(())
        }
        fn may_attack(
            &mut self,
            _: i32,
            _: &Seeker,
        ) -> Result<Option<(i16, [f32; 3])>, GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
        fn seek_unexplored(
            &mut self,
            _: &UnitAi,
            _: &mut Well512,
        ) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn make_scout_map(&mut self) -> Result<(), GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
        fn seen_unit(&mut self, id: i32) -> Result<Option<UnitSeen>, GenieError> {
            Ok(self.holds.then_some(UnitSeen {
                id,
                class: 35,
                owner: 0,
                combat_level: 0,
                state: 4,
                target: None,
                gap_squared: 0.0,
                position: [101.5, 158.5, 0.0],
                hp: 0.0,
                master: 0,
                kind: 0,
                main: None,
                allied: false,
            }))
        }
        fn call_for_help(&mut self, attacker: i32) -> Result<(), GenieError> {
            self.walks.push(attacker);
            Ok(())
        }
    }

    /// The priest's slot 0x1b8 (`FUN_140c1af00`): faith asked unless `any`; never Gaia, sheep,
    /// town centres, walls or a master whose hero flag forbids it; the convert task's attribute
    /// (`+0x26`) must be above 0 for the object's class.
    #[test]
    fn a_priest_converts_by_its_faith_its_task_and_the_object() {
        let attributes = {
            let mut a = vec![0.0f32; 100];
            a[67] = 1.0;
            a[28] = 0.0;
            a
        };
        let tasks = [
            crate::masters::Task {
                action_type: 0x68,
                ..crate::masters::Task::default()
            },
            crate::masters::Task {
                action_type: 0x68,
                object_group: 3,
                attribute_type4: 28,
                ..crate::masters::Task::default()
            },
        ];
        let monk = Converter {
            class: 18,
            master: 125,
            faith: 100.0,
            attributes: &attributes,
            tasks: &tasks,
        };
        let knight = Convertee {
            owner: 2,
            class: 12,
            master: 38,
            hero: 0,
        };
        assert!(can_convert(&monk, Some(&knight), false));
        let tired = Converter {
            faith: 99.0,
            ..monk
        };
        assert!(!can_convert(&tired, Some(&knight), false));
        assert!(can_convert(&tired, Some(&knight), true));
        assert!(!can_convert(&monk, None, true));
        let gaia = Convertee { owner: 0, ..knight };
        assert!(!can_convert(&monk, Some(&gaia), true));
        let sheep = Convertee {
            class: 58,
            ..knight
        };
        assert!(!can_convert(&monk, Some(&sheep), true));
        let tc = Convertee {
            class: 3,
            master: 109,
            ..knight
        };
        assert!(!can_convert(&monk, Some(&tc), true));
        let hero = Convertee { hero: 1, ..knight };
        assert!(!can_convert(&monk, Some(&hero), true));
        // a building's task names attribute 28, which the owner holds at 0
        let house = Convertee {
            class: 3,
            master: 70,
            ..knight
        };
        assert!(!can_convert(&monk, Some(&house), true));
    }

    /// `FUN_140c42080`: an order joins the history first; a ninth recycles the oldest.
    #[test]
    fn the_order_history_keeps_the_eight_newest_first() {
        let mut ai = UnitAi::default();
        for t in 0..10 {
            ai.remember_order(OrderRecord {
                order: 700,
                action: 600,
                time: t,
                target: 100 + t as i32,
                category: 1,
                ..OrderRecord::default()
            });
        }
        let targets: Vec<i32> = ai.order_history.iter().map(|r| r.target).collect();
        assert_eq!(targets, [109, 108, 107, 106, 105, 104, 103, 102]);
    }

    /// The full update's start (`0x140c66b10`): an expired retarget entry goes and a live one
    /// stays; a target gone or below 1 hit point empties them all.
    #[test]
    fn the_retarget_entries_expire_and_a_dead_target_empties_them() {
        let seen = |hp: f32| UnitSeen {
            hp,
            ..UnitSeen::default()
        };
        let mut ai = UnitAi {
            retries: vec![(1, 900), (2, 1100), (3, 1000)],
            ..UnitAi::default()
        };
        ai.expire_retries(1000, &mut |_| Ok(Some(seen(5.0))))
            .unwrap();
        assert_eq!(ai.retries, [(2, 1100)]);
        let mut ai = UnitAi {
            retries: vec![(1, 2000), (2, 2000)],
            ..UnitAi::default()
        };
        ai.expire_retries(1000, &mut |id| Ok((id == 1).then(|| seen(5.0))))
            .unwrap();
        assert_eq!(ai.retries, []);
        let mut ai = UnitAi {
            retries: vec![(1, 2000)],
            ..UnitAi::default()
        };
        ai.expire_retries(1000, &mut |_| Ok(Some(seen(0.5))))
            .unwrap();
        assert_eq!(ai.retries, []);
    }

    /// `AskForHelp` (`FUN_140c42ce0`): an attacker the world does not hold asks nothing; else the
    /// unit searches from its place (slot 0x50, mode 1) over the classes its constructor gave
    /// it within its search radius (the line of sight plus 1 when farther; plus 7 for the
    /// villager and the fishing ship), the search sets the alert mode, and the walk of its lists
    /// looks for helpers. 426048706's gate 7558 (a building, p3) in update 4.
    #[test]
    fn a_call_for_help_searches_what_the_unit_cares_about_then_walks_the_lists() {
        let at = Surroundings {
            position: [124.5, 174.5, 0.0],
            los: 6.0,
            search_radius: 4.0,
            class: 39,
            ..Surroundings::default()
        };
        let mut gone = HelpSide {
            holds: false,
            searches: Vec::new(),
            walks: Vec::new(),
        };
        let mut gate = idle(BUILDING_AI);
        gate.ask_for_help(7955, &at, false, &mut gone).unwrap();
        assert_eq!((gone.searches.len(), gone.walks.len()), (0, 0));
        let mut side = HelpSide {
            holds: true,
            searches: Vec::new(),
            walks: Vec::new(),
        };
        gate.ask_for_help(7955, &at, false, &mut side).unwrap();
        assert_eq!(side.searches, [(BUILDING_CARED.to_vec(), 7.0)]);
        assert_eq!(side.walks, [7955]);
        assert!(gate.alert_mode);
        let mut villager = idle(CIVILIAN);
        let mut side = HelpSide {
            holds: true,
            searches: Vec::new(),
            walks: Vec::new(),
        };
        villager.ask_for_help(7955, &at, false, &mut side).unwrap();
        assert_eq!(side.searches, [(CIVILIAN_CARED.to_vec(), 14.0)]);
        for (ai_type, class, cared) in [
            (TRANSPORT_AI, 20, &TRANSPORT_CARED[..]),
            (SIEGE_TOWER_AI, 13, &SIEGE_TOWER_CARED[..]),
            (TRADE_AI, 2, &TRADE_SHIP_CARED[..]),
            (TRADE_AI, 19, &TRADE_CART_CARED[..]),
        ] {
            let at = Surroundings {
                class,
                ..at.clone()
            };
            let mut side = HelpSide {
                holds: true,
                searches: Vec::new(),
                walks: Vec::new(),
            };
            idle(ai_type)
                .ask_for_help(7955, &at, false, &mut side)
                .unwrap();
            assert_eq!(side.searches, [(cared.to_vec(), 7.0)]);
        }
        let mut side = HelpSide {
            holds: true,
            searches: Vec::new(),
            walks: Vec::new(),
        };
        idle(FISHING_AI)
            .ask_for_help(7955, &at, false, &mut side)
            .unwrap();
        assert_eq!(side.searches, [(FISHING_CARED.to_vec(), 14.0)]);
    }

    /// A unit sent off a footprint: what [`UnitSide::off_footprint`] answers.
    struct OffSide(OffFootprint, Vec<(i32, [f32; 2])>);

    impl UnitSide for OffSide {
        fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
            None
        }
        fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
            Err(GenieError::unsupported("no attack here"))
        }
        fn set_under_attack(&mut self, _: bool) {}
        fn stop(&mut self) -> Result<(), GenieError> {
            Ok(())
        }
        fn may_attack(
            &mut self,
            _: i32,
            _: &Seeker,
        ) -> Result<Option<(i16, [f32; 3])>, GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
        fn seek_unexplored(
            &mut self,
            _: &UnitAi,
            _: &mut Well512,
        ) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn make_scout_map(&mut self) -> Result<(), GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
        fn off_footprint(
            &mut self,
            site: i32,
            radius: [f32; 2],
        ) -> Result<OffFootprint, GenieError> {
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

    /// 426049848's deer 14054 (Gaia's, unit-AI type 2, no actions) in update 685: a town centre's
    /// foundation sent it off its footprint (0x216). Its move off (slot 0x148 forced: the unit's
    /// slot 0x340) gives it its `MoveTo` at once, so the full update's later check (slot 0x430)
    /// sees it busy and the action value 0x2bf stays.
    #[test]
    fn a_unit_sent_off_a_footprint_keeps_its_move_aside_through_the_update() {
        let mut u = idle(2);
        u.post(Notify::own(14054, SENT_OFF, [200, 200, 14100]), 2)
            .unwrap();
        let off = OffFootprint {
            spot: Some([164.5, 51.5]),
            z: 0.0,
            class: PREY_CLASS,
            computer: false,
            player: 0,
            site: None,
            action_target: None,
        };
        let mut side = OffSide(off, Vec::new());
        let at = Surroundings {
            position: [164.5, 50.5, 0.0],
            state: 2,
            class: PREY_CLASS,
            ..Surroundings::default()
        };
        let fx = u
            .update(50, 22643, false, &at, &mut rng(), &mut side)
            .unwrap();
        assert_eq!(
            fx,
            vec![Effect::MoveAside(AiMove {
                point: [164.5, 51.5, 0.0],
                range: 0.0,
                force: true,
                walk: false,
            })]
        );
        assert_eq!((u.current_action, u.last_action), (MOVE_ASIDE_ACTION, -1));
    }

    /// What the unit's slot 0x388 answers ([`UnitSide::wait_solidify`]), and the foundations
    /// asked.
    struct WaitSide(bool, Vec<i32>);

    impl UnitSide for WaitSide {
        fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
            None
        }
        fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
            Err(GenieError::unsupported("no attack here"))
        }
        fn set_under_attack(&mut self, _: bool) {}
        fn stop(&mut self) -> Result<(), GenieError> {
            Ok(())
        }
        fn may_attack(
            &mut self,
            _: i32,
            _: &Seeker,
        ) -> Result<Option<(i16, [f32; 3])>, GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
        fn seek_unexplored(
            &mut self,
            _: &UnitAi,
            _: &mut Well512,
        ) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn make_scout_map(&mut self) -> Result<(), GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
        fn wait_solidify(&mut self, target: i32) -> Result<bool, GenieError> {
            self.1.push(target);
            Ok(self.0)
        }
    }

    /// The queue a villager sent off a foundation's footprint holds once its move aside ended
    /// (0x2bf, which the full update resets): the foundation's order, then its work.
    fn sent_off_queue() -> UnitAi {
        let order = |order: i32, target: i32, x: f32| QueuedOrder {
            issuer: 1,
            order,
            priority: 100,
            target,
            target_owner: 1,
            point: [x, 2.0, 0.0],
            range: 0.0,
            shown: [x, 2.0],
            ready: true,
            target_type: 70,
        };
        UnitAi {
            current_action: -1,
            last_action: MOVE_ASIDE_ACTION,
            current_order: -1,
            order_priority: -1,
            current_target: -1,
            orders: vec![
                order(FOUNDATION_ORDER, 10028, 30.0),
                order(ORDER_ON_TARGET, 600, 33.5),
            ],
            ..UnitAi::default()
        }
    }

    /// Case 0x2be, the build (`FUN_140c5c170`): `BuildObject` forced (slot 0xf0, `FUN_140c47d80`)
    /// gives the unit its build command, which the caller runs after the update in the order the
    /// AI gave it ([`Effect::BuildObject`]), and at once makes the build the AI's action and the
    /// foundation its target at its place with its class; the tail takes the order and its
    /// priority (428326873's villager 8766 at update 1804, whose hunt after it must come later).
    #[test]
    fn the_build_order_builds_and_takes_the_order_at_once() {
        let mut ai = UnitAi {
            current_action: -1,
            current_order: -1,
            current_target: 7,
            target_type: 10,
            orders: vec![QueuedOrder {
                issuer: 1,
                order: BUILD_ORDER,
                priority: -1,
                target: 10028,
                target_owner: 1,
                point: [30.0, 88.0, 0.0],
                range: 2.0,
                shown: [30.0, 88.0],
                ready: true,
                target_type: 3,
            }],
            ..UnitAi::default()
        };
        let at = Surroundings {
            units: vec![UnitSeen {
                id: 10028,
                class: 3,
                position: [30.0, 88.0, 0.0],
                ..UnitSeen::default()
            }],
            ..scout_at(true, false)
        };
        let mut side = WaitSide(false, Vec::new());
        let order = ai.orders[0];
        let mut fx = Vec::new();
        let r = ai
            .process_order(
                &order,
                Some((&at, &mut side, false)),
                None,
                &mut rng(),
                &mut fx,
            )
            .unwrap();
        assert_eq!((r, fx), (1, vec![Effect::BuildObject(10028)]));
        assert_eq!(
            (
                ai.current_action,
                ai.current_target,
                ai.target_type,
                ai.target_point
            ),
            (BUILD_ACTION, 10028, 3, [30.0, 88.0, 0.0])
        );
        assert_eq!((ai.last_target, ai.last_target_type), (7, 10));
        assert_eq!((ai.current_order, ai.order_priority), (BUILD_ORDER, -1));
    }

    /// Case 0x2de (`FUN_140c5c170`): slot 0x108 forced (`FUN_140c68a60`) runs the unit's slot
    /// 0x388 on the foundation; its 1 makes 0x27a the action value with the old one the last,
    /// then the case sets 0x27a again (the last takes it) and the tail takes the order and its
    /// priority, which ends the queue's turn: the work waits in the queue.
    #[test]
    fn a_foundation_order_waits_for_the_foundation_and_ends_the_queue_s_turn() {
        let mut ai = sent_off_queue();
        let mut side = WaitSide(true, Vec::new());
        let fx = ai
            .off_turn(&scout_at(true, false), &mut side, false, &mut rng())
            .unwrap();
        assert_eq!(fx, vec![]);
        assert_eq!(side.1, vec![10028]);
        assert_eq!(
            (ai.current_action, ai.last_action),
            (WAIT_SOLIDIFY_ACTION, WAIT_SOLIDIFY_ACTION)
        );
        assert_eq!(
            (ai.current_order, ai.order_priority),
            (FOUNDATION_ORDER, 100)
        );
        let left: Vec<_> = ai.orders.iter().map(|o| (o.order, o.target)).collect();
        assert_eq!(left, vec![(ORDER_ON_TARGET, 600)]);
    }

    /// Slot 0x388's 0 (no foundation, or no task on it): the case still makes 0x27a the action
    /// value, the old one the last, and returns 0; the order goes and the queue runs the next,
    /// the work on its old target (priority 100 passes the action value).
    #[test]
    fn a_foundation_order_without_a_wait_goes_and_the_work_runs() {
        let mut ai = sent_off_queue();
        let mut side = WaitSide(false, Vec::new());
        // the world holds the work's target
        let at = Surroundings {
            units: vec![UnitSeen {
                id: 600,
                ..UnitSeen::default()
            }],
            ..scout_at(true, false)
        };
        let fx = ai.off_turn(&at, &mut side, false, &mut rng()).unwrap();
        assert_eq!(side.1, vec![10028]);
        assert_eq!(
            (ai.current_action, ai.last_action),
            (WAIT_SOLIDIFY_ACTION, -1)
        );
        assert_eq!((ai.current_order, ai.order_priority), (-1, -1));
        assert_eq!(ai.orders, vec![]);
        let [Effect::OrderOnTarget(work)] = fx.as_slice() else {
            panic!("the work on its target: {fx:?}");
        };
        assert_eq!((work.order, work.target), (ORDER_ON_TARGET, 600));
    }

    /// Slot 0x108 (`FUN_140c68a60`): unforced, a unit with an action value does not ask its
    /// unit; forced or without one it does, and only its 1 changes the action values.
    #[test]
    fn slot_0x108_asks_the_unit_when_forced_or_without_an_action_value() {
        let mut ai = UnitAi {
            current_action: GATHER_ACTION,
            last_action: 7,
            ..UnitAi::default()
        };
        let mut side = WaitSide(true, Vec::new());
        assert_eq!(ai.wait_for_foundation(10028, false, &mut side), Ok(false));
        assert_eq!(side.1, Vec::<i32>::new());
        assert_eq!((ai.current_action, ai.last_action), (GATHER_ACTION, 7));
        let mut refused = WaitSide(false, Vec::new());
        assert_eq!(ai.wait_for_foundation(10028, true, &mut refused), Ok(false));
        assert_eq!(refused.1, vec![10028]);
        assert_eq!((ai.current_action, ai.last_action), (GATHER_ACTION, 7));
        assert_eq!(ai.wait_for_foundation(10028, true, &mut side), Ok(true));
        assert_eq!(
            (ai.current_action, ai.last_action),
            (WAIT_SOLIDIFY_ACTION, GATHER_ACTION)
        );
        let mut idle = UnitAi {
            current_action: -1,
            ..UnitAi::default()
        };
        assert_eq!(idle.wait_for_foundation(10028, false, &mut side), Ok(true));
        assert_eq!(
            (idle.current_action, idle.last_action),
            (WAIT_SOLIDIFY_ACTION, -1)
        );
    }

    /// What [`UnitSide::closest_resource`] answers, and the searches asked (group, workers cap,
    /// drop site, current target).
    struct ResourceSide(Option<Gatherable>, Vec<(u8, i32, i32, i32)>);

    impl UnitSide for ResourceSide {
        fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
            None
        }
        fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
            Err(GenieError::unsupported("no attack here"))
        }
        fn set_under_attack(&mut self, _: bool) {}
        fn stop(&mut self) -> Result<(), GenieError> {
            Ok(())
        }
        fn may_attack(
            &mut self,
            _: i32,
            _: &Seeker,
        ) -> Result<Option<(i16, [f32; 3])>, GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
        fn seek_unexplored(
            &mut self,
            _: &UnitAi,
            _: &mut Well512,
        ) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn make_scout_map(&mut self) -> Result<(), GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
        fn closest_resource(
            &mut self,
            search: ResourceSearch,
        ) -> Result<Option<Gatherable>, GenieError> {
            assert_eq!((search.point, search.min_state), (NO_POINT, 2));
            self.1
                .push((search.kind, search.cap, search.site, search.current));
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

    /// The villager's notify 0x202 on its hunt (`FUN_140c22bf0`, action 0x265) of a class the
    /// civilian's slot 0x1b0 names a kind of: `ClosestResourceObject` (slot 0x80) from the
    /// notify's drop site, kind 0 below 2 workers, another any; what it finds it hunts
    /// (`HuntObject`, slot 0xc8 with 1). Finding nothing a human player's villager stays.
    #[test]
    fn a_hunter_of_a_kind_searches_by_it_from_its_drop_site() {
        let tree = Gatherable {
            id: 28300,
            position: [50.5, 60.5, 0.0],
            class: 15,
        };
        let mut ai = UnitAi {
            ai_type: CIVILIAN,
            current_action: 0x265,
            current_target: 28301,
            target_type: 15,
            ..UnitAi::default()
        };
        let mut side = ResourceSide(Some(tree), Vec::new());
        let mut fx = Vec::new();
        let at = Surroundings::default();
        let n = Notify::own(31237, REQUIRES_SEARCH, [0x265, 15, 31247]);
        assert_eq!(
            ai.hunter_search(n, &at, false, &mut side, &mut fx),
            Ok(BREAK)
        );
        assert_eq!(side.1, vec![(0, 2, 31247, 28301)]);
        assert_eq!(fx, vec![Effect::HuntObject(28300)]);
        assert_eq!(ai.current_target, 28300);
        let mut ai = UnitAi {
            ai_type: CIVILIAN,
            current_action: 0x265,
            current_target: -1,
            ..UnitAi::default()
        };
        let mut side = ResourceSide(None, Vec::new());
        let mut fx = Vec::new();
        let n = Notify::own(31237, REQUIRES_SEARCH, [0x265, 32, -1]);
        assert_eq!(
            ai.hunter_search(n, &at, false, &mut side, &mut fx),
            Ok(GO_ON)
        );
        assert_eq!(side.1, vec![(4, -1, -1, -1)]);
        assert_eq!(fx, Vec::new());
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

    /// `FUN_140c63e20`'s alert: a fresh search's enemies set the alert mode and start the time
    /// since the last sighting again; a fresh one without clears the alert mode; a search that
    /// read the last answer changes nothing.
    #[test]
    fn a_fresh_search_s_enemies_set_the_alert_mode() {
        let found = |fresh, enemies| Found {
            enemies,
            fresh,
            ..Found::NONE
        };
        let mut u = UnitAi {
            time_since_enemy: 12.0,
            ..idle(CIVILIAN)
        };
        u.alert(found(true, true), 0.05);
        assert_eq!((u.alert_mode, u.time_since_enemy), (true, 0.0));
        u.time_since_enemy = 3.0;
        u.alert(found(false, false), 0.05);
        assert_eq!((u.alert_mode, u.time_since_enemy), (true, 3.0));
        u.alert(found(true, false), 0.05);
        assert_eq!((u.alert_mode, u.time_since_enemy), (false, 3.0));
    }

    /// 428506340's Gaia 31602 (update 4609): a cap on the last human order time and a stop wait
    /// when an attack command comes; both run before it, in their order. Another effect still
    /// stops the slice.
    #[test]
    fn an_attack_command_runs_the_pending_cap_and_stop_first() {
        let mut u = hit_boar();
        let at = Surroundings {
            id: 31602,
            ..Surroundings::default()
        };
        let mut side = Recorder::default();
        let mut fx = vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP), Effect::Stop];
        assert_eq!(
            u.attack_command(27240, true, &at, &mut side, &mut fx),
            Ok(true)
        );
        assert_eq!(
            (side.caps, side.stops, side.attacks, fx),
            (
                vec![(HUMAN_ORDER_CAP, 0)],
                vec![0],
                vec![(27240, true)],
                vec![]
            )
        );
        let mut fx = vec![Effect::Stop, Effect::RestorePatrol];
        assert!(
            u.attack_command(27240, true, &at, &mut Recorder::default(), &mut fx)
                .is_err()
        );
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
        fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
            None
        }
        fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
            let id = self.found[self.searches];
            self.searches += 1;
            Ok(Found {
                id,
                enemies: id != -1,
                position: Some([0.0; 3]),
                fresh: true,
                players: 0,
                distance: -1,
                retries: [None; 3],
            })
        }
        fn attack(&mut self, target: i32, force: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
            self.attacks.push((target, force));
            Ok(Attacked {
                attacks: true,
                posted: Vec::new(),
            })
        }
        fn set_under_attack(&mut self, _: bool) {}
        fn stop(&mut self) -> Result<(), GenieError> {
            Ok(())
        }
        fn may_attack(
            &mut self,
            _: i32,
            _: &Seeker,
        ) -> Result<Option<(i16, [f32; 3])>, GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
        fn seek_unexplored(
            &mut self,
            _: &UnitAi,
            _: &mut Well512,
        ) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn make_scout_map(&mut self) -> Result<(), GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
        fn seen_unit(&mut self, id: i32) -> Result<Option<UnitSeen>, GenieError> {
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
        patrol: Option<(PatrolPath, i32)>,
    }

    impl UnitSide for SoldierSide {
        fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
            self.patrol.clone()
        }
        fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
            Ok(Found {
                id: self.found,
                enemies: self.found != -1,
                position: Some([10.0, 11.0, 0.0]),
                fresh: true,
                players: 0,
                distance: -1,
                retries: [None; 3],
            })
        }
        fn attack(&mut self, target: i32, force: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
            self.attacks.push((target, force));
            Ok(Attacked {
                attacks: true,
                posted: Vec::new(),
            })
        }
        fn set_under_attack(&mut self, _: bool) {}
        fn stop(&mut self) -> Result<(), GenieError> {
            Ok(())
        }
        fn may_attack(
            &mut self,
            _: i32,
            _: &Seeker,
        ) -> Result<Option<(i16, [f32; 3])>, GenieError> {
            Ok(Some((58, [10.0, 11.0, 0.0])))
        }
        fn seek_unexplored(
            &mut self,
            _: &UnitAi,
            _: &mut Well512,
        ) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn make_scout_map(&mut self) -> Result<(), GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
    }

    /// A side for the base handler's damage case: the objects it holds, the attack search's
    /// answer, the reach, the duels, and what the case asked of it.
    #[derive(Default)]
    struct DamageSide {
        units: Vec<UnitSeen>,
        found: i32,
        reaches: bool,
        top: Option<i32>,
        duels: Vec<(i32, Duel)>,
        attack_searches: usize,
        attacks: Vec<i32>,
        helps: Vec<i32>,
        under_attack: Vec<bool>,
        patrol: Option<(PatrolPath, i32)>,
        harmless: Vec<i32>,
        defended: Option<i32>,
    }

    impl UnitSide for DamageSide {
        fn closest(&mut self, search: &Search<'_>) -> Result<Found, GenieError> {
            let id = if matches!(search.pick, Pick::Attack { .. }) {
                self.attack_searches += 1;
                self.found
            } else {
                -1
            };
            Ok(Found {
                id,
                enemies: id != -1,
                position: None,
                fresh: true,
                players: 0,
                distance: -1,
                retries: [None; 3],
            })
        }
        fn attack(&mut self, target: i32, _: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
            self.attacks.push(target);
            Ok(Attacked {
                attacks: true,
                posted: Vec::new(),
            })
        }
        fn set_under_attack(&mut self, value: bool) {
            self.under_attack.push(value);
        }
        fn stop(&mut self) -> Result<(), GenieError> {
            Ok(())
        }
        fn may_attack(
            &mut self,
            target: i32,
            _: &Seeker,
        ) -> Result<Option<(i16, [f32; 3])>, GenieError> {
            Ok(self
                .units
                .iter()
                .find(|u| u.id == target)
                .map(|u| (u.class, u.position)))
        }
        fn seek_unexplored(
            &mut self,
            _: &UnitAi,
            _: &mut Well512,
        ) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn make_scout_map(&mut self) -> Result<(), GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
        fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
            self.patrol.clone()
        }
        fn reaches(&mut self, _: i32, _: f32) -> Result<bool, GenieError> {
            Ok(self.reaches)
        }
        fn call_for_help(&mut self, attacker: i32) -> Result<(), GenieError> {
            self.helps.push(attacker);
            Ok(())
        }
        fn seen_unit(&mut self, id: i32) -> Result<Option<UnitSeen>, GenieError> {
            Ok(self.units.iter().find(|u| u.id == id).copied())
        }
        fn duel(&mut self, other: i32) -> Result<Duel, GenieError> {
            self.duels
                .iter()
                .find(|d| d.0 == other)
                .map(|d| d.1)
                .ok_or(GenieError::unsupported("no duel here"))
        }
        fn top_target_now(&mut self) -> Result<Option<i32>, GenieError> {
            Ok(self.top)
        }
        fn harmless(&mut self, attacker: i32) -> Result<bool, GenieError> {
            Ok(self.harmless.contains(&attacker))
        }
        fn defended_attacker(&mut self, _: i32) -> Result<Option<i32>, GenieError> {
            Ok(self.defended)
        }
        fn can_stand(&mut self, _: [f32; 2]) -> Result<bool, GenieError> {
            Ok(true)
        }
    }

    /// A human player's villager at (10, 10) on its way to gather (order `0x2c5`, its Gather in
    /// state 5, animating) and its unit AI, hit by `foe(30, 6)`'s 4 damage.
    fn hit_villager() -> (UnitAi, Surroundings, Notify) {
        let ai = UnitAi {
            current_order: crate::ai_kinds::ORDER_GATHER,
            current_action: GATHER_ACTION,
            current_target: 40,
            order_priority: 3,
            ..idle(CIVILIAN)
        };
        let at = Surroundings {
            id: 20,
            position: [10.0, 10.0, 0.0],
            speed: 1.0,
            class: CLASS_VILLAGER,
            player: 1,
            state: 2,
            map: [100, 100],
            anim_speed: Some(1.0),
            action_state: Some(5),
            units: vec![foe(30, 6)],
            ..Surroundings::default()
        };
        let hit = Notify {
            caller: 30,
            recipient: 20,
            kind: DAMAGED,
            p: [30, 21, 25],
        };
        (ai, at, hit)
    }

    /// A villager hit by another player's unit it fears ("I have %d attackers, evading them.")
    /// calls for help, sends its order to the queue's end, stops its action and runs from the
    /// attacker (to its west, the attacker being east of it); the call keeps the attacker.
    #[test]
    fn a_villager_evades_another_player_s_attacker() {
        let (mut u, at, hit) = hit_villager();
        let mut side = DamageSide {
            units: vec![foe(30, 6)],
            ..DamageSide::default()
        };
        let mut fx = Vec::new();
        let r = u.villager_damaged(hit, 0, &at, false, &mut rng(), &mut side, &mut fx);
        assert_eq!(r, Ok(BREAK));
        assert_eq!((side.helps, side.under_attack), (vec![30], vec![true]));
        assert_eq!(u.attackers, vec![30]);
        let queued: Vec<_> = u
            .orders
            .iter()
            .map(|o| (o.order, o.target, o.issuer))
            .collect();
        assert_eq!(queued, vec![(crate::ai_kinds::ORDER_GATHER, 40, 20)]);
        assert_eq!(fx.first(), Some(&Effect::Stop));
        let Some(Effect::Move(m)) = fx.last() else {
            panic!("no move: {fx:?}");
        };
        assert!(m.point[0] < 10.0 && m.point[1] == 10.0, "{m:?}");
        assert_eq!(side.attacks, Vec::<i32>::new());
        assert_eq!(u.current_action, MOVE_ACTION);
    }

    /// The villager's cases 0x1fb, 0x1fc and 0x202 off its searching actions: on the move
    /// action an invalidated Attack (600) makes the target go and the action stop (records
    /// 426339330's villager 34028 in update 5296, running from its attackers); a search (0x202)
    /// or the action `0x26a` changes nothing; an invalidated gather or hunt of a class other
    /// than 10 would search for another of that class (not modelled).
    #[test]
    fn a_villager_s_invalidated_action_off_its_searches_stops_it() {
        let moving = || UnitAi {
            current_action: MOVE_ACTION,
            current_target: 30,
            ..idle(CIVILIAN)
        };
        let at = Surroundings {
            id: 20,
            state: 2,
            ..Surroundings::default()
        };
        let n = Notify::own(20, ACTION_INVALIDATED, [ATTACK_ACTION, 0, 514]);
        let mut side = DamageSide::default();
        let mut u = moving();
        let mut fx = Vec::new();
        assert_eq!(
            u.villager_search(n, &at, false, &mut side, &mut fx),
            Ok(GO_ON)
        );
        assert_eq!((fx, u.current_target), (vec![Effect::Stop], -1));
        for (kind, action) in [(REQUIRES_SEARCH, MOVE_ACTION), (TARGET_MOVED, 0x26a)] {
            let mut u = UnitAi {
                current_action: action,
                ..moving()
            };
            let mut fx = Vec::new();
            let n = Notify { kind, ..n };
            assert_eq!(
                u.villager_search(n, &at, false, &mut side, &mut fx),
                Ok(GO_ON)
            );
            assert_eq!((fx, u.current_target), (Vec::new(), 30));
        }
        let gather = Notify::own(20, ACTION_INVALIDATED, [GATHER_ACTION, 5, -1]);
        assert!(
            moving()
                .villager_search(gather, &at, false, &mut side, &mut Vec::new())
                .is_err()
        );
        let mut fx = Vec::new();
        let tree = Notify::own(20, ACTION_INVALIDATED, [GATHER_ACTION, 10, -1]);
        assert_eq!(
            moving().villager_search(tree, &at, false, &mut side, &mut fx),
            Ok(GO_ON)
        );
        assert_eq!(fx, vec![Effect::Stop]);
    }

    /// A villager dodges a missile's warning by its owner's chance (one AI draw); one it does
    /// not dodge is its damage.
    #[test]
    fn a_villager_dodges_a_missile_s_warning_or_takes_it_as_damage() {
        let r = rng().rand15() as i32 * 100 / 0x7fff;
        let (mut u, mut at, hit) = hit_villager();
        let warning = Notify {
            kind: MISSILE_WARNING,
            ..hit
        };
        at.dodge_chance = u8::try_from(r + 1).unwrap();
        let mut side = DamageSide {
            units: vec![foe(30, 6)],
            ..DamageSide::default()
        };
        let mut ai = rng();
        let mut fx = Vec::new();
        assert_eq!(
            u.handle(warning, 0, &at, false, &mut ai, &mut side, &mut fx),
            Ok(GO_ON)
        );
        assert_eq!((ai.draws, fx.len(), u.attackers.len()), (1, 0, 0));
        at.dodge_chance = u8::try_from(r).unwrap();
        assert_eq!(
            u.handle(warning, 0, &at, false, &mut rng(), &mut side, &mut fx),
            Ok(BREAK)
        );
        assert_eq!(u.attackers, vec![30]);
    }

    /// A villager's harmless attacker ([`UnitSide::harmless`]) is attacked back, as a Gaia
    /// attacker is: without an order straight away (`AttackObject` forced), and it stops after
    /// the kill.
    #[test]
    fn a_villager_attacks_a_harmless_attacker_back() {
        let (mut u, at, hit) = hit_villager();
        u.current_order = -1;
        u.current_action = -1;
        u.current_target = -1;
        let mut side = DamageSide {
            units: vec![foe(30, 6)],
            harmless: vec![30],
            ..DamageSide::default()
        };
        let mut fx = Vec::new();
        let r = u.villager_damaged(hit, 0, &at, false, &mut rng(), &mut side, &mut fx);
        assert_eq!(r, Ok(BREAK));
        assert_eq!(side.attacks, vec![30]);
        assert!(u.stop_after_kill && u.orders.is_empty());
    }

    /// On the move action (`0x262`) a villager at a speed keeps moving (and returns 3); at a
    /// standstill it evades; without its animation speed the slice stops.
    #[test]
    fn a_moving_villager_keeps_its_way() {
        let (mut u, at, hit) = hit_villager();
        u.current_action = MOVE_ACTION;
        let mut side = DamageSide {
            units: vec![foe(30, 6)],
            ..DamageSide::default()
        };
        let mut fx = Vec::new();
        let walking = u.clone();
        let r = u.villager_damaged(hit, 0, &at, false, &mut rng(), &mut side, &mut fx);
        assert_eq!(r, Ok(BREAK));
        assert!(fx.is_empty() && u.orders.is_empty());
        let nan = Surroundings {
            anim_speed: Some(f32::NAN),
            ..at.clone()
        };
        let mut u = walking.clone();
        let r = u.villager_damaged(hit, 0, &nan, false, &mut rng(), &mut side, &mut fx);
        assert_eq!(r, Ok(BREAK));
        assert!(fx.is_empty() && u.orders.is_empty());
        let still = Surroundings {
            anim_speed: Some(0.0),
            ..at.clone()
        };
        let mut u = walking.clone();
        let r = u.villager_damaged(hit, 0, &still, false, &mut rng(), &mut side, &mut fx);
        assert_eq!(r, Ok(BREAK));
        assert_eq!(u.orders.len(), 1);
        assert!(matches!(fx.last(), Some(Effect::Move(_))));
        let unknown = Surroundings {
            anim_speed: None,
            ..at
        };
        let mut u = walking;
        assert!(
            u.villager_damaged(
                hit,
                0,
                &unknown,
                false,
                &mut rng(),
                &mut side,
                &mut Vec::new()
            )
            .is_err()
        );
    }

    /// A friend (player 1, allied) at (11, 10) the soldier guards, `gap_squared` from it.
    fn ward(gap_squared: f32) -> UnitSeen {
        UnitSeen {
            id: 20,
            class: 6,
            owner: 1,
            state: 2,
            position: [11.0, 10.0, 0.0],
            gap_squared,
            kind: 70,
            allied: true,
            ..UnitSeen::default()
        }
    }

    /// A soldier on the defend order of `defend_target` within 2.5.
    fn guarding(defend_target: i32) -> UnitAi {
        UnitAi {
            current_order: DEFEND_ORDER,
            current_action: DEFEND_ACTION,
            current_target: defend_target,
            defend_target,
            desired_distance: 2.5,
            target_point: [11.0, 10.0, 0.0],
            ..idle(13)
        }
    }

    /// processMisc's defend order: a ward the world does not hold, another player's or a dead
    /// player's ward ends it (the target goes, the action stops, the order is −1); Gaia's dead
    /// ward does not; an unset target point takes the ward's place.
    #[test]
    fn a_defend_order_ends_with_its_ward_gone_foreign_or_dead() {
        let at = soldier_at();
        let foreign = UnitSeen {
            allied: false,
            ..ward(1.0)
        };
        let dead = UnitSeen {
            state: 3,
            ..ward(1.0)
        };
        for units in [vec![], vec![foreign], vec![dead]] {
            let mut u = guarding(20);
            let mut side = DamageSide {
                units,
                ..DamageSide::default()
            };
            let mut fx = Vec::new();
            assert_eq!(
                u.defend_misc(&at, false, &mut rng(), &mut side, &mut fx),
                Ok(true)
            );
            assert_eq!(
                (u.defend_target, u.current_target, u.current_order),
                (-1, -1, -1)
            );
            assert!(fx.contains(&Effect::Stop), "{fx:?}");
        }
        let gaia = UnitSeen { owner: 0, ..dead };
        let mut u = guarding(20);
        let mut side = DamageSide {
            units: vec![gaia],
            ..DamageSide::default()
        };
        assert_eq!(
            u.defend_misc(&at, false, &mut rng(), &mut side, &mut Vec::new()),
            Ok(false)
        );
        assert_eq!(u.defend_target, 20);
        // on the attack action only the unset point takes the ward's place
        let mut u = UnitAi {
            current_action: ATTACK_ACTION,
            target_point: [-1.0, -1.0, 3.0],
            ..guarding(20)
        };
        let mut side = DamageSide {
            units: vec![ward(1.0)],
            ..DamageSide::default()
        };
        let mut fx = Vec::new();
        assert_eq!(
            u.defend_misc(&at, false, &mut rng(), &mut side, &mut fx),
            Ok(false)
        );
        assert_eq!((u.target_point, fx), ([11.0, 10.0, 3.0], Vec::new()));
    }

    /// processMisc's defend order beyond the desired distance plus 1: the soldier walks to its
    /// ward (slot 0x138) or runs to the point it defends (slot 0x130, forced); within it, it
    /// attacks the ward's nearest attacker (slot 0xb8, forced), or with none its processIdle runs.
    #[test]
    fn a_defend_order_follows_its_ward_and_attacks_its_attackers() {
        let at = soldier_at();
        let mut u = guarding(20);
        let mut side = DamageSide {
            units: vec![ward(100.0)],
            ..DamageSide::default()
        };
        let mut fx = Vec::new();
        assert_eq!(
            u.defend_misc(&at, false, &mut rng(), &mut side, &mut fx),
            Ok(true)
        );
        assert_eq!(
            fx,
            vec![Effect::MoveToObject {
                target: 20,
                state: 2,
                range: 2.5
            }]
        );
        assert_eq!((u.current_action, u.current_target), (MOVE_ACTION, 20));
        // (2.5 + 1)^2 is 12.25: within it the ward's nearest attacker is attacked
        let mut u = guarding(20);
        let mut side = DamageSide {
            units: vec![ward(12.25), foe(40, 6)],
            defended: Some(40),
            ..DamageSide::default()
        };
        let mut fx = Vec::new();
        assert_eq!(
            u.defend_misc(&at, false, &mut rng(), &mut side, &mut fx),
            Ok(true)
        );
        assert_eq!(side.attacks, vec![40]);
        // a point to defend, beyond reach: the forced run to it
        let mut u = UnitAi {
            target_point: [20.0, 10.0, 0.0],
            ..guarding(-1)
        };
        let mut fx = Vec::new();
        assert_eq!(
            u.defend_misc(&at, false, &mut rng(), &mut DamageSide::default(), &mut fx),
            Ok(true)
        );
        assert_eq!(
            fx,
            vec![Effect::Move(AiMove {
                point: [20.0, 10.0, 0.0],
                range: 2.5,
                force: true,
                walk: false
            })]
        );
        // within reach of its point, nothing
        let mut u = UnitAi {
            target_point: [12.0, 10.0, 0.0],
            ..guarding(-1)
        };
        assert_eq!(
            u.defend_misc(
                &at,
                false,
                &mut rng(),
                &mut DamageSide::default(),
                &mut Vec::new()
            ),
            Ok(false)
        );
    }

    /// An enemy (player 2) of `class` the damage case's side holds.
    fn foe(id: i32, class: i16) -> UnitSeen {
        UnitSeen {
            id,
            class,
            owner: 2,
            state: 2,
            position: [12.0, 10.0, 0.0],
            hp: 30.0,
            kind: 70,
            ..UnitSeen::default()
        }
    }

    /// A human player's soldier at (10, 10) that moves.
    fn soldier_at() -> Surroundings {
        Surroundings {
            id: 1,
            player: 1,
            kind: 70,
            class: 6,
            speed: 1.0,
            los: 4.0,
            weapon_range: 0.0,
            action_state: Some(0),
            position: [10.0, 10.0, 0.0],
            ..Surroundings::default()
        }
    }

    fn damage(caller: i32, hp: i32) -> Notify {
        Notify {
            caller,
            recipient: 1,
            kind: DAMAGED,
            p: [0, hp, 0],
        }
    }

    /// `FUN_140c589c0`'s case 500 up to its attack search: a known attacker of a unit whose top
    /// action has a target changes nothing but the retarget entries; the dead give up; an allied
    /// attacker is let be once it joined the set; a human player's unit on an order other than
    /// a patrol ignores it.
    #[test]
    fn the_damage_case_lets_known_dead_allied_and_ordered_ones_be() {
        let at = soldier_at();
        let mut fx = Vec::new();
        let mut u = UnitAi {
            attackers: vec![7],
            retries: vec![(3, 900)],
            ..idle(13)
        };
        let mut side = DamageSide {
            units: vec![foe(7, 6)],
            top: Some(9),
            found: 7,
            ..DamageSide::default()
        };
        let r = u.damaged(damage(7, 20), 0, &at, false, &mut rng(), &mut side, &mut fx);
        assert_eq!(r.unwrap(), GO_ON);
        assert_eq!(u.retries, vec![]);
        assert_eq!(side.under_attack, Vec::<bool>::new());
        assert_eq!(side.attack_searches, 0);

        let mut u = idle(13);
        let mut side = DamageSide {
            units: vec![foe(7, 6)],
            ..DamageSide::default()
        };
        let r = u.damaged(damage(7, 0), 0, &at, false, &mut rng(), &mut side, &mut fx);
        assert_eq!(r.unwrap(), DEAD);
        assert_eq!(
            (u.attackers.clone(), side.under_attack),
            (vec![7], vec![true])
        );

        let mut u = idle(13);
        let mut side = DamageSide {
            units: vec![UnitSeen {
                allied: true,
                ..foe(7, 6)
            }],
            found: 7,
            ..DamageSide::default()
        };
        let r = u.damaged(damage(7, 20), 0, &at, false, &mut rng(), &mut side, &mut fx);
        assert_eq!(r.unwrap(), GO_ON);
        assert_eq!(u.attackers, vec![7]);
        assert_eq!((side.attack_searches, side.helps.len()), (0, 0));

        let mut u = UnitAi {
            current_order: ATTACK_ORDER,
            ..idle(13)
        };
        let mut side = DamageSide {
            units: vec![foe(7, 6)],
            found: 7,
            ..DamageSide::default()
        };
        let r = u.damaged(damage(7, 20), 0, &at, false, &mut rng(), &mut side, &mut fx);
        assert_eq!(r.unwrap(), GO_ON);
        // the new attacker made it ask for help first
        assert_eq!((side.attack_searches, side.helps.clone()), (0, vec![7]));
    }

    /// The unit AIs' class lists by factory type (their constructors' `+0x190`): the animals',
    /// the boar's and ranged units' empty ones, the priest's 30, the trade AI's by class, a
    /// computer player's soldier's with 10.
    #[test]
    fn each_unit_ai_type_searches_its_constructor_s_classes() {
        let list = |t: u32, class: i16, computer: bool| cared_classes(t, class, computer).unwrap();
        assert_eq!(list(2, 9, false), &HUNTED_CARED);
        assert_eq!(list(3, 10, false), &[58, 4]);
        let none: &[i32] = &[];
        assert_eq!(list(BOAR_AI, 10, false), none);
        assert_eq!((list(6, 0, false), list(7, 0, false)), (none, none));
        assert_eq!(list(5, 10, false), &LION_CARED);
        assert_eq!(list(PRIEST_AI, 18, false).len(), 30);
        assert_eq!(list(TRADE_AI, TRADE_BOAT_CLASS, false), &TRADE_SHIP_CARED);
        assert_eq!(list(TRADE_AI, 19, false), &TRADE_CART_CARED);
        assert_eq!(list(SOLDIER_AI, 6, true).last(), Some(&10));
        assert_eq!(list(SOLDIER_AI, 6, false), &SOLDIER_CARED);
        assert_eq!(list(LIVESTOCK_AI, 58, false), none);
        assert!(cared_classes(1, 58, false).is_err());
    }

    /// Slot 0xa8 by factory type: the civilian and the building never attack a neutral object,
    /// the base's types always; the predator and the lion spare their five classes, the priest
    /// its mask below 50 (61 passes), the tower the soldier's mask; attack priority 3 keeps only
    /// its classes.
    #[test]
    fn each_unit_ai_type_tests_a_neutral_object_by_its_module() {
        for t in [CIVILIAN, BUILDING_AI] {
            assert!(!type_attacks_neutral(t, 0, 6));
        }
        for t in [
            2,
            BOAR_AI,
            6,
            7,
            TRADE_AI,
            TRANSPORT_AI,
            FISHING_AI,
            ARTIFACT_AI,
        ] {
            assert!(type_attacks_neutral(t, 0, 4), "type {t}");
            assert!(type_attacks_neutral(t, 3, 6), "type {t}");
        }
        assert!(type_attacks_neutral(SIEGE_TOWER_AI, 0, 3));
        assert!(type_attacks_neutral(LIVESTOCK_AI, 0, 3));
        for t in [PREDATOR_AI, LION_AI] {
            for class in [13, 22, 52, 54, 55] {
                assert!(!type_attacks_neutral(t, 0, class), "type {t} class {class}");
            }
            assert!(type_attacks_neutral(t, 0, 4));
            assert!(!type_attacks_neutral(t, 3, 6));
        }
        for class in [2, 3, 4, 19, 20, 21, 27, 39, 49] {
            assert!(!type_attacks_neutral(PRIEST_AI, 0, class), "class {class}");
            assert!(!type_attacks_neutral(TOWER_AI, 0, class), "class {class}");
        }
        assert!(type_attacks_neutral(PRIEST_AI, 0, 61));
        assert!(!type_attacks_neutral(TOWER_AI, 0, 61));
        assert!(type_attacks_neutral(PRIEST_AI, 0, 6));
        assert!(!type_attacks_neutral(PRIEST_AI, 3, 6));
        assert_eq!(
            type_attacks_neutral(SIEGE_AI, 0, 19),
            siege_attacks_neutral(0, 19)
        );
    }

    /// An idle human player's soldier without a target: `bestUnitToAttack` names the attacker,
    /// which it reaches and attacks back (3); a second attacker in the same update takes the
    /// first search's answer and is ignored. Unreached, it runs (its stance keeps it: 3);
    /// nothing found and too slow to run, it lets it be.
    #[test]
    fn an_idle_soldier_attacks_back_what_best_unit_to_attack_names() {
        let at = soldier_at();
        let mut fx = Vec::new();
        let mut u = idle(13);
        let mut side = DamageSide {
            units: vec![foe(7, 6), foe(8, 6)],
            found: 7,
            reaches: true,
            ..DamageSide::default()
        };
        let r = u.damaged(damage(7, 20), 0, &at, false, &mut rng(), &mut side, &mut fx);
        assert_eq!(r.unwrap(), BREAK);
        assert_eq!(u.best_this_update, 7);
        assert_eq!(side.attacks, vec![7]);
        assert_eq!((u.current_action, u.current_target), (ATTACK_ACTION, 7));
        let r = u.damaged(damage(8, 20), 0, &at, false, &mut rng(), &mut side, &mut fx);
        assert_eq!(r.unwrap(), GO_ON);
        assert_eq!(side.attack_searches, 1);
        assert_eq!(side.helps, vec![7, 8]);
        assert_eq!(u.attackers, vec![7, 8]);

        let mut u = UnitAi {
            stance: 3,
            ..idle(13)
        };
        let mut side = DamageSide {
            units: vec![foe(7, 6)],
            found: 7,
            ..DamageSide::default()
        };
        let r = u.damaged(damage(7, 20), 0, &at, false, &mut rng(), &mut side, &mut fx);
        assert_eq!(r.unwrap(), BREAK);
        assert_eq!(side.attacks, Vec::<i32>::new());

        let still = Surroundings {
            speed: 0.0,
            ..soldier_at()
        };
        let mut u = idle(13);
        let mut side = DamageSide {
            units: vec![foe(7, 6)],
            found: -1,
            ..DamageSide::default()
        };
        let r = u.damaged(
            damage(7, 20),
            0,
            &still,
            false,
            &mut rng(),
            &mut side,
            &mut fx,
        );
        assert_eq!(r.unwrap(), GO_ON);
        assert_eq!(u.best_this_update, -1);
    }

    /// A human player's soldier with a live target attacks back only from a building, wall or
    /// tower target; on a patrol its patrol is saved and, the attack taken, the order ends with
    /// its timers (an adjusted idle timeout of 750 redrawn, the secondary timer capped).
    #[test]
    fn a_soldier_with_a_target_attacks_back_from_buildings_only() {
        let at = soldier_at();
        let run = |class: i16, order: i32| {
            let mut fx = Vec::new();
            let mut u = UnitAi {
                current_order: order,
                current_action: ATTACK_ACTION,
                current_target: 9,
                idle_timeout: 3000,
                adjusted_idle_timeout: 750,
                secondary_timer: 900,
                ..idle(13)
            };
            let mut side = DamageSide {
                units: vec![foe(7, 6), foe(9, class)],
                found: 7,
                reaches: true,
                patrol: Some((PatrolPath::default(), 2)),
                ..DamageSide::default()
            };
            let r = u
                .damaged(damage(7, 20), 0, &at, false, &mut rng(), &mut side, &mut fx)
                .unwrap();
            (r, u, side.attacks)
        };
        let (r, _, attacks) = run(6, -1);
        assert_eq!((r, attacks), (GO_ON, vec![]));
        for class in [3, 27, 52] {
            let (r, u, attacks) = run(class, -1);
            assert_eq!((r, attacks, u.current_target), (BREAK, vec![7], 7));
        }
        let (r, u, attacks) = run(3, PATROL);
        assert_eq!((r, attacks), (BREAK, vec![7]));
        assert_eq!(u.current_order, -1);
        assert_eq!(u.patrol_waypoint, 2);
        assert!(u.patrol_path.is_some());
        assert_eq!(u.secondary_timer, 750);
        assert_ne!(u.adjusted_idle_timeout, 750);
    }

    /// The computer player's end of the damage case: a wonder target, a tower against a tower,
    /// a missile, and a tower under `disableTowerPriority` (but for a siege weapon) are let be;
    /// a harmless target makes it ask for help and attack back, then the kill times decide again
    /// (the attacker's sooner, or its conversion, but never against a converting target).
    #[test]
    fn a_computer_s_unit_weighs_its_target_against_its_attacker() {
        let duel = |converts: bool, attacks: f32, harmed: f32, hp: f32| Duel {
            converts,
            attacks,
            harms: 2.0,
            harmed,
            hp,
        };
        let run = |at: &Surroundings, attacker: UnitSeen, target: UnitSeen, duels| {
            let mut fx = Vec::new();
            let mut u = UnitAi {
                current_action: ATTACK_ACTION,
                current_target: target.id,
                ..idle(13)
            };
            let mut side = DamageSide {
                units: vec![attacker, target],
                duels,
                ..DamageSide::default()
            };
            let r = u
                .computer_weighs(&attacker, &target, at, &mut rng(), &mut side, &mut fx)
                .unwrap();
            (r, side.attacks, side.helps)
        };
        let at = soldier_at();
        let fair = vec![
            (9, duel(false, 5.0, 10.0, 100.0)),
            (7, duel(false, 5.0, 10.0, 50.0)),
        ];
        let wonder = UnitSeen {
            master: crate::turn::WONDER,
            ..foe(9, 3)
        };
        assert_eq!(run(&at, foe(7, 6), wonder, fair.clone()).0, GO_ON);
        let tower = UnitSeen {
            class: 52,
            ..foe(9, 52)
        };
        assert_eq!(
            run(&at, foe(7, 52), tower, fair.clone()),
            (GO_ON, vec![], vec![])
        );
        let missile = UnitSeen {
            kind: 60,
            ..foe(7, 6)
        };
        assert_eq!(run(&at, missile, foe(9, 6), fair.clone()).1, vec![]);
        let shy = Surroundings {
            disable_tower_priority: true,
            ..soldier_at()
        };
        assert_eq!(run(&shy, foe(7, 52), foe(9, 6), fair.clone()).1, vec![]);
        let siege = Surroundings { class: 13, ..shy };
        // the attacker dies in 5 hits, the target in 10
        assert_eq!(run(&siege, foe(7, 52), foe(9, 6), fair.clone()).1, vec![7]);
        assert_eq!(run(&at, foe(7, 6), foe(9, 6), fair).1, vec![7]);
        let slow = vec![
            (9, duel(false, 5.0, 10.0, 100.0)),
            (7, duel(false, 5.0, 10.0, 200.0)),
        ];
        assert_eq!(run(&at, foe(7, 6), foe(9, 6), slow).1, vec![]);
        let converting = vec![
            (9, duel(false, 5.0, 10.0, 100.0)),
            (7, duel(true, 5.0, 10.0, 200.0)),
        ];
        assert_eq!(run(&at, foe(7, 6), foe(9, 6), converting).1, vec![7]);
        let harmless = vec![
            (9, duel(false, 0.0, 10.0, 100.0)),
            (7, duel(false, 5.0, 0.0, 50.0)),
        ];
        // the attack back, then the kill times: the attacker never dies (25000) against 10
        assert_eq!(
            run(&at, foe(7, 6), foe(9, 6), harmless),
            (GO_ON, vec![7], vec![7])
        );
        let priest = vec![
            (9, duel(true, 0.0, 10.0, 100.0)),
            (7, duel(true, 5.0, 10.0, 1.0)),
        ];
        assert_eq!(run(&at, foe(7, 6), foe(9, 6), priest).1, vec![]);
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
                patrol: None,
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

    /// The target block's attack whose target died on a patrol order: the order ends with its
    /// timers reset (an adjusted idle timeout of 750 redrawn, the secondary timer capped).
    #[test]
    fn a_patrol_s_kill_ends_the_patrol_order_with_its_timers() {
        let mut scout = UnitAi {
            current_order: PATROL,
            current_action: ATTACK_ACTION,
            current_target: 25954,
            target_point: [5.0, 6.0, 0.0],
            idle_timeout: 3000,
            adjusted_idle_timeout: 750,
            secondary_timer: 900,
            ..idle(13)
        };
        let at = Surroundings {
            player: 1,
            kind: 70,
            action_state: Some(0),
            ..Surroundings::default()
        };
        let mut side = SoldierSide {
            found: -1,
            attacks: Vec::new(),
            patrol: None,
        };
        scout
            .next_after_kill(&at, false, &mut rng(), &mut side, &mut Vec::new())
            .unwrap();
        assert_eq!(
            (scout.current_order, scout.last_order, scout.secondary_timer),
            (-1, PATROL, 750)
        );
        assert_ne!(scout.adjusted_idle_timeout, 750);
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
            scout(2).retarget_outcome(sheep, 0.0, false, Some(4), Some(true)),
            Ok(None)
        );
        assert_eq!(
            scout(0).retarget_outcome(sheep, 0.0, false, Some(6), Some(true)),
            Ok(None)
        );
        assert_eq!(
            scout(0).retarget_outcome(sheep, 0.0, false, Some(4), Some(true)),
            Ok(Some(RetargetSearch {
                building: false,
                work_state: false
            }))
        );
        // a building target is searched past even while attacking
        assert_eq!(
            scout(0).retarget_outcome(Some((1, 3)), 0.0, true, Some(6), Some(true)),
            Ok(Some(RetargetSearch {
                building: true,
                work_state: true
            }))
        );
        // without an Attack in the action list (slot 0x538 with 9) there is no search: a king
        // entering a castle, its target a building (records 426406723's type-2 unit AIs)
        let entering = UnitAi {
            current_order: crate::garrison::ENTER_ORDER,
            current_action: crate::garrison::ENTER_ACTION,
            current_target: 3301,
            stance: 0,
            ..idle(2)
        };
        assert_eq!(
            entering.retarget_outcome(Some((8, 3)), 0.0, false, Some(4), Some(false)),
            Ok(None)
        );
        // an action list the slice does not hold stops at the state's test
        assert!(
            entering
                .retarget_outcome(Some((8, 3)), 0.0, false, None, None)
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
        // on the attack order, its target gone (the target block forgets it), the civilian's
        // processMisc stops the action and ends the order (slot 0xb0 with 0)
        u.current_order = 700;
        let fx = u.update(50, 0, false, &at, &mut r, &mut no_search).unwrap();
        assert_eq!(fx, [Effect::Stop]);
        assert_eq!(
            (u.current_order, u.last_order, u.current_action),
            (-1, 700, -1)
        );
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
            u.retarget_check(0, false, &at, &mut r, &mut finds_nothing(), &mut fx)
                .is_ok()
        );
        assert_eq!(fx, Vec::<Effect>::new());
        // 5.1 tiles off its footprint: it stops, forgets its target and walks back to its state
        // position (FUN_140c5e7d0), then goes on to the retarget check (its timer not used up)
        let (mut u, at) = defender(15.6);
        assert_eq!(
            u.retarget_check(0, false, &at, &mut r, &mut finds_nothing(), &mut fx),
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
        let ai = UnitAi::constructed(17, &placed([30.0, 88.0, 0.0], 3), &mut r).unwrap();
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
                defend_target: -1,
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
                patrol_path: None,
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
                order_history: Vec::new(),
                retries: Vec::new(),
                best_this_update: -1,
            }
        );
    }

    /// The artifact's unit AI (14, `TribeArtifactUnitAIModule` `FUN_140c11340`, the rock church's):
    /// the base's three draws, then `FUN_140c41bc0(5000)`: its idle timeout 5000 and a fourth
    /// draw for its adjusted one (`r % 2500 + 3750`), stance 0, five notify slots. The recording
    /// 426048706 holds rock churches of base 5000 with adjusted ones of 3881 and 6012.
    #[test]
    fn a_new_artifact_unit_ai_draws_four_timers() {
        let mut r = rng();
        let ai = UnitAi::constructed(ARTIFACT_AI, &placed([136.0, 100.0, 0.0], 1), &mut r).unwrap();
        let mut expected = rng();
        let (a, _, c, d) = (
            expected.rand15(),
            expected.rand15(),
            expected.rand15(),
            expected.rand15(),
        );
        assert_eq!(r.draws, 4);
        assert_eq!(
            (
                ai.ai_type,
                ai.last_retarget_time,
                ai.retarget_timer,
                ai.idle_timeout,
                ai.adjusted_idle_timeout,
                ai.stance,
                ai.notify_capacity,
            ),
            (
                ARTIFACT_AI,
                a % 500,
                750 + c % 500,
                5000,
                d % 2500 + 3750,
                0,
                5
            )
        );
    }

    /// What the artifact's tests answer: the search's answer and the owner of the object found,
    /// with the searches asked (their radius, classes and reach test) and the unit's
    /// under-attack value.
    struct ArtifactSide {
        found: Found,
        owner: Option<usize>,
        searches: Vec<(f32, Vec<i32>, bool)>,
        under_attack: bool,
    }

    impl ArtifactSide {
        fn finding(id: i32, players: u32, owner: Option<usize>) -> Self {
            Self {
                found: Found {
                    id,
                    players,
                    distance: -1,
                    position: Some([140.0, 100.0, 0.0]),
                    ..Found::NONE
                },
                owner,
                searches: Vec::new(),
                under_attack: false,
            }
        }
    }

    impl UnitSide for ArtifactSide {
        fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
            None
        }
        fn closest(&mut self, search: &Search<'_>) -> Result<Found, GenieError> {
            let reach = match search.pick {
                Pick::Closest { reach, .. } => reach,
                _ => true,
            };
            self.searches
                .push((search.radius, search.classes.to_vec(), reach));
            Ok(self.found)
        }
        fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
            Ok(Attacked {
                attacks: false,
                posted: Vec::new(),
            })
        }
        fn set_under_attack(&mut self, value: bool) {
            self.under_attack = value;
        }
        fn stop(&mut self) -> Result<(), GenieError> {
            Ok(())
        }
        fn may_attack(
            &mut self,
            _: i32,
            _: &Seeker,
        ) -> Result<Option<(i16, [f32; 3])>, GenieError> {
            Ok(None)
        }
        fn seek_unexplored(
            &mut self,
            _: &UnitAi,
            _: &mut Well512,
        ) -> Result<Option<[i32; 2]>, GenieError> {
            Ok(None)
        }
        fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
            Ok(None)
        }
        fn make_scout_map(&mut self) -> Result<(), GenieError> {
            Ok(())
        }
        fn seen_unit(&mut self, id: i32) -> Result<Option<UnitSeen>, GenieError> {
            Ok(self.owner.map(|owner| UnitSeen {
                id,
                owner,
                ..UnitSeen::default()
            }))
        }
    }

    /// A Gaia rock church (MarvinBlue/426048706's 7108: line of sight 5, no speed) and what it
    /// stands on.
    fn rock_church() -> (UnitAi, Surroundings) {
        let ai = UnitAi {
            ai_type: ARTIFACT_AI,
            current_order: -1,
            current_action: -1,
            current_target: -1,
            idle_timeout: 5000,
            adjusted_idle_timeout: 3881,
            retarget_timer: 1224,
            ..UnitAi::default()
        };
        let at = Surroundings {
            position: [136.0, 100.0, 0.0],
            size: [2.0, 2.0],
            los: 5.0,
            search_radius: 5.0,
            player: 0,
            state: 2,
            class: 1,
            kind: 80,
            ..Surroundings::default()
        };
        (ai, at)
    }

    /// The artifact's processIdle (`FUN_140c1e410`): `ClosestObject` over its cared classes from
    /// its place within its line of sight and a tile, without the reach test for a unit that
    /// does not move; the object found takes it over for its owner (slot 0x70, 6) while the
    /// lists hold one player's objects at most; two players, none found, the world without it
    /// or its own owner's leave it (5).
    #[test]
    fn the_artifact_goes_to_the_owner_of_the_closest_object_one_player_sees() {
        let (mut ai, at) = rock_church();
        let mut side = ArtifactSide::finding(500, 1, Some(2));
        let mut fx = Vec::new();
        let took = ai.process_idle(&at, false, &mut rng(), &mut side, &mut fx);
        assert_eq!(took, Ok(true));
        assert_eq!(fx, [Effect::ChangeOwner(2)]);
        assert_eq!(side.searches, [(6.0, ARTIFACT_CARED.to_vec(), false)]);
        for (players, id, owner) in [
            (2, 500, Some(2)),
            (1, -1, Some(2)),
            (1, 500, None),
            (0, 500, Some(0)),
        ] {
            let mut side = ArtifactSide::finding(id, players, owner);
            let mut fx = Vec::new();
            let took = ai.process_idle(&at, false, &mut rng(), &mut side, &mut fx);
            assert_eq!((took, fx), (Ok(false), vec![]));
        }
        // a moving one asks for the reach test
        let walking = Surroundings { speed: 0.8, ..at };
        let mut side = ArtifactSide::finding(-1, 0, None);
        let _ = ai.process_idle(&walking, false, &mut rng(), &mut side, &mut Vec::new());
        assert!(side.searches[0].2);
    }

    /// The artifact's slot 73 (`FUN_140c21d40`) in its full update: its processIdle every
    /// update; a takeover forgets its target, stops its action (slot 0xb0 with 1) after the
    /// ownership change, and its idle timer is 0.
    #[test]
    fn the_artifact_s_update_hands_it_over_then_stops() {
        let (mut ai, at) = rock_church();
        ai.idle_timer = 900;
        let mut side = ArtifactSide::finding(500, 1, Some(3));
        let fx = ai
            .update(100, 5000, false, &at, &mut rng(), &mut side)
            .unwrap();
        assert_eq!(fx, [Effect::ChangeOwner(3), Effect::Stop]);
        assert_eq!((ai.current_target, ai.idle_timer), (-1, 0));
        // nobody near: nothing, the idle timer runs
        let (mut ai, at) = rock_church();
        let mut side = ArtifactSide::finding(-1, 0, None);
        let fx = ai
            .update(100, 5000, false, &at, &mut rng(), &mut side)
            .unwrap();
        assert_eq!(fx, []);
        assert_eq!(ai.idle_timer, 100);
    }

    /// The artifact's notify handler (`FUN_140c21f70`): the kinds it lets be go on; a unit
    /// attacking it joins its attackers and puts it under attack; its own action's completion or
    /// failure (with an order, the order's action) and any kind it does not know end in its
    /// stop; a moved target it still sees goes on, one it does not see ends.
    #[test]
    fn the_artifact_s_notifies_stop_it_or_go_on() {
        let run = |ai: &mut UnitAi, kind: i32, p: [i32; 3], at: &Surroundings| {
            let n = Notify {
                caller: 500,
                recipient: 7108,
                kind,
                p,
            };
            let mut side = ArtifactSide::finding(-1, 0, None);
            let mut fx = Vec::new();
            let r = ai.handle(n, 0, at, false, &mut rng(), &mut side, &mut fx);
            (r, fx, side.under_attack)
        };
        let (mut ai, at) = rock_church();
        for kind in [
            500,
            0x1f7,
            0x1ff,
            0x202,
            OWNER_CHANGED,
            0x25d,
            0x268,
            699,
            0x2be,
        ] {
            assert_eq!(run(&mut ai, kind, [0; 3], &at), (Ok(GO_ON), vec![], false));
        }
        assert_eq!(run(&mut ai, 600, [0; 3], &at), (Ok(GO_ON), vec![], true));
        assert_eq!(ai.attackers, [500]);
        ai.current_action = 0x262;
        ai.current_target = 9;
        assert_eq!(
            run(&mut ai, ACTION_COMPLETED, [0x261, 0, 0], &at),
            (Ok(GO_ON), vec![], false)
        );
        assert_eq!(
            run(&mut ai, ACTION_FAILED, [0x262, 0, 0], &at),
            (Ok(GO_ON), vec![Effect::Stop], false)
        );
        assert_eq!((ai.current_action, ai.current_target), (-1, -1));
        // with an order: its action's (action + 100 = order)
        ai.current_order = 0x2c6;
        ai.current_action = 0x262;
        assert_eq!(
            run(&mut ai, ACTION_COMPLETED, [0x1, 0, 0], &at),
            (Ok(GO_ON), vec![Effect::Stop], false)
        );
        let unknown = run(&mut ai, 0x210, [0; 3], &at);
        assert_eq!(unknown, (Ok(GO_ON), vec![Effect::Stop], false));
        // a moved target it sees goes on; one it does not see stops it
        ai.current_target = 9;
        let seen = Surroundings {
            target: Some(Seen {
                sight: 0xf,
                ..Seen::default()
            }),
            ..at.clone()
        };
        assert_eq!(
            run(&mut ai, TARGET_MOVED, [0; 3], &seen),
            (Ok(GO_ON), vec![], false)
        );
        let explored = Surroundings {
            target: Some(Seen {
                sight: 0x80,
                ..Seen::default()
            }),
            ..at
        };
        assert_eq!(
            run(&mut ai, TARGET_MOVED, [0; 3], &explored),
            (Ok(GO_ON), vec![Effect::Stop], false)
        );
    }

    /// The tower's unit AI (15, `TribeTowerUnitAIModule` `FUN_140c12960`, a town centre's with
    /// its volley missile): the base's three draws, then `FUN_140c41bc0(1000)`: its idle timeout
    /// 1000 and a fourth draw for its adjusted idle timeout (`r % 500 + 750`). records/428759682
    /// update 2019: the town centre 8768 a 621's completion makes holds base 1000, adjusted 818.
    #[test]
    fn a_new_tower_unit_ai_draws_four_timers() {
        let mut r = rng();
        let ai = UnitAi::constructed(TOWER_AI, &placed([54.0, 147.0, 0.0], 52), &mut r).unwrap();
        let mut expected = rng();
        let (a, _, c, d) = (
            expected.rand15(),
            expected.rand15(),
            expected.rand15(),
            expected.rand15(),
        );
        assert_eq!(r.draws, 4);
        assert_eq!(
            (
                ai.ai_type,
                ai.last_retarget_time,
                ai.retarget_timer,
                ai.idle_timeout,
                ai.adjusted_idle_timeout,
                ai.stance,
            ),
            (TOWER_AI, a % 500, 750 + c % 500, 1000, d % 500 + 750, 0)
        );
    }

    /// A new object of class `class` at `position`, of master 83 (its own copy), a human
    /// player's in the Dark Age with posture 0.
    fn placed(position: [f32; 3], class: i16) -> NewObject {
        NewObject {
            position,
            master: 83,
            copy_id: 83,
            class,
            ..NewObject::default()
        }
    }

    /// The base's three draws (`0x140c2e080`) and, for the types whose constructor sets its own
    /// idle timeout (`FUN_140c41bc0`), slot 0x1a8's fourth (`0x140c41e00`: `r % (t / 2) + 3 t /
    /// 4`); the hunted animal's slot 0x1a8 (`FUN_140c17170`) is 5/4 of its timeout without a
    /// draw. Each type's stance (`+0x1a9`) and notify capacity (5 for 0xe and 0x13).
    #[test]
    fn each_unit_ai_constructor_draws_its_timers_and_sets_its_stance() {
        // (type, class, idle timeout, fourth draw, stance, notify capacity)
        let types: [(u32, i16, u32, bool, u8, usize); 17] = [
            (2, 9, 4000, false, 3, 10),
            (3, 10, 3000, false, 0, 10),
            (BOAR_AI, 10, 3000, false, 0, 10),
            (5, 10, 6000, true, 0, 10),
            (6, 0, 3000, false, 3, 10),
            (7, 0, 3000, false, 3, 10),
            (CIVILIAN, 4, 3000, false, 0, 10),
            (9, 18, 500, true, 1, 10),
            (0xa, 2, 3000, false, 0, 10),
            (0xb, 20, 3000, false, 0, 10),
            (0xc, 21, 3000, false, 0, 10),
            (SOLDIER_AI, 6, 500, true, 0, 10),
            (ARTIFACT_AI, 1, 5000, true, 0, 5),
            (TOWER_AI, 52, 1000, true, 0, 10),
            (0x10, 13, 1000, true, 1, 10),
            (17, 3, 3000, false, 0, 10),
            (0x12, 6, 3000, false, 0, 10),
        ];
        for (ai_type, class, timeout, fourth, stance, capacity) in types {
            let mut r = rng();
            let ai =
                UnitAi::constructed(ai_type, &placed([12.5, 40.0, 1.0], class), &mut r).unwrap();
            let mut expected = rng();
            let (a, b, c) = (expected.rand15(), expected.rand15(), expected.rand15());
            let adjusted = if fourth {
                expected.rand15() % (timeout / 2) + timeout * 3 / 4
            } else if ai_type == 2 {
                timeout * 5 / 4
            } else {
                b % 1500 + 2250
            };
            assert_eq!(r.draws, if fourth { 4 } else { 3 }, "type {ai_type}");
            assert_eq!(
                (
                    ai.ai_type,
                    ai.last_retarget_time,
                    ai.retarget_timer,
                    ai.idle_timeout,
                    ai.adjusted_idle_timeout,
                    ai.stance,
                    ai.notify_capacity,
                    ai.state_position,
                ),
                (
                    ai_type,
                    a % 500,
                    750 + c % 500,
                    timeout,
                    adjusted,
                    stance,
                    capacity,
                    [12.5, 40.0],
                ),
                "type {ai_type}"
            );
        }
        let mut r = rng();
        let livestock = UnitAi::constructed(0x13, &placed([1.0; 3], 58), &mut r).unwrap();
        assert_eq!(
            (r.draws, livestock.stance, livestock.notify_capacity),
            (3, 1, 5)
        );
    }

    /// `FUN_141215560` has no constructor for the types outside 2..=0x13: such an object gets
    /// no unit AI and nothing is drawn.
    #[test]
    fn a_unit_ai_type_without_a_constructor_makes_none_and_draws_nothing() {
        for ai_type in [0, 1, 0x14, 99] {
            let mut r = rng();
            assert_eq!(
                UnitAi::constructed(ai_type, &placed([0.0; 3], 6), &mut r),
                None
            );
            assert_eq!(r.draws, 0);
        }
    }

    /// The hunted animal's constructor (`FUN_140c11920`): its origin is the object's place, its
    /// leash the object's slot 0x7b0 (`+0xb74`); the king (class 59) takes stance 0, a deer
    /// keeps the base's 3. The boar's (`FUN_140c11720`) origin is its place too, neither
    /// furious nor sent home.
    #[test]
    fn a_new_hunted_animal_and_boar_keep_their_place_as_their_origin() {
        let mut deer = placed([20.0, 30.0, 2.0], 9);
        deer.hunt_range = 7.5;
        let mut r = rng();
        let ai = UnitAi::constructed(2, &deer, &mut r).unwrap();
        assert_eq!(
            (ai.hunted, ai.stance, ai.home),
            (
                Some(Hunted {
                    origin: [20.0, 30.0, 2.0],
                    leash: 7.5,
                }),
                3,
                None
            )
        );
        let king = UnitAi::constructed(2, &placed([0.0; 3], CLASS_KING), &mut rng()).unwrap();
        assert_eq!(king.stance, 0);
        let boar = UnitAi::constructed(BOAR_AI, &placed([5.0, 6.0, 0.5], 10), &mut rng()).unwrap();
        assert_eq!(
            (boar.home, boar.hunted, boar.stance),
            (
                Some(Home {
                    origin: [5.0, 6.0, 0.5],
                    furious: false,
                    go_home: false,
                }),
                None,
                0
            )
        );
    }

    /// The soldier's constructor (`FUN_140c12300`): a computer player's idle timeout is 1500
    /// (the fourth draw `r % 750 + 1125`); its stance by the rules of [`soldier_stance`].
    #[test]
    fn a_new_soldier_takes_its_stance_from_its_master_and_owner() {
        let mut computer = placed([0.0; 3], 6);
        computer.computer = true;
        computer.posture = 1;
        let mut r = rng();
        let ai = UnitAi::constructed(SOLDIER_AI, &computer, &mut r).unwrap();
        let mut expected = rng();
        let _ = (expected.rand15(), expected.rand15(), expected.rand15());
        assert_eq!(
            (ai.idle_timeout, ai.adjusted_idle_timeout, ai.stance),
            (1500, expected.rand15() % 750 + 1125, 1)
        );
        let stance = |f: &dyn Fn(&mut NewObject)| {
            let mut new = placed([0.0; 3], 6);
            f(&mut new);
            soldier_stance(&new)
        };
        // the owner's posture, its low byte
        assert_eq!(stance(&|_| {}), 0);
        assert_eq!(stance(&|n| n.posture = 0x101), 1);
        // the scout of the Dark Age stands its ground: flag 0x10, its own copy, age 0 (or -0)
        assert_eq!(stance(&|n| n.object_flags = 0x10), 2);
        assert_eq!(
            stance(&|n| {
                n.object_flags = 0x10;
                n.age = -0.0;
            }),
            2
        );
        assert_eq!(
            stance(&|n| {
                n.object_flags = 0x10;
                n.age = 1.0;
            }),
            0
        );
        assert_eq!(
            stance(&|n| {
                n.object_flags = 0x10;
                n.age = f32::NAN;
            }),
            0
        );
        assert_eq!(
            stance(&|n| {
                n.object_flags = 0x10;
                n.copy_id = 448;
            }),
            0
        );
        assert_eq!(
            stance(&|n| {
                n.object_flags = 0x10;
                n.computer = true;
            }),
            0
        );
        // the demolition ships defend
        for master in DEFENSIVE_MASTERS {
            assert_eq!(
                stance(&|n| {
                    n.master = master;
                    n.copy_id = master;
                }),
                1
            );
        }
        assert_eq!(
            stance(&|n| {
                n.master = 527;
                n.computer = true;
            }),
            0
        );
        // a human player's petards and class 64 do not attack; a computer's take its posture
        assert_eq!(stance(&|n| n.class = 35), 3);
        assert_eq!(stance(&|n| n.class = 64), 3);
        assert_eq!(
            stance(&|n| {
                n.class = 35;
                n.computer = true;
            }),
            0
        );
        // a hero defends: bit 0 or 3, not 3 with 7
        assert_eq!(stance(&|n| n.hero_flag = 1), 1);
        assert_eq!(stance(&|n| n.hero_flag = 8), 1);
        assert_eq!(stance(&|n| n.hero_flag = 0x81), 1);
        assert_eq!(stance(&|n| n.hero_flag = 0x88), 0);
        assert_eq!(stance(&|n| n.hero_flag = 0x10), 0);
        // class 61 does not attack, unless a hero
        assert_eq!(stance(&|n| n.class = 61), 3);
        assert_eq!(
            stance(&|n| {
                n.class = 61;
                n.hero_flag = 1;
            }),
            1
        );
        assert_eq!(
            stance(&|n| {
                n.class = 61;
                n.computer = true;
            }),
            3
        );
    }

    /// The siege unit's stance (`FUN_140c12040`): a computer player's and the trebuchets' (51,
    /// 54) 0, the scorpion's (55) the owner's posture, any other's 1.
    #[test]
    fn a_new_siege_unit_takes_its_stance_from_its_class_and_owner() {
        let stance = |class: i16, computer: bool, posture: i32| {
            siege_stance(&NewObject {
                class,
                computer,
                posture,
                ..NewObject::default()
            })
        };
        assert_eq!(stance(13, false, 0), 1);
        assert_eq!(stance(13, true, 0), 0);
        assert_eq!(stance(51, false, 1), 0);
        assert_eq!(stance(54, false, 1), 0);
        assert_eq!(stance(55, false, 1), 1);
        assert_eq!(stance(55, false, 0), 0);
        assert_eq!(stance(55, true, 1), 0);
        assert_eq!(stance(6, false, 0), 1);
    }

    /// [`NewObject::of`]: the master's copy, class, flags and hero flag, and the work range of
    /// its last task of action type 6 (0 without one).
    #[test]
    fn a_new_object_takes_its_hunt_range_from_its_master_s_last_task_of_type_6() {
        let task = |action_type, work_range| crate::masters::Task {
            action_type,
            work_range,
            ..crate::masters::Task::default()
        };
        let made = crate::masters::Master {
            copy_id: 1240,
            class: 9,
            object_flags: 0x30,
            hero_flag: 4,
            tasks: vec![task(6, 2.0), task(7, 9.0), task(6, 3.5), task(5, 1.0)],
            ..crate::masters::Master::default()
        };
        let new = NewObject::of([1.0, 2.0, 3.0], 1239, &made);
        assert_eq!(
            new,
            NewObject {
                position: [1.0, 2.0, 3.0],
                master: 1239,
                copy_id: 1240,
                class: 9,
                object_flags: 0x30,
                hero_flag: 4,
                hunt_range: 3.5,
                ..NewObject::default()
            }
        );
        let none = NewObject::of([0.0; 3], 1, &crate::masters::Master::default());
        assert_eq!(none.hunt_range, 0.0);
    }

    #[test]
    fn an_order_redraws_an_adjusted_idle_timeout_of_750_or_more_and_caps_the_secondary_timer() {
        // FUN_140e6cfc0 for each unit of an INTERACT, MOVE or RETREAT
        let mut u = UnitAi {
            idle_timeout: 3000,
            adjusted_idle_timeout: 2600,
            secondary_timer: 900,
            patrol_path: Some(PatrolPath::default()),
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
        assert_eq!((u.patrol_path, u.patrol_waypoint), (None, 0));
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
                players: 0,
                distance: -1,
                retries: [None; 3],
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
        let fx = u
            .off_turn(&scout_at(true, true), &mut no_search, false, &mut rng())
            .unwrap();
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
        assert_eq!(
            walking.off_turn(&scout_at(false, true), &mut no_search, false, &mut rng()),
            Ok(vec![])
        );
        assert_eq!(walking.orders.len(), 2);
        let mut told = UnitAi {
            notifies: vec![Notify::own(9183, ACTION_COMPLETED, [MOVE_ACTION, 0, 0])],
            ..scout()
        };
        assert_eq!(
            told.off_turn(&scout_at(true, true), &mut no_search, false, &mut rng()),
            Ok(vec![])
        );
        assert_eq!(told.orders.len(), 2);
        // an order not ready stays and the next one runs
        let mut u = scout();
        u.orders[0].ready = false;
        let _ = u
            .off_turn(&scout_at(true, true), &mut no_search, false, &mut rng())
            .unwrap();
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
                at.group,
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
        assert_eq!(
            run(&mut u, &at),
            Err(GenieError::unsupported("no search here"))
        );
        assert_eq!(u.idle_timer, 0);
        // a building without a missile and a single volley forgets its target, then searches
        let single = Surroundings {
            ranged: false,
            firepower: 1.0,
            ..at
        };
        let mut u = tc(29826);
        assert_eq!(
            run(&mut u, &single),
            Err(GenieError::unsupported("no search here"))
        );
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

    /// The full update with a target on the orders and action values the recordings met (it
    /// reads the target generally, whatever the order): a town centre's make action on the
    /// attack order (6091, p6's 27171), a wolf's own attack, a building a MOVE took (no action
    /// value, its research's 620), a builder and a hunter sent off a footprint, a hunt without
    /// an order after `HuntObject`, a villager waiting for its foundation (734/634 with the
    /// foundation), a boar moved aside off its attacker (-1/703).
    #[test]
    fn a_unit_ai_with_a_target_runs_its_update_on_any_order() {
        let at = Surroundings {
            busy: true,
            player: 1,
            state: 2,
            target: Some(Seen {
                state: 2,
                kind: 70,
                owner: 2,
                class: 4,
                ..Seen::default()
            }),
            ..Surroundings::default()
        };
        for (ai_type, order, action) in [
            (15, ATTACK_ORDER, crate::spawn::MAKE_ACTION),
            (5, -1, ATTACK_ACTION),
            (17, MOVE_ORDER, -1),
            (17, MOVE_ORDER, 620),
            (CIVILIAN, BUILD_ORDER, MOVE_ASIDE_ACTION),
            (CIVILIAN, HUNT_ORDER, MOVE_ASIDE_ACTION),
            (CIVILIAN, -1, HUNT_ACTION),
            (CIVILIAN, FOUNDATION_ORDER, WAIT_SOLIDIFY_ACTION),
            (BOAR_AI, -1, MOVE_ASIDE_ACTION),
        ] {
            let mut u = UnitAi {
                ai_type,
                current_order: order,
                current_action: action,
                current_target: 14230,
                adjusted_idle_timeout: 9000,
                look_around_timeout: 9000,
                retarget_timer: 90_000,
                home: (ai_type == BOAR_AI).then(Home::default),
                ..UnitAi::default()
            };
            let r = u.update(10, 100, false, &at, &mut rng(), &mut TailSide::default());
            assert_eq!(r, Ok(Vec::new()), "{ai_type} {order} {action}");
            assert_eq!(u.current_target, 14230, "{ai_type} {order} {action}");
        }
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
                state: 2,
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

    /// The base handler's case 0x1fc (a `MoveTo`'s moved target): nothing without a saved
    /// patrol; with one processIdle runs (slot 0x240 with 1; this Gaia soldier only caps).
    #[test]
    fn a_moved_target_notify_runs_process_idle_with_a_saved_patrol() {
        let (mut u, at) = follower([115.0, 40.2, 0.0], true);
        let n = Notify::own(9183, TARGET_MOVED, [0, 0, 0]);
        let (mut r, mut side) = (rng(), Recorder::default());
        let mut fx = Vec::new();
        assert_eq!(
            u.base_notify(n, 0, &at, false, &mut r, &mut side, &mut fx),
            Ok(GO_ON)
        );
        assert_eq!(fx, vec![]);
        // off its follow order (whose last human order time stays) the idle check caps it
        u.patrol_path = Some(PatrolPath::default());
        u.current_order = -1;
        assert_eq!(
            u.base_notify(n, 0, &at, false, &mut r, &mut side, &mut fx),
            Ok(GO_ON)
        );
        assert_eq!(fx, vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)]);
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
                players: 0,
                distance: -1,
                retries: [None; 3],
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
        capture: 0,
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
        target_kind: None,
    };

    /// Slot 0xa0 by unit-AI type: the base's always, the building's never, the hunted
    /// animal's only with an Attack (or a type 7) in its list, the predator's and lion's by
    /// class under attack priority 0 (anything under 1 and 2), the civilian's but classes 1 and
    /// 42; the priest's is not modelled.
    #[test]
    fn each_unit_ai_type_has_its_own_attack_check() {
        let check = |ai_type: u32, top_action: Option<i32>, priority: u8, class: i16| {
            let me = Attacker {
                ai_type,
                top_action,
                attack_priority: priority,
                ..TC
            };
            can_attack(&me, &IDLE, &Target { class, ..BOAR })
        };
        for t in [
            BOAR_AI,
            6,
            7,
            TRADE_AI,
            TRANSPORT_AI,
            FISHING_AI,
            ARTIFACT_AI,
            0x12,
            0x13,
        ] {
            assert_eq!(check(t, None, 0, 42), Ok(true), "{t}");
        }
        assert_eq!(check(BUILDING_AI, None, 0, 4), Ok(false));
        assert_eq!(check(2, None, 0, 4), Ok(false));
        assert_eq!(check(2, Some(9), 0, 4), Ok(true));
        assert_eq!(check(2, Some(7), 0, 4), Ok(true));
        // the predator's list and the lion's: 4 (villagers) yes, 10 no
        assert_eq!(
            (check(3, None, 0, 4), check(3, None, 0, 10)),
            (Ok(true), Ok(false))
        );
        assert_eq!(check(3, None, 2, 10), Ok(true));
        assert_eq!(
            (check(5, None, 0, 9), check(3, None, 0, 9)),
            (Ok(true), Ok(false))
        );
        assert_eq!(check(CIVILIAN, None, 0, 42), Ok(false));
        assert_eq!(check(CIVILIAN, None, 0, 10), Ok(true));
        assert!(check(PRIEST_AI, None, 0, 4).is_err());
    }

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
        // a type whose attack check is not modelled (the priest's conversion test)
        assert!(
            can_attack(
                &Attacker {
                    ai_type: PRIEST_AI,
                    ..TC
                },
                &IDLE,
                &BOAR
            )
            .is_err()
        );
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
        u.order_now(
            order,
            Some(9),
            Some(([5.0, 6.0], 4)),
            None,
            &mut rng(),
            &mut fx,
        )
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
    /// and slot 0x230 runs the order in front: its roundup (slot 0xc0) attacks the new target,
    /// and the tail makes the order 700. Outside the update (no surroundings) the case does not
    /// run.
    #[test]
    fn an_immediate_attack_roundup_keeps_the_target_until_it_attacks_the_next() {
        let attacking = || UnitAi {
            current_order: ATTACK_ORDER,
            current_action: ATTACK_ACTION,
            current_target: 50,
            notifies: vec![Notify::own(9, DAMAGED, [1, 5, 0])],
            ..idle(13)
        };
        let roundup = QueuedOrder {
            order: crate::ai_kinds::ORDER_ATTACK_ROUNDUP,
            priority: 100,
            target: 51,
            ready: true,
            ..QueuedOrder::default()
        };
        let mut u = attacking();
        let mut fx = Vec::new();
        let r = u.take_order_now(2, roundup, ([0.0; 2], 6), &mut rng(), &mut fx);
        assert!(r.is_err());
        assert_eq!(fx, Vec::new());
        assert_eq!((u.current_target, u.current_action), (50, ATTACK_ACTION));
        assert_eq!(u.notifies, Vec::new());
        let mut u = attacking();
        let mut side = OrderSide::holding(&[(51, 6)]);
        side.reach = true;
        let at = Surroundings {
            class: 6,
            ..Surroundings::default()
        };
        let mut fx = Vec::new();
        u.order_now(
            roundup,
            None,
            None,
            Some((&at, &mut side, false)),
            &mut rng(),
            &mut fx,
        )
        .unwrap();
        assert_eq!(fx, Vec::new());
        assert_eq!(side.attacks, [(51, true)]);
        assert_eq!(
            (
                u.current_target,
                u.last_target,
                u.current_action,
                u.last_action
            ),
            (51, 50, ATTACK_ACTION, ATTACK_ACTION)
        );
        assert_eq!((u.current_order, u.order_priority), (ATTACK_ORDER, 100));
        assert_eq!((u.notifies.len(), u.orders.len()), (0, 0));
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
        // a patrolling group: the patrol branch, its secondary timer run out, restarts it at 750
        // and runs processIdle (the ship's caps its last human order time)
        let mut u = idle(11);
        let mut g = group(false, true, false);
        g.as_mut().unwrap().patrol = true;
        assert_eq!(
            run(&mut u, &at(g)),
            Ok(vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)])
        );
        assert_eq!(u.secondary_timer, 750);
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
            sight: 0xf,
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
            Err(GenieError::unsupported(
                "the patrol range of a unit AI's target"
            ))
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
                players: 0,
                distance: -1,
                retries: [None; 3],
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
            patrol: None,
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
    ) -> (Result<u32, GenieError>, Vec<Effect>, Recorder) {
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
                players: 0,
                distance: -1,
                retries: [None; 3],
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
                players: 0,
                distance: -1,
                retries: [None; 3],
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
        fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
            None
        }
        fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
            Err(GenieError::unsupported("no attack here"))
        }
        fn set_under_attack(&mut self, _: bool) {}
        fn stop(&mut self) -> Result<(), GenieError> {
            Ok(())
        }
        fn may_attack(
            &mut self,
            _: i32,
            _: &Seeker,
        ) -> Result<Option<(i16, [f32; 3])>, GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
        fn seek_unexplored(
            &mut self,
            _: &UnitAi,
            _: &mut Well512,
        ) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn make_scout_map(&mut self) -> Result<(), GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
        fn can_stand(&mut self, point: [f32; 2]) -> Result<bool, GenieError> {
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
            current_order: ATTACK_MOVE,
            adjusted_idle_timeout: 750,
            idle_timeout: 3000,
            secondary_timer: 900,
            ..idle(13)
        };
        let mut r = rng();
        u.set_order(-1, &mut r);
        assert_eq!((u.current_order, u.last_order), (-1, ATTACK_MOVE));
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
    /// (`FUN_140fde4f0`: the boar's for 1, else the lion's).
    #[test]
    fn the_factory_picks_the_unit_ai_type_by_master_and_class() {
        assert_eq!(factory_type(6, 1105, -1, 0), Some(0x12));
        assert_eq!(factory_type(6, 1744, -1, 0), Some(0x10));
        assert_eq!(factory_type(6, 1746, -1, 0), Some(0x10));
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
                factory_type(class, 83, -1, 0),
                Some(ai_type),
                "class {class}"
            );
        }
        for class in [3, 27, 39, 49, 60] {
            assert_eq!(factory_type(class, 70, -1, 0), Some(17));
            assert_eq!(factory_type(class, 70, 5, 0), Some(0xf));
        }
        for class in [5, 7, 8, 11, 14, 15, 16, 30, 31, 32, 33, 34, 38, 42, 48, 63] {
            assert_eq!(factory_type(class, 83, -1, 0), None);
        }
        // class 10: the boar's for combat level 1, else the lion's
        assert_eq!(factory_type(10, 48, -1, 1), Some(BOAR_AI));
        assert_eq!(factory_type(10, 126, -1, 0), Some(5));
        assert_eq!(factory_type(10, 126, -1, 4), Some(5));
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

    /// `0x140aa2a70` in state 3: a unit without an AI asks for its path even in range and keeps
    /// its attack move; one off the patrol orders ends in range and stops attack-moving.
    #[test]
    fn a_unit_off_patrol_asks_a_plain_path() {
        let ask = UnitAi::path_ask(None, [20.0, 20.0], [30.0, 20.0]);
        assert_eq!(
            ask,
            PathAsk {
                ends_in_range: false,
                ..PathAsk::default()
            }
        );
        let mut u = idle(13);
        let ask = UnitAi::path_ask(Some(&mut u), [20.0, 20.0], [30.0, 20.0]);
        assert_eq!(
            ask,
            PathAsk {
                attack_move: Some(false),
                ..PathAsk::default()
            }
        );
        // a saved patrol off the patrol orders stays, and so does the attack move
        u.patrol_path = Some(PatrolPath::default());
        let ask = UnitAi::path_ask(Some(&mut u), [20.0, 20.0], [30.0, 20.0]);
        assert_eq!((ask.attack_move, u.patrol_path.is_some()), (None, true));
    }

    /// `0x140aa2a70` in state 3 on a patrol order: the path loops; without a saved patrol it goes
    /// back and forth from 3 tiles (`SETBE`: a NaN distance too); order 733 attack-moves.
    #[test]
    fn a_patrol_order_s_path_loops_and_turns_back_from_three_tiles() {
        let mut u = UnitAi {
            current_order: PATROL,
            ..idle(13)
        };
        let ask = UnitAi::path_ask(Some(&mut u), [20.0, 20.0], [22.9, 20.0]);
        assert_eq!(
            ask,
            PathAsk {
                ends_in_range: false,
                patrol: true,
                back_and_forth: false,
                resume: -1,
                attack_move: Some(false),
            }
        );
        let far = UnitAi::path_ask(Some(&mut u), [20.0, 20.0], [20.0, 23.0]);
        assert!(far.back_and_forth);
        let nan = UnitAi::path_ask(Some(&mut u), [20.0, 20.0], [f32::NAN, 23.0]);
        assert!(nan.back_and_forth);
        u.current_order = ATTACK_MOVE;
        let attack = UnitAi::path_ask(Some(&mut u), [20.0, 20.0], [22.9, 20.0]);
        assert_eq!((attack.patrol, attack.attack_move), (true, Some(true)));
    }

    /// `0x140aa2a70` in state 3 on a patrol order with a saved patrol: its waypoint and its back
    /// and forth (`getPatrolLinear`, flags bit 1) are the path's, and it goes
    /// (`zeroPatrolInformation`), which then stops the attack move.
    #[test]
    fn a_saved_patrol_gives_its_waypoint_to_the_path_and_goes() {
        let mut u = UnitAi {
            current_order: PATROL,
            patrol_path: Some(PatrolPath {
                flags: 3,
                ..PatrolPath::default()
            }),
            patrol_waypoint: 2,
            ..idle(13)
        };
        let ask = UnitAi::path_ask(Some(&mut u), [20.0, 20.0], [20.5, 20.0]);
        assert_eq!(
            (ask.resume, ask.back_and_forth, ask.attack_move),
            (2, true, Some(false))
        );
        assert_eq!((u.patrol_path, u.patrol_waypoint), (None, 0));
    }

    /// A human player's soldier walking its patrol (711 with the move action, its `MoveTo` the
    /// top action) as the full update finds it, its secondary timer `secondary`.
    fn patroller(secondary: u32) -> (UnitAi, Surroundings) {
        let u = UnitAi {
            current_order: PATROL,
            current_action: MOVE_ACTION,
            idle_timeout: 3000,
            adjusted_idle_timeout: 750,
            secondary_timer: secondary,
            ..idle(SOLDIER_AI)
        };
        let at = Surroundings {
            position: [20.0, 20.0, 0.0],
            player: 1,
            kind: 70,
            state: 2,
            size: [0.2, 0.2],
            los: 4.0,
            weapon_range: 1.0,
            busy: true,
            ..Surroundings::default()
        };
        (u, at)
    }

    /// The idle part's patrol branch (`0x140c67f0b`) while the secondary timer runs: nothing
    /// but the timer's count down; the busy check lets the walking patrol through.
    #[test]
    fn a_patrol_waits_for_its_secondary_timer() {
        let (mut u, at) = patroller(300);
        let mut side = SoldierSide {
            found: 77,
            attacks: Vec::new(),
            patrol: None,
        };
        let fx = u.update(28, 0, false, &at, &mut rng(), &mut side).unwrap();
        assert_eq!(fx, vec![]);
        assert_eq!((u.secondary_timer, u.current_order), (272, PATROL));
        assert_eq!(side.attacks, Vec::<(i32, bool)>::new());
    }

    /// The patrol branch with its secondary timer run out: it restarts at 750 (the adjusted
    /// idle timeout of 750 redrawn), processIdle's search finds 77: the patrol is saved (the
    /// unit's slot 0x468), the attack command given, and the order goes without its history
    /// (`FUN_140c64b60` with -1 and 0).
    #[test]
    fn a_patrol_saves_itself_and_attacks_what_it_finds() {
        let (mut u, at) = patroller(0);
        u.last_order = 9;
        let saved = PatrolPath {
            flags: 3,
            ..PatrolPath::default()
        };
        let mut side = SoldierSide {
            found: 77,
            attacks: Vec::new(),
            patrol: Some((saved.clone(), 2)),
        };
        let mut r = rng();
        let _ = u.update(28, 0, false, &at, &mut r, &mut side).unwrap();
        assert_eq!(side.attacks, vec![(77, true)]);
        assert_eq!((u.current_order, u.last_order), (-1, 9));
        assert_eq!((u.patrol_path, u.patrol_waypoint), (Some(saved), 2));
        assert_eq!(u.secondary_timer, 750);
        assert_ne!(u.adjusted_idle_timeout, 750);
        assert!(0 < r.draws);
    }

    /// The patrol branch passes over processIdle while the top action's target is a building
    /// (slot 400); a unit target it does not.
    #[test]
    fn a_patrol_attacking_a_building_keeps_at_it() {
        let (mut u, mut at) = patroller(0);
        at.top_target = Some(TopTarget {
            id: 50,
            kind: 80,
            position: [30.0, 30.0, 0.0],
            radius: [1.0, 1.0],
        });
        let mut side = SoldierSide {
            found: 77,
            attacks: Vec::new(),
            patrol: None,
        };
        let _ = u.update(28, 0, false, &at, &mut rng(), &mut side).unwrap();
        assert_eq!(side.attacks, Vec::<(i32, bool)>::new());
        assert_eq!(u.secondary_timer, 750);
    }

    /// The soldier's idle check (`FUN_140c20db0`) takes its top action's target within its
    /// range (slot 400, slot 0x1e8: the footprints' squared gap between the least range and the
    /// weapon range squared) without a search; out of range it searches.
    #[test]
    fn a_soldier_s_idle_check_attacks_its_top_action_s_target_in_range() {
        let (mut u, mut at) = patroller(0);
        u.current_order = -1;
        let near = TopTarget {
            id: 50,
            kind: 70,
            position: [21.0, 20.0, 0.0],
            radius: [0.2, 0.2],
        };
        at.top_target = Some(near);
        assert_eq!(top_target_in_range(&at), Some(50));
        at.min_range = 1.0;
        assert_eq!(top_target_in_range(&at), None);
        at.min_range = 0.0;
        let mut side = SoldierSide {
            found: 77,
            attacks: Vec::new(),
            patrol: None,
        };
        let mut fx = Vec::new();
        assert_eq!(
            u.soldier_idle(&at, false, &mut rng(), &mut side, &mut fx),
            Ok(true)
        );
        assert_eq!(side.attacks, vec![(50, true)]);
        // too far: the search's 77
        let (mut u, mut at) = patroller(0);
        u.current_order = -1;
        at.top_target = Some(TopTarget {
            position: [25.0, 20.0, 0.0],
            ..near
        });
        side.attacks.clear();
        assert_eq!(
            u.soldier_idle(&at, false, &mut rng(), &mut side, &mut Vec::new()),
            Ok(true)
        );
        assert_eq!(side.attacks, vec![(77, true)]);
    }

    /// The soldier's idle check with nothing to attack and a saved patrol, in no group:
    /// `restorePatrolInformation` gives the unit its walk back onto the patrol, the order 711
    /// (733 for a unit that attack-moves) and a secondary timer of 750; processIdle produced.
    #[test]
    fn a_soldier_with_nothing_to_attack_goes_back_to_its_patrol() {
        let (mut u, mut at) = patroller(0);
        u.current_order = -1;
        u.current_action = -1;
        u.patrol_path = Some(PatrolPath::default());
        at.busy = false;
        let mut side = SoldierSide {
            found: -1,
            attacks: Vec::new(),
            patrol: None,
        };
        let mut fx = Vec::new();
        assert_eq!(
            u.soldier_idle(&at, false, &mut rng(), &mut side, &mut fx),
            Ok(true)
        );
        assert_eq!(
            fx,
            vec![
                Effect::CapHumanOrder(HUMAN_ORDER_CAP),
                Effect::RestorePatrol
            ]
        );
        assert_eq!((u.current_order, u.secondary_timer), (PATROL, 750));
        u.current_order = -1;
        at.attack_moving = true;
        assert_eq!(
            u.soldier_idle(&at, false, &mut rng(), &mut side, &mut Vec::new()),
            Ok(true)
        );
        assert_eq!(u.current_order, ATTACK_MOVE);
    }

    /// The order check of a unit no longer busy: the attack move (733) ends, its timers reset
    /// (an adjusted idle timeout of 750 redrawn, the secondary timer capped at 750); the patrol
    /// order (711) stays.
    #[test]
    fn an_attack_move_not_busy_ends_and_a_patrol_stays() {
        let at = Surroundings::default();
        let mut u = UnitAi {
            current_order: ATTACK_MOVE,
            idle_timeout: 3000,
            adjusted_idle_timeout: 750,
            secondary_timer: 900,
            ..idle(SOLDIER_AI)
        };
        u.check_order(&at, &mut rng());
        assert_eq!((u.current_order, u.secondary_timer), (-1, 750));
        assert_ne!(u.adjusted_idle_timeout, 750);
        let mut u = UnitAi {
            current_order: PATROL,
            ..idle(SOLDIER_AI)
        };
        u.check_order(&at, &mut rng());
        assert_eq!(u.current_order, PATROL);
    }

    /// Slot 0xb0 (`FUN_140c64ed0`) on a patrol order: the order ends even when the stop keeps
    /// orders, its secondary timer capped at 750; with an adjusted idle timeout of exactly 750
    /// the redraw (the AI stream's) is not modelled here.
    #[test]
    fn a_stop_ends_a_patrol_order_with_its_secondary_timer_capped() {
        let mut u = UnitAi {
            current_order: PATROL,
            current_action: MOVE_ACTION,
            adjusted_idle_timeout: 2600,
            secondary_timer: 900,
            ..idle(SOLDIER_AI)
        };
        let mut fx = Vec::new();
        u.stop_with(true, Some(([3.0, 4.0], 6)), &mut fx).unwrap();
        assert_eq!(
            (u.current_order, u.last_order, u.secondary_timer),
            (-1, PATROL, 750)
        );
        assert_eq!((fx, u.state_position), (vec![Effect::Stop], [3.0, 4.0]));
        let mut u = UnitAi {
            current_order: ATTACK_MOVE,
            adjusted_idle_timeout: 750,
            ..idle(SOLDIER_AI)
        };
        assert!(u.stop_with(true, None, &mut Vec::new()).is_err());
    }

    /// The order's tail (`LAB_140c5d611`) and the Build's end (`FUN_140c64b60` with −1): leaving
    /// a patrol order caps the secondary timer; staying on it does not.
    #[test]
    fn leaving_a_patrol_order_caps_its_secondary_timer() {
        let order = |order| QueuedOrder {
            order,
            priority: 100,
            ..QueuedOrder::default()
        };
        let patrolling = || UnitAi {
            current_order: PATROL,
            adjusted_idle_timeout: 2600,
            secondary_timer: 900,
            ..idle(SOLDIER_AI)
        };
        let tail = |u: &mut UnitAi, o: QueuedOrder| {
            u.order_tail(o.order, o.priority, 6, false, &mut rng())
        };
        let mut u = patrolling();
        assert_eq!(tail(&mut u, order(MOVE_ORDER)), 1);
        assert_eq!((u.current_order, u.secondary_timer), (MOVE_ORDER, 750));
        let mut u = patrolling();
        assert_eq!(tail(&mut u, order(PATROL)), 1);
        assert_eq!(u.secondary_timer, 900);
        let mut u = patrolling();
        u.end_order_and_action().unwrap();
        assert_eq!(
            (u.current_order, u.last_order, u.secondary_timer),
            (-1, PATROL, 750)
        );
    }

    /// A unit AI with an order on a target ready in its queue, its idle timer, look-around timer
    /// and retarget timer all past their timeouts.
    fn ordered(ai_type: u32) -> UnitAi {
        UnitAi {
            orders: vec![QueuedOrder {
                order: ORDER_ON_TARGET,
                priority: 100,
                target: 77,
                ready: true,
                ..QueuedOrder::default()
            }],
            idle_timer: 5000,
            look_around_timer: 2000,
            last_retarget_time: 0,
            retarget_timer: 900,
            ..idle(ai_type)
        }
    }

    /// `0x140c66b10` runs the order on a target inside the update (slot 0x230's case 0x2db), so
    /// the update stops there: the order and what the rest of the update reads of its start
    /// ([`AfterOrder`]: its `busy` and group) go to the caller; slot 73, the idle part and the
    /// look-around check have not run (no draw, the timers as they grew).
    #[test]
    fn a_full_update_stops_at_its_order_on_a_target() {
        let mut u = ordered(SOLDIER_AI);
        let order = u.orders[0];
        let mut r = rng();
        // the world holds the order's target
        let at = Surroundings {
            units: vec![UnitSeen {
                id: 77,
                ..UnitSeen::default()
            }],
            ..Surroundings::default()
        };
        let fx = u
            .update(100, 10_000, false, &at, &mut r, &mut no_search)
            .unwrap();
        assert_eq!(
            fx,
            vec![
                Effect::OrderOnTarget(order),
                Effect::AfterOrder(AfterOrder {
                    busy: false,
                    group: None,
                }),
            ]
        );
        assert_eq!(r.draws, 0);
        assert_eq!(
            (
                u.idle_timer,
                u.look_around_timer,
                u.retarget_timer,
                u.orders.len()
            ),
            (5100, 2100, 900, 0)
        );
    }

    /// The update's rest after the order (`0x140c66b10` from slot 73 on): the retarget check
    /// reads the order's attack (its timer redrawn, then a human unit in an attack's work state
    /// stops there); the idle part sees the action, so an action of a player above Gaia keeps the
    /// idle timer at 0; the look-around check redraws its timeout.
    #[test]
    fn the_rest_after_an_order_that_gave_an_action_reads_it() {
        let mut u = ordered(SOLDIER_AI);
        u.orders.clear();
        u.current_order = ATTACK_ORDER;
        u.current_action = ATTACK_ACTION;
        u.current_target = 77;
        let at = Surroundings {
            player: 1,
            target: Some(Seen {
                owner: 2,
                class: 6,
                ..Seen::default()
            }),
            action_state: Some(6),
            ..Surroundings::default()
        };
        let mut r = rng();
        let fx = u
            .after_order(
                AfterOrder::default(),
                10_000,
                false,
                &at,
                &mut r,
                &mut no_search,
            )
            .unwrap();
        let mut expected = rng();
        let (retarget, look) = (expected.rand15(), expected.rand15());
        assert_eq!(fx, Vec::<Effect>::new());
        assert_eq!(
            (
                u.last_retarget_time,
                u.retarget_timer,
                u.idle_timer,
                u.look_around_timer,
                u.look_around_timeout,
                r.draws
            ),
            (10_000, 750 + retarget % 500, 0, 0, 4500 + look % 1000, 2)
        );
    }

    /// An order that gave the AI no action (no task on its target): the idle timer past its
    /// timeout restarts and slot 72 runs with 1 (the villager's: its cap of the last human order
    /// time). With the update's `busy` and a target the order gave, it does not; the idle part's
    /// end then keeps the timer of a player above Gaia at 0.
    #[test]
    fn the_rest_after_an_order_that_gave_no_action_runs_the_idle_check() {
        let mut u = ordered(CIVILIAN);
        u.orders.clear();
        let at = Surroundings {
            player: 1,
            ..Surroundings::default()
        };
        let mut r = rng();
        let fx = u
            .after_order(
                AfterOrder::default(),
                10_000,
                false,
                &at,
                &mut r,
                &mut no_search,
            )
            .unwrap();
        assert_eq!(fx, vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)]);
        assert_eq!((u.idle_timer, r.draws), (0, 1));
        let mut u = ordered(CIVILIAN);
        u.orders.clear();
        u.current_target = 77;
        let busy = AfterOrder {
            busy: true,
            group: None,
        };
        let fx = u
            .after_order(busy, 10_000, false, &at, &mut rng(), &mut no_search)
            .unwrap();
        assert_eq!((fx, u.idle_timer), (Vec::<Effect>::new(), 0));
        // Gaia's unit keeps its timer
        let mut u = ordered(CIVILIAN);
        u.orders.clear();
        u.current_target = 77;
        let gaia = Surroundings::default();
        let _ = u
            .after_order(busy, 10_000, false, &gaia, &mut rng(), &mut no_search)
            .unwrap();
        assert_eq!(u.idle_timer, 5000);
    }

    /// A human player's transport ship at (20, 30) attacked by unit 77 two tiles east of it,
    /// already among its attackers.
    fn attacked_transport() -> (UnitAi, Surroundings) {
        let u = UnitAi {
            attackers: vec![77],
            ..idle(11)
        };
        let at = Surroundings {
            id: 500,
            position: [20.0, 30.0, 0.0],
            speed: 1.0,
            map: [64, 64],
            player: 1,
            class: 20,
            state: 2,
            units: vec![UnitSeen {
                id: 77,
                class: 22,
                owner: 2,
                state: 2,
                target: Some(500),
                position: [22.0, 30.0, 0.0],
                ..UnitSeen::default()
            }],
            ..Surroundings::default()
        };
        (u, at)
    }

    /// The transport ship's handler (`FUN_140c2a580`): a notify other than damage and a
    /// missile's warning is the base handler's, as it answers it.
    #[test]
    fn the_transport_ship_passes_other_notifies_to_the_base_handler() {
        let (mut u, at) = attacked_transport();
        u.current_action = MOVE_ACTION;
        u.current_order = MOVE_ORDER;
        let mut base = u.clone();
        let n = Notify::own(500, ACTION_COMPLETED, [MOVE_ACTION, 0, 0]);
        let (mut fx, mut base_fx) = (Vec::new(), Vec::new());
        let r = u.handle(
            n,
            0,
            &at,
            false,
            &mut rng(),
            &mut Recorder::default(),
            &mut fx,
        );
        let b = base.base_notify(
            n,
            0,
            &at,
            false,
            &mut rng(),
            &mut Recorder::default(),
            &mut base_fx,
        );
        assert_eq!((r, fx), (b, base_fx));
        assert_eq!(u, base);
    }

    /// Its damage notify: the attacker it knows joins no one new (no call for help), the unit is
    /// under attack; without an order or an action it runs from its attackers by twice its speed
    /// (slot 0x160 with 1) and breaks off (3). With an order it ignores the damage (2). At no hit
    /// points left it gives up (4). A computer player's AI would hear the alert (not modelled).
    #[test]
    fn a_transport_ship_runs_from_its_attackers_without_an_order() {
        let (mut u, at) = attacked_transport();
        let n = Notify::own(77, DAMAGED, [77, 40, 0]);
        let mut side = StandSide {
            refusals: 0,
            tested: Vec::new(),
        };
        let mut fx = Vec::new();
        assert_eq!(
            u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
            Ok(BREAK)
        );
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
        let (mut u, at) = attacked_transport();
        u.current_order = MOVE_ORDER;
        let mut fx = Vec::new();
        assert_eq!(
            u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
            Ok(GO_ON)
        );
        assert_eq!(fx, Vec::<Effect>::new());
        // busy with its top action in state 2 it runs too; in another state it stays
        let (mut u, mut at) = attacked_transport();
        at.busy = true;
        at.action_state = Some(4);
        let mut fx = Vec::new();
        assert_eq!(
            u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
            Ok(BREAK)
        );
        assert_eq!(fx, Vec::<Effect>::new());
        let (mut u, at) = attacked_transport();
        let dead = Notify::own(77, DAMAGED, [77, 0, 0]);
        assert_eq!(
            u.handle(dead, 0, &at, false, &mut rng(), &mut side, &mut Vec::new()),
            Ok(DEAD)
        );
        let (mut u, at) = attacked_transport();
        assert!(
            u.handle(n, 0, &at, true, &mut rng(), &mut side, &mut Vec::new())
                .is_err()
        );
        // a new attacker that is not its target: the call for help (AskForHelp) looks the
        // attacker up in the world first, which this side does not hold
        let (mut u, at) = attacked_transport();
        u.attackers.clear();
        assert_eq!(
            u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut Vec::new()),
            Err(GenieError::unsupported(
                "an object's look without the world"
            ))
        );
    }

    /// A missile's warning (`0x20f`): one draw, `r * 100 / 0x7fff` below the owner's
    /// `chance_to_dodge_missiles` dodges it (2); else it is damage.
    #[test]
    fn a_transport_ship_dodges_a_missile_s_warning_by_its_owner_s_chance() {
        let r = rng().rand15() as i32 * 100 / 0x7fff;
        let warning = Notify::own(77, MISSILE_WARNING, [77, 40, 0]);
        let mut side = StandSide {
            refusals: 0,
            tested: Vec::new(),
        };
        let (mut u, mut at) = attacked_transport();
        at.dodge_chance = u8::try_from(r + 1).unwrap();
        let mut ai = rng();
        let mut fx = Vec::new();
        assert_eq!(
            u.handle(warning, 0, &at, false, &mut ai, &mut side, &mut fx),
            Ok(GO_ON)
        );
        assert_eq!((ai.draws, fx.len()), (1, 0));
        let (mut u, mut at) = attacked_transport();
        at.dodge_chance = u8::try_from(r).unwrap();
        let mut fx = Vec::new();
        assert_eq!(
            u.handle(warning, 0, &at, false, &mut rng(), &mut side, &mut fx),
            Ok(BREAK)
        );
        assert_eq!(fx.len(), 1);
    }

    /// The siege tower (0x12): its idle check (`FUN_140c20690`) only caps the last human order
    /// time; its damage notify (`FUN_140c28400`) is the transport ship's, but a human player's
    /// tower in a group that is not idle ignores the damage (2), and one in an idle group runs.
    #[test]
    fn a_siege_tower_in_a_busy_group_ignores_its_damage() {
        let mut u = idle(SIEGE_TOWER_AI);
        let mut fx = Vec::new();
        let mut r = rng();
        assert_eq!(
            u.process_idle(
                &Surroundings::default(),
                false,
                &mut r,
                &mut Recorder::default(),
                &mut fx
            ),
            Ok(false)
        );
        assert_eq!(
            (fx, r.draws),
            (vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)], 0)
        );
        let n = Notify::own(77, DAMAGED, [77, 40, 0]);
        let mut side = StandSide {
            refusals: 0,
            tested: Vec::new(),
        };
        let tower = |idle_group: bool| {
            let (mut u, mut at) = attacked_transport();
            u.ai_type = SIEGE_TOWER_AI;
            at.group = Some(GroupState {
                idle: idle_group,
                ..GroupState::default()
            });
            (u, at)
        };
        let (mut u, at) = tower(false);
        let mut fx = Vec::new();
        assert_eq!(
            u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
            Ok(GO_ON)
        );
        assert_eq!((fx.len(), u.attackers.clone()), (0, vec![77]));
        let (mut u, at) = tower(true);
        let mut fx = Vec::new();
        assert_eq!(
            u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
            Ok(BREAK)
        );
        assert_eq!(fx.len(), 1);
    }

    /// `AttackObject` (slot 0xb8) of the types whose slot 0xa0 is the base's (`FUN_140c48400`,
    /// anything): the ranged units', the ships', the artifact's and the siege tower's attack a
    /// unit the world holds; the AI takes the attack and the target.
    #[test]
    fn the_base_attack_check_lets_every_type_without_its_own_attack() {
        for ai_type in [6, 7, 10, 11, 12, ARTIFACT_AI, SIEGE_TOWER_AI] {
            let mut u = idle(ai_type);
            let at = Surroundings {
                units: vec![UnitSeen {
                    id: 77,
                    class: 6,
                    position: [3.0, 4.0, 0.0],
                    ..UnitSeen::default()
                }],
                ..Surroundings::default()
            };
            let mut side = Recorder::default();
            let mut fx = Vec::new();
            assert_eq!(
                u.attack_object(77, true, false, &at, &mut side, &mut fx),
                Ok(true),
                "type {ai_type}"
            );
            assert_eq!(
                (side.attacks, u.current_action, u.current_target),
                (vec![(77, true)], ATTACK_ACTION, 77)
            );
        }
    }

    /// A ship's world: its searches answer from `found` (one answer per call, in order) and are
    /// recorded; the objects it names stand at their places; every point can be stood on and
    /// every target reached within `reach`.
    #[derive(Default)]
    struct ShipSide {
        found: Vec<Found>,
        searched: Vec<(f32, Vec<i32>, Pick)>,
        units: Vec<UnitSeen>,
        reach: bool,
    }

    impl UnitSide for ShipSide {
        fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
            None
        }
        fn closest(&mut self, search: &Search<'_>) -> Result<Found, GenieError> {
            self.searched
                .push((search.radius, search.classes.to_vec(), search.pick));
            Ok(if self.found.is_empty() {
                Found::NONE
            } else {
                self.found.remove(0)
            })
        }
        fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
            Err(GenieError::unsupported("no attack here"))
        }
        fn set_under_attack(&mut self, _: bool) {}
        fn stop(&mut self) -> Result<(), GenieError> {
            Ok(())
        }
        fn may_attack(
            &mut self,
            _: i32,
            _: &Seeker,
        ) -> Result<Option<(i16, [f32; 3])>, GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
        fn seek_unexplored(
            &mut self,
            _: &UnitAi,
            _: &mut Well512,
        ) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn make_scout_map(&mut self) -> Result<(), GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
        fn can_stand(&mut self, _: [f32; 2]) -> Result<bool, GenieError> {
            Ok(true)
        }
        fn seen_unit(&mut self, id: i32) -> Result<Option<UnitSeen>, GenieError> {
            Ok(self.units.iter().find(|u| u.id == id).copied())
        }
        fn reaches(&mut self, _: i32, _: f32) -> Result<bool, GenieError> {
            Ok(self.reach)
        }
    }

    /// The ships' idle check (`FUN_140c21930`, `FUN_140c1eab0`): a human player's only caps the
    /// last human order time and searches nothing (5); a computer player's fishing ship looks for
    /// the closest object of its classes, its radius grown by 7, and moves away from it by seven
    /// times its speed (6); a trade cart searches its own classes; finding nothing, 5.
    #[test]
    fn a_computer_player_s_ship_evades_the_closest_object_it_sees() {
        let at = Surroundings {
            id: 500,
            position: [20.0, 30.0, 0.0],
            speed: 1.0,
            map: [64, 64],
            los: 4.0,
            player: 1,
            class: 21,
            ..Surroundings::default()
        };
        let mut u = idle(FISHING_AI);
        let mut fx = Vec::new();
        let mut side = ShipSide::default();
        assert_eq!(
            u.process_idle(&at, false, &mut rng(), &mut side, &mut fx),
            Ok(false)
        );
        assert_eq!(
            (fx, side.searched.len()),
            (vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)], 0)
        );
        let mut side = ShipSide {
            found: vec![Found {
                id: 77,
                ..Found::NONE
            }],
            units: vec![UnitSeen {
                id: 77,
                position: [22.0, 30.0, 0.0],
                ..UnitSeen::default()
            }],
            ..ShipSide::default()
        };
        let mut fx = Vec::new();
        assert_eq!(
            u.process_idle(&at, true, &mut rng(), &mut side, &mut fx),
            Ok(true)
        );
        assert_eq!(
            fx,
            vec![
                Effect::CapHumanOrder(HUMAN_ORDER_CAP),
                Effect::Move(AiMove {
                    point: [13.0, 30.0, 0.0],
                    range: 0.0,
                    force: true,
                    walk: false,
                }),
            ]
        );
        let (radius, classes, pick) = side.searched[0].clone();
        assert_eq!((radius, classes), (12.0, FISHING_CARED.to_vec()));
        assert!(matches!(
            pick,
            Pick::Closest {
                group: -1,
                relation: -1,
                reach: true,
                ..
            }
        ));
        // a trade cart (class 19) finding nothing
        let mut cart = idle(TRADE_AI);
        let mut side = ShipSide::default();
        let at = Surroundings { class: 19, ..at };
        assert_eq!(
            cart.process_idle(&at, true, &mut rng(), &mut side, &mut Vec::new()),
            Ok(false)
        );
        assert_eq!(side.searched[0].0, 5.0);
        assert_eq!(side.searched[0].1, TRADE_CART_CARED.to_vec());
    }

    /// The trade ship's case 0x202 (`FUN_140c29bf0`) off its trade and explore actions: the
    /// target goes, the action stops, and the processing goes on (2).
    #[test]
    fn a_trade_ship_whose_action_needs_a_search_stops() {
        let (mut u, at) = attacked_transport();
        u.ai_type = TRADE_AI;
        u.current_action = MOVE_ACTION;
        u.current_target = 12;
        let n = Notify::own(500, REQUIRES_SEARCH, [MOVE_ACTION, 0, 0]);
        let mut fx = Vec::new();
        assert_eq!(
            u.handle(
                n,
                0,
                &at,
                false,
                &mut rng(),
                &mut Recorder::default(),
                &mut fx
            ),
            Ok(GO_ON)
        );
        assert_eq!((u.current_action, u.current_target), (-1, -1));
        assert_eq!(fx, vec![Effect::Stop]);
        for action in [TRADE_ACTION, EXPLORE_ACTION] {
            let (mut u, at) = attacked_transport();
            u.ai_type = TRADE_AI;
            u.current_action = action;
            assert!(
                u.handle(
                    n,
                    0,
                    &at,
                    false,
                    &mut rng(),
                    &mut Recorder::default(),
                    &mut fx
                )
                .is_err()
            );
        }
    }

    /// The trade ship's damage notify (`FUN_140c29bf0`): an attacker it knows raises no alert
    /// and no call for help; without an order, off the move action, the action stops and it
    /// runs from its attackers (3); on the move action it runs on (no stop).
    #[test]
    fn a_trade_ship_stops_and_runs_from_its_attackers() {
        let n = Notify::own(77, DAMAGED, [77, 40, 0]);
        let (mut u, at) = attacked_transport();
        u.ai_type = TRADE_AI;
        u.current_action = 0x261;
        let mut fx = Vec::new();
        let mut side = ShipSide::default();
        assert_eq!(
            u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
            Ok(BREAK)
        );
        assert_eq!(fx.first(), Some(&Effect::Stop));
        assert_eq!(fx.len(), 2);
        assert_eq!(u.orders, Vec::new());
        let (mut u, at) = attacked_transport();
        u.ai_type = TRADE_AI;
        u.current_action = MOVE_ACTION;
        let mut fx = Vec::new();
        assert_eq!(
            u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
            Ok(BREAK)
        );
        assert_eq!(u.current_action, MOVE_ACTION);
        assert!(!fx.contains(&Effect::Stop));
    }

    /// The fishing ship's damage notify (`FUN_140c25ca0`): on the move action it neither stops
    /// nor runs (3, nothing); off it, it stops and runs.
    #[test]
    fn a_fishing_ship_on_the_move_takes_its_damage_without_running() {
        let n = Notify::own(77, DAMAGED, [77, 40, 0]);
        let (mut u, at) = attacked_transport();
        u.ai_type = FISHING_AI;
        u.current_action = MOVE_ACTION;
        let mut fx = Vec::new();
        let mut side = ShipSide::default();
        assert_eq!(
            u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
            Ok(BREAK)
        );
        assert_eq!(fx, Vec::new());
        let (mut u, at) = attacked_transport();
        u.ai_type = FISHING_AI;
        u.current_action = GATHER_ACTION;
        let mut fx = Vec::new();
        assert_eq!(
            u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
            Ok(BREAK)
        );
        assert_eq!(fx.first(), Some(&Effect::Stop));
        assert_eq!(fx.len(), 2);
    }

    /// The fishing ship's builder search (`FUN_140c25ca0`, case 0x202 on its build): its own
    /// foundations (relation 3, state 0) of the classes 3, 27, 39, 52 and 49, in that order;
    /// the nearest by `ClosestObject`'s distance (a tie keeps the first) that it reaches it
    /// builds (3). Reaching none, a human player's ship goes on (2); a computer player's posts
    /// itself its build's completion.
    #[test]
    fn a_fishing_ship_builds_the_nearest_foundation_it_reaches() {
        let at = Surroundings {
            id: 500,
            position: [20.0, 30.0, 0.0],
            los: 4.0,
            player: 1,
            state: 2,
            ..Surroundings::default()
        };
        let found = |id, distance| Found {
            id,
            distance,
            ..Found::NONE
        };
        let mut u = UnitAi {
            current_action: BUILD_ACTION,
            ..idle(FISHING_AI)
        };
        let mut side = ShipSide {
            found: vec![
                found(-1, -1),
                found(41, 6),
                found(42, 3),
                found(43, 3),
                Found::NONE,
            ],
            reach: true,
            ..ShipSide::default()
        };
        let n = Notify::own(500, REQUIRES_SEARCH, [BUILD_ACTION, 0, 0]);
        let mut fx = Vec::new();
        assert_eq!(
            u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
            Ok(BREAK)
        );
        assert_eq!(fx, vec![Effect::BuildObject(42)]);
        let groups: Vec<i32> = side
            .searched
            .iter()
            .map(|(_, _, p)| match p {
                Pick::Closest {
                    group,
                    relation: 3,
                    state: 0,
                    ..
                } => *group,
                _ => -9,
            })
            .collect();
        assert_eq!(groups, FISHING_BUILDS.to_vec());
        // out of reach: the human's goes on, the computer's posts its completion
        let mut side = ShipSide {
            found: vec![found(41, 6)],
            ..ShipSide::default()
        };
        let mut fx = Vec::new();
        assert_eq!(
            u.handle(n, 0, &at, false, &mut rng(), &mut side, &mut fx),
            Ok(GO_ON)
        );
        assert_eq!((fx.len(), u.buffered.len(), u.notifies.len()), (0, 0, 0));
        let mut side = ShipSide::default();
        assert_eq!(
            u.handle(n, 0, &at, true, &mut rng(), &mut side, &mut Vec::new()),
            Ok(GO_ON)
        );
        assert_eq!(
            u.notifies,
            vec![Notify::own(500, ACTION_COMPLETED, [BUILD_ACTION, 0, 0])]
        );
    }

    /// A siege unit of player 1 (a mangonel: class 13, master 280, range 7, radii 0.6) at its
    /// state position.
    const MANGONEL: Attacker = Attacker {
        ai_type: SIEGE_AI,
        player: 1,
        kind: 70,
        class: 13,
        master: 280,
        missile: 656,
        range: 7.0,
        los: 9.0,
        position: [20.0, 20.0, 0.0],
        radius: [0.6, 0.6],
        speed: 0.6,
        ..TC
    };

    /// The siege unit's attack check (`FUN_140c19190`): never classes 1 and 42 nor in
    /// `NoAttack`; standing its ground within its range of its state position less its own
    /// diagonal (`powf`, `sqrtf`), a Gaia target's range 1.5 longer; defending within the range
    /// plus 5; a Gaia unit only taking owners by sight; never from below state 2, a human
    /// player's packed unit (unless any) or while packing; by priority 0 a ram only at siege and
    /// buildings, a trebuchet only at buildings; priority 3 only its classes.
    #[test]
    fn a_siege_unit_attacks_by_its_stance_its_class_and_its_priority() {
        let villager = |x: f32| Target {
            owner: 2,
            class: 4,
            master: 83,
            position: [x, 20.0, 0.0],
            radius: [0.2, 0.2],
            ..BOAR
        };
        let seeker = |stance| Seeker {
            stance,
            state_position: [20.0, 20.0],
            ..IDLE
        };
        let check = |me: &Attacker, s: &Seeker, t: &Target| siege_can_attack(me, s, t, false);
        assert_eq!(check(&MANGONEL, &seeker(0), &villager(40.0)), Ok(true));
        assert_eq!(check(&MANGONEL, &seeker(3), &villager(21.0)), Ok(false));
        for class in [1, 42] {
            let t = Target {
                class,
                ..villager(21.0)
            };
            assert_eq!(check(&MANGONEL, &seeker(0), &t), Ok(false));
        }
        // the diagonal: sqrt(0.36 + 0.36) = 0.8485; a gap of 7.8 - 0.8485 is within 7
        let diagonal = crate::fmath::sqrt(0.6f32 * 0.6 + 0.6 * 0.6);
        let x = 20.0 + 0.2 + 7.0 + diagonal;
        assert_eq!(check(&MANGONEL, &seeker(2), &villager(x - 0.01)), Ok(true));
        assert_eq!(check(&MANGONEL, &seeker(2), &villager(x + 0.01)), Ok(false));
        let gaia = Target {
            owner: 0,
            ..villager(x + 1.4)
        };
        assert_eq!(check(&MANGONEL, &seeker(2), &gaia), Ok(true));
        let defend = seeker(1);
        assert_eq!(check(&MANGONEL, &defend, &villager(x + 4.99)), Ok(true));
        assert_eq!(check(&MANGONEL, &defend, &villager(x + 5.01)), Ok(false));
        let patrolling = Seeker {
            order: PATROL,
            ..defend
        };
        assert!(check(&MANGONEL, &patrolling, &villager(30.0)).is_err());
        // Gaia's unit, a low state, a packed unit, a packing one
        let gaia_unit = Attacker {
            player: 0,
            capture: 1,
            ..MANGONEL
        };
        assert_eq!(check(&gaia_unit, &seeker(0), &villager(30.0)), Ok(false));
        let gaia_taker = Attacker {
            capture: 0,
            ..gaia_unit
        };
        assert_eq!(check(&gaia_taker, &seeker(0), &villager(30.0)), Ok(true));
        let low = Attacker {
            state: 1,
            ..MANGONEL
        };
        assert_eq!(check(&low, &seeker(0), &villager(30.0)), Ok(false));
        let packed = Attacker {
            class: 51,
            ..MANGONEL
        };
        assert_eq!(check(&packed, &seeker(0), &villager(30.0)), Ok(false));
        assert_eq!(
            siege_can_attack(&packed, &seeker(0), &villager(30.0), true),
            Ok(true)
        );
        let packing = Attacker {
            top_action: Some(124),
            ..MANGONEL
        };
        assert_eq!(check(&packing, &seeker(0), &villager(30.0)), Ok(false));
        // a ram attacks buildings and siege, a trebuchet buildings only
        let ram = Attacker {
            master: 35,
            ..MANGONEL
        };
        let siege = Target {
            class: 13,
            ..villager(30.0)
        };
        assert_eq!(check(&ram, &seeker(0), &villager(30.0)), Ok(false));
        assert_eq!(check(&ram, &seeker(0), &siege), Ok(true));
        let trebuchet = Attacker {
            class: 54,
            ..MANGONEL
        };
        assert_eq!(check(&trebuchet, &seeker(0), &siege), Ok(false));
        let house = Target {
            class: 3,
            kind: 80,
            ..villager(30.0)
        };
        assert_eq!(check(&trebuchet, &seeker(0), &house), Ok(true));
        // masters 1744 and 1746 on a building: priority 1 or 4 and up only
        let elephant = |attack_priority| Attacker {
            master: 1744,
            attack_priority,
            ..MANGONEL
        };
        let on_building = Seeker {
            target: 900,
            target_kind: Some(80),
            ..seeker(0)
        };
        for (priority, attacks) in [(0, false), (1, true), (2, false), (3, false), (4, true)] {
            assert_eq!(
                check(&elephant(priority), &on_building, &villager(30.0)),
                Ok(attacks),
                "priority {priority}"
            );
        }
        let priority_3 = Attacker {
            attack_priority: 3,
            ..MANGONEL
        };
        assert_eq!(check(&priority_3, &seeker(0), &villager(30.0)), Ok(false));
        assert_eq!(check(&priority_3, &seeker(0), &house), Ok(true));
        // its neutral test spares the trade cart's class no more
        assert!(siege_attacks_neutral(0, 19));
        assert!(!soldier_attacks_neutral(0, 19));
        assert!(!siege_attacks_neutral(0, 4));
    }

    /// A siege unit's world: its search finds `found`, the attack check lets `allowed` be
    /// attacked, the attack command takes, and `reach` answers the reach test.
    #[derive(Default)]
    struct SiegeSide {
        found: Option<Found>,
        allowed: Option<(i16, [f32; 3])>,
        attacks: Vec<(i32, bool)>,
        reach: bool,
        searches: usize,
    }

    impl UnitSide for SiegeSide {
        fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
            None
        }
        fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
            self.searches += 1;
            Ok(self.found.unwrap_or(Found::NONE))
        }
        fn attack(&mut self, target: i32, force: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
            self.attacks.push((target, force));
            Ok(Attacked {
                attacks: true,
                posted: Vec::new(),
            })
        }
        fn set_under_attack(&mut self, _: bool) {}
        fn stop(&mut self) -> Result<(), GenieError> {
            Ok(())
        }
        fn may_attack(
            &mut self,
            _: i32,
            _: &Seeker,
        ) -> Result<Option<(i16, [f32; 3])>, GenieError> {
            Ok(self.allowed)
        }
        fn seek_unexplored(
            &mut self,
            _: &UnitAi,
            _: &mut Well512,
        ) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn make_scout_map(&mut self) -> Result<(), GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
        fn reaches(&mut self, _: i32, _: f32) -> Result<bool, GenieError> {
            Ok(self.reach)
        }
    }

    /// The siege unit's idle check (`FUN_140c206b0`): it caps the last human order time, finds a
    /// target with `bestUnitToAttack` and attacks it (forced), the order going (6); finding
    /// nothing, a unit in a group gives 5 and one defending walks back. Gaia's that takes no
    /// owners by sight does nothing.
    #[test]
    fn a_siege_unit_attacks_what_its_idle_search_finds() {
        let at = Surroundings {
            id: 500,
            position: [20.0, 20.0, 0.0],
            player: 1,
            class: 13,
            kind: 70,
            los: 9.0,
            weapon_range: 7.0,
            state: 2,
            ..Surroundings::default()
        };
        let mut u = UnitAi {
            current_order: MOVE_ORDER,
            ..idle(SIEGE_AI)
        };
        let mut side = SiegeSide {
            found: Some(Found {
                id: 77,
                ..Found::NONE
            }),
            allowed: Some((4, [25.0, 20.0, 0.0])),
            ..SiegeSide::default()
        };
        let mut fx = Vec::new();
        assert_eq!(
            u.process_idle(&at, false, &mut rng(), &mut side, &mut fx),
            Ok(true)
        );
        assert_eq!(fx, vec![Effect::CapHumanOrder(HUMAN_ORDER_CAP)]);
        assert_eq!(side.attacks, vec![(77, true)]);
        assert_eq!(
            (u.current_action, u.current_target, u.current_order),
            (ATTACK_ACTION, 77, -1)
        );
        // nothing found: in a group 5 and no walk; defending without a group walks back
        let grouped = Surroundings {
            group: Some(GroupState::default()),
            ..at.clone()
        };
        let mut u = UnitAi {
            stance: 1,
            state_position: [10.0, 20.0],
            ..idle(SIEGE_AI)
        };
        let mut side = SiegeSide::default();
        let mut fx = Vec::new();
        assert_eq!(
            u.process_idle(&grouped, false, &mut rng(), &mut side, &mut fx),
            Ok(false)
        );
        assert_eq!(fx.len(), 1);
        let mut fx = Vec::new();
        assert_eq!(
            u.process_idle(&at, false, &mut rng(), &mut side, &mut fx),
            Ok(false)
        );
        assert!(matches!(fx.last(), Some(Effect::Move(_))));
        // Gaia's that does not take owners by sight
        let gaia = Surroundings {
            player: 0,
            capture: 1,
            ..at.clone()
        };
        let mut side = SiegeSide::default();
        assert_eq!(
            idle(SIEGE_AI).process_idle(&gaia, false, &mut rng(), &mut side, &mut Vec::new()),
            Ok(false)
        );
        assert_eq!(side.searches, 0);
    }

    /// With retargeting off and its current target held, a top target the siege unit reaches
    /// within its range ends the check (5); not reached, retargeting comes back on.
    #[test]
    fn a_siege_unit_with_retargeting_off_keeps_a_reachable_top_target() {
        let at = Surroundings {
            id: 500,
            position: [20.0, 20.0, 0.0],
            player: 1,
            class: 13,
            kind: 70,
            weapon_range: 7.0,
            state: 2,
            target: Some(Seen::default()),
            top_target: Some(TopTarget {
                id: 88,
                kind: 70,
                position: [24.0, 20.0, 0.0],
                radius: [0.2, 0.2],
            }),
            ..Surroundings::default()
        };
        let mut u = UnitAi {
            retarget_disabled: 1,
            current_target: 88,
            ..idle(SIEGE_AI)
        };
        let mut side = SiegeSide {
            reach: true,
            ..SiegeSide::default()
        };
        assert_eq!(
            u.process_idle(&at, false, &mut rng(), &mut side, &mut Vec::new()),
            Ok(false)
        );
        assert_eq!((u.retarget_disabled, side.attacks.len()), (1, 0));
        let mut side = SiegeSide {
            allowed: Some((4, [24.0, 20.0, 0.0])),
            ..SiegeSide::default()
        };
        assert_eq!(
            u.process_idle(&at, false, &mut rng(), &mut side, &mut Vec::new()),
            Ok(true)
        );
        assert_eq!(
            (u.retarget_disabled, side.attacks.clone()),
            (0, vec![(88, true)])
        );
    }

    /// The siege unit's handler (`FUN_140c287e0`): a target under its least range makes it back
    /// up (not modelled); a human player's executed action and damage are the base handler's; a
    /// computer player's executed attack draws whether it keeps its distance.
    #[test]
    fn a_siege_unit_hands_its_notifies_to_the_base_handler() {
        let at = Surroundings {
            id: 500,
            weapon_range: 7.0,
            player: 1,
            ..Surroundings::default()
        };
        let mut u = idle(SIEGE_AI);
        let mut side = SiegeSide::default();
        let under = Notify::own(500, 0x1fe, [77, 0, 0]);
        assert!(
            u.handle(under, 0, &at, false, &mut rng(), &mut side, &mut Vec::new())
                .is_err()
        );
        let executed = Notify::own(500, ACTION_EXECUTED, [600, 77, 0]);
        assert_eq!(
            u.handle(
                executed,
                0,
                &at,
                false,
                &mut rng(),
                &mut side,
                &mut Vec::new()
            ),
            Ok(GO_ON)
        );
        // a computer player's, off the attack action: one draw, then 2
        let mut r = rng();
        assert_eq!(
            u.handle(executed, 0, &at, true, &mut r, &mut side, &mut Vec::new()),
            Ok(GO_ON)
        );
        assert_eq!(r.draws, 1);
        // a building target: no draw, the base handler's
        let mut on_house = UnitAi {
            target_type: 3,
            ..idle(SIEGE_AI)
        };
        let mut r = rng();
        assert_eq!(
            on_house.handle(executed, 0, &at, true, &mut r, &mut side, &mut Vec::new()),
            Ok(GO_ON)
        );
        assert_eq!(r.draws, 0);
        // its run: a top target or class 54 keeps it
        let pinned = Surroundings {
            top_target: Some(TopTarget::default()),
            ..at.clone()
        };
        let mut fx = Vec::new();
        assert_eq!(
            u.run_away(true, &pinned, false, &mut rng(), &mut side, &mut fx),
            Ok(false)
        );
        assert_eq!(fx, Vec::new());
    }

    /// The priest's slot 73 (`FUN_140c21f30`): the base's retarget check (a used timer redrawn),
    /// but on the order on a target and for a computer player's priest none.
    #[test]
    fn a_priest_retargets_but_on_an_order_on_a_target() {
        let at = Surroundings {
            player: 1,
            ..Surroundings::default()
        };
        let priest = |order| UnitAi {
            current_order: order,
            retarget_timer: 900,
            last_retarget_time: 0,
            adjusted_idle_timeout: 9000,
            look_around_timeout: 9000,
            ..idle(PRIEST_AI)
        };
        for (order, computer, draws) in [(-1, false, 1), (ORDER_ON_TARGET, false, 0), (-1, true, 0)]
        {
            let mut u = priest(order);
            let mut r = rng();
            let _ = u.update_tail(
                Retarget::Priest,
                10_000,
                computer,
                false,
                None,
                &at,
                &mut r,
                &mut Recorder::default(),
                &mut Vec::new(),
            );
            assert_eq!(r.draws, draws, "order {order}, computer {computer}");
        }
        assert!(matches!(retarget_kind(PRIEST_AI), Ok(Retarget::Priest)));
    }

    /// A side for the full update's tail: the unit's top action's target now, the objects'
    /// flags and states.
    #[derive(Default)]
    struct TailSide {
        top: Option<i32>,
        flags: Vec<(i32, TargetFlags)>,
        states: Vec<(i32, u8)>,
    }

    impl UnitSide for TailSide {
        fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
            None
        }
        fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn attack(&mut self, _: i32, _: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
            Err(GenieError::unsupported("no attack here"))
        }
        fn set_under_attack(&mut self, _: bool) {}
        fn stop(&mut self) -> Result<(), GenieError> {
            Ok(())
        }
        fn may_attack(
            &mut self,
            _: i32,
            _: &Seeker,
        ) -> Result<Option<(i16, [f32; 3])>, GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
        fn seek_unexplored(
            &mut self,
            _: &UnitAi,
            _: &mut Well512,
        ) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn make_scout_map(&mut self) -> Result<(), GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
        fn top_target_now(&mut self) -> Result<Option<i32>, GenieError> {
            Ok(self.top)
        }
        fn target_flags(&mut self, id: i32) -> Result<Option<TargetFlags>, GenieError> {
            Ok(self.flags.iter().find(|f| f.0 == id).map(|f| f.1))
        }
        fn seen_unit(&mut self, id: i32) -> Result<Option<UnitSeen>, GenieError> {
            Ok(self
                .states
                .iter()
                .find(|s| s.0 == id)
                .map(|&(id, state)| UnitSeen {
                    id,
                    state,
                    ..UnitSeen::default()
                }))
        }
    }

    /// A target the tail lets be: a player's, targetable and attackable, outside a pasture.
    const PLAIN: TargetFlags = TargetFlags {
        owner: 2,
        kind: 70,
        capture: 0,
        pasture: -1,
        untargetable: false,
        attackable: true,
    };

    fn on_target(action: i32) -> UnitAi {
        UnitAi {
            current_action: action,
            current_target: 500,
            target_type: 6,
            target_point: [10.0, 10.0, 0.0],
            ..idle(5)
        }
    }

    fn tail(u: &mut UnitAi, player: usize, flags: TargetFlags, fx: &mut Vec<Effect>) {
        let at = Surroundings {
            player,
            ..Surroundings::default()
        };
        let mut side = TailSide {
            top: Some(500),
            flags: vec![(500, flags)],
            ..TailSide::default()
        };
        u.top_target_tail(&at, &mut side, fx).unwrap();
    }

    #[test]
    fn the_tail_stops_a_gaia_unit_on_a_gaia_object_others_can_take_again() {
        // 0x140c68519: a Gaia unit, a Gaia target of type 70 or more, its +0x924 2: the target
        // goes (FUN_140c5e1e0) and the action stops (slot 0xb0 with 1)
        let again = TargetFlags {
            owner: 0,
            capture: 2,
            ..PLAIN
        };
        let mut u = on_target(ATTACK_ACTION);
        let mut fx = Vec::new();
        tail(&mut u, 0, again, &mut fx);
        assert_eq!(fx, [Effect::Stop]);
        assert_eq!((u.current_target, u.last_target), (-1, 500));
        assert_eq!((u.current_action, u.last_action), (-1, ATTACK_ACTION));
        // a player's unit, a player's target, another capture flag or a type below 70 stays
        for (player, flags) in [
            (1, again),
            (0, TargetFlags { owner: 1, ..again }),
            (
                0,
                TargetFlags {
                    capture: 1,
                    ..again
                },
            ),
            (0, TargetFlags { kind: 60, ..again }),
        ] {
            let mut u = on_target(ATTACK_ACTION);
            let mut fx = Vec::new();
            tail(&mut u, player, flags, &mut fx);
            assert_eq!(fx, Vec::<Effect>::new(), "{player} {flags:?}");
            assert_eq!(u.current_target, 500);
        }
    }

    #[test]
    fn the_tail_stops_work_on_a_target_without_its_targeting_flag() {
        // 0x140c685cf: outside a pasture, a target that cannot be targeted (+0x689 0) stops any
        // action but the trade
        let untargetable = TargetFlags {
            untargetable: true,
            ..PLAIN
        };
        for action in [GATHER_ACTION, ATTACK_ACTION, -1] {
            let mut u = on_target(action);
            let mut fx = Vec::new();
            tail(&mut u, 1, untargetable, &mut fx);
            assert_eq!(fx, [Effect::Stop], "{action}");
            assert_eq!(u.current_target, -1);
        }
        let mut u = on_target(TRADE_ACTION);
        let mut fx = Vec::new();
        tail(&mut u, 1, untargetable, &mut fx);
        assert_eq!(fx, Vec::<Effect>::new());
        // one in a pasture stays
        let mut u = on_target(GATHER_ACTION);
        tail(
            &mut u,
            1,
            TargetFlags {
                pasture: 77,
                ..untargetable
            },
            &mut fx,
        );
        assert_eq!(fx, Vec::<Effect>::new());
        // one that cannot be attacked (+0x68a 0) stops only the attack
        let unattackable = TargetFlags {
            attackable: false,
            ..PLAIN
        };
        let mut u = on_target(GATHER_ACTION);
        tail(&mut u, 1, unattackable, &mut fx);
        assert_eq!(fx, Vec::<Effect>::new());
        let mut u = on_target(ATTACK_ACTION);
        tail(&mut u, 1, unattackable, &mut fx);
        assert_eq!(fx, [Effect::Stop]);
        // both: the second test reads the action value the first stop left (−1), so an
        // untargetable target stops it again
        let both = TargetFlags {
            owner: 0,
            capture: 2,
            untargetable: true,
            ..PLAIN
        };
        let mut u = on_target(ATTACK_ACTION);
        let mut fx = Vec::new();
        tail(&mut u, 0, both, &mut fx);
        assert_eq!(fx, [Effect::Stop, Effect::Stop]);
        assert_eq!((u.current_action, u.last_action), (-1, -1));
        let unattackable_again = TargetFlags {
            untargetable: false,
            attackable: false,
            ..both
        };
        let mut u = on_target(ATTACK_ACTION);
        let mut fx = Vec::new();
        tail(&mut u, 0, unattackable_again, &mut fx);
        assert_eq!(fx, [Effect::Stop]);
    }

    #[test]
    fn the_tail_reads_the_top_target_the_update_s_commands_left() {
        let untargetable = TargetFlags {
            untargetable: true,
            ..PLAIN
        };
        let at = Surroundings {
            player: 1,
            ..Surroundings::default()
        };
        let mut side = TailSide {
            top: Some(500),
            flags: vec![(500, untargetable), (600, untargetable)],
            ..TailSide::default()
        };
        // a move to a point, a stop or an explore left no object to test
        for pending in [
            Effect::Stop,
            Effect::Explore([1.0, 2.0]),
            Effect::Move(AiMove {
                point: [1.0, 2.0, 0.0],
                range: 0.0,
                force: true,
                walk: false,
            }),
        ] {
            let mut u = on_target(GATHER_ACTION);
            let mut fx = vec![pending];
            u.top_target_tail(&at, &mut side, &mut fx).unwrap();
            assert_eq!(fx, [pending]);
        }
        // a move to an object holds it
        let mut u = on_target(MOVE_ACTION);
        let to = Effect::MoveToObject {
            target: 600,
            state: 2,
            range: 2.0,
        };
        let mut fx = vec![to];
        u.top_target_tail(&at, &mut side, &mut fx).unwrap();
        assert_eq!(fx, [to, Effect::Stop]);
        // after a hunt the caller gives once the update ends, the stop would reach the unit
        // first
        let mut u = on_target(HUNT_ACTION);
        let mut fx = vec![Effect::HuntObject(600)];
        assert!(u.top_target_tail(&at, &mut side, &mut fx).is_err());
        // no top action, or one the world no longer holds: nothing
        let mut u = on_target(GATHER_ACTION);
        let mut fx = Vec::new();
        u.top_target_tail(
            &at,
            &mut TailSide {
                top: Some(700),
                ..TailSide::default()
            },
            &mut fx,
        )
        .unwrap();
        u.top_target_tail(&at, &mut TailSide::default(), &mut fx)
            .unwrap();
        assert_eq!(fx, Vec::<Effect>::new());
    }

    #[test]
    fn queued_builds_on_started_or_gone_foundations_leave_the_queue() {
        // 0x140c686e0: off the build order, each queued build order (the last first) on a
        // foundation the world does not hold or holds out of state 0 goes; other orders stay
        let order = |order: i32, target: i32| QueuedOrder {
            order,
            target,
            ready: true,
            ..QueuedOrder::default()
        };
        let queue = vec![
            order(BUILD_ORDER, 1),
            order(BUILD_ORDER, 2),
            order(MOVE_ORDER, 3),
            order(BUILD_ORDER, 4),
            order(BUILD_ORDER, 5),
        ];
        let mut side = TailSide {
            states: vec![(1, 0), (2, 2), (3, 2), (5, 0)],
            ..TailSide::default()
        };
        let mut u = UnitAi {
            orders: queue.clone(),
            ..idle(8)
        };
        u.drop_started_builds(&mut side).unwrap();
        let left: Vec<i32> = u.orders.iter().map(|o| o.target).collect();
        assert_eq!(left, [1, 3, 5]);
        // on the build order the queue stays
        let mut u = UnitAi {
            current_order: BUILD_ORDER,
            orders: queue,
            ..idle(8)
        };
        u.drop_started_builds(&mut side).unwrap();
        assert_eq!(u.orders.len(), 5);
    }

    #[test]
    fn a_unit_standing_its_ground_that_strayed_with_an_action_stops_and_walks_back() {
        // slot 73's stand ground leash (0x140c57bc4): the squared gap to the state position
        // (4 tiles: 16) against the range plus 0.25
        let at = Surroundings {
            position: [14.0, 10.0, 0.0],
            weapon_range: 15.0,
            player: 1,
            ..Surroundings::default()
        };
        let standing = |action| UnitAi {
            stance: 2,
            state_position: [10.0, 10.0],
            current_action: action,
            current_target: 500,
            retarget_timer: 100_000,
            ..idle(13)
        };
        let mut u = standing(ATTACK_ACTION);
        let mut fx = Vec::new();
        assert_eq!(
            u.retarget_check(0, false, &at, &mut rng(), &mut finds_nothing(), &mut fx),
            Ok(false)
        );
        assert_eq!(fx[0], Effect::Stop);
        assert_eq!(
            fx[1],
            Effect::Move(AiMove {
                point: [10.0, 10.0, 0.0],
                range: WALK_BACK,
                force: true,
                walk: false,
            })
        );
        assert_eq!((u.current_target, u.current_action), (-1, MOVE_ACTION));
        // within the range plus 0.25, without an action value, moving aside, on a patrol order
        // or in a group that patrols: it stays
        let near = Surroundings {
            weapon_range: 15.75,
            ..at.clone()
        };
        let patrolling = Surroundings {
            group: Some(GroupState {
                patrol: true,
                ..GroupState::default()
            }),
            ..at.clone()
        };
        for (mut u, at) in [
            (standing(ATTACK_ACTION), &near),
            (standing(-1), &at),
            (standing(MOVE_ASIDE_ACTION), &at),
            (standing(GROUP_ASIDE_ACTION), &at),
            (
                UnitAi {
                    current_order: PATROL,
                    ..standing(ATTACK_ACTION)
                },
                &at,
            ),
            (standing(ATTACK_ACTION), &patrolling),
        ] {
            let mut fx = Vec::new();
            let _ = u
                .retarget_check(0, false, at, &mut rng(), &mut finds_nothing(), &mut fx)
                .unwrap();
            assert_eq!(fx, Vec::<Effect>::new(), "{:?}", u.current_action);
        }
        // a gap that is not a number stays (COMISS, JBE)
        let mut u = UnitAi {
            state_position: [f32::NAN, 10.0],
            ..standing(ATTACK_ACTION)
        };
        let mut fx = Vec::new();
        let _ = u
            .retarget_check(0, false, &at, &mut rng(), &mut finds_nothing(), &mut fx)
            .unwrap();
        assert_eq!(fx, Vec::<Effect>::new());
    }

    /// `FUN_14087e7c0` and `FUN_140f95e10`: the least distance to the lines through a path's
    /// consecutive points (the lines, not the segments); a path of one point answers the largest
    /// f32, an empty one is unreachable; a NaN distance is kept until the next one.
    #[test]
    fn a_patrol_s_distance_is_the_least_to_the_lines_through_its_points() {
        assert_eq!(
            line_distance([3.0, 0.0, 4.0], [0.0; 3], [10.0, 0.0, 0.0]),
            4.0
        );
        // beyond the segment's end the line still counts
        assert_eq!(
            line_distance([30.0, 0.0, 4.0], [0.0; 3], [10.0, 0.0, 0.0]),
            4.0
        );
        let path = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 0.0, 10.0]];
        assert_eq!(patrol_distance(&path, [9.0, 0.0, 4.0]), Ok(1.0));
        assert_eq!(
            patrol_distance(&path[..1], [9.0, 0.0, 4.0]),
            Ok(PATROL_FARTHEST)
        );
        assert!(patrol_distance(&[], [9.0, 0.0, 4.0]).is_err());
        // two equal points make a NaN, which the next line replaces
        let doubled = [[0.0; 3], [0.0; 3], [10.0, 0.0, 0.0]];
        assert_eq!(patrol_distance(&doubled, [3.0, 0.0, 4.0]), Ok(4.0));
        assert!(
            patrol_distance(&doubled[..2], [3.0, 0.0, 4.0])
                .unwrap()
                .is_nan()
        );
    }

    /// processMisc's leash with patrol information (`+0x1c0`): from where the unit last moved to
    /// (slot 0x520) to the saved patrol's path (`FUN_140f95e10`). Beyond stand ground's 0.5 or
    /// the defend stance's 5 the unit stops, forgets its target and walks its patrol on, its
    /// order the patrol's (10); within it stand ground goes on to the retarget check (its
    /// state position's leash not read), the defend stance ends slot 73 (9) before it.
    #[test]
    fn a_unit_with_patrol_information_leashes_to_its_patrol_s_path() {
        let path = PatrolPath {
            path: crate::tile_path::Path {
                points: vec![[0.0, 0.0, 10.0], [20.0, 0.0, 10.0]],
                length: 20.0,
            },
            flags: 3,
        };
        // where the unit last moved to: on the path's line, 1 or 6 off it
        let at = |y: f32| Surroundings {
            position: [14.0, y, 0.0],
            moved_at: [14.0, 0.0, y],
            weapon_range: 15.0,
            player: 1,
            ..Surroundings::default()
        };
        let unit = |stance| UnitAi {
            stance,
            state_position: [10.0, 30.0],
            current_action: ATTACK_ACTION,
            current_target: 500,
            retarget_timer: 0,
            last_retarget_time: 0,
            patrol_path: Some(path.clone()),
            ..idle(13)
        };
        let run = |u: &mut UnitAi, at: &Surroundings| {
            let mut fx = Vec::new();
            let r = u.retarget_check(1000, false, at, &mut rng(), &mut finds_nothing(), &mut fx);
            (r, fx)
        };
        // stand ground within 0.5: no leash, on to the retarget check (the timer runs out)
        let mut u = unit(2);
        let (r, fx) = run(&mut u, &at(10.0));
        assert_eq!((r, fx), (Ok(true), vec![]));
        assert_eq!(u.last_retarget_time, 1000);
        // beyond it: stop, the target forgotten, the patrol walked on (10)
        let mut u = unit(2);
        let (r, fx) = run(&mut u, &at(11.0));
        assert_eq!(
            (r, fx),
            (Ok(false), vec![Effect::Stop, Effect::RestorePatrol])
        );
        assert_eq!(
            (u.current_target, u.current_order, u.secondary_timer),
            (-1, PATROL, 750)
        );
        // the defend stance within 5 ends slot 73 before the retarget check (9)
        let mut u = unit(1);
        let (r, fx) = run(&mut u, &at(11.0));
        assert_eq!((r, fx), (Ok(false), vec![]));
        assert_eq!(u.last_retarget_time, 0);
        // beyond 5 it walks the patrol on
        let mut u = unit(1);
        let (r, fx) = run(&mut u, &at(16.0));
        assert_eq!(
            (r, fx),
            (Ok(false), vec![Effect::Stop, Effect::RestorePatrol])
        );
    }

    #[test]
    fn a_busy_villager_on_the_attack_order_runs_its_full_update() {
        // records (6)/426406723's villagers attacking (order 700, action 600, a target), busy: the
        // update runs whole (the civilian's processMisc, the idle part keeping the timer of a
        // player's unit with an action at 0)
        let mut u = UnitAi {
            current_order: ATTACK_ORDER,
            current_action: ATTACK_ACTION,
            current_target: 18842,
            target_type: 10,
            idle_timer: 400,
            ..idle(8)
        };
        let at = Surroundings {
            busy: true,
            player: 3,
            class: CLASS_VILLAGER,
            state: 2,
            target: Some(Seen {
                state: 2,
                kind: 70,
                owner: 0,
                class: 10,
                ..Seen::default()
            }),
            ..Surroundings::default()
        };
        let fx = u
            .update(10, 1000, false, &at, &mut rng(), &mut TailSide::default())
            .unwrap();
        assert_eq!(fx, Vec::<Effect>::new());
        assert_eq!(u.idle_timer, 0);
        assert_eq!(
            (u.current_order, u.current_action, u.current_target),
            (ATTACK_ORDER, ATTACK_ACTION, 18842)
        );
    }

    /// A side for the order slots: the objects the world holds (their class, place, state and
    /// gap), the unit's attack commands and other commands, which it takes unless `refuses`, the
    /// reach test's answer, the soldier's group stops and the waits for foundations.
    #[derive(Default)]
    struct OrderSide {
        world: Vec<UnitSeen>,
        attacks: Vec<(i32, bool)>,
        commands: Vec<UnitCommand>,
        refuses: bool,
        reach: bool,
        stops: usize,
        group_stops: usize,
        waits: Vec<i32>,
        /// What the resource searches find, and the searches asked.
        resource: Option<Gatherable>,
        searches: Vec<ResourceSearch>,
        /// The classes of the players' masters: ((player, master), class).
        classes: Vec<((i32, i32), i16)>,
    }

    impl OrderSide {
        /// A world of objects `(id, class)` in state 2 at (10, 20, 0), a gap of 1 from the unit.
        fn holding(objects: &[(i32, i16)]) -> Self {
            Self {
                world: objects
                    .iter()
                    .map(|&(id, class)| UnitSeen {
                        id,
                        class,
                        state: 2,
                        gap_squared: 1.0,
                        position: [10.0, 20.0, 0.0],
                        ..UnitSeen::default()
                    })
                    .collect(),
                ..Self::default()
            }
        }
    }

    impl UnitSide for OrderSide {
        fn patrol_information(&self) -> Option<(PatrolPath, i32)> {
            None
        }
        fn closest(&mut self, _: &Search<'_>) -> Result<Found, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn attack(&mut self, target: i32, force: bool, _: &UnitAi) -> Result<Attacked, GenieError> {
            self.attacks.push((target, force));
            Ok(Attacked {
                attacks: !self.refuses,
                posted: Vec::new(),
            })
        }
        fn set_under_attack(&mut self, _: bool) {}
        fn stop(&mut self) -> Result<(), GenieError> {
            self.stops += 1;
            Ok(())
        }
        fn may_attack(
            &mut self,
            target: i32,
            _: &Seeker,
        ) -> Result<Option<(i16, [f32; 3])>, GenieError> {
            Ok(self.seen_unit(target)?.map(|t| (t.class, t.position)))
        }
        fn seek_unexplored(
            &mut self,
            _: &UnitAi,
            _: &mut Well512,
        ) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn seek_beyond_sight(&mut self, _: &mut Well512) -> Result<Option<[i32; 2]>, GenieError> {
            Err(GenieError::unsupported("no search here"))
        }
        fn make_scout_map(&mut self) -> Result<(), GenieError> {
            Err(GenieError::unsupported("no world here"))
        }
        fn reaches(&mut self, _: i32, _: f32) -> Result<bool, GenieError> {
            Ok(self.reach)
        }
        fn can_stand(&mut self, _: [f32; 2]) -> Result<bool, GenieError> {
            Ok(true)
        }
        fn seen_unit(&mut self, id: i32) -> Result<Option<UnitSeen>, GenieError> {
            Ok(self.world.iter().find(|u| u.id == id).copied())
        }
        fn wait_solidify(&mut self, target: i32) -> Result<bool, GenieError> {
            self.waits.push(target);
            Ok(!self.refuses)
        }
        fn command(&mut self, command: UnitCommand) -> Result<bool, GenieError> {
            self.commands.push(command);
            Ok(!self.refuses)
        }
        fn group_attack_stop(&mut self) -> Result<(), GenieError> {
            self.group_stops += 1;
            Ok(())
        }
        fn closest_resource(
            &mut self,
            search: ResourceSearch,
        ) -> Result<Option<Gatherable>, GenieError> {
            self.searches.push(search);
            Ok(self.resource)
        }
        fn master_class(&mut self, player: i32, master: i32) -> Result<Option<i16>, GenieError> {
            Ok(self
                .classes
                .iter()
                .find(|c| c.0 == (player, master))
                .map(|c| c.1))
        }
    }

    /// An order of value `order` on `target` at (12.7, 30.2, 0.5) within 3, priority 40, ready.
    fn queued(order: i32, target: i32) -> QueuedOrder {
        QueuedOrder {
            issuer: 1,
            order,
            priority: 40,
            target,
            target_owner: -1,
            point: [12.7, 30.2, 0.5],
            range: 3.0,
            shown: [12.5, 30.5],
            ready: true,
            target_type: -1,
        }
    }

    /// Slot 0x230 on order `o` inside the update: the unit of class `class` at (5, 6, 0.25), its
    /// master 4, player 1; the result, the effects and the draws.
    fn run_order(
        u: &mut UnitAi,
        o: QueuedOrder,
        class: i16,
        side: &mut OrderSide,
    ) -> (Result<u32, GenieError>, Vec<Effect>, u32) {
        let at = Surroundings {
            id: 9,
            position: [5.0, 6.0, 0.25],
            map: [100, 100],
            class,
            master: 4,
            player: 1,
            state: 2,
            ..Surroundings::default()
        };
        let mut fx = Vec::new();
        let mut r = rng();
        let res = u.process_order(&o, Some((&at, side, false)), None, &mut r, &mut fx);
        (res, fx, r.draws)
    }

    #[test]
    fn an_order_not_ready_stays_and_patrol_or_unnamed_orders_take_nothing() {
        let mut u = idle(13);
        let mut side = OrderSide::default();
        let waiting = QueuedOrder {
            ready: false,
            ..queued(MOVE_ORDER, -1)
        };
        assert_eq!(run_order(&mut u, waiting, 6, &mut side).0, Ok(0xe));
        // "I don't know what to do with patrol orders"; an order no case names
        for order in [PATROL, 0x2c4, 0x2cb, 0x2d5, 0x323] {
            let (r, fx, _) = run_order(&mut u, queued(order, -1), 6, &mut side);
            assert_eq!((r, fx), (Ok(0), Vec::new()), "{order:#x}");
        }
        assert_eq!(u, idle(13));
    }

    #[test]
    fn the_attack_order_attacks_by_the_unit_ai_s_type_and_becomes_the_ai_s() {
        // a soldier: the base's AttackObject through its slot 0xa0 (the side's test), then a
        // member of a group stops moving (FUN_140c17550)
        let mut u = idle(SOLDIER_AI);
        let mut side = OrderSide::holding(&[(70, 6)]);
        let (r, fx, _) = run_order(&mut u, queued(ATTACK_ORDER, 70), 6, &mut side);
        assert_eq!((r, fx), (Ok(1), Vec::new()));
        assert_eq!(side.attacks, [(70, true)]);
        assert_eq!(side.group_stops, 0);
        assert_eq!(
            (u.current_order, u.order_priority, u.current_action),
            (ATTACK_ORDER, 40, ATTACK_ACTION)
        );
        assert_eq!(
            (u.current_target, u.target_type, u.target_point),
            (70, 6, [10.0, 20.0, 0.0])
        );
        let mut u = idle(SOLDIER_AI);
        let mut side = OrderSide::holding(&[(70, 6)]);
        let o = queued(ATTACK_ORDER, 70);
        let at = Surroundings {
            class: 6,
            group: Some(GroupState::default()),
            ..Surroundings::default()
        };
        let r = u.process_order(
            &o,
            Some((&at, &mut side, false)),
            None,
            &mut rng(),
            &mut Vec::new(),
        );
        assert_eq!((r, side.group_stops), (Ok(1), 1));
        // a villager hunts an animal (class 9) and attacks anything else it may (class 6)
        let mut u = idle(CIVILIAN);
        let mut side = OrderSide::holding(&[(71, 9), (72, 6)]);
        let (r, fx, _) = run_order(&mut u, queued(ATTACK_ORDER, 71), 4, &mut side);
        assert_eq!((r, fx), (Ok(1), vec![Effect::HuntObject(71)]));
        assert_eq!(
            (u.current_order, u.current_action),
            (ATTACK_ORDER, HUNT_ACTION)
        );
        let mut u = idle(CIVILIAN);
        let (r, _, _) = run_order(&mut u, queued(ATTACK_ORDER, 72), 4, &mut side);
        assert_eq!((r, u.current_action), (Ok(1), ATTACK_ACTION));
        // a unit of class 18: the tail forgets the target (its class and point stay) and ends
        // the order, the order's priority kept
        let mut u = idle(BOAR_AI);
        let (r, _, _) = run_order(&mut u, queued(ATTACK_ORDER, 72), CLASS_18, &mut side);
        assert_eq!(r, Ok(1));
        assert_eq!(
            (
                u.current_order,
                u.order_priority,
                u.current_target,
                u.target_type
            ),
            (-1, 40, -1, 6)
        );
        // a command the unit refuses, or a target the world does not hold: not taken
        let mut u = idle(BOAR_AI);
        side.refuses = true;
        assert_eq!(
            run_order(&mut u, queued(ATTACK_ORDER, 72), 10, &mut side).0,
            Ok(0)
        );
        assert_eq!(
            run_order(&mut u, queued(ATTACK_ORDER, 99), 10, &mut side).0,
            Ok(0)
        );
        assert_eq!((u.current_order, u.current_action), (-1, -1));
    }

    #[test]
    fn the_defend_order_keeps_near_its_target_or_its_point() {
        let mut side = OrderSide::holding(&[(70, 6)]);
        let mut u = UnitAi {
            current_action: MOVE_ACTION,
            ..idle(SOLDIER_AI)
        };
        let (r, fx, _) = run_order(&mut u, queued(DEFEND_ORDER, 70), 6, &mut side);
        assert_eq!((r, fx), (Ok(1), Vec::new()));
        assert_eq!(
            (u.current_order, u.current_action, u.last_action),
            (DEFEND_ORDER, DEFEND_ACTION, MOVE_ACTION)
        );
        assert_eq!(
            (u.current_target, u.target_type, u.target_point),
            (70, 6, [10.0, 20.0, 0.0])
        );
        assert_eq!((u.desired_distance, u.defend_target), (3.0, 70));
        // without a target: the point at the unit's height, no defend target
        let mut u = idle(SOLDIER_AI);
        assert_eq!(
            run_order(&mut u, queued(DEFEND_ORDER, -1), 6, &mut side).0,
            Ok(1)
        );
        assert_eq!(
            (u.current_target, u.target_point, u.defend_target),
            (-1, [12.7, 30.2, 0.25], -1)
        );
        // a target the world does not hold: not taken
        let mut u = idle(SOLDIER_AI);
        assert_eq!(
            run_order(&mut u, queued(DEFEND_ORDER, 99), 6, &mut side).0,
            Ok(0)
        );
        // master 128 takes no defend order
        let at = Surroundings {
            master: DEFENDLESS_MASTER,
            ..Surroundings::default()
        };
        let o = queued(DEFEND_ORDER, -1);
        let r = u.process_order(
            &o,
            Some((&at, &mut side, false)),
            None,
            &mut rng(),
            &mut Vec::new(),
        );
        assert_eq!(r, Ok(0));
        assert_eq!(u.current_action, -1);
    }

    #[test]
    fn the_heal_conversion_and_repair_orders_give_the_unit_s_commands() {
        for (order, action, command) in [
            (
                crate::ai_kinds::ORDER_HEAL,
                HEAL_ACTION,
                UnitCommand::Heal(70),
            ),
            (
                crate::ai_kinds::ORDER_CONVERT,
                CONVERT_ACTION,
                UnitCommand::Convert(70),
            ),
            (REPAIR_ORDER, REPAIR_ACTION, UnitCommand::Repair(70)),
        ] {
            let mut side = OrderSide::holding(&[(70, 3)]);
            let mut u = UnitAi {
                current_target: 5,
                target_type: 9,
                ..idle(PRIEST_AI)
            };
            let (r, fx, _) = run_order(&mut u, queued(order, 70), 18, &mut side);
            assert_eq!((r, fx), (Ok(1), Vec::new()));
            assert_eq!(side.commands, [command]);
            assert_eq!((u.current_order, u.current_action), (order, action));
            assert_eq!(
                (u.current_target, u.target_type, u.target_point),
                (70, 3, [10.0, 20.0, 0.0])
            );
            assert_eq!((u.last_target, u.last_target_type), (5, 9));
            // refused: nothing changes
            let mut u = idle(PRIEST_AI);
            side.refuses = true;
            assert_eq!(run_order(&mut u, queued(order, 70), 18, &mut side).0, Ok(0));
            assert_eq!(u, idle(PRIEST_AI));
        }
        // the unit's commands the slice models on no unit
        let mut u = idle(PRIEST_AI);
        let o = queued(crate::ai_kinds::ORDER_HEAL, 70);
        let at = Surroundings::default();
        let mut tail = TailSide::default();
        assert!(
            u.process_order(
                &o,
                Some((&at, &mut tail, false)),
                None,
                &mut rng(),
                &mut Vec::new()
            )
            .is_err()
        );
    }

    #[test]
    fn the_explore_orders_explore_their_point_cut_to_tiles() {
        let mut side = OrderSide::default();
        for (order, action, last) in [
            (crate::ai_kinds::ORDER_EXPLORE, EXPLORE_ACTION, -1),
            (crate::ai_kinds::ORDER_SCOUT, SCOUT_ACTION, EXPLORE_ACTION),
            (AUTO_SCOUT_ORDER, AUTO_SCOUT_ACTION, EXPLORE_ACTION),
        ] {
            let mut u = idle(SOLDIER_AI);
            let (r, fx, _) = run_order(&mut u, queued(order, -1), 12, &mut side);
            assert_eq!((r, fx), (Ok(1), vec![Effect::Explore([12.0, 30.0])]));
            assert_eq!(
                (u.current_order, u.current_action, u.last_action),
                (order, action, last)
            );
            assert_eq!(u.target_point, [12.0, 30.0, EXPLORE_Z]);
        }
        // a point that is not a number cuts to i32::MIN (CVTTSS2SI)
        let mut u = idle(SOLDIER_AI);
        let o = QueuedOrder {
            point: [f32::NAN, 1.5, 0.0],
            ..queued(crate::ai_kinds::ORDER_EXPLORE, -1)
        };
        let (_, fx, _) = run_order(&mut u, o, 12, &mut side);
        assert_eq!(fx, vec![Effect::Explore([i32::MIN as f32, 1.0])]);
    }

    #[test]
    fn the_stop_order_stops_and_ends_the_order() {
        let mut side = OrderSide::default();
        let mut u = UnitAi {
            current_order: MOVE_ORDER,
            order_priority: 100,
            current_action: MOVE_ACTION,
            current_target: 70,
            ..idle(SOLDIER_AI)
        };
        let (r, fx, _) = run_order(&mut u, queued(STOP_ORDER, -1), 6, &mut side);
        assert_eq!((r, fx), (Ok(1), vec![Effect::Stop]));
        assert_eq!(
            (
                u.current_order,
                u.order_priority,
                u.current_action,
                u.current_target
            ),
            (-1, -1, -1, -1)
        );
        assert_eq!((u.last_order, u.last_target), (MOVE_ORDER, 70));
        assert_eq!(u.state_position, [5.0, 6.0]);
    }

    #[test]
    fn the_run_away_order_runs_from_the_unit_s_attackers() {
        // no attackers: a random direction (two draws), by twice the speed (0 here: the unit's
        // own place), through evasiveMoveTo
        let mut side = OrderSide::default();
        let mut u = idle(SOLDIER_AI);
        let (r, fx, draws) = run_order(&mut u, queued(RUN_AWAY_ORDER, -1), 6, &mut side);
        assert_eq!(r, Ok(1));
        assert_eq!(draws, 2);
        assert_eq!(
            fx,
            vec![Effect::Move(AiMove {
                point: [5.0, 6.0, 0.25],
                range: 0.0,
                force: true,
                walk: false,
            })]
        );
        assert_eq!(
            (u.current_order, u.current_action),
            (RUN_AWAY_ORDER, MOVE_ACTION)
        );
        // standing its ground, a player's unit stays: not taken
        let mut u = UnitAi {
            stance: 2,
            ..idle(SOLDIER_AI)
        };
        assert_eq!(
            run_order(&mut u, queued(RUN_AWAY_ORDER, -1), 6, &mut side).0,
            Ok(0)
        );
    }

    #[test]
    fn the_gather_and_hunt_orders_work_an_object_the_world_holds() {
        let mut side = OrderSide::holding(&[(70, 15)]);
        let mut u = idle(CIVILIAN);
        let gather = crate::ai_kinds::ORDER_GATHER;
        let (r, fx, _) = run_order(&mut u, queued(gather, 70), 4, &mut side);
        assert_eq!((r, fx), (Ok(1), vec![Effect::GatherObject(70)]));
        assert_eq!(
            (
                u.current_order,
                u.current_action,
                u.current_target,
                u.target_type
            ),
            (gather, GATHER_ACTION, 70, 15)
        );
        // gone: the gather command takes nothing
        let mut u = idle(CIVILIAN);
        let (r, fx, _) = run_order(&mut u, queued(gather, 99), 4, &mut side);
        assert_eq!((r, fx), (Ok(0), Vec::new()));
        // the hunt stops first, then hunts; on a gone target the stop stays
        let mut u = UnitAi {
            current_action: GATHER_ACTION,
            ..idle(CIVILIAN)
        };
        let (r, fx, _) = run_order(&mut u, queued(HUNT_ORDER, 70), 4, &mut side);
        assert_eq!((r, fx), (Ok(1), vec![Effect::HuntObject(70)]));
        assert_eq!(
            (u.current_order, u.current_action),
            (HUNT_ORDER, HUNT_ACTION)
        );
        let mut u = UnitAi {
            current_action: GATHER_ACTION,
            ..idle(CIVILIAN)
        };
        let (r, fx, _) = run_order(&mut u, queued(HUNT_ORDER, 99), 4, &mut side);
        assert_eq!((r, fx), (Ok(0), vec![Effect::Stop]));
        assert_eq!((u.current_order, u.current_action), (-1, -1));
        // the auto boar luring hunts, then holds 0x2be either way
        let luring = crate::ai_kinds::ORDER_AUTO_BOAR_LURING;
        let mut u = idle(CIVILIAN);
        let (r, _, _) = run_order(&mut u, queued(luring, 70), 4, &mut side);
        assert_eq!(r, Ok(1));
        assert_eq!(
            (u.current_order, u.current_action, u.last_action),
            (luring, BOAR_LURING_ACTION, HUNT_ACTION)
        );
        let mut u = idle(CIVILIAN);
        let (r, _, _) = run_order(&mut u, queued(luring, 99), 4, &mut side);
        assert_eq!(
            (r, u.current_order, u.current_action),
            (Ok(0), -1, BOAR_LURING_ACTION)
        );
    }

    #[test]
    fn the_move_orders_move_to_their_point_within_their_range() {
        let mut side = OrderSide::default();
        for order in [MOVE_ORDER, crate::ai_kinds::ORDER_GROUP_GATHER] {
            let mut u = idle(SOLDIER_AI);
            let (r, fx, _) = run_order(&mut u, queued(order, -1), 6, &mut side);
            assert_eq!(
                (r, fx),
                (
                    Ok(1),
                    vec![Effect::Move(AiMove {
                        point: [12.7, 30.2, 0.5],
                        range: 3.0,
                        force: true,
                        walk: true,
                    })]
                )
            );
            assert_eq!((u.current_order, u.current_action), (order, MOVE_ACTION));
        }
    }

    #[test]
    fn the_follow_order_follows_and_moves_to_a_target_beyond_its_range() {
        // the gap (1) within the range squared (9): no move
        let mut side = OrderSide::holding(&[(70, 6)]);
        let mut u = idle(SOLDIER_AI);
        let (r, fx, _) = run_order(&mut u, queued(FOLLOW_ORDER, 70), 6, &mut side);
        assert_eq!((r, fx), (Ok(1), Vec::new()));
        assert_eq!(
            (
                u.current_order,
                u.current_action,
                u.current_target,
                u.desired_distance
            ),
            (FOLLOW_ORDER, FOLLOW_ACTION, 70, 3.0)
        );
        // beyond it: the move to the object (slot 0x138)
        side.world[0].gap_squared = 9.5;
        let mut u = idle(SOLDIER_AI);
        let (r, fx, _) = run_order(&mut u, queued(FOLLOW_ORDER, 70), 6, &mut side);
        assert_eq!(r, Ok(1));
        assert_eq!(
            fx,
            vec![Effect::MoveToObject {
                target: 70,
                state: 2,
                range: 3.0,
            }]
        );
        assert_eq!(
            (u.current_action, u.last_action),
            (MOVE_ACTION, FOLLOW_ACTION)
        );
        // a gap that is not a number stays (COMISS, JBE); a gone target is not taken
        side.world[0].gap_squared = f32::NAN;
        let mut u = idle(SOLDIER_AI);
        assert_eq!(
            run_order(&mut u, queued(FOLLOW_ORDER, 70), 6, &mut side).1,
            Vec::new()
        );
        let mut u = idle(SOLDIER_AI);
        assert_eq!(
            run_order(&mut u, queued(FOLLOW_ORDER, 99), 6, &mut side).0,
            Ok(0)
        );
    }

    #[test]
    fn the_transport_enter_and_unload_orders_give_the_unit_s_commands() {
        let mut side = OrderSide::holding(&[(70, 3)]);
        let transport = crate::ai_kinds::ORDER_TRANSPORT;
        let mut u = idle(11);
        let (r, _, _) = run_order(&mut u, queued(transport, -1), 20, &mut side);
        assert_eq!(r, Ok(1));
        assert_eq!(side.commands, [UnitCommand::Transport([12.7, 30.2, 0.5])]);
        assert_eq!(
            (
                u.current_order,
                u.current_action,
                u.current_target,
                u.target_point
            ),
            (transport, TRANSPORT_ACTION, -1, [12.7, 30.2, 0.5])
        );
        // the enter: the target at (-1, -1, -1) with its class
        let mut side = OrderSide::holding(&[(70, 3)]);
        let enter = crate::garrison::ENTER_ORDER;
        let mut u = idle(SOLDIER_AI);
        let (r, _, _) = run_order(&mut u, queued(enter, 70), 6, &mut side);
        assert_eq!(r, Ok(1));
        assert_eq!(side.commands, [UnitCommand::Enter(70)]);
        assert_eq!(
            (
                u.current_action,
                u.current_target,
                u.target_type,
                u.target_point
            ),
            (crate::garrison::ENTER_ACTION, 70, 3, [-1.0; 3])
        );
        // a villager takes no enter into an object the world does not hold; others ask
        let mut u = idle(CIVILIAN);
        assert_eq!(run_order(&mut u, queued(enter, 99), 4, &mut side).0, Ok(0));
        assert_eq!(side.commands.len(), 1);
        // (FUN_140c64c10 keeps the class it held for an object the world does not hold)
        let mut u = UnitAi {
            target_type: 12,
            ..idle(SOLDIER_AI)
        };
        assert_eq!(run_order(&mut u, queued(enter, 99), 6, &mut side).0, Ok(1));
        assert_eq!((u.current_target, u.target_type), (99, 12));
        // the unloads: the point at the unit's height; the unit itself the target
        for (order, over_wall, action) in [
            (crate::ai_kinds::ORDER_UNLOAD, false, UNLOAD_ACTION),
            (
                crate::ai_kinds::ORDER_UNLOAD_OVER_WALL,
                true,
                UNLOAD_OVER_WALL_ACTION,
            ),
        ] {
            let mut side = OrderSide::default();
            let mut u = idle(11);
            let (r, _, _) = run_order(&mut u, queued(order, -1), 20, &mut side);
            assert_eq!(r, Ok(1));
            assert_eq!(
                side.commands,
                [UnitCommand::Unload {
                    to: [12.7, 30.2, 0.25],
                    over_wall,
                }]
            );
            assert_eq!(
                (
                    u.current_order,
                    u.current_action,
                    u.current_target,
                    u.target_type
                ),
                (order, action, 9, 20)
            );
            assert_eq!(u.target_point, [5.0, 6.0, 0.25]);
        }
    }

    #[test]
    fn seek_and_destroy_attacks_and_explore_and_destroy_explores_at_tiles() {
        let mut side = OrderSide::holding(&[(70, 6)]);
        let seek = crate::ai_kinds::ORDER_SEEK_AND_DESTROY;
        let mut u = idle(SOLDIER_AI);
        let (r, fx, _) = run_order(&mut u, queued(seek, 70), 6, &mut side);
        assert_eq!((r, fx), (Ok(1), Vec::new()));
        assert_eq!(side.attacks, [(70, true)]);
        assert_eq!(
            (
                u.current_order,
                u.current_action,
                u.current_target,
                u.target_type
            ),
            (seek, SEEK_ACTION, 70, 6)
        );
        assert_eq!(u.target_point, [12.0, 30.0, EXPLORE_Z]);
        let destroy = crate::ai_kinds::ORDER_EXPLORE_AND_DESTROY;
        let mut u = idle(SOLDIER_AI);
        let (r, fx, _) = run_order(&mut u, queued(destroy, -1), 6, &mut side);
        assert_eq!((r, fx), (Ok(1), vec![Effect::Explore([12.0, 30.0])]));
        assert_eq!(
            (
                u.current_order,
                u.current_action,
                u.current_target,
                u.target_point
            ),
            (destroy, EXPLORE_DESTROY_ACTION, -1, [12.0, 30.0, EXPLORE_Z])
        );
    }

    #[test]
    fn the_wait_order_waits_and_leaves_a_patrol_order() {
        // case 0x2d8: the action value 0x274 (the last stays), the patrol order's timers, then
        // the order; an adjusted idle timeout of 750 is redrawn (one draw)
        let mut side = OrderSide::default();
        let mut u = UnitAi {
            current_order: PATROL,
            current_action: MOVE_ACTION,
            last_action: 5,
            adjusted_idle_timeout: 750,
            idle_timeout: 3000,
            secondary_timer: 900,
            ..idle(SOLDIER_AI)
        };
        let wait = crate::ai_kinds::ORDER_WAIT;
        let (r, fx, draws) = run_order(&mut u, queued(wait, -1), 6, &mut side);
        assert_eq!((r, fx, draws), (Ok(1), Vec::new(), 1));
        assert_eq!(
            (
                u.current_order,
                u.order_priority,
                u.current_action,
                u.last_action
            ),
            (wait, 40, WAIT_ACTION, 5)
        );
        assert_eq!(u.secondary_timer, 750);
        assert_ne!(u.adjusted_idle_timeout, 750);
    }

    #[test]
    fn the_attack_roundup_passes_its_own_target_and_one_it_cannot_reach() {
        let mut side = OrderSide::holding(&[(70, 6)]);
        let roundup = crate::ai_kinds::ORDER_ATTACK_ROUNDUP;
        let mut u = UnitAi {
            current_target: 70,
            ..idle(SOLDIER_AI)
        };
        assert_eq!(
            run_order(&mut u, queued(roundup, 70), 6, &mut side).0,
            Ok(0)
        );
        // with an action value it must reach the target
        let mut u = UnitAi {
            current_action: MOVE_ACTION,
            ..idle(SOLDIER_AI)
        };
        assert_eq!(
            run_order(&mut u, queued(roundup, 70), 6, &mut side).0,
            Ok(0)
        );
        assert_eq!(side.attacks, Vec::new());
        // without one it attacks, and the order becomes 700, leaving a patrol order
        let mut u = UnitAi {
            current_order: PATROL,
            secondary_timer: 900,
            ..idle(SOLDIER_AI)
        };
        assert_eq!(
            run_order(&mut u, queued(roundup, 70), 6, &mut side).0,
            Ok(1)
        );
        assert_eq!(
            (u.current_order, u.current_action, u.secondary_timer),
            (ATTACK_ORDER, ATTACK_ACTION, 750)
        );
    }

    #[test]
    fn the_order_on_a_target_goes_to_the_caller_or_moves_to_a_gone_one_s_point() {
        let mut side = OrderSide::holding(&[(70, 6)]);
        let o = queued(ORDER_ON_TARGET, 70);
        let mut u = idle(CIVILIAN);
        let (r, fx, _) = run_order(&mut u, o, 4, &mut side);
        assert_eq!((r, fx), (Ok(1), vec![Effect::OrderOnTarget(o)]));
        assert_eq!(u.current_order, -1);
        // gone, without its type: the move to the point, and the order on a target the AI's
        for order in [ORDER_ON_TARGET, ORDER_ON_TARGET_2] {
            let mut u = idle(CIVILIAN);
            let (r, fx, _) = run_order(&mut u, queued(order, 99), 4, &mut side);
            assert_eq!(r, Ok(1));
            assert_eq!(
                fx,
                vec![Effect::Move(AiMove {
                    point: [12.7, 30.2, 0.5],
                    range: 3.0,
                    force: true,
                    walk: true,
                })]
            );
            assert_eq!((u.current_order, u.order_priority), (order, 40));
        }
    }

    #[test]
    fn a_gone_target_of_a_type_sends_the_unit_to_the_closest_resource_of_its_kind() {
        // the order's type a class (no owner): a tree's (15, wood: kind 0, fewer than 2
        // workers), searched from the order's shown point at any state; the unit works what it
        // finds (slot 0x7e8) and the order is not taken
        let tree = Gatherable {
            id: 80,
            position: [13.5, 31.5, 0.0],
            class: 15,
        };
        let mut side = OrderSide {
            resource: Some(tree),
            ..OrderSide::default()
        };
        let typed = QueuedOrder {
            target_type: 15,
            ..queued(ORDER_ON_TARGET, 99)
        };
        let mut u = UnitAi {
            current_target: 7,
            ..idle(CIVILIAN)
        };
        let before = u.clone();
        let (r, fx, _) = run_order(&mut u, typed, 4, &mut side);
        assert_eq!(
            (r, fx),
            (
                Ok(0),
                vec![Effect::WorkOn {
                    target: 80,
                    point: [13.5, 31.5, 0.0],
                }]
            )
        );
        assert_eq!(u, before);
        assert_eq!(
            side.searches,
            [ResourceSearch {
                kind: 0,
                cap: 2,
                site: -1,
                current: 7,
                point: [12.5, 30.5],
                min_state: 0,
            }]
        );
        // with an owner the type is a master of that player's: a gold mine's class (32: kind
        // 4, any workers); a master the player does not hold, or nothing found, gives nothing
        let mut side = OrderSide {
            resource: Some(tree),
            classes: vec![((2, 66), 32)],
            ..OrderSide::default()
        };
        let owned = |master| QueuedOrder {
            target_type: master,
            target_owner: 2,
            ..queued(ORDER_ON_TARGET, 99)
        };
        let mut u = idle(CIVILIAN);
        assert_eq!(run_order(&mut u, owned(66), 4, &mut side).0, Ok(0));
        assert_eq!((side.searches[0].kind, side.searches[0].cap), (4, -1));
        let (r, fx, _) = run_order(&mut u, owned(67), 4, &mut side);
        assert_eq!((r, fx, side.searches.len()), (Ok(0), Vec::new(), 1));
        side.resource = None;
        let (r, fx, _) = run_order(&mut u, owned(66), 4, &mut side);
        assert_eq!((r, fx, side.searches.len()), (Ok(0), Vec::new(), 2));
        // a class of no kind (a soldier's 6), or a unit AI type whose slot 0x1b0 gives none
        let mut side = OrderSide::default();
        let soldier_type = QueuedOrder {
            target_type: 6,
            ..queued(ORDER_ON_TARGET, 99)
        };
        let mut u = idle(CIVILIAN);
        assert_eq!(run_order(&mut u, soldier_type, 4, &mut side).0, Ok(0));
        let mut u = idle(SOLDIER_AI);
        assert_eq!(run_order(&mut u, typed, 6, &mut side).0, Ok(0));
        assert_eq!(side.searches, Vec::new());
    }

    #[test]
    fn slot_0x1b0_gives_the_kind_of_resource_by_the_unit_ai_s_type() {
        let kind = |ai_type, class| idle(ai_type).resource_kind(class);
        for ai_type in [CIVILIAN, 17] {
            assert_eq!(
                [15, 7, 5, 31, 33, 8, 32, 63, 6].map(|c| kind(ai_type, c)),
                [0, 1, 2, 2, 2, 3, 4, 4, -1]
            );
        }
        assert_eq!(
            [5, 31, 33, 63, 15, 32, 34].map(|c| kind(FISHING_AI, c)),
            [2, 2, 2, 4, -1, -1, -1]
        );
        assert_eq!([15, 5, 63].map(|c| kind(SOLDIER_AI, c)), [-1, -1, -1]);
    }

    #[test]
    fn the_attack_move_attacks_with_a_trebuchet_and_moves_otherwise() {
        let mut side = OrderSide::holding(&[(70, 3)]);
        for class in [TREBUCHET_CLASS, TREBUCHET_PACKED_CLASS] {
            let mut u = idle(SIEGE_AI);
            let (r, fx, _) = run_order(&mut u, queued(ATTACK_MOVE, 70), class, &mut side);
            assert_eq!((r, fx), (Ok(1), Vec::new()));
            assert_eq!(
                (u.current_order, u.current_action),
                (ATTACK_ORDER, ATTACK_ACTION)
            );
        }
        for (class, target) in [(6, 70), (TREBUCHET_CLASS, -1)] {
            let mut u = idle(SOLDIER_AI);
            let (r, fx, _) = run_order(&mut u, queued(ATTACK_MOVE, target), class, &mut side);
            assert_eq!(r, Ok(1));
            assert!(matches!(fx[..], [Effect::Move(_)]));
            assert_eq!(
                (u.current_order, u.current_action),
                (ATTACK_MOVE, MOVE_ACTION)
            );
        }
    }

    #[test]
    fn the_order_tail_takes_the_order_and_leaves_a_patrol_order() {
        let patrolling = |order| UnitAi {
            current_order: order,
            order_priority: 7,
            adjusted_idle_timeout: 750,
            idle_timeout: 3000,
            secondary_timer: 2000,
            current_target: 70,
            ..idle(SOLDIER_AI)
        };
        // leaving a patrol order for another redraws a timeout of exactly 750 and caps the
        // secondary timer
        let mut u = patrolling(PATROL);
        let mut r = rng();
        assert_eq!(u.order_tail(MOVE_ORDER, 40, 6, false, &mut r), 1);
        assert_eq!(
            (
                u.current_order,
                u.order_priority,
                u.secondary_timer,
                r.draws
            ),
            (MOVE_ORDER, 40, 750, 1)
        );
        // staying on it does nothing to the timers
        let mut u = patrolling(ATTACK_MOVE);
        let mut r = rng();
        let _ = u.order_tail(ATTACK_MOVE, 40, 6, false, &mut r);
        assert_eq!(
            (u.secondary_timer, u.adjusted_idle_timeout, r.draws),
            (2000, 750, 0)
        );
        // the stop ends the order and its priority
        let mut u = patrolling(PATROL);
        let _ = u.order_tail(STOP_ORDER, 40, 6, false, &mut rng());
        assert_eq!(
            (u.current_order, u.order_priority, u.secondary_timer),
            (-1, -1, 750)
        );
        // the attack move's attack and the roundup become the attack order
        let mut u = patrolling(ATTACK_MOVE);
        let _ = u.order_tail(ATTACK_MOVE, 40, 51, true, &mut rng());
        assert_eq!((u.current_order, u.secondary_timer), (ATTACK_ORDER, 750));
        // a class-18 unit's attack order: the target goes, the order is -1
        let mut u = patrolling(-1);
        let _ = u.order_tail(ATTACK_ORDER, 40, CLASS_18, false, &mut rng());
        assert_eq!(
            (u.current_order, u.order_priority, u.current_target),
            (-1, 40, -1)
        );
    }

    #[test]
    fn the_attack_command_runs_a_pending_stop_first() {
        // the attack command runs at once: each stop pending runs first (the second on an empty
        // list with its standing sprite shown already); a stop with another command pending is
        // not modelled
        let mut u = idle(BOAR_AI);
        let mut side = OrderSide::holding(&[(70, 6)]);
        let at = Surroundings::default();
        let mut fx = vec![Effect::Stop, Effect::Stop];
        assert_eq!(
            u.attack_command(70, true, &at, &mut side, &mut fx),
            Ok(true)
        );
        assert_eq!((fx, side.stops, u.commanded), (Vec::new(), 2, true));
        let mut fx = vec![Effect::Stop, Effect::GatherObject(1)];
        assert!(u.attack_command(70, true, &at, &mut side, &mut fx).is_err());
    }

    /// The villager's handler (`FUN_140c22bf0`) on the attack action (600): a moved target
    /// (0x1fc) with `+0x1a8` set posts the Attack's failure to itself (the unit's slot 0x328:
    /// 0x1f9 with 600, 0, 0), else an invalidation, a moved target or a search goes to the base
    /// handler (a moved target without a saved patrol: nothing).
    #[test]
    fn a_villager_s_attack_target_that_moved_is_the_base_handler_s_or_its_failure() {
        let attacking = |stop_after_kill| UnitAi {
            current_order: ATTACK_ORDER,
            current_action: ATTACK_ACTION,
            current_target: 18842,
            target_type: 47,
            stop_after_kill,
            ..idle(CIVILIAN)
        };
        let at = Surroundings {
            id: 6116,
            state: 2,
            ..Surroundings::default()
        };
        let moved = Notify::own(6116, TARGET_MOVED, [0, 0, 0]);
        let mut u = attacking(true);
        let (r, fx, _) = handled(&mut u, moved, &at, false);
        assert_eq!((r, fx), (Ok(GO_ON), Vec::new()));
        assert_eq!(
            u.notifies,
            [Notify::own(6116, ACTION_FAILED, [ATTACK_ACTION, 0, 0])]
        );
        let mut u = attacking(false);
        let (r, fx, _) = handled(&mut u, moved, &at, false);
        assert_eq!((r, fx), (Ok(GO_ON), Vec::new()));
        assert_eq!(u.notifies, Vec::new());
        assert_eq!(u.current_action, ATTACK_ACTION);
        // the base handler's invalidation of the current action stops it
        let mut u = attacking(false);
        let invalid = Notify::own(6116, ACTION_INVALIDATED, [ATTACK_ACTION, 0, 0]);
        let (r, _, _) = handled(&mut u, invalid, &at, false);
        assert!(r.is_ok());
        assert_eq!(u.current_action, -1);
    }

    /// The villager's case 0x21c (`FUN_140c22bf0`), a load of another resource asked off: the
    /// target of its top action (slot 0x190), then the drop-site table's building
    /// (`FUN_141658e20`), which the unit works and whose way back the caller queues; without a
    /// building nothing. The processing goes on.
    #[test]
    fn a_villager_drops_another_resource_at_the_table_s_building() {
        let at = Surroundings {
            id: 20,
            state: 2,
            ..Surroundings::default()
        };
        let other = Notify::own(20, OTHER_LOAD, [0; 3]);
        let mut side = Recorder {
            drop_site: Some(45),
            top: Some(77),
            ..Recorder::default()
        };
        let mut fx = Vec::new();
        let r = idle(CIVILIAN).handle(other, 0, &at, false, &mut rng(), &mut side, &mut fx);
        assert_eq!(r, Ok(GO_ON));
        assert_eq!(
            fx,
            [Effect::DropElsewhere {
                site: 45,
                back: Some(77)
            }]
        );
        let mut none = Recorder::default();
        let mut fx = Vec::new();
        let r = idle(CIVILIAN).handle(other, 0, &at, false, &mut rng(), &mut none, &mut fx);
        assert_eq!((r, fx), (Ok(GO_ON), Vec::new()));
    }

    /// The villager's case 0x21d (`FUN_140c22bf0`): off the build or the hunt action the object
    /// its action made (429381190's villager 15295 and its pasture 15342 in update 1386), which
    /// the world holds, the unit works at its place (slot 0x7e8); off another action nothing.
    /// The processing goes on.
    #[test]
    fn a_villager_works_the_pasture_it_built() {
        let pasture = UnitSeen {
            id: 15342,
            position: [136.0, 72.0, 0.0],
            ..UnitSeen::default()
        };
        let at = Surroundings {
            id: 15295,
            state: 2,
            units: vec![pasture],
            ..Surroundings::default()
        };
        for action in [BUILD_ACTION, HUNT_ACTION] {
            let built = Notify::own(15295, PASTURE_BUILT, [action, 15342, 0]);
            let (r, fx, _) = handled(&mut idle(CIVILIAN), built, &at, false);
            assert_eq!(r, Ok(GO_ON));
            assert_eq!(
                fx,
                [Effect::WorkOn {
                    target: 15342,
                    point: [136.0, 72.0, 0.0]
                }]
            );
        }
        let other = Notify::own(15295, PASTURE_BUILT, [GATHER_ACTION, 15342, 0]);
        assert_eq!(
            handled(&mut idle(CIVILIAN), other, &at, false).1,
            Vec::new()
        );
    }

    /// The base handler's completion (`FUN_140c589c0` case 0x1fa; the villager's own case takes
    /// it before) of an attack on the attack order 700: processIdle with 0; when it produced an action the processing breaks (0xd),
    /// else the target goes and slot 0xb0 with 0 ends the order. A unit without an order in a
    /// group with a command keeps its place as its state position.
    #[test]
    fn an_attack_completed_on_the_attack_order_looks_for_the_next_or_ends_it() {
        let done = Notify::own(9, ACTION_COMPLETED, [ATTACK_ACTION, 0, 0]);
        // a transport ship's processIdle only caps its last human order time (5)
        let mut u = UnitAi {
            current_order: ATTACK_ORDER,
            order_priority: 100,
            current_action: ATTACK_ACTION,
            current_target: 70,
            ..idle(11)
        };
        let here = Surroundings {
            position: [5.0, 6.0, 0.0],
            class: 20,
            ..Surroundings::default()
        };
        let (r, fx, _) = handled(&mut u, done, &here, false);
        assert_eq!(r, Ok(GO_ON));
        assert_eq!(fx, [Effect::CapHumanOrder(HUMAN_ORDER_CAP), Effect::Stop]);
        assert_eq!(
            (u.current_order, u.order_priority, u.last_order),
            (-1, -1, ATTACK_ORDER)
        );
        assert_eq!(
            (u.current_target, u.last_target, u.current_action),
            (-1, 70, -1)
        );
        assert_eq!(u.state_position, [5.0, 6.0]);
        // a boar beyond its leash heads home (6): the processing breaks there
        let mut u = UnitAi {
            current_order: ATTACK_ORDER,
            current_action: ATTACK_ACTION,
            current_target: 70,
            ..boar()
        };
        let mut fx = Vec::new();
        let r = u.handle(
            done,
            0,
            &at([43.5, 105.5, 0.0]),
            false,
            &mut seed_for(false),
            &mut Recorder::default(),
            &mut fx,
        );
        assert_eq!(r, Ok(PRODUCED));
        assert!(matches!(
            fx[..],
            [Effect::CapHumanOrder(HUMAN_ORDER_CAP), Effect::Move(_)]
        ));
        // (its move forgot the target: FUN_140c64c10 with -1)
        assert_eq!(
            (u.current_order, u.current_target, u.last_target),
            (ATTACK_ORDER, -1, 70)
        );
        // without an order, in a group with a command: the state position, then the stop
        let mut u = UnitAi {
            current_action: ATTACK_ACTION,
            current_target: 70,
            ..idle(SOLDIER_AI)
        };
        let grouped = Surroundings {
            group: Some(GroupState {
                commanded: true,
                ..GroupState::default()
            }),
            ..here.clone()
        };
        let (r, fx, _) = handled(&mut u, done, &grouped, false);
        assert_eq!((r, fx), (Ok(GO_ON), vec![Effect::Stop]));
        assert_eq!(u.state_position, [5.0, 6.0]);
        let mut u = UnitAi {
            current_action: ATTACK_ACTION,
            current_target: 70,
            ..idle(SOLDIER_AI)
        };
        let _ = handled(&mut u, done, &here, false);
        assert_eq!(u.state_position, [0.0, 0.0]);
    }
}
