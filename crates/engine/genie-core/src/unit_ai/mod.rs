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

mod actions;
mod attack_orders;
mod attack_pick;
mod attack_rules;
mod attacked;
mod cared;
mod damaged;
mod flight;
mod handle;
mod idle;
mod idle_animals;
mod ids;
mod life;
mod notifies;
mod order_kinds;
mod process_order;
mod retarget;
mod side;
mod surroundings;
mod update;

pub(crate) use self::attack_pick::spares_friends;
pub use self::attack_pick::{
    AttackAsk, AttackPicker, Attacker, Memory, NoAttack, Pick, Search, Seeker, Target, can_attack,
    soldier_can_attack, tower_can_attack,
};
use self::attack_pick::{DEFEND_GROWTH, PRIORITY_3_CLASSES, point_gap_squared};
pub(crate) use self::attack_rules::patrol_distance;
pub use self::attack_rules::{
    Attacked, Convertee, Converter, attack_sight, attacks_neutral, can_convert,
    priest_attacks_neutral, siege_attacks_neutral, siege_can_attack, soldier_attacks_neutral,
    type_attacks_neutral,
};
use self::attack_rules::{
    DEFEND_PATROL_LEASH, PACKED_UNIT_CLASS, STAND_GROUND_PATROL_LEASH, back_up,
    civilian_may_attack, computer_idle_check, defend_patrol_range, lion_may_attack,
};
#[cfg(test)]
use self::attack_rules::{PATROL_FARTHEST, line_distance};
#[cfg(test)]
pub(crate) use self::cared::cared_classes;
pub use self::cared::{
    ARTIFACT_AI, ARTIFACT_CARED, BUILDING_CARED, CIVILIAN, CIVILIAN_CARED, CIVILIAN_SEARCH_GROWTH,
    FISHING_CARED, HUNTED_CARED, LION_CARED, PREDATOR_CARED, PRIEST_CARED, SIEGE_CARED,
    SIEGE_TOWER_CARED, SOLDIER_CARED, SOLDIER_CARED_COMPUTER, TOWER_CARED, TRADE_CART_CARED,
    TRADE_SHIP_CARED, TRANSPORT_CARED, civilian_kind, civilian_search_radius,
};
use self::cared::{
    BUILDING_AI, CLASS_22, EVADE_EDGE, EVADE_REACH, EVASIVE_NEAR, LIVE_TARGET_ACTIONS,
    RUN_DRAW_SCALE, RUN_EPSILON, RUN_LEASH, RUN_ONE, RUN_REACH, TRANSPORT_AI,
    adjusted_idle_timeout, attack_search_radius,
};
pub(crate) use self::cared::{search_area, search_radius};
#[cfg(test)]
use self::ids::DEFENSIVE_MASTERS;
use self::ids::{
    AI_MOVE_RANGE, ARTIFACT_IDLE_TIMEOUT, ATTACK_ACTION, ATTACK_ORDER, BOAR_HOME, BOAR_LEASH,
    BOAR_LEASH_DRAW, BOAR_LURING_ACTION, BOAR_OFFSET, BOAR_STEP, BOAR_WANDER, CLASS_KING,
    CLASS_VILLAGER, COMPUTER_SOLDIER_IDLE_TIMEOUT, CONVERT_ACTION, DEFEND_ACTION, DEFEND_SLACK,
    DEFENDLESS_MASTER, DESIRED_DISTANCE, ENDED_ACTIONS, EXPLORE_DESTROY_ACTION, EXPLORE_Z,
    FOLLOW_SLACK_HIGH, FOLLOW_SLACK_LOW, FOUNDATION_REACH, HEAL_ACTION, HUNTED_HOME,
    HUNTED_IDLE_TIMEOUT, HUNTED_SIGHT_GROWTH, HUNTED_STEP, IDLE_TIMEOUT, INVALID_POINT, KING_UNLED,
    LION_ATTACKED_TIMEOUT, LION_IDLE_TIMEOUT, LION_SIGHT_DIVISOR, LION_SIGHT_SHARE, NO_ANIM_SPEED,
    OUTSIDE_ORDER, PRIEST_IDLE_TIMEOUT, REPAIR_ACTION, REPAIR_ORDER, RETARGET_RANGE,
    RETARGET_REACH, RUN_AWAY_ORDER, Retarget, RetargetSearch, SCOUT_ACTION, SEEK_ACTION,
    SHEPHERD_ACTION, SIEGE_IDLE_TIMEOUT, SOLDIER_IDLE_MASTER, SOLDIER_IDLE_TIMEOUT,
    TOWER_IDLE_TIMEOUT, TRANSPORT_ACTION, TREBUCHET_CLASS, TREBUCHET_PACKED_CLASS, UNLOAD_ACTION,
    UNLOAD_OVER_WALL_ACTION, VILLAGER_NOTIFY, WAIT_ACTION, WALK_BACK, closest_undiscovered_tile,
    dodges, held_action_state, order_move, retarget_kind, siege_stance, soldier_stance,
};
pub use self::ids::{
    AUTO_SCOUT_ACTION, AUTO_SCOUT_ORDER, DEFEND_ORDER, EXPLORE_ACTION, FOLLOW_ACTION, FOLLOW_ORDER,
    HUMAN_ORDER_CAP, MOVE_ACTION, MOVE_ORDER, NewObject, STOP_ORDER, factory_type,
};
pub(crate) use self::ids::{ORDER_ON_TARGET_2, alert_owner, helper_answers};
pub(crate) use self::side::UNMODELLED_COMMAND;
pub use self::side::{
    Gatherable, NO_POINT, Named, OffFootprint, ResourceSearch, TargetFlags, UnitCommand, UnitSide,
};
pub(crate) use self::surroundings::{ATTACK_MOVE, PATROL};
pub use self::surroundings::{
    AiMove, Duel, Found, GroupState, Home, Seen, Surroundings, TopTarget, UnitSeen, flood_search,
    no_search,
};
use self::surroundings::{
    BUILDING_KIND, CLASS_3, CLASS_10, CLASS_18, CLASS_20, CLASS_52, COMBAT_KIND, LiveTop,
    PATROL_TURN_REACH, PendingTop, SINGLE_VOLLEY, changes_actions, group_lets_idle,
    top_target_after,
};
#[cfg(test)]
use crate::math::rng::Well512;
#[cfg(test)]
use crate::movement::PathAsk;

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
const STILL: f32 = crate::math::image::unit_ai::STILL;
/// The damage case's kill time of an object the unit cannot harm (`DAT_142efe41c`, 25000).
const NEVER_KILLED: f32 = crate::math::image::unit_ai::NEVER_KILLED;
/// The action value of a unit moving off a footprint with a group.
pub const GROUP_ASIDE_ACTION: i32 = 0x2c0;
/// The order on a foundation a villager sent off its footprint takes up again (slot 0x108,
/// `FUN_140c5c170` case 0x2de).
pub const FOUNDATION_ORDER: i32 = 0x2de;
/// The action value of a unit waiting for a foundation (slot 0x108, `FUN_140c68a60`, and the
/// order switch's case 0x2de); the full update's reset of ended actions leaves it.
pub const WAIT_SOLIDIFY_ACTION: i32 = 0x27a;
/// [`SENT_OFF`]'s radii are hundredths (`DAT_14330ebcc`).
const RADIUS_SCALE: f32 = crate::math::image::unit_ai::RADIUS_SCALE;
/// The prey animals' class, whose move off a footprint runs (slot 0x148's seventh argument).
const PREY_CLASS: i16 = 9;
/// The villagers' class.
const VILLAGER_CLASS: i16 = 4;

/// What a notify handler (unit-AI slot 0x238) returns: go on with the next notify.
const GO_ON: u32 = 2;
/// The defend stance's leash, squared (`DAT_142ed6f20`).
const DEFEND_LEASH: f32 = crate::math::image::unit_ai::DEFEND_LEASH;
/// What the stand ground stance's leash adds to the unit's range (`DAT_142eb880c`, 0.25, read by
/// `ADDSS` at `0x140c57c6a`).
const STAND_GROUND_SLACK: f32 = crate::math::image::unit_ai::STAND_GROUND_SLACK;
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
const SHIP_EVADE_REACH: f32 = crate::math::image::unit_ai::SHIP_EVADE_REACH;
/// The classes of the foundations a fishing ship's builder looks for (`FUN_140c25ca0`, in its
/// order): buildings, walls, gates, towers and farms.
const FISHING_BUILDS: [i32; 5] = [3, 27, 39, 52, 49];
/// The fishing ship's builder takes the nearest foundation nearer than this (`FUN_140c25ca0`'s
/// 9999).
const FISHING_BUILD_FAR: i32 = 9999;
/// The seconds without a sighting after which enemies found are the owner's news
/// (`DAT_142eb0f4c`).
const ENEMY_SIGHTING_GAP: f32 = crate::math::image::unit_ai::ENEMY_SIGHTING_GAP;

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
        /// ([`crate::movement::move_to::holds_object`]).
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
    pub path: crate::pathing::tile_path::Path,
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

/// The engine's error ([`genie_error::GenieError`]), here for the modules' imports.
pub use crate::GenieError;

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
mod tests;
