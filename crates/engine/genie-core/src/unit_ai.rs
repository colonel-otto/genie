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
/// `0x200`: "`ActionExecuted`" (the soldier's log line), an attack's swing (p1 600, p2 the target).
pub const ACTION_EXECUTED: i32 = 0x200;
/// `0x20b`: the unit changed hands (p1 the new owner).
pub const OWNER_CHANGED: i32 = 0x20b;
/// `500`: the unit took damage.
pub const DAMAGED: i32 = crate::ai_kinds::NOTIFY_DAMAGED;
/// `699`: a unit is no longer doing an action on this one.
const NO_LONGER_ON_ME: i32 = crate::ai_kinds::NOTIFY_NO_LONGER_ON_ME;

/// `0x2db`: the order an INTERACT queues on its target (slot 0x230 runs it as the unit's slot
/// 0x128 on the target, case `0x2db`/`0x325` of `FUN_140c5c170`, which logs no line to name it).
pub const ORDER_ON_TARGET: i32 = 0x2db;

/// What a notify handler (unit-AI slot 0x238) returns: go on with the next notify.
const GO_ON: u32 = 2;
/// "Notify processing returned stop or new action, breaking."
const BREAK: u32 = 3;

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
    /// The unit's explore (slot 0x398, `FUN_141084db0`, with its argument 4 set) to the point
    /// (x, y): its Explore takes the point (slot 0x48), else a new Explore to it (`FUN_140a87d30`
    /// at the unit's height) replaces its actions (`FUN_140a98620`, slot 0x808).
    Explore([f32; 2]),
    /// The explorer's search (the base notify's case 0x202 on the auto scout's action `0x2bd`,
    /// `FUN_140c65490`): the caller finds the point ([`crate::explore::unexplored_area`]), gives
    /// the AI its target point and the unit [`Effect::Explore`] to it. No point found is not
    /// modelled (slot 0x90's search, then `FUN_140c4b130`).
    SeekUnexplored,
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
    /// The notify queue (`+0x28` count, `+0x30` array), oldest first. Its capacity (`+0x2c`)
    /// decides only where a notify posted while the queue is processed goes, which the slice
    /// never does.
    pub notifies: Vec<Notify>,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    },
    /// The search alone (slot 0x50 as a `NoAttack` soldier's idle check calls it): no object, and
    /// whether it found enemies (the alert mode).
    Look,
}

/// What a unit AI's attack check (its slot 0xa0) reads of itself: its owner's id, its state
/// (`+0x640`) and its master's class, `attack_priority` (`+0x92c`) and `missile_id` (`+0x83c`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attacker {
    /// The unit AI's type.
    pub ai_type: u32,
    /// The unit's player.
    pub player: usize,
    /// The unit's state.
    pub state: u8,
    /// Its master's class.
    pub class: i16,
    /// Its master's `attack_priority`.
    pub attack_priority: u8,
    /// Its master's missile, −1 for none.
    pub missile: i16,
    /// The game mode is a scenario's.
    pub scenario: bool,
}

/// What a unit AI's attack check reads of an object it found: the owner's id, the master's class,
/// type, id and `combat_level` (`+0x62c`), and the object's `+0x924`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

/// The classes an attacker of `attack_priority` 3 may attack (`FUN_140fddf10`).
const PRIORITY_3_CLASSES: [i16; 6] = [3, 27, 39, 49, 52, 60];

/// The masters a tower never attacks (`FUN_140c1a760`, raw `0x60ce`, `0x653f`).
const TOWER_SPARED: [i16; 2] = [50, 1187];

/// Slot 0xa0 of `me`'s unit AI, `CanAttack`: whether it may attack `t`. The tower's
/// (`FUN_140c1a760`): never for Gaia's own tower nor the classes 1 and 42; Gaia's objects only of
/// the classes 9 and 10, unless they are of the combat type with `+0x924` 0 outside the scenario
/// modes; a building's `desolid_flag` (`+0xc60`) is not held; never masters 50 and 1187, nor
/// from a tower in a state below 2. Class 10 with an attack priority below 3 only of combat
/// level 4 (`FUN_140fde120`); else when the tower is armed (`FUN_14120ea30`: classes 51 and 54,
/// or a missile; without one the object's `+0xa00` decides, not held) and its attack priority
/// is not 3 or the class one of [`PRIORITY_3_CLASSES`]. The soldier's (`FUN_140c19a20`) is not
/// modelled.
pub fn can_attack(me: &Attacker, t: &Target) -> Result<bool, Unsupported> {
    if me.ai_type != 15 {
        return Err(Unsupported("a soldier's attack check (slot 0xa0)"));
    }
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

/// The caller's answer to a [`Search`].
pub type Closest<'a> = dyn FnMut(&Search<'_>) -> Result<Found, Unsupported> + 'a;

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
    /// `+0x1fd`: its next idle check sends it home. Not saved; a loaded boar starts without it.
    pub go_home: bool,
}

/// What an idle check reads of its unit and the game.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
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
    /// The master's radii (`+0x5d8`, `+0x5dc`), line of sight (`+0x5d0`) and search radius
    /// (`+0x794`, the unit's slot 0x2c8).
    pub size: [f32; 2],
    /// The master's line of sight (`+0x5d0`).
    pub los: f32,
    /// The search radius (`+0x794`, the unit's slot 0x2c8).
    pub search_radius: f32,
    /// The unit's slot 0x2d0 with 0 (`FUN_14106d170`): its top action may be interrupted, or it
    /// has none.
    pub interruptible: bool,
    /// The master has a task of action type 0x99 (`FUN_1415c60b0`; the mule cart's): such a
    /// unit with an action or a target takes no queued order (`FUN_140c4eea0`).
    pub task_0x99: bool,
    /// An object of type 70 or more inside another (`+0x2f0`): it runs no queued order.
    pub garrisoned: bool,
    /// A building of the owner's in state 0 (a foundation) stands on a tile of the builder's
    /// search square (`FUN_140c1c7c0`: the larger of the line of sight and the search radius,
    /// truncated, each way from the unit's tile), whose search the slice does not run: without one
    /// it finds nothing.
    pub foundations: bool,
    /// The unit's state (`+0x640`), type (`+0xf`), `+0x924` and master id.
    pub state: u8,
    /// The unit's type (`+0xf`).
    pub kind: u8,
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
/// The class whose stop leaves the state position (`-0xe9a` raw, slot 0xb0).
const CLASS_18: i16 = 18;
/// The patrol orders (`0x2c7`, `0x2dd`), whose ends reset the idle and secondary timers.
const PATROL: i32 = 0x2c7;
const PATROL_2: i32 = 0x2dd;
/// Classes whose actions `0x26b`, `0x26c` take another branch of the idle part (`-0xea9`,
/// `-0xe78` raw).
const CLASS_3: i16 = 3;
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

/// The unit-AI type the factory (`FUN_141215a90`) gives a new object of a master of `class`:
/// none for classes 5, 7, 8, 11, 14, 15, 16, 30 to 34, 38, 42, 48 and 63, the civilian's (8)
/// for class 4. Masters 1105, 1744 and 1746 have their own, and the other classes' types are
/// not modelled.
pub const fn factory_type(class: i16, master: i16) -> Result<Option<u32>, Unsupported> {
    if matches!(master, 1105 | 1744 | 1746) {
        return Err(Unsupported("a new object of a master with its own unit AI"));
    }
    match class {
        5 | 7 | 8 | 11 | 14 | 15 | 16 | 30..=34 | 38 | 42 | 48 | 63 => Ok(None),
        4 => Ok(Some(8)),
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
            notifies: Vec::new(),
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

    /// The boar's idle check (slot 72, `FUN_140c1e630`): a furious boar, or any in mode 1, stays.
    /// Then one draw of `0..81` widens its leash: outside it, a boar not yet heading home goes
    /// home; inside, once in 7 checks (a second draw) it goes home from more than 2 tiles away, or
    /// wanders up to 3 tiles (two more draws). Its moves keep its height. `FUN_140c41e50`, first,
    /// lowers the unit's last human order time to 5000 (`FUN_140fe0d10`); the slice does not hold it.
    fn boar_idle(&mut self, at: &Surroundings, ai: &mut Well512) -> Option<AiMove> {
        let home = self.home?;
        if home.furious || at.mode_1 {
            return None;
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
            return m;
        }
        // comiss; jbe: unordered counts as inside
        if d2.partial_cmp(&leash) != Some(std::cmp::Ordering::Greater) {
            if !ai.rand15().is_multiple_of(BOAR_WANDER) {
                return None;
            }
            if d2 > BOAR_HOME {
                return self.move_to(go_home);
            }
            let step = |ai: &mut Well512| (ai.rand15() % BOAR_STEP) as i32 - BOAR_OFFSET;
            let nx = step(ai) as f32 + x;
            let ny = step(ai) as f32 + y;
            return self.move_to(AiMove {
                point: [nx, ny, z],
                ..go_home
            });
        }
        if self.target_point[0] == home.origin[0] || self.target_point[1] == home.origin[1] {
            return None;
        }
        self.move_to(go_home)
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
        closest: &mut Closest<'_>,
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
        let found = closest(&Search {
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
        closest: &mut Closest<'_>,
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
        let mut found = closest(&search(4))?;
        if found.id < 0 {
            found = closest(&search(-1))?;
        }
        self.alert(found, at.dt_seconds);
        if found.id != -1 {
            return Err(Unsupported(
                "a lion attacking an object it sees (slot 0xb8)",
            ));
        }
        Ok(None)
    }

    /// Slot 72 for the factory types modelled: the boar's, the hunted animal's, the lion's and
    /// the soldier's ([`UnitAi::soldier_idle`]) do something for an idle unit; each, and the
    /// transport ship's, first lowers the unit's last human order time to `FUN_140fe0d10`'s 5000
    /// (`FUN_140c41e50`; not on orders 701, 711, 712 and 733, and a group's would decide, the
    /// slice holds none), as does the tower's ([`UnitAi::tower_idle`]). The building's does
    /// nothing.
    fn process_idle(
        &mut self,
        at: &Surroundings,
        computer: bool,
        ai: &mut Well512,
        closest: &mut Closest<'_>,
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        if matches!(self.ai_type, 2 | 4 | 5 | 11 | 13 | 15)
            && !matches!(self.current_order, 0x2bd | 0x2c7 | 0x2c8 | 0x2dd)
        {
            fx.push(Effect::CapHumanOrder(HUMAN_ORDER_CAP));
        }
        match self.ai_type {
            13 => return self.soldier_idle(at, computer, closest),
            15 => return self.tower_idle(at, computer, closest),
            _ => {}
        }
        let moved = match self.ai_type {
            2 => self.hunted_idle(at, ai, closest)?,
            4 => self.boar_idle(at, ai),
            5 => self.lion_idle(at, ai, closest)?,
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
        closest: &mut Closest<'_>,
    ) -> Result<(), Unsupported> {
        let searches = !at.scenario && 70 <= at.kind && at.capture == 0;
        if at.player == 0 && !searches {
            return Ok(());
        }
        if self.stance == 3 {
            return self.no_attack_idle(at, computer, closest);
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
        let found = closest(&Search {
            at: [at.position[0], at.position[1]],
            radius: attack_search_radius(at),
            classes: &SOLDIER_CARED,
            size: at.size,
            pick: Pick::Attack {
                lowest: if at.scenario { 2 } else { 0 },
            },
        })?;
        if found.id != -1 {
            return Err(Unsupported(
                "a soldier attacking what its idle search found (slot 0xb8)",
            ));
        }
        if found.fresh {
            self.alert_mode = false;
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
        closest: &mut Closest<'_>,
    ) -> Result<(), Unsupported> {
        if self.current_order == DEFEND_ORDER {
            return Err(Unsupported(
                "a defend order's search (its target's objects)",
            ));
        }
        if computer {
            return Err(Unsupported("a computer player's soldier's NoAttack search"));
        }
        let found = closest(&Search {
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
    /// ([`Pick::Attack`], as [`UnitAi::soldier_idle`]'s): an object found would be attacked
    /// (slot 0xb8, which sets `+0x1d5`), which is not modelled. The search leaves the alert mode
    /// clear.
    fn tower_idle(
        &mut self,
        at: &Surroundings,
        computer: bool,
        closest: &mut Closest<'_>,
    ) -> Result<(), Unsupported> {
        if self.stance == 3 {
            return Err(Unsupported("a tower's idle search in NoAttack (stance 3)"));
        }
        let found = closest(&Search {
            at: [at.position[0], at.position[1]],
            radius: attack_search_radius(at),
            classes: &TOWER_CARED,
            size: at.size,
            pick: Pick::Attack {
                lowest: if at.scenario && !computer { 2 } else { 0 },
            },
        })?;
        if found.id != -1 {
            return Err(Unsupported(
                "a tower attacking what its idle search found (slot 0xb8)",
            ));
        }
        if found.fresh {
            self.alert_mode = false;
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
        closest: &mut Closest<'_>,
    ) -> Result<Vec<Effect>, Unsupported> {
        let retarget = retarget_kind(self.ai_type)?;
        // the full update's start: the time since the last enemy sighting grows by the world's dt
        self.time_since_enemy += at.dt_seconds;
        let mut fx = Vec::new();
        self.target_block(at, computer, &mut fx)?;
        self.check_busy_order(retarget, at.busy && fx.is_empty())?;
        if self.current_order != -1 {
            self.state_position = [at.position[0], at.position[1]];
        }
        self.secondary_timer = self.secondary_timer.saturating_sub(dt);
        self.look_around_timer = self.look_around_timer.wrapping_add(dt);
        self.idle_timer = self.idle_timer.wrapping_add(dt);
        let result = self.process_notifies(dt, at, computer, &mut fx)?;
        self.end_notifies(dt, at, &mut fx)?;
        // slot 0x430 again: a stop emptied the action list
        let stopped = fx.contains(&Effect::Stop);
        let busy = at.busy && !stopped;
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
            self.run_orders(&mut fx)?;
        }
        if matches!(retarget, Retarget::Base) {
            self.retarget_check(world_time, at, ai, &mut fx)?;
        }
        self.idle_part(busy, at, computer, ai, closest, &mut fx)?;
        self.look_around(computer, ai)?;
        // the boar's slot 4 (0x140c2c010) after the base: its second idle check runs only for a
        // timer the base's left past the timeout (a unit with an action), and a boar without a
        // target calms down
        if let Some(home) = self.home
            && !home.furious
            && self.adjusted_idle_timeout < self.idle_timer
        {
            self.idle_timer = 0;
            self.process_idle(at, computer, ai, closest, &mut fx)?;
        }
        self.calm_down();
        Ok(fx)
    }

    /// The target block at the full update's start (`0x140c66b10`): a unit inside another
    /// (`+0x2f0`) drops its target, as does one whose target no longer exists (an attack action's
    /// idle timer then takes its timeout; `FUN_140c5e1e0` finds no target left). A dead target (a
    /// state above 2, not a building) of a live unit ends an action that needs a live one
    /// ([`LIVE_TARGET_ACTIONS`]) or any action with `+0x1a8` set: `+0x1a8` clears and, but for a
    /// hunt or `0x2be`, the target goes and the action stops (slot 0xb0 with 1). An attack's next
    /// target (slots 0x2b0, 0x240), the patrol range (`FUN_140c41690`), a computer's priority by
    /// the target's distance and action `0x2be`'s check (`FUN_140c40f10`) are not modelled; the
    /// slice holds no unit groups (`+0x6d0`).
    fn target_block(
        &mut self,
        at: &Surroundings,
        computer: bool,
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
                    if action == ATTACK_ACTION {
                        return Err(Unsupported("an attack action whose target died"));
                    }
                    if self.stop_after_kill || LIVE_TARGET_ACTIONS.contains(&action) {
                        self.stop_after_kill = false;
                        if action != HUNT_ACTION && action != BUILD_ORDER {
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

    /// The units the update models after its target block ([`UnitAi::target_block`]): a target
    /// only on a villager's build or hunt (702/602, 713/613), on the follow order (712 with its
    /// action 612, the move's 610 or none) or without an order and an action (a hunt ended by
    /// its queued orders); busy (`busy`, the actions left), only those or a unit
    /// on MOVE's order without a target (710/610, or an animal's own wander), and for the types
    /// that never retarget also one whose move ended (710 with no action value).
    fn check_busy_order(&self, retarget: Retarget, busy: bool) -> Result<(), Unsupported> {
        let order = (self.current_order, self.current_action);
        let working = matches!(
            order,
            (BUILD_ORDER, BUILD_ACTION)
                | (HUNT_ORDER, HUNT_ACTION)
                | (FOLLOW_ORDER, FOLLOW_ACTION | MOVE_ACTION | -1)
        );
        let moving = matches!(order, (MOVE_ORDER | -1, MOVE_ACTION)) && self.current_target == -1;
        // a move whose action ended (the action value reset) while the default action runs
        let moved = order == (MOVE_ORDER, -1) && self.current_target == -1;
        let scouting = order == (AUTO_SCOUT_ORDER, AUTO_SCOUT_ACTION) && self.current_target == -1;
        let following = order.0 == FOLLOW_ORDER;
        // without an order and an action the target is a stale one: the order queue's next order
        // takes over
        if self.current_target != -1 && !working && order != (-1, -1) {
            return Err(Unsupported("a unit AI's target the slice does not model"));
        }
        if !busy || self.is_idle() {
            return Ok(());
        }
        match retarget {
            Retarget::Never if working || moving || moved => Ok(()),
            Retarget::Base if moving || scouting || following => Ok(()),
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
        fx: &mut Vec<Effect>,
    ) -> Result<u32, Unsupported> {
        let mut result = GO_ON;
        while !self.notifies.is_empty() && result == GO_ON {
            let mut i = 0;
            while i < self.notifies.len() {
                let n = self.notifies[i];
                result = self.handle(n, dt, at, computer, fx)?;
                if result == BREAK || result == 0xd {
                    break;
                }
                i += 1;
            }
            if result == GO_ON {
                self.end_notifies(dt, at, fx)?;
            }
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
    /// A notify the AI does not take (any but 699) goes to the owner's slot 0x2e8
    /// (`FUN_14162dab0`, the human player's alerts), which the slice does not hold; 699 needs the
    /// attackers list, which it does not hold either.
    pub fn post(&mut self, n: Notify, unit_state: u8) -> Result<(), Unsupported> {
        if unit_state != 2 {
            return Ok(());
        }
        let action = self.current_action;
        let taken = match n.kind {
            NO_LONGER_ON_ME => return Err(Unsupported("a notify 699 (the attackers list)")),
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
            self.notifies.push(n);
        }
        Ok(())
    }

    /// Slot 0x238 per factory type: each type's handler, most of which pass the notify on to the
    /// base handler ([`UnitAi::base_notify`]).
    fn handle(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        computer: bool,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, Unsupported> {
        match self.ai_type {
            // TribeHuntedAnimalUnitAIModule (FUN_140c26420): its damage notify evades
            2 => match n.kind {
                DAMAGED => Err(Unsupported("a hunted animal's damage notify (its evade)")),
                0x20f => Ok(GO_ON),
                _ => self.base_notify(n, time, at, fx),
            },
            // the boar (FUN_140c25770): its damage notify attacks
            4 => match n.kind {
                DAMAGED => Err(Unsupported("a boar's damage notify (its attack)")),
                ACTION_COMPLETED if self.current_action == 0x2bf => {
                    Err(Unsupported("a boar's completion of action 0x2bf"))
                }
                0x20f => Ok(GO_ON),
                _ => self.base_notify(n, time, at, fx),
            },
            // the lion (FUN_140c266f0)
            5 => match n.kind {
                ACTION_COMPLETED if n.p[0] == 600 => Err(Unsupported("a lion's completed attack")),
                0x1fc => Err(Unsupported("a lion's moved target")),
                _ => self.base_notify(n, time, at, fx),
            },
            // the villager (FUN_140c22bf0): a completion or failure of its action stops it, and slot 0x48
            // passes the notify to its owner's slot 0x2e8 (not held); a failed build (0x202, its
            // Build waiting without work, action 0x25a) looks for another foundation to build
            // (FUN_140c1c7c0, when the player's +0x613 is 0: unsaved, 0 in this game) and, finding
            // none, a human player's villager stops (slot 0x3c0) and the processing breaks
            8 => match n.kind {
                0x202 if self.current_action == BUILD_ACTION => {
                    if at.foundations {
                        return Err(Unsupported(
                            "a builder's search that finds another foundation (FUN_140c1c7c0)",
                        ));
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
                ACTION_EXECUTED => self.base_notify(n, time, at, fx),
                _ => Err(Unsupported("a villager's notify the slice does not model")),
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
                _ => self.base_notify(n, time, at, fx),
            },
            // the tower (FUN_140c299f0)
            15 => match n.kind {
                DAMAGED => Err(Unsupported("a tower's damage notify")),
                ACTION_COMPLETED if n.p[0] == 600 && self.current_order == 700 => {
                    Err(Unsupported("a tower's completed attack"))
                }
                _ => self.base_notify(n, time, at, fx),
            },
            // the building (FUN_140c22610): most notifies end there
            17 => match n.kind {
                DAMAGED | 600 | NO_LONGER_ON_ME => Err(Unsupported("a building's attack notify")),
                0x202 | 0x216 | 0x21b => self.base_notify(n, time, at, fx),
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
    fn base_notify(
        &mut self,
        n: Notify,
        time: u32,
        at: &Surroundings,
        fx: &mut Vec<Effect>,
    ) -> Result<u32, Unsupported> {
        match n.kind {
            ACTION_COMPLETED => self.action_completed(n, at, fx),
            crate::explore::WANTS_POINT if self.explorer() => self.seek_unexplored(fx),
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
            _ => Err(Unsupported("a notify kind the base handler does not model")),
        }
    }

    /// The action values of an explorer (the base handler's case 0x202): 605, 623 and 630, and
    /// the auto scout's `0x2bd`.
    const fn explorer(&self) -> bool {
        let a = self.current_action;
        let k = a.wrapping_sub(EXPLORE_ACTION) as u32;
        (k < 0x1a && (0x204_0001u32 >> (k & 0x1f)) & 1 != 0) || a == AUTO_SCOUT_ACTION
    }

    /// The base handler's case 0x202 for an explorer ("I'm an explorer, looking for the closest
    /// undiscovered tile."): on the auto scout's action, with the player's explored-points grid
    /// never made (`player+0x1370`), the search around the town centre (`FUN_140c65490`,
    /// [`Effect::SeekUnexplored`]; the caller finds the point, its absence is not modelled), then
    /// slot 0x100 with 1 to it ([`UnitAi::explore_at`]: the explore's action value becomes the
    /// last) and the action value back; processing breaks. The other explorers' search (slot
    /// 0x90) is not modelled.
    fn seek_unexplored(&mut self, fx: &mut Vec<Effect>) -> Result<u32, Unsupported> {
        let action = self.current_action;
        if action != AUTO_SCOUT_ACTION {
            return Err(Unsupported(
                "an explorer's search for the closest undiscovered tile (slot 0x90)",
            ));
        }
        fx.push(Effect::SeekUnexplored);
        self.last_action = std::mem::replace(&mut self.current_action, EXPLORE_ACTION);
        self.last_target = self.current_target;
        self.last_target_type = self.target_type;
        self.current_target = -1;
        self.last_action = std::mem::replace(&mut self.current_action, action);
        Ok(BREAK)
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
    const fn remove_target(&mut self) {
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
    fn stop_with(
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
            self.run_orders(&mut fx)?;
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
    pub fn run_orders(&mut self, fx: &mut Vec<Effect>) -> Result<(), Unsupported> {
        let mut k = 0;
        while k < self.orders.len() {
            if self.current_action != -1 && self.orders[k].priority < 100 {
                return Ok(());
            }
            let order = self.orders[k];
            let r = self.process_order(&order, fx)?;
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
    /// taken. Other orders are not modelled.
    fn process_order(&mut self, o: &QueuedOrder, fx: &mut Vec<Effect>) -> Result<u32, Unsupported> {
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

    /// The explorer's point (the base notify's case 0x202 after [`Effect::SeekUnexplored`]): the
    /// AI's target point.
    pub const fn explore_point(&mut self, x: i32, y: i32) {
        self.target_point = [x as f32, y as f32, EXPLORE_Z];
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
        if self.process_order(&order, fx)? != 0xe {
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
        closest: &mut Closest<'_>,
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        let class_20 = busy && at.class == CLASS_20;
        if !class_20
            && self.adjusted_idle_timeout < self.idle_timer
            && self.current_action == -1
            && (self.current_target == -1 || !busy)
        {
            self.idle_timer = 0;
            self.process_idle(at, computer, ai, closest, fx)?;
            return Ok(());
        }
        if matches!(self.current_order, PATROL | PATROL_2) {
            return Err(Unsupported("a patrol order's idle part"));
        }
        if matches!(at.class, CLASS_3 | CLASS_52) && matches!(self.current_action, 0x26b | 0x26c) {
            return Err(Unsupported("a unit of class 3 or 52 on action 0x26b/0x26c"));
        }
        if self.current_action == 0x2c0 {
            return Err(Unsupported("the idle part of action 0x2c0"));
        }
        if (self.current_action != -1 || self.current_target != -1) && 0 < at.player {
            self.idle_timer = 0;
        }
        Ok(())
    }

    /// Slot 73 (`0x140c567c0`, processMisc). Its order part first, by the order (an action's
    /// value plus 100 without one): the follow order's ([`UnitAi::follow`]); the defend order's
    /// is not modelled. Then the leash checks: stance 1's is not modelled; stance 2's (stand
    /// ground) stops a unit with an action that strayed from its state position by more than its
    /// range plus `DAT_142eb880c` (0.25), which a unit on an order, whose state position this
    /// update set, never has, and an idle unit has no action to stop. Then the retarget check,
    /// not on the attack order 700: with retargeting on (`+0x1d6` 0), a timer used up is redrawn
    /// (`750 + r % 500`); a unit with a target of Gaia's ("I have a GAIA target, so I can't
    /// retarget.") or without a target stops there. A target of a player's is not modelled.
    fn retarget_check(
        &mut self,
        world_time: u32,
        at: &Surroundings,
        ai: &mut Well512,
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        let order = match (self.current_order, self.current_action) {
            (-1, -1) => -1,
            (-1, action) => action + 100,
            (order, _) => order,
        };
        if order == DEFEND_ORDER {
            return Err(Unsupported("a defend order's processMisc"));
        }
        if order == FOLLOW_ORDER && self.follow(at, fx)? {
            return Ok(());
        }
        if self.stance == 1 {
            return Err(Unsupported("stance 1 leash check"));
        }
        if self.current_order == 700 {
            return Err(Unsupported("a retarget on order 700"));
        }
        if self.retarget_disabled != 0 {
            return Ok(());
        }
        if world_time.wrapping_sub(self.last_retarget_time) < self.retarget_timer {
            return Ok(());
        }
        self.last_retarget_time = world_time;
        self.retarget_timer = 750 + ai.rand15() % 500;
        if self.current_target != -1 {
            match at.target {
                Some(t) if t.owner == 0 => {}
                Some(_) => return Err(Unsupported("a retarget with a player's target")),
                None => return Err(Unsupported("a retarget with a target the world lacks")),
            }
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
            closest: &mut Closest<'_>,
        ) -> Result<Option<AiMove>, Unsupported> {
            if !self.is_idle() {
                return Err(Unsupported("unit is not idle"));
            }
            self.update(dt, world_time, computer, at, ai, closest)
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
        let mut defensive = UnitAi {
            stance: 1,
            ..idle(13)
        };
        assert!(
            defensive
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
                notifies: Vec::new(),
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
        u.target_block(&standing, false, &mut fx).unwrap();
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
        u.target_block(&gone, false, &mut fx).unwrap();
        assert_eq!((u.current_target, u.target_type), (-1, 3));
        assert_eq!(u.idle_timer, 3000);
        // inside another, a unit drops its target before the lookup
        let inside = Surroundings {
            garrisoned: true,
            ..standing
        };
        let mut u = builder_done();
        u.target_block(&inside, false, &mut fx).unwrap();
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
        u.target_block(&dead, false, &mut fx).unwrap();
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
        u.target_block(&dead, false, &mut fx).unwrap();
        assert_eq!((u.current_target, fx), (10028, vec![]));
        // an attack whose target died retargets, which is not modelled
        let mut u = UnitAi {
            current_action: ATTACK_ACTION,
            ..builder_done()
        };
        assert!(u.target_block(&dead, false, &mut Vec::new()).is_err());
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
        assert_eq!(u.base_notify(n, 0, &at, &mut Vec::new()), Ok(GO_ON));
        u.patrol_path = true;
        assert!(u.base_notify(n, 0, &at, &mut Vec::new()).is_err());
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
        assert_eq!(searched, vec![(5.0, 39, Pick::Attack { lowest: 0 })]);
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

    #[test]
    fn a_tower_passes_a_boar_but_not_a_wolf() {
        // player 2's TC 9156 in update 1506 and Gaia's boar 9731 (master 48, class 10)
        let tc = Attacker {
            ai_type: 15,
            player: 2,
            state: 2,
            class: 3,
            attack_priority: 0,
            missile: 54,
            scenario: false,
        };
        let boar = Target {
            owner: 0,
            class: 10,
            kind: 70,
            master: 48,
            combat_level: 0,
            capture: 0,
        };
        assert_eq!(can_attack(&tc, &boar), Ok(false));
        let wolf = Target {
            combat_level: 4,
            ..boar
        };
        assert_eq!(can_attack(&tc, &wolf), Ok(true));
        // Gaia's tower attacks nothing; a Gaia object outside classes 9 and 10 that takes owners
        // by sight neither
        assert_eq!(can_attack(&Attacker { player: 0, ..tc }, &wolf), Ok(false));
        let sheep = Target {
            class: 58,
            capture: 3,
            ..boar
        };
        assert_eq!(can_attack(&tc, &sheep), Ok(false));
        // the soldier's check is not modelled
        assert!(can_attack(&Attacker { ai_type: 13, ..tc }, &boar).is_err());
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
