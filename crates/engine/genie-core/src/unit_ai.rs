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
    /// A building's or tower's command queue (`+0x1f0`..`+0x1f8`, 0x34-byte orders: the
    /// building command queue a gather point of several steps fills), its length.
    pub building_orders: usize,
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
/// `+0x924` 0 outside the scenario modes; a building's `desolid_flag` (`+0xc60`) is not held;
/// never masters 50 and 1187, nor from a tower in a state below 2. Class 10 with an attack
/// priority below 3 only of combat level 4 (`FUN_140fde120`); else when the tower is armed
/// (`FUN_14120ea30`: classes 51 and 54, or a missile; without one the object's `+0xa00` decides,
/// not held) and its attack priority is not 3 or the class one of [`PRIORITY_3_CLASSES`].
pub fn tower_can_attack(me: &Attacker, t: &Target) -> Result<bool, Unsupported> {
    if me.player == 0 || matches!(t.class, 1 | 42) {
        return Ok(false);
    }
    let gaia_combat = !me.scenario && 70 <= t.kind && t.capture == 0;
    if !gaia_combat && t.owner == 0 && !matches!(t.class, 9 | 10) {
        return Ok(false);
    }
    if t.kind == 80 {
        return Err(Unsupported("a tower's attack check of a building (+0xc60)"));
    }
    if TOWER_SPARED.contains(&t.master) || me.state <= 1 {
        return Ok(false);
    }
    if t.class == 10 && me.attack_priority < 3 {
        return Ok(t.combat_level == 4);
    }
    if !matches!(me.class, 51 | 54) && me.missile == -1 {
        return Err(Unsupported("an unarmed attacker's +0xa00 (FUN_14120ea30)"));
    }
    Ok(me.attack_priority != 3 || PRIORITY_3_CLASSES.contains(&t.class))
}

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
/// distance less the radius, at least 0).
fn point_gap_squared(point: [f32; 2], at: [f32; 3], radius: [f32; 2]) -> f32 {
    let gap = |d: f32, r: f32| {
        let g = d.abs() - r;
        if 0.0 <= g { g } else { 0.0 }
    };
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
/// those of the mask `0x200200800838001c` (2, 3, 4, 15, 16, 17, 21, 27, 35, 49, 53, 61) never.
const NEUTRAL_SPARED: u64 = 0x2002_0080_0838_001c;

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
        let class = t.class as u16;
        let spared = class < 0x3e && NEUTRAL_SPARED >> class & 1 != 0;
        if spared || (me.attack_priority == 3 && !PRIORITY_3_CLASSES.contains(&t.class)) {
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
    /// An override that never reaches the check: villagers (8), livestock (19).
    Never,
}

const fn retarget_kind(ai_type: u32) -> Result<Retarget, Unsupported> {
    match ai_type {
        // villagers (0x140c21da0) and livestock (0x140c21f20) return 9 from slot 73
        8 | 19 => Ok(Retarget::Never),
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
/// The class of the king's branch of the hunted animal's idle check (`-0xe71` raw).
const CLASS_KING: i16 = 59;

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
    let sight = at.los + HUNTED_SIGHT_GROWTH;
    if sight <= at.search_radius {
        at.search_radius
    } else {
        sight
    }
}

/// `UpdateAdjustedIdleTimeout` (`0x140c41e00`): `r % (base / 2) + 3 * base / 4`, one AI draw.
const fn adjusted_idle_timeout(base: u32, ai: &mut Well512) -> u32 {
    ai.rand15() % (base >> 1) + ((base * 3) >> 2)
}

/// The unit-AI type the factory (`FUN_141215a90`) gives a new object of a master of `class`
/// with `missile` (`+0x8d4`, −1 for none): none for classes 5, 7, 8, 11, 14, 15, 16, 30 to 34,
/// 38, 42, 48 and 63, the civilian's (8) for class 4, the building's (17) for classes 3, 27, 39,
/// 49 and 60 without a missile. Masters 1105, 1744 and 1746 have their own, and the other
/// classes' types are not modelled.
pub const fn factory_type(
    class: i16,
    master: i16,
    missile: i16,
) -> Result<Option<u32>, Unsupported> {
    if matches!(master, 1105 | 1744 | 1746) {
        return Err(Unsupported("a new object of a master with its own unit AI"));
    }
    match class {
        5 | 7 | 8 | 11 | 14 | 15 | 16 | 30..=34 | 38 | 42 | 48 | 63 => Ok(None),
        4 => Ok(Some(8)),
        3 | 27 | 39 | 49 | 60 if missile < 0 => Ok(Some(17)),
        _ => Err(Unsupported(
            "a new object whose unit-AI type is not modelled",
        )),
    }
}

impl UnitAi {
    /// The unit AI a new object gets. `UnitAIModule`'s constructor (`0x140c2e080`) draws three
    /// AI-stream values, in this order: the last retarget time `r % 500`, the adjusted idle timeout
    /// `r % 1500 + 2250` and the retarget timer `750 + r % 500`. The factory (`FUN_141215a90`)
    /// picks the type's constructor; the building's (17, `TribeBuildingUnitAIModule`
    /// `0x140c11470`) and the civilian's (8, `TribeCivilianUnitAIModule` `0x140c115d0`) are
    /// modelled, both of which set the stance (`+0x1a9`) to 0 after the base's 3; for any other
    /// type nothing is drawn.
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
            // the object's place
            state_position: [position[0], position[1]],
            retarget_disabled: 0,
            alert_mode: false,
            time_since_enemy: 0.0,
            patrol_path: false,
            stop_after_kill: false,
            home: None,
            hunted: None,
            last_update_time: 0,
            orders: Vec::new(),
            building_orders: 0,
            notifies: Vec::new(),
            notify_capacity: notify_capacity(ai_type, 0),
            processing: false,
            buffered: Vec::new(),
            commanded: false,
            attackers: Vec::new(),
        })
    }

    /// What an order (INTERACT, MOVE, RETREAT) does to each of its units' AI before it runs
    /// (`FUN_140e6cfc0`): an adjusted idle timeout of 750 or more is redrawn
    /// (`UpdateAdjustedIdleTimeout`, unit-AI slot 53, `0x140c41e00`), and the secondary timer is
    /// capped at 750.
    pub fn reset_for_order(&mut self, ai: &mut Well512) {
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

    /// The order setter with its history (`FUN_140c64b60` with 1): leaving order 711 or 733
    /// redraws an adjusted idle timeout of exactly 750 and caps the secondary timer at 750.
    pub(crate) fn set_order(&mut self, order: i32, ai: &mut Well512) {
        let leaving = |o| self.current_order == o && order != o;
        if leaving(711) || leaving(733) {
            if self.adjusted_idle_timeout == 750 {
                self.update_adjusted_idle_timeout(ai);
            }
            self.secondary_timer = self.secondary_timer.min(750);
        }
        self.last_order = self.current_order;
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
            self.last_target = -1;
            self.last_target_type = -1;
            self.current_target = -1;
            self.target_type = -1;
            self.target_point = [-1.0; 3];
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
    /// without a target (`FUN_140c64c10` with −1, which leaves the target class).
    const fn move_to(&mut self, m: AiMove) -> Option<AiMove> {
        if !m.force && self.current_action != -1 {
            return None;
        }
        self.set_action(MOVE_ACTION);
        self.last_target = self.current_target;
        self.last_target_type = self.target_type;
        self.current_target = -1;
        self.target_point = m.point;
        Some(m)
    }

    /// `FUN_140c41e50`: the unit's last human order time drops to `FUN_140fe0d10`'s 5000, not on
    /// orders 701, 711, 712 and 733.
    fn cap_human_order(&self, fx: &mut Vec<Effect>) {
        if !matches!(self.current_order, 0x2bd | 0x2c7 | 0x2c8 | 0x2dd) {
            fx.push(Effect::CapHumanOrder(HUMAN_ORDER_CAP));
        }
    }

    /// The boar's idle check (slot 72, `FUN_140c1e630`): a furious boar, or any in mode 1, stays.
    /// Then one draw of `0..81` widens its leash: outside it, a boar not yet heading home goes
    /// home; inside, once in 7 checks (a second draw) it goes home from more than 2 tiles away, or
    /// wanders up to 3 tiles (two more draws). Its moves keep its height. `FUN_140c41e50`, first,
    /// lowers the unit's last human order time to 5000 (`FUN_140fe0d10`); the slice does not hold it.
    /// True when it produced an action (6, not 5): every way but staying and the 6 in 7 checks
    /// inside the leash that do nothing, including the one heading home already.
    fn boar_idle(&mut self, at: &Surroundings, ai: &mut Well512) -> (bool, Option<AiMove>) {
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

    /// The hunted animal's idle check (slot 72, `FUN_140c1eea0`). `FUN_140c41e50`, first, lowers
    /// the unit's last human order time to 5000, which the slice does not hold; the king's branch
    /// (class 59) is not modelled. Beyond its leash (squared, from the unit's footprint,
    /// `FUN_140a847c0`) it goes home. Then its search: with nothing found, one draw; once in 7 it
    /// goes home from more than 2 tiles away, else once in 7 (the same draw) it wanders, two more
    /// draws; mode 1's herd is not modelled. A found object it would evade (slot 0x150), which is
    /// not modelled. Its moves keep its height and come within 0.5, forced and walking.
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

    /// The hunted animal's evade of the object its search found (`FUN_140c1eea0`, then slot
    /// 0x150, `FUN_140c4d140`, with arguments 5 and 6 as the idle's frame leaves them: 1 and 0 at
    /// every call probed): from its place away from the object by three times its speed (slot
    /// 0x208; one tile more in mode 1, not here), held on the map (below 0 to 0, at the far edge
    /// one tile in), the unit's move there (slot 0x340: range 0, forced, running); the AI holds
    /// the move action and the point.
    fn evade(&mut self, at: &Surroundings, other: [f32; 3]) -> Option<AiMove> {
        let [x, y, z] = at.position;
        let reach = at.speed * EVADE_REACH;
        let (dx, dy) = (other[0] - x, other[1] - y);
        let d = crate::fmath::sqrt(dy * dy + dx * dx);
        let held = |v: f32, size: i32| {
            if v < 0.0 {
                0.0
            } else if size as f32 <= v {
                size as f32 - EVADE_EDGE
            } else {
                v
            }
        };
        let ex = held(x - (dx / d) * reach, at.map[0]);
        let ey = held(y - (dy / d) * reach, at.map[1]);
        self.move_to(AiMove {
            point: [ex, ey, z],
            range: 0.0,
            force: true,
            walk: false,
        })
    }

    fn hunted_idle(
        &mut self,
        at: &Surroundings,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
    ) -> Result<Option<AiMove>, Unsupported> {
        let h = self
            .hunted
            .ok_or(Unsupported("a hunted animal without its origin"))?;
        if at.class == CLASS_KING {
            return Err(Unsupported("the hunted animal's branch for class 59"));
        }
        let [x, y, z] = at.position;
        let go_home = AiMove {
            point: [h.origin[0], h.origin[1], z],
            range: AI_MOVE_RANGE,
            force: true,
            walk: true,
        };
        let leash = h.leash * h.leash;
        if 0.0 < leash {
            let gap = |d: f32, r: f32| {
                let g = d.abs() - r;
                if 0.0 <= g { g } else { 0.0 }
            };
            let dx = gap(h.origin[0] - x, at.size[0]);
            let dy = gap(h.origin[1] - y, at.size[1]);
            if leash < dy * dy + dx * dx {
                return Ok(self.move_to(go_home));
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
            },
        })?;
        self.alert(found, at.dt_seconds);
        if found.id != -1 {
            let other = found
                .position
                .ok_or(Unsupported("a found object without its place"))?;
            return Ok(self.evade(at, other));
        }
        let r = ai.rand15();
        if r.is_multiple_of(BOAR_WANDER) {
            let dx = x - h.origin[0];
            let dy = y - h.origin[1];
            if HUNTED_HOME < dy * dy + dx * dx {
                return Ok(self.move_to(go_home));
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
            return Ok(self.move_to(AiMove {
                point: [x + sx, sy + y, z],
                ..go_home
            }));
        }
        Ok(None)
    }

    /// The lion's idle check (slot 72, `FUN_140c1fb70`, factory type 5: wolves, bears).
    /// `FUN_140c41e50` first (not held, as for the hunted animal); its idle timeout becomes 6000,
    /// redrawn (`FUN_140c41bc0`, slot 0x1a8: one draw). Its search looks for an enemy villager
    /// (class 4) in state 2, then for any enemy in state 2, from the same place (the second reads
    /// the first's answer). A found one it would attack (slot 0xb8, within half its line of sight
    /// outside mode 1), which is not modelled.
    fn lion_idle(
        &mut self,
        at: &Surroundings,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
    ) -> Result<Option<AiMove>, Unsupported> {
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
            },
        };
        // each search that queries sets the alert mode (`FUN_140c63e20`)
        let mut found = side.closest(&search(4))?;
        self.alert(found, at.dt_seconds);
        if found.id < 0 {
            found = side.closest(&search(-1))?;
            self.alert(found, at.dt_seconds);
        }
        if found.id != -1 {
            return Err(Unsupported(
                "a lion attacking an object it sees (slot 0xb8)",
            ));
        }
        Ok(None)
    }

    /// Slot 72 for the factory types modelled: the boar's, the hunted animal's, the lion's and
    /// the soldier's ([`UnitAi::soldier_idle`]) do something for an idle unit; each, and the
    /// transport ship's and the villager's (`FUN_140c1e610`, which does nothing else), first
    /// lowers the unit's last human order time to `FUN_140fe0d10`'s 5000
    /// (`FUN_140c41e50`; not on orders 701, 711, 712 and 733, and a group's would decide, the
    /// slice holds none), as does the tower's ([`UnitAi::tower_idle`]). The building's does
    /// nothing.
    fn process_idle(
        &mut self,
        at: &Surroundings,
        computer: bool,
        ai: &mut Well512,
        side: &mut dyn UnitSide,
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        if matches!(self.ai_type, 2 | 4 | 5 | 8 | 11 | 13 | 15) {
            self.cap_human_order(fx);
        }
        match self.ai_type {
            13 => return self.soldier_idle(at, computer, side),
            15 => {
                if self.tower_idle(at, computer, side, fx)? {
                    return Err(Unsupported(
                        "an idle tower's attack (processIdle 6 in the idle part)",
                    ));
                }
                return Ok(());
            }
            _ => {}
        }
        let moved = match self.ai_type {
            2 => self.hunted_idle(at, ai, side)?,
            BOAR_AI => self.boar_idle(at, ai).1,
            5 => self.lion_idle(at, ai, side)?,
            _ => None,
        };
        fx.extend(moved.map(Effect::Move));
        Ok(())
    }

    /// The soldier's idle check (slot 72, `FUN_140c20db0`, factory type 13) as the idle part calls
    /// it (its second argument 1), after the cap of [`UnitAi::process_idle`]. Before the cap, a
    /// player that is out (`+0x89` 2; not held, 0 in every player probed) would stop, and slot
    /// 0x250 (`FUN_140c5d9a0`) retries no order, the soldier's slot 0x60 taking none as
    /// retryable. A Gaia unit stops unless it is of the combat type with `+0x924` 0 outside the
    /// scenario modes (`FUN_141302540`); one in `NoAttack` (stance 3) searches and returns to its
    /// state position ([`UnitAi::no_attack_idle`]); master 706 stops. Its target is the target of its top action
    /// within its range (slot 400, slot 0x1e8) with retargeting on (`+0x1d6` 0), else its current
    /// target; neither is modelled, nor a computer's branches (patrol data, priorities, evasion).
    /// Without one, `bestUnitToAttack` from its place ([`Pick::Attack`]): an object found would be
    /// attacked (slot 0xb8), which is not modelled. A patrol order then stops. The search leaves the alert mode clear, so `+0x1bc`
    /// stays. In Defend or `StandGround` (stance 1, 2) a unit more than a tile from its state
    /// position (the squared footprint gap, `FUN_140a847c0`) would walk back (`FUN_140c5e7d0`,
    /// not modelled); a patrol path (`+0x1c0`) and the healer's ally search (`FUN_140c16f00`, for
    /// a master with a task of action type 0x69) are not modelled.
    fn soldier_idle(
        &mut self,
        at: &Surroundings,
        computer: bool,
        side: &mut dyn UnitSide,
    ) -> Result<(), Unsupported> {
        let searches = !at.scenario && 70 <= at.kind && at.capture == 0;
        if at.player == 0 && !searches {
            return Ok(());
        }
        if self.stance == 3 {
            return self.no_attack_idle(at, computer, side);
        }
        if at.master == SOLDIER_IDLE_MASTER {
            return Ok(());
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
            return Err(Unsupported(
                "a soldier attacking what its idle search found (slot 0xb8)",
            ));
        }
        if matches!(self.current_order, PATROL | PATROL_2) {
            return Ok(());
        }
        if matches!(self.stance, 1 | 2) {
            let gap = |d: f32, r: f32| {
                let g = d.abs() - r;
                if 0.0 <= g { g } else { 0.0 }
            };
            let dx = gap(self.state_position[0] - at.position[0], at.size[0]);
            let dy = gap(self.state_position[1] - at.position[1], at.size[1]);
            if 1.0 < dy * dy + dx * dx {
                return Err(Unsupported(
                    "a soldier walking back to its state position (FUN_140c5e7d0)",
                ));
            }
        }
        if self.patrol_path {
            return Err(Unsupported("a soldier's patrol path in its idle check"));
        }
        if at.task_0x69 {
            return Err(Unsupported("a healer's search for an ally to heal"));
        }
        Ok(())
    }

    /// The soldier's idle check in `NoAttack` (stance 3, `FUN_140c20db0`: "searching and returning
    /// to state pos."): the search alone from its place (slot 0x50, `FUN_140c63e20` with mode 1,
    /// [`Pick::Look`]) leaves the alert mode clear when it finds no enemies; then, but on the
    /// patrol orders, a unit more than a tile from its state position (the squared footprint gap,
    /// `FUN_140c5e7d0`) would walk back; last the healer's ally search (`FUN_140c16f00`). Not
    /// modelled: enemies found (the alert mode on, its object `+0x1bc` from slot 0x1d8, and
    /// after 10 s without a sighting the player's notify 0x210, slot 0x2e8), a defend order's
    /// search (from its target's objects), a computer's cared classes (`+0x190` with 10), the
    /// walk back and the healer's search.
    fn no_attack_idle(
        &mut self,
        at: &Surroundings,
        computer: bool,
        side: &mut dyn UnitSide,
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
            let gap = |d: f32, r: f32| {
                let g = d.abs() - r;
                if 0.0 <= g { g } else { 0.0 }
            };
            let dx = gap(self.state_position[0] - at.position[0], at.size[0]);
            let dy = gap(self.state_position[1] - at.position[1], at.size[1]);
            if 1.0 < dy * dy + dx * dx {
                return Err(Unsupported(
                    "a soldier walking back to its state position (FUN_140c5e7d0)",
                ));
            }
        }
        if at.task_0x69 {
            return Err(Unsupported("a healer's search for an ally to heal"));
        }
        Ok(())
    }

    /// The base unit AI's idle check (slot 72, `FUN_140c564c0`), the tower's (factory type 15),
    /// as the idle part calls it, after the cap of [`UnitAi::process_idle`] (slot 0x250 first
    /// retries no order, the tower's slot 0x60 taking none as retryable). In `NoAttack` (stance 3)
    /// it only searches, which is not modelled; else `bestUnitToAttack` from its place
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
        if self.stance == 3 {
            return Err(Unsupported("a tower's idle search in NoAttack (stance 3)"));
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
    /// applies in order. A unit in a group is not modelled (the slice holds none), nor are the
    /// retarget entries and the attackers list (the save's, empty in every unit probed).
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
        self.target_block(at, computer, ai, &mut fx)?;
        self.keep_attackers(at, side);
        self.check_busy_order(retarget, at.busy && fx.is_empty())?;
        if self.current_order != -1 {
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
            && self.can_order(at)?
            && !at.garrisoned
        {
            self.run_orders(at, &mut fx)?;
        }
        let ordered = fx.iter().any(|e| matches!(e, Effect::OrderOnTarget(_)));
        let mut retargeted = false;
        if matches!(retarget, Retarget::Base) {
            retargeted = self.retarget_check(world_time, computer, at, ai, ordered, &mut fx)?;
        }
        if ordered {
            self.idle_part_before_order(busy, at)?;
        } else {
            self.idle_part(busy, at, computer, ai, side, &mut fx)?;
        }
        self.look_around(computer, ai)?;
        self.boar_tail(at, computer, ai, side, &mut fx)?;
        if ordered {
            fx.push(Effect::AfterOrder(AfterOrder { retargeted }));
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
            self.retarget_outcome(target, weapon_range, computer)?;
        }
        self.idle_part_end(class, player)
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
            self.process_idle(at, computer, ai, side, fx)?;
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
    /// other action loses its target and stops (slot 0xb0 with 1). The patrol range
    /// (`FUN_140c41690`), a computer's priority by the target's distance and action `0x2be`'s check
    /// (`FUN_140c40f10`) are not modelled; the slice holds no unit groups (`+0x6d0`).
    fn target_block(
        &mut self,
        at: &Surroundings,
        computer: bool,
        ai: &mut Well512,
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
                            self.next_after_kill(at, ai, fx)?;
                        } else if action != HUNT_ACTION && action != BUILD_ORDER {
                            self.remove_target();
                            self.stop_current(true, at, fx)?;
                        }
                    }
                }
                if matches!(self.current_order, PATROL | PATROL_2) {
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

    /// The target block's attack whose target died (`0x140c66b10`): processIdle (slot 0x240 with
    /// 0; the boar's [`UnitAi::boar_idle`] after the cap) may find the next thing to do (6); with
    /// nothing (5) the AI moves to its old target's point within the unit's weapon range (slot
    /// 0x130, forced, running: it always takes it). Then the order ends: it becomes the last order,
    /// the order and its priority −1. The other factory types' slot 0x240 and a patrol order's
    /// part (orders 711, 733) are not modelled.
    fn next_after_kill(
        &mut self,
        at: &Surroundings,
        ai: &mut Well512,
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        let produced = match self.ai_type {
            BOAR_AI => {
                self.cap_human_order(fx);
                let (produced, moved) = self.boar_idle(at, ai);
                fx.extend(moved.map(Effect::Move));
                produced
            }
            _ => {
                return Err(Unsupported(
                    "an attack whose target died: processIdle of another unit AI (slot 0x240)",
                ));
            }
        };
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
    /// order (712 with its action 612, the move's 610 or none), on a boar's own attack (−1/600)
    /// or the attack order (700/600), or without an order and an action (a hunt ended by its
    /// queued orders); busy (`busy`, the actions left), only those or a unit on MOVE's order
    /// without a target (710/610, or an animal's own wander), and for the types that never
    /// retarget also one whose move ended (710 with no action value); the attacks only for the
    /// types that do.
    fn check_busy_order(&self, retarget: Retarget, busy: bool) -> Result<(), Unsupported> {
        let order = (self.current_order, self.current_action);
        let working = matches!(
            order,
            (BUILD_ORDER, BUILD_ACTION)
                | (HUNT_ORDER | ATTACK_ORDER | -1, HUNT_ACTION)
                | (crate::ai_kinds::ORDER_GATHER, GATHER_ACTION)
                | (crate::garrison::ENTER_ORDER, crate::garrison::ENTER_ACTION)
                | (FOLLOW_ORDER, FOLLOW_ACTION | MOVE_ACTION | -1)
        );
        let moving = matches!(order, (MOVE_ORDER | -1, MOVE_ACTION)) && self.current_target == -1;
        // a move whose action ended (the action value reset) while the default action runs
        let moved = order == (MOVE_ORDER, -1) && self.current_target == -1;
        let scouting = order == (AUTO_SCOUT_ORDER, AUTO_SCOUT_ACTION) && self.current_target == -1;
        let following = order.0 == FOLLOW_ORDER;
        // a boar's own attack on its attacker ([`UnitAi::attack_object`]), or a work command's
        // attack (the attack order and action, notify kind 9); a building's make action that
        // started during it takes the action value (619, its constructor `FUN_140acb140`) while
        // the Attack goes on
        let attacking = (self.ai_type == 4 && order == (-1, ATTACK_ACTION))
            || order == (ATTACK_ORDER, ATTACK_ACTION)
            || order == (ATTACK_ORDER, crate::spawn::MAKE_ACTION);
        // a move to an object (the work command's without a task, slot 0x138)
        let moving_to = order == (MOVE_ORDER, MOVE_ACTION);
        // without an order and an action the target is a stale one: the order queue's next order
        // takes over
        if self.current_target != -1 && !working && !attacking && !moving_to && order != (-1, -1) {
            return Err(Unsupported("a unit AI's target the slice does not model"));
        }
        if !busy || self.is_idle() {
            return Ok(());
        }
        match retarget {
            Retarget::Never if working || moving || moving_to || moved => Ok(()),
            Retarget::Base if moving || scouting || following || attacking => Ok(()),
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

    /// The unit's notify (its slot 0x328, `FUN_141086a10`) to its unit AI: a unit in state 2
    /// passes it to the AI's slot 0x38 (`FUN_140c556d0`), which queues it (`FUN_140c42470`) but
    /// for an action's invalidation that is not the current action's and a movement completion
    /// while the AI has an action other than a move (`0x262`, `0x268`, `0x2bf`, `0x2c0`).
    /// A notify 699 is taken only from one of its attackers. A notify the AI does not take (any
    /// but 699) goes to the owner's slot 0x2e8 (`FUN_14162dab0`, the human player's alerts),
    /// which the slice does not hold.
    pub fn post(&mut self, n: Notify, unit_state: u8) -> Result<(), Unsupported> {
        if unit_state != 2 {
            return Ok(());
        }
        let action = self.current_action;
        let taken = match n.kind {
            // "That unit is not attacking me, so I'm ignoring it."
            NO_LONGER_ON_ME => self.attackers.binary_search(&n.caller).is_ok(),
            ACTION_INVALIDATED => n.p[0] == action,
            ACTION_COMPLETED => {
                // ignored (and handled) for a non-movement action
                if n.p[0] == MOVE_ACTION && !matches!(action, 0x262 | 0x268 | 0x2bf | 0x2c0) {
                    return Ok(());
                }
                true
            }
            _ => true,
        };
        if taken {
            self.append(n);
        }
        Ok(())
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
        if let Err(i) = self.attackers.binary_search(&n.caller) {
            self.attackers.insert(i, n.caller);
        }
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
    fn villager_damaged(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
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
        let added = match self.attackers.binary_search(&n.caller) {
            Ok(_) => false,
            Err(i) => {
                self.attackers.insert(i, n.caller);
                true
            }
        };
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
            if matches!(self.current_order, PATROL | PATROL_2) {
                return Err(Unsupported(
                    "a villager leaving a patrol order to attack back",
                ));
            }
            self.current_order = crate::ai_kinds::ORDER_ATTACK;
            self.order_priority = 100;
        }
        // slot 0xb8 forced, the civilian's (`FUN_140c17370`): HuntObject on an animal or a tree
        // (classes 9, 10, 15), else AttackObject
        if matches!(attacker.class, 9 | 10 | 15) {
            self.hunt_object(
                n.caller,
                i32::from(attacker.class),
                attacker.position,
                at,
                fx,
            )?;
        } else {
            let _ = self.attack_object(n.caller, true, false, at, side, fx)?;
        }
        self.stop_after_kill = true;
        Ok(BREAK)
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
    /// ready): a unit above state 2 or without an order takes none. The current order goes in
    /// front of the queue (`FUN_140c42640`), then the given one, the same record, in front too;
    /// then (an order other than `0x2d9`) the target goes (`FUN_140c5e1e0`) and the action stops
    /// (slot 0xb0 with 1, which ends an order of its action), the notifies end
    /// (`FUN_140c5dc10` with 0), the queue's first order runs at once (slot 0x230,
    /// [`UnitAi::process_order`]) and, unless it was not ready, leaves the queue
    /// (`FUN_140c5e280`).
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
        self.orders.insert(0, record);
        self.orders.insert(0, record);
        if record.order != crate::ai_kinds::ORDER_ATTACK_ROUNDUP {
            self.remove_target();
            self.stop_current(true, at, fx)?;
        }
        self.clear_notifies(0)?;
        let front = self.orders[0];
        if self.process_order(&front, Some(at), fx)? != 0xe {
            let _ = self.orders.remove(0);
        }
        Ok(())
    }

    /// `AttackObject` (slot 0xb8, `FUN_140c43720`): `+0x1d5` clears; not `force`d, an AI with an
    /// action fails (false). The target must be in the world; its slot 0xa0 with `any` must let it
    /// attack the target: the boar's (`FUN_140c48400`) anything, the lion's
    /// ([`lion_may_attack`]), the soldier's and the tower's ([`UnitSide::may_attack`]; the other
    /// factory types' are not modelled); then the unit's attack command ([`UnitSide::attack`]), on
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
            let Some(t) = at.unit(target) else {
                return Ok(false);
            };
            let may = match self.ai_type {
                4 => true,
                5 => any || lion_may_attack(at.attack_priority, t.class),
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
        self.set_target(target, i32::from(class), position);
        Ok(true)
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
                DAMAGED => Err(Unsupported("a hunted animal's damage notify (its evade)")),
                0x20f => Ok(GO_ON),
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the boar (FUN_140c25770): its damage notify attacks
            4 => match n.kind {
                DAMAGED => self.boar_damaged(n, time, at, ai_rng, side, fx),
                ACTION_COMPLETED if self.current_action == 0x2bf => {
                    Err(Unsupported("a boar's completion of action 0x2bf"))
                }
                0x20f => Ok(GO_ON),
                _ => self.base_notify(n, time, at, computer, ai_rng, side, fx),
            },
            // the lion (FUN_140c266f0)
            5 => match n.kind {
                ACTION_COMPLETED if n.p[0] == 600 => Err(Unsupported("a lion's completed attack")),
                0x1fc => Err(Unsupported("a lion's moved target")),
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
                DAMAGED => self.villager_damaged(n, time, at, computer, side, fx),
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
                DAMAGED => Err(Unsupported("a tower's damage notify")),
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
                DAMAGED | 600 | NO_LONGER_ON_ME => Err(Unsupported("a building's attack notify")),
                0x202 | 0x216 | 0x21b => self.base_notify(n, time, at, computer, ai_rng, side, fx),
                _ => Ok(GO_ON),
            },
            // livestock (FUN_140c26ac0): any notify but 0x202 and 0x216 drops the action value
            // (FUN_140c64b50) and breaks off
            19 => match n.kind {
                0x202 | 0x216 => Err(Unsupported("a livestock's notify 0x202 or 0x216")),
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
            ACTION_FAILED => self.action_failed(n, at, computer, ai_rng, fx),
            ACTION_INVALIDATED => self.action_invalidated(n, at, computer, ai_rng, side, fx),
            crate::explore::WANTS_POINT if self.explorer() => {
                self.seek_unexplored(at, ai_rng, side, fx)
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
    /// 0xb8, forced, its slot 0xa0 unforced, [`UnitAi::attack_object`]) when the world's table
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
            let _ = self.attack_object(n.p[0], true, false, at, side, fx)?;
        }
        Ok(GO_ON)
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
    /// posted again, which is not modelled. On the scout order (`0x2da`) the player's slot 0x2e8
    /// with `0x211` is not modelled; on any other the unit AI forgets its target
    /// (`FUN_140c5e1e0`) and stops (slot 0xb0 with 1; slot 0x48 hands the notify to the owner,
    /// not held; the interface's flag a last auto scout order sets is not the sim's) and
    /// processing goes on. The other explorers' search is not modelled.
    fn seek_unexplored(
        &mut self,
        at: &Surroundings,
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
                    return Err(Unsupported(
                        "an auto scout with no undiscovered tile left (the scout map made)",
                    ));
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
    /// (700) or with the attack action current runs processIdle (slot 0x240 with 0; the boar's
    /// only, [`UnitAi::boar_idle`]): an action it produced breaks off (0xd), none makes a human
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
            if self.ai_type != BOAR_AI {
                return Err(Unsupported(
                    "a failed attack on the attack order or action (processIdle, slot 0x240)",
                ));
            }
            self.cap_human_order(fx);
            let (produced, moved) = self.boar_idle(at, ai);
            fx.extend(moved.map(Effect::Move));
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
    /// processIdle (slot 0x240 with 0): the tower's (the base `FUN_140c564c0`, which skips the
    /// order retry of slot 0x250 with 0; [`UnitAi::tower_idle`] after the cap) attacks what its
    /// search finds (not modelled) or does nothing (5), the boar's ([`UnitAi::boar_idle`], after
    /// the cap) may produce an action (6); the building's handler never passes the notify on
    /// (`FUN_140c1e600` would do nothing either). Unless it is 5 the
    /// notify goes on; with 5 a human player's unit forgets its target (`FUN_140c5e1e0`) and stops
    /// (slot 0xb0 with 1); a computer player's tests (slot 0x130) are not modelled, nor are the
    /// other factory types' slot 0x240 with 0. Without an order, the invalidation of the current
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
            let produced = match self.ai_type {
                TOWER_AI => {
                    self.cap_human_order(fx);
                    self.tower_idle(at, computer, side, fx)?
                }
                BOAR_AI => {
                    self.cap_human_order(fx);
                    let (produced, moved) = self.boar_idle(at, ai);
                    fx.extend(moved.map(Effect::Move));
                    produced
                }
                _ => {
                    return Err(Unsupported(
                        "an invalidated attack on the attack order (its retarget, slot 0x240)",
                    ));
                }
            };
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
    /// current one (`+0x48`, `+0x4c`) go, the target point with them.
    pub(crate) const fn forget_targets(&mut self) {
        self.last_target = -1;
        self.last_target_type = -1;
        self.current_target = -1;
        self.target_type = -1;
        self.target_point = [-1.0; 3];
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
    fn check_order(&mut self, at: &Surroundings) -> Result<(), Unsupported> {
        let ends = matches!(
            self.current_order,
            0x269 | 700 | 0x2c5 | 0x2c6 | 0x2c9 | 0x2cf | 0x2d0 | 0x2dd | 0x322
        );
        if ends && self.can_order(at)? {
            if self.current_order == PATROL_2 {
                return Err(Unsupported("the end of a patrol order 733"));
            }
            self.remove_target();
            self.current_order = -1;
        }
        Ok(())
    }

    /// `FUN_140c4eea0` with 0: a unit takes orders unless its master has a task of action type
    /// 0x99 and it has a target or an action (not modelled).
    const fn can_order(&self, at: &Surroundings) -> Result<bool, Unsupported> {
        if at.task_0x99 {
            return Err(Unsupported("a unit with a task of action type 0x99"));
        }
        Ok(true)
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
        if self.can_order(at)? {
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
            return Err(Unsupported("an immediate order for the unit AI's queue"));
        }
        if in_front && !self.orders.is_empty() {
            self.orders.insert(0, order);
        } else {
            self.orders.push(order);
        }
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
            let _ = self.orders.remove(k);
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
        self.last_target = self.current_target;
        self.last_target_type = self.target_type;
        self.current_target = -1;
        self.target_point = [point[0], point[1], EXPLORE_Z];
        true
    }

    /// Slot 0x28 (`FUN_140c55c90`) with Immediate set and inFront clear: for a unit in state 2 or
    /// below and an order (not −1), the order goes to the front of the queue (`FUN_140c42640`
    /// with inFront 1); then, but for an attack roundup (0x2d9, not modelled), the target goes
    /// and the action stops (`FUN_140c5e1e0`, slot 0xb0 with 1), the notifies go
    /// (`FUN_140c5dc10` with 0) and the order runs at once (slot 0x230), leaving the queue unless
    /// it was not ready (`FUN_140c5e280`). `place` is the unit's position (x, y) and master's
    /// class, for the stop.
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
        if order.order == 0x2d9 {
            return Err(Unsupported("an immediate attack roundup order"));
        }
        self.orders.insert(0, order);
        self.remove_target();
        self.stop_with(true, Some(place), fx)?;
        self.clear_notifies(0)?;
        if self.process_order(&order, None, fx)? != 0xe {
            let _ = self.orders.remove(0);
        }
        Ok(())
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
        {
            self.idle_timer = 0;
            self.process_idle(at, computer, ai, side, fx)?;
            return Ok(());
        }
        if !matches!(self.current_order, PATROL | PATROL_2)
            && matches!(at.class, CLASS_3 | CLASS_52)
            && matches!(self.current_action, 0x26b | 0x26c)
        {
            return self.making_idle_part(at, computer, ai, side, fx);
        }
        self.idle_part_end(at.class, at.player)
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
        self.process_idle(at, computer, ai, side, fx)
    }

    /// [`UnitAi::idle_part`] when the update took an order on a target, which in the game ran
    /// before it: its end waits for the order ([`UnitAi::after_order`]); a timer past its timeout
    /// would run slot 72 if the order gave the AI no action, which is not modelled.
    const fn idle_part_before_order(
        &self,
        busy: bool,
        at: &Surroundings,
    ) -> Result<(), Unsupported> {
        let class_20 = busy && at.class == CLASS_20;
        if !class_20
            && self.adjusted_idle_timeout < self.idle_timer
            && (self.current_target == -1 || !busy)
        {
            return Err(Unsupported(
                "an idle part past its timeout after an order on a target",
            ));
        }
        Ok(())
    }

    /// The idle part past slot 72: patrol orders, class 3's and 52's actions `0x26b`/`0x26c` (but
    /// in [`UnitAi::idle_part`], [`UnitAi::making_idle_part`]) and action `0x2c0` are not
    /// modelled; an action or a target keeps the idle timer of a player above Gaia at 0.
    const fn idle_part_end(&mut self, class: i16, player: usize) -> Result<(), Unsupported> {
        if matches!(self.current_order, PATROL | PATROL_2) {
            return Err(Unsupported("a patrol order's idle part"));
        }
        if matches!(class, CLASS_3 | CLASS_52) && matches!(self.current_action, 0x26b | 0x26c) {
            return Err(Unsupported("a unit of class 3 or 52 on action 0x26b/0x26c"));
        }
        if self.current_action == 0x2c0 {
            return Err(Unsupported("the idle part of action 0x2c0"));
        }
        if (self.current_action != -1 || self.current_target != -1) && 0 < player {
            self.idle_timer = 0;
        }
        Ok(())
    }

    /// Slot 73 (`0x140c567c0`, processMisc). Its order part first, by the order (an action's
    /// value plus 100 without one): the follow order's ([`UnitAi::follow`]); the defend order's
    /// is not modelled. Then the leash checks: stance 1's is not modelled; stance 2's (stand
    /// ground) stops a unit with an action that strayed from its state position by more than its
    /// range plus `DAT_142eb880c` (0.25), which a unit on an order, whose state position this
    /// update set, never has, and an idle unit has no action to stop. Then the retarget check:
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
            let gap = |d: f32, r: f32| {
                let g = d.abs() - r;
                if 0.0 <= g { g } else { 0.0 }
            };
            let dx = gap(self.state_position[0] - at.position[0], at.size[0]);
            let dy = gap(self.state_position[1] - at.position[1], at.size[1]);
            if DEFEND_LEASH < dy * dy + dx * dx {
                self.stop_current(true, at, fx)?;
                self.remove_target();
                return Err(Unsupported(
                    "a defending unit's walk back to its state position (FUN_140c5e7d0)",
                ));
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
            self.retarget_outcome(target, at.weapon_range, computer)?;
        }
        Ok(true)
    }

    /// The retarget check after its timer's redraw: a human player's unit of a range above 3 on
    /// the attack order stops there, as does a unit with a Gaia target or without one (or one
    /// gone; `target` its owner and class as the world holds it), and one with a player's target
    /// off the attack order but for a building, wall or trebuchet target, whose search for a
    /// better target is not modelled.
    fn retarget_outcome(
        &self,
        target: Option<(usize, i16)>,
        weapon_range: f32,
        computer: bool,
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
        Err(Unsupported(
            "a unit AI's search for a better target (slot 0x538 with 9, then slot 0x520)",
        ))
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

    /// The AI's move to an object (slot 0x138, `FUN_140c55490`, its fourth argument 1): the
    /// unit's slot 0x348 ([`Effect::MoveToObject`]) takes a target the world holds, and the AI
    /// the move action and the target with its class and point (`FUN_140c64c10`; the target and
    /// class it had become the last).
    fn move_to_object(&mut self, id: i32, target: Seen, range: f32, fx: &mut Vec<Effect>) {
        fx.push(Effect::MoveToObject { target: id, range });
        self.set_action(MOVE_ACTION);
        self.last_target = self.current_target;
        self.last_target_type = self.target_type;
        self.current_target = id;
        self.target_type = i32::from(target.class);
        self.target_point = target.position;
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
        let r = u.villager_damaged(hit, 0, &at, false, &mut side, &mut fx);
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
            u.villager_damaged(hit, 0, &at, false, &mut side, &mut fx)
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
        // 5.1 tiles off its footprint: it stops and forgets its target (the walk back is not
        // modelled)
        let (mut u, at) = defender(15.6);
        assert!(
            u.retarget_check(0, false, &at, &mut r, false, &mut fx)
                .is_err()
        );
        assert_eq!(fx, vec![Effect::Stop]);
        assert_eq!((u.current_action, u.current_target), (-1, -1));
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
                state_position: [30.0, 88.0],
                retarget_disabled: 0,
                alert_mode: false,
                time_since_enemy: 0.0,
                patrol_path: false,
                stop_after_kill: false,
                home: None,
                hunted: None,
                order_priority: -1,
                last_update_time: 0,
                orders: Vec::new(),
                building_orders: 0,
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
            ..idle(13)
        };
        let mut r = rng();
        u.reset_for_order(&mut r);
        assert_eq!(
            (u.adjusted_idle_timeout, u.secondary_timer),
            (7241 % 1500 + 2250, 750)
        );
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
            u.action_failed(n, &at([10.0, 10.0, 0.0]), false, &mut rng(), &mut fx)
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
            .action_failed(n, &at([10.0, 12.0, 0.0]), false, &mut rng(), &mut fx)
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
            .action_failed(n, &at([10.0, 12.0, 0.0]), false, &mut rng(), &mut fx)
            .unwrap();
        assert_eq!(u.current_action, 0x25a);
        let mut u = UnitAi {
            current_order: 700,
            ..UnitAi::default()
        };
        let attack = Notify::own(9183, ACTION_FAILED, [600, 0, 0]);
        assert!(
            u.action_failed(attack, &at([10.0, 12.0, 0.0]), false, &mut rng(), &mut fx)
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
        let r = u.action_failed(attack, &at([169.5, 50.6, 0.0]), false, &mut rng(), &mut fx);
        assert_eq!(r, Ok(PRODUCED));
        assert_eq!(first_move(&fx).map(|m| m.point), Some([171.5, 51.5, 0.0]));
        assert_eq!((u.current_action, u.last_action), (MOVE_ACTION, 600));
        assert!(u.home.is_some_and(|h| !h.go_home));
        let mut u = boar(home(false, true));
        let mut fx = Vec::new();
        let r = u.action_failed(attack, &at([169.5, 50.6, 0.0]), false, &mut rng(), &mut fx);
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
        u.target_block(&standing, false, &mut Well512::default(), &mut fx)
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
        u.target_block(&gone, false, &mut Well512::default(), &mut fx)
            .unwrap();
        assert_eq!((u.current_target, u.target_type), (-1, 3));
        assert_eq!(u.idle_timer, 3000);
        // inside another, a unit drops its target before the lookup
        let inside = Surroundings {
            garrisoned: true,
            ..standing
        };
        let mut u = builder_done();
        u.target_block(&inside, false, &mut Well512::default(), &mut fx)
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
        u.target_block(&dead, false, &mut Well512::default(), &mut fx)
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
        u.target_block(&dead, false, &mut Well512::default(), &mut fx)
            .unwrap();
        assert_eq!((u.current_target, fx), (10028, vec![]));
        // an attack whose target died waits while its Attack swings on the kill (state 2); past
        // it a villager's processIdle is not modelled, nor is an Attack the slice does not hold
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
        u.target_block(&swinging, false, &mut Well512::default(), &mut fx)
            .unwrap();
        assert_eq!((u.current_target, fx), (10028, vec![]));
        let over = Surroundings {
            action_state: Some(1),
            ..dead.clone()
        };
        for at in [&over, &dead] {
            assert!(
                attack()
                    .target_block(at, false, &mut Well512::default(), &mut Vec::new())
                    .is_err()
            );
        }
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
        // away from its state position, a stance-2 soldier would walk back
        let (mut u, at) = idle_scout();
        u.state_position = [25.0, 92.5];
        let mut none = |_: &Search<'_>| Ok(Found::NONE);
        assert!(u.update(28, 0, false, &at, &mut rng(), &mut none).is_err());
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
    }

    /// A scout cavalry (master 448, class 12) of player 1 in `StandGround` at its state position,
    /// as records/425906646's 27272 at update 4268.
    const SCOUT: Attacker = Attacker {
        ai_type: 13,
        player: 1,
        kind: 70,
        class: 12,
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
}
