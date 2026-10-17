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
    pub caller: i32,
    pub recipient: i32,
    pub kind: i32,
    pub p: [i32; 3],
}

impl Notify {
    /// The notify an object posts to itself.
    pub fn own(id: i32, kind: i32, p: [i32; 3]) -> Self {
        Notify {
            caller: id,
            recipient: id,
            kind,
            p,
        }
    }

    /// The record as the save holds it.
    pub fn from_bytes(b: &[u8; 24]) -> Self {
        let w = |i: usize| i32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        Notify {
            caller: w(0),
            recipient: w(4),
            kind: w(8),
            p: [w(12), w(16), w(20)],
        }
    }
}

/// The notify kinds the slice posts or processes (named by the game's log strings).
/// `0x1fa`: "Action %d has completed" (p1 the action value).
pub const ACTION_COMPLETED: i32 = 0x1fa;
/// `0x1fb`: an action's invalidation (p1 the action value).
pub const ACTION_INVALIDATED: i32 = 0x1fb;
/// `0x200`: "ActionExecuted", an attack's swing (p1 600, p2 the target).
pub const ACTION_EXECUTED: i32 = 0x200;
/// `0x20b`: the unit changed hands (p1 the new owner).
pub const OWNER_CHANGED: i32 = 0x20b;
/// `500`: "I took some damage".
pub const DAMAGED: i32 = 500;
/// `699`: "Unit #%d is no longer doing an action on me".
const NO_LONGER_ON_ME: i32 = 699;

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
    /// `+0x4`: the order value (`0x2c6` a move).
    pub order: i32,
    /// `+0x8`: below 100 it waits while the unit AI has an action.
    pub priority: i32,
    /// `+0xc`, `+0x10`: the target and a second id (−1 for none).
    pub target: i32,
    pub target2: i32,
    /// `+0x14`..`+0x1c`: the point; `+0x20`: the range.
    pub point: [f32; 3],
    pub range: f32,
    /// `+0x24`, `+0x28`: the point the unit's slot 0x2d8 shows (not held).
    pub shown: [f32; 2],
    /// `+0x2c` (a byte): 0 leaves the order in the queue.
    pub ready: bool,
    /// `+0x30`.
    pub extra: i32,
}

impl QueuedOrder {
    /// The record as the save holds it (`+0x2d`..`+0x2f` are padding).
    pub fn from_bytes(b: &[u8; 52]) -> Self {
        let w = |i: usize| i32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let f = |i: usize| f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        QueuedOrder {
            issuer: w(0),
            order: w(4),
            priority: w(8),
            target: w(12),
            target2: w(16),
            point: [f(20), f(24), f(28)],
            range: f(32),
            shown: [f(36), f(40)],
            ready: b[44] != 0,
            extra: w(48),
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
}

/// The first move among a full update's effects.
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
    pub target_point: [f32; 3],
    /// `+0x64`, `+0x68`, `+0x6c`, `+0x70`: the order, action, target and target class before the
    /// current ones.
    pub last_order: i32,
    pub last_action: i32,
    pub last_target: i32,
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

/// A unit AI's `ClosestObject` (`FUN_140c488b0`) as the animals' idle checks call it: its search
/// (`FUN_140c63e20`) queries its player's objects in sight of `classes` within `radius` of `at`
/// for a searcher of radii `size` (`GetVisibleUnits`, mode 0, every relation); then the objects of
/// master class `group`, of relation `relation` (0 itself or friends, 1 neutrals, 2 enemies, 3
/// itself) and in state `state` (each −1 for any), reachable ones only. The caller answers with
/// the closest object's id, −1 for none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Search<'a> {
    pub at: [f32; 2],
    pub radius: f32,
    pub classes: &'a [i32],
    pub size: [f32; 2],
    pub group: i32,
    pub relation: i32,
    pub state: i32,
}

/// The caller's answer to a [`Search`].
pub type Closest<'a> = dyn FnMut(&Search<'_>) -> Result<i32, Unsupported> + 'a;

/// A search that never runs (units without a search).
pub fn no_search(_: &Search<'_>) -> Result<i32, Unsupported> {
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
    pub los: f32,
    pub search_radius: f32,
    /// The unit's slot 0x2d0 with 0 (`FUN_14106d170`): its top action may be interrupted, or it
    /// has none.
    pub interruptible: bool,
    /// The master has a task of action type 0x99 (`FUN_1415c60b0`; the mule cart's): such a
    /// unit with an action or a target takes no queued order (`FUN_140c4eea0`).
    pub task_0x99: bool,
    /// An object of type 70 or more inside another (`+0x2f0`): it runs no queued order.
    pub garrisoned: bool,
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
/// 0x340): a MoveTo to `point` within `range`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AiMove {
    pub point: [f32; 3],
    pub range: f32,
    /// The sixth argument: the move replaces even a MoveTo to the same point.
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

fn retarget_kind(ai_type: u32) -> Result<Retarget, Unsupported> {
    match ai_type {
        // villagers (0x140c21da0) and livestock (0x140c21f20) return 9 from slot 73
        8 | 19 => Ok(Retarget::Never),
        // hunted animal (2, slot 72 FUN_140c1eea0), boar (4, slot 72 FUN_140c1e630), lion (5,
        // slot 72 FUN_140c1fb70), transport ship (11, slot 72 0x140c21d20), soldier (13), tower
        // (15), building (17); the slot 72 of the ship, the soldier, the tower and the building
        // draws nothing for an idle unit
        2 | 4 | 5 | 11 | 13 | 15 | 17 => Ok(Retarget::Base),
        _ => Err(Unsupported("unit-AI type not modelled")),
    }
}

/// The base idle timeout (`+0x168`) the constructor sets; the adjusted one is drawn around it.
const IDLE_TIMEOUT: u32 = 3000;

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
pub const MOVE_ORDER: i32 = 0x2c6;
pub const MOVE_ACTION: i32 = 0x262;

/// The hunted animal's idle check (`FUN_140c1eea0`): away from its origin by more than
/// `DAT_142eb0f48` (4, squared) it goes home once in 7 checks; otherwise it wanders 1 to 3 tiles
/// each way (`HUNTED_STEP`), in mode 1 once in 28 and by a tile. Its search reaches its line of
/// sight plus `DAT_14330eb8c` (1), or its search radius if that is as far, plus `+0x1e8` (unsaved,
/// 0 in every unit probed).
const HUNTED_HOME: f32 = 4.0;
const HUNTED_STEP: i32 = 3;
const HUNTED_SIGHT_GROWTH: f32 = 1.0;
/// What an animal's idle check lowers its last human order time to (`FUN_140fe0d10`).
const HUMAN_ORDER_CAP: u32 = 5000;
/// The lion's idle timeout (`FUN_140c1fb70`, as its constructor `FUN_140c11bd0`).
const LION_IDLE_TIMEOUT: u32 = 6000;
/// The class of the king's branch of the hunted animal's idle check (`-0xe71` raw).
const CLASS_KING: i16 = 59;

/// The action values the full update resets once the unit's actions ended (`0x140c66b10`, the
/// unit's slot 0x430 false): attack, `0x261`, move, hunt, `0x269`, `0x26b`, `0x26c`, build and
/// `0x2bf`.
const ENDED_ACTIONS: [i32; 9] = [600, 0x261, 0x262, 0x265, 0x269, 0x26b, 0x26c, 0x2be, 0x2bf];

/// `UpdateAdjustedIdleTimeout` (`0x140c41e00`): `r % (base / 2) + 3 * base / 4`, one AI draw.
fn adjusted_idle_timeout(base: u32, ai: &mut Well512) -> u32 {
    ai.rand15() % (base >> 1) + ((base * 3) >> 2)
}

impl UnitAi {
    /// The unit AI a new object gets. `UnitAIModule`'s constructor (`0x140c2e080`) draws three
    /// AI-stream values, in this order: the last retarget time `r % 500`, the adjusted idle timeout
    /// `r % 1500 + 2250` and the retarget timer `750 + r % 500`. Only the building class (17,
    /// `TribeBuildingUnitAIModule` `0x140c11470`, which sets the stance to 0) is modelled; for any
    /// other type nothing is drawn.
    pub fn constructed(
        ai_type: u32,
        position: [f32; 3],
        ai: &mut Well512,
    ) -> Result<UnitAi, Unsupported> {
        if ai_type != 17 {
            return Err(Unsupported("constructor of a unit-AI type not modelled"));
        }
        let last_retarget_time = ai.rand15() % 500;
        let adjusted_idle_timeout = adjusted_idle_timeout(IDLE_TIMEOUT, ai);
        let retarget_timer = 750 + ai.rand15() % 500;
        Ok(UnitAi {
            ai_type,
            current_order: -1,
            order_priority: -1,
            current_action: -1,
            current_target: -1,
            target_type: -1,
            target_point: [-1.0; 3],
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
            // the constructor's two bytes at `+0x1a8`: 0x300
            stance: 3,
            // the object's place
            state_position: [position[0], position[1]],
            retarget_disabled: 0,
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
    fn update_adjusted_idle_timeout(&mut self, ai: &mut Well512) {
        self.adjusted_idle_timeout = if self.ai_type == 2 {
            self.idle_timeout.wrapping_mul(5) >> 2
        } else {
            adjusted_idle_timeout(self.idle_timeout, ai)
        };
    }

    /// The order setter with its history (`FUN_140c64b60` with 1): leaving order 711 or 733
    /// redraws an adjusted idle timeout of exactly 750 and caps the secondary timer at 750.
    fn set_order(&mut self, order: i32, ai: &mut Well512) {
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

    /// `FUN_140c64b50`.
    fn set_action(&mut self, action: i32) {
        self.last_action = self.current_action;
        self.current_action = action;
    }

    /// `FUN_140c64be0`: the target, its class and its point.
    fn set_target(&mut self, target: i32, class: i32, point: [f32; 3]) {
        self.last_target = self.current_target;
        self.last_target_type = self.target_type;
        self.target_type = class;
        self.target_point = point;
        self.current_target = target;
    }

    /// The work command's notify of its new action (`FUN_14106e4e0`, object slot 0x828): the unit
    /// AI takes the order and action values of the action's type, and the target with its class
    /// (None for a point) and the order's point (`FUN_140c64b60`, `FUN_140c64b50`,
    /// `FUN_140c64be0`); then `FUN_140c64e50` clears the notifies with time 0 (`FUN_140c5dc10`;
    /// for a unit of type 70 or more only with its `+0xa14`, which is 1 but inside slot 0x128's
    /// work command, where INTERACT passes 1 too). The order history (`FUN_140c42080`) is not
    /// held.
    pub fn notify_work(
        &mut self,
        order: i32,
        action: i32,
        target: Option<(i32, i32)>,
        point: [f32; 3],
        ai: &mut Well512,
    ) -> Result<(), Unsupported> {
        self.set_order(order, ai);
        self.set_action(action);
        let (id, class) = target.unwrap_or((-1, -1));
        self.set_target(id, class, point);
        self.clear_notifies(0)
    }

    /// MOVE's notify (`FUN_140e67800`): a unit that took the move as a MoveTo (`moved`) gets the
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

    /// The AI's move (slot 0x130, `FUN_140c55620`): with `force` or no action value, the unit takes
    /// it (its slot 0x340, which the caller runs), and the AI holds the move action and the point
    /// without a target (`FUN_140c64c10` with −1, which leaves the target class).
    fn move_to(&mut self, m: AiMove) -> Option<AiMove> {
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
            group: -1,
            relation: -1,
            state: -1,
        })?;
        if found != -1 {
            return Err(Unsupported(
                "a hunted animal evading an object it sees (slot 0x150)",
            ));
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
            group,
            relation: 2,
            state: 2,
        };
        let mut found = closest(&search(4))?;
        if found < 0 {
            found = closest(&search(-1))?;
        }
        if found != -1 {
            return Err(Unsupported(
                "a lion attacking an object it sees (slot 0xb8)",
            ));
        }
        Ok(None)
    }

    /// Slot 72 for the factory types modelled: the boar's, the hunted animal's and the lion's do
    /// something for an idle unit; each first lowers the unit's last human order time to
    /// `FUN_140fe0d10`'s 5000 (`FUN_140c41e50`; not on orders 701, 711, 712 and 733, and a
    /// group's would decide, the slice holds none).
    fn process_idle(
        &mut self,
        at: &Surroundings,
        ai: &mut Well512,
        closest: &mut Closest<'_>,
        fx: &mut Vec<Effect>,
    ) -> Result<(), Unsupported> {
        if matches!(self.ai_type, 2 | 4 | 5)
            && !matches!(self.current_order, 0x2bd | 0x2c7 | 0x2c8 | 0x2dd)
        {
            fx.push(Effect::CapHumanOrder(HUMAN_ORDER_CAP));
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

    fn is_idle(&self) -> bool {
        self.current_order == -1 && self.current_action == -1 && self.current_target == -1
    }

    /// The full update (slot 4, `0x140c66b10`; the boar's `0x140c2c010` adds its tail):
    /// `dt` is the time the owner accumulated since its last turn (`world+0x18c[player]`),
    /// `computer` the owner's type test (`player+0x10 == 3`). In the game's order: the state
    /// position of a unit with an order, the timers, the notifies ([`UnitAi::post`]: each through
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
        self.check_busy_order(at, retarget)?;
        let mut fx = Vec::new();
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
            self.reset_ended_action(at)?;
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
        if let Retarget::Base = retarget {
            self.retarget_check(world_time, ai)?;
        }
        self.idle_part(busy, at, ai, closest, &mut fx)?;
        self.look_around(computer, ai)?;
        // the boar's slot 4 (0x140c2c010) after the base: its second idle check runs only for a
        // timer the base's left past the timeout (a unit with an action), and a boar without a
        // target calms down
        if let Some(home) = self.home
            && !home.furious
            && self.adjusted_idle_timeout < self.idle_timer
        {
            self.idle_timer = 0;
            self.process_idle(at, ai, closest, &mut fx)?;
        }
        self.calm_down();
        Ok(fx)
    }

    /// The busy units the update models: a villager building or hunting (702/602, 713/613) or a
    /// unit on MOVE's order without a target (710/610, or an animal's own wander). The target
    /// block at the update's start (a target that died, the patrol range) is not modelled: a
    /// building's or a hunted tree's target never counts as dead there (both actions are
    /// exempt).
    fn check_busy_order(&self, at: &Surroundings, retarget: Retarget) -> Result<(), Unsupported> {
        let order = (self.current_order, self.current_action);
        let working = matches!(
            order,
            (BUILD_ORDER, BUILD_ACTION) | (HUNT_ORDER, HUNT_ACTION)
        );
        let moving = matches!(order, (MOVE_ORDER | -1, MOVE_ACTION)) && self.current_target == -1;
        if self.current_target != -1 && !working {
            return Err(Unsupported("a unit AI's target the slice does not model"));
        }
        if self.current_target != -1 && !at.busy {
            return Err(Unsupported(
                "a unit AI's target once its actions ended (the target block)",
            ));
        }
        if !at.busy || self.is_idle() {
            return Ok(());
        }
        match retarget {
            Retarget::Never if working || moving => Ok(()),
            Retarget::Base if moving => Ok(()),
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
            // the villager (FUN_140c22bf0): a completion of its action stops it, and slot 0x48
            // passes the notify to its owner's slot 0x2e8 (not held)
            8 => match n.kind {
                ACTION_COMPLETED => {
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
            OWNER_CHANGED => {
                fx.push(Effect::Stop);
                self.remove_target();
                self.stop_current(true, at, fx)?;
                self.end_notifies(time, at, fx)?;
                self.orders.clear();
                Ok(GO_ON)
            }
            // "I'm ignoring that."
            0x1f6 | 0x1f7 | 0x1f8 | 0x1ff | ACTION_EXECUTED | 0x203 => Ok(GO_ON),
            0x259..=0x262 | 0x265..=0x269 | 0x2be => Ok(GO_ON),
            _ => Err(Unsupported("a notify kind the base handler does not model")),
        }
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
    fn remove_target(&mut self) {
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
            if at.class != CLASS_18 {
                self.state_position = [at.position[0], at.position[1]];
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
    fn reset_ended_action(&mut self, at: &Surroundings) -> Result<(), Unsupported> {
        if ENDED_ACTIONS.contains(&self.current_action) {
            if self.current_action == 0x2bf && self.stance == 2 {
                self.state_position = [at.position[0], at.position[1]];
            }
            self.current_action = -1;
        }
        Ok(())
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
    fn can_order(&self, at: &Surroundings) -> Result<bool, Unsupported> {
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
            self.orders.remove(k);
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
    /// timer). Other orders are not modelled.
    fn process_order(&mut self, o: &QueuedOrder, fx: &mut Vec<Effect>) -> Result<u32, Unsupported> {
        if !o.ready {
            return Ok(0xe);
        }
        if o.order != MOVE_ORDER {
            return Err(Unsupported("a queued order other than a move"));
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
        self.current_order = o.order;
        self.order_priority = o.priority;
        Ok(1)
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
            self.process_idle(at, ai, closest, fx)?;
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

    /// The retarget check of slot 73 (`0x140c567c0`) for a unit without a target, not on the
    /// attack order 700: a timer used up is redrawn (`750 + r % 500`) and slot 73 returns. Its
    /// leash checks come first: stance 1's is not modelled; stance 2's (stand ground) stops a
    /// unit with an action that strayed from its state position by more than its range plus
    /// `DAT_142eb880c` (0.25), which a unit on an order, whose state position this update set,
    /// never has, and an idle unit has no action to stop.
    fn retarget_check(&mut self, world_time: u32, ai: &mut Well512) -> Result<(), Unsupported> {
        if self.stance == 1 {
            return Err(Unsupported("stance 1 leash check"));
        }
        if self.current_order == 700 || self.current_target != -1 {
            return Err(Unsupported("a retarget on order 700 or with a target"));
        }
        if self.retarget_disabled == 0
            && world_time.wrapping_sub(self.last_retarget_time) >= self.retarget_timer
        {
            self.last_retarget_time = world_time;
            self.retarget_timer = 750 + ai.rand15() % 500;
        }
        Ok(())
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
    fn look_around(&mut self, computer: bool, ai: &mut Well512) -> Result<(), Unsupported> {
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
pub const BUILD_ACTION: i32 = 602;
/// The hunt order (`0x2c9`) and action value (`0x265`) of a villager's unit AI.
pub const HUNT_ORDER: i32 = 713;
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
        u.update(50, 0, false, &at, &mut r, &mut no_search).unwrap();
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
        u.update_idle(
            50,
            899,
            false,
            &Surroundings::default(),
            &mut r,
            &mut no_search,
        )
        .unwrap();
        assert_eq!((u.retarget_timer, r.draws), (900, 0));
        u.update_idle(
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
        u.update_idle(
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
            u.update_idle(
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
        u.update_idle(
            1000,
            0,
            false,
            &Surroundings::default(),
            &mut r,
            &mut no_search,
        )
        .unwrap();
        assert_eq!((u.look_around_timer, r.draws), (1000, 0));
        u.update_idle(
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
        u.update_idle(
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
        u.update_idle(
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
                stance: 3,
                state_position: [30.0, 88.0],
                retarget_disabled: 0,
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
    fn seed_for(wanders: bool) -> Well512 {
        (0..)
            .map(|seed| {
                let mut r = Well512::default();
                r.set_seed(seed);
                r
            })
            .find(|r| {
                let mut e = r.clone();
                e.below(81);
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
        e.below(81);
        e.rand15();
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
        u.notify_move(true, None, [21.947916, 92.489586], &mut rng());
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
        assert_eq!(u.target_point, [21.947916, 92.489586, 0.0]);
    }

    #[test]
    fn an_idle_transport_ship_retargets_like_a_soldier() {
        // type 11, TribeTransportShipUnitAIModule: base slots 4 and 73, slot 72 0x140c21d20 draws
        // nothing
        let mut u = idle(11);
        let mut r = rng();
        u.update_idle(
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
            Ok(-1)
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
        e.rand15();
        let step = |n: u32| if 3 <= n { n as i32 - 6 } else { n as i32 + 1 };
        let (dx, dy) = (step(e.rand15() % 6), step(e.rand15() % 6));
        let moved = u
            .update_idle(10, 0, false, &place, &mut r, &mut |_: &Search<'_>| Ok(-1))
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
            .update_idle(10, 0, false, &away, &mut r, &mut |_: &Search<'_>| Ok(-1))
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

    #[test]
    fn a_deer_that_sees_an_object_would_evade_it_which_is_not_modelled() {
        let mut u = deer();
        let mut r = wander_seed(false);
        let place = deer_at([31.5, 106.5, 0.0]);
        assert!(
            u.update_idle(10, 0, false, &place, &mut r, &mut |_: &Search<'_>| Ok(9185))
                .is_err()
        );
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
        u.update(10, 0, false, &walking, &mut r, &mut no_search)
            .unwrap();
        assert_eq!(
            (u.current_action, u.idle_timer, u.state_position),
            (0x262, 5010, [0.0; 2])
        );
    }

    /// MarvinBlue/426048731's scout 9183 (soldier AI, type 13) as saved after 821: on its MOVE
    /// order with five queued moves, its MoveTo about to arrive.
    fn scout() -> UnitAi {
        let order = |x: f32, y: f32| QueuedOrder {
            issuer: 1,
            order: MOVE_ORDER,
            priority: 100,
            target: -1,
            target2: -1,
            point: [x, y, 0.0],
            range: 0.0,
            shown: [x, y],
            ready: true,
            extra: -1,
        };
        UnitAi {
            current_order: MOVE_ORDER,
            order_priority: -1,
            current_action: MOVE_ACTION,
            target_type: -1,
            target_point: [93.583336, 57.791668, 0.0],
            last_order: MOVE_ORDER,
            last_action: MOVE_ACTION,
            last_target: -1,
            last_target_type: -1,
            stance: 2,
            orders: vec![order(89.09375, 60.677082), order(91.822914, 52.489582)],
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
                point: [89.09375, 60.677082, 0.0],
                range: 0.0,
                force: true,
                walk: true,
            })]
        );
        assert_eq!(
            (u.current_order, u.order_priority, u.current_action),
            (MOVE_ORDER, 100, MOVE_ACTION)
        );
        assert_eq!(u.target_point, [89.09375, 60.677082, 0.0]);
        assert_eq!(u.orders.len(), 1);
        assert_eq!(u.orders[0].point, [91.822914, 52.489582, 0.0]);
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
        u.off_turn(&scout_at(true, true)).unwrap();
        assert_eq!(u.orders.len(), 1);
        assert_eq!(u.orders[0].point, [89.09375, 60.677082, 0.0]);
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
        assert!(builder.notifies.is_empty());
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
        assert!(dying.notifies.is_empty());
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

    #[test]
    fn a_saved_order_and_notify_read_as_the_game_wrote_them() {
        let mut order = [0u8; 52];
        for (i, w) in [1i32, 710, 100, -1, -1].iter().enumerate() {
            order[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
        }
        for (i, f) in [91.822914f32, 52.489582, 0.0, 0.0, 91.822914, 52.489582]
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
                o.issuer, o.order, o.priority, o.point, o.range, o.ready, o.extra
            ),
            (1, 710, 100, [91.822914, 52.489582, 0.0], 0.0, true, -1)
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
}
