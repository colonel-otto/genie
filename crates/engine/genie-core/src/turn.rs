//! The per-tick skeleton the unit-AI slice needs: the frame-seed chain and the unit-AI turn.
//!
//! 158041 `BaseWorld::update` (`0x14154fc70`), in order: advance the world time by dt,
//! `SeedAll(frame seed)`, the player loop (each player's time slot `world+0x18c[p]` gains dt; only the
//! turn player `world+0x188` runs the full unit-AI update), `s1 = World.rand15`, `SeedAll(s1)`, zero
//! the turn player's slot and pass the turn on, then the next frame seed `= World.rand15`. The slice
//! assumes no other World draw runs in a tick (no triggers, XS or unit groups).
//!
//! A player's update walks its live list in order (`RGE_Object_List::update`, `0x141166550`). Each
//! object moves by its velocity ([`Mover::step_velocity`]), runs its static state step
//! ([`Lifecycle`]), then its actions (the livestock default action, a builder's Build action with
//! its walk), then its unit AI. An object whose update returns 1 leaves: the last object takes its
//! slot and updates there in the same tick.

use crate::artifact::Artifact;
use crate::attack::Strike;
use crate::build::{BUILD, BuildAction, BuildTask, Builder, Completion, Construction, Site, Walk};
use crate::checkpoint::TopAction;
use crate::damage::Victim;
use crate::fly::Fly;
use crate::hunt::{Gatherer, HuntAction, HuntSprites};
use crate::lifecycle::{Class, Lifecycle, seconds};
use crate::masters::{Master, Masters};
use crate::move_to::{MoveTo, MoveToAction};
use crate::movement::{Me, Mover, Planning, footprint_gap, footprint_gap_squared};
use crate::order::{Interact, MoveOrder};
use crate::pathing::Pathing;
use crate::production::{Production, Queued};
use crate::rng::Well512;
use crate::rng::{Stream, Streams};
use crate::spatial::{Placed, Spatial};
use crate::sprite::{ActiveSprite, Graphic};
use crate::tasks::{Actor, Diplomacy, DropSite, TaskTarget};
use crate::unit_ai::{
    ACTION_COMPLETED, ACTION_EXECUTED, AiMove, BUILD_ACTION, BUILD_ORDER, Effect, Notify,
    OWNER_CHANGED, Search, Surroundings, TARGET_MOVED, UnitAi, Unsupported,
};
use crate::visibility::{BuildingSight, Seer, capture_kind};

/// What the slice tracks of one object.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Unit {
    /// The object's id (`+0x8`), which names it across ticks.
    pub id: i32,
    /// x, y, z (`+0x3c8`, `+0x4c8`, `+0x5a8`, obfuscated in the game's memory).
    pub position: [f32; 3],
    /// Its life cycle: state, hit points, master, what it holds, its dying sprite.
    pub life: Lifecycle,
    /// Its unit AI, when it has one.
    pub ai: Option<UnitAi>,
    /// The livestock default action, the object's top action.
    pub artifact: Option<Artifact>,
    /// A default task other than the Artifact action. `RGE_Action_Object::update` (0x14106ac90)
    /// starts it in an object's first update if the object is still in state 2 after its static
    /// step; the slice does not model it, so that update stops the player.
    pub unmodelled_action: Option<Unsupported>,
    /// A building's production queue and make action.
    pub production: Option<Production>,
    /// A building's gather point, which its new units go to.
    pub gather_point: Option<GatherPoint>,
    /// A builder's Build action, its top action.
    pub build: Option<BuildAction>,
    /// A moving object's movement state (types 30 and up).
    pub mover: Option<Mover>,
    /// A `MoveTo` as the object's own top action (a wandering animal).
    pub move_to: Option<MoveToAction>,
    /// A bird's Fly action, its top action.
    pub fly: Option<Fly>,
    /// A building's construction.
    pub construction: Option<Construction>,
    /// `+0xa30`, `mWorkRatePowerup`: what the unit adds to its master's work rate.
    pub work_rate_powerup: f32,
    /// `+0x68a`, `unit_attackable_flag`.
    pub attackable: bool,
    /// `+0x689`, `mUnitTargetingFlag`, is 0 (a new object's is 1): a gatherer's search takes the
    /// object only through a pasture (`ClosestResourceObject`).
    pub untargetable: bool,
    /// `+0x685`, `underAttackValue`: its unit AI keeps attackers (`FUN_14119d2d0`).
    pub under_attack: bool,
    /// `+0x790`, `parent_pasture_id`: the pasture it is in; `+0x794`, `my_pasture_id`: the one it
    /// tends. -1 for none.
    pub pasture: i32,
    /// `+0x794`, `my_pasture_id`: the pasture it tends, −1 for none.
    pub my_pasture: i32,
    /// An annex's main building (`+0xbb8`, the save's linked owner).
    pub annex_of: Option<i32>,
    /// `mHitpointsPowerup` and `mHitpointsMaxPowerup` (`+0xa38`, `+0xa3c`), which the task finder
    /// adds to the hit points.
    pub hp_powerups: [f32; 2],
    /// A villager's Hunt action, its top action.
    pub hunt: Option<HuntAction>,
    /// Its own Attack (type 9, not a Hunt's), its top action: a boar's on its attacker.
    pub attack: Option<crate::attack::Attack>,
    /// An auto scout's Explore action, its top action.
    pub explore: Option<crate::explore::ExploreAction>,
    /// A unit's Enter action (a garrison), its top action.
    pub enter: Option<crate::garrison::EnterAction>,
    /// A missile's own part with its action ([`crate::missile`]).
    pub missile: Option<crate::missile::Missile>,
    /// `+0x900`, `attack_count`: the swings that fired a missile (`FUN_141081020`).
    pub attack_count: i32,
    /// `+0xa00` (not saved): a combat object's missiles per volley, its master's volley fire plus
    /// the shares of the units inside it ([`crate::garrison::share`]).
    pub firepower: f32,
    /// `next_volley` (`+0xa04`): the missiles of the volley under way already fired.
    pub next_volley: u8,
    /// `mReloadWorldtime` (`+0x920`, slots 0x248 and 0x250): the world time its attack timer
    /// runs out at (`FUN_141089820`), 0 for none; once past, the volley under way is over.
    pub reload_worldtime: u32,
    /// The objects inside it (`+0x2f8`), in the order they came in (`FUN_141164af0`); one that
    /// leaves takes the last one's place (`FUN_141166450`).
    pub garrison: Vec<i32>,
    /// A building's gate lock ([`Gate`]).
    pub gate: Option<Gate>,
    /// Its active sprite and animation speed (`+0x7b8`, the save's `current_speed`), held from
    /// an attack's swing on; None while the slice does not know them.
    pub sprite: Option<ActiveSprite>,
    /// Its animation speed (`+0x7b8`), held with its sprite.
    pub anim_speed: Option<f32>,
    /// `+0x91c`, `attack_timer`: the time until it may attack again; `mReloadStinger` and
    /// `mReloadPowerup` (`+0xb40`, `+0xa28`), which change its reload time.
    pub attack_timer: f32,
    /// `mReloadStinger` (`+0xb40`), added to its master's reload time.
    pub reload_stinger: f32,
    /// `mReloadPowerup` (`+0xa28`), taken off its master's reload time.
    pub reload_powerup: f32,
    /// What a hit reads of its unit and its target besides the masters: `+0xa24`
    /// (`mAttackPowerup`, the attacker's slot 0x6d8), the armor powerups (`+0xa48` melee,
    /// `+0xa4c` pierce, slots 0x708 and 0x710; whole numbers), the armor attacks removed
    /// (`+0x9e4`, `+0x9e6`), the charge (`+0x9e0`) and `mHitSomethingValidForChargeAttack`
    /// (`+0x8a8`: its last hit was its charge's).
    pub attack_powerup: f32,
    /// Its melee and pierce armor powerups (`+0x9e4`, `+0x9e6`).
    pub armor_powerups: [i16; 2],
    /// The melee and pierce armor taken off it (its charge's hits).
    pub armor_removed: [i16; 2],
    /// Its charge (`+0x9e0`).
    pub charge: f32,
    /// `mHitSomethingValidForChargeAttack` (`+0x8a8`): its last hit was its charge's.
    pub charge_hit: bool,
    /// `+0x644`, `worker_num`: the actions that have the object as their target.
    pub workers: i32,
    /// `+0x650`, `mLastHumanOrderTime`: the world time of the last order a human gave it
    /// (`FUN_140e5e780`, for a unit with a unit AI); an animal's idle check lowers it to 5000.
    pub last_human_order: u32,
    /// A combat object's town-bell flag and back-to-work records ([`crate::town_bell`]).
    pub town_bell: crate::town_bell::TownBell,
    /// The object is a combat object (types 50 and up): its update keeps its sight mask and its
    /// records in the objects-in-sight registry.
    pub combat: bool,
    /// What its master's default task would start once its action list is empty, when the
    /// slice does not model it (None: nothing, as a wild animal's Explore).
    pub default_task: Option<Unsupported>,
    /// The Fly task its master's default task starts once its action list is empty (a bird at
    /// a game's start, before its first update).
    pub default_fly: Option<crate::masters::Task>,
    /// The slice could not model one of its updates: it freezes (its place is known only within
    /// its speed since) while the rest of its player's objects go on.
    pub stopped: bool,
}

/// What every building holds of a gate: `mGateLocked` (`+0xc7c`; a lock also sets the
/// building's last move time, [`Mover::last_move`], slot 0x670, `FUN_1411dba30`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gate {
    /// `mGateLocked`: the gate is locked.
    pub locked: bool,
}

/// A building's gather point (`+0xc30`..`+0xc44`: it exists, x, y, z, the target and the target's
/// master; the save's `gather_point_*`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GatherPoint {
    /// The point its trained units walk to.
    pub point: [f32; 3],
    /// The object it is on, −1 for none.
    pub target: i32,
    /// The target's master, −1 without a target.
    pub target_master: i16,
}

/// A recorded command, run in the command phase of its tick: after the frame seed's `SeedAll`, before
/// the players update (`TRIBE_Command::do_command`, `0x140e77b00`). The slice runs only what other
/// players' lanes see of a command; the issuing player's lane stops.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// BUILD (0x66, `FUN_140e7a400`): the player places a foundation of `master` at `position`
    /// (state 0, hit points 1), with a unit AI of the given type if its class gets one; `Err` when
    /// the slice cannot tell. The player pays the master's cost, and each builder, in packet order,
    /// takes the master it builds with and starts a Build action (`Turns::masters`). The slice
    /// takes the placement as given: the recording's game accepted it.
    Build {
        /// The player who places it.
        player: usize,
        /// The foundation's master.
        master: i16,
        /// The unit AI type its class gets, if any, or why the slice cannot tell.
        unit_ai: Result<Option<u32>, Unsupported>,
        /// Where it is placed.
        position: [f32; 3],
        /// The units told to build it, in packet order.
        builders: Vec<i32>,
        /// `+0x1e`: queued (a builder with a unit AI takes it as an order in its queue).
        queued: bool,
        /// `+0x1f` is 0: a builder with a unit AI holding an order not ready takes it as an order
        /// too, and the queue's orders become ready after (`FUN_140c42b60`).
        commits: bool,
    },
    /// SET GATHER POINT (0x78, `FUN_140e7e9a0`): each of the player's objects of `ids` that takes
    /// a gather point (its master garrisons, `+0x5d4`; the list of units it trains,
    /// `FUN_141625a20`, is not modelled) takes `point` and `target` with its master, or with
    /// `own` itself (`FUN_1411c7ff0`); a target of −1 at (−1, −1) clears it. The ids of other
    /// players' objects are left out (the game setting `+0x1074` that lets them is not modelled),
    /// and the unit AI's command queue (`+0x1f0`, which its slot 0x210 empties) is not held.
    GatherPoint {
        /// The player whose objects take it.
        player: usize,
        /// The objects of the packet.
        ids: Vec<i32>,
        /// The point.
        point: [f32; 2],
        /// The object the point is on, −1 for none.
        target: i32,
        /// That object's master.
        target_master: i16,
        /// The building itself is the gather point (`+0x18`).
        own: bool,
    },
    /// QUEUE (0x81, the player's queue `FUN_141614450`, the building's `FUN_1411cbc40`): `amount`
    /// units of master `unit` at one building. Each paid `cost` entry `(resource, amount, paid)`
    /// comes off the player's resources per unit; a villager's master is drawn male or female from
    /// the Action stream. The make action then works `rate` seconds of `train_time` per second.
    Queue {
        /// The player who queues.
        player: usize,
        /// The building.
        building: i32,
        /// The master to train (0x712 for the villager).
        unit: i16,
        /// How many.
        amount: u16,
        /// The seconds of work each takes at that building.
        train_time: f32,
        /// The building's work rate.
        rate: f32,
        /// The unit's three cost entries as its master holds them: resource (−1 for none),
        /// amount, paid.
        cost: Vec<(i16, i16, bool)>,
    },
    /// RESEARCH (0x65, `0x140e7e270`): the technology `tech` at the building `building`
    /// ([`Turns::research`]). `count` (`+0x8`) counts the buildings the packet lists, `target`
    /// (`+0xc`) is −1 for a research to run, and `shift` (`+0x10`) asks for five at once.
    Research {
        /// The player who researches.
        player: usize,
        /// The building of the packet.
        building: i32,
        /// `+0x8`: how many buildings the packet lists.
        count: i16,
        /// `+0xa`: the technology.
        tech: i16,
        /// `+0xc`: −1 for a research to run.
        target: i32,
        /// `+0x10`: five at once.
        shift: bool,
    },
    /// A command the slice runs part of or none, for a reason the decoder names: it stops the
    /// issuing player's lane.
    Unmodelled {
        /// The issuing player.
        player: usize,
        /// Why its command is not modelled.
        why: Unsupported,
    },
    /// GAME (0x67, `0x140e7c330`) sub-commands 0x10 and 0x13: the player's `autoFarmQueue`
    /// (`+0x1424`) or `autoFishTrapQueue` (`+0x1425`) flips (`fishtrap` true for 0x13). The other
    /// sub-commands are the decoder's [`Command::Unmodelled`].
    AutoQueue {
        /// The player.
        player: usize,
        /// The fish trap queue flips (0x13), else the farm queue (0x10).
        fishtrap: bool,
    },
    /// GAME (0x67, `0x140e7c330`) sub-command 0x14 [`default_stance`]: the player's offensive
    /// posture (`+0x71c`, `WorldPlayer` slot 0x14 `FUN_14163a630`) flips between 0 and 1.
    Posture {
        /// The player.
        player: usize,
    },
    /// GATE (0x72, `do_command` case 0x72): the player's building `id` toggles its gate lock
    /// ([`Turns::gate`]).
    Gate {
        /// The player.
        player: usize,
        /// Its building.
        id: i32,
    },
    /// UNLOAD (0x6f, `FUN_14161f410`): the units `ids` (a building, the units inside it; a unit
    /// inside one, itself) let units out ([`Turns::unload`]) toward `point` (−1, −1 for none),
    /// those `flag` and `unit_type` pick (`FUN_14118a610`); `sound` (`+0x18`) plays the
    /// building's.
    Unload {
        /// The player.
        player: usize,
        /// Its objects.
        ids: Vec<i32>,
        /// Where they go.
        point: [f32; 2],
        /// Which units leave (`FUN_14118a610`'s switch).
        flag: u8,
        /// The unit type or object the flag reads.
        unit_type: i32,
        /// `+0x18`.
        sound: bool,
    },
    /// `TRANSFORM_OBJECT` (0x29, `RGE_Command::do_command`'s case): the player's object `id`
    /// takes the master `master` ([`Turns::transform_object`]).
    TransformObject {
        /// The player.
        player: usize,
        /// The object (`+8`).
        id: i32,
        /// The new master (`+4`).
        master: i16,
    },
    /// RESIGN (0x0b, `FUN_140d25af0`): the comm player `player` resigns, or drops (`dropped`)
    /// ([`Turns::resign`]).
    Resign {
        /// The comm player (the world player of the same number in the slice's games).
        player: usize,
        /// A drop from the game, not a resignation.
        dropped: bool,
    },
    /// `AUTO_SCOUT` (0x26, `FUN_140e666f0`): each of `ids` (at most 60, the decoder's) takes the
    /// auto scout order, or the stop order when it scouts already ([`Turns::auto_scout`]).
    AutoScout {
        /// The player.
        player: usize,
        /// Its units.
        ids: Vec<i32>,
    },
    /// `UNIT_AI_STATE` (0x12, `FUN_140e6a7c0`): the unit AIs of `ids` (at most 60, the decoder's)
    /// take the stance `state` ([`Turns::unit_ai_state`]).
    UnitAiState {
        /// The player whose units take it.
        player: usize,
        /// The units.
        ids: Vec<i32>,
        /// The stance.
        state: u8,
    },
    /// `UNIT_ORDER` (0x75, `FUN_140e7f200` -> `FUN_14161e370`): the units `ids` (at most 60, the
    /// decoder's) take the order `action` ([`Turns::unit_order`]) with the object `target`
    /// (`tId`, `+8`, which action 5 reads) and the flags `+0x1d`, `+0x1e`; `human` (`+0x20`) sets
    /// their last human order time first. Its point, `qId` and `param` are not read by the
    /// actions modelled.
    UnitOrder {
        /// The player whose units take it.
        player: usize,
        /// The units.
        ids: Vec<i32>,
        /// `tId`: the target object's id.
        target: i32,
        /// `+0x1d`: a garrison goes through the units' unit AIs (their slot 0x30).
        through_ai: bool,
        /// `+0x1e`: a garrison takes the buildings of the target's master near it.
        nearby: bool,
        /// The order's action (8: the drop-off).
        action: u8,
        /// `+0x20`: it sets their last human order time.
        human: bool,
    },
    /// INTERACT (0x00), MOVE (0x03) or RETREAT (0x23): before the order runs, each listed unit's AI
    /// resets its idle timeout ([`UnitAi::reset_for_order`]), in list order, wherever the unit is.
    /// INTERACT's order then runs ([`Interact`]), and MOVE's ([`MoveOrder`]); RETREAT is not
    /// modelled.
    Order {
        /// The player who orders.
        player: usize,
        /// The units of the packet.
        units: Vec<i32>,
        /// The order they take, None for a command that only resets their idle timeouts.
        order: Option<Order>,
    },
}

/// What an order command gives its units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Order {
    /// INTERACT: work on, attack or follow a target.
    Interact(Interact),
    /// MOVE: walk to a point.
    Move(MoveOrder),
    /// FOLLOW's target (`+0x8`).
    Follow(i32),
}

/// RESEARCH's fields ([`Command::Research`] but the player).
struct ResearchAt {
    building: i32,
    count: i16,
    tech: i16,
    target: i32,
    shift: bool,
}

/// The building unit AI's action value a research it queued sets (`FUN_140c64b50` with 0x26c).
pub(crate) const RESEARCH_ACTION: i32 = 0x26c;
/// The building top actions that keep RESEARCH from a building (`FUN_141227a00`: 0x7b to 0x7d).
const BLOCKS_RESEARCH: std::ops::RangeInclusive<i16> = 0x7b..=0x7d;
/// The building master type RESEARCH asks for (`'P'`, 80).
const BUILDING_KIND: u8 = 80;

/// QUEUE's unit and building (the fields of [`Command::Queue`] but the cost).
struct QueueAt {
    building: i32,
    unit: i16,
    amount: u16,
    train_time: f32,
    rate: f32,
}

/// The villager master QUEUE draws a sex for (`0x53`), and the generic villager id that becomes
/// 0x53 first (`0x712`), both in `FUN_1411cbc40`.
const VILLAGER: i16 = 0x53;
const VILLAGER_ANY: i16 = 0x712;
/// The female villager master (`0x125`).
const VILLAGER_FEMALE: i16 = 0x125;
/// A building's queue holds at most 15 units (`FUN_1411cbc40`).
const QUEUE_LIMIT: usize = 15;
/// The notify a Build waiting without work posts (0x202) with its action value
/// ([`BUILD_ACTION`]).
const BUILD_IDLE: i32 = 0x202;
/// A velocity longer than this sets the animation speed (`DAT_142eb0c60`).
const SPEED_EPSILON: f32 = crate::image::turn::SPEED_EPSILON;
/// A building attacks with a firepower above this (`FUN_14120ea30`).
const ARMED_FIREPOWER: f32 = crate::image::turn::ARMED_FIREPOWER;

/// A queued villager's master: male when the 64-bit Action draw times 100 (wrapping) is below 50
/// modulo 100.
const fn villager(draw: u64) -> i16 {
    if draw.wrapping_mul(100) % 100 < 50 {
        VILLAGER
    } else {
        VILLAGER_FEMALE
    }
}

/// What a tick's update of one object needs from the world.
/// What a hunt's drop-off ([`Unit::drop_off_in`]) reads besides its unit: the owner's masters,
/// the world with its zone maps and planners, the world's time, the Object stream (the drop-site
/// search's plans), the owner's type and construction rate.
pub(crate) struct DropWorld<'a> {
    pub masters: Option<&'a Masters>,
    pub spatial: &'a Spatial,
    pub zones: &'a mut crate::zones::Zones,
    pub pathing: &'a mut Pathing,
    pub object: &'a mut Well512,
    pub world_time: u32,
    pub computer: bool,
    pub construction_rate: f32,
}

struct Context<'a> {
    /// The tick's dt in milliseconds, and in seconds (`world+0x164`).
    dt: u32,
    dt_seconds: f32,
    /// The ticks so far, this one included.
    tick: u32,
    /// The owner's accumulated time when it has the turn: the full unit-AI update runs.
    turn: Option<u32>,
    world_time: u32,
    computer: bool,
    streams: &'a mut Streams,
    /// Every object's place, which a walker reads and its moves update.
    spatial: &'a mut Spatial,
    /// The planners and the mesh, which a walker's path request uses.
    pathing: &'a mut Pathing,
    /// The owner's construction rate (attribute 195).
    construction_rate: f32,
    /// The owner's masters, which an attack reads.
    masters: Option<&'a Masters>,
    /// The target of the unit's attack, read before its update and written back after it, and
    /// the objects about a unit with an area effect, which its blast may reach.
    victim: Option<&'a mut Victim>,
    /// The unit AI's current target as its target block reads it ([`Turns::seen`]).
    ai_target: Result<Option<crate::unit_ai::Seen>, Unsupported>,
    blasted: Vec<Victim>,
    /// The unit was killed in its update (a retaliation), which the caller handles after it.
    own_killed: Option<crate::damage::Kill>,
    /// What the unit's death leaves for the caller: the remains its dying sprite's end makes
    /// (`FUN_1411a4e90`: the master, the place and the dead object's facet) and the paid cost its
    /// owner takes back (attributes 98 and 164).
    remains: Option<(i16, [f32; 3], Option<u8>)>,
    refund: Option<f32>,
    /// The pauses the unit's walk gave units it met ([`Mover::pauses`]).
    pauses: Vec<(i32, f32)>,
    /// The objects the unit's actions let go (slot 0x180, [`Turns::release`]) and take (slot
    /// 0x178, [`Turns::take`]) in other players' lists, which the caller applies after the hit's
    /// write-backs.
    releases: Vec<(i32, Releaser)>,
    takes: Vec<i32>,
    /// The objects the unit made (a ranged unit's missiles), which join the end of its owner's
    /// list after it and update later in the same tick.
    made: Vec<Unit>,
    /// The objects whose master leaves ghosts that died in this update ([`crate::ghost`]), the
    /// caller's.
    ghost_deaths: Vec<i32>,
    /// The buildings whose construction started in this update: their owner watches them
    /// ([`Turns::watch_started`]), the caller's.
    started: Vec<i32>,
    /// The buildings that died in this update: their owner's watched list lets them go
    /// (`FUN_141091310`), the caller's.
    unwatched: Vec<i32>,
    /// The world's reusable ids and pools.
    recycling: &'a mut Recycling,
    /// The game's setting at `+0x680` is 1 ([`Turns::mode_680`]).
    mode_1: bool,
    /// The owner's attributes, which a gatherer's productivity reads.
    attributes: &'a [f32],
    /// The graphics, for the sprites a hunter takes.
    graphics: &'a std::collections::BTreeMap<i16, Graphic>,
    /// The drop-site table, whose livestock entries a hunt leads its livestock to.
    drop_sites: &'a [DropSite],
    /// The owner's id and civ (`player+0x738`).
    player: usize,
    civ: u8,
    /// Every player's masters and attribute 97 (by player), for a capture.
    every_masters: &'a [Masters],
    attribute_97: &'a [f32],
    /// The player whose turn this tick is (`world+0x188`), every player's accumulated time
    /// (`world+0x18c`, as it stands when the object updates) and computer flag.
    player_turn: usize,
    player_time: &'a [u32],
    computers: &'a [bool],
    /// Some player's attribute 204 is not 0, which a building's completion reads.
    attribute_204: bool,
    /// [`Turns::mode_680`].
    mode_680: i32,
    /// The game mode is one of `FUN_141302540`'s ([`scenario_mode`]).
    scenario: bool,
    /// The attributes the owner gains from the object's update (a building it completed), which
    /// the caller adds after it.
    attribute_adds: Vec<(i16, f32)>,
    /// The technologies the owner researches from the object's update (a completed building's),
    /// which the caller completes after the attributes ([`Turns::complete_research`]).
    techs_done: Vec<i16>,
    /// The work a builder found after its building (slot 0x7e8 from the Build's case 0xdf),
    /// which the caller gives after the update ([`Turns::order_work`]).
    work_order: Option<crate::order::Work>,
    /// The unit AI's order on a target (`0x2db`) the unit took from its queue, which the caller
    /// runs after the update ([`Turns::order_on_target`]).
    order_on_target: Option<crate::unit_ai::QueuedOrder>,
    /// The map's zone maps, which an auto scout's search reads.
    zones: &'a mut crate::zones::Zones,
    /// The other players' lists, as they stand before the unit's update.
    foreign: Foreign<'a>,
}

/// The players but the owner of the unit that updates, read-only: the players before it and
/// after it.
#[derive(Clone, Copy)]
struct Foreign<'a> {
    before: &'a [Player],
    after: &'a [Player],
}

impl<'a> Foreign<'a> {
    /// The object `id` in another player's live or sleeping list, with that player.
    fn find(&self, id: i32) -> Option<(usize, &'a Unit)> {
        let skip = self.before.len() + 1;
        let before = self.before.iter().enumerate();
        let after = self.after.iter().enumerate().map(|(q, p)| (q + skip, p));
        before.chain(after).find_map(|(q, p)| {
            p.objects
                .iter()
                .chain(&p.sleeping)
                .find(|u| u.id == id)
                .map(|u| (q, u))
        })
    }
}

/// A unit and the world as its unit AI asks them while it runs, the AI taken out of the unit
/// for that time ([`crate::unit_ai::UnitSide`]): the searches and the unit's own commands.
struct UnitView<'u> {
    /// The unit, without its unit AI.
    unit: &'u mut Unit,
    /// What the attack check of a search reads of the unit.
    attacker: crate::unit_ai::Attacker,
    /// The owner, as the AI runs (a captured unit's new one).
    player: usize,
    /// The owner is a computer player.
    computer: bool,
    /// The world's objects.
    spatial: &'u Spatial,
    /// The planners and the mesh, which a reach test walks.
    pathing: &'u mut Pathing,
    /// The Object stream, which a reach test's new path draws from.
    object: &'u mut Well512,
    /// The Sprite stream, which a new sprite draws from.
    sprite: &'u mut Well512,
    /// The world's time.
    world_time: u32,
    /// The owner's construction rate.
    construction_rate: f32,
    /// The owner's other objects.
    others: &'u [Unit],
    /// The map's terrain zones, which an explorer's search reads.
    zones: &'u mut crate::zones::Zones,
    /// The other players' lists.
    foreign: Foreign<'u>,
    /// Every player's masters.
    every: &'u [Masters],
    /// The owner's masters.
    masters: Option<&'u Masters>,
    /// The sprites.
    graphics: &'u std::collections::BTreeMap<i16, Graphic>,
    /// The objects an action took (slot 0x178), the caller's.
    takes: &'u mut Vec<i32>,
}

impl crate::unit_ai::UnitSide for UnitView<'_> {
    fn closest(&mut self, search: &Search<'_>) -> Result<crate::unit_ai::Found, Unsupported> {
        let every = self.every;
        let attacker = &self.attacker;
        let can_attack = |p: &Placed, capture: u8| {
            let m = every
                .get(p.player)
                .and_then(|ms| ms.get(p.master))
                .ok_or(Unsupported(
                    "a found object of a master the slice does not have",
                ))?;
            crate::unit_ai::can_attack(
                attacker,
                &crate::unit_ai::Target {
                    owner: p.player,
                    class: m.class,
                    kind: m.kind,
                    master: p.master,
                    combat_level: m.combat_level,
                    capture,
                },
            )
        };
        // ClosestObject's reachability check (slot 0x3d0) walks the unit's paths
        let mut plan = Planning {
            pathing: self.pathing,
            now: self.world_time,
            computer: self.computer,
            object: self.object,
            construction_rate: self.construction_rate,
        };
        let unit = &*self.unit;
        let walker = Me {
            id: unit.id,
            position: unit.position,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let (mover, master_id, spatial) = (unit.mover.as_ref(), unit.life.master_id, self.spatial);
        let mut reach = |id: i32, range: f32| {
            let m = mover.ok_or(Unsupported("a reach test of a unit that does not move"))?;
            m.reaches_object(&walker, spatial, &mut plan, id, range, master_id)
        };
        let (others, foreign) = (self.others, self.foreign);
        let moving = |id: i32| herded(others, foreign, id);
        spatial.search(
            unit.id,
            self.player,
            search,
            &can_attack,
            &mut reach,
            &moving,
        )
    }

    /// The attack command (slot 0x338, `FUN_14120cea0`; classes 51 and 54 are not modelled) ->
    /// `FUN_141081a80`: a target the world does not hold fails; `+0x648` takes the world's time
    /// (not held); the unit's own Attack on the target answers true unless forced (its slot 0x48
    /// then, not modelled); a villager's master switch (slot 0x90) is not modelled. Then the new
    /// Attack ([`crate::attack::Attack::command`]): the constructor takes its target (slot 0x178,
    /// the caller's), the action list clears (the old actions' teardowns), and the install's start
    /// clears `+0x900`, puts the unit on its standing sprite (slot 0x68 with `+0x80`) and runs the
    /// range test. A `MoveTo` that starts arrived and a notify posted while the AI runs (its
    /// buffer, `FUN_140c55270`) are not modelled.
    fn attack(
        &mut self,
        target: i32,
        force: bool,
        ai: &UnitAi,
    ) -> Result<crate::unit_ai::Attacked, Unsupported> {
        let unit = &mut *self.unit;
        let master = self
            .masters
            .and_then(|m| m.get(unit.life.master_id))
            .ok_or(Unsupported(
                "an attacker of a master the slice does not have",
            ))?;
        if matches!(master.class, 51 | 54) {
            return Err(Unsupported(
                "the attack command of a unit of class 51 or 54",
            ));
        }
        let no = crate::unit_ai::Attacked::default;
        let Some(t) = self.spatial.get_known(target)?.copied() else {
            return Ok(no());
        };
        if unit.attack.is_some_and(|a| a.target == target) {
            if force {
                return Err(Unsupported(
                    "a forced attack command on its own Attack's target (slot 0x48)",
                ));
            }
            return Ok(crate::unit_ai::Attacked {
                attacks: true,
                posted: Vec::new(),
            });
        }
        if master.class == 4 {
            return Err(Unsupported(
                "a villager's attack command (its master switch, slot 0x90)",
            ));
        }
        let me_at = *self
            .spatial
            .get(unit.id)
            .ok_or(Unsupported("an attacker without a place"))?;
        let t_master = self
            .every
            .get(t.player)
            .and_then(|m| m.get(t.master))
            .ok_or(Unsupported(
                "an attack's target of a master the slice does not have",
            ))?;
        let (tx, ty) = (t.position[0] as i32, t.position[1] as i32);
        let visible = self.spatial.sees_tile(self.player, tx, ty)
            || (t_master.fog_visibility != 0 && self.spatial.explored(self.player, tx, ty)?);
        let arms = crate::attack::arms(master, self.graphics, [-1; 4]);
        let standing = arms
            .idle_sprite
            .ok_or(Unsupported("an attacker without its standing sprite"))?;
        if unit.sprite.is_none() {
            return Err(Unsupported(
                "an attack command of a unit whose sprite the slice does not hold",
            ));
        }
        unit.clear_own_actions()?;
        unit.attack_count = 0;
        crate::attack::set_sprite(&mut unit.sprite, standing, self.sprite);
        let mover = unit
            .mover
            .as_mut()
            .ok_or(Unsupported("an attacker without movement state"))?;
        let e = crate::attack::engagement(
            Some(ai),
            mover.stats.speed,
            master,
            &me_at,
            &t,
            self.world_time,
        )?;
        let crate::attack::Commanded {
            attack,
            arrived,
            posted,
            ready,
        } = crate::attack::Attack::command(target, &e, arms, visible, t.player, mover)?;
        if ready {
            return Err(Unsupported(
                "an attack command of a unit that cannot move (its set_state 6)",
            ));
        }
        if arrived {
            return Err(Unsupported("an attack command whose MoveTo starts arrived"));
        }
        unit.attack = Some(attack);
        self.takes.push(target);
        Ok(crate::unit_ai::Attacked {
            attacks: true,
            posted,
        })
    }

    fn set_under_attack(&mut self, value: bool) {
        self.unit.under_attack = value;
    }

    fn seek_unexplored(
        &mut self,
        ai: &UnitAi,
        rng: &mut Well512,
    ) -> Result<Option<[i32; 2]>, Unsupported> {
        let tc = town_centre(self.others)?;
        let (at, computer) = (self.unit.position, self.computer);
        let (mut area, map, line_of_sight) = self.scout_area()?;
        if let Some(p) = crate::explore::unexplored_area(tc, map, &mut area)? {
            return Ok(Some(p));
        }
        // none about the town centre: the unit AI's own search (slot 0x90)
        let seeker = crate::explore::Seeker {
            at,
            line_of_sight,
            order: ai.current_order,
            target_point: [ai.target_point[0], ai.target_point[1]],
            computer,
        };
        crate::explore::closest_undiscovered(&mut area, seeker, map, rng)
    }

    fn seek_beyond_sight(&mut self, rng: &mut Well512) -> Result<Option<[i32; 2]>, Unsupported> {
        let at = self.unit.position;
        let (mut area, map, line_of_sight) = self.scout_area()?;
        crate::explore::beyond_sight(&mut area, at, line_of_sight, map, rng)
    }
}

impl UnitView<'_> {
    /// What an explorer's searches ask of the world, the map's size and the unit's line of sight.
    fn scout_area(&mut self) -> Result<(ScoutArea<'_>, [i32; 2], f32), Unsupported> {
        let unit = &*self.unit;
        let master = self
            .masters
            .and_then(|m| m.get(unit.life.master_id))
            .ok_or(Unsupported(
                "an auto scout of a master the slice does not have",
            ))?;
        let mover = unit
            .mover
            .as_ref()
            .ok_or(Unsupported("an auto scout without movement state"))?;
        let [w, h] = self.spatial.map;
        let terrain = self
            .spatial
            .terrain
            .as_ref()
            .ok_or(Unsupported("the map's zones without its terrain"))?;
        let values = terrain
            .restrictions
            .get(mover.stats.restriction)
            .ok_or(Unsupported("a restriction the slice's data has not got"))?;
        let zones = self.zones.of(w, h, &terrain.kinds, values);
        let me = Me {
            id: unit.id,
            position: unit.position,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let area = ScoutArea {
            spatial: self.spatial,
            zones,
            plan: Planning {
                pathing: self.pathing,
                now: self.world_time,
                computer: self.computer,
                object: self.object,
                construction_rate: self.construction_rate,
            },
            mover,
            me,
            player: self.player,
            master: master.id,
            radius: master.radius,
            reach: master.line_of_sight - LOS_MARGIN,
        };
        Ok((area, [w, h], master.line_of_sight))
    }
}

impl Context<'_> {
    /// A walk and the Sprite stream, for an attacker's update.
    fn walk_and_sprites<'b>(
        &'b mut self,
        mover: Option<&'b mut Mover>,
        me: Me,
    ) -> (Walk<'b>, &'b mut Well512) {
        let (object, sprite) = self.streams.two(Stream::Object, Stream::Sprite);
        (
            Walk {
                mover,
                me,
                dt: self.dt_seconds,
                world: self.spatial,
                plan: Some(Planning {
                    pathing: self.pathing,
                    now: self.world_time,
                    computer: self.computer,
                    object,
                    construction_rate: self.construction_rate,
                }),
            },
            sprite,
        )
    }

    /// A walk and another stream than the walk's Object stream.
    fn walk_with<'b>(
        &'b mut self,
        mover: Option<&'b mut Mover>,
        me: Me,
        other: Stream,
    ) -> (Walk<'b>, &'b mut Well512) {
        let (object, other) = self.streams.two(Stream::Object, other);
        (
            Walk {
                mover,
                me,
                dt: self.dt_seconds,
                world: self.spatial,
                plan: Some(Planning {
                    pathing: self.pathing,
                    now: self.world_time,
                    computer: self.computer,
                    object,
                    construction_rate: self.construction_rate,
                }),
            },
            other,
        )
    }

    /// What an object's walking action needs from the tick.
    const fn walk<'b>(&'b mut self, mover: Option<&'b mut Mover>, me: Me) -> Walk<'b> {
        Walk {
            mover,
            me,
            dt: self.dt_seconds,
            world: self.spatial,
            plan: Some(Planning {
                pathing: self.pathing,
                now: self.world_time,
                computer: self.computer,
                object: self.streams.get(Stream::Object),
                construction_rate: self.construction_rate,
            }),
        }
    }
}

/// The auto scout's search reaches within its line of sight less this (`FUN_140c65490`,
/// `DAT_14330eb8c`).
const LOS_MARGIN: f32 = crate::image::explore::LOS_MARGIN;

/// The masters of a town centre (`FUN_1416597e0`: 109, then 444).
const TOWN_CENTRES: [i16; 2] = [109, 444];

/// `FUN_1416597e0` with 0: the player's first town centre, by the player's index of objects by
/// master (`player+0x18`, `FUN_14104f560`: those of master 109, then 444), as its tile. The
/// index's order within a master is not held: two objects of the first master found are not
/// modelled.
fn town_centre(units: &[Unit]) -> Result<Option<[i32; 2]>, Unsupported> {
    for master in TOWN_CENTRES {
        let mut found = units.iter().filter(|u| u.life.master_id == master);
        let Some(first) = found.next() else { continue };
        if found.next().is_some() {
            return Err(Unsupported(
                "two town centres (the order of the player's index)",
            ));
        }
        return Ok(Some([first.position[0] as i32, first.position[1] as i32]));
    }
    Ok(None)
}

/// What the auto scout's search asks of the world for one unit ([`crate::explore::Area`]).
struct ScoutArea<'a> {
    spatial: &'a Spatial,
    zones: &'a crate::zones::ZoneMap,
    plan: Planning<'a>,
    mover: &'a Mover,
    me: Me,
    player: usize,
    master: i16,
    radius: [f32; 2],
    /// The reach of slot 0x3d8: the line of sight less a tile.
    reach: f32,
}

impl crate::explore::Area for ScoutArea<'_> {
    fn explored(&self, x: i32, y: i32) -> Result<bool, Unsupported> {
        self.spatial.explored(self.player, x, y)
    }

    fn zone(&mut self, x: i32, y: i32) -> u16 {
        self.zones.zone(x, y)
    }

    fn own_zone(&mut self) -> u16 {
        let [x, y, _] = self.me.position;
        self.zones.zone(x as i32, y as i32)
    }

    fn clear(&self, x: i32, y: i32) -> Result<bool, Unsupported> {
        let mask = self.mover.stats.restriction_mask;
        self.spatial
            .point_clear(self.me.id, [x as f32, y as f32], self.radius[0], mask)
    }

    fn reaches(&mut self, x: i32, y: i32) -> Result<bool, Unsupported> {
        let to = [x as f32, y as f32];
        self.mover.reaches(
            &self.me,
            self.spatial,
            &mut self.plan,
            to,
            self.reach,
            self.master,
        )
    }

    /// `FUN_14118faf0`: the gap from the footprint on each axis (at least 0), then its length.
    fn distance(&self, x: i32, y: i32) -> f32 {
        let gap = |d: f32, r: f32| {
            let g = d.abs() - r;
            if g <= 0.0 { 0.0 } else { g }
        };
        let gx = gap(x as f32 - self.me.position[0], self.radius[0]);
        let gy = gap(y as f32 - self.me.position[1], self.radius[1]);
        crate::fmath::sqrt(gy * gy + gx * gx)
    }
}

impl Unit {
    /// The target of its hunt, which it attacks or gathers from.
    fn attack_target(&self) -> Option<i32> {
        self.hunt
            .map(|h| h.target)
            .or_else(|| self.attack.map(|a| a.target))
            .or_else(|| {
                self.missile
                    .as_ref()
                    .and_then(|m| m.action.as_ref())
                    .map(|a| a.target)
                    .filter(|&t| t != -1)
            })
    }

    /// What the walk reads of it when it meets it ([`crate::spatial::Gait`]).
    #[must_use]
    pub fn gait(&self) -> Option<crate::spatial::Gait> {
        let m = self.mover.as_ref()?;
        Some(crate::spatial::Gait {
            moving: m
                .velocity
                .as_ref()
                .is_some_and(|v| 0.0 < crate::fmath::length(v.velocity)),
            last_move: m.last_move,
            pause: m.paths.first().map(|l| l.pause_time),
            human_order: self.last_human_order,
            path_made: m
                .paths
                .first()
                .filter(|l| !l.points.is_empty())
                .map(|l| l.created),
        })
    }

    /// The type of its top action, if it has one.
    #[must_use]
    pub fn top_action_kind(&self) -> Option<i16> {
        self.top_action().map(|a| a.kind)
    }

    /// Its top action as the slice models it, type and state.
    #[must_use]
    pub fn top_action(&self) -> Option<TopAction> {
        let action = |kind, state| TopAction { kind, state };
        None.or_else(|| {
            self.artifact
                .map(|a| action(crate::artifact::ARTIFACT_TYPE, a.state))
        })
        .or_else(|| self.build.map(|b| action(BUILD, b.state)))
        .or_else(|| {
            self.move_to
                .map(|m| action(crate::move_to::MOVE_TO, m.state.number()))
        })
        .or_else(|| self.fly.map(|f| action(crate::fly::FLY, f.state)))
        .or_else(|| self.hunt.map(|h| action(crate::hunt::HUNT, h.state)))
        .or_else(|| self.attack.map(|a| action(crate::attack::ATTACK, a.state)))
        .or_else(|| {
            self.explore
                .map(|e| action(crate::explore::EXPLORE, e.state))
        })
        .or_else(|| self.enter.map(|e| action(crate::garrison::ENTER, e.state)))
        .or_else(|| {
            self.missile
                .as_ref()
                .and_then(|m| m.action.as_ref())
                .map(|a| action(crate::missile::MISSILE, a.state))
        })
    }

    /// The target of its top action ([`Unit::top_action`]), −1 for an action without one (an
    /// Artifact, a Fly, an Explore); None without an action the slice holds.
    #[must_use]
    pub fn top_target(&self) -> Option<i32> {
        if self.artifact.is_some() {
            return Some(-1);
        }
        None.or_else(|| self.build.map(|b| b.target))
            .or_else(|| self.move_to.map(|m| m.target))
            .or_else(|| self.fly.map(|_| -1))
            .or_else(|| self.hunt.map(|h| h.target))
            .or_else(|| self.attack.map(|a| a.target))
            .or_else(|| self.explore.map(|_| -1))
            .or_else(|| self.enter.map(|e| e.target))
            .or_else(|| {
                self.missile
                    .as_ref()
                    .and_then(|m| m.action.as_ref())
                    .map(|a| a.target)
            })
    }

    /// The second target of its top action (`+0x34`, `FUN_140a78430`), −1 for none: a Hunt's
    /// drop site, a missile's shooter; the slice's other actions hold none (an import checks the
    /// save's). None without an action the slice holds.
    #[must_use]
    pub fn top_second_target(&self) -> Option<i32> {
        match self.top_action_kind()? {
            crate::hunt::HUNT => self.hunt.map(|h| h.carry_to),
            crate::missile::MISSILE => self
                .missile
                .as_ref()
                .and_then(|m| m.action.as_ref())
                .map(|a| a.fired_from),
            crate::attack::ATTACK => self.attack.map(|a| a.target2),
            _ => Some(-1),
        }
    }

    /// Slot 0x2d0 with 0 (`FUN_14106d170`): whether its top action may be interrupted (the group
    /// test first; the slice holds no groups). None may; type `0x7e` never; by state, 0, 1, 3,
    /// 0xd and 0xe may, 2 unless an Attack (9); in any other state only an Artifact (`0x6b`)
    /// without a target or a point (`+0x30`, `+0x38` = −1). A trade (`0x6f`) in state 4 is not
    /// modelled, nor is an action the slice does not hold.
    fn interruptible(&self) -> Result<bool, Unsupported> {
        if self.unmodelled_action.is_some() {
            return Err(Unsupported(
                "slot 0x2d0 of an action the slice does not model",
            ));
        }
        let Some(TopAction { kind, state }) = self.top_action() else {
            return Ok(true);
        };
        Ok(match (kind, state) {
            (0x7e, _) => false,
            (0x6f, 4) => return Err(Unsupported("slot 0x2d0 of a trade in state 4")),
            (_, 0 | 1 | 3 | 0xd | 0xe) => true,
            (_, 2) => kind != crate::attack::ATTACK,
            (crate::artifact::ARTIFACT_TYPE, _) => self
                .artifact
                .is_some_and(|a| a.goal.is_none_or(|g| g[0] == -1.0)),
            _ => false,
        })
    }

    /// The Enter's update ([`crate::garrison::EnterAction::update`]) with what it reads of its
    /// building, one of its own player's (another player's is not modelled): its state, room
    /// for the unit (its count, the units of the player inside it, plus 1 within its master's
    /// capacity) and whether the unit is in reach of it. The enter itself (`FUN_140a867b0`) is
    /// not modelled.
    fn enter_update(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
        me: Me,
    ) -> Result<(), Unsupported> {
        let enter = self.enter.expect("checked by the caller");
        let target = others.iter().find(|u| u.id == enter.target);
        let building = match target {
            Some(t) => {
                let master = c
                    .masters
                    .and_then(|m| m.get(t.life.master_id))
                    .ok_or(Unsupported(
                        "an Enter's building of a master the slice lacks",
                    ))?;
                let inside = others
                    .iter()
                    .filter(|u| u.life.inside == Some(t.id))
                    .count();
                let room = inside < usize::from(master.garrison);
                let (me_at, me_r) = c
                    .spatial
                    .get_known(self.id)?
                    .map(|p| (p.position, p.radius))
                    .ok_or(Unsupported("an entering unit without a place"))?;
                let (t_at, t_r) = c
                    .spatial
                    .get_known(t.id)?
                    .map(|p| (p.position, p.radius))
                    .ok_or(Unsupported("an Enter's building without a place"))?;
                let gap = footprint_gap_squared(me_at, me_r, t_at, t_r);
                Some(crate::garrison::EnterTarget {
                    state: t.life.state,
                    allied: true,
                    room,
                    position: t.position,
                    in_reach: crate::garrison::EnterAction::in_reach(enter.range, gap, 0.0)?,
                })
            }
            None if c.foreign.find(enter.target).is_some() => {
                return Err(Unsupported("an Enter into another player's building"));
            }
            None => None,
        };
        let walk = c.walk(self.mover.as_mut(), me);
        let step = self
            .enter
            .as_mut()
            .expect("checked above")
            .update(walk, building)?;
        if step.enter {
            let building = others
                .iter()
                .find(|u| u.id == enter.target)
                .ok_or(Unsupported("an enter into a building of another player"))?;
            let inside = others
                .iter()
                .filter(|u| u.life.inside == Some(enter.target))
                .count();
            let mut w = EnterWorld {
                spatial: c.spatial,
                pathing: c.pathing,
                masters: c.masters,
                graphics: c.graphics,
                sprite_rng: c.streams.get(Stream::Sprite),
                drop_sites: c.drop_sites,
                attributes: c.attributes,
                civ: c.civ,
                player: c.player,
                attribute_adds: &mut c.attribute_adds,
                firepower: 0.0,
            };
            if !self.enter_building(building, inside, &mut w)? {
                return Err(Unsupported("an Enter that did not get in (state 2)"));
            }
            // the update's own set_state(1) after the enter's
            self.entered(&mut w)?;
            let firepower = w.firepower;
            if let Some(b) = others.iter_mut().find(|u| u.id == enter.target) {
                b.firepower += firepower;
                b.garrison.push(self.id);
            }
        }
        if step.done {
            return Err(Unsupported("a done Enter outside its building"));
        }
        Ok(())
    }

    /// The Enter's `set_state(1)` (`FUN_140a87320` case 1): its sub-actions go (`FUN_140a98620`: a
    /// `MoveTo`'s teardown stops the unit, slot 0x5d0), the unit shows its master's standing
    /// sprite ([`Unit::shows_standing_sprite`]) and, the Enter not being quiet, its unit AI gets
    /// notify 0x1f9 with the enter action value.
    pub(crate) fn entered(&mut self, w: &mut EnterWorld<'_>) -> Result<(), Unsupported> {
        let enter = self
            .enter
            .as_mut()
            .ok_or(Unsupported("an entered unit without its Enter"))?;
        if enter.move_to.take().is_some()
            && let Some(m) = self.mover.as_mut()
        {
            m.stop()?;
        }
        enter.state = crate::garrison::ENTERED;
        self.shows_standing_sprite(w.masters, w.graphics, w.sprite_rng)?;
        self.post_own(
            crate::ai_kinds::NOTIFY_ACTION_FAILED,
            [crate::garrison::ENTER_ACTION, 0, 0],
        )
    }

    /// The enter (`FUN_140a867b0`) into `target`, one of the unit's player's buildings: a
    /// building that takes what the unit carries (`FUN_140fdd190`, as a drop site) takes its load
    /// first (the resource for kinds 15 to 17 is food, 0 to 3 and 56 their own, others none:
    /// [`give_load`]; the unit carries nothing after, `FUN_14119d310` with −1). A building at no
    /// more than a fifth of its master's hit points (in floats) and a master with a transform
    /// task (0x88) on the building's are not modelled. Then slot 0x1a0 (`FUN_141212e10` ->
    /// `FUN_141084b30` -> `FUN_1411921d0`): with room in the building, the unit's sight goes from
    /// its tile (slot 0x1d8), it leaves its tile's list (`FUN_140f5f560`), the obstruction
    /// manager (`FUN_141055b40`) and the mesh (`+0x660`, `FUN_141053840`), and joins the
    /// building's (slot 0x1c0, `FUN_1411cb850` -> `FUN_14118c890`: `+0x2f0`, at the building's
    /// x and y); its records leave the registry's lists (`FUN_14128a1d0` from its mask to 0);
    /// the building's sight takes a circle of 5 (slot 0x1d0 with 5) and the villager's town-bell
    /// flag follows the garrison (`FUN_1412067f0` with 3). Then notify 0x1fa with the enter
    /// action value and `set_state(1)` ([`Unit::entered`]). The building's firepower (`+0xa00`)
    /// and villager count (`+0xba8`) follow from the units inside; the sound, the tracker and
    /// a Gaia building's capture (slot 0x70) are not held.
    pub(crate) fn enter_building(
        &mut self,
        building: &Self,
        inside: usize,
        w: &mut EnterWorld<'_>,
    ) -> Result<bool, Unsupported> {
        let masters = w
            .masters
            .ok_or(Unsupported("an enter without the masters"))?;
        let target = building.id;
        let bm = masters.get(building.life.master_id).ok_or(Unsupported(
            "an enter into a building of a master the slice lacks",
        ))?;
        let me = masters
            .get(self.life.master_id)
            .ok_or(Unsupported("an entering unit of a master the slice lacks"))?;
        let actor = Actor {
            id: self.id,
            master: me,
            player: w.player,
            held: (self.life.attribute_type, self.life.amount),
            attribute_95: w.attributes.get(95).copied().unwrap_or(0.0),
            civ: w.civ,
        };
        if crate::tasks::drops_at(w.drop_sites, bm.id, &actor)
            && 0.0 < self.life.amount
            && bm.kind == crate::order::BUILDING
        {
            let kind = self.life.attribute_type;
            let resource = match kind {
                15..=17 => Some(0),
                0..=3 | 56 => Some(kind),
                _ => None,
            };
            if let Some(resource) = resource {
                give_load(
                    &w.spatial.relations,
                    w.player,
                    w.attribute_adds,
                    resource,
                    self.life.amount,
                )?;
                self.life.attribute_type = -1;
                self.life.amount = 0.0;
            }
        }
        if bm.kind == crate::order::BUILDING
            && building.life.hp <= f32::from(bm.hit_points) * crate::image::garrison::ENTER_HP_SHARE
        {
            return Err(Unsupported(
                "an enter into a building at a fifth of its hit points (state 13)",
            ));
        }
        if me
            .tasks
            .iter()
            .find(|t| t.action_type == 0x88)
            .is_some_and(|t| t.object_id == bm.id)
        {
            return Err(Unsupported(
                "an enter of a master that transforms (task 0x88)",
            ));
        }
        if usize::from(bm.garrison) <= inside {
            return Ok(false);
        }
        if self.life.inside.is_some() {
            return Err(Unsupported("an enter of a unit inside another"));
        }
        // the building's `+0x9a8` takes the units inside the unit (slot 0x498)
        if !self.garrison.is_empty() {
            return Err(Unsupported("an enter of a unit with units inside it"));
        }
        let placed = w
            .spatial
            .get(self.id)
            .copied()
            .ok_or(Unsupported("an entering unit without a place"))?;
        if let Some(v) = w.spatial.sight.as_mut() {
            v.sight_off(self.id, self.position, placed.radius, false, false);
        }
        let at = building.position;
        w.spatial.go_inside(self.id, at);
        w.pathing.removed(self.id);
        let building_radius = w.spatial.get(target).map_or(bm.radius, |p| p.radius);
        self.life.inside = Some(target);
        // the building's slot 0x1c0 (FUN_1411cb850 -> FUN_14120b790): a unit (type 70) adds its
        // share to the building's volleys
        if me.kind == 70 {
            let rate = crate::garrison::attack_rate(bm, masters, building.reload_powerup);
            w.firepower = crate::garrison::share(me, masters, self.reload_powerup, rate)?;
        }
        self.position = [at[0], at[1], self.position[2]];
        if let Some(v) = w.spatial.sight.as_mut() {
            if let Some(r) = v.registry.as_mut().filter(|_| 0 <= self.id) {
                let mask = r.entry(self.id).map_or(0, |e| e.mask);
                r.update_unit(
                    self.id,
                    w.player as u8,
                    [self.position[0], self.position[1]],
                    placed.radius,
                    placed.class,
                    mask & 0xffff_ffff,
                    0,
                )?;
                // its mask (`+0x8f8`) is none after
                r.entries.entry(self.id).or_default().mask = 0;
            }
            let _ = v.sight_on_los(target, at, building_radius, false, 5, false);
        }
        if !self.town_bell.flag {
            return Err(Unsupported(
                "a town-bell record at the enter (FUN_1412067f0 with 3)",
            ));
        }
        self.town_bell.flag =
            bm.building.is_some_and(|b| b.garrison_type & 1 != 0) && building.life.state == 2;
        self.post_own(
            crate::ai_kinds::NOTIFY_ACTION_COMPLETED,
            [crate::garrison::ENTER_ACTION, 0, 0],
        )?;
        self.entered(w)?;
        Ok(true)
    }

    /// stopAction (slot 0x3c0, `FUN_14108c160`): the action list goes (`FUN_140a98620`; a
    /// `MoveTo`'s teardown stops the unit) and a unit in state 2 takes its standing sprite (slot
    /// 0x68 with slot 0x798's, [`crate::attack::set_sprite`]) where the slice holds its sprite.
    /// A Build waiting without its foundation (state 3) goes with nothing to release
    /// (`FUN_140aadbf0`), its own Attack and an Enter let their target go; other actions'
    /// teardowns are not modelled.
    fn stop_action(&mut self, c: &mut Context<'_>) -> Result<(), Unsupported> {
        // its own Attack's teardown: the MoveTo's stops the unit; it lets its target go
        if let Some(mut a) = self.attack.take() {
            a.tear_down(self.mover.as_mut())?;
            for t in [a.target, a.target2] {
                if t != -1 {
                    c.releases.push((t, self.releaser(c)));
                }
            }
        }
        // the Enter's teardown: its MoveTo's stops the unit, and the base teardown
        // (FUN_140a73fc0) lets the building go (slot 0x180; its `+0x60` is 0)
        if let Some(e) = self.enter.take() {
            if e.move_to.is_some()
                && let Some(m) = self.mover.as_mut()
            {
                m.stop()?;
            }
            c.releases.push((e.target, self.releaser(c)));
        }
        self.clear_own_actions()?;
        if self.life.state == 2
            && let Some(m) = self.mover.as_mut()
        {
            m.shows_standing();
        }
        if self.life.state == 2 && self.sprite.is_some() {
            let standing = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .map(|m| m.standing_sprite)
                .and_then(|id| c.graphics.get(&id).copied())
                .ok_or(Unsupported("a stopped unit without its standing sprite"))?;
            crate::attack::set_sprite(&mut self.sprite, standing, c.streams.get(Stream::Sprite));
        }
        Ok(())
    }

    /// The action list's clearing (`FUN_140a98620`): each action's teardown (a `MoveTo`'s, also
    /// an Explore's, stops the unit, slot 0x5d0). A Build waiting without work goes; the others
    /// the slice holds are not modelled.
    fn clear_own_actions(&mut self) -> Result<(), Unsupported> {
        if self
            .build
            .as_ref()
            .is_some_and(|b| b.state == 3 && b.target == -1 && b.move_to.is_none())
        {
            self.build = None;
        }
        if self.artifact.is_some()
            || self.build.is_some()
            || self.fly.is_some()
            || self.hunt.is_some()
            || self.attack.is_some()
            || self.enter.is_some()
            || self.unmodelled_action.is_some()
        {
            return Err(Unsupported(
                "a unit AI's stop of an action list the slice does not model",
            ));
        }
        // the Explore's teardown (0x140a87dd0): its MoveTo's stops the unit (slot 0x5d0)
        if self.explore.take().is_some_and(|e| e.move_to.is_some()) {
            self.mover
                .as_mut()
                .ok_or(Unsupported("a walker without movement state"))?
                .stop()?;
        }
        if self.move_to.take().is_some() {
            self.mover
                .as_mut()
                .ok_or(Unsupported("a walker without movement state"))?
                .stop()?;
        }
        Ok(())
    }

    /// Its unit AI's effects, in order.
    fn apply(
        &mut self,
        effects: Vec<Effect>,
        class: Option<i16>,
        c: &mut Context<'_>,
    ) -> Result<(), Unsupported> {
        for e in effects {
            match e {
                Effect::Stop => self.stop_action(c)?,
                Effect::CapHumanOrder(cap) => {
                    self.last_human_order = self.last_human_order.min(cap);
                    c.spatial.set_gait(self.id, self.gait());
                }
                Effect::Move(m) => {
                    let class = class.ok_or(Unsupported(
                        "an AI move of a master the slice does not have",
                    ))?;
                    self.ai_move(m, class)?;
                }
                Effect::OrderOnTarget(order) => {
                    if c.order_on_target.replace(order).is_some() {
                        return Err(Unsupported("two orders on targets in one update"));
                    }
                }
                Effect::Explore(point) => self.explore_to(point, c)?,
                Effect::MoveToObject { target, range } => self.move_to_object(target, range, c)?,
            }
        }
        Ok(())
    }

    /// The hunt's state 8 after its load went (`0x140ac7a00` case 7, after `FUN_14119d310`): a
    /// hunt with a target walks back to it (`set_state` 4), not modelled. One without asks its unit
    /// AI: the queued orders (gather, hunt, on a target) whose target is gone or dead go
    /// (`FUN_140c5de60`); orders left end the hunt (the AI's order and action clear, `set_state`
    /// 1, the action list removes it; its teardown's release of the drop site is not held, as its
    /// take is not). The target class (the AI's target's, else the hunt's `+0x68`) the
    /// civilian's slot 0x1b0 knows (`FUN_140c1c4f0`) takes the search by kind (slot 0x80), not
    /// modelled; a hunt of class 10, a unit in a pasture (`FUN_1411c7670`) and a farm found are
    /// not modelled either. Else `ClosestObject` (`FUN_140c488b0`, every relation, below one
    /// worker for class 49) of the class over the AI's search (`FUN_140c63e20`: its sight plus a
    /// tile, at least its search radius, plus the civilian's growth; enemies found are not
    /// modelled, else the alert mode clears). The object found becomes the hunt's target (slot
    /// 0x48, [`HuntAction::retarget`], which takes it: its workers count one more); finding none
    /// is not modelled (the hunt's point, then its own search). A hunt that fails at once takes
    /// the standing sprite (slot 0x68 with `FUN_140ac5210`'s: the unit carries nothing now).
    fn hunt_after_drop(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<(), Unsupported> {
        let hunt = self.hunt.as_ref().expect("the caller's hunt");
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(Unsupported("a hunter of a master the slice does not have"))?;
        if hunt.target != -1 {
            // back to the target: the drop site goes (FUN_140a7a190 with none), set_state(4)
            let v = c
                .victim
                .as_deref()
                .ok_or(Unsupported("a hunt's target the slice does not hold live"))?;
            let quarry = crate::hunt::Quarry {
                id: v.id,
                position: v.position,
                class: v.class,
                kind: v.kind,
                state: v.state,
                player: v.player,
                amount: v.amount,
                allied: v.allied[0],
                top_action: None,
                drop_site: false,
            };
            let (me_at, them) = (
                c.spatial
                    .get_known(self.id)?
                    .ok_or(Unsupported("a hunter without a place"))?,
                c.spatial
                    .get_known(v.id)?
                    .ok_or(Unsupported("a hunt's target without a place"))?,
            );
            let speed = self.mover.as_ref().map_or(0.0, |m| m.stats.speed);
            let engagement = crate::attack::engagement(
                self.ai.as_ref(),
                speed,
                master,
                me_at,
                them,
                c.world_time,
            )?;
            let arms = crate::attack::arms(master, c.graphics, hunt.task.sprites);
            let (Some(hunt), Some(mover)) = (self.hunt.as_mut(), self.mover.as_mut()) else {
                return Err(Unsupported("a hunter without its hunt or movement state"));
            };
            hunt.carry_to = -1;
            hunt.chase(&quarry, c.player, &engagement, arms, mover)?;
            if hunt.state == crate::hunt::FAILED {
                return Err(Unsupported(
                    "a hunt failing on its target after its drop-off",
                ));
            }
            return Ok(());
        }
        let ai = self
            .ai
            .as_mut()
            .ok_or(Unsupported("a hunt without a unit AI after its drop-off"))?;
        let spatial: &Spatial = c.spatial;
        let gone = |id: i32| match spatial.get(id) {
            None => true,
            Some(_) => spatial
                .sight
                .as_ref()
                .and_then(|v| v.seer(id))
                .is_some_and(|s| 5 < s.state),
        };
        ai.orders
            .retain(|o| !(matches!(o.order, 0x2c5 | 0x2c9 | 0x2db) && gone(o.target)));
        if !ai.orders.is_empty() {
            // the AI's order and action clear (FUN_140c64b60, FUN_140c64b50), the hunt ends
            // (set_state 1; its update returns 1 and the action list removes it): its unit AI
            // runs the queued order
            ai.end_order_and_action()?;
            let mut hunt = self.hunt.take().expect("the caller's hunt");
            hunt.tear_down(self.mover.as_mut())?;
            return Ok(());
        }
        let class = if ai.target_type == -1 {
            i32::from(hunt.target_class)
        } else {
            ai.target_type
        };
        if ai.ai_type != crate::unit_ai::CIVILIAN {
            return Err(Unsupported(
                "slot 0x1b0 of another unit AI than the civilian's",
            ));
        }
        if crate::unit_ai::civilian_kind(class) != -1 {
            return Err(Unsupported("a hunt's search by kind (slot 0x80)"));
        }
        if hunt.target_class == 10 {
            return Err(Unsupported("a hunt of class 10 after its drop-off"));
        }
        if self.my_pasture != -1 {
            return Err(Unsupported("a hunter tending a pasture (FUN_1411c7670)"));
        }
        let mover = self
            .mover
            .as_ref()
            .ok_or(Unsupported("a hunter without movement state"))?;
        let sight = master.line_of_sight + 1.0;
        let radius = if sight <= master.search_radius {
            master.search_radius
        } else {
            sight
        } + crate::unit_ai::CIVILIAN_SEARCH_GROWTH;
        let search = Search {
            at: [self.position[0], self.position[1]],
            radius,
            classes: &crate::unit_ai::CIVILIAN_CARED,
            size: master.radius,
            pick: crate::unit_ai::Pick::Closest {
                group: class,
                relation: -1,
                state: -1,
                master: -1,
                workers: if class == 49 { 1 } else { -1 },
            },
        };
        let me = Me {
            id: self.id,
            position: self.position,
            action_state: hunt.state,
        };
        let mut plan = Planning {
            pathing: c.pathing,
            now: c.world_time,
            computer: c.computer,
            object: c.streams.get(Stream::Object),
            construction_rate: c.construction_rate,
        };
        let master_id = self.life.master_id;
        let mut reach = |id: i32, range: f32| {
            mover.reaches_object(&me, spatial, &mut plan, id, range, master_id)
        };
        let no_attack = |_: &Placed, _: u8| -> Result<bool, Unsupported> {
            Err(Unsupported("an attack check in a closest search"))
        };
        let foreign = c.foreign;
        let moving = |id: i32| herded(others, foreign, id);
        let result = spatial.search(self.id, c.player, &search, &no_attack, &mut reach, &moving)?;
        if result.fresh {
            if result.enemies {
                return Err(Unsupported(
                    "a hunt's search after its drop-off finding enemies",
                ));
            }
            ai.alert_mode = false;
        }
        let found = result.id;
        if found == -1 {
            return Err(Unsupported("a hunt that found nothing after its drop-off"));
        }
        let foreign = c.foreign;
        let (owner, t) = match others.iter().find(|u| u.id == found) {
            Some(t) => (c.player, t),
            None => foreign
                .find(found)
                .ok_or(Unsupported("a hunt's new target the slice does not hold"))?,
        };
        let t_master = c
            .every_masters
            .get(owner)
            .and_then(|m| m.get(t.life.master_id))
            .ok_or(Unsupported(
                "a hunt's target of a master the slice does not have",
            ))?;
        let relations = c
            .spatial
            .relations
            .get(c.player)
            .map_or(&[][..], |r| &r[..]);
        if matches!(t.life.master_id, 1893 | 1897) {
            return Err(Unsupported("a hunt finding a farm (FUN_1411c7670)"));
        }
        let quarry = crate::hunt::Quarry {
            id: found,
            position: t.position,
            class: t_master.class,
            kind: t_master.kind,
            state: t.life.state,
            player: owner,
            amount: t.life.amount,
            allied: crate::tasks::allied(relations, c.player, owner, true),
            top_action: t.top_action_kind(),
            drop_site: false,
        };
        let (me_at, them) = (
            c.spatial
                .get_known(self.id)?
                .ok_or(Unsupported("a hunter without a place"))?,
            c.spatial
                .get_known(found)?
                .ok_or(Unsupported("a hunt's target without a place"))?,
        );
        let speed = self.mover.as_ref().map_or(0.0, |m| m.stats.speed);
        let engagement =
            crate::attack::engagement(self.ai.as_ref(), speed, master, me_at, them, c.world_time)?;
        let sprites = self.hunt.as_ref().map_or([-1; 4], |h| h.task.sprites);
        let arms = crate::attack::arms(master, c.graphics, sprites);
        let drops_here = master.drop_sites.contains(&t.life.master_id);
        let held = self.life.amount;
        let (Some(hunt), Some(mover)) = (self.hunt.as_mut(), self.mover.as_mut()) else {
            return Err(Unsupported("a hunter without its hunt or movement state"));
        };
        if hunt.retarget(
            &quarry,
            c.player,
            held,
            drops_here,
            &engagement,
            arms,
            mover,
        )? {
            if owner == c.player {
                let u = others
                    .iter_mut()
                    .find(|u| u.id == found)
                    .expect("found above");
                if u.workers < 550 {
                    u.workers += 1;
                }
                u.life.goto_sleep = false;
            } else {
                c.takes.push(found);
            }
        }
        if hunt.state == crate::hunt::FAILED
            && let Some(m) = self.mover.as_mut()
        {
            m.shows_standing();
        }
        if hunt.state == crate::hunt::FAILED && self.sprite.is_some() {
            let standing = c
                .graphics
                .get(&master.standing_sprite)
                .copied()
                .ok_or(Unsupported("a hunter without its standing sprite"))?;
            crate::attack::set_sprite(&mut self.sprite, standing, c.streams.get(Stream::Sprite));
        }
        Ok(())
    }

    /// The hunt's `set_state(8)` after its gathering (`0x140ac60e0` case 8): a unit that carries
    /// nothing goes back to state 3 (not modelled). Without a drop site, its master's drop sites
    /// (`+0x7a0`; none when the first is −1) are searched among its player's objects, from the
    /// unit by path ([`drop_site_search`] with the unit), else by distance from its place (without
    /// it); none found is not modelled (notify 0x203, `set_state` 2). The drop site found is the
    /// second target (`FUN_140a7a190`) and a quiet `MoveTo` to it starts ([`HuntAction::set_drop`]).
    fn hunt_drop_off(&mut self, c: &mut Context<'_>, others: &[Self]) -> Result<(), Unsupported> {
        let w = DropWorld {
            masters: c.masters,
            spatial: c.spatial,
            zones: c.zones,
            pathing: c.pathing,
            object: c.streams.get(Stream::Object),
            world_time: c.world_time,
            computer: c.computer,
            construction_rate: c.construction_rate,
        };
        self.drop_off_in(w, others)
    }

    /// [`Unit::hunt_drop_off`] in `w`, from the update or a command.
    pub(crate) fn drop_off_in(
        &mut self,
        c: DropWorld<'_>,
        others: &[Self],
    ) -> Result<(), Unsupported> {
        if self.life.amount <= 0.0 {
            return Err(Unsupported(
                "a hunt's drop-off carrying nothing (set_state 3)",
            ));
        }
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(Unsupported("a hunter of a master the slice does not have"))?;
        let mover = self
            .mover
            .as_ref()
            .ok_or(Unsupported("a hunter without movement state"))?;
        let hunt = self.hunt.as_ref().expect("the caller's hunt");
        let mut site = hunt.carry_to;
        if site == -1 && master.drop_sites.first().is_some_and(|&d| d != -1) {
            let masters = c
                .masters
                .ok_or(Unsupported("a drop-off of a player without masters"))?;
            let [w, h] = c.spatial.map;
            let terrain = c
                .spatial
                .terrain
                .as_ref()
                .ok_or(Unsupported("the map's zones without its terrain"))?;
            let values = terrain
                .restrictions
                .get(mover.stats.restriction)
                .ok_or(Unsupported("a restriction the slice's data has not got"))?;
            let zones = c.zones.of(w, h, &terrain.kinds, values);
            let me = Me {
                id: self.id,
                position: self.position,
                action_state: hunt.state,
            };
            let mut plan = Planning {
                pathing: c.pathing,
                now: c.world_time,
                computer: c.computer,
                object: c.object,
                construction_rate: c.construction_rate,
            };
            let seeker = Seeker {
                me,
                mover,
                plan: &mut plan,
                spatial: c.spatial,
                zones,
                class: master.class,
            };
            let sites = &master.drop_sites;
            site = drop_site_search(others, masters, sites, self.position, Some(seeker))?
                .or(drop_site_search(
                    others,
                    masters,
                    sites,
                    self.position,
                    None,
                )?)
                .unwrap_or(-1);
        }
        if site == -1 {
            return Err(Unsupported(
                "a hunt without a drop site (notify 0x203, set_state 2)",
            ));
        }
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        let s = c
            .spatial
            .get_known(site)?
            .ok_or(Unsupported("a drop site without a place"))?;
        let gap = footprint_gap(self.position, ground, s.position, s.radius);
        let (Some(hunt), Some(mover)) = (self.hunt.as_mut(), self.mover.as_mut()) else {
            return Err(Unsupported("a hunter without its hunt or movement state"));
        };
        hunt.carry_to = site;
        hunt.set_drop(gap, mover)
    }

    /// The unit's explore (slot 0x398, `FUN_141084db0` with its argument 4 set) to the point at
    /// its own height: its top Explore takes the point (slot 0x48), else a new Explore
    /// (`FUN_140a87d30`) becomes its only action (`set_only_action`, `FUN_14106e990`: the actions
    /// it had go, `FUN_140a98620`, and the new one starts, `FUN_140a977e0`). A `MoveTo` of the
    /// Explore that goes stops the unit (its teardown, slot 0x5d0); other actions it would
    /// replace are not modelled.
    fn explore_to(&mut self, point: [f32; 2], c: &mut Context<'_>) -> Result<(), Unsupported> {
        let at = [point[0], point[1], self.position[2]];
        let owner = if let Some(e) = self.explore.as_mut() {
            let walked = e.move_to.is_some();
            let owner = e.take_point(at);
            if walked {
                self.stop_mover()?;
            }
            owner
        } else {
            if self.top_action().is_some() || self.unmodelled_action.is_some() {
                return Err(Unsupported("an Explore replacing another action"));
            }
            let (e, owner) = crate::explore::ExploreAction::new(at);
            self.explore = Some(e);
            owner
        };
        self.explore_owner(owner, c)
    }

    /// What an Explore's `set_state` asks of its unit ([`crate::explore::Owner`]): its notify 0x202
    /// with the explore's action value to the unit AI, and the standing sprite (slot 0x68 with
    /// the master's `+0x470`) where the slice holds the unit's sprite. `Set_state` 2's slots are
    /// not modelled.
    fn explore_owner(
        &mut self,
        owner: crate::explore::Owner,
        c: &mut Context<'_>,
    ) -> Result<(), Unsupported> {
        use crate::explore::Owner;
        match owner {
            Owner::Nothing => return Ok(()),
            Owner::Stopped => return Err(Unsupported("an Explore's set_state 2 (slots 0x68)")),
            Owner::WantsPoint => self.post_own(
                crate::explore::WANTS_POINT,
                [crate::explore::EXPLORE_ACTION, 0, 0],
            )?,
            Owner::Standing => {}
        }
        if let Some(m) = self.mover.as_mut() {
            m.shows_standing();
        }
        if self.sprite.is_some() {
            let standing = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .map(|m| m.standing_sprite)
                .and_then(|id| c.graphics.get(&id).copied())
                .ok_or(Unsupported("an explorer without its standing sprite"))?;
            crate::attack::set_sprite(&mut self.sprite, standing, c.streams.get(Stream::Sprite));
        }
        Ok(())
    }

    /// Slot 0x5d0 of a unit whose `MoveTo` went.
    fn stop_mover(&mut self) -> Result<(), Unsupported> {
        self.mover
            .as_mut()
            .ok_or(Unsupported("a walker without movement state"))?
            .stop()
    }

    /// A stop since the last look cleared the animation speed (`+0x7b8`, [`Mover::speed_stopped`]).
    fn take_stopped_speed(&mut self) {
        if let Some(m) = self.mover.as_mut().filter(|m| m.speed_stopped) {
            m.speed_stopped = false;
            self.anim_speed = Some(0.0);
        }
    }

    /// The default task's Fly ([`crate::fly::Fly::start`]): a player's bird (its task's
    /// `+0x15`, not held) is not modelled; a master of fly mode 1 lifts the bird to the fly
    /// height (slot 0x58, the position setter: its tile stays, so do its counts).
    fn start_fly(
        &mut self,
        c: &mut Context<'_>,
        task: crate::masters::Task,
    ) -> Result<(), Unsupported> {
        if c.player != 0 {
            return Err(Unsupported(
                "a player's bird's default Fly (its task's +0x15)",
            ));
        }
        let fly_mode = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .map(|m| m.fly_mode)
            .ok_or(Unsupported("a bird of a master the slice does not have"))?;
        self.fly = Some(Fly::start(
            task,
            self.position,
            c.streams.get(Stream::Action),
        ));
        if fly_mode == 1 {
            self.position[2] = crate::fly::HEIGHT;
            c.spatial.set_position(self.id, self.position);
            // the position setter marks the move (`+0x824`, `+0x870`)
            if let Some(m) = self.mover.as_mut() {
                m.mark_move(self.position, c.world_time);
            }
        }
        Ok(())
    }

    /// The death's `set_state(3)` of a combat object of type 70 (`TRIBE_Combat_Object`'s,
    /// `FUN_14121cc40`, then the base `FUN_14119d710`, whose storage and sight are the caller's)
    /// and the static update's dying sprite after it (slot 0x7a0, then slot 0x68,
    /// `FUN_141217000`): its mesh shape and its obstruction record go (`FUN_141055b40`), its own
    /// master copy (`FUN_141186d10`) has no height (`+0x5e0` 0: no longer tall), and its owner
    /// takes back its paid cost from attributes 98 and 164 (each at least 0; the caller's). Not
    /// modelled: a pasture (`FUN_141189ca0`), the artifact class (1), the classes 43 and 13 with
    /// their own cases, and a dying sprite of another type; not held: the damage sprites
    /// (`FUN_1413b9e20`), the hero attachments (`FUN_141201930`), the particles and the sounds.
    fn died(&mut self, c: &mut Context<'_>) -> Result<(), Unsupported> {
        let Some(master) = c.masters.and_then(|m| m.get(self.life.master_id)) else {
            if self.life.dying_sprite.is_some() {
                return Err(Unsupported(
                    "a dying object of a master the slice does not have",
                ));
            }
            return Ok(());
        };
        if master.kind == 80 {
            // the building's set_state (FUN_1411dbb70) from state 2: an annex's death is the
            // combat object's below; a built building's first lets its parts go
            if self.annex_of.is_none() {
                self.building_died(c, master)?;
            } else if !self.garrison.is_empty()
                || master
                    .storage
                    .iter()
                    .any(|&(kind, _, flag)| 0 <= kind && matches!(flag, 4 | 0x10))
                || self.life.attribute_type == 4
            {
                return Err(Unsupported(
                    "a dying annex with units inside or a storage of kind 4 or 0x10",
                ));
            }
        } else if master.kind != 70 {
            // a static object's dying sprite is the lifecycle's ([`Lifecycle::die`])
            if self.life.class == Class::Animated && self.life.dying_sprite.is_some() {
                return Err(Unsupported(
                    "a dying sprite of an object of another type than 70",
                ));
            }
            return Ok(());
        }
        if self.pasture != -1 || self.my_pasture != -1 {
            return Err(Unsupported("a dying unit in a pasture (FUN_141189ca0)"));
        }
        if matches!(master.class, 1 | 13 | 43) {
            return Err(Unsupported("a dying unit of class 1, 13 or 43"));
        }
        // its hit points go to 0 (`+0x278`)
        self.life.hp = 0.0;
        c.pathing.removed(self.id);
        c.spatial.drop_record(self.id);
        c.spatial.set_tall(self.id, false);
        let cost: i32 = master
            .cost
            .iter()
            .filter(|&&(resource, _, paid)| resource >= 0 && paid)
            .map(|&(_, amount, _)| i32::from(amount))
            .sum();
        c.refund = Some(cost as f32);
        if let Some(id) = self.life.dying_sprite {
            let g = c
                .graphics
                .get(&id)
                .copied()
                .ok_or(Unsupported("a dying sprite the data file does not have"))?;
            crate::attack::set_sprite(&mut self.sprite, g, c.streams.get(Stream::Sprite));
        }
        Ok(())
    }

    /// A built building's death (`FUN_1411dbb70`'s state 3 with `+0x640` 2), before the combat
    /// object's (`FUN_14121cc40`, the caller's): its owner's watched list lets it go
    /// (`FUN_141091310`, the caller's); its pasture's workers (`+0xcb8`), its storage of kinds 4
    /// and 0x10 (`FUN_1411e63f0`, `FUN_1411e6580`), the terrain under it (`+0x2e8`,
    /// `FUN_140f65580`), the castle's, monastery's and wonder's attributes, its linked objects
    /// (`+0xb78`, none held), the computer player's part (`FUN_1411e7ca0`) and the last building
    /// of a master with a technology (`FUN_141166ab0`, `FUN_1413d4ec0`) are not modelled.
    fn building_died(&self, c: &mut Context<'_>, master: &Master) -> Result<(), Unsupported> {
        let built = self
            .construction
            .is_some_and(|k| f32::from(k.build_time) <= k.progress);
        if !built {
            return Err(Unsupported(
                "a dying foundation (FUN_1411dbb70 before it is built)",
            ));
        }
        let building = master.building.ok_or(Unsupported(
            "a dying building of a master without its building part",
        ))?;
        if c.computer
            || self.pasture != -1
            || self.my_pasture != -1
            || !self.garrison.is_empty()
            || self.life.attribute_type == 4
            || master
                .storage
                .iter()
                .any(|&(kind, amount, flag)| 0 <= kind && 0.0 < amount && matches!(flag, 4 | 0x10))
        {
            return Err(Unsupported(
                "a dying building of a computer player, with a pasture, units inside or a storage of kind 4 or 0x10",
            ));
        }
        if 0 <= building.foundation_terrain
            || 0 <= building.tech
            || matches!(master.id, 82 | 104 | 276)
        {
            return Err(Unsupported(
                "a dying building with a foundation terrain or a technology, the castle, the monastery or the wonder",
            ));
        }
        c.unwatched.push(self.id);
        Ok(())
    }

    /// A notify the unit posts to its own unit AI (slot 0x328).
    /// The Build's case 0xdf once its work ended without queued orders (`0x140ab0b43`): the
    /// builder looks for more work around building `building` of master `built`
    /// ([`crate::gather_search::more_work`] with the drop site's entry for the building, the
    /// builder's class and civ, the reach the building's line of sight held to 4 to 20, and the
    /// planner of the builder's restriction). Nothing found: None, the caller lets the foundation
    /// go. A farm found is not modelled; else the unit AI's `ClosestResourceObject` for the found
    /// object's gather group and workers cap ([`crate::gather_search::gather_group`],
    /// [`crate::gather_search::closest_resource`]) names the target, the found object when it
    /// names none, and slot 0x7e8 works it at its place (the work flag 1 only logged, any task,
    /// no carried-resource test, `+0xa14` as it stands outside slot 0x128: 1; the caller's).
    fn more_work(
        &self,
        c: &mut Context<'_>,
        others: &[Self],
        building: i32,
        built: i16,
    ) -> Result<Option<crate::order::Work>, Unsupported> {
        use crate::gather_search::{self, Candidate};
        use crate::image::gather_search::{REACH_CAP, REACH_FLOOR};
        /// The reach a line of sight below the floor takes (the Build's immediate 4.0).
        const REACH_LOW: f32 = 4.0;
        let masters = c
            .masters
            .ok_or(Unsupported("a builder without its player's masters"))?;
        let master = masters
            .get(self.life.master_id)
            .ok_or(Unsupported("a builder of a master the slice does not have"))?;
        let site = masters.get(built).ok_or(Unsupported(
            "a building of a master the slice does not have",
        ))?;
        let at = *c
            .spatial
            .get(building)
            .ok_or(Unsupported("a builder's building without a place"))?;
        let entry = crate::tasks::drop_site(c.drop_sites, built, master.class, c.civ);
        let los = site.line_of_sight;
        let held = if REACH_FLOOR <= los { los } else { REACH_LOW };
        let reach = if held <= REACH_CAP { held } else { REACH_CAP };
        let builder = gather_search::Builder {
            position: self.position,
            radius: master.radius[0],
            building: at.position,
            range: reach as i32 as i16,
        };
        let mover = self
            .mover
            .as_ref()
            .ok_or(Unsupported("a builder without movement state"))?;
        let (restriction, mask) = (mover.stats.restriction, mover.stats.restriction_mask);
        let (player, me_id, foreign, every) = (c.player, self.id, c.foreign, c.every_masters);
        let candidate = |id: i32, owner: usize, u: &Self, position: [f32; 3]| {
            let m = every
                .get(owner)
                .and_then(|m| m.get(u.life.master_id))
                .ok_or(Unsupported("an object of a master the slice does not have"))?;
            Ok(Candidate {
                id,
                player: owner,
                class: m.class,
                master: m.id,
                state: u.life.state,
                workers: u.workers,
                position,
                pasture: u.pasture,
                untargetable: u.untargetable,
                attribute: u.life.attribute_type,
                amount: u.life.amount,
                auto_gather: m.auto_gather,
                gather_group: m.gather_group,
            })
        };
        let me = candidate(me_id, player, self, self.position)?;
        let spatial: &Spatial = c.spatial;
        let objects = |x: i32, y: i32| -> Result<Vec<Candidate>, Unsupported> {
            spatial
                .objects_on(x, y)
                .map(|(id, p)| {
                    if id == me_id {
                        return Ok(me);
                    }
                    let (owner, u) = others
                        .iter()
                        .find(|u| u.id == id)
                        .map(|u| (player, u))
                        .or_else(|| foreign.find(id))
                        .ok_or(Unsupported("an object on a tile the slice does not hold"))?;
                    candidate(id, owner, u, p.position)
                })
                .collect()
        };
        let found = {
            let planner = c
                .pathing
                .planner(restriction, spatial.tiles())
                .ok_or(Unsupported("a builder's restriction without a planner"))?;
            let width = i64::from(planner.level_sizes().0[0]);
            let cells = planner.level(0);
            let blocked =
                move |x: i32, y: i32| cells[(width * i64::from(y) + i64::from(x)) as usize][0] == 0;
            gather_search::more_work(entry, &builder, spatial.map, Some(&blocked), &objects)?
        };
        let Some(found) = found else {
            return Ok(None);
        };
        if found.class == gather_search::FARM {
            return Err(Unsupported(
                "a builder's search for more work finding a farm",
            ));
        }
        let (kind, cap) = gather_search::gather_group(found.attribute);
        let ai = self
            .ai
            .as_ref()
            .ok_or(Unsupported("a builder without a unit AI"))?;
        let seeker = gather_search::Seeker {
            player,
            position: self.position,
            line_of_sight: master.line_of_sight,
            state: self.life.state,
            current_target: ai.current_target,
            my_pasture: self.my_pasture,
        };
        let me_walk = Me {
            id: self.id,
            position: self.position,
            action_state: self.top_action().map_or(0, |a| a.state),
        };
        let master_id = self.life.master_id;
        let mut plan = Planning {
            pathing: c.pathing,
            now: c.world_time,
            computer: c.computer,
            object: c.streams.get(Stream::Object),
            construction_rate: c.construction_rate,
        };
        let blocked = u64::from(mask ^ 0xff) | crate::spatial::BLOCKED;
        let target = gather_search::closest_resource(
            &seeker,
            kind,
            cap,
            spatial.map,
            &|x, y| spatial.explored(player, x, y),
            &|x, y| spatial.tiles().word(x, y).is_some_and(|w| w & blocked == 0),
            &objects,
            &mut |id| mover.reaches_object(&me_walk, spatial, &mut plan, id, 0.0, master_id),
        )?;
        let target = if target != -1 && spatial.get(target).is_some() {
            target
        } else {
            found.id
        };
        let point = spatial
            .get(target)
            .ok_or(Unsupported("a builder's new work without a place"))?
            .position;
        Ok(Some(crate::order::Work {
            target,
            point,
            carry_test: false,
            filter: -1,
            notified: true,
        }))
    }

    /// The unit as the releaser of its actions' targets (slot 0x180's argument).
    fn releaser(&self, c: &Context<'_>) -> Releaser {
        Releaser {
            id: self.id,
            class: c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .map_or(-1, |m| m.class),
            player: c.player,
        }
    }

    /// The objects its unit AI names besides its target, as the world's table holds them
    /// ([`crate::unit_ai::Surroundings::units`]): its attackers and the callers of its damage
    /// notifies, from its own list or another player's, each with its master's class, state, its
    /// unit AI's target and its footprint's squared gap to the unit's (`FUN_14118f8c0`).
    fn named_units(
        &self,
        c: &Context<'_>,
        others: &[Self],
        master: Option<&Master>,
    ) -> Result<Vec<crate::unit_ai::UnitSeen>, Unsupported> {
        let Some(ai) = self.ai.as_ref() else {
            return Ok(Vec::new());
        };
        let damaged = ai
            .notifies
            .iter()
            .filter(|n| n.kind == crate::unit_ai::DAMAGED)
            .map(|n| n.caller);
        let mut ids: Vec<i32> = ai.attackers.iter().copied().chain(damaged).collect();
        ids.sort_unstable();
        ids.dedup();
        let mut seen = Vec::new();
        for id in ids {
            let found = others
                .iter()
                .find(|u| u.id == id)
                .map(|u| (c.player, u))
                .or_else(|| c.foreign.find(id));
            let (Some((owner, u)), Some(p)) = (found, c.spatial.get(id)) else {
                continue;
            };
            let m = c
                .every_masters
                .get(owner)
                .and_then(|ms| ms.get(u.life.master_id))
                .ok_or(Unsupported(
                    "a named object of a master the slice does not have",
                ))?;
            let me = master.ok_or(Unsupported(
                "a unit AI's unit of a master the slice does not have",
            ))?;
            seen.push(crate::unit_ai::UnitSeen {
                id,
                class: m.class,
                owner,
                combat_level: m.combat_level,
                state: u.life.state,
                target: u.ai.as_ref().map(|a| a.current_target),
                gap_squared: footprint_gap_squared(self.position, me.radius, p.position, p.radius),
                position: p.position,
            });
        }
        Ok(seen)
    }

    fn post_own(&mut self, kind: i32, p: [i32; 3]) -> Result<(), Unsupported> {
        let (id, state) = (self.id, self.life.state);
        self.ai
            .as_mut()
            .map_or(Ok(()), |ai| ai.post(Notify::own(id, kind, p), state))
    }

    /// The unit's own part of its hit, written back: what a retaliation or reflected damage left
    /// of its hit points and powerups, its charge and `+0x8a8`, the damage notify (500) a
    /// retaliation posts to its unit AI; a retaliation that killed it is the caller's (Gaia
    /// taking a killed attacker is not modelled).
    fn hit_back(&mut self, own: Victim) -> Result<Option<crate::damage::Kill>, Unsupported> {
        self.life.hp = own.hp;
        if let Some(a) = own.armored.as_ref() {
            self.hp_powerups = a.hp_powerups;
        }
        self.charge = own.source.charge;
        self.charge_hit = own.source.charge_hit;
        if let (Some((attacker, p)), Some(ai)) = (own.damaged, self.ai.as_mut()) {
            let n = Notify {
                caller: attacker,
                recipient: self.id,
                kind: crate::unit_ai::DAMAGED,
                p,
            };
            ai.post(n, self.life.state)?;
        }
        if own.killed.is_some_and(|k| k.to_gaia) {
            return Err(Unsupported("a killed attacker that goes to Gaia"));
        }
        Ok(own.killed)
    }

    /// The action list's deletion of the unit's own Attack (`FUN_140a98e20`): its teardown (a
    /// `MoveTo`'s stops the unit) and the base teardown (`FUN_140a73fc0`), which lets its target,
    /// then its second target go (slot 0x180).
    fn delete_attack(&mut self, c: &mut Context<'_>) -> Result<(), Unsupported> {
        let Some(mut a) = self.attack.take() else {
            return Ok(());
        };
        a.tear_down(self.mover.as_mut())?;
        let by = self.releaser(c);
        for t in [a.target, a.target2] {
            if t != -1 {
                c.releases.push((t, by));
            }
        }
        Ok(())
    }

    /// Slot 0x858 (`FUN_14120c0a0`), a combat object's hit at its target from the Attack's swing
    /// when it has a missile, a volley missile or a charge projectile
    /// ([`crate::attack::volleys`]; the others' melee hit is the Attack's). A master without
    /// `max_attacks_in_volley` fires its missile ([`Unit::fire`]). One with it fires its volley
    /// (`next_volley`, `+0xa04`, already fired of [`crate::missile::volley_count`] missiles):
    /// unless its `combat_ability` has bit 4, a master with a spread fires 1 to 3 at a time (an
    /// Object-stream draw, "attack"), the rest of the volley next with its attack timer at 0.05;
    /// the volley's last (masters 73 and 559 tell their unit AI the action executed) leaves
    /// `next_volley` 0. Of the missiles at a time, the first of a volley is its missile (none
    /// without one) and the others its extra missiles ([`Unit::volley_missile`]), each scattered
    /// by two draws. Not modelled: the self-destructing masters (527, 528, 1104, 2133..2135,
    /// classes 35 and 64 alive), a charge (charge types 6 and 7), a target of another player
    /// than Gaia (its player's object 112, slot 0x1c0) or of a computer player (its notify
    /// 0x214). Not held: the local player's statistics (`FUN_140a4ba80`).
    fn volley(&mut self, c: &mut Context<'_>, aimed: Aimed) -> Result<(), Unsupported> {
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(Unsupported(
                "an attacker of a master the slice does not have",
            ))?;
        if (matches!(master.id, 527 | 528 | 1104 | 2133..=2135) || matches!(master.class, 35 | 64))
            && self.life.state == 2
        {
            return Err(Unsupported(
                "a self-destructing attacker's hit (its object 112 and its death)",
            ));
        }
        if matches!(master.charge_type, 6 | 7) {
            return Err(Unsupported(
                "a charged hit (slot 0x2f8 with charge types 6 and 7)",
            ));
        }
        let missile = 0 <= master.missile;
        let (first, extra) = if master.max_volley == 0 {
            (true, 0)
        } else {
            let count = crate::missile::volley_count(self.firepower, master.max_volley);
            let old = self.next_volley;
            let mut n = count - i32::from(old);
            let [x_spread, y_spread, _] = master.volley_spread;
            let mut all = true;
            if master.combat_ability & 0x10 == 0 {
                let k = if 0.0 < x_spread || 0.0 < y_spread {
                    let draw = c.streams.get(Stream::Object).rand15() as i32;
                    (draw * 2) / 0x7fff + 1
                } else {
                    1
                };
                if k < n {
                    self.next_volley = self.next_volley.wrapping_add(k as u8);
                    // an immediate 0.05
                    self.attack_timer = 0.05;
                    n = k;
                    all = false;
                }
            }
            if all {
                self.next_volley = 0;
                if matches!(master.id, 73 | 559) {
                    self.post_own(ACTION_EXECUTED, [600, -1, 0])?;
                }
            }
            (old == 0, if old == 0 { n - 1 } else { n })
        };
        if missile || 0 < extra {
            if 0 < aimed.player {
                return Err(Unsupported(
                    "a hit on a player's object (its player's object 112, slot 0x1c0)",
                ));
            }
            let [x_spread, y_spread, _] = master.volley_spread;
            for _ in 0..extra {
                let object = c.streams.get(Stream::Object);
                let (x, y) = (object.rand15(), object.rand15());
                let scatter = [
                    crate::missile::scatter(x, x_spread),
                    crate::missile::scatter(y, y_spread),
                ];
                self.volley_missile(c, &aimed, scatter)?;
            }
        }
        if (extra != 0 || missile) && c.computers.get(aimed.player).copied().unwrap_or(false) {
            return Err(Unsupported(
                "a hit on a computer player's object (its notify 0x214)",
            ));
        }
        if first && missile {
            self.fire(c, &aimed)?;
        }
        Ok(())
    }

    /// `FUN_141081020`, a ranged unit's swing at its target (from its volley): its attack count
    /// (`+0x900`) rises; its master's missile (`missile_id`; a charge projectile, slot 0x2f8 with
    /// charge types 6 and 7, is not modelled) at its weapon offset turned by its facing
    /// ([`crate::missile::spawn_point`]), its reach [`crate::missile::reach`], aimed at the
    /// target's place, the shooter its origin ([`Unit::launch`], `FUN_14113f520`). Last the
    /// shooter tells its unit AI the action executed (0x200 with 600, but for masters 73 and
    /// 559). Not modelled: a target of a computer player, which a shooter whose blast does not
    /// spare friends notifies (0x20f). Not held: the damaged flag of masters 83, 293, 122 and 216
    /// (`+0x683`).
    fn fire(&mut self, c: &mut Context<'_>, aimed: &Aimed) -> Result<(), Unsupported> {
        self.attack_count += 1;
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(Unsupported(
                "a ranged unit of a master the slice does not have",
            ))?;
        if matches!(master.charge_type, 6 | 7) {
            return Err(Unsupported(
                "a charge projectile (charge types 6 and 7, slot 0x2f8)",
            ));
        }
        let facing = self
            .mover
            .as_ref()
            .map(|m| m.angle)
            .ok_or(Unsupported("a ranged unit that does not move"))?;
        let shot = Shot {
            made: master.missile,
            at: crate::missile::spawn_point(self.position, facing, master.weapon_offset),
            point: aimed.position,
            reach: crate::missile::reach(master.radius, master.weapon_range),
            shooter: true,
            aimer: crate::missile::Aimer {
                hit_chance: master.hit_chance,
                spread: master.missed_spread,
                class: master.class,
            },
            attack_count: self.attack_count,
        };
        self.launch(c, aimed, &shot)?;
        if !matches!(self.life.master_id, 73 | 559) {
            self.post_own(ACTION_EXECUTED, [600, aimed.target, 0])?;
        }
        // FUN_1410bfcb0: a blast of a unit of range above 1 without the level's bit 2 does not
        // spare friends
        let spares_friends = !(1.0 < master.weapon_range && master.blast_level & 4 == 0);
        if !spares_friends && c.computers.get(aimed.player).copied().unwrap_or(false) {
            return Err(Unsupported(
                "a missile fired at a computer player's object (its notify 0x20f)",
            ));
        }
        Ok(())
    }

    /// `FUN_141226690`, a volley's extra missile scattered by `scatter`: the master's volley
    /// missile (`FUN_141203760`, a master its player has), shooterless, else its missile with
    /// the shooter; placed by [`crate::missile::volley_spawn_point`], its reach
    /// [`crate::missile::volley_reach`], aimed at the target's place plus the scatter turned by
    /// the facing (`FUN_14113f410`); the missile's own aim when shooterless; it records the
    /// object as what it was fired from (`+0x998`). Last the object tells its unit AI the action
    /// executed (0x200 with 600 and no target, but for masters 73 and 559). Not modelled: a
    /// missile of master 527 or 528 (whose aim reads its attack count, set after it) and a
    /// master without a missile of either kind.
    fn volley_missile(
        &mut self,
        c: &mut Context<'_>,
        aimed: &Aimed,
        scatter: [f32; 2],
    ) -> Result<(), Unsupported> {
        let masters = c
            .masters
            .ok_or(Unsupported("a volley without the masters"))?;
        let master = masters.get(self.life.master_id).ok_or(Unsupported(
            "a ranged unit of a master the slice does not have",
        ))?;
        let facing = self
            .mover
            .as_ref()
            .map(|m| m.angle)
            .ok_or(Unsupported("a ranged unit that does not move"))?;
        let building = master.class == crate::garrison::BUILDING_CLASS;
        let at = crate::missile::volley_spawn_point(
            self.position,
            facing,
            master.weapon_offset,
            master.volley_spread[2],
            scatter,
            building.then_some([aimed.position[0], aimed.position[1]]),
        );
        let volley = master.volley_missile.and_then(|id| masters.get(id));
        let made = match volley {
            Some(v) => v,
            None if 0 <= master.missile => masters
                .get(master.missile)
                .ok_or(Unsupported("a missile its player's masters do not have"))?,
            None => {
                return Err(Unsupported(
                    "a volley's extra missile of a master without a missile",
                ));
            }
        };
        if matches!(made.id, 527 | 528) {
            return Err(Unsupported(
                "a volley's extra missile of master 527 or 528 (its attack count)",
            ));
        }
        let shooter = volley.is_none();
        let aimer = if shooter { master } else { made };
        let shot = Shot {
            made: made.id,
            at,
            point: crate::missile::spawn_point(
                aimed.position,
                facing,
                [scatter[0], scatter[1], 0.0],
            ),
            reach: crate::missile::volley_reach(master.radius, master.weapon_range),
            shooter,
            aimer: crate::missile::Aimer {
                hit_chance: aimer.hit_chance,
                spread: aimer.missed_spread,
                class: aimer.class,
            },
            attack_count: 0,
        };
        self.launch(c, aimed, &shot)?;
        if !matches!(self.life.master_id, 73 | 559) {
            self.post_own(ACTION_EXECUTED, [600, -1, 0])?;
        }
        Ok(())
    }

    /// A missile's making and start, which [`Unit::fire`] and [`Unit::volley_missile`] share.
    /// The missile master comes from the player's master's slot 0x40 (`FUN_1410d5920`): from the
    /// pool of type 60 when it holds one (keeping its id), else new with the next reusable id
    /// ([`Recycling`]). The setups draw as a new object's do (its standing sprite,
    /// [`ActiveSprite::start`] on the Sprite stream; its facet for a sprite with a random first
    /// frame and several facets, then its angle for a turning one: Object stream) and put it on
    /// the map (slot 0x58: held inside it; a pooled missile that last flew no farther than 0.1
    /// takes the ground by its fly mode); it joins the end of its owner's list. Then its action
    /// (`FUN_140a9a040`): the shot's point, the master's hit mode, the target taken (slot 0x178)
    /// and, with the shooter, the shooter taken too and its place, master and owner with the
    /// missile's radii as its origin (else none); the list's add starts it (`FUN_140a9c5b0`):
    /// `set_state` 3 aims ([`crate::missile::aim`], Action stream) and launches
    /// ([`crate::missile::MissileAction::launch`], its angle). The missile keeps the object's id
    /// as what it was fired from, its reach and the object's attack count. Not held: the
    /// missile's sprite and pitch facet, and a shooter's own master copy (its origin's master
    /// −1).
    fn launch(
        &mut self,
        c: &mut Context<'_>,
        aimed: &Aimed,
        shot: &Shot,
    ) -> Result<(), Unsupported> {
        let made = c
            .masters
            .and_then(|m| m.get(shot.made))
            .ok_or(Unsupported("a missile its player's masters do not have"))?;
        let stats = made
            .missile_stats
            .ok_or(Unsupported("a missile master without its flight"))?;
        if !made.recyclable {
            return Err(Unsupported("a missile of a master that is not recyclable"));
        }
        let moving = made
            .moving
            .ok_or(Unsupported("a missile master that does not move"))?;
        let (id, flying) = if let Some(pooled) = c.recycling.missiles.pop() {
            pooled
        } else {
            let id = c.recycling.next_id;
            c.recycling.next_id -= 1;
            (id, true)
        };
        // the setups' draws: static (its sprite, its facet), then moving (its angle)
        let standing = (0 <= made.standing_sprite)
            .then(|| c.graphics.get(&made.standing_sprite).copied())
            .flatten();
        if let Some(g) = standing {
            let _ = ActiveSprite::start(g, c.streams.get(Stream::Sprite));
            if g.flags & 4 != 0 && 1 < g.facets {
                let _ = c.streams.get(Stream::Object).rand15();
            }
        }
        let facets = standing.map(|g| crate::facing::SpriteFacets {
            count: g.facets,
            flag_1: g.flags & 2 != 0,
        });
        let mut angle = 0.0;
        if let Some(f) = facets.filter(|_| crate::facing::draws_angle(facets)) {
            let r = c.streams.get(Stream::Object).rand15();
            angle = crate::facing::drawn_angle(r, f);
        }
        let mut mover = Mover {
            forward: [1.0, 0.0, 0.0],
            right: [0.0, 0.0, crate::spawn::START_RIGHT_Z],
            velocity: None,
            paths: Vec::new(),
            factor: 1.0,
            stats: moving,
            waiting: 0,
            wait_delays: 0,
            attack_moving: false,
            last_move: c.world_time,
            moved_at: [0.0; 3],
            substitute_count: 0,
            ignore_units: Vec::new(),
            last_turn: 0.0,
            angle,
            move_to_point: None,
            speed_stopped: false,
            pauses: Vec::new(),
        };
        let mut position = mover.set_position(shot.at, c.spatial.map);
        if !flying {
            let ground = c
                .spatial
                .terrain
                .as_ref()
                .ok_or(Unsupported("a missile without the map's terrain"))?
                .ground(position[0], position[1])?;
            position[2] = crate::missile::grounded(position[2], ground, false, stats.fly_mode);
        }
        mover.mark_move(position, c.world_time);
        let solid = 0.0 < made.radius[0] && 0.0 < made.radius[1] && made.tall;
        let placed = Placed {
            position,
            radius: made.radius,
            class: made.class,
            player: c.player,
            master: made.id,
            obstruction_type: made.obstruction_type,
            buildable_on: made.buildable_on,
            tall: made.tall,
            speed: mover.stats.speed,
            unknown: None,
            obstruction: (made.obstruction_type != 0 && solid).then_some(
                crate::spatial::Obstruction {
                    kind: made.obstruction_type,
                    class: made.obstruction_class,
                },
            ),
        };
        c.spatial.insert(id, placed);
        c.pathing.moved(id, &placed, false);
        if let Some(v) = c.spatial.sight.as_mut() {
            v.add_seer(
                id,
                Seer {
                    player: c.player,
                    master: made.id,
                    los: made.line_of_sight,
                    state: 2,
                    explored: false,
                    building: None,
                    umv: 0,
                    capture: capture_kind(c.player, made.class, made.id),
                    kind: made.kind,
                },
            );
            let _ = v.sight_on(id, position, made.radius, false, false);
        }
        let mut held = (-1, 0.0);
        for &(kind, amount, flag) in &made.storage {
            if kind >= 0 && matches!(flag, 1 | 2 | 32) {
                c.attribute_adds.push((kind, amount));
            }
            if kind != -1 && flag == 0 {
                held = (kind, amount);
            }
        }
        // its action: the target and the shooter taken (slot 0x178), then the start
        let mut action = crate::missile::MissileAction {
            state: 0,
            target: aimed.target,
            fired_from: if shot.shooter { self.id } else { -1 },
            point: shot.point,
            timer: 0.0,
            direction: [0.0; 3],
            ballistic_velocity: 0.0,
            ballistic_acceleration: 0.0,
            distance: 0.0,
            hit_mode: stats.hit_mode,
            result: 0,
            origin: if shot.shooter {
                crate::missile::Origin {
                    position: self.position,
                    radius: made.radius,
                    master: i32::from(self.life.master_id),
                    owner: c.player as i32,
                }
            } else {
                crate::missile::Origin::NONE
            },
            met: Vec::new(),
        };
        c.takes.push(aimed.target);
        if shot.shooter {
            if self.workers < 550 {
                self.workers += 1;
            }
            self.life.goto_sleep = false;
        }
        let point = {
            let draws = c.streams.get(Stream::Action);
            crate::missile::aim(
                action.point,
                position,
                Some((aimed.radius, aimed.class)),
                shot.aimer,
                made.id,
                shot.attack_count,
                &mut action.hit_mode,
                &mut || draws.rand15(),
            )?
        };
        if let Some(a) = action.launch(position, point, &stats)? {
            mover.angle = a;
        }
        let remains = made.death_spawns[1];
        c.made.push(Self {
            id,
            position,
            life: Lifecycle {
                class: Class::Animated,
                state: 2,
                hp: f32::from(made.hit_points),
                attribute_type: held.0,
                amount: held.1,
                dying_sprite: (0 <= made.death_sprite).then_some(made.death_sprite),
                death_spawn: 0 <= made.death_spawns[0],
                remains,
                death_facets: None,
                inside: None,
                goto_sleep: false,
                master_id: made.id,
                master: made.life,
            },
            mover: Some(mover),
            missile: Some(crate::missile::Missile {
                max_range: shot.reach,
                fired_from: self.id,
                charge: false,
                flying,
                step: [0.0; 3],
                attack_count: self.attack_count,
                action: Some(action),
            }),
            attackable: true,
            pasture: -1,
            my_pasture: -1,
            ..Self::default()
        });
        Ok(())
    }

    /// The missile action's update (slot 6, `FUN_140a9e6e0`). First its target and the object it
    /// was fired from: one gone from the world's table is dropped; a dead target (above state 2
    /// but 6, `FUN_1411944f0`) is let go (slot 0x68 with none, `FUN_140a7a230`: slot 0x180).
    /// Then state 4 flies ([`crate::missile::MissileAction::fly`]): the step (`+0x7dc`) moves the
    /// missile (`FUN_14113fb60`: off the map its hit points go, slot 0xd8; the position setter,
    /// slot 0x58, holds it on the map and puts it on the ground by its fly mode once it is no
    /// farther than 0.1 from its point, [`crate::missile::grounded`]); the objects its step
    /// crossed meet it ([`Unit::missile_hits`]); a missile whose height is the ground's lands
    /// ([`Unit::missile_lands`]). A dead shooter's weapons going to the missile
    /// (`FUN_14113e170`), the sprite's pitch facet and the projectile types 1 and 2 are not
    /// modelled.
    fn fly(&mut self, c: &mut Context<'_>, others: &mut [Self]) -> Result<(), Unsupported> {
        let Some(mut missile) = self.missile.take() else {
            return Ok(());
        };
        let flown = self.fly_missile(&mut missile, c, others);
        self.missile = Some(missile);
        flown
    }

    fn fly_missile(
        &mut self,
        missile: &mut crate::missile::Missile,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<(), Unsupported> {
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(Unsupported("a missile of a master the slice does not have"))?;
        let stats = master
            .missile_stats
            .ok_or(Unsupported("a missile whose master has no flight"))?;
        let Some(action) = missile.action.as_mut() else {
            return Ok(());
        };
        let dead = |state: u8| 2 < state && state != 6;
        let target_state = match c.victim.as_deref() {
            Some(v) if v.id == action.target => Some(v.state),
            _ => None,
        };
        let shooter_state = others
            .iter()
            .find(|u| u.id == action.fired_from)
            .map(|u| u.life.state);
        if action.target != -1 && target_state.is_none() {
            if c.spatial.get(action.target).is_some() {
                return Err(Unsupported("a missile's target the slice cannot read"));
            }
            action.target = -1;
        }
        if action.fired_from != -1 && shooter_state.is_none() {
            if c.spatial.get(action.fired_from).is_some() {
                return Err(Unsupported("a missile's shooter the slice cannot read"));
            }
            action.fired_from = -1;
        }
        if action.target != -1 && target_state.is_some_and(dead) {
            c.releases.push((action.target, self.releaser(c)));
            action.target = -1;
        }
        if action.fired_from != -1 && shooter_state.is_some_and(dead) {
            return Err(Unsupported(
                "a missile whose shooter died (its weapons, FUN_14113e170)",
            ));
        }
        match action.state {
            crate::missile::FLYING => {}
            crate::missile::DONE => {
                return Err(Unsupported("a missile action's update in state 1"));
            }
            _ => return Err(Unsupported("a missile action in a state but 1 and 4")),
        }
        let (step, flying) = action.fly(c.dt_seconds, missile.max_range, &stats)?;
        missile.step = step;
        missile.flying = flying;
        let old = self.position;
        let to = [old[0] + step[0], old[1] + step[1], old[2] + step[2]];
        let [w, h] = c.spatial.map;
        if to[0] < 0.0 || w as f32 <= to[0] || to[1] < 0.0 || h as f32 <= to[1] {
            self.life.hp = 0.0;
        }
        let mover = self
            .mover
            .as_mut()
            .ok_or(Unsupported("a missile without its moving part"))?;
        let mut at = mover.set_position(to, [w, h]);
        let ground = c
            .spatial
            .terrain
            .as_ref()
            .ok_or(Unsupported("a missile without the map's terrain"))?
            .ground(at[0], at[1])?;
        at[2] = crate::missile::grounded(at[2], ground, flying, stats.fly_mode);
        mover.mark_move(at, c.world_time);
        self.position = at;
        c.spatial.set_position(self.id, at);
        if matches!(stats.projectile_type, 0..=2) {
            let fired = Fired {
                charge: missile.charge,
                by: missile.fired_from,
            };
            self.missile_hits(action, &stats, fired, old, c, others)?;
        }
        if ground == at[2] {
            self.missile_lands(missile, &stats, c, others)?;
        }
        Ok(())
    }

    /// `FUN_140a9e6e0`'s hits (projectile types 0 to 2): the objects whose obstruction boxes,
    /// grown by the missile's radius on x, its step from `old` crossed (`FUN_14113f740` ->
    /// `FUN_1411570c0`: the query `FUN_141051900` with flags 0x10, the missile itself ignored, the
    /// ends held inside the map) and that lie at its height ([`crate::missile::meets_height`]).
    /// Each one the last step did not meet, while the missile still flies: hit mode 0 hits its
    /// target alone; hit mode 1 (a miss) an object of another player that is no ally's and no
    /// missile, but not one of Gaia's above type 15 or one below the missile's blast level unless
    /// it is the target. A hit is the shooter's ([`Unit::missile_strike`]); unless the master's
    /// vanish mode has bit 0 or a shield blocked it, the missile ends (`set_state` 1: slot 0xd8,
    /// its hit points 0). The step's objects become the met ones (`+0x88`) when there are any. A
    /// missed missile's hit on another object than its target, hit mode 2 and the Hussite
    /// wagon's case (`+0x90`) are not modelled.
    fn missile_hits(
        &mut self,
        action: &mut crate::missile::MissileAction,
        stats: &crate::missile::MissileStats,
        fired: Fired,
        old: [f32; 3],
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<(), Unsupported> {
        if !(0.0 < stats.radius[0] && 0.0 < stats.radius[1]) {
            return Ok(());
        }
        let blast_level = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .map_or(0, |m| m.blast_level);
        let [w, h] = c.spatial.map;
        let hold = |v: f32, size: i32| {
            if v < 0.0 {
                0.0
            } else if size as f32 <= v {
                size as f32 - crate::missile::QUERY_EDGE
            } else {
                v
            }
        };
        let at = self.position;
        let from = [hold(old[0], w), hold(old[1], h)];
        let to = [hold(at[0], w), hold(at[1], h)];
        let rect = [
            from[0].min(to[0]),
            from[1].min(to[1]),
            from[0].max(to[0]),
            from[1].max(to[1]),
        ];
        let grow = stats.radius[0];
        c.spatial.check_known_records([
            rect[0] - grow,
            rect[1] - grow,
            rect[2] + grow,
            rect[3] + grow,
        ])?;
        let crossed = c
            .spatial
            .walk_hits(from, to, rect, grow, 1.0, Some(c.player), &[self.id])?;
        let mut met = Vec::new();
        for id in crossed {
            let Some(p) = c.spatial.get(id) else {
                continue;
            };
            let m = c
                .every_masters
                .get(p.player)
                .and_then(|m| m.get(p.master))
                .ok_or(Unsupported(
                    "an object a missile meets of a master the slice does not have",
                ))?;
            if crate::missile::meets_height(p.position[2], m.height, at[2], stats.radius[2]) {
                met.push((id, p.player, m.kind, m.blast_defense));
            }
        }
        for &(id, player, kind, level) in &met {
            if action.state == crate::missile::DONE {
                break;
            }
            if action.met.contains(&id) {
                continue;
            }
            match action.hit_mode {
                0 => {
                    if id != action.target {
                        continue;
                    }
                    let result = self.missile_strike(action, fired, c, others)?;
                    action.result = result;
                    if stats.vanish_mode & 1 == 0 && result != 2 {
                        self.end_missile(action);
                    }
                }
                1 => {
                    if player == c.player || kind == 60 {
                        continue;
                    }
                    let other = id != action.target;
                    if c.spatial.allied_with(c.player, player)
                        || (other && player == 0 && 15 < kind)
                        || (other && level < blast_level)
                    {
                        continue;
                    }
                    if other {
                        return Err(Unsupported(
                            "a missed missile meeting another object than its target",
                        ));
                    }
                    let result = self.missile_strike(action, fired, c, others)?;
                    action.result = result;
                    // the master's dying sprite plays there (a sprite of its own, not held)
                    if (stats.vanish_mode & 1 != 0 || result == 2) && blast_level <= level {
                        continue;
                    }
                    self.end_missile(action);
                }
                _ => return Err(Unsupported("a missile of hit mode 2")),
            }
        }
        if !met.is_empty() {
            action.met = met.iter().map(|m| m.0).collect();
        }
        Ok(())
    }

    /// The missile action's `set_state(1)` (`FUN_140a9de70`): the missile's slot 0xd8
    /// (`FUN_14118f580`), its hit points to 0.
    const fn end_missile(&mut self, action: &mut crate::missile::MissileAction) {
        action.state = crate::missile::DONE;
        self.life.hp = 0.0;
    }

    /// The shooter's hit for its missile (`FUN_140a9e6e0`: slot 0x860 of the object it was fired
    /// from, `FUN_1412115e0`, at the missile's place with 1 and the missile flag, the action's
    /// origin as the hit's, whose height the elevation factor reads): [`crate::attack::strike`] on
    /// the target, the executed action posted to the shooter's unit AI, the shooter's own part
    /// written back; its result. A shooterless missile hits on its own ([`Unit::own_strike`]). A
    /// charge projectile's own hit, a shooter with an area effect (its trample or blast), a
    /// building shooter (its elevation factor, `FUN_1411cc890`, reads its `+0xc20` and `+0xc24`)
    /// and a retaliation killing the shooter are not modelled.
    fn missile_strike(
        &mut self,
        action: &crate::missile::MissileAction,
        fired: Fired,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<u8, Unsupported> {
        if fired.charge {
            return Err(Unsupported("a charge projectile's own hit"));
        }
        if action.fired_from == -1 {
            return self.own_strike(action, fired.by, c);
        }
        let shooter = others
            .iter_mut()
            .find(|u| u.id == action.fired_from)
            .ok_or(Unsupported("a missile's shooter outside its list"))?;
        let master = c
            .masters
            .and_then(|m| m.get(shooter.life.master_id))
            .ok_or(Unsupported(
                "a missile's shooter of a master the slice does not have",
            ))?;
        if 0.0 < master.area_effect {
            return Err(Unsupported("a missile of a shooter with an area effect"));
        }
        if master.kind == crate::order::BUILDING {
            return Err(Unsupported(
                "a building's missile hit (its elevation factor, FUN_1411cc890)",
            ));
        }
        let terrain = c.spatial.terrain.as_ref();
        let mut own = victim_of(shooter, master, c.attributes, c.player, terrain, c.mode_680);
        let victim = c
            .victim
            .as_deref_mut()
            .filter(|v| v.id == action.target)
            .ok_or(Unsupported(
                "a missile's target the slice does not hold live",
            ))?;
        let hit = crate::attack::Hit {
            at: self.position,
            from_z: Some(action.origin.position[2]),
            missile: true,
            chance: master.hit_chance,
        };
        let object = c.streams.get(Stream::Object);
        let struck = crate::attack::strike(&mut own, victim, &mut [], hit, object)?;
        if let Some(target) = struck.executed {
            shooter.post_own(ACTION_EXECUTED, [600, target, 0])?;
        }
        if shooter.hit_back(own)?.is_some() {
            return Err(Unsupported("a retaliation killing a missile's shooter"));
        }
        Ok(struck.result)
    }

    /// A shooterless missile's own hit (`FUN_140a9e6e0`: slot 0x860 of the missile,
    /// `FUN_141084230`, at its place): [`crate::attack::strike`] with the missile as the attacker,
    /// its own master's weapons and hit chance (its origin names no master), and no elevation
    /// factor (the missile's slot 0x1f0, `FUN_14113e550`: 1 for an origin without an owner); the
    /// damage's notify comes from what it was fired from (`+0x998`, [`crate::damage`]). The
    /// executed action goes to the missile's unit AI, which it has none of; what a retaliation
    /// does to it is written back. A missile with an area effect (its blast) or whose origin
    /// names an owner is not modelled.
    fn own_strike(
        &mut self,
        action: &crate::missile::MissileAction,
        fired_from: i32,
        c: &mut Context<'_>,
    ) -> Result<u8, Unsupported> {
        if action.origin.owner != -1 {
            return Err(Unsupported(
                "a shooterless missile with an origin (its elevation, FUN_14113e550)",
            ));
        }
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(Unsupported("a missile of a master the slice does not have"))?;
        if 0.0 < master.area_effect {
            return Err(Unsupported("a shooterless missile with an area effect"));
        }
        let terrain = c.spatial.terrain.as_ref();
        let mut own = victim_of(self, master, c.attributes, c.player, terrain, c.mode_680);
        // the missile's own part is out of the unit while it flies
        own.source.fired_from = fired_from;
        let victim = c
            .victim
            .as_deref_mut()
            .filter(|v| v.id == action.target)
            .ok_or(Unsupported(
                "a missile's target the slice does not hold live",
            ))?;
        let hit = crate::attack::Hit {
            at: self.position,
            from_z: None,
            missile: true,
            chance: master.hit_chance,
        };
        let object = c.streams.get(Stream::Object);
        let struck = crate::attack::strike(&mut own, victim, &mut [], hit, object)?;
        if self.hit_back(own)?.is_some() {
            return Err(Unsupported("a retaliation killing a missile"));
        }
        Ok(struck.result)
    }

    /// `FUN_140a9e6e0`'s landing: a missile still flying toward a target it did not meet lands on
    /// it when it lies on the target's box ([`crate::missile::on_target`]) and its master's hit
    /// mode is 0 or 2, or 1 against another player's target: the shooter's hit
    /// ([`Unit::missile_strike`]), which ends it unless a shield blocked it. A missile that still
    /// flies after that hits the ground: unless its last hit was blocked, the blast of the object
    /// it was fired from there (slot 0x848, `FUN_14107f690`: nothing without an area effect; one
    /// is not modelled); it ends (`set_state` 1) and, unless its master's vanish mode has bit 1, is
    /// destroyed (slot 0xd0, `FUN_14106c420`: its action list goes, which lets its target and
    /// shooter go; `FUN_14118f1e0`: its hit points 0 and state 7).
    fn missile_lands(
        &mut self,
        missile: &mut crate::missile::Missile,
        stats: &crate::missile::MissileStats,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<(), Unsupported> {
        let fired = Fired {
            charge: missile.charge,
            by: missile.fired_from,
        };
        let charge = fired.charge;
        let Some(action) = missile.action.as_mut() else {
            return Ok(());
        };
        if action.state != crate::missile::DONE
            && action.target != -1
            && !action.met.contains(&action.target)
        {
            let v = c
                .victim
                .as_deref()
                .filter(|v| v.id == action.target)
                .ok_or(Unsupported(
                    "a missile's target the slice does not hold live",
                ))?;
            let (target_at, target_radius, target_player) = (v.position, v.radius, v.player);
            let mode = stats.hit_mode;
            if (matches!(mode, 0 | 2) || (mode == 1 && c.player != target_player))
                && crate::missile::on_target(self.position, stats.radius, target_at, target_radius)
            {
                let result = self.missile_strike(action, fired, c, others)?;
                action.result = result;
                if result != 2 {
                    self.end_missile(action);
                }
            }
        }
        if action.state == crate::missile::DONE {
            return Ok(());
        }
        if action.result != 2 {
            let blaster = if !charge && action.fired_from != -1 {
                others
                    .iter()
                    .find(|u| u.id == action.fired_from)
                    .map(|u| u.life.master_id)
            } else {
                Some(self.life.master_id)
            };
            let area = blaster
                .and_then(|id| c.masters.and_then(|m| m.get(id)))
                .map(|m| m.area_effect)
                .ok_or(Unsupported(
                    "a landed missile's blast of a master the slice does not have",
                ))?;
            if 0.0 < area {
                return Err(Unsupported("a landed missile's blast"));
            }
        }
        self.end_missile(action);
        if stats.vanish_mode & 2 == 0 {
            if let Some(a) = missile.action.take() {
                let by = self.releaser(c);
                c.releases.extend(
                    [a.target, a.fired_from]
                        .into_iter()
                        .filter(|&t| t != -1)
                        .map(|t| (t, by)),
                );
            }
            if self.life.state < 7 {
                if self.life.inside.is_some() {
                    return Err(Unsupported(
                        "a destroyed object inside another (slot 0x1a8)",
                    ));
                }
                self.life.hp = 0.0;
                self.life.goto_sleep = false;
                self.life.state = 7;
                c.spatial.sight_state(self.id, 7);
            }
        }
        Ok(())
    }

    /// `TRIBE_Building_Object::update` (`0x1411c9000`) before its combat update: a building in
    /// state 0 whose build points (`+0xc50`, the construction's progress) reached its build time
    /// (its master's first train time, `FUN_140ace5e0`) completes (`set_state(2)`,
    /// [`Construction::complete`]); outside the walls' branch (class 27, master 70) only one that
    /// is not an annex (`+0xbb8`). A builder's work completes a foundation itself
    /// (`FUN_1411cc430`); this one completes a building made without a build time (a death
    /// spawn's). The walls' branch's facet and the other branch's work before it are not modelled
    /// for such a building.
    fn complete_on_update(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), Unsupported> {
        let due = self.life.state == 0
            && self
                .construction
                .is_some_and(|cons| f32::from(cons.build_time) <= cons.progress);
        if !due {
            return Ok(());
        }
        let master = c.masters.and_then(|m| m.get(self.life.master_id));
        let walls = master.is_some_and(|m| m.class == 27 || m.id == 70);
        if !walls && self.annex_of.is_some() {
            return Ok(());
        }
        let annexed = others.iter().any(|u| u.annex_of == Some(self.id));
        let Some(construction) = self.construction.as_mut() else {
            return Ok(());
        };
        let mut site = Site {
            id: self.id,
            life: &mut self.life,
            construction,
            mover: self.mover.as_mut(),
            completion: Completion {
                master,
                annexed,
                attribute_204: c.attribute_204,
                mode_680: c.mode_680,
            },
        };
        let built = Construction::complete(&mut site, c.spatial)?;
        c.attribute_adds.extend(built.attributes);
        c.techs_done.extend(built.tech);
        Ok(())
    }

    /// One update, and whether the object stays in its list, leaves it or goes to sleep. `others`
    /// are the rest of the owner's objects, whose foundations a builder works on.
    fn update(&mut self, c: &mut Context<'_>, others: &mut [Self]) -> Result<Step, Unsupported> {
        // RGE_Static_Object::update (0x14118ad50): an object flagged to sleep (`+0x643`) drops
        // the flag, is marked asleep (`+0x641`) and moves to its player's sleeping list (player
        // slots 0x2d8 and 0x2d0), and its update ends; only static objects (type 10) are flagged
        if self.life.goto_sleep {
            if self.life.class != Class::Static || self.mover.is_some() || self.ai.is_some() {
                return Err(Unsupported("an object above the static type goes to sleep"));
            }
            self.life.goto_sleep = false;
            return Ok(Step::Sleep);
        }
        self.complete_on_update(c, others)?;
        self.take_stopped_speed();
        // RGE_Combat_Object::update (0x14107dfb0): the attack timer runs down by dt times
        // `+0x87c` (1.0 for every unit the slice has met; its writer is not ported)
        if 0.0 < self.attack_timer {
            self.attack_timer -= c.dt_seconds;
            if self.attack_timer < 0.0 {
                self.attack_timer = 0.0;
            }
        }
        // then a reload world time past ends the volley under way (`+0xa04`) and is none
        if self.reload_worldtime != 0 && self.reload_worldtime < c.world_time {
            self.next_volley = 0;
            self.reload_worldtime = 0;
        }
        if let Some(mover) = self.mover.as_mut() {
            let at = mover.step_velocity(self.position, c.dt_seconds, c.spatial.map, c.world_time);
            if at != self.position {
                self.position = at;
                c.spatial.set_position(self.id, at);
                if let Some(p) = c.spatial.get(self.id) {
                    c.pathing.moved(self.id, p, mover.is_moving());
                }
            }
            // RGE_Moving_Object::update: a last human order time (+0x650) at most FUN_140fe0d10's
            // counts down by the world's dt (+0x7c), not below 0
            if self.last_human_order <= crate::unit_ai::HUMAN_ORDER_CAP {
                self.last_human_order = self.last_human_order.saturating_sub(c.dt);
            }
            // RGE_Moving_Object::update: a unit that moves animates at its speed (+0x7b8)
            if let Some(v) = mover.velocity.as_ref() {
                let [x, y, z] = v.velocity;
                let speed = crate::fmath::sqrt(x * x + y * y + z * z);
                if SPEED_EPSILON < speed {
                    self.anim_speed = Some(speed);
                }
            }
        }
        if self.life.is_alive() && self.life.hp < 1.0 {
            // set_state(3)'s storage (FUN_1411a56c0 with 0: kinds 2 and 16) leaves the player
            let stores = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .is_some_and(|m| m.storage.iter().any(|&(_, _, kind)| matches!(kind, 2 | 16)));
            if stores {
                return Err(Unsupported("a dying object's storage leaves its player"));
            }
        }
        let before = self.life.state;
        if before == 4 && self.sprite.is_none() {
            return Err(Unsupported(
                "a dying object whose sprite the slice does not hold",
            ));
        }
        // the sprite list's slot 0x20 on the current sprite (FUN_1413b38a0)
        let animating = self
            .sprite
            .is_some_and(|s| s.animate != 0 && s.graphic.flags & 1 != 0);
        let leaves = if self
            .life
            .update(c.dt_seconds, c.streams.get(Stream::Object), animating)?
        {
            Step::Leave
        } else {
            Step::Stay
        };
        // the set_states of the steps (FUN_14119d710): a death goes through state 3
        let after = self.life.state;
        if before <= 2 && 3 <= after {
            self.died(c)?;
            // the static set_state's ghosts (FUN_14118e990), the caller's
            if c.masters
                .and_then(|m| m.get(self.life.master_id))
                .is_some_and(|m| m.ghost_on_death == crate::ghost::LEAVES_GHOSTS)
            {
                c.ghost_deaths.push(self.id);
            }
        }
        if after != before {
            if before <= 2 && 3 <= after {
                c.spatial.sight_state(self.id, 3);
            }
            if after != 3 {
                c.spatial.sight_state(self.id, after);
            }
        }
        // the dying sprite ended (RGE_Animated_Object::update): the animation stops and an object
        // below type 80 leaves its remains (FUN_1411a4e90)
        if before == 4 && after == 5 {
            self.anim_speed = Some(0.0);
            let kind = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .map(|m| m.kind)
                .ok_or(Unsupported(
                    "a dead object of a master the slice does not have",
                ))?;
            if kind < 80 && 0 <= self.life.remains {
                // its facet (`+0xe`) as its angle left it ([`crate::facing::facet_of_angle`])
                let facet = self
                    .mover
                    .as_ref()
                    .map(|m| crate::facing::facet_of_angle(m.angle, m.stats.facets).0);
                c.remains = Some((self.life.remains, self.position, facet));
            }
        }
        // a dead object that holds nothing goes to state 7 and its death spawn comes (slot 0x160,
        // FUN_1411a5090: the master's `death_spawn_obj_id` makes an object at its place; a moving
        // object's override gives it the dead object's facet and that facet's angle); the
        // spawn's sprite from slot 0x7a8 (the master's undead sprite) is not held
        if matches!(before, 3 | 5) && after == 7 && self.life.death_spawn {
            let spawn = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .map(|m| m.death_spawns[0])
                .ok_or(Unsupported(
                    "a dead object of a master the slice does not have",
                ))?;
            let facet = self
                .mover
                .as_ref()
                .map(|m| crate::facing::facet_of_angle(m.angle, m.stats.facets).0);
            c.remains = Some((spawn, self.position, facet));
        }
        // the animated object's update (0x141071880): its sprite's clock at its animation speed
        if let (Some(sprite), Some(speed)) = (self.sprite.as_mut(), self.anim_speed) {
            sprite.update(c.dt_seconds, speed);
        }
        if let Some(e) = self.unmodelled_action.filter(|_| self.life.state == 2) {
            return Err(e);
        }
        if !self.life.is_alive() {
            // RGE_Action_Object::update deletes a dead object's action list: the Artifact's
            // MoveTo's teardown (0x140aa1f30) stops the unit (slot 0x5d0)
            if let Some(artifact) = self.artifact.take()
                && artifact.move_to.is_some()
                && let Some(m) = self.mover.as_mut()
            {
                m.stop()?;
            }
            if let Some(explore) = self.explore.take()
                && explore.move_to.is_some()
                && let Some(m) = self.mover.as_mut()
            {
                m.stop()?;
            }
            // a MoveTo of its own: its teardown stops the unit, and the base teardown
            // (FUN_140a73fc0) lets its target go (slot 0x180; its `+0x60` is 0)
            if let Some(move_to) = self.move_to.take() {
                if let Some(m) = self.mover.as_mut() {
                    m.stop()?;
                }
                if move_to.target != -1 {
                    c.releases.push((move_to.target, self.releaser(c)));
                }
            }
            // an Attack's teardown, then the base teardown's releases
            self.delete_attack(c)?;
            // the missile action's teardown (FUN_140a73fc0) lets its target and the object it was
            // fired from go (slot 0x180)
            if let Some(a) = self.missile.as_mut().and_then(|m| m.action.take()) {
                let by = self.releaser(c);
                c.releases.extend(
                    [a.target, a.fired_from]
                        .into_iter()
                        .filter(|&t| t != -1)
                        .map(|t| (t, by)),
                );
            }
        }
        // RGE_Action_Object::update with no action in state 2 starts the master's default task
        // (slot 0x7f0): a bird's Fly, whose first update comes in the next tick
        let mut flies_from_now = false;
        if self.life.state == 2
            && self.top_action_kind().is_none()
            && self.production.is_none()
            && let Some(task) = self.default_fly
        {
            self.start_fly(c, task)?;
            flies_from_now = true;
        }
        if self.life.state == 2
            && self.top_action_kind().is_none()
            && self.production.is_none()
            && let Some(e) = self.default_task
        {
            return Err(e);
        }
        // The default action exists from tick 1 on, but its first update is in tick 2: fitted to
        // 16 checkpoints of idle players with livestock (workspace FD-008).
        let me = Me {
            id: self.id,
            position: self.position,
            action_state: self.top_action().map_or(0, |a| a.state),
        };
        let mut captured = None;
        if let Some(artifact) = self.artifact.as_mut().filter(|_| c.tick > 1) {
            let dt = c.dt;
            let ended = artifact.walk(Some(c.walk(self.mover.as_mut(), me)))?;
            // a failed MoveTo's teardown stops the unit
            if ended == Some(crate::unit_ai::ACTION_FAILED)
                && let Some(m) = self.mover.as_mut()
            {
                m.stop()?;
            }
            // FUN_14107cc90: an object of the combat types whose +0x924 is 0 never changes hands
            let capture = c
                .spatial
                .sight
                .as_ref()
                .and_then(|v| v.seer(self.id))
                .map(|s| s.capture);
            let searches = !(self.combat && capture == Some(0));
            let spatial: &Spatial = c.spatial;
            let attribute_97 = c.attribute_97;
            let id = self.id;
            let new = artifact.step(dt, searches, &mut || {
                spatial.artifact_owner(id, attribute_97)
            })?;
            if let Some(kind) = ended {
                self.post_own(kind, [crate::unit_ai::MOVE_ACTION, 0, 0])?;
            }
            if let Some(new) = new {
                captured = Some((new, self.capture(c, others, new)?));
            }
        }
        let by = self.releaser(c);
        if let Some(build) = self.build.as_mut() {
            let target = build.target;
            let annexed = others.iter().any(|u| u.annex_of == Some(target));
            let masters = c.masters;
            let (attribute_204, mode_680) = (c.attribute_204, c.mode_680);
            let built = others
                .iter()
                .find(|u| u.id == target)
                .map(|u| u.life.master_id);
            let site = others.iter_mut().find(|u| u.id == target).and_then(|u| {
                let master = masters.and_then(|m| m.get(u.life.master_id));
                Some(Site {
                    id: u.id,
                    life: &mut u.life,
                    construction: u.construction.as_mut()?,
                    mover: u.mover.as_mut(),
                    completion: Completion {
                        master,
                        annexed,
                        attribute_204,
                        mode_680,
                    },
                })
            });
            let class = masters
                .and_then(|m| m.get(self.life.master_id))
                .map_or(-1, |m| m.class);
            let builder = Builder {
                class,
                held: self.life.amount,
                ai: self.ai.as_mut(),
            };
            let step = build.update(c.walk(self.mover.as_mut(), me), site, builder)?;
            // a construction that started: its owner watches it (FUN_1411e5670), the caller's
            if let Some(cons) = others
                .iter_mut()
                .find(|u| u.id == target)
                .and_then(|u| u.construction.as_mut())
                .filter(|cons| cons.started)
            {
                cons.started = false;
                c.started.push(target);
            }
            if let Some(built) = step.completed {
                c.attribute_adds.extend(built.attributes);
                c.techs_done.extend(built.tech);
            }
            if step.done {
                // the action list removes it; its destructor (FUN_140aadbf0) finds it out of the
                // builders (set_state left them) and releases the foundation (slot 0x180)
                self.build = None;
                release_own(others, masters, target, by)?;
            }
            if step.released {
                release_own(others, masters, target, by)?;
                self.post_own(BUILD_IDLE, [BUILD_ACTION, 0, 0])?;
            }
            if step.search {
                let built =
                    built.ok_or(Unsupported("a builder's building the slice does not hold"))?;
                if let Some(work) = self.more_work(c, others, target, built)? {
                    c.work_order = Some(work);
                } else {
                    if let Some(b) = self.build.as_mut() {
                        b.no_more_work()?;
                    }
                    release_own(others, masters, target, by)?;
                    self.post_own(BUILD_IDLE, [BUILD_ACTION, 0, 0])?;
                }
            }
        }
        // RGE_Action_Object::update: the actions of an object inside another do not update; its
        // inside update (FUN_140a98790) deletes its top action (FUN_140a78480 -> FUN_140a784a0:
        // a dead target let go, the sub-actions' end; then the action's teardown, which lets its
        // building go, slot 0x180)
        if self.life.inside.is_some() {
            if let Some(e) = self.enter.take() {
                if e.move_to.is_some() {
                    return Err(Unsupported("an inside unit's Enter with its MoveTo"));
                }
                c.releases.push((e.target, self.releaser(c)));
            } else if self.top_action_kind().is_some() || self.unmodelled_action.is_some() {
                return Err(Unsupported("the inside update of an action not an Enter"));
            }
        } else if self.enter.is_some() {
            self.enter_update(c, others, me)?;
        }
        if self.missile.as_ref().is_some_and(|m| m.action.is_some()) {
            self.fly(c, others)?;
        }
        let step = match self.move_to.as_mut() {
            Some(move_to) => move_to.update(c.walk(self.mover.as_mut(), me))?,
            None => crate::move_to::MoveStep::default(),
        };
        if step.target_moved {
            self.post_own(TARGET_MOVED, [0, 0, 0])?;
        }
        if step.done {
            // its update in state 1 posts its completion (0x1fa with the move action value; an
            // own MoveTo is never quiet), then the action list removes it (FUN_140a98e20): its
            // teardown (0x140aa1f30) stops the unit (slot 0x5d0)
            self.post_own(ACTION_COMPLETED, [crate::unit_ai::MOVE_ACTION, 0, 0])?;
            self.move_to = None;
            if let Some(m) = self.mover.as_mut() {
                m.stop()?;
            }
        }
        if let Some(explore) = self.explore.as_mut() {
            // 0x140a880c0: state 1 is done (the action list removes it, the default task's
            // restart is not modelled), 11 walks, 13 fails; the others wait
            match explore.state {
                crate::explore::DONE => {
                    return Err(Unsupported("a done Explore (the action list after it)"));
                }
                crate::explore::FAILED => {
                    return Err(Unsupported("a failed Explore (notify 0x1f9, set_state 2)"));
                }
                crate::explore::WALK => {
                    // the MoveTo's update code (FUN_140a992b0): 1 once it arrived, 1 without it
                    let quiet = self
                        .ai
                        .as_ref()
                        .is_some_and(|ai| !matches!(ai.current_order, 711 | 733));
                    let code = match explore.move_to {
                        Some(MoveTo::Start) => {
                            explore.move_to = Some(MoveTo::Moving);
                            let goal = crate::move_to::Goal::Point(explore.point);
                            let walk = c.walk(self.mover.as_mut(), me);
                            explore.move_to =
                                Some(crate::move_to::start_walk(walk, goal, 0.0, quiet)?);
                            0
                        }
                        Some(MoveTo::Moving) => {
                            let walk = c.walk(self.mover.as_mut(), me);
                            explore.move_to = Some(crate::move_to::walk_on(walk)?);
                            0
                        }
                        Some(MoveTo::Arrived) | None => 1,
                        Some(MoveTo::Waiting | MoveTo::Failed) => {
                            return Err(Unsupported(
                                "an Explore's waiting or failed MoveTo (states 2, 13)",
                            ));
                        }
                    };
                    let walked = explore.move_to.is_some();
                    let owner = explore.after_walk(code)?;
                    if walked && self.explore.is_some_and(|e| e.move_to.is_none()) {
                        self.stop_mover()?;
                    }
                    self.explore_owner(owner, c)?;
                }
                _ => {}
            }
        }
        if let Some(fly) = self.fly.as_mut().filter(|_| !flies_from_now) {
            let mover = self
                .mover
                .as_mut()
                .ok_or(Unsupported("a bird without movement state"))?;
            let (speed, radius) = (
                mover.stats.speed,
                [mover.stats.radius[0], mover.stats.radius[1]],
            );
            let map = c.spatial.map;
            let (walk, action) = c.walk_with(Some(&mut *mover), me, Stream::Action);
            let sky = crate::fly::Sky {
                speed,
                position: me.position,
                radius,
                map,
                action,
            };
            if fly.update(walk, sky)? {
                mover.stop()?;
            }
        }
        // the Hunt's update (0x140ac7a00) first checks its target: one gone from the world's table
        // is dropped (slot 0x68 with none), one past state 6 is let go (slot 0x180, its `+0x60`
        // being 0) and dropped; the drop site's checks (`+0x34`) are not modelled
        if let Some(target) = self.hunt.map(|h| h.target).filter(|&t| t != -1) {
            let state = c
                .victim
                .as_deref()
                .filter(|v| v.id == target)
                .map(|v| v.state);
            let gone = match state {
                Some(s) => 6 < s,
                None if c.spatial.get(target).is_some() => {
                    return Err(Unsupported("a hunt's target the slice cannot read"));
                }
                None => true,
            };
            if gone {
                if state.is_some() {
                    c.releases.push((target, self.releaser(c)));
                }
                if let Some(h) = self.hunt.as_mut() {
                    h.target = -1;
                }
            }
        }
        // the hunter as a combat object, which its hits read and change
        let own = (self.hunt.is_some() || self.attack.is_some())
            .then_some(())
            .and_then(|()| {
                let master = c.masters?.get(self.life.master_id)?;
                let terrain = c.spatial.terrain.as_ref();
                Some(victim_of(
                    self,
                    master,
                    c.attributes,
                    c.player,
                    terrain,
                    c.mode_680,
                ))
            });
        if let Some(hunt) = self
            .hunt
            .as_mut()
            .filter(|h| h.state == crate::hunt::DROP && h.timer < 0.0)
        {
            // case 7 waiting to try again: the timer runs by dt; at its end the drop site goes
            // (FUN_140a7a190 with none) and set_state(8) searches again
            hunt.timer += c.dt_seconds;
            if 0.0 <= hunt.timer {
                hunt.timer = 0.0;
                hunt.carry_to = -1;
                self.hunt_drop_off(c, others)?;
            }
        } else if let Some(hunt) = self.hunt.as_mut().filter(|h| h.state == crate::hunt::DROP) {
            // state 8 has no target: its drop site (the second target) is what it walks to
            let master = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(Unsupported("a hunter of a master the slice does not have"))?;
            let me_at = c
                .spatial
                .get_known(self.id)?
                .ok_or(Unsupported("a hunter without a place"))?;
            let (me_position, me_radius) = (me_at.position, me_at.radius);
            let site = match c.spatial.get_known(hunt.carry_to)? {
                Some(s) => {
                    let actor = Actor {
                        id: self.id,
                        master,
                        player: c.player,
                        held: (self.life.attribute_type, self.life.amount),
                        attribute_95: c.attributes.get(95).copied().unwrap_or(0.0),
                        civ: c.civ,
                    };
                    Some(crate::hunt::DropOff {
                        takes: crate::tasks::drops_at(c.drop_sites, s.master, &actor),
                        gap_squared: footprint_gap_squared(
                            me_position,
                            me_radius,
                            s.position,
                            s.radius,
                        ),
                    })
                }
                None => None,
            };
            let attributes = c.attributes.len();
            let walk = c.walk(self.mover.as_mut(), me);
            if let Some((kind, amount)) = hunt.update_drop(
                walk,
                &mut self.life.attribute_type,
                &mut self.life.amount,
                attributes,
                site,
            )? {
                // the player's slot 0xf8 (FUN_141616820) with 1 and 1: the resource, and its
                // gathered total (166 to 168; gold's 49 and 169); the statistics (slot 0xb8) and
                // the local player's tracker (FUN_14097daf0) are not held. With 1 the resources
                // would go to the allies too under +0x1426 (FUN_141626780): a player with allies
                // is not modelled
                give_load(
                    &c.spatial.relations,
                    c.player,
                    &mut c.attribute_adds,
                    kind,
                    amount,
                )?;
                self.hunt_after_drop(c, others)?;
            }
        } else if let Some(hunt) = self
            .hunt
            .as_mut()
            .filter(|h| h.state == crate::hunt::FAILED)
        {
            // state 13 (0x140ac7a00 case 0xc): notify 0x1f9 with the hunt's action value, and the
            // update returns 3: the action list removes the hunt (FUN_140a98e20), whose teardown
            // lets its target go
            let target = hunt.target;
            if hunt.attack.is_some() || hunt.move_to.is_some() {
                return Err(Unsupported("a failed hunt with a sub-action"));
            }
            self.post_own(
                crate::unit_ai::ACTION_FAILED,
                [crate::unit_ai::HUNT_ACTION, 0, 0],
            )?;
            self.hunt = None;
            if target != -1 {
                if !others.iter().any(|u| u.id == target) {
                    return Err(Unsupported("a failed hunt's target of another player"));
                }
                let by = self.releaser(c);
                release_own(others, c.masters, target, by)?;
            }
        } else if let Some(hunt) = self
            .hunt
            .as_mut()
            .filter(|h| h.state == crate::hunt::WALK_POINT)
        {
            // state 15: the walk to a move's point; its end is set_state(2): the unit takes the
            // hunt's sprite to stand with (slot 0x68 with FUN_140ac5210's), the target and the
            // second target go
            let computer = c.computer;
            let walk = c.walk(self.mover.as_mut(), me);
            if hunt.update_point_walk(walk, computer)? {
                let task = hunt.task;
                let second = std::mem::replace(&mut hunt.carry_to, -1);
                let master = c
                    .masters
                    .and_then(|m| m.get(self.life.master_id))
                    .ok_or(Unsupported("a hunter of a master the slice does not have"))?;
                let stand = crate::hunt::stand_sprite(
                    &task,
                    master.standing_sprite,
                    self.life.attribute_type,
                    self.life.amount,
                    c.graphics,
                );
                if let Some(s) = stand {
                    crate::attack::set_sprite(&mut self.sprite, s, c.streams.get(Stream::Sprite));
                }
                if second != -1 {
                    c.releases.push((second, self.releaser(c)));
                }
            }
        } else if self
            .hunt
            .as_ref()
            .is_some_and(|h| h.state == crate::hunt::IDLE)
        {
            // state 2 (case 1): a done hunt waits; one of a tree's target class would search
            // again (set_state 3)
            if self.hunt.as_ref().is_some_and(|h| h.target_class == 15) {
                return Err(Unsupported("a done hunt of a tree's class (set_state 3)"));
            }
        } else if let Some(hunt) = self
            .hunt
            .as_mut()
            .filter(|h| h.target == -1 && h.state == crate::hunt::WALK && h.timer < 0.0)
        {
            // case 3 waiting after a given-up MoveTo, its target gone: the timer runs by dt
            hunt.timer += c.dt_seconds;
            if 0.0 <= hunt.timer {
                return Err(Unsupported("a hunt's set_state 11 after its wait"));
            }
        } else if let Some(hunt) = self.hunt.as_mut() {
            let computer = c.computer;
            let master = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(Unsupported("a hunter of a master the slice does not have"))?;
            let victim = c.victim.take().ok_or(Unsupported(
                "an attack's target the slice does not hold live",
            ))?;
            let own = own.ok_or(Unsupported("a hunter of a master the slice does not have"))?;
            // the gatherer's view: the master's work rate (+0x798) plus the powerup (slot 0x6e8)
            let productivity = usize::try_from(hunt.task.productivity)
                .ok()
                .and_then(|i| c.attributes.get(i))
                .copied()
                .filter(|&v| 0.0 < v)
                .unwrap_or(1.0);
            let g = |id: i16| (0 <= id).then(|| c.graphics.get(&id).copied()).flatten();
            let sprites = HuntSprites {
                moving: g(hunt.task.sprites[0]),
                working: g(hunt.task.sprites[2]),
                carrying: g(hunt.task.sprites[3]),
                standing: g(master.standing_sprite),
            };
            let mut gather = Gatherer {
                held_kind: &mut self.life.attribute_type,
                held: &mut self.life.amount,
                work_rate: self.work_rate_powerup + master.work_rate,
                capacity: f32::from(master.capacity),
                productivity,
                target_class: victim.class,
                target_storage: victim.storage,
                stops_decay: c.attributes.get(292) == Some(&1.0),
                ai_class: self.ai.as_ref().map(|ai| ai.target_type),
                sprites,
            };
            // what set_state(6) on live livestock reads (0x140ac60e0 case 6)
            let herd = if hunt.state == crate::hunt::WALK
                && hunt.move_to == Some(MoveTo::Arrived)
                && victim.class == crate::hunt::LIVESTOCK
                && victim.state <= 2
            {
                let at = |id| {
                    c.spatial
                        .get_known(id)?
                        .ok_or(Unsupported("a hunt's object without a place"))
                };
                let (me_at, them) = (at(self.id)?, at(hunt.target)?);
                let speed = self.mover.as_ref().map_or(0.0, |m| m.stats.speed);
                let engagement = crate::attack::engagement(
                    self.ai.as_ref(),
                    speed,
                    master,
                    me_at,
                    them,
                    c.world_time,
                )?;
                let arms = crate::attack::arms(master, c.graphics, hunt.task.sprites);
                let own = them.player == c.player;
                let drop_site = if !own {
                    None
                } else if hunt.drop_site != -1 {
                    let site = c.spatial.get(hunt.drop_site);
                    site.map(|s| {
                        let d =
                            footprint_gap_squared(them.position, them.radius, s.position, s.radius);
                        (hunt.drop_site, d)
                    })
                } else {
                    if computer {
                        return Err(Unsupported(
                            "a computer player's hunt of its livestock (its drop-site search)",
                        ));
                    }
                    let sites: Vec<i16> = c
                        .drop_sites
                        .iter()
                        .filter(|d| d.livestock)
                        .map(|d| d.building)
                        .collect();
                    nearest_drop_site(others, &sites, them.position)
                        .map(|(id, _)| -> Result<(i32, f32), Unsupported> {
                            let s = at(id)?;
                            Ok((
                                id,
                                footprint_gap_squared(
                                    them.position,
                                    them.radius,
                                    s.position,
                                    s.radius,
                                ),
                            ))
                        })
                        .transpose()?
                };
                Some(crate::hunt::Herd {
                    own,
                    drop_site,
                    engagement,
                    arms,
                })
            } else {
                None
            };
            let mut blasted = std::mem::take(&mut c.blasted);
            // the unit AI's canMoveToAttack (FUN_140c48420) and stance
            let can_move = self
                .ai
                .as_ref()
                .is_some_and(|ai| ai.current_order != -1 || ai.stance <= 1);
            let stance = self.ai.as_ref().map_or(0, |ai| ai.stance);
            let world_time = c.world_time;
            let standing = c.graphics.get(&master.standing_sprite).copied();
            let volleys = c.masters.is_some_and(|m| crate::attack::volleys(master, m));
            let (walk, sprite_rng) = c.walk_and_sprites(self.mover.as_mut(), me);
            let mut strike = Strike {
                sprite: &mut self.sprite,
                anim_speed: &mut self.anim_speed,
                standing,
                volleys,
                attack_timer: &mut self.attack_timer,
                reload_worldtime: &mut self.reload_worldtime,
                reload: self.reload_stinger + (master.reload - self.reload_powerup),
                hit_chance: master.hit_chance,
                victim,
                blasted: &mut blasted,
                sprite_rng,
                own,
                executed: None,
                fire: None,
                can_move,
                stance,
                posted: Vec::new(),
                queue_empty: self.ai.as_ref().is_some_and(|ai| ai.orders.is_empty()),
                world_time,
                ai_order: self.ai.as_ref().map(|ai| ai.current_order),
                computer,
                armed: true,
                let_go: None,
                forgets: false,
            };
            let full = hunt.update(walk, computer, &mut strike, &mut gather, herd)?;
            let fired = strike.fire.map(|target| {
                let v = &*strike.victim;
                Aimed {
                    target,
                    position: v.position,
                    radius: [v.radius[0], v.radius[1], v.height],
                    class: v.class,
                    player: v.player,
                }
            });
            let (executed, own) = (strike.executed, std::mem::take(&mut strike.own));
            c.blasted = blasted;
            if let Some(aimed) = fired {
                self.volley(c, aimed)?;
            }
            if let Some(target) = executed {
                self.post_own(ACTION_EXECUTED, [600, target, 0])?;
            }
            c.own_killed = self.hit_back(own)?;
            if full {
                self.hunt_drop_off(c, others)?;
            }
        } else if let Some(attack) = self.attack.as_mut().filter(|a| a.target == -1) {
            // the unit's own Attack after its target went ([`Attack::lost`])
            let master = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(Unsupported(
                    "an attacker of a master the slice does not have",
                ))?;
            let armed = armed(master, self.firepower);
            let code = attack.update_targetless(c.dt_seconds, armed, &mut self.anim_speed)?;
            if matches!(code, 1 | 3) {
                self.delete_attack(c)?;
            }
        } else if let Some(attack) = self.attack.as_mut() {
            // the unit's own Attack (slot 6, 0x140a82ae0), its notifies posted to its unit AI
            // (slot 0x328); codes 1 and 3 delete it (RGE_Action_Object::update)
            let master = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(Unsupported(
                    "an attacker of a master the slice does not have",
                ))?;
            let armed = armed(master, self.firepower);
            let computer = c.computers.get(c.player).copied().unwrap_or(false);
            let victim = c.victim.take().ok_or(Unsupported(
                "an attack's target the slice does not hold live",
            ))?;
            let own = own.ok_or(Unsupported(
                "an attacker of a master the slice does not have",
            ))?;
            let mut blasted = std::mem::take(&mut c.blasted);
            let can_move = self
                .ai
                .as_ref()
                .is_some_and(|ai| ai.current_order != -1 || ai.stance <= 1);
            let stance = self.ai.as_ref().map_or(0, |ai| ai.stance);
            let world_time = c.world_time;
            let standing = c.graphics.get(&master.standing_sprite).copied();
            let volleys = c.masters.is_some_and(|m| crate::attack::volleys(master, m));
            let (walk, sprite_rng) = c.walk_and_sprites(self.mover.as_mut(), me);
            let mut strike = Strike {
                sprite: &mut self.sprite,
                anim_speed: &mut self.anim_speed,
                standing,
                volleys,
                attack_timer: &mut self.attack_timer,
                reload_worldtime: &mut self.reload_worldtime,
                reload: self.reload_stinger + (master.reload - self.reload_powerup),
                hit_chance: master.hit_chance,
                victim,
                blasted: &mut blasted,
                sprite_rng,
                own,
                executed: None,
                fire: None,
                can_move,
                stance,
                posted: Vec::new(),
                queue_empty: self.ai.as_ref().is_some_and(|ai| ai.orders.is_empty()),
                world_time,
                ai_order: self.ai.as_ref().map(|ai| ai.current_order),
                computer,
                armed,
                let_go: None,
                forgets: false,
            };
            let code = attack.update(walk, &mut strike)?;
            let fired = strike.fire.map(|target| {
                let v = &*strike.victim;
                Aimed {
                    target,
                    position: v.position,
                    radius: [v.radius[0], v.radius[1], v.height],
                    class: v.class,
                    player: v.player,
                }
            });
            let (executed, own) = (strike.executed, std::mem::take(&mut strike.own));
            let posted = std::mem::take(&mut strike.posted);
            let (let_go, forgets) = (strike.let_go, strike.forgets);
            c.blasted = blasted;
            if let Some(t) = let_go {
                c.releases.push((t, self.releaser(c)));
            }
            if forgets && let Some(ai) = self.ai.as_mut() {
                ai.remove_target();
            }
            for (kind, p) in posted {
                self.post_own(kind, p)?;
            }
            if let Some(aimed) = fired {
                self.volley(c, aimed)?;
            }
            if let Some(target) = executed {
                self.post_own(ACTION_EXECUTED, [600, target, 0])?;
            }
            c.own_killed = self.hit_back(own)?;
            if matches!(code, 1 | 3) {
                self.delete_attack(c)?;
            }
        }
        // the motion step may have put the unit onto a waypoint it reached
        if let Some(p) = c.spatial.get(self.id) {
            self.position = p.position;
        }
        if let Some(m) = self.mover.as_mut() {
            c.pauses.append(&mut m.pauses);
        }
        // RGE_Action_Object::update's end (FUN_141163080): an action object's shape
        if let Some(m) = c
            .masters
            .and_then(|ms| ms.get(self.life.master_id))
            .filter(|m| 40 <= m.kind)
            && let Some(p) = c.spatial.get(self.id).copied()
        {
            let moving = self.mover.as_ref().is_some_and(Mover::is_moving);
            c.pathing.refresh(self.id, &p, moving, m.outline);
        }
        // RGE_Combat_Object::update after the actions: the object's sight mask
        if self.combat {
            c.spatial.combat_sight(self.id)?;
        }
        let busy = self.top_action_kind().is_some() || self.unmodelled_action.is_some();
        let master = c.masters.and_then(|ms| ms.get(self.life.master_id));
        let class = master.map(|ms| ms.class);
        // a captured unit's unit AI runs as its new owner's (RGE_Combat_Object::update reads the
        // owner then): on that player's turn, with its time as it stands
        let (turn, player, computer) = match &captured {
            Some((new, _)) => (
                (*new == c.player_turn)
                    .then(|| c.player_time.get(*new).copied())
                    .flatten(),
                *new,
                c.computers.get(*new).copied().unwrap_or(false),
            ),
            None => (c.turn, c.player, c.computer),
        };
        // RGE_Combat_Object::update: on the owner's turn the full update (slot 4), else
        // FUN_140c68980 for a unit not inside another; a dead unit's full update only adds the
        // world's dt to its time since the last enemy sighting (`+0x1b4`) before it returns
        if turn.is_some() && self.ai.is_some() {
            // the full update's start lets the unit AIs' search cache go
            c.spatial.forget_search();
        }
        if turn.is_some()
            && !self.life.is_alive()
            && let Some(ai) = self.ai.as_mut()
        {
            ai.time_since_enemy += c.dt_seconds;
        }
        let runs = turn.is_some() || self.life.inside.is_none();
        if let Some(orders) = self
            .ai
            .as_ref()
            .filter(|_| self.life.is_alive() && runs)
            .map(|ai| !ai.orders.is_empty())
        {
            if turn.is_some() && busy && self.life.inside.is_some() && self.enter.is_none() {
                return Err(Unsupported("a busy unit inside another"));
            }
            // slot 0x2d0 matters only to a queue with orders
            let interruptible = match self.interruptible() {
                Ok(i) => i,
                Err(e) if orders => return Err(e),
                Err(_) => false,
            };
            let at = Surroundings {
                position: self.position,
                speed: self.mover.as_ref().map_or(0.0, |m| m.stats.speed),
                map: c.spatial.map,
                dt_seconds: c.dt_seconds,
                mode_1: c.mode_1,
                busy,
                player,
                class: match class {
                    Some(class) => class,
                    None if busy => {
                        return Err(Unsupported(
                            "a busy unit of a master the slice does not have",
                        ));
                    }
                    None => -1,
                },
                size: master.map_or([0.0; 2], |m| m.radius),
                los: master.map_or(0.0, |m| m.line_of_sight),
                search_radius: master.map_or(0.0, |m| m.search_radius),
                weapon_range: master.map_or(0.0, |m| m.weapon_range),
                interruptible,
                task_0x99: master.is_some_and(|m| m.tasks.iter().any(|t| t.action_type == 0x99)),
                garrisoned: self.life.inside.is_some(),
                foundations: master.is_some_and(|m| {
                    let reach = m.search_radius.max(m.line_of_sight) as i32;
                    let (x, y) = (self.position[0] as i32, self.position[1] as i32);
                    others.iter().any(|u| {
                        let (ux, uy) = (u.position[0] as i32, u.position[1] as i32);
                        u.life.state == 0
                            && c.masters
                                .and_then(|ms| ms.get(u.life.master_id))
                                .is_some_and(|um| 80 <= um.kind)
                            && x - reach <= ux
                            && ux < x + reach
                            && y - reach <= uy
                            && uy < y + reach
                    })
                }),
                state: self.life.state,
                kind: master.map_or(0, |m| m.kind),
                capture: c
                    .spatial
                    .sight
                    .as_ref()
                    .and_then(|v| v.seer(self.id))
                    .map_or(0, |s| s.capture),
                master: self.life.master_id,
                scenario: c.scenario,
                task_0x69: master.is_some_and(|m| m.tasks.iter().any(|t| t.action_type == 0x69)),
                target: c.ai_target?,
                under_attack: self.under_attack,
                id: self.id,
                anim_speed: self.anim_speed,
                action_state: if self.unmodelled_action.is_some() {
                    None
                } else {
                    Some(self.top_action().map_or(0, |a| a.state))
                },
                units: self.named_units(c, others, master)?,
            };
            // the unit AI runs on its own; what it asks of its unit comes back through `UnitView`
            let mut ai = self.ai.take().expect("filtered above");
            let effects = if let Some(dt) = turn {
                {
                    let attacker = crate::unit_ai::Attacker {
                        ai_type: ai.ai_type,
                        player,
                        state: self.life.state,
                        class: master.map_or(-1, |m| m.class),
                        attack_priority: master.map_or(0, |m| m.attack_priority),
                        missile: master.map_or(-1, |m| m.missile),
                        scenario: c.scenario,
                    };
                    let [ai_stream, object, sprite] =
                        c.streams.three(Stream::Ai, Stream::Object, Stream::Sprite);
                    let result = {
                        let mut side = UnitView {
                            unit: self,
                            attacker,
                            player,
                            computer,
                            spatial: c.spatial,
                            pathing: c.pathing,
                            object,
                            sprite,
                            world_time: c.world_time,
                            construction_rate: c.construction_rate,
                            others,
                            zones: c.zones,
                            foreign: c.foreign,
                            every: c.every_masters,
                            masters: c.masters,
                            graphics: c.graphics,
                            takes: &mut c.takes,
                        };
                        ai.update(dt, c.world_time, computer, &at, ai_stream, &mut side)
                    };
                    self.ai = Some(ai);
                    result?
                }
            } else {
                let result = ai.off_turn(&at);
                self.ai = Some(ai);
                result?
            };
            self.apply(effects, class, c)?;
        }
        if let Some((new, also)) = captured {
            return Ok(Step::Captured(new, also));
        }
        Ok(leaves)
    }

    /// `FUN_140aab570`: player `new` takes the animal over. Its stop (slot 0x148,
    /// `FUN_14108c050` -> `FUN_14106fa60`): its Artifact action's slot 0x38 (the unit AI of a
    /// class other than livestock would stop too, which is not modelled), then the ownership
    /// change (slot 0x70, [`Unit::change_owner`]); a livestock's new owner (not Gaia) then takes
    /// Gaia's livestock in state 2 on the tiles it sees within 3 of the animal's tile, x then y,
    /// each the same way. Returns the others taken; the lists' moves and the storage are the
    /// caller's. The new owner's computer AI part (`FUN_140aab130`), the local player's sound and
    /// statistics are not held.
    fn capture(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
        new: usize,
    ) -> Result<Vec<i32>, Unsupported> {
        let class = c
            .spatial
            .get(self.id)
            .map(|p| p.class)
            .ok_or(Unsupported("a captured object the world does not hold"))?;
        if class != LIVESTOCK && self.ai.is_some() {
            return Err(Unsupported("a captured unit's unit AI stop (slot 0xb0)"));
        }
        if let Some(a) = self.artifact.as_mut() {
            a.stop()?;
        }
        self.change_owner(c, new)?;
        let mut taken = Vec::new();
        if class == LIVESTOCK && new != 0 {
            scan_livestock(c, others, self.position, new, &mut taken)?;
        }
        Ok(taken)
    }

    /// The ownership change (slot 0x70, `FUN_14120f800`) of an object whose new owner has its
    /// master: the world's part ([`Spatial::change_owner`], the new owner's master's line of sight
    /// for livestock's own master copy, `FUN_141186d10`) and its Artifact's wait (the owner's).
    /// Not held: `+0x6a8`, the names (`FUN_141189810`), the special-object list
    /// (`FUN_141677380`, `+0x30c` bit 0), the class counters (`FUN_141226060`: none for
    /// livestock), the own master copy itself (the slice keeps reading the new owner's master
    /// by id).
    fn change_owner(&mut self, c: &mut Context<'_>, new: usize) -> Result<(), Unsupported> {
        let master = c
            .every_masters
            .get(new)
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(Unsupported(
                "an ownership change to a player without the master",
            ))?;
        if self.life.inside.is_some() {
            return Err(Unsupported(
                "an ownership change of an object inside another",
            ));
        }
        c.spatial.change_owner(self.id, new, master.line_of_sight)?;
        if let Some(a) = self.artifact.as_mut() {
            a.gaia = new == 0;
        }
        // the unit AI's notifies go with the world time (FUN_140c5dc10), then the unit tells it
        // it changed hands (0x20b with the new owner)
        if let Some(ai) = self.ai.as_mut() {
            ai.clear_notifies(c.world_time)?;
        }
        self.post_own(OWNER_CHANGED, [new as i32, 0, 0])?;
        Ok(())
    }

    /// The unit's slot 0x340 (`FUN_141216c50`) on its unit AI's move: a unit of class 51 or 54
    /// (packing) is not modelled; any other runs `FUN_1410866e0`. Without `force`, a `MoveTo` to the
    /// same point (its `+0x38`..`+0x40`) stands. Then the current action's slot 0x40 may take the
    /// point: an Artifact's (`FUN_140aaca30`, [`Artifact::take_point`]; a `MoveTo` it walked goes and
    /// stops the unit) does, a `MoveTo`'s (`FUN_140a78490`) never (other actions are not modelled).
    /// Otherwise a `MoveTo` to the point within the range (`FUN_140aa0d30`; its sprite, the master's
    /// walking or running one, is not held) replaces the action list (`FUN_140a98620`: the old
    /// `MoveTo` stops the unit) and starts (slot 0x808, `FUN_14106e990`; its start `0x140aa2100`
    /// gives a unit far from the point state 3).
    fn ai_move(&mut self, m: AiMove, class: i16) -> Result<(), Unsupported> {
        if matches!(class, 51 | 54) {
            return Err(Unsupported("an AI move of a unit of class 51 or 54"));
        }
        let top = self.top_action_kind();
        if !m.force
            && top == Some(crate::move_to::MOVE_TO)
            && self.move_to.is_some_and(|mt| mt.goal == Some(m.point))
        {
            return Ok(());
        }
        if self.unmodelled_action.is_some() {
            return Err(Unsupported(
                "an AI move of a unit with an action (its slot 0x40)",
            ));
        }
        match top {
            Some(crate::artifact::ARTIFACT_TYPE) => {
                let walked = self
                    .artifact
                    .as_mut()
                    .is_some_and(|a| a.take_point(m.point));
                if walked {
                    self.mover
                        .as_mut()
                        .ok_or(Unsupported("a walker without movement state"))?
                        .stop()?;
                }
                return Ok(());
            }
            Some(crate::move_to::MOVE_TO) | None => {}
            Some(_) => {
                return Err(Unsupported(
                    "an AI move of a unit with an action (its slot 0x40)",
                ));
            }
        }
        if self.move_to.take().is_some() {
            self.mover
                .as_mut()
                .ok_or(Unsupported("a walker without movement state"))?
                .stop()?;
        }
        self.move_to = Some(MoveToAction {
            state: MoveTo::Start,
            goal: Some(m.point),
            target: -1,
            range: m.range,
            run: !m.walk,
        });
        Ok(())
    }

    /// The unit's slot 0x348 (`FUN_141086440`), forced, on its unit AI's move to an object: a
    /// `MoveTo` to the target within `range`, showing the master's running sprite (the walking one
    /// without it), replaces its action list (`FUN_140a98620`: an old `MoveTo` stops the unit) and
    /// starts ([`MoveToAction::to_target`]). A unit with another action is not modelled.
    fn move_to_object(
        &mut self,
        target: i32,
        range: f32,
        c: &mut Context<'_>,
    ) -> Result<(), Unsupported> {
        let top = self.top_action_kind();
        if self.unmodelled_action.is_some() || !matches!(top, Some(crate::move_to::MOVE_TO) | None)
        {
            return Err(Unsupported(
                "an AI move to an object of a unit with an action",
            ));
        }
        let mover = self.mover.as_mut().ok_or(Unsupported(
            "an AI move to an object of a unit that does not move",
        ))?;
        if self.move_to.take().is_some() {
            mover.stop()?;
        }
        let move_to =
            MoveToAction::to_target(c.spatial, self.position, mover, target, range, true)?;
        if move_to.arrived() {
            self.shows_standing_sprite(c.masters, c.graphics, c.streams.get(Stream::Sprite))?;
        }
        self.move_to = Some(move_to);
        Ok(())
    }

    /// A `MoveTo`'s `set_state` 1, not quiet, on the unit (`0x140aa29e0`): the master's standing
    /// sprite where the slice holds the unit's sprite (slot 0x68 with slot 0x798's, a draw of the
    /// Sprite stream), and its animation speed (`+0x7b8`) cleared where known.
    pub(crate) fn shows_standing_sprite(
        &mut self,
        masters: Option<&Masters>,
        graphics: &std::collections::BTreeMap<i16, Graphic>,
        sprite_rng: &mut Well512,
    ) -> Result<(), Unsupported> {
        if self.sprite.is_some() {
            let standing = masters
                .and_then(|m| m.get(self.life.master_id))
                .map(|m| m.standing_sprite)
                .and_then(|id| graphics.get(&id).copied())
                .ok_or(Unsupported("an arrived walker without its standing sprite"))?;
            crate::attack::set_sprite(&mut self.sprite, standing, sprite_rng);
        }
        if let Some(a) = self.anim_speed.as_mut() {
            *a = 0.0;
        }
        Ok(())
    }
}

/// The player's slot 0xf8 (`FUN_141616820`) with 1 and 1 on a unit's load: the resource and its
/// gathered total (166 to 168; gold's 49 and 169), which the caller adds after the update; the
/// statistics (slot 0xb8) and the local player's tracker (`FUN_14097daf0`) are not held. With 1
/// the resources would go to the allies too under `+0x1426` (`FUN_141626780`): a player with
/// allies is not modelled.
fn give_load(
    relations: &[Vec<u8>],
    player: usize,
    adds: &mut Vec<(i16, f32)>,
    kind: i16,
    amount: f32,
) -> Result<(), Unsupported> {
    let own = relations.get(player).map_or(&[][..], |r| &r[..]);
    let allied = (1..own.len()).any(|q| q != player && crate::tasks::allied(own, player, q, true));
    if allied {
        return Err(Unsupported(
            "a load given to a player with allies (+0x1426)",
        ));
    }
    adds.push((kind, amount));
    match kind {
        0..=2 => adds.push((166 + kind, amount)),
        3 => {
            adds.push((49, amount));
            adds.push((169, amount));
        }
        _ => {}
    }
    Ok(())
}

/// What the enter into a building ([`Unit::enter_building`]) reads and changes of the world
/// besides the unit and the building.
pub(crate) struct EnterWorld<'a> {
    /// Every object's place, the sight and the registry.
    pub spatial: &'a mut Spatial,
    /// The mesh the unit's shape leaves.
    pub pathing: &'a mut Pathing,
    /// The owner's masters.
    pub masters: Option<&'a Masters>,
    /// The graphics, for the standing sprite.
    pub graphics: &'a std::collections::BTreeMap<i16, Graphic>,
    /// The Sprite stream, which a new sprite may draw from.
    pub sprite_rng: &'a mut Well512,
    /// The drop-site table.
    pub drop_sites: &'a [DropSite],
    /// The owner's attributes and civ, as the drop-site test's actor.
    pub attributes: &'a [f32],
    /// The owner's civ.
    pub civ: u8,
    /// The owner.
    pub player: usize,
    /// The attributes the owner gains (a load), which the caller adds.
    pub attribute_adds: &'a mut Vec<(i16, f32)>,
    /// What the building's volleys gain (`+0xa00`, [`crate::garrison::share`]), which the caller
    /// adds.
    pub firepower: f32,
}

/// The population's resource, which a cost check skips (`FUN_141618db0` with its fifth argument
/// 1).
const POPULATION: i16 = 4;

/// The livestock class (`-0xe72` raw).
const LIVESTOCK: i16 = 58;

/// `FUN_140aab570`'s scan after a livestock's capture: Gaia's livestock in state 2 whose tile
/// the new owner sees (`FUN_141663500`), within 3 tiles of `at` (x outer, y inner); each is taken
/// in turn, with its own scan. Two on one tile would need the tile's object list order.
fn scan_livestock(
    c: &mut Context<'_>,
    others: &mut [Unit],
    at: [f32; 3],
    new: usize,
    taken: &mut Vec<i32>,
) -> Result<(), Unsupported> {
    let (tx, ty) = (at[0] as i32, at[1] as i32);
    let [w, h] = c.spatial.map;
    for x in (tx - 3).max(0)..=(tx + 3).min(w - 1) {
        for y in (ty - 3).max(0)..=(ty + 3).min(h - 1) {
            if !c.spatial.sees_tile(new, x, y) {
                continue;
            }
            let on: Vec<usize> = others
                .iter()
                .enumerate()
                .filter(|(_, u)| {
                    u.life.state == 2
                        && u.position[0] as i32 == x
                        && u.position[1] as i32 == y
                        && !taken.contains(&u.id)
                        && c.spatial
                            .get(u.id)
                            .is_some_and(|p| p.class == LIVESTOCK && p.player == 0)
                })
                .map(|(i, _)| i)
                .collect();
            match on[..] {
                [] => {}
                [k] => {
                    let mut unit = std::mem::take(&mut others[k]);
                    let done = (|| {
                        if let Some(a) = unit.artifact.as_mut() {
                            a.stop()?;
                        }
                        unit.change_owner(c, new)
                    })();
                    let (id, position) = (unit.id, unit.position);
                    others[k] = unit;
                    done?;
                    taken.push(id);
                    scan_livestock(c, others, position, new, taken)?;
                }
                _ => {
                    return Err(Unsupported(
                        "two livestock to take on one tile (its object list's order)",
                    ));
                }
            }
        }
    }
    Ok(())
}

/// What an object's update leaves of it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    Stay,
    /// It leaves its list for good (`RGE_Object_List::update` deletes it).
    Leave,
    /// It moves to its player's sleeping list.
    Sleep,
    /// A player took it (and these other objects of its player) over: they move to the end of
    /// that player's list.
    Captured(usize, Vec<i32>),
}

/// The unit a drop-site search goes from ([`drop_site_search`]).
struct Seeker<'a, 'b> {
    me: Me,
    mover: &'a Mover,
    plan: &'a mut Planning<'b>,
    spatial: &'a Spatial,
    zones: &'a crate::zones::ZoneMap,
    /// Its master's class.
    class: i16,
}

/// A search for drop sites (`FUN_1411671e0` as `set_state(8)` calls it: in state 2, not at work
/// on task `0x7d` or `0x7c` (an object of type 40 up whose top action is one), not inside
/// another): of the player's live objects (`units`, in list order) of a master in `sites`, the
/// nearest to `at`. With the `seeker`, only objects in its zone count (its restriction's zone of
/// its tile against each object's tile) and the nearest is by path: the tile planner's path
/// length (radii the seeker's and the object's, [`Mover::plan_length`]) when it reaches the goal
/// or its range, then within 10 tiles the shorter of it and the seeker's short path (slot 0x3e8
/// within 0.1, [`Mover::short_reach`]); for any other result the game keeps the distance it last
/// measured (of an earlier object, else the largest float), as the slice does. Without one, by
/// the squared distance of the centres. The seeker's class 21 (its gap test) and a villager
/// tending a pasture (its pasture's place) are not modelled. The first of equals wins.
fn drop_site_search(
    units: &[Unit],
    masters: &Masters,
    sites: &[i16],
    at: [f32; 3],
    mut seeker: Option<Seeker<'_, '_>>,
) -> Result<Option<i32>, Unsupported> {
    const AT_WORK: [i16; 2] = [0x7d, 0x7c];
    let [x, y, _] = at;
    if seeker.as_ref().is_some_and(|s| s.class == 21) {
        return Err(Unsupported("a drop-site search of class 21 (its gap test)"));
    }
    let own_zone = seeker.as_ref().map(|s| s.zones.zone(x as i32, y as i32));
    let mut best: Option<i32> = None;
    let mut best_d = NO_DROP_DISTANCE;
    let mut last = NO_DROP_DISTANCE;
    for o in units {
        if !sites.contains(&o.life.master_id) || o.life.state != 2 {
            continue;
        }
        let master = masters.get(o.life.master_id).ok_or(Unsupported(
            "a drop site of a master the slice does not have",
        ))?;
        if let (Some(s), Some(zone)) = (seeker.as_ref(), own_zone)
            && s.zones.zone(o.position[0] as i32, o.position[1] as i32) != zone
        {
            continue;
        }
        let at_work =
            40 <= master.kind && o.top_action_kind().is_some_and(|k| AT_WORK.contains(&k));
        if at_work || o.life.inside.is_some() {
            continue;
        }
        let d = if let Some(s) = seeker.as_mut() {
            let radii = s.mover.stats.radius[0] + master.radius[0];
            let to = [o.position[0], o.position[1]];
            match s.mover.plan_length(&s.me, s.spatial, s.plan, to, radii)? {
                Some((code, length)) if code < 2 => {
                    last = length;
                    if length <= SHORT_DROP_REACH
                        && let Some(short) =
                            s.mover.short_reach(&s.me, s.spatial, o.id, DROP_REACH)?
                        && short < length
                    {
                        last = short;
                    }
                    last
                }
                Some((code, _)) if 3 <= code => last,
                _ => last,
            }
        } else {
            let (dx, dy) = (x - o.position[0], y - o.position[1]);
            dy * dy + dx * dx
        };
        if d < best_d {
            best = Some(o.id);
            best_d = d;
        }
    }
    Ok(best)
}

/// A drop site within this path length is measured by the short path too (`FUN_1411671e0`,
/// `DAT_142eb0f4c`).
const SHORT_DROP_REACH: f32 = crate::image::turn::SHORT_DROP_REACH;
/// The range the short path measures a drop site within (`DAT_142eb5690`).
const DROP_REACH: f32 = crate::image::turn::DROP_REACH;
/// The distance a drop-site search starts from (`DAT_14330ebf4`, the largest float).
const NO_DROP_DISTANCE: f32 = crate::image::turn::NO_DROP_DISTANCE;

/// `FUN_1411671e0` with a point and no unit: of the player's live objects (`others`, in list
/// order) of a master in `sites` and in state 2, the one nearest `at` (its centre's squared
/// distance, the first of equals), with that distance.
fn nearest_drop_site(others: &[Unit], sites: &[i16], at: [f32; 3]) -> Option<(i32, f32)> {
    let mut best: Option<(i32, f32)> = None;
    for o in others {
        if !sites.contains(&o.life.master_id) || o.life.state != 2 {
            continue;
        }
        let [x, y, _] = at;
        let (dx, dy) = (x - o.position[0], y - o.position[1]);
        let d = dy * dy + dx * dx;
        if best.is_none_or(|(_, b)| d < b) {
            best = Some((o.id, d));
        }
    }
    best
}

/// What a ranged unit's fire reads of its target ([`Unit::fire`]): its id, place, master's radii
/// and class, and player.
struct Aimed {
    target: i32,
    position: [f32; 3],
    radius: [f32; 3],
    class: i16,
    player: usize,
}

/// `FUN_14120ea30` of an attacker of `master` with `firepower` (`+0xa00`): a building can attack
/// when of class 51 or 54, with a missile, or with a firepower above 1; any other attacker can.
fn armed(master: &Master, firepower: f32) -> bool {
    master.kind != crate::order::BUILDING
        || matches!(master.class, 51 | 54)
        || master.missile != -1
        || ARMED_FIREPOWER < firepower
}

/// Whether two lists hold the same objects, in any order.
fn same_members(a: &[i32], b: &[i32]) -> bool {
    let (mut a, mut b) = (a.to_vec(), b.to_vec());
    a.sort_unstable();
    b.sort_unstable();
    a == b
}

/// What a missile was fired as, which its hits read while its own part is out of the unit: a
/// charge's projectile (`+0x99c`) and what it was fired from (`+0x998`).
#[derive(Clone, Copy, Debug)]
struct Fired {
    charge: bool,
    by: i32,
}

/// A missile [`Unit::launch`] makes: its master, where it starts, the point its action aims at,
/// its reach (`+0x990`), whether its action has the object as its shooter (else none), what its
/// aim reads of the firing object (or the missile) and the attack count the aim sees.
struct Shot {
    made: i16,
    at: [f32; 3],
    point: [f32; 3],
    reach: f32,
    shooter: bool,
    aimer: crate::missile::Aimer,
    attack_count: i32,
}

/// Slot 0x180 (`Turns::release`) on an object of the unit's own live list, from inside the unit's
/// update: its worker count falls, and a static object no one works on any more that is alive is
/// flagged to sleep.
/// `ClosestObject`'s "Sheep is moving" (`FUN_140c488b0`): the object's top action is the Artifact
/// (slot 0x530 107) with a point (slot 0x288, the action's `+0x38`, not −1).
fn herded(others: &[Unit], foreign: Foreign<'_>, id: i32) -> bool {
    let foreign = || foreign.find(id).map(|(_, u)| u);
    others
        .iter()
        .find(|u| u.id == id)
        .or_else(foreign)
        .is_some_and(|u| {
            u.top_action_kind() == Some(crate::artifact::ARTIFACT_TYPE)
                && u.artifact
                    .as_ref()
                    .is_some_and(|a| a.goal.is_some_and(|g| g[0] != -1.0))
        })
}

fn release_own(
    others: &mut [Unit],
    masters: Option<&Masters>,
    id: i32,
    by: Releaser,
) -> Result<(), Unsupported> {
    let u = others.iter_mut().find(|u| u.id == id).ok_or(Unsupported(
        "a release of an object not in the unit's live list",
    ))?;
    let kind = masters
        .and_then(|m| m.get(u.life.master_id))
        .ok_or(Unsupported(
            "a released object of a master the slice does not have",
        ))?
        .kind;
    // one of the releaser's own player: allied
    released(u, kind, by, true)
}

/// The object whose action lets its target go (the target's slot 0x180 takes it): its id, its
/// master's class and its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Releaser {
    pub(crate) id: i32,
    pub(crate) class: i16,
    pub(crate) player: usize,
}

/// Slot 0x180 on object `u` of master type `kind`, let go by `by` (`allied` to its owner):
/// the static object's (`FUN_141197d00`) counts a worker less and a static one without workers
/// left, alive (state 2 with hit points, or 6), goes to sleep. A combat object's
/// (`FUN_141086e60`) counts the worker less (`FUN_141071ea0`) and tells its unit AI that the
/// releaser no longer acts on it (notify 699, the releaser's master class as its first
/// parameter; none from an object of a negative id). A building's (`FUN_1411d6020`) first drops
/// an unallied releaser from a list of its own while it stands in state 0 or 1, which is not
/// modelled.
pub(crate) fn released(
    u: &mut Unit,
    kind: u8,
    by: Releaser,
    allied: bool,
) -> Result<(), Unsupported> {
    if 0 < u.workers {
        u.workers -= 1;
    }
    let alive = (u.life.state == 2 && 0.0 <= u.life.hp) || u.life.state == 6;
    if kind < 30 && u.workers == 0 && alive {
        u.life.goto_sleep = true;
    }
    if !u.combat {
        return Ok(());
    }
    if kind == 80 && u.life.state < 2 && !allied {
        return Err(Unsupported(
            "an enemy's release of a building in state 0 or 1 (FUN_1411d6020)",
        ));
    }
    if 0 <= by.id
        && let Some(ai) = u.ai.as_mut()
    {
        let n = Notify {
            caller: by.id,
            recipient: u.id,
            kind: crate::unit_ai::NO_LONGER_ON_ME,
            p: [i32::from(by.class), 0, 0],
        };
        ai.post(n, u.life.state)?;
    }
    Ok(())
}

/// A player's settings for the groups an order's splitter forms ([`Player::formations`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Formations {
    /// The splitter groups units at all (`+0x6c0` not 0).
    pub auto: bool,
    /// How near two units join (`+0x6c4`).
    pub influence: f32,
    /// Units whose speeds differ by more than [`crate::order`]'s gap do not join (`+0x6c8` 1).
    pub by_speed: bool,
}

/// One player's objects in live-list order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Player {
    /// Its live objects, in their list's order.
    pub objects: Vec<Unit>,
    /// `player+0x10 == 3`.
    pub computer: bool,
    /// Food, wood, stone and gold (the player's attributes 0 to 3).
    pub resources: [f32; 4],
    /// Attribute 195, the construction rate modifier, which builders' work reads.
    pub construction_rate: f32,
    /// Every attribute as saved (the first four are `resources`, which the slice keeps); the
    /// task finder reads attribute 95, the work command a task's enabling resource.
    pub attributes: Vec<f32>,
    /// The civ (`player+0x738`), which drop sites read.
    pub civ: u8,
    /// `relation[i]` (`player+0x260`): how the player stands to player i.
    pub relations: Vec<u8>,
    /// The sleeping objects, in their list's order: they are not updated; a unit's work on one
    /// wakes it.
    pub sleeping: Vec<Unit>,
    /// `autoFarmQueue` (`+0x1424`) and `autoFishTrapQueue` (`+0x1425`), which GAME 0x10 and 0x13
    /// flip; nothing the slice models reads them yet.
    pub auto_farm_queue: bool,
    /// `autoFishTrapQueue` (`+0x1425`).
    pub auto_fishtrap_queue: bool,
    /// The order splitter's settings (`+0x6c0` `mAutoFormations`, `+0x6c4`
    /// `mFormationInfluenceDistance`, `+0x6c8` `mBreakAutoFormationsBySpeed`).
    pub formations: Formations,
    /// `mPendingDebit` (`+0x6cc`): what the player's BUILD and RESEARCH commands listed as their
    /// costs, food, wood, stone and gold, taken off (`FUN_141630d10`); nothing modelled reads it.
    pub pending_debit: [f32; 4],
    /// The player's state of each technology (`player+0x760`'s, saved).
    pub techs: Vec<crate::tech::TechState>,
    /// The first update this slice could not model, with the tick (1-based) and object index.
    pub unsupported: Option<(u32, usize, Unsupported)>,
    /// Every object the slice stopped updating, with the tick and why.
    pub freezes: Vec<(u32, i32, Unsupported)>,
    /// A command the slice could not model stopped the whole player: its objects freeze.
    pub stopped: bool,
    /// `game_status` (`+0x89`, saved): playing, won or lost ([`crate::game_status`]).
    pub game_status: u8,
    /// `+0x8b` (saved with `+0x8c` as its bit 1): the player resigned (slot 3, RESIGN).
    pub resigned: bool,
    /// `+0x8c`: its resignation was a drop from the game.
    pub dropped: bool,
    /// `allied_victory` (`+0x2d0`, saved): its allies' victory counts as its own.
    pub allied_victory: bool,
    /// Its victory conditions' state (`player+0x68`'s `+0x20`, the save's `victory`): 2 when
    /// they are met (`FUN_1414f35d0`).
    pub victory: u8,
    /// `fealty` (`+0x12f8`, saved): the player a defeated player's resources and units would go
    /// to.
    pub fealty: i16,
    /// `+0x71c`, the offensive posture (0 or 1, GAME 0x14): not saved, a load sets 0; new soldiers
    /// read it (their unit AI's constructor, not modelled).
    pub posture: i32,
    /// `allied_LOS_Enable` (`+0x2cc`, saved): the player shares its mutual allies' sight
    /// ([`crate::vision`]).
    pub allied_vision: bool,
    /// The objects whose ghosts the player's list follows (`+0x78`, [`crate::ghost`]): not
    /// saved; the load builds it ([`Turns::build_watched`]).
    pub watched: Vec<crate::ghost::Watched>,
    /// The player's ghosts (its doppelganger list `+0x40`), in the list's order.
    pub ghosts: Vec<crate::ghost::Ghost>,
}

impl Turns {
    /// What a hit of an attacker of player `attacker` reads of the object `id` ([`Victim`]): a
    /// live object the slice holds, with its relations to the attacker's player and whether its
    /// place is a guess.
    pub(crate) fn victim(&self, id: i32, attacker: usize) -> Option<Victim> {
        let (p, u) =
            self.players.iter().enumerate().find_map(|(p, player)| {
                player.objects.iter().find(|u| u.id == id).map(|u| (p, u))
            })?;
        let master = self.masters.get(p)?.get(u.life.master_id)?;
        let attributes = &self.players[p].attributes;
        let terrain = self.spatial.terrain.as_ref();
        let mut v = victim_of(u, master, attributes, p, terrain, self.mode_680);
        let world = World {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        v.allied = [
            world.allied(attacker, p, true),
            world.allied(p, attacker, true),
        ];
        v.uncertain = self.spatial.get(id).is_none_or(|o| o.unknown.is_some());
        Some(v)
    }

    /// The object `id` as the unit AI of a unit of player `viewer` finds it in the world's object
    /// table (`FUN_1415417a0`, [`crate::unit_ai::Seen`]), `None` when no list holds it. An object
    /// the slice holds as a place alone cannot be read. Without the players' sight its tile
    /// counts as unseen.
    fn seen(&self, id: i32, viewer: usize) -> Result<Option<crate::unit_ai::Seen>, Unsupported> {
        for (p, player) in self.players.iter().enumerate() {
            if let Some(u) = player
                .objects
                .iter()
                .chain(&player.sleeping)
                .find(|u| u.id == id)
            {
                let master = self
                    .masters
                    .get(p)
                    .and_then(|m| m.get(u.life.master_id))
                    .ok_or(Unsupported(
                        "a unit AI's target of a master the slice does not have",
                    ))?;
                let [x, y, _] = u.position;
                return Ok(Some(crate::unit_ai::Seen {
                    state: u.life.state,
                    kind: master.kind,
                    owner: p,
                    class: master.class,
                    position: u.position,
                    radius: master.radius,
                    visible: self.spatial.visible(viewer, x as i32, y as i32),
                }));
            }
        }
        match self.spatial.get(id) {
            Some(_) => Err(Unsupported("a unit AI's target the slice cannot read")),
            None => Ok(None),
        }
    }

    /// The objects a blast of `unit` (player `p`) may reach while it hits `target`: for a master
    /// with an area effect, the objects but the two on the tiles of the blast's rectangle about
    /// the unit, a tile wider each way (the unit may step in its update). An object the slice
    /// cannot read stands in with its place alone, as a guess.
    fn blasted(&self, unit: &Unit, p: usize, target: i32) -> Vec<Victim> {
        let Some(r) = self
            .masters
            .get(p)
            .and_then(|m| m.get(unit.life.master_id))
            .map(|m| m.area_effect)
            .filter(|&r| 0.0 < r)
        else {
            return Vec::new();
        };
        let [x, y, _] = unit.position;
        let reach = r + 6.0 + 2.0;
        self.spatial
            .objects_within(x - reach, y - reach, x + reach, y + reach)
            .filter(|&(id, _)| id != unit.id && id != target)
            .map(|(id, o)| {
                self.victim(id, p).unwrap_or_else(|| Victim {
                    id,
                    position: o.position,
                    uncertain: true,
                    ..Victim::default()
                })
            })
            .collect()
    }

    /// Slot 0x5f0 (`FUN_141158b90`): the unit's walked level, if it has one, pauses for `pause`
    /// seconds; the world's view of it follows.
    fn pause(&mut self, id: i32, pause: f32) {
        let Some(u) = self
            .players
            .iter_mut()
            .flat_map(|p| p.objects.iter_mut())
            .find(|u| u.id == id)
        else {
            return;
        };
        if let Some(level) = u.mover.as_mut().and_then(|m| m.paths.first_mut()) {
            level.pause_time = pause;
            let gait = u.gait();
            self.spatial.set_gait(id, gait);
        }
    }

    /// An attack's hit on its target, written back.
    fn set_victim(&mut self, v: &Victim) -> Result<(), Unsupported> {
        let Some((q, j)) = self.players.iter().enumerate().find_map(|(q, player)| {
            player
                .objects
                .iter()
                .position(|u| u.id == v.id)
                .map(|j| (q, j))
        }) else {
            return Ok(());
        };
        let u = &mut self.players[q].objects[j];
        u.life.hp = v.hp;
        u.life.amount = v.amount;
        u.life.goto_sleep = v.goto_sleep;
        if let Some(a) = v.armored.as_ref() {
            u.hp_powerups = a.hp_powerups;
        }
        // a shield spends its charge
        u.charge = v.source.charge;
        // the damage notify (500) to its unit AI
        if let (Some((attacker, p)), Some(ai)) = (v.damaged, u.ai.as_mut()) {
            let n = Notify {
                caller: attacker,
                recipient: v.id,
                kind: crate::unit_ai::DAMAGED,
                p,
            };
            ai.post(n, u.life.state)?;
        }
        v.killed.map_or(Ok(()), |kill| self.killed(q, j, kill))
    }

    /// A combat object's death by a hit (`FUN_141082e80`, `FUN_1412108e0`): its owner's computer
    /// AI is told (slot 0x2f0, not modelled); the killed handling (slot 0x8b0, `FUN_141205880` ->
    /// `FUN_1412005b0`): unless its class is one of the non-military ones the attacker's player
    /// counts a kill (attribute 20) and its owner a loss (154); the attacker's player counts it
    /// against the owner (300 + the owner) and the owner against the attacker (325 + the
    /// attacker's player); both count its paid cost (170 and 152, and 400 + the owner). A killed
    /// villager's statistics (attributes 226 to 229 by the attacker's class and player type) are
    /// not modelled. Gaia takes a player's livestock (slot 0x70): its place and sight, its Artifact's
    /// wait, its unit AI's notifies (cleared, then 0x20b), its storage (the old owner loses flags 2,
    /// 16 and 32, Gaia gains 1, 2 and 32) and its list (the old owner's last object takes its
    /// place). Not held: the killer (`+0x690`, `+0x694`), `+0x954`, the statistics of the local
    /// player and the particles (`FUN_141158bb0`).
    fn killed(&mut self, q: usize, j: usize, kill: crate::damage::Kill) -> Result<(), Unsupported> {
        const PEACEFUL: [i16; 26] = [
            1, 3, 5, 7, 8, 9, 10, 11, 14, 15, 16, 27, 29, 30, 31, 32, 33, 34, 38, 39, 40, 48, 52,
            58, 60, 63,
        ];
        if self.players[q].computer {
            return Err(Unsupported(
                "a computer player's killed object (its AI's slot 0x2f0)",
            ));
        }
        let u = &self.players[q].objects[j];
        let master = self
            .masters
            .get(q)
            .and_then(|m| m.get(u.life.master_id))
            .ok_or(Unsupported(
                "a killed object of a master the slice does not have",
            ))?
            .clone();
        if master.class == 4 {
            return Err(Unsupported(
                "a killed villager's statistics (attributes 226 to 229)",
            ));
        }
        let ap = kill.attacker_player;
        if !PEACEFUL.contains(&master.class) {
            self.players[ap].add_attribute(20, 1.0)?;
            self.players[q].add_attribute(154, 1.0)?;
        }
        self.players[ap].add_attribute(q as i16 + 300, 1.0)?;
        self.players[q].add_attribute(ap as i16 + 325, 1.0)?;
        let mut cost = 0.0f32;
        for &(resource, amount, paid) in &master.cost {
            if resource >= 0 && paid {
                cost += f32::from(amount);
            }
        }
        self.players[ap].add_attribute(170, cost)?;
        self.players[q].add_attribute(152, cost)?;
        self.players[ap].add_attribute(q as i16 + 400, cost)?;
        if kill.to_gaia {
            self.give_gaia(q, j)?;
        }
        Ok(())
    }

    /// Slot 0x70 (`FUN_14120f800`) to Gaia of the object at `j` of player `q`, from outside its
    /// update ([`Unit::change_owner`]'s parts and the list and storage moves of a capture).
    fn give_gaia(&mut self, q: usize, j: usize) -> Result<(), Unsupported> {
        let u = &self.players[q].objects[j];
        let (id, master_id) = (u.id, u.life.master_id);
        if u.life.inside.is_some() {
            return Err(Unsupported(
                "an ownership change of an object inside another",
            ));
        }
        let gaia_master =
            self.masters
                .first()
                .and_then(|m| m.get(master_id))
                .ok_or(Unsupported(
                    "an ownership change to a player without the master",
                ))?;
        let los = gaia_master.line_of_sight;
        let storage = self
            .masters
            .get(q)
            .and_then(|m| m.get(master_id))
            .map(|m| m.storage.clone())
            .unwrap_or_default();
        self.spatial.change_owner(id, 0, los)?;
        let world_time = self.world_time;
        let mut unit = self.players[q].objects.swap_remove(j);
        if let Some(a) = unit.artifact.as_mut() {
            a.gaia = true;
        }
        if let Some(ai) = unit.ai.as_mut() {
            ai.clear_notifies(world_time)?;
        }
        unit.post_own(OWNER_CHANGED, [0, 0, 0])?;
        for &(kind, amount, flag) in &storage {
            if kind < 0 {
                continue;
            }
            if matches!(flag, 2 | 16 | 32) {
                self.players[q].add_attribute(kind, -amount)?;
            }
            if matches!(flag, 1 | 2 | 32) {
                self.players[0].add_attribute(kind, amount)?;
            }
        }
        self.players[0].objects.push(unit);
        Ok(())
    }
}

/// The game's state after a tick, for the oracle's forced replay (dev only: the oracle reads it
/// from the tick oracle's saves, genie-core never reads the game): the frame seed and every
/// player's live units and resources as the game holds them.
#[derive(Clone, Debug, Default)]
pub struct GameState {
    /// `world+0x7d0`, the frame seed.
    pub frame_seed: u32,
    /// Every player's live objects.
    pub players: Vec<Vec<Unit>>,
    /// Every player's food, wood, stone and gold.
    pub resources: Vec<[f32; 4]>,
    /// Every player's attributes as saved (the first four are the resources), which the oracle
    /// compares; forcing leaves them alone.
    pub attributes: Vec<Vec<f32>>,
    /// The players' pending debits and technology states.
    pub pending_debit: Vec<[f32; 4]>,
    /// Every player's technology states.
    pub techs: Vec<Vec<crate::tech::TechState>>,
    /// Every player's game status and resignation (`+0x89`, `+0x8b`).
    pub statuses: Vec<(u8, bool)>,
    /// The world's game state (`+0x88`).
    pub game_state: u32,
    /// Objects the slice modelled but got wrong this tick: they take the game's state too.
    pub resync: Vec<i32>,
    /// The players' visible maps and the unified map, when the slice's differ.
    pub sight: Option<(Vec<crate::visibility::VisibleMap>, Vec<u64>)>,
    /// `next_reusable_object_id`: the fog ghosts that take reusable ids are not modelled.
    pub next_reusable_id: i32,
    /// The map tiles' object lists in the game's order (`x`, `y`, ids; each tile of two objects
    /// or more), when the oracle dumped them: the order is the game's history, no save holds it.
    pub tile_lists: Option<Vec<(i32, i32, Vec<i32>)>>,
    /// The pool of recycled missiles ([`Recycling::missiles`]), when the oracle dumped it: no
    /// save holds it.
    pub missile_pool: Option<Vec<(i32, bool)>>,
    /// The world's pool of recycled ghosts, when the oracle dumped it.
    pub ghost_pool: Option<Vec<i32>>,
}

impl Turns {
    /// The objects' inside lists in the game's order (`+0x2f8`, the tick oracle's dump): each
    /// container listed takes the order when it holds the same objects. Returns how many did.
    pub fn force_garrisons(&mut self, lists: &[(i32, Vec<i32>)]) -> usize {
        let mut n = 0;
        for (id, order) in lists {
            let Some(c) = self
                .players
                .iter_mut()
                .flat_map(|p| p.objects.iter_mut().chain(p.sleeping.iter_mut()))
                .find(|u| u.id == *id)
            else {
                continue;
            };
            if same_members(&c.garrison, order) {
                c.garrison.clone_from(order);
                n += 1;
            }
        }
        n
    }

    /// The forced replay's step after a tick: the frame seed is the game's, and so is every
    /// object the slice did not model (it froze, or its player stopped) or got wrong (`resync`),
    /// and the players' sight where it differs: its unit is the game's
    /// (with what the save does not hold, a hunted animal's origin and a boar's flag, kept),
    /// placed where the game has it, and it runs again from the next tick; a frozen object the
    /// game no longer holds goes. A stopped player takes the game's resources and runs again.
    /// The world's next reusable id is the game's (the fog ghosts that take them are not
    /// modelled). A stopped player's pending debit and technologies are the game's too, and so
    /// is the tiles' objects' order where the game's dump has it, and a forced object's inside
    /// list keeps the slice's order while it holds the same objects. Returns
    /// what could not be forced (an object that changed hands or masters, or
    /// that the game made and the slice did not). Not part of a replay: a dev hook to see past a
    /// freeze.
    pub fn force(&mut self, game: &GameState) -> Vec<(usize, i32, Unsupported)> {
        let mut notes = Vec::new();
        self.frame_seed = game.frame_seed;
        self.game_state = game.game_state;
        self.recycling.next_id = game.next_reusable_id;
        if let Some(pool) = game.ghost_pool.as_ref() {
            self.recycling.ghosts.clone_from(pool);
        }
        if let Some(pool) = game.missile_pool.as_ref() {
            self.recycling.missiles.clone_from(pool);
        }
        if let (Some((maps, unified)), Some(sight)) =
            (game.sight.as_ref(), self.spatial.sight.as_mut())
        {
            sight.maps.clone_from(maps);
            sight.unified.clone_from(unified);
        }
        // the objects the slice holds, sorted: forcing only takes away objects the game no
        // longer holds
        let mut known: Vec<i32> = self
            .players
            .iter()
            .flat_map(|q| q.objects.iter().chain(&q.sleeping).map(|u| u.id))
            .collect();
        known.sort_unstable();
        for p in 0..self.players.len() {
            let Some(theirs) = game.players.get(p) else {
                continue;
            };
            let stopped = self.players[p].stopped;
            if stopped {
                if let Some(r) = game.resources.get(p) {
                    self.players[p].resources = *r;
                }
                if let Some(d) = game.pending_debit.get(p) {
                    self.players[p].pending_debit = *d;
                }
                if let Some(t) = game.techs.get(p) {
                    self.players[p].techs.clone_from(t);
                }
                self.players[p].stopped = false;
            }
            if let Some(&(status, resigned)) = game.statuses.get(p) {
                self.players[p].game_status = status;
                self.players[p].resigned = resigned;
            }
            // each update's first unmodelled part is its own: a freeze's would hide a later stop
            self.players[p].unsupported = None;
            let mut i = 0;
            while i < self.players[p].objects.len() {
                let unit = &self.players[p].objects[i];
                let resync = game.resync.contains(&unit.id);
                let (id, frozen) = (unit.id, stopped || unit.stopped || resync);
                // a dead object never freezes, but one that differs takes the game's state too
                // (a felled tree's wood that a frozen gatherer did not take)
                if !frozen || (!unit.life.is_alive() && !resync) {
                    i += 1;
                    continue;
                }
                let Some(g) = theirs.iter().find(|g| g.id == id) else {
                    let elsewhere = game.players.iter().any(|q| q.iter().any(|g| g.id == id));
                    if elsewhere {
                        notes.push((p, id, Unsupported("a forced object changed hands")));
                        i += 1;
                        continue;
                    }
                    let gone = self.players[p].objects.swap_remove(i);
                    let _ = self.spatial.leave(gone.id);
                    self.pathing.removed(gone.id);
                    continue;
                };
                if g.life.master_id != unit.life.master_id {
                    notes.push((p, id, Unsupported("a forced object changed masters")));
                    i += 1;
                    continue;
                }
                let mut forced = g.clone();
                // the objects inside it keep the slice's order (the game's history, which no save
                // holds) while they are the same
                if same_members(&unit.garrison, &forced.garrison) {
                    forced.garrison.clone_from(&unit.garrison);
                }
                if let (Some(old), Some(new)) = (unit.ai.as_ref(), forced.ai.as_mut()) {
                    new.hunted = old.hunted;
                    if let (Some(o), Some(n)) = (old.home, new.home.as_mut()) {
                        n.go_home = o.go_home;
                    }
                }
                let (before, state) = (unit.life.state, forced.life.state);
                if forced.position != unit.position {
                    self.spatial.set_position(id, forced.position);
                }
                if state != before {
                    if before <= 2 && 3 <= state {
                        self.spatial.sight_state(id, 3);
                    }
                    if state != 3 {
                        self.spatial.sight_state(id, state);
                    }
                }
                self.spatial.unfreeze(id);
                self.spatial.set_gait(id, forced.gait());
                if let Some(placed) = self.spatial.get(id).copied() {
                    let moving = forced.mover.as_ref().is_some_and(Mover::is_moving);
                    self.pathing.moved(id, &placed, moving);
                }
                forced.stopped = false;
                self.players[p].objects[i] = forced;
                i += 1;
            }
            for g in theirs {
                if known.binary_search(&g.id).is_err() {
                    notes.push((
                        p,
                        g.id,
                        Unsupported("an object the game made and the slice did not"),
                    ));
                }
            }
        }
        if let Some(lists) = game.tile_lists.as_ref() {
            // the tiles whose objects differ keep the slice's order: their objects' places
            // differ, and that is reported
            let _ = self.spatial.force_tile_lists(lists);
        }
        self.pathing.tiles_changed(self.spatial.take_changes());
        notes
    }
}

impl crate::tech::Attributes for Player {
    /// An attribute's value (the resources' from [`Player::resources`]).
    fn get(&self, i: i16) -> Result<f32, Unsupported> {
        match usize::try_from(i) {
            Ok(r @ 0..=3) => Ok(self.resources[r]),
            Ok(k) => self
                .attributes
                .get(k)
                .copied()
                .ok_or(Unsupported("an attribute the player does not have")),
            Err(_) => Err(Unsupported("a negative attribute")),
        }
    }

    /// An attribute set to `v`, as a technology's payment writes it (no side effects).
    fn set(&mut self, i: i16, v: f32) -> Result<(), Unsupported> {
        match usize::try_from(i) {
            Ok(r @ 0..=3) => self.resources[r] = v,
            Ok(k) => {
                *self
                    .attributes
                    .get_mut(k)
                    .ok_or(Unsupported("an attribute the player does not have"))? = v;
            }
            Err(_) => return Err(Unsupported("a negative attribute")),
        }
        Ok(())
    }
}

impl Player {
    /// A command the slice does not model for this player: the player stops.
    pub(crate) fn stop(&mut self, tick: u32, why: Unsupported) {
        let _ = self.unsupported.get_or_insert((tick, 0, why));
        self.stopped = true;
    }

    /// `FUN_141616820` for the attributes an object's storage holds: the attribute's value grows
    /// by `amount` (`FUN_14165d040`; the peak of attribute 11, `+0x6fc`, and the resource events of
    /// `FUN_140c898a0` are not held). Food, wood, stone and gold are the slice's `resources`; the
    /// attributes whose change has side effects (50, 183, 196) are not modelled.
    pub fn add_attribute(&mut self, kind: i16, amount: f32) -> Result<(), Unsupported> {
        let i = usize::try_from(kind).map_err(|_| Unsupported("a negative attribute"))?;
        match i {
            50 | 183 | 196 => Err(Unsupported("an attribute whose change has side effects")),
            0..=3 => {
                self.resources[i] += amount;
                Ok(())
            }
            _ => {
                let v = self
                    .attributes
                    .get_mut(i)
                    .ok_or(Unsupported("an attribute the player does not have"))?;
                *v += amount;
                Ok(())
            }
        }
    }
}

/// World state the slice carries from tick to tick. All of it is in the header save.
#[derive(Clone, Debug, PartialEq)]
pub struct Turns {
    /// The world's time in milliseconds (`world_time`).
    pub world_time: u32,
    /// `world+0x7d0`.
    pub frame_seed: u32,
    /// `world+0x188`, `player_turn` in the save.
    pub player_turn: usize,
    /// `world+0x18c[p]`, the time each player accumulated since its last turn.
    pub player_time: Vec<u32>,
    /// Every player, Gaia first.
    pub players: Vec<Player>,
    /// The updates run so far.
    pub ticks: u32,
    /// `mNextObjectId`, the id the next new object gets.
    pub next_object_id: i32,
    /// Each player's masters, which BUILD reads; a player without them stops at its own BUILD.
    pub masters: Vec<Masters>,
    /// Every object's place on the map.
    pub spatial: Spatial,
    /// The planners and the mesh.
    pub pathing: Pathing,
    /// What the sprite clock reads of each graphic (the `.dat`'s).
    pub graphics: std::collections::BTreeMap<i16, Graphic>,
    /// The game's drop-site table (`dropsites.json`).
    pub drop_sites: Vec<DropSite>,
    /// The game settings' test `FUN_141302150` (their `+0x1070` flag, then a table entry's
    /// `+0x9c`): a hunter drops another resource it carries. The tick oracle read the flag as 0 in
    /// MarvinBlue/426048731; where the save or the lobby holds it is not known yet.
    pub carry_rule: bool,
    /// The game's setting that `FUN_140ec0cd0` reads at its `+0x680` (a value and a kind); a boar
    /// does not wander when it is 1. The tick oracle reads 0 in MarvinBlue/426048731; where the
    /// save or the lobby holds it is not known yet (the save's `mGameMode` sits elsewhere).
    pub mode_680: i32,
    /// The game mode (the save's `mGameMode`, options `+0x34`).
    pub game_mode: i32,
    /// The world's game state (`+0x88`, the save's `tempGameState`): 1 once the game is over.
    pub game_state: u32,
    /// The scenario's `victory_conquest` (`world+0x7e0`'s `+0x2c0`, `FUN_140ca4d20`): the game
    /// ends when one side is left.
    pub victory_conquest: bool,
    /// The flag a new combat object's setup reads at `FUN_140982ff0()`'s `+0x921`: the unit's
    /// cost also counts in attribute 246. The tick oracle reads 1 in MarvinBlue/426048731 (the
    /// attribute grows by a villager's cost at its spawn); where the save holds it is not known.
    pub unit_cost_246: bool,
    /// The map's zone maps (`map+0x5e28`), made when first asked for.
    pub zones: crate::zones::Zones,
    /// The world's ids and pools for the objects of recyclable masters ([`Recycling`]).
    pub recycling: Recycling,
    pub(crate) streams: Streams,
    /// The `.dat`'s technologies, by id.
    pub techs: Vec<crate::tech::TechDef>,
}

/// The objects of recyclable masters (`+0x680`, the `.dat`'s `recyclable`): a new one takes the
/// world's next reusable id (`FUN_1415301b0`), counting down from −1; one that leaves its list
/// goes to the world's pool for its type (`RGE_Static_Object::recycle_out_of_game`,
/// `FUN_141546960`), from which the next of that type comes back, the last in first
/// (`FUN_141546870`), keeping its id (`recycle_in_to_game` gives none). The pools are not saved:
/// a game loaded from a save starts them empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recycling {
    /// `next_reusable_object_id`.
    pub next_id: i32,
    /// The pool of type 60 (`world+0x1d8`): each missile's id and its flying flag (`+0x994`,
    /// which the object keeps), the last in last.
    pub missiles: Vec<(i32, bool)>,
    /// The pool of type 25 (`world+0x1e0`): the ghosts' ids, the last in last.
    pub ghosts: Vec<i32>,
}

/// The players' relations and the drop sites, as the task finder reads them.
pub(crate) struct World<'a> {
    pub(crate) players: &'a [Player],
    pub(crate) drop_sites: &'a [DropSite],
}

impl Diplomacy for World<'_> {
    /// [`crate::tasks::allied`] with `from`'s diplomacy.
    fn allied(&self, from: usize, to: usize, with_ally: bool) -> bool {
        let relations = self.players.get(from).map_or(&[][..], |p| &p.relations[..]);
        crate::tasks::allied(relations, from, to, with_ally)
    }

    fn drops_at(&self, building: i16, actor: &Actor<'_>) -> bool {
        crate::tasks::drops_at(self.drop_sites, building, actor)
    }
}

impl Turns {
    /// The world as the header save holds it: its time, frame seed, turn order and players.
    #[must_use]
    pub fn new(
        world_time: u32,
        frame_seed: u32,
        player_turn: usize,
        player_time: Vec<u32>,
        players: Vec<Player>,
    ) -> Self {
        Self {
            world_time,
            frame_seed,
            player_turn,
            player_time,
            players,
            ticks: 0,
            next_object_id: 0,
            masters: Vec::new(),
            spatial: Spatial::default(),
            pathing: Pathing::default(),
            graphics: std::collections::BTreeMap::default(),
            drop_sites: Vec::new(),
            carry_rule: false,
            mode_680: 0,
            game_mode: 0,
            game_state: 0,
            victory_conquest: false,
            unit_cost_246: false,
            zones: crate::zones::Zones::default(),
            recycling: Recycling::default(),
            streams: Streams::default(),
            techs: Vec::new(),
        }
    }

    /// Run a command in the command phase. Where the slice does not model what the command does to
    /// the issuing player's units, that player's lane stops.
    fn run(&mut self, command: &Command) {
        match *command {
            Command::Build {
                player,
                master,
                unit_ai,
                position,
                ref builders,
                queued,
                commits,
            } => {
                // the pending debit first (FUN_141630d10 with 0), then the placement test and
                // the payment's: a refused BUILD places nothing
                if let Err(e) = self.debit_listed(player, master) {
                    let tick = self.ticks;
                    self.players[player].stop(tick, e);
                    return;
                }
                match self.build_allowed(player, master, [position[0], position[1]]) {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(e) => {
                        let tick = self.ticks;
                        self.players[player].stop(tick, e);
                        return;
                    }
                }
                let id = self.next_object_id;
                self.next_object_id += 1;
                let ai = unit_ai.and_then(|t| {
                    t.map(|t| UnitAi::constructed(t, position, self.streams.get(Stream::Ai)))
                        .transpose()
                });
                let ai = ai.unwrap_or_else(|e| {
                    // the constructor's draws are unknown, so the turn player's are too
                    let turn = &mut self.players[self.player_turn];
                    let at = turn.objects.len();
                    let _ = turn.unsupported.get_or_insert((self.ticks, at, e));
                    turn.stopped = true;
                    None
                });
                let owner = &mut self.players[player];
                owner.objects.push(Unit {
                    id,
                    position,
                    life: Lifecycle {
                        class: Class::Animated,
                        state: 0,
                        hp: 1.0,
                        master_id: master,
                        ..Lifecycle::default()
                    },
                    ai,
                    artifact: None,
                    unmodelled_action: None,
                    production: None,
                    gather_point: None,
                    build: None,
                    mover: None,
                    move_to: None,
                    fly: None,
                    construction: self
                        .masters
                        .get(player)
                        .and_then(|m| m.get(master))
                        .map(|m| Construction {
                            progress: 0.0,
                            desolid: true,
                            builders: 0,
                            hit_points: m.hit_points,
                            build_time: m.build_time,
                            record: crate::spatial::Obstruction {
                                kind: m.obstruction_type,
                                class: m.obstruction_class,
                            },
                            snow: false,
                            sprite_override: false,
                            started: false,
                        }),
                    work_rate_powerup: 0.0,
                    attackable: true,
                    untargetable: false,
                    under_attack: false,
                    last_human_order: 0,
                    town_bell: crate::town_bell::TownBell::default(),
                    pasture: -1,
                    my_pasture: -1,
                    annex_of: None,
                    hp_powerups: [0.0; 2],
                    hunt: None,
                    attack: None,
                    explore: None,
                    enter: None,
                    missile: None,
                    attack_count: 0,
                    // the TRIBE combat setup (FUN_14121df60): the master's volley fire
                    firepower: self
                        .masters
                        .get(player)
                        .and_then(|m| m.get(master))
                        .map_or(0.0, |m| m.volley_fire),
                    next_volley: 0,
                    reload_worldtime: 0,
                    garrison: Vec::new(),
                    // every building holds a gate's lock, open
                    gate: Some(Gate::default()),
                    workers: 0,
                    combat: true,
                    default_task: None,
                    default_fly: None,
                    stopped: false,
                    sprite: None,
                    anim_speed: None,
                    attack_timer: 0.0,
                    reload_stinger: 0.0,
                    reload_powerup: 0.0,
                    attack_powerup: 0.0,
                    armor_powerups: [0; 2],
                    armor_removed: [0; 2],
                    charge: 0.0,
                    charge_hit: false,
                });
                let built = self
                    .clear_footprint(player, master, [position[0], position[1]])
                    .and_then(|()| {
                        self.task_builders(
                            player,
                            id,
                            master,
                            position,
                            builders,
                            (queued, commits),
                        )
                    });
                if let Err(e) = built {
                    let owner = &mut self.players[player];
                    owner.stop(self.ticks, e);
                }
            }
            Command::Queue {
                player,
                building,
                unit,
                amount,
                train_time,
                rate,
                ref cost,
            } => {
                let queue = QueueAt {
                    building,
                    unit,
                    amount,
                    train_time,
                    rate,
                };
                if let Err(e) = self.queue(player, &queue, cost) {
                    let owner = &mut self.players[player];
                    owner.stop(self.ticks, e);
                }
            }
            Command::Research {
                player,
                building,
                count,
                tech,
                target,
                shift,
            } => {
                let r = ResearchAt {
                    building,
                    count,
                    tech,
                    target,
                    shift,
                };
                if let Err(e) = self.research(player, &r) {
                    let owner = &mut self.players[player];
                    owner.stop(self.ticks, e);
                }
            }
            Command::Unmodelled { player, why } => {
                let owner = &mut self.players[player];
                owner.stop(self.ticks, why);
            }
            Command::AutoQueue { player, fishtrap } => {
                let owner = &mut self.players[player];
                if fishtrap {
                    owner.auto_fishtrap_queue = !owner.auto_fishtrap_queue;
                } else {
                    owner.auto_farm_queue = !owner.auto_farm_queue;
                }
            }
            Command::Posture { player } => {
                let owner = &mut self.players[player];
                owner.posture = i32::from(owner.posture == 0);
            }
            Command::Gate { player, id } => {
                if let Err(e) = self.gate(player, id) {
                    let owner = &mut self.players[player];
                    owner.stop(self.ticks, e);
                }
            }
            Command::Unload {
                player,
                ref ids,
                point,
                flag,
                unit_type,
                sound,
            } => {
                if let Err(e) = self.unload(player, ids, point, flag, unit_type, sound) {
                    let owner = &mut self.players[player];
                    owner.stop(self.ticks, e);
                }
            }
            Command::UnitOrder {
                player,
                ref ids,
                target,
                through_ai,
                nearby,
                action,
                human,
            } => {
                if human {
                    self.human_order(ids);
                }
                let result = if action == 5 {
                    self.garrison_order(player, ids, target, through_ai, nearby)
                } else {
                    self.unit_order(player, ids, action)
                };
                if let Err(e) = result {
                    let owner = &mut self.players[player];
                    owner.stop(self.ticks, e);
                }
            }
            Command::TransformObject { player, id, master } => {
                if let Err(e) = self.transform_object(player, id, master) {
                    let owner = &mut self.players[player];
                    owner.stop(self.ticks, e);
                }
            }
            Command::Resign { player, dropped } => {
                if let Err(e) = self.resign(player, dropped) {
                    let owner = &mut self.players[player];
                    owner.stop(self.ticks, e);
                }
            }
            Command::AutoScout { player, ref ids } => {
                if let Err(e) = self.auto_scout(player, ids) {
                    let owner = &mut self.players[player];
                    owner.stop(self.ticks, e);
                }
            }
            Command::UnitAiState {
                player,
                ref ids,
                state,
            } => {
                if let Err(e) = self.unit_ai_state(player, ids, state) {
                    let owner = &mut self.players[player];
                    owner.stop(self.ticks, e);
                }
            }
            Command::GatherPoint {
                player,
                ref ids,
                point,
                target,
                target_master,
                own,
            } => {
                if let Err(e) = self.gather_point(player, ids, point, (target, target_master), own)
                {
                    let owner = &mut self.players[player];
                    owner.stop(self.ticks, e);
                }
            }
            Command::Order {
                player,
                ref units,
                order,
            } => {
                // FOLLOW (FUN_140e66a60) of a target the world lacks does nothing
                if let Some(Order::Follow(target)) = order
                    && self.object(target).is_none()
                {
                    return;
                }
                // +0x15: each unit with a unit AI takes the world time as its last human order
                // time (FUN_140e5e780), after the resets; FOLLOW sets it when a unit the world
                // holds belongs to a player who is not a computer (FUN_141659cf0)
                let human = match order {
                    Some(Order::Interact(o)) => o.human,
                    Some(Order::Move(o)) => o.human,
                    Some(Order::Follow(_)) => units.iter().any(|&id| {
                        self.object(id)
                            .is_some_and(|(p, _)| !self.players[p].computer)
                    }),
                    None => false,
                };
                for &id in units {
                    let unit = self
                        .players
                        .iter_mut()
                        .flat_map(|p| p.objects.iter_mut())
                        .find(|o| o.id == id);
                    if let Some(ai) = unit.and_then(|o| o.ai.as_mut()) {
                        ai.reset_for_order(self.streams.get(Stream::Ai));
                    }
                }
                if human {
                    self.human_order(units);
                }
                let result = match order {
                    Some(Order::Interact(o)) => self.interact(player, units, &o),
                    Some(Order::Move(o)) => self.move_order(player, units, &o),
                    Some(Order::Follow(target)) => self.follow(player, units, target),
                    None => Err(Unsupported("an order moves its units")),
                };
                if let Err(e) = result {
                    let owner = &mut self.players[player];
                    owner.stop(self.ticks, e);
                }
            }
        }
    }

    /// `FUN_140e5e780`: each unit of `ids` with a unit AI takes the world time as its last human
    /// order time (`+0x650`).
    fn human_order(&mut self, ids: &[i32]) {
        for &id in ids {
            let unit = self
                .players
                .iter_mut()
                .flat_map(|p| p.objects.iter_mut())
                .find(|o| o.id == id)
                .filter(|o| o.ai.is_some());
            if let Some(u) = unit {
                u.last_human_order = self.world_time;
                self.spatial.set_gait(u.id, u.gait());
            }
        }
    }

    /// BUILD's pending debit (`FUN_141630d10` with 0, before its placement test): every cost
    /// entry of the master, paid or not, by resource, comes off the player's pending debit.
    fn debit_listed(&mut self, player: usize, master: i16) -> Result<(), Unsupported> {
        let site = self
            .masters
            .get(player)
            .and_then(|m| m.get(master))
            .ok_or(Unsupported("BUILD of a master the player does not have"))?;
        let mut listed = [0i16; 4];
        for &(resource, amount, _) in &site.cost {
            if let Some(r) = usize::try_from(resource).ok().filter(|&r| r < 4) {
                listed[r] = listed[r].wrapping_add(amount);
            }
        }
        for (debit, amount) in self.players[player].pending_debit.iter_mut().zip(listed) {
            *debit -= f32::from(amount);
        }
        Ok(())
    }

    /// Whether BUILD (`FUN_140e7a400`) places a foundation of `master` at `at` for `player`: its
    /// placement test ([`crate::spawn::build_placement`], the master's slot 0x50), then the
    /// payment's (`FUN_14162fb60` -> `FUN_141618db0`: every cost entry of a resource, paid or
    /// not, at most what the player holds). A refused BUILD places, pays and draws nothing (the
    /// player's slot 0x380 shows why). A farm (master 50) on the terrains that swap it for 1187
    /// and a cost in population (attribute 4) are not modelled.
    fn build_allowed(&self, player: usize, master: i16, at: [f32; 2]) -> Result<bool, Unsupported> {
        if master == 50 {
            return Err(Unsupported("BUILD of a farm (its terrain's master swap)"));
        }
        let site = self.master_of(player, master)?;
        if self.spatial.sight.is_none() {
            return Err(Unsupported(
                "BUILD's placement test without the players' sight",
            ));
        }
        let restriction = self
            .spatial
            .terrain
            .as_ref()
            .and_then(|t| {
                usize::try_from(site.terrain_restriction)
                    .ok()
                    .and_then(|r| t.restrictions.get(r))
            })
            .ok_or(Unsupported(
                "a foundation's terrain restriction the data file does not have",
            ))?;
        let build = crate::spawn::BuildSite {
            player,
            human: !self.players[player].computer,
            clearance: site.clearance,
            class: site.class,
            obstructs: !matches!(site.obstruction_type, 0 | 12),
            restriction,
            hill_mode: site.hill_mode,
            placement_terrains: site.placement_terrains,
            zones: false,
        };
        let map = crate::spawn::WorldMap { turns: self };
        if crate::spawn::build_placement(&build, &map, at)? != 0 {
            return Ok(false);
        }
        let owner = &self.players[player];
        for &(resource, amount, _) in &site.cost {
            let Ok(r) = usize::try_from(resource) else {
                continue;
            };
            if r == 4 {
                return Err(Unsupported("a BUILD whose cost lists the population"));
            }
            let held = if r < 4 {
                owner.resources[r]
            } else {
                owner.attributes.get(r).copied().unwrap_or(0.0)
            };
            if held < f32::from(amount) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// A new building's footprint clearing (`FUN_1415c3620` from the building's constructor
    /// `FUN_1411de5f0`, both flags 0) for a building of `master` of `player` at `at`: the
    /// objects on the tiles within 4 of it (rows, then columns, held on the map), not of its own
    /// master, whose master can be built on (`+0x5e9`) and whose box overlaps its master's radii
    /// (on each axis the gap below the two radii summed), are deleted (slot 0xd0,
    /// `FUN_14118f1e0`: hit points 0, a sleeping one wakes to the end of its player's live list,
    /// `FUN_14119deb0`, and `set_state` 7; the updates take it to 8 and out of the list). Nothing
    /// for a master without radii. Not modelled: a wall under one of the player's gates (class
    /// 27, `FUN_1411ccbb0`), the flag masters without height (`FUN_140fde270`), an object other
    /// than a static one, with an obstruction record or inside another, and two on one tile (the
    /// tile's list order).
    fn clear_footprint(
        &mut self,
        player: usize,
        master: i16,
        at: [f32; 2],
    ) -> Result<(), Unsupported> {
        let site = self.master_of(player, master)?;
        let [rx, ry] = site.radius;
        if rx <= 0.0 && ry <= 0.0 {
            return Ok(());
        }
        let [w, h] = self.spatial.map;
        let edge = |v: f32, size: i32| {
            let low = (v - 4.0) as i32 as i16;
            let high = (v + 4.0) as i32 as i16;
            (
                low.max(0),
                if i32::from(high) < size {
                    high
                } else {
                    (size - 1) as i16
                },
            )
        };
        let (x0, x1) = edge(at[0], w);
        let (y0, y1) = edge(at[1], h);
        let mut deleted = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                let here: Vec<i32> = self
                    .spatial
                    .objects_on(i32::from(x), i32::from(y))
                    .filter(|(_, o)| {
                        o.master != master
                            && o.buildable_on != 0
                            && (o.position[0] - at[0]).abs() < o.radius[0] + rx
                            && (o.position[1] - at[1]).abs() < o.radius[1] + ry
                    })
                    .map(|(id, _)| id)
                    .collect();
                if 1 < here.len() {
                    return Err(Unsupported("a footprint clearing of two objects on a tile"));
                }
                deleted.extend(here);
            }
        }
        for id in deleted {
            let place = self.place(id).ok_or(Unsupported(
                "a footprint clearing of an object the slice does not hold",
            ))?;
            let (p, i) = match place {
                crate::order::Place::Live(p, i) => (p, i),
                crate::order::Place::Sleeping(p, i) => {
                    let woken = self.players[p].sleeping.swap_remove(i);
                    self.players[p].objects.push(woken);
                    (p, self.players[p].objects.len() - 1)
                }
            };
            let u = &mut self.players[p].objects[i];
            if u.life.class != Class::Static || u.mover.is_some() || u.ai.is_some() {
                return Err(Unsupported(
                    "a footprint clearing of an object above the static type",
                ));
            }
            if u.life.master_id == 0x112 || u.life.master_id == 0x699 || u.life.master_id == 0x6f9 {
                return Err(Unsupported(
                    "a footprint clearing of a flag master (FUN_140fde270)",
                ));
            }
            if u.life.inside.is_some()
                || self
                    .spatial
                    .get(id)
                    .is_some_and(|o| o.obstruction.is_some())
            {
                return Err(Unsupported(
                    "a footprint clearing of an object inside another or with a record",
                ));
            }
            if u.life.state < 7 {
                u.life.hp = 0.0;
                u.life.goto_sleep = false;
                u.life.state = 7;
                self.spatial.sight_state(id, 7);
            }
        }
        Ok(())
    }

    /// BUILD's part for its own player: the cost, paid when the foundation is placed (the
    /// pending debit came before, [`Turns::debit_listed`]), and per
    /// builder its ignore list emptied (`FUN_141197da0`), then slot 0x128 on the foundation (its
    /// slot 0x7e8 with task filter −1: the town-bell record, [`Turns::town_bell_order`]), then
    /// the work command (slot 37, `FUN_14106ff70`): the master it builds with
    /// ([`Masters::work_master`]), then the Build action from that master's task. A builder with
    /// a unit AI takes a queued BUILD, or one that `commits` while its AI holds an order not
    /// ready (`FUN_140c4dfa0`), as the build order in its queue instead (slot 0x30 into slot 0x28:
    /// order 0x2be on the foundation at BUILD's point, priority 100, the point shown, ready, as
    /// the game's saves hold it; [`Turns::build_order`] runs it). The readiness pass of a committing BUILD with
    /// orders queued (`FUN_140c42b60`) is not modelled.
    fn task_builders(
        &mut self,
        player: usize,
        foundation: i32,
        master: i16,
        position: [f32; 3],
        builders: &[i32],
        (queued, commits): (bool, bool),
    ) -> Result<(), Unsupported> {
        let masters = self
            .masters
            .get(player)
            .ok_or(Unsupported("BUILD tasks the builders: no master table"))?;
        let site = masters
            .get(master)
            .ok_or(Unsupported("BUILD of a master the player does not have"))?;
        let site_class = i32::from(site.class);
        let owner = &mut self.players[player];
        for &(resource, amount, paid) in &site.cost {
            let slot = usize::try_from(resource)
                .ok()
                .and_then(|r| owner.resources.get_mut(r))
                .ok_or(Unsupported("a cost in a resource other than the four"))?;
            if paid {
                *slot -= f32::from(amount);
            }
        }
        // a new foundation is desolid: its obstruction record comes with its first builder
        self.spatial.insert(
            foundation,
            Placed {
                position,
                radius: site.radius,
                class: site.class,
                player,
                master,
                obstruction_type: site.obstruction_type,
                buildable_on: site.buildable_on,
                tall: site.tall,
                speed: 0.0,
                unknown: None,
                obstruction: None,
            },
        );
        // its sight waits for its construction to start; its place links its tiles to its anchor
        // (FUN_1411e6780) and its desolid flag unlinks them (FUN_1411cf770)
        if let Some(v) = self.spatial.sight.as_mut() {
            let umv = v.link_building(position, site.radius);
            v.unlink_building(position, site.radius);
            v.add_seer(
                foundation,
                Seer {
                    player,
                    master,
                    los: site.line_of_sight,
                    state: 0,
                    explored: false,
                    building: Some(BuildingSight::default()),
                    umv: umv as u8,
                    capture: capture_kind(player, site.class, master),
                    kind: 80,
                },
            );
        }
        let target = TaskTarget {
            id: foundation,
            master: site,
            player,
            state: 0,
            held: (-1, 0.0),
            hp: 1.0,
            max_hp: f32::from(site.hit_points),
            attackable: true,
            pasture: -1,
            top_action: None,
        };
        let world = World {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        let mut chosen = Vec::new();
        let mut ordered = Vec::new();
        for &id in builders {
            let owner = &world.players[player];
            let unit = owner
                .objects
                .iter()
                .find(|o| o.id == id)
                .ok_or(Unsupported("a builder the slice does not have"))?;
            // the unit AI's queue path (the handler's last loop)
            if let Some(ai) = unit.ai.as_ref()
                && (queued || (commits && ai.has_unready_order()))
            {
                ordered.push(id);
                continue;
            }
            if unit.artifact.is_some() || unit.production.is_some() || unit.fly.is_some() {
                return Err(Unsupported("a builder with an action (its replacement)"));
            }
            let actor = Actor {
                id,
                master: masters
                    .get(unit.life.master_id)
                    .ok_or(Unsupported("a unit of a master the player does not have"))?,
                player,
                held: (unit.life.attribute_type, unit.life.amount),
                attribute_95: owner.attributes.get(95).copied().unwrap_or(0.0),
                civ: owner.civ,
            };
            let (with, task) = masters
                .work_master(&actor, &target, &target, -1, &world)?
                .ok_or(Unsupported("a builder with no task for its foundation"))?;
            if task.action_type != BUILD {
                return Err(Unsupported(
                    "a builder whose task for its foundation is not Build",
                ));
            }
            if !crate::tasks::enabled(&owner.attributes, task.enablement) {
                return Err(Unsupported("a builder whose Build task is not enabled"));
            }
            let builder = masters
                .get(with)
                .expect("work_master returns a master the player has");
            let gap = footprint_gap(unit.position, builder.radius, position, site.radius);
            let task = BuildTask {
                range: task.work_range,
                work_value: task.work_val1,
                work_rate: builder.work_rate + unit.work_rate_powerup,
            };
            chosen.push((id, with, BuildAction::start(foundation, gap, task)));
        }
        if !ordered.is_empty() && !chosen.is_empty() {
            return Err(Unsupported(
                "a BUILD whose builders take both paths (their order in the packet)",
            ));
        }
        for id in ordered {
            let idx = self.players[player]
                .objects
                .iter()
                .position(|o| o.id == id)
                .expect("found above");
            let unit = &mut self.players[player].objects[idx];
            if let Some(m) = unit.mover.as_mut() {
                m.ignore_units.clear();
            }
            let state = unit.life.state;
            let order = crate::unit_ai::QueuedOrder {
                issuer: player as i32,
                order: BUILD_ORDER,
                priority: 100,
                target: foundation,
                target_owner: player as i32,
                point: [position[0], position[1], 0.0],
                range: 0.0,
                shown: [position[0], position[1]],
                ready: true,
                target_type: i32::from(master),
            };
            let ai = unit.ai.as_mut().expect("chosen with a unit AI");
            let _ = ai.queue_order(state, order, false, false)?;
            if commits {
                return Err(Unsupported(
                    "the readiness pass of a committing BUILD with orders queued (FUN_140c42b60)",
                ));
            }
        }
        for (id, with, build) in chosen {
            let idx = self.players[player]
                .objects
                .iter()
                .position(|o| o.id == id)
                .expect("found above");
            if let Some(m) = self.players[player].objects[idx].mover.as_mut() {
                m.ignore_units.clear();
            }
            // slot 0x128 -> slot 0x7e8 (task filter -1): the villager's town-bell record, its
            // flag cleared (FUN_1412067f0)
            self.town_bell_order(player, idx, foundation, -1)?;
            // the work command (FUN_14106ff70): the master changes (slot 0x90, its action list
            // cleared); the Build action takes its foundation (FUN_140a7a230, slot 0x178)
            if self.players[player].objects[idx].life.master_id != with {
                self.change_master(player, idx, with)?;
            }
            self.take(foundation);
            // the work command's notify (slot 0x828): the build order and the foundation, with
            // the point slot 0x128 passes (BUILD's x and y, on the ground)
            let unit = &mut self.players[player].objects[idx];
            if let Some(ai) = unit.ai.as_mut() {
                ai.notify_work(
                    BUILD_ORDER,
                    BUILD_ACTION,
                    Some((foundation, site_class)),
                    [position[0], position[1], 0.0],
                    true,
                    self.streams.get(Stream::Ai),
                )?;
            }
            // the install (slot 0x808): the action list clears (its actions' teardowns), then
            // holds the Build
            self.clear_actions(player, idx)?;
            self.players[player].objects[idx].build = Some(build);
        }
        Ok(())
    }

    /// The build order (`0x2be`) a unit AI's queue runs (slot 0x230's case: slot 0xf0,
    /// `FUN_140c47d80`, with 1) on its foundation: the unit's slot 0x380 (`FUN_14120d410`) finds
    /// the task through slot 0x820 (the work command's master switch,
    /// [`Masters::work_master`]) and installs its Build action (the action list cleared); then
    /// the AI's action becomes the build action (602; the last one kept), its target the
    /// foundation (the last one kept) at its place with its master's class, and the tail takes
    /// the order (702) with its priority. Unlike BUILD's own path neither the town-bell record nor
    /// the work's notify. A unit already building (its Build kept or retargeted, slot 0x48) is
    /// not modelled.
    pub(crate) fn build_order(
        &mut self,
        player: usize,
        idx: usize,
        order: crate::unit_ai::QueuedOrder,
    ) -> Result<(), Unsupported> {
        let foundation = order.target;
        if self.players[player].objects[idx].build.is_some() {
            return Err(Unsupported(
                "a build order of a unit already building (slot 0x48)",
            ));
        }
        let (fp, f) = self
            .object(foundation)
            .ok_or(Unsupported("a build order on a foundation that is gone"))?;
        let (f_master, f_state, f_held, f_hp, f_attackable, f_pasture, f_at) = (
            f.life.master_id,
            f.life.state,
            (f.life.attribute_type, f.life.amount),
            f.life.hp,
            f.attackable,
            f.pasture,
            f.position,
        );
        let masters = self
            .masters
            .get(player)
            .ok_or(Unsupported("a build order: no master table"))?;
        let site = self
            .masters
            .get(fp)
            .and_then(|m| m.get(f_master))
            .ok_or(Unsupported(
                "a build order on a master the slice does not have",
            ))?;
        let target = TaskTarget {
            id: foundation,
            master: site,
            player: fp,
            state: f_state,
            held: f_held,
            hp: f_hp,
            max_hp: f32::from(site.hit_points),
            attackable: f_attackable,
            pasture: f_pasture,
            top_action: None,
        };
        let world = World {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        let owner = &world.players[player];
        let unit = &owner.objects[idx];
        let actor = Actor {
            id: unit.id,
            master: masters
                .get(unit.life.master_id)
                .ok_or(Unsupported("a unit of a master the player does not have"))?,
            player,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: owner.attributes.get(95).copied().unwrap_or(0.0),
            civ: owner.civ,
        };
        let (with, task) = masters
            .work_master(&actor, &target, &target, -1, &world)?
            .ok_or(Unsupported("a build order with no task for its foundation"))?;
        if task.action_type != BUILD || !crate::tasks::enabled(&owner.attributes, task.enablement) {
            return Err(Unsupported(
                "a build order whose task is not an enabled Build",
            ));
        }
        let builder = masters
            .get(with)
            .expect("work_master returns a master the player has");
        let gap = footprint_gap(unit.position, builder.radius, f_at, site.radius);
        let build = BuildAction::start(
            foundation,
            gap,
            BuildTask {
                range: task.work_range,
                work_value: task.work_val1,
                work_rate: builder.work_rate + unit.work_rate_powerup,
            },
        );
        let class = i32::from(site.class);
        if self.players[player].objects[idx].life.master_id != with {
            self.change_master(player, idx, with)?;
        }
        self.take(foundation);
        self.clear_actions(player, idx)?;
        let unit = &mut self.players[player].objects[idx];
        unit.build = Some(build);
        let ai = unit
            .ai
            .as_mut()
            .ok_or(Unsupported("a build order without a unit AI"))?;
        // FUN_140c47d80
        ai.last_action = ai.current_action;
        ai.current_action = BUILD_ACTION;
        ai.target_point = f_at;
        ai.last_target = ai.current_target;
        ai.last_target_type = ai.target_type;
        ai.current_target = foundation;
        ai.target_type = class;
        // the tail (`LAB_140c5d432`)
        ai.current_order = BUILD_ORDER;
        ai.order_priority = order.priority;
        Ok(())
    }

    /// SET GATHER POINT's handler ([`Command::GatherPoint`]).
    fn gather_point(
        &mut self,
        player: usize,
        ids: &[i32],
        [x, y]: [f32; 2],
        (target, target_master): (i32, i16),
        own: bool,
    ) -> Result<(), Unsupported> {
        for &id in ids {
            // another player's object (the game clears its id) or one that is gone
            let Some(i) = self.players[player].objects.iter().position(|u| u.id == id) else {
                continue;
            };
            let unit = &self.players[player].objects[i];
            let master = self
                .masters
                .get(player)
                .and_then(|m| m.get(unit.life.master_id))
                .ok_or(Unsupported(
                    "SET GATHER POINT on a master the slice does not have",
                ))?;
            if master.garrison == 0 {
                return Err(Unsupported(
                    "SET GATHER POINT on a building that does not garrison (its train list)",
                ));
            }
            let z = unit.gather_point.map_or(0.0, |g| g.point[2]);
            let gather = if own {
                Some(GatherPoint {
                    point: [unit.position[0], unit.position[1], z],
                    target: unit.id,
                    target_master: unit.life.master_id,
                })
            } else if target == -1 && x == -1.0 && y == -1.0 {
                None
            } else {
                Some(GatherPoint {
                    point: [x, y, z],
                    target,
                    target_master,
                })
            };
            self.players[player].objects[i].gather_point = gather;
        }
        Ok(())
    }

    /// RESEARCH (`0x140e7e270`) for `player`: a target other than −1, or a technology the tree
    /// has not got, does nothing. The costs it lists (`FUN_1413d4a00`) come off the pending
    /// debit (`FUN_141630d10`). The building is the packet's (the least loaded of several,
    /// `FUN_1411d0e90`, and the shared kind's every building, `FUN_1411c3fd0`, are not modelled);
    /// a building of master type 80 alive (state 2) whose master is among the technology's
    /// locations, with the technology in state 1 or repeatable, and without a top action of
    /// 0x7b to 0x7d (`FUN_141227a00`), queues it ([`Turns::queue_research`]); queued, its unit AI
    /// takes the action value 0x26c. Not held: the interface's view and selection
    /// (`FUN_140e6de20`), the game's per-technology overrides (`FUN_140f39f40`,
    /// `FUN_140f39e80`: none, probed for Loom), the local player's statistics flag. Not
    /// modelled: the computer players' parts (`player+0x7a8`, `FUN_1411d5520`), the player's
    /// `+0x648` (0 for every player probed), Castle Age's flag (tech 0x66, game type 0x10).
    fn research(&mut self, player: usize, r: &ResearchAt) -> Result<(), Unsupported> {
        let owner = &self.players[player];
        if r.target != -1 || r.tech < 0 || owner.techs.len() <= r.tech as usize {
            return Ok(());
        }
        let tech = r.tech as usize;
        let def = self
            .techs
            .get(tech)
            .ok_or(Unsupported("a technology the data file has not got"))?;
        let get = |i: i16| crate::tech::Attributes::get(owner, i);
        let factors = crate::tech::factors(r.tech, &get)?;
        let listed = owner.techs[tech].listed_costs(def, factors);
        let mut sums = [0i16; 4];
        for (attribute, amount) in listed {
            if let Some(k) = usize::try_from(attribute).ok().filter(|&k| k < 4) {
                sums[k] = sums[k].wrapping_add(amount);
            }
        }
        for (debit, amount) in self.players[player].pending_debit.iter_mut().zip(sums) {
            *debit -= f32::from(amount);
        }
        let owner = &self.players[player];
        let state = &owner.techs[tech];
        let repeatable = state.repeatable(def, false);
        if def.kind == crate::tech::SHARED_KIND {
            return Err(Unsupported(
                "RESEARCH of the shared kind (FUN_1411c3fd0 over the packet's buildings)",
            ));
        }
        if 1 < r.count {
            return Err(Unsupported(
                "RESEARCH at the least loaded of several buildings (FUN_1411d0e90)",
            ));
        }
        if state.state != 1 && !repeatable {
            return Err(Unsupported(
                "RESEARCH of a technology not in state 1 (FUN_140f39e80)",
            ));
        }
        let Some(i) = owner.objects.iter().position(|u| u.id == r.building) else {
            let elsewhere = self.players.iter().any(|q| {
                q.objects
                    .iter()
                    .chain(&q.sleeping)
                    .any(|u| u.id == r.building)
            });
            return if elsewhere {
                Err(Unsupported("RESEARCH at another player's object"))
            } else {
                Ok(())
            };
        };
        let building = &owner.objects[i];
        let master = self.master_of(player, building.life.master_id)?;
        let at = state
            .locations
            .iter()
            .any(|l| l.building == building.life.master_id);
        if master.kind != BUILDING_KIND || building.life.state != 2 || !at {
            return Ok(());
        }
        if owner.computer {
            return Err(Unsupported(
                "a computer player's RESEARCH (FUN_1411d5520, player+0x7a8)",
            ));
        }
        let top = building.top_action_kind().unwrap_or(0);
        if building.unmodelled_action.is_some() {
            return Err(Unsupported(
                "RESEARCH at a building with an unmodelled action",
            ));
        }
        if BLOCKS_RESEARCH.contains(&top) {
            return Ok(());
        }
        if r.tech == 0x66 {
            return Err(Unsupported(
                "RESEARCH of Castle Age (the game's +0x820 flag)",
            ));
        }
        if self.queue_research(player, i, r.tech, r.shift)? {
            let building = &mut self.players[player].objects[i];
            if let Some(ai) = building.ai.as_mut() {
                ai.set_action(RESEARCH_ACTION);
            }
        }
        Ok(())
    }

    /// `FUN_1411d70f0` and the technology's part of `FUN_1411cbc40` for the building at `i` of
    /// `player`: one research, five with `shift` unless the technology allows only one at a time
    /// or its effect upgrades a unit. A building with 15 queued already refuses (the game's
    /// event 0x84 is not held); the count is held to the queue's room, and to the cap of the
    /// technology's researches queued and done (0x100 unless its cap is at least 0) for a
    /// stacked technology. Each research is paid ([`crate::tech::TechState::pay`]) until one
    /// cannot be; those paid join the queue, as one entry, each with the research time of the
    /// building's location, and count in the technology's pending researches. True when any
    /// did. Not held: the building's queue entries as the game groups them (the slice's queue
    /// holds one item per research), their costs (`FUN_1411c81a0`, `FUN_1411c3190`) and serials
    /// (`+0xcd8`).
    fn queue_research(
        &mut self,
        player: usize,
        i: usize,
        tech: i16,
        shift: bool,
    ) -> Result<bool, Unsupported> {
        let t = tech as usize;
        let def = &self.techs[t];
        let owner = &mut self.players[player];
        let building = &owner.objects[i];
        let master = building.life.master_id;
        let state = &owner.techs[t];
        let one = !state.allow_multiple || def.upgrades();
        let count: i16 = if !shift || one { 1 } else { 5 };
        let queued = building.production.as_ref().map_or(0, |p| p.queue.len()) as i16;
        if 14 < queued {
            return Ok(false);
        }
        let mut n = if count + queued < 16 {
            count
        } else {
            15 - queued
        };
        let done = state.pending.wrapping_add(state.times_researched);
        let cap = if state.research_cap < 0 {
            0x100
        } else {
            state.research_cap
        };
        if state.stack && cap - done < n {
            n = cap - done;
        }
        let time = state
            .locations
            .iter()
            .find(|l| l.building == master)
            .map(|l| f32::from(l.time))
            .ok_or(Unsupported(
                "a research at a building not among its locations",
            ))?;
        // FUN_14162fd90's repeat flag: the player's +0x648 (not held) or FUN_1413d1050
        let repeat = state.repeatable(def, false);
        let mut paid = 0;
        while paid < n {
            let get = |k: i16| crate::tech::Attributes::get(&*owner, k);
            let factors = crate::tech::factors(tech, &get)?;
            let mut state = std::mem::take(&mut owner.techs[t]);
            let ok = state.pay(def, factors, repeat, master, owner);
            owner.techs[t] = state;
            if !ok? {
                break;
            }
            paid += 1;
        }
        if paid <= 0 {
            return Ok(false);
        }
        let get = |k: i16| crate::tech::Attributes::get(&*owner, k);
        let costs = owner.techs[t].entry_costs(def, crate::tech::factors(tech, &get)?);
        let building = &mut owner.objects[i];
        let production = building
            .production
            .as_mut()
            .ok_or(Unsupported("RESEARCH at a building without its production"))?;
        for _ in 0..paid {
            production.queue.push_back(Queued {
                master: -1,
                tech,
                train_time: time,
                costs,
            });
        }
        owner.techs[t].pending += paid;
        Ok(true)
    }

    /// QUEUE at one building: per unit (the game recurses, so the last unit drawn is queued first)
    /// the villager's male or female draw, then the cost and the queue entry, which keeps the
    /// master's three cost entries (resource and amount) for a cancel's refund. A unit the
    /// player cannot pay for (`FUN_14162fb60` -> `FUN_141618db0`: each cost entry but the
    /// population's, paid or not, needs its amount) is not queued; its draw stays.
    fn queue(
        &mut self,
        player: usize,
        q: &QueueAt,
        cost: &[(i16, i16, bool)],
    ) -> Result<(), Unsupported> {
        let action = self.streams.get(Stream::Action);
        let masters: Vec<i16> = (0..q.amount)
            .map(|_| match q.unit {
                VILLAGER | VILLAGER_ANY => villager(action.raw64()),
                other => other,
            })
            .collect();
        let owner = &mut self.players[player];
        let index = owner
            .objects
            .iter()
            .position(|o| o.id == q.building)
            .ok_or(Unsupported("QUEUE at a building the slice does not have"))?;
        let queued = owner.objects[index]
            .production
            .as_ref()
            .map_or(0, |p| p.queue.len());
        if queued + masters.len() > QUEUE_LIMIT {
            return Err(Unsupported("QUEUE beyond a building's 15 units"));
        }
        for master in masters.into_iter().rev() {
            let mut can_pay = true;
            for &(resource, amount, _) in cost.iter().filter(|c| 0 <= c.0 && c.0 != POPULATION) {
                let held = owner
                    .resources
                    .get(resource as usize)
                    .ok_or(Unsupported("a cost in a resource the slice does not track"))?;
                can_pay &= f32::from(amount) <= *held;
            }
            if !can_pay {
                continue;
            }
            for &(resource, amount, _) in cost.iter().filter(|c| 0 <= c.0 && c.2) {
                let slot = owner
                    .resources
                    .get_mut(resource as usize)
                    .ok_or(Unsupported("a cost in a resource the slice does not track"))?;
                *slot -= f32::from(amount);
            }
            let production = owner.objects[index]
                .production
                .get_or_insert_with(|| Production {
                    queue: std::collections::VecDeque::default(),
                    action: false,
                    make_paid: false,
                    progress: 0.0,
                    rate: q.rate,
                });
            let entry = |k: usize| {
                cost.get(k)
                    .map_or((-1, 0.0), |&(r, a, _)| (i32::from(r), f32::from(a)))
            };
            production.queue.push_back(Queued {
                master,
                tech: -1,
                train_time: q.train_time,
                costs: [entry(0), entry(1), entry(2)],
            });
        }
        Ok(())
    }

    /// The shape mesh the game's load builds, which no save holds: an empty mesh of the map's
    /// width, then the load's rehook of each object (`RGE_Static_Object::rehook`, slot 9, through
    /// `FUN_141665650` and `FUN_1411689f0`: each player's live list, then its sleeping list):
    /// its position set again (slot 0x58 -> `FUN_14118c540`: below state 7 and tall, then
    /// `FUN_141056460`'s shape by its master's obstruction kind, not 0 or 12, and class, not 0,
    /// for an object that does not move, [`Pathing::place_shape`]), then with an obstruction
    /// record the refresh an update ends with (`FUN_141054fe0`, [`Pathing::refresh`]). An object
    /// inside another has none (it left the obstruction manager); `mROOAlreadyCalled`
    /// (`+0x686`) is not held (0 at a game's start), and a desolid foundation in a save is left
    /// without one, as the game's own placement leaves it (unchecked at a load).
    pub fn build_mesh(&mut self) {
        let width = u32::try_from(self.spatial.map[0]).unwrap_or(0);
        self.pathing.start_mesh(width);
        for (q, p) in self.players.iter().enumerate() {
            for u in p.objects.iter().chain(&p.sleeping) {
                if u.life.inside.is_some() || 7 <= u.life.state {
                    continue;
                }
                let (Some(placed), Some(m)) = (
                    self.spatial.get(u.id),
                    self.masters.get(q).and_then(|m| m.get(u.life.master_id)),
                ) else {
                    continue;
                };
                let moving = u.mover.as_ref().is_some_and(Mover::is_moving);
                let desolid = u.construction.is_some_and(|c| c.desolid);
                let kind = m.obstruction_type;
                if placed.tall
                    && kind != 0
                    && kind != 12
                    && m.obstruction_class != 0
                    && !moving
                    && !desolid
                {
                    self.pathing.place_shape(u.id, placed);
                }
                self.pathing.refresh(u.id, placed, moving, m.outline);
            }
        }
    }

    /// One world update of `dt` game milliseconds without commands.
    pub fn tick(&mut self, dt: u32) {
        self.tick_with(dt, &[]);
    }

    /// Tell the world which objects move now and how the players stand to each other, for the
    /// walk's query ([`Spatial::walk_hits`]), as each update does first: for a look at the
    /// world between updates.
    pub fn sync_world(&mut self) {
        self.sync_moving();
    }

    /// Tell the world which objects move now and how the players stand to each other, for the
    /// walk's query ([`Spatial::walk_hits`]).
    fn sync_moving(&mut self) {
        for p in &self.players {
            for u in p.objects.iter().chain(&p.sleeping) {
                self.spatial.set_gait(u.id, u.gait());
                self.spatial.set_annex(u.id, u.annex_of);
            }
        }
        if self.spatial.relations.is_empty() {
            self.spatial.relations = self.players.iter().map(|p| p.relations.clone()).collect();
        }
    }

    /// One world update of `dt` game milliseconds, with the commands recorded in it.
    ///
    /// # Panics
    ///
    /// Never: each player updates in turn, so the one it splits off exists.
    pub fn tick_with(&mut self, dt: u32, commands: &[Command]) {
        self.ticks += 1;
        self.world_time = self.world_time.wrapping_add(dt);
        self.streams.seed_all(self.frame_seed);
        self.sync_moving();
        for command in commands {
            self.run(command);
        }
        self.sync_moving();
        for p in 0..self.players.len() {
            self.player_time[p] = self.player_time[p].wrapping_add(dt);
            let turn = (p == self.player_turn).then_some(self.player_time[p]);
            self.pathing.tiles_changed(self.spatial.take_changes());
            // An object whose update is not modelled freezes, and so do all live objects of a
            // player a command stopped; dead objects still run their state steps, which draw
            // nothing and depend on nothing the stop changed.
            let attribute_97: Vec<f32> = self
                .players
                .iter()
                .map(|q| q.attributes.get(97).copied().unwrap_or(0.0))
                .collect();
            let computers: Vec<bool> = self.players.iter().map(|q| q.computer).collect();
            let attribute_204 = self
                .players
                .iter()
                .any(|q| q.attributes.get(204).is_some_and(|&v| v != 0.0));
            let mut i = 0;
            while i < self.players[p].objects.len() {
                let player = &self.players[p];
                let stopped = player.stopped || player.objects[i].stopped;
                if stopped && player.objects[i].life.is_alive() {
                    self.spatial.freeze(player.objects[i].id);
                    i += 1;
                    continue;
                }
                let mut unit = std::mem::take(&mut self.players[p].objects[i]);
                let mut victim = unit.attack_target().and_then(|t| self.victim(t, p));
                let ai_target = unit
                    .ai
                    .as_ref()
                    .map(|ai| ai.current_target)
                    .filter(|&t| t != -1)
                    .map_or(Ok(None), |t| self.seen(t, p));
                let blasted = victim
                    .as_ref()
                    .map_or_else(Vec::new, |v| self.blasted(&unit, p, v.id));
                let (before, rest) = self.players.split_at_mut(p);
                let (player, after) = rest.split_first_mut().expect("the player updating");
                let mut c = Context {
                    dt,
                    dt_seconds: seconds(dt),
                    tick: self.ticks,
                    turn,
                    world_time: self.world_time,
                    computer: player.computer,
                    streams: &mut self.streams,
                    spatial: &mut self.spatial,
                    pathing: &mut self.pathing,
                    construction_rate: player.construction_rate,
                    masters: self.masters.get(p),
                    victim: victim.as_mut(),
                    ai_target,
                    blasted,
                    own_killed: None,
                    remains: None,
                    refund: None,
                    pauses: Vec::new(),
                    releases: Vec::new(),
                    takes: Vec::new(),
                    made: Vec::new(),
                    ghost_deaths: Vec::new(),
                    started: Vec::new(),
                    unwatched: Vec::new(),
                    recycling: &mut self.recycling,
                    mode_1: self.mode_680 == 1,
                    player: p,
                    civ: player.civ,
                    attributes: &player.attributes,
                    graphics: &self.graphics,
                    drop_sites: &self.drop_sites,
                    every_masters: &self.masters,
                    attribute_97: &attribute_97,
                    player_turn: self.player_turn,
                    player_time: &self.player_time,
                    computers: &computers,
                    attribute_204,
                    mode_680: self.mode_680,
                    scenario: scenario_mode(self.game_mode),
                    attribute_adds: Vec::new(),
                    techs_done: Vec::new(),
                    work_order: None,
                    order_on_target: None,
                    zones: &mut self.zones,
                    foreign: Foreign { before, after },
                };
                let mut result = unit.update(&mut c, &mut player.objects);
                let (adds, ordered) = (
                    std::mem::take(&mut c.attribute_adds),
                    c.order_on_target.take(),
                );
                let techs_done = std::mem::take(&mut c.techs_done);
                let work_order = c.work_order.take();
                let (blasted, own_killed) = (std::mem::take(&mut c.blasted), c.own_killed.take());
                let (remains, refund) = (c.remains.take(), c.refund.take());
                let pauses = std::mem::take(&mut c.pauses);
                let releases = std::mem::take(&mut c.releases);
                let (takes, made) = (std::mem::take(&mut c.takes), std::mem::take(&mut c.made));
                let (ghost_deaths, started, unwatched) = (
                    std::mem::take(&mut c.ghost_deaths),
                    std::mem::take(&mut c.started),
                    std::mem::take(&mut c.unwatched),
                );
                if let Some(e) = self.spatial.take_sight_unsupported() {
                    result = result.and(Err(e));
                }
                self.spatial.set_gait(unit.id, unit.gait());
                player.objects[i] = unit;
                player.objects.extend(made);
                for (kind, amount) in adds {
                    if let Err(e) = self.players[p].add_attribute(kind, amount) {
                        result = result.and(Err(e));
                    }
                }
                for t in techs_done {
                    if let Err(e) = self.research_built(p, t) {
                        result = result.and(Err(e));
                    }
                }
                // slot 0x5f0 on each unit the walk paused: its walked level's pause
                for (id, pause) in pauses {
                    self.pause(id, pause);
                }
                // a dead building leaves its owner's watched list (FUN_141091310), then the
                // ghosts of a death (FUN_14118e990, while the game state is not 2) and the
                // watch of a started construction (FUN_1411e5670)
                for id in unwatched {
                    self.unwatch(p, id);
                }
                for id in ghost_deaths {
                    if self.game_state != 2
                        && let Err(e) = self.ghosts_at_death(p, id)
                    {
                        result = result.and(Err(e));
                    }
                }
                for id in started {
                    if let Some((owner, _)) = self.object(id)
                        && let Err(e) = self.watch_started(owner, id)
                    {
                        result = result.and(Err(e));
                    }
                }
                // a dead unit's paid cost leaves attributes 98 and 164, each at least 0
                // (FUN_14121cc40: slots 0xf8, then 0xf0 with 0)
                if let Some(cost) = refund.filter(|&c| c != 0.0) {
                    for kind in [98, 164] {
                        let owner = &mut self.players[p];
                        if let Err(e) = owner.add_attribute(kind, -cost) {
                            result = result.and(Err(e));
                        } else if owner.attributes[kind as usize] < 0.0 {
                            owner.attributes[kind as usize] = 0.0;
                        }
                    }
                }
                // the remains of a dying sprite's end, a new object of the owner (FUN_1411a4e90)
                if let Some((master, at, facet)) = remains
                    && let Err(e) = self.remains(p, master, at, facet)
                {
                    result = result.and(Err(e));
                }
                // TRIBE_Building_Object::update: the production after the combat update
                if matches!(result, Ok(Step::Stay))
                    && let Err(e) = self.produce(p, i, seconds(dt))
                {
                    result = Err(e);
                }
                if let Some(work) = work_order {
                    if !matches!(result, Ok(Step::Stay)) {
                        result = result.and(Err(Unsupported(
                            "a builder's work order of a unit that leaves its list",
                        )));
                    } else if let Err(e) = self.order_work(p, i, &work) {
                        result = Err(e);
                    }
                }
                if let (Ok(Step::Stay), Some(order)) = (&result, ordered) {
                    if let Err(e) = self.order_on_target(p, i, order) {
                        result = Err(e);
                    }
                } else if ordered.is_some() && result.is_ok() {
                    result = Err(Unsupported(
                        "an order on a target of a unit that leaves its list",
                    ));
                }
                // a hit's write-backs in the order the hit made them: the blast's objects, the
                // unit itself (a retaliation), its target
                for v in &blasted {
                    if let Err(e) = self.set_victim(v) {
                        result = result.and(Err(e));
                    }
                }
                if let Some(kill) = own_killed
                    && let Err(e) = self.killed(p, i, kill)
                {
                    result = result.and(Err(e));
                }
                if let Some(v) = victim
                    && let Err(e) = self.set_victim(&v)
                {
                    result = result.and(Err(e));
                }
                for id in takes {
                    self.take(id);
                }
                for (id, by) in releases {
                    if let Err(e) = self.release(id, by) {
                        result = result.and(Err(e));
                    }
                }
                self.pathing.tiles_changed(self.spatial.take_changes());
                let player = &mut self.players[p];
                match result {
                    Ok(Step::Leave) => {
                        let gone = player.objects.swap_remove(i);
                        if let Err(e) = self.spatial.leave(gone.id) {
                            let _ = player.unsupported.get_or_insert((self.ticks, i, e));
                        }
                        // its owner's watched list lets it go (FUN_141091310 as it leaves)
                        self.unwatch(p, gone.id);
                        // a recyclable master's missile goes to the pool of type 60
                        let recyclable = self
                            .masters
                            .get(p)
                            .and_then(|m| m.get(gone.life.master_id))
                            .is_some_and(|m| m.recyclable);
                        if let Some(m) = gone.missile.as_ref().filter(|_| recyclable) {
                            self.recycling.missiles.push((gone.id, m.flying));
                        }
                        self.pathing.removed(gone.id);
                        self.pathing.tiles_changed(self.spatial.take_changes());
                    }
                    Ok(Step::Sleep) => {
                        // the list's remove (FUN_1411664a0) puts its last object in the
                        // sleeper's place, and the list's loop (0x141166550) goes on past it:
                        // that object's update waits for the next tick
                        let asleep = player.objects.swap_remove(i);
                        player.sleeping.push(asleep);
                        i += 1;
                    }
                    Ok(Step::Captured(new, also)) => {
                        // the old owner's list loses them (FUN_1411664a0, the last in each
                        // place; the loop goes on past the first) and the new owner's gains them
                        // (FUN_141616200); the storage: the old owner loses flags 2, 16 and 32
                        // (FUN_1411a56c0 with 1), the new one gains flags 1, 2 and 32
                        // (FUN_141193860)
                        let mut moved = vec![player.objects.swap_remove(i)];
                        for id in also {
                            if let Some(k) = player.objects.iter().position(|u| u.id == id) {
                                moved.push(player.objects.swap_remove(k));
                            }
                        }
                        let mut transfer = Vec::new();
                        for u in &moved {
                            let storage = self
                                .masters
                                .get(p)
                                .and_then(|m| m.get(u.life.master_id))
                                .map(|m| m.storage.clone())
                                .unwrap_or_default();
                            transfer.extend(storage);
                        }
                        let mut failed = None;
                        for &(kind, amount, flag) in &transfer {
                            if kind < 0 {
                                continue;
                            }
                            if matches!(flag, 2 | 16 | 32)
                                && let Err(e) = self.players[p].add_attribute(kind, -amount)
                            {
                                let _ = failed.get_or_insert(e);
                            }
                            if matches!(flag, 1 | 2 | 32)
                                && let Err(e) = self.players[new].add_attribute(kind, amount)
                            {
                                let _ = failed.get_or_insert(e);
                            }
                        }
                        if let Some(e) = failed {
                            let _ = self.players[p]
                                .unsupported
                                .get_or_insert((self.ticks, i, e));
                        }
                        self.players[new].objects.extend(moved);
                        i += 1;
                    }
                    Ok(Step::Stay) => i += 1,
                    Err(e) => {
                        player.freezes.push((self.ticks, player.objects[i].id, e));
                        let _ = player.unsupported.get_or_insert((self.ticks, i, e));
                        player.objects[i].stopped = true;
                        self.spatial.freeze(player.objects[i].id);
                        i += 1;
                    }
                }
            }
        }
        self.spatial.grow_unknown(seconds(dt));
        let s1 = self.streams.get(Stream::World).rand15();
        // every player's ghosts (FUN_141166550 on +0x40), then every player's watched list
        // (FUN_1410905e0); the objects' visibility masks (FUN_14107d280 on +0x80, before the
        // draw) are the registry's, kept as the objects update
        for p in 0..self.players.len() {
            if let Err(e) = self.update_ghosts(p) {
                let tick = self.ticks;
                self.players[p].stop(tick, e);
            }
        }
        for p in 0..self.players.len() {
            if let Err(e) = self.watch_pass(p) {
                let tick = self.ticks;
                self.players[p].stop(tick, e);
            }
        }
        self.streams.seed_all(s1);
        self.player_time[self.player_turn] = 0;
        self.player_turn = (self.player_turn + 1) % self.players.len();
        self.victory_check();
        self.frame_seed = self.streams.get(Stream::World).rand15();
    }
}

/// What a hit reads of the object `u` of player `p` ([`Victim`]): its master's values and its
/// own, its player's elevation attributes, the terrain defense of its tile (`terrain`, the map's;
/// 1 for an object inside another, which has no tile) and the game's setting `mode_680`. Its
/// relations to an attacker (`allied`) and whether its place is a guess (`uncertain`) are the
/// caller's. The slice holds neither its unit AI's alert mode and attackers nor its facing.
/// `FUN_141302540`: the game mode (options `+0x34`) is Scenario (3), Campaign (4), King of the
/// Hill (5) or Co-op Campaign (15), by aoc-mgz's game types.
#[must_use]
pub const fn scenario_mode(game_mode: i32) -> bool {
    matches!(game_mode, 3 | 4 | 5 | 15)
}

pub(crate) fn victim_of(
    u: &Unit,
    master: &Master,
    attributes: &[f32],
    p: usize,
    terrain: Option<&crate::terrain::Terrain>,
    mode_680: i32,
) -> Victim {
    let at = |i: usize| attributes.get(i).copied().unwrap_or(0.0);
    let kind = master.kind;
    let attack_elevation = if 212 < attributes.len() {
        [attributes[211], attributes[212]]
    } else {
        [0.0; 2]
    };
    let defense = if master.terrain_bonus < 0 || u.life.inside.is_some() {
        Some(1.0)
    } else {
        terrain.and_then(|t| {
            t.defense(
                master.terrain_bonus,
                u.position[0] as i32,
                u.position[1] as i32,
            )
        })
    };
    let tasks = |kind: i16| master.tasks.iter().any(|t| t.action_type == kind);
    Victim {
        id: u.id,
        hp: u.life.hp,
        amount: u.life.amount,
        state: u.life.state,
        position: u.position,
        kind,
        class: master.class,
        master_id: master.id,
        radius: master.radius,
        height: master.height,
        storage: master.storage.first().map_or(0.0, |s| s.1),
        capacity: master.capacity,
        blast_defense: master.blast_defense,
        fog_visibility: master.fog_visibility,
        goto_sleep: u.life.goto_sleep,
        decay: u.life.master.map_or(0.0, |m| m.decay),
        player: p,
        elevation: [at(272), at(273)],
        attack_elevation,
        allied: [false; 2],
        uncertain: false,
        inside: u.life.inside.is_some(),
        source: crate::damage::Attacker {
            id: u.id,
            kind,
            class: master.class,
            master_id: master.id,
            ability: master.combat_ability,
            missile: master.missile,
            weapon_range: master.weapon_range,
            area_effect: master.area_effect,
            blast_level: master.blast_level,
            area_damage: master.area_damage,
            charge_type: master.charge_type,
            max_charge: master.max_charge,
            charge_target: master.charge_target,
            special_attack: master.special_attack,
            charge: u.charge,
            charge_hit: u.charge_hit,
            attack_powerup: u.attack_powerup,
            mode_680,
            bonus_tasks: tasks(0xa0),
            player: p,
            kill_bonus_tasks: tasks(0x9a),
            fired_from: u.missile.as_ref().map_or(-1, |m| m.fired_from),
        },
        weapons: master.weapons.clone(),
        // a combat object's slot 0xf0 is FUN_141082530 (types 50 and up)
        armored: (50 <= kind).then(|| crate::damage::Armored {
            armors: master.armors.clone(),
            base_armor: master.base_armor,
            terrain_bonus: master.terrain_bonus,
            resistance: master.resistance,
            ability: master.combat_ability,
            reflection: master.reflection,
            invulnerability: master.invulnerability,
            special_attack: master.special_attack,
            master_hp: master.hit_points,
            friendly_fire: master.friendly_fire,
            special_graphic: master.special_graphic,
            terrain: defense,
            armor_powerups: u.armor_powerups,
            armor_removed: u.armor_removed,
            hp_powerups: u.hp_powerups,
            kill_tasks: master
                .tasks
                .iter()
                .any(|t| t.action_type == 0xa1 && 1e-7 < t.work_val1),
            busy: u.top_action_kind().is_some() || u.unmodelled_action.is_some(),
            unit_ai: u.ai.is_some(),
            alert_idle: None,
            facing: None,
        }),
        damaged: None,
        killed: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::{Class, Lifecycle, MasterData};

    fn player(objects: Vec<Option<UnitAi>>) -> Player {
        Player {
            objects: objects
                .into_iter()
                .map(|ai| Unit {
                    ai,
                    ..Unit::default()
                })
                .collect(),
            ..Player::default()
        }
    }

    fn building() -> UnitAi {
        UnitAi {
            ai_type: 17,
            current_order: -1,
            current_action: -1,
            current_target: -1,
            adjusted_idle_timeout: 3000,
            look_around_timeout: 1000,
            retarget_timer: 1,
            ..UnitAi::default()
        }
    }

    #[test]
    fn the_turn_rotates_and_only_the_turn_player_updates() {
        let mut t = Turns::new(
            0,
            29910,
            1,
            vec![0, 0, 0],
            vec![
                player(vec![Some(building())]),
                player(vec![Some(building())]),
                player(vec![]),
            ],
        );
        t.tick(10);
        assert_eq!(t.player_turn, 2);
        assert_eq!(t.player_time, [10, 0, 10]);
        assert_eq!(
            t.players[0].objects[0]
                .ai
                .as_ref()
                .unwrap()
                .last_retarget_time,
            0
        );
        assert_eq!(
            t.players[1].objects[0]
                .ai
                .as_ref()
                .unwrap()
                .last_retarget_time,
            10
        );
        t.tick(10);
        t.tick(10);
        // turns 2 (no objects), then 0; back at player 1
        assert_eq!(t.player_turn, 1);
        assert_eq!(t.player_time, [0, 20, 10]);
        assert_eq!(
            t.players[0].objects[0]
                .ai
                .as_ref()
                .unwrap()
                .last_retarget_time,
            30
        );
    }

    #[test]
    fn the_frame_seed_chains_through_two_world_draws() {
        let mut t = Turns::new(0, 29910, 0, vec![0], vec![player(vec![])]);
        t.tick(10);
        // SeedAll(29910); s1 = World.rand15 = 19550 (see rng); SeedAll(19550); World.rand15
        let mut s = Streams::default();
        s.seed_all(19550);
        assert_eq!(t.frame_seed, s.get(Stream::World).rand15());
    }

    #[test]
    fn artifacts_of_every_player_start_updating_in_the_second_tick() {
        let animal = |gaia| Unit {
            artifact: Some(Artifact::new(gaia)),
            ..Unit::default()
        };
        let players = vec![
            Player {
                objects: vec![animal(true)],
                ..Player::default()
            },
            Player {
                objects: vec![animal(false)],
                ..Player::default()
            },
        ];
        let mut t = Turns::new(0, 1, 0, vec![0, 0], players);
        let state = |t: &Turns, p: usize| t.players[p].objects[0].artifact.unwrap().state;
        t.tick(50);
        assert_eq!((state(&t, 0), state(&t, 1)), (6, 6));
        t.tick(50);
        assert_eq!((state(&t, 0), state(&t, 1)), (10, 10));
    }

    #[test]
    fn an_unsupported_unit_stops_and_is_recorded() {
        let mut odd = building();
        // a unit-AI type the slice does not model
        odd.ai_type = 99;
        let mut t = Turns::new(0, 1, 0, vec![0], vec![player(vec![Some(odd)])]);
        t.tick(10);
        t.tick(10);
        assert_eq!(
            t.players[0].unsupported,
            Some((1, 0, Unsupported("unit-AI type not modelled")))
        );
    }

    fn dead_building() -> Unit {
        Unit {
            id: 1,
            ai: Some(building()),
            life: Lifecycle {
                class: Class::Animated,
                hp: 0.0,
                master: Some(MasterData::default()),
                ..Lifecycle::default()
            },
            ..Unit::default()
        }
    }

    /// A live building whose retarget timer falls due in the fourth tick of 10 ms (world time 40).
    fn live(id: i32) -> Unit {
        Unit {
            id,
            ai: Some(UnitAi {
                retarget_timer: 40,
                ..building()
            }),
            ..Unit::default()
        }
    }

    fn ids(t: &Turns) -> Vec<i32> {
        t.players[0].objects.iter().map(|o| o.id).collect()
    }

    fn retarget(t: &Turns) -> Vec<u32> {
        t.players[0]
            .objects
            .iter()
            .map(|o| o.ai.as_ref().unwrap().retarget_timer)
            .collect()
    }

    #[test]
    fn a_removed_object_is_replaced_by_the_last_one_which_updates_in_the_same_tick() {
        // RGE_Object_List::update: the dead building leaves in its fourth update; the last object
        // takes its slot and updates there, so it draws before the object behind the slot
        let mut t = Turns::new(
            0,
            29910,
            0,
            vec![0],
            vec![Player {
                objects: vec![dead_building(), live(2), live(3)],
                ..Player::default()
            }],
        );
        for _ in 0..4 {
            t.tick(10);
        }
        assert_eq!(ids(&t), [3, 2]);
        // the same ticks with the two live buildings in that order from the start: their timers
        // fall due in the fourth tick, so its draws depend only on the order
        let mut reordered = Turns::new(
            0,
            29910,
            0,
            vec![0],
            vec![Player {
                objects: vec![live(3), live(2)],
                ..Player::default()
            }],
        );
        for _ in 0..4 {
            reordered.tick(10);
        }
        assert_eq!(retarget(&t), retarget(&reordered));
    }

    #[test]
    fn a_dead_objects_unit_ai_draws_nothing() {
        let one = |objects| {
            let mut t = Turns::new(
                0,
                29910,
                0,
                vec![0],
                vec![Player {
                    objects,
                    ..Player::default()
                }],
            );
            t.tick(10);
            t
        };
        let with_dead = one(vec![dead_building(), live(2)]);
        let alone = one(vec![live(2)]);
        assert_eq!(with_dead.players[0].objects[0].life.state, 5);
        // its full update adds the world's dt to the time since the last enemy sighting, and
        // returns before anything else
        let dead_ai = dead_building().ai.map(|ai| UnitAi {
            time_since_enemy: ai.time_since_enemy + 0.01,
            ..ai
        });
        assert_eq!(
            with_dead.players[0].objects[0].ai, dead_ai,
            "the dead building's timers stay"
        );
        assert_eq!(
            with_dead.players[0].objects[1].ai,
            alone.players[0].objects[0].ai
        );
    }

    /// The masters of MarvinBlue/426048731's player 2 that its BUILD at tick 679 uses, shortened
    /// from build 158041's save and `.dat`.
    fn builder_masters() -> Masters {
        use crate::masters::{Master, Masters, Task};
        let task = |action_type, object_group| Task {
            action_type,
            object_group,
            ..Task::default()
        };
        let villager = |id, group, tasks: Vec<Task>| Master {
            id,
            kind: 70,
            class: 4,
            task_by_group: group,
            radius: [0.2, 0.2],
            tall: true,
            obstruction_type: 0,
            buildable_on: 0,
            tasks,
            cost: vec![],
            obstruction_class: 0,
            object_flags: 0,
            combat_level: 0,
            work_rate: 1.0,
            hit_points: 25,
            build_time: 0,
            ..Master::default()
        };
        let base = || vec![task(7, -1), task(3, 3)];
        let builder = || vec![task(101, -1), task(3, 3)];
        Masters::new([
            villager(83, 1, base()),
            villager(118, 1, builder()),
            villager(212, 2, builder()),
            villager(293, 2, base()),
            Master {
                id: 70,
                kind: 80,
                class: 3,
                task_by_group: 0,
                radius: [1.0, 1.0],
                tall: true,
                obstruction_type: 0,
                buildable_on: 0,
                tasks: vec![],
                cost: vec![(1, 25, true)],
                obstruction_class: 2,
                object_flags: 0,
                combat_level: 0,
                work_rate: 0.0,
                hit_points: 550,
                build_time: 25,
                // a house's clearance, which BUILD's placement test reads
                clearance: [1.0, 1.0],
                ..Master::default()
            },
        ])
    }

    /// A flat map of terrain 0 that the first terrain restriction lets anything stand on, every
    /// tile seen by every player: BUILD's placement test passes anywhere free.
    fn placeable(t: &mut Turns) {
        let (w, h) = (120, 120);
        let n = (w * h) as usize;
        t.spatial.map = [w, h];
        let mut terrain = crate::terrain::Terrain::new(
            w,
            h,
            vec![(0, 255, -1); n],
            vec![
                crate::terrain::TerrainKind {
                    shown: true,
                    flags: 0,
                    forest: false,
                };
                1
            ],
            vec![vec![1.0]],
        );
        terrain.elevations = vec![0; n];
        t.spatial.terrain = Some(terrain);
        let players = t.players.len();
        let seen = (0..players).fold(0u64, |b, p| b | (1 << p) | (1 << (32 + p)));
        t.spatial.sight = Some(crate::visibility::Visibility::new(
            w,
            h,
            (0..players)
                .map(|_| crate::visibility::VisibleMap {
                    counts: vec![0; n],
                    explored: 0,
                })
                .collect(),
            vec![seen; n],
            Vec::new(),
            0,
            vec![false; players],
        ));
    }

    #[test]
    fn a_build_pays_the_cost_and_tasks_the_builders_of_its_own_player() {
        let villager = |id, master, x, y| Unit {
            id,
            position: [x, y, 0.0],
            life: Lifecycle {
                class: Class::Animated,
                state: 2,
                hp: 25.0,
                master_id: master,
                ..Lifecycle::default()
            },
            // a villager's unit AI (type 8) without an order or action
            ai: Some(UnitAi {
                ai_type: 8,
                current_order: -1,
                current_action: -1,
                current_target: -1,
                ..UnitAi::default()
            }),
            ..Unit::default()
        };
        let mut t = Turns::new(
            0,
            29910,
            1,
            vec![0, 0],
            vec![
                Player::default(),
                Player {
                    objects: vec![
                        villager(9175, 83, 30.0, 89.0),
                        villager(9181, 293, 29.0, 99.0),
                    ],
                    resources: [200.0, 100.0, 200.0, 100.0],
                    ..Player::default()
                },
            ],
        );
        t.next_object_id = 10028;
        t.masters = vec![Masters::default(), builder_masters()];
        placeable(&mut t);
        t.tick_with(
            50,
            &[Command::Build {
                player: 1,
                master: 70,
                unit_ai: Ok(None),
                position: [30.0, 88.0, 0.0],
                builders: vec![9175, 9181],
                queued: false,
                commits: false,
            }],
        );
        let p = &t.players[1];
        assert_eq!(p.resources, [200.0, 75.0, 200.0, 100.0]);
        let foundation = p.objects.last().unwrap();
        assert_eq!(
            (
                foundation.id,
                foundation.life.master_id,
                foundation.position
            ),
            (10028, 70, [30.0, 88.0, 0.0])
        );
        let builder = |i: usize| {
            (
                p.objects[i].life.master_id,
                p.objects[i].build.map(|b| b.state),
            )
        };
        // the builder at the foundation works in its first update; the other asks for a path, for
        // which these villagers lack their movement state
        assert_eq!((builder(0), builder(1)), ((118, Some(6)), (212, Some(4))));
        // each idle villager's town-bell record: no work, at its own place
        assert_eq!(
            p.objects[1].town_bell.work,
            crate::town_bell::BackToWork {
                point: [29.0, 99.0],
                ..crate::town_bell::BackToWork::NONE
            }
        );
        assert_eq!(
            p.unsupported,
            Some((1, 1, Unsupported("a walker without movement state")))
        );
    }

    #[test]
    fn a_build_creates_a_foundation_whose_unit_ai_draws_before_the_turn_players_units() {
        // MarvinBlue/426048731 tick 281: player 1's BUILD draws three AI values in the command
        // phase, so player 2's units, on their turn, get the draws after them
        let world = |objects: Vec<Unit>| {
            let mut t = Turns::new(
                0,
                29910,
                1,
                vec![0, 0],
                vec![
                    Player {
                        resources: [0.0, 100.0, 0.0, 0.0],
                        ..Player::default()
                    },
                    Player {
                        objects,
                        ..Player::default()
                    },
                ],
            );
            t.next_object_id = 100;
            t.masters = vec![builder_masters(), builder_masters()];
            placeable(&mut t);
            t
        };
        let mut built = world(vec![live(2)]);
        built.tick_with(
            40,
            &[Command::Build {
                player: 0,
                master: 70,
                unit_ai: Ok(Some(17)),
                position: [10.0, 10.0, 0.0],
                builders: vec![],
                queued: false,
                commits: false,
            }],
        );
        let foundation = &built.players[0].objects[0];
        assert_eq!(
            (
                foundation.id,
                foundation.life.state,
                foundation.life.master_id
            ),
            (100, 0, 70)
        );
        assert_eq!(built.next_object_id, 101);
        // the same tick without the BUILD, with three AI draws taken by hand after the reseed
        let mut by_hand = world(vec![live(2)]);
        let mut streams = Streams::default();
        streams.seed_all(29910);
        let ai = streams.get(Stream::Ai);
        let first = UnitAi::constructed(17, [10.0, 10.0, 0.0], ai).unwrap();
        assert_eq!(foundation.ai, Some(first));
        let next = 750 + ai.rand15() % 500;
        by_hand.tick(40);
        assert_eq!(
            built.players[1].objects[0]
                .ai
                .as_ref()
                .unwrap()
                .retarget_timer,
            next
        );
        assert_ne!(
            by_hand.players[1].objects[0]
                .ai
                .as_ref()
                .unwrap()
                .retarget_timer,
            next
        );
    }

    #[test]
    fn an_order_resets_the_idle_timeouts_of_its_units_wherever_they_are() {
        // MOVE: one AI draw per listed unit with a unit AI, in list order, before the turn player's
        // units update; the ordering player's lane stops
        let ordered = |id| Unit {
            id,
            ai: Some(UnitAi {
                idle_timeout: 3000,
                adjusted_idle_timeout: 3000,
                ..building()
            }),
            ..Unit::default()
        };
        let mut t = Turns::new(
            0,
            29910,
            1,
            vec![0, 0],
            vec![
                Player {
                    objects: vec![ordered(7), ordered(8)],
                    ..Player::default()
                },
                Player {
                    objects: vec![live(2)],
                    ..Player::default()
                },
            ],
        );
        t.tick_with(
            40,
            &[Command::Order {
                player: 0,
                units: vec![8, 99, 7],
                order: None,
            }],
        );
        let mut streams = Streams::default();
        streams.seed_all(29910);
        let ai = streams.get(Stream::Ai);
        let (eight, seven) = (ai.rand15() % 1500 + 2250, ai.rand15() % 1500 + 2250);
        let next = 750 + ai.rand15() % 500;
        let timeout = |t: &Turns, i: usize| {
            t.players[0].objects[i]
                .ai
                .as_ref()
                .unwrap()
                .adjusted_idle_timeout
        };
        assert_eq!((timeout(&t, 1), timeout(&t, 0)), (eight, seven));
        assert_eq!(
            t.players[1].objects[0].ai.as_ref().unwrap().retarget_timer,
            next
        );
        assert!(t.players[0].unsupported.is_some());
    }

    #[test]
    fn an_unmodelled_action_stops_its_player_unless_the_object_dies_first() {
        let task = Unsupported("a default task the slice does not model");
        let with_task = |life| Unit {
            id: 5,
            life,
            unmodelled_action: Some(task),
            ..Unit::default()
        };
        let run = |unit| {
            let mut t = Turns::new(0, 1, 0, vec![0], vec![player(vec![])]);
            t.players[0].objects.push(unit);
            t.tick(10);
            t.players[0].unsupported
        };
        assert_eq!(run(with_task(Lifecycle::default())), Some((1, 0, task)));
        let dying = Lifecycle {
            class: Class::Animated,
            hp: 0.0,
            master: Some(MasterData::default()),
            ..Lifecycle::default()
        };
        assert_eq!(run(with_task(dying)), None);
    }

    fn town_centre(id: i32) -> Unit {
        Unit {
            id,
            ..Unit::default()
        }
    }

    fn queue(building: i32, unit: i16, amount: u16) -> Command {
        Command::Queue {
            player: 0,
            building,
            unit,
            amount,
            train_time: 25.0,
            rate: 1.0,
            cost: vec![(0, 46, true), (4, 1, false)],
        }
    }

    #[test]
    fn the_forced_replay_takes_the_game_s_state_for_what_froze() {
        // a frozen unit takes the game's (and runs again), a frozen unit the game no longer
        // holds goes, a unit that ran keeps the slice's, a stopped player takes its resources
        let mut t = one_player(vec![live(7), live(8), live(9)], [0.0; 4]);
        t.players[0].objects[0].stopped = true;
        t.players[0].objects[2].stopped = true;
        t.players[0].stopped = false;
        let game_unit = |id, timer| Unit {
            id,
            ai: Some(UnitAi {
                retarget_timer: timer,
                ..building()
            }),
            ..Unit::default()
        };
        let game = GameState {
            frame_seed: 77,
            players: vec![vec![game_unit(7, 900), game_unit(8, 901)]],
            resources: vec![[1.0, 2.0, 3.0, 4.0]],
            ..GameState::default()
        };
        assert_eq!(t.force(&game), vec![]);
        assert_eq!(t.frame_seed, 77);
        assert_eq!(ids(&t), vec![7, 8]);
        assert_eq!(retarget(&t), vec![900, 40]);
        assert!(t.players[0].objects.iter().all(|u| !u.stopped));
        // a stopped player runs again with the game's resources; what differed is forced too
        t.players[0].stopped = true;
        let game = GameState {
            resync: vec![8],
            ..game
        };
        let _ = t.force(&game);
        assert_eq!(
            (t.players[0].stopped, t.players[0].resources),
            (false, [1.0, 2.0, 3.0, 4.0])
        );
        assert_eq!(retarget(&t), vec![900, 901]);
        // a game object the slice never made is named, not added
        let made = GameState {
            players: vec![vec![game_unit(7, 900), game_unit(8, 901), game_unit(10, 5)]],
            ..GameState::default()
        };
        assert_eq!(
            t.force(&made),
            vec![(
                0,
                10,
                Unsupported("an object the game made and the slice did not")
            )]
        );
    }

    /// A player with the villagers' masters (a population of one each) and room for more.
    fn one_player(objects: Vec<Unit>, resources: [f32; 4]) -> Turns {
        let mut attributes = vec![0.0; 250];
        (attributes[4], attributes[11], attributes[32]) = (5.0, 4.0, 10.0);
        let mut t = Turns::new(
            0,
            29910,
            0,
            vec![0],
            vec![Player {
                objects,
                resources,
                attributes,
                ..Player::default()
            }],
        );
        let villager = |id| Master {
            id,
            class: 4,
            storage: vec![(4, -1.0, 2), (11, 1.0, 2)],
            ..Master::default()
        };
        t.masters = vec![Masters::new([villager(83), villager(293)])];
        t
    }

    #[test]
    fn a_queue_pays_its_cost_and_the_building_starts_making() {
        let mut t = one_player(vec![town_centre(9)], [200.0; 4]);
        t.tick_with(104, &[queue(9, 83, 1)]);
        let p = &t.players[0];
        assert_eq!(p.resources, [154.0, 200.0, 200.0, 200.0]);
        let production = p.objects[0].production.as_ref().unwrap();
        // the make action works in the tick it is created in
        assert_eq!(production.progress, 0.104);
        assert_eq!(production.queue.len(), 1);
        assert!(p.unsupported.is_none());
    }

    #[test]
    fn a_queued_villager_is_male_or_female_by_an_action_stream_draw() {
        let mut t = one_player(vec![town_centre(9)], [200.0; 4]);
        t.tick_with(104, &[queue(9, 83, 1)]);
        let mut streams = Streams::default();
        streams.seed_all(29910);
        let r = streams.get(Stream::Action).raw64();
        let expected = if r.wrapping_mul(100) % 100 < 50 {
            83
        } else {
            293
        };
        assert_eq!(
            t.players[0].objects[0].production.as_ref().unwrap().queue[0].master,
            expected
        );
    }

    #[test]
    fn a_queue_the_player_cannot_pay_queues_nothing_and_a_missing_building_stops_the_player() {
        // as at 2730: too little food for the villager (46 here): no entry, no cost, the draw
        // stays
        let mut poor = one_player(vec![town_centre(9)], [40.0, 0.0, 0.0, 0.0]);
        poor.tick_with(104, &[queue(9, 83, 1)]);
        assert!(poor.players[0].unsupported.is_none());
        assert_eq!(poor.players[0].resources[0], 40.0);
        let tc = &poor.players[0].objects[0];
        assert!(tc.production.as_ref().is_none_or(|p| p.queue.is_empty()));
        // the male or female draw as a paid queue's
        let mut paid = one_player(vec![town_centre(9)], [46.0, 0.0, 0.0, 0.0]);
        paid.tick_with(104, &[queue(9, 83, 1)]);
        assert_eq!(paid.players[0].resources[0], 0.0);
        assert_eq!(
            poor.streams.get(Stream::Action).draws,
            paid.streams.get(Stream::Action).draws
        );
        let mut missing = one_player(vec![town_centre(9)], [200.0; 4]);
        missing.tick_with(104, &[queue(8, 83, 1)]);
        assert!(missing.players[0].unsupported.is_some());
    }

    #[test]
    fn an_unmodelled_command_stops_its_player() {
        let mut t = one_player(vec![town_centre(9)], [200.0; 4]);
        let why = Unsupported("QUEUE at two buildings");
        t.tick_with(104, &[Command::Unmodelled { player: 0, why }]);
        assert_eq!(t.players[0].unsupported, Some((1, 0, why)));
    }

    /// MarvinBlue/426048731's update 1250: 10029's second hit kills its sheep 9520 (2 hit points,
    /// 12 damage): it has none, Gaia takes it (its list, its storage, its unit AI's notifies),
    /// and the players count the kill (300 + 1 and 325 + 1; a sheep costs nothing).
    #[test]
    fn a_killed_sheep_goes_to_gaia() {
        let sheep = Unit {
            id: 9520,
            life: Lifecycle {
                master_id: 705,
                hp: 2.0,
                ..Lifecycle::default()
            },
            ai: Some(UnitAi::default()),
            ..Unit::default()
        };
        let mut t = Turns::new(
            0,
            1,
            0,
            vec![0, 0],
            vec![
                Player {
                    attributes: vec![0.0; 500],
                    ..Player::default()
                },
                Player {
                    objects: vec![town_centre(10029), sheep],
                    attributes: vec![0.0; 500],
                    ..Player::default()
                },
            ],
        );
        let master = Master {
            id: 705,
            class: 58,
            storage: vec![(16, 100.0, 1)],
            ..Master::default()
        };
        t.masters = vec![Masters::new([master.clone()]), Masters::new([master])];
        t.spatial.insert(
            9520,
            Placed {
                position: [107.0, 48.0, 0.0],
                radius: [0.35, 0.35],
                class: 58,
                player: 1,
                master: 705,
                obstruction_type: 0,
                buildable_on: 0,
                tall: true,
                speed: 0.0,
                unknown: None,
                obstruction: None,
            },
        );
        let mut v = t.victim(9520, 1).unwrap();
        v.hp = 0.0;
        v.damaged = Some((10029, [10029, -10, 14]));
        v.killed = Some(crate::damage::Kill {
            attacker: 10029,
            attacker_player: 1,
            to_gaia: true,
        });
        t.set_victim(&v).unwrap();
        assert_eq!(t.players[1].objects.len(), 1);
        let gone = &t.players[0].objects[0];
        assert_eq!((gone.id, gone.life.hp), (9520, 0.0));
        assert_eq!(t.spatial.get(9520).map(|p| p.player), Some(0));
        assert_eq!(t.players[1].attributes[301], 1.0);
        assert_eq!(t.players[1].attributes[326], 1.0);
        assert_eq!(t.players[0].attributes[16], 100.0);
    }

    /// MarvinBlue/426048731's 1086: SET GATHER POINT on TC 9149 at (107.2451, 46.075863) on the
    /// sheep 9520 (master 705).
    #[test]
    fn set_gather_point_takes_the_point_and_target_or_clears_it() {
        let mut tc = town_centre(9149);
        tc.life.master_id = 109;
        let mut t = one_player(vec![tc], [200.0; 4]);
        t.masters = vec![Masters::new([Master {
            id: 109,
            garrison: 15,
            ..Master::default()
        }])];
        let set = |target, point, own| Command::GatherPoint {
            player: 0,
            ids: vec![9149],
            point,
            target,
            target_master: 705,
            own,
        };
        t.tick_with(25, &[set(9520, [107.2451, 46.075_863], false)]);
        assert_eq!(t.players[0].unsupported, None);
        assert_eq!(
            t.players[0].objects[0].gather_point,
            Some(GatherPoint {
                point: [107.2451, 46.075_863, 0.0],
                target: 9520,
                target_master: 705,
            })
        );
        t.tick_with(25, &[set(-1, [-1.0, -1.0], false)]);
        assert_eq!(t.players[0].objects[0].gather_point, None);
        t.tick_with(25, &[set(-1, [-1.0, -1.0], true)]);
        assert_eq!(
            t.players[0].objects[0].gather_point.map(|g| g.target),
            Some(9149)
        );
    }

    #[test]
    fn a_unit_due_to_spawn_stops_the_player() {
        let mut t = one_player(vec![town_centre(9)], [200.0; 4]);
        let quick = Command::Queue {
            player: 0,
            building: 9,
            unit: 83,
            amount: 1,
            train_time: 2.0,
            rate: 1.0,
            cost: vec![],
        };
        t.tick_with(1000, &[quick]);
        // the building makes the make action in its update: one unit in production
        assert_eq!(t.players[0].unsupported, None);
        assert_eq!(t.players[0].attributes[81], 1.0);
        t.tick(1000);
        assert_eq!(
            t.players[0].freezes.first().map(|f| f.2),
            Some(Unsupported("a spawn without the players' sight"))
        );
    }
}
