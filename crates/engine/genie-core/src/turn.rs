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
    /// `+0x889`: its waits as a builder for an occupied footprint (not saved; 0 at a load).
    pub footprint_waits: u8,
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
    /// `mDeletionEnabled` (`+0x687`, 1 from its constructor) is 0: no delete takes it.
    pub undeletable: bool,
    /// `capture_flag` (`+0x924`).
    pub capture_flag: u8,
    /// An annex's main building (`+0xbb8`, the save's linked owner).
    pub annex_of: Option<i32>,
    /// Its unit group, if any ([`crate::group::GroupLink`]).
    pub group: crate::group::GroupLink,
    /// `mHitpointsPowerup` and `mHitpointsMaxPowerup` (`+0xa38`, `+0xa3c`), which the task finder
    /// adds to the hit points.
    pub hp_powerups: [f32; 2],
    /// A villager's Hunt action, its top action.
    pub hunt: Option<HuntAction>,
    /// A forager's Gather action, its top action.
    pub gather: Option<crate::gather::GatherAction>,
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
    /// A building's `last_garrison_time` (`+0xbe0`): the world time of its last whole second,
    /// on which its garrison heals ([`Unit::building_seconds`]).
    pub garrison_time: u32,
    /// A building's `relic_count` and `specific_relic_count` (`+0xc20`, `+0xc24`).
    pub relics: [i32; 2],
    /// A combat object's `raider_build_countdown` (`+0x9a0`): the seconds it may stay inside a
    /// building.
    pub raider_countdown: i32,
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
    /// `+0x78c`: which of its master's standing sprites its idle swap last showed (1 the second,
    /// [`Master::standing_sprite_2`]); not saved, 0 at a load and from the constructor.
    pub idle_variant: i32,
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
    /// Its master's default task is a Make, which starts once its action list is empty in
    /// state 2 (a farm or fish trap before its first update as a built building), with the task
    /// its master holds then.
    pub default_make: bool,
    /// A farm's or fish trap's Make action, its top action ([`crate::make`]).
    pub make: Option<crate::make::MakeAction>,
    /// A villager's wait for a foundation (`TRIBE_Action_Wait_Solidify`), its only action
    /// ([`crate::wait_solidify`]).
    pub wait_solidify: Option<crate::wait_solidify::WaitSolidify>,
    /// The slice could not model one of its updates: it freezes (its place is known only within
    /// its speed since) while the rest of its player's objects go on.
    pub stopped: bool,
    /// `mROOAlreadyCalled` (`+0x686`, saved): its obstruction record went (its death's
    /// `FUN_14121cc40`, a footprint clearing's `FUN_141189670`, a construction start's clearing
    /// `FUN_1411cd0d0`, a stack unit's completion); later ones leave it alone.
    pub roo_called: bool,
    /// A building's `+0xcd0`: its constructor found mangrove or rice terrain at its place
    /// (`FUN_141188db0`), which master 1889's completion reads.
    pub rice: bool,
    /// A building's own store (`+0xae0`, `FUN_1411abad0`'s map by attribute): what its storage
    /// entries of mode 16 put in at its completion and take out at its death
    /// (`FUN_1411e6580`).
    pub store: Vec<(i16, f32)>,
}

/// What every building holds of a gate: `mGateLocked` (`+0xc7c`; a lock also sets the
/// building's last move time, [`Mover::last_move`], slot 0x670, `FUN_1411dba30`), `mFirstUpdate`
/// (`+0xc80`) and `mCloseTimer` (`+0xc84`) ([`Unit::gate_update`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Gate {
    /// `mGateLocked`: the gate is locked.
    pub locked: bool,
    /// `mFirstUpdate`: a new building's is set; a gate's first update in state 2 takes its
    /// annexes' places as its endpoints (`mEndpoint1`, `mEndpoint2`, not held) and clears it.
    pub first_update: bool,
    /// `mCloseTimer`: the seconds an open gate with nothing in it waits before it shuts.
    pub close_timer: f32,
    /// The building has a saved open or shut master (`+0xc58`, the save's
    /// `has_open_closed_master_object`) or its original owner's masters (`mOriginalOwner`,
    /// `+0xc88`): a gate's toggle then takes another master, not modelled. A master of its own
    /// (`+0x6c0`, which a foundation holds until its first builder starts) the slice does not
    /// hold.
    pub other_masters: bool,
}

/// A gate's height (its master's `radius_z`) above this shuts it (`DAT_142eb0c60`); a unit
/// slower than it counts in a gate only when it is a player's.
const GATE_EPSILON: f32 = crate::image::turn::GATE_EPSILON;
/// A gate's query grows its records and its own box by this (`FUN_1410562d0` with
/// `DAT_142eb3630`).
const GATE_GROW: f32 = crate::image::turn::GATE_GROW;
/// The least height of a shut gate its player opens (`FUN_14162f7e0`, `DAT_14330eb8c`).
const GATE_SHUT_HEIGHT: f32 = crate::image::turn::GATE_SHUT_HEIGHT;
/// The seconds an open gate with nothing in it waits before it shuts (`FUN_1411e7730`'s
/// immediate `0x3f000000`).
const GATE_CLOSE_DELAY: f32 = 0.5;
/// The most records a gate's query lists (`FUN_1411d0570`'s list of 10) and the most units it
/// counts (25).
const GATE_QUERY_RECORDS: usize = 10;
const GATE_COUNT_CAP: usize = 25;
/// The world constructor's history period (`FUN_141599e50`'s immediate 0x19 at `world+0x8b8`),
/// until the `.dat`'s `time_slice` replaces it.
pub const HISTORY_PERIOD: i32 = 25;

/// The sprite a hunt's walk shows ([`crate::hunt::walk_sprite`]) from its task's sprites.
fn hunt_walk_sprite(
    hunt: &HuntAction,
    graphics: &std::collections::BTreeMap<i16, Graphic>,
    life: &Lifecycle,
) -> Option<Graphic> {
    let g = |id: i16| (0 <= id).then(|| graphics.get(&id).copied()).flatten();
    let sprites = HuntSprites {
        moving: g(hunt.task.sprites[0]),
        working: g(hunt.task.sprites[2]),
        carrying: g(hunt.task.sprites[3]),
        standing: None,
    };
    crate::hunt::walk_sprite(
        hunt.task.resource,
        life.attribute_type,
        life.amount,
        &sprites,
    )
}

/// A building's annexes among `objects` (`+0xbc0`.., the save's linked object ids), in id order,
/// as the building made them.
pub(crate) fn annexes_of(id: i32, objects: &[Unit]) -> Vec<i32> {
    let mut annexes: Vec<i32> = objects
        .iter()
        .filter(|u| u.annex_of == Some(id))
        .map(|u| u.id)
        .collect();
    annexes.sort_unstable();
    annexes
}

/// The units in gate `gate` (`FUN_1411d0570`): the obstruction manager's query from its first
/// annex to its second (`+0xbc0`, `+0xbc8`) with flags 0x10, the gate and its annexes ignored
/// (`FUN_1410562e0`), every record and the query grown by [`GATE_GROW`] (`FUN_1410562d0`), any
/// kind and player ([`Spatial::walk_hits`]; its list holds 10 records, more are not modelled).
/// Of the objects it lists, those not of the classes 27, 39 and 52, nor of class 3 standing
/// still, and either moving (a top speed of at least [`GATE_EPSILON`]) or not Gaia's count, at
/// most 25; the second answer is how many of them are of a player the gate's counts an ally
/// (`FUN_141663d90` with the ally flag).
pub(crate) fn gate_units(
    spatial: &Spatial,
    gate: i32,
    annexes: &[i32],
) -> Result<(Vec<i32>, usize), Unsupported> {
    let [a, b] = annexes[..] else {
        return Err(Unsupported("a gate without its two annexes"));
    };
    let place = |id| {
        spatial
            .get(id)
            .map(|p| [p.position[0], p.position[1]])
            .ok_or(Unsupported("a gate's annex without a place"))
    };
    let (from, to) = (place(a)?, place(b)?);
    let rect = [
        from[0].min(to[0]),
        from[1].min(to[1]),
        from[0].max(to[0]),
        from[1].max(to[1]),
    ];
    let owner = spatial
        .get(gate)
        .map(|p| p.player)
        .ok_or(Unsupported("a gate without a place"))?;
    spatial.check_known_records([
        rect[0] - GATE_GROW,
        rect[1] - GATE_GROW,
        rect[2] + GATE_GROW,
        rect[3] + GATE_GROW,
    ])?;
    let hits = spatial.walk_hits(from, to, rect, GATE_GROW, 1.0, Some(owner), &[gate, a, b])?;
    if GATE_QUERY_RECORDS < hits.len() {
        return Err(Unsupported("a gate's query of more than 10 records"));
    }
    let (mut units, mut allied) = (Vec::new(), 0);
    for id in hits {
        if GATE_COUNT_CAP <= units.len() {
            break;
        }
        let Some(p) = spatial.get(id) else {
            continue;
        };
        let counts = !matches!(p.class, 27 | 39 | 52)
            && (p.class != 3 || p.speed != 0.0)
            && (GATE_EPSILON <= p.speed || p.player != 0);
        if counts {
            units.push(id);
            if spatial.allied_with(owner, p.player) {
                allied += 1;
            }
        }
    }
    Ok((units, allied))
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
    /// `BUILD_WALL` (0x69, `FUN_140e7b2e0`): a line of the player's wall foundations and its
    /// builders shared out over them ([`crate::wall`]).
    BuildWall(crate::wall::Wall),
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
    /// `BUILDING_COMMAND_QUEUE` (0x2d, `0x140e64310`): the objects `ids` (at most a human
    /// player's 60) queue a step to `target` (−1 for none) or `point` ([`Turns::command_queue`]).
    CommandQueue {
        /// The player.
        player: usize,
        /// The objects of the packet.
        ids: Vec<i32>,
        /// The object the step is on, −1 for none.
        target: i32,
        /// Its point.
        point: [f32; 2],
    },
    /// `GO_BACK_TO_WORK` (0x80 with one object, `+4`; 0x86 with a list, its count at `+4`, the ids
    /// from `+8`): the player's buildings let their villagers back out to work
    /// ([`Turns::go_back_to_work`]).
    GoBackToWork {
        /// The player.
        player: usize,
        /// Its buildings.
        ids: Vec<i32>,
    },
    /// `TOWN_BELL` (0x7f, `do_command` case 0x7f): the town centre `id` rings its owner's bell
    /// (`activate`) or ends it ([`Turns::town_bell`]).
    TownBell {
        /// The player.
        player: usize,
        /// The town centre.
        id: i32,
        /// Ring (1) or end (0).
        activate: bool,
    },
    /// `CANCEL_BUILD` (0x6a, `do_command` case 0x6a): the player deletes its object `id`
    /// ([`Turns::delete`]).
    Delete {
        /// The player.
        player: usize,
        /// The object.
        id: i32,
    },
    /// FLARE (0x73, `0x140e7bfd0`): every player from 1 below `players` (`+0x10`, at most the
    /// world's) gets a flare at `target`'s place (`+4`, −1 for none) or at `point` (`+8`, `+0xc`)
    /// ([`Turns::flare`]).
    Flare {
        /// The player who sent it.
        player: usize,
        /// The object it is on, −1 for none.
        target: i32,
        /// Its point without a target.
        point: [f32; 2],
        /// The players the command counts (`+0x10`).
        players: u8,
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
    /// STOP (0x01, `FUN_14161e200`): the units `ids` (at most 60, the decoder's) stop
    /// ([`Turns::stop_command`]).
    Stop {
        /// The player whose units stop.
        player: usize,
        /// The units.
        ids: Vec<i32>,
    },
    /// `SEEK_SHELTER` (0x2c, `FUN_140e6a3a0`): the villagers of `ids` (at most 60, the decoder's)
    /// seek shelter in the player's buildings ([`Turns::seek_shelter`]).
    SeekShelter {
        /// The player whose units seek shelter.
        player: usize,
        /// The units.
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
        /// `+0x1d`: a garrison goes through the units' unit AIs (their slot 0x30); a cancel
        /// (action 4) cancels up to five of a kind.
        through_ai: bool,
        /// `+0x1e`: a garrison takes the buildings of the target's master near it.
        nearby: bool,
        /// The order's action (4: a queue's cancel, 8: the drop-off).
        action: u8,
        /// `+0x18`: a cancel's unit index in the queue.
        param: u32,
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
/// The first master type whose slot 0x128 (a work at an object) does something: the action
/// objects' (`FUN_14106c590`) and up; below it `FUN_141071bd0` returns at once.
const STATIC_TYPES_END: u8 = 40;
/// The buildings' paid costs (attribute 247), which the combat setup adds for a building with a
/// train location (`FUN_14121df60`).
const BUILDING_COST: i16 = 247;
/// The masters whose restrictions the world's load gives planners (`World::load_world`'s two
/// calls of `FUN_140f7d310`: Gaia's entries `+0x298` and `+0x10b8`).
const PLANNED_MASTERS: [i16; 2] = [83, 535];

/// The masters whose death lowers a count of their owner's (`FUN_1411dbb70`, raw `0x6104`,
/// `0x61b0`, `0x60ee`): the monastery's ([`MONASTERY`]) in attribute 52, the wonder's in 42,
/// the castle's in 134.
const WONDER: i16 = 276;
const CASTLE: i16 = 82;
const MONASTERIES: i16 = 52;
const WONDERS: i16 = 42;
const CASTLES: i16 = 134;
/// The foundation terrain a building's death covers with terrain 1 (`FUN_1411dbb70`: `0x1c`).
const RESTORED_FOUNDATION: i16 = 28;

/// What a built building's death leaves its owner's update to do after it
/// ([`Unit::building_died`]): the count attribute it lowers, and its master with the master's
/// technology.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BuildingDeath {
    pub(crate) count: Option<i16>,
    pub(crate) tech: Option<(i16, i16)>,
}

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
/// The farm, which BUILD places as the rice farm on mangrove or rice farm terrain.
const FARM: i16 = 50;
const RICE_FARM: i16 = 1187;
const VILLAGER_ANY: i16 = 0x712;
/// The female villager master (`0x125`).
const VILLAGER_FEMALE: i16 = 0x125;
/// The attribute naming a building master that changes the terrain around its buildings
/// (`FUN_1411c39b0`, the player's attribute 255).
const BUILDING_TERRAIN: usize = 255;
/// A building at most this share of its hit points lets its garrison out (`DAT_142eae8d0`).
const EJECT_SHARE: f32 = crate::image::turn::EJECT_SHARE;
/// The monastery (master 104, `0x6104` raw), whose relics give goods every second.
const MONASTERY: i16 = 104;
/// The petard (master 440, `0x6254` raw), which a class-10 searcher passes (`FUN_140c488b0`).
const PETARD: i16 = 440;
/// The villager's class (4), which a building's repair by its garrison counts.
const VILLAGER_CLASS: i16 = 4;
/// The building masters `FUN_140fde180` counts as drop sites besides the drop-site table's:
/// 562, 584 and 621 (`0x232` plus the bits of `0x800000000400001`), 68, 1808, 50, 1187 and 199.
const DROP_SITE_MASTERS: [i16; 8] = [562, 584, 621, 68, 1808, 50, 1187, 199];
/// A building's queue holds at most 15 units (`FUN_1411cbc40`).
const QUEUE_LIMIT: usize = 15;
/// A computer player's QUEUE fills a building's queue to at most 2 (`FUN_141614450`).
const COMPUTER_QUEUE_LIMIT: usize = 2;
/// The game type whose QUEUE trains one unit in the first age (`FUN_141614450`, the settings'
/// `+0x34`).
const ONE_UNIT_GAME: u8 = 0x10;
/// The Hunt's search timer: it tells its unit AI again every 10 seconds (`0xc1200000`, in
/// `0x140ac60e0` and `0x140ac7a00`).
const SEARCH_AGAIN: f32 = -10.0;
/// `FUN_140ac4670`'s first argument when a hunt's target went on the walk to it, or its `MoveTo`
/// failed (`DAT_142eb9430`): the hunt searches within this of its point.
const TREE_SEARCH_WITHIN: f32 = crate::image::turn::TREE_SEARCH_WITHIN;
/// `FUN_140ac4670`'s second argument when a hunt's Attack failed (`DAT_14330ebb8`): the tree
/// found counts within this of the unit.
const TREE_NEAR: f32 = crate::image::hunt::TREE_NEAR;
/// The notify "Action %d has been invalidated or requires a search." (0x202) with the action's
/// value: a Build waiting without work posts it ([`BUILD_ACTION`]), a searching Hunt too.
const SEARCH_NEEDED: i32 = 0x202;
/// A velocity longer than this sets the animation speed (`DAT_142eb0c60`).
const SPEED_EPSILON: f32 = crate::image::turn::SPEED_EPSILON;
/// A building attacks with a firepower above this (`FUN_14120ea30`).
const ARMED_FIREPOWER: f32 = crate::image::turn::ARMED_FIREPOWER;

/// The pasture master whose foundation keeps its snow and whose stack unit depends on the rice
/// terrain under it (1889).
const PASTURE_1889: i16 = 1889;
/// The gate's class (39), whose annexes stay in their owner's watched list.
const GATE: i16 = 39;
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
    pub groups: &'a crate::group::UnitGroups,
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
    /// The world's unit groups, which a walker's long path reads.
    groups: &'a crate::group::UnitGroups,
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
    refund: Vec<(i16, f32)>,
    /// What a built building's death takes from its owner before its combat part's refund: the
    /// counts of the monastery, the wonder and the castle (slot 0xf8, then slot 0xf0 with 0 for
    /// a value below 0), and its master's technology (`+0x98c`) the caller withdraws when no
    /// other building of the master stands ([`Turns::withdraw_tech`]).
    building_death: Option<BuildingDeath>,
    /// The pauses the unit's walk gave units it met ([`Mover::pauses`]).
    pauses: Vec<(i32, f32)>,
    /// The gate's update asked its player to open or shut it ([`Turns::toggle_gate`], the
    /// caller's after the update).
    gate_toggle: bool,
    /// The unit's update stopped before its unit AI for the caller's work order
    /// ([`Unit::ai_part`]); given back, the update runs only that part.
    ai_resume: Option<AiResume>,
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
    /// The annexes a building's `set_state` (`FUN_1411dbb70`) took to its own state (slot 0xa0
    /// on each, before its own parts), the caller's after the update ([`Turns::set_state`]).
    annex_states: Vec<(i32, u8)>,
    /// The objects a builder's footprint clearing deletes (the Build's `set_state(7)`,
    /// `FUN_1415c3620` with `param_6`; their records went already), the caller's after the
    /// update ([`Turns::clear_object`]).
    cleared: Vec<crate::build::Cleared>,
    /// The units on a foundation's footprint its builder's test met (the foundation, them):
    /// the caller sends off those the test would ([`Turns::evacuate`]).
    evacuations: Vec<(i32, Vec<i32>)>,
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
    /// The player shares its gathered resources with its allies (`+0x1426`).
    sharing: bool,
    /// The game's farm setting ([`Turns::farm_stops`]).
    farm_stops: bool,
    /// The game's setting [`Turns::predator_hunts_end`].
    predator_hunts_end: bool,
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
    /// The owner's object tracker, which a completion's first part (`FUN_1411c6bd0`) moves a
    /// building in, and the unit lines its keys read ([`crate::tracker`]).
    tracker: &'a mut crate::tracker::Tracker,
    lines: &'a crate::lines::UnitLines,
    /// The attributes the owner gains from the object's update (a building it completed), which
    /// the caller adds after it.
    attribute_adds: Vec<(i16, f32)>,
    /// The technologies the owner researches from the object's update (a completed building's),
    /// which the caller completes after the attributes ([`Turns::complete_research`]).
    techs_done: Vec<i16>,
    /// The mule carts the object's update completed, whose search for resources the caller runs
    /// after it ([`Turns::cart_search`]).
    carts: Vec<(i32, u8)>,
    /// The buildings the object's update completed with a stack unit or that disappear when
    /// built: the caller makes the stack unit ([`Turns::stack_unit`]) before the technologies
    /// and destroys the building (slot 0xd0) after them.
    completions: Vec<(i32, Option<crate::build::StackUnit>, bool)>,
    /// The work a builder found after its building (slot 0x7e8 from the Build's case 0xdf),
    /// which the caller gives after the update ([`Turns::order_work`]).
    work_order: Option<crate::order::Work>,
    /// The unit AI's order on a target (`0x2db`) the unit took from its queue, which the caller
    /// runs after the update ([`Turns::order_on_target`]), and the full update's tail that reads
    /// what the order gave ([`crate::unit_ai::UnitAi::after_order`]).
    order_on_target: Option<(
        crate::unit_ai::QueuedOrder,
        Option<crate::unit_ai::AfterOrder>,
    )>,
    /// The targets of the unit AI's `HuntObject`s, in order, whose unit part (the stop, then the
    /// hunt command) the caller runs after the update ([`Turns::hunt_command`]).
    hunt_objects: Vec<i32>,
    /// The targets of the unit AI's `GatherObject`s, in order, whose gather command the caller
    /// runs after the update ([`Turns::gather_command`]).
    gather_objects: Vec<i32>,
    /// The foundation of the unit AI's `BuildObject`, which the caller builds after the update
    /// ([`Turns::build_object`]).
    build_object: Option<i32>,
    /// The foundation of the unit's own build command (slot 0x380, a Gather's `set_state(6)` on
    /// a target not built yet), which the caller runs after the update ([`Turns::build_command`]).
    build_command: Option<i32>,
    /// The unit AI's moves off foundations' footprints (slot 0x148), in order, which the caller
    /// runs after the update ([`Turns::move_aside`]).
    move_aside: Vec<AiMove>,
    /// The waits for foundations the unit's slot 0x388 found while its unit AI ran, which the
    /// caller makes the unit's after the update ([`Turns::wait_for_foundation`]).
    waits: Vec<Wait>,
    /// The work a builder gave an annex whose main building takes it (the foundation, the
    /// seconds; [`crate::build::Worked::Main`]), which the caller runs after the update
    /// ([`Turns::work_building`]).
    main_works: Vec<(i32, f32)>,
    /// The owner's diplomacy (`player+0x260`), which the unit's task search reads.
    relations: &'a [u8],
    /// The livestock a hunt sends to a drop site with its herd, which the caller sends after the
    /// update ([`Turns::lead_herd`]).
    leads: Option<(i32, i32)>,
    /// The map's zone maps, which an auto scout's search reads.
    zones: &'a mut crate::zones::Zones,
    /// The other players' lists, as they stand before the unit's update.
    foreign: Foreign<'a>,
}

/// What the unit's slot 0x388 (`FUN_141226fc0`) found while its unit AI ran
/// ([`crate::unit_ai::UnitSide::wait_solidify`]): the foundation, the task its slot 0x820 found
/// on it and the master the search switches the unit to (slot 0x90; None when its own master's
/// list gave the task). The caller makes them the unit's after the update
/// ([`Turns::wait_for_foundation`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Wait {
    pub(crate) target: i32,
    pub(crate) task: crate::masters::Task,
    pub(crate) switch: Option<i16>,
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

    /// Player `q`, unless it is the one whose list is out.
    fn player(&self, q: usize) -> Option<&'a Player> {
        let own = self.before.len();
        match q.cmp(&own) {
            std::cmp::Ordering::Less => self.before.get(q),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => self.after.get(q - own - 1),
        }
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
    /// The world's objects (and the players' scout maps, which an auto scout makes and marks).
    spatial: &'u mut Spatial,
    /// The planners and the mesh, which a reach test walks.
    pathing: &'u mut Pathing,
    /// The world's unit groups, which a reach test's long path reads.
    groups: &'u crate::group::UnitGroups,
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
    /// The objects an action let go (slot 0x180), the caller's.
    releases: &'u mut Vec<(i32, Releaser)>,
    /// The waits for foundations its slot 0x388 found ([`Wait`]), the caller's.
    waits: &'u mut Vec<Wait>,
    /// What its task search reads of its owner: its diplomacy, attributes and civ, and the drop
    /// sites.
    relations: &'u [u8],
    attributes: &'u [f32],
    civ: u8,
    drop_sites: &'u [DropSite],
}

/// What `bestUnitToAttack` reads of the world for the unit whose AI runs
/// ([`crate::unit_ai::AttackPicker`]): the objects it found, by what the world holds of them, and
/// the unit's quick path to them.
struct WorldPicker<'a> {
    /// What the tests read of the unit.
    attacker: &'a crate::unit_ai::Attacker,
    /// What they read of its unit AI; None outside `bestUnitToAttack`.
    seeker: Option<crate::unit_ai::Seeker>,
    /// The unit.
    me: i32,
    /// Its owner.
    player: usize,
    /// The owner's other objects.
    others: &'a [Unit],
    /// The other players' lists.
    foreign: Foreign<'a>,
    /// The world's objects.
    spatial: &'a Spatial,
    /// Every player's masters.
    every: &'a [Masters],
    /// The unit's master's terrain restriction (`+0x606`).
    restriction: i16,
}

impl WorldPicker<'_> {
    /// The object `id` as the attack tests read it, and whether it moves (slot 0x208 above 0).
    fn target(&self, id: i32) -> Result<(crate::unit_ai::Target, bool), Unsupported> {
        let (owner, t) = self
            .foreign
            .find(id)
            .or_else(|| {
                self.others
                    .iter()
                    .find(|u| u.id == id)
                    .map(|u| (self.player, u))
            })
            .ok_or(Unsupported("a found object the slice does not hold"))?;
        let m = self
            .every
            .get(owner)
            .and_then(|ms| ms.get(t.life.master_id))
            .ok_or(Unsupported(
                "a found object of a master the slice does not have",
            ))?;
        let placed = self
            .spatial
            .get_known(id)?
            .ok_or(Unsupported("a found object without a place"))?;
        let capture = self
            .spatial
            .sight
            .as_ref()
            .and_then(|v| v.seer(id))
            .map_or(0, |s| s.capture);
        let target = crate::unit_ai::Target {
            owner,
            class: m.class,
            kind: m.kind,
            master: t.life.master_id,
            combat_level: m.combat_level,
            capture,
            has_ai: t.ai.is_some(),
            inside: t.life.inside.is_some(),
            targeting: !t.untargetable,
            attackable: t.attackable,
            desolid: t.construction.as_ref().is_some_and(|c| c.desolid),
            position: placed.position,
            radius: placed.radius,
        };
        let moving = 0.0 < t.mover.as_ref().map_or(0.0, |m| m.stats.speed);
        Ok((target, moving))
    }
}

impl crate::unit_ai::AttackPicker for WorldPicker<'_> {
    fn candidate(
        &self,
        id: i32,
        list: Option<usize>,
    ) -> Result<Option<crate::unit_ai::AttackStat>, Unsupported> {
        let seeker = self
            .seeker
            .ok_or(Unsupported("an attack test outside bestUnitToAttack"))?;
        let (target, moving) = self.target(id)?;
        let gap = crate::unit_ai::attack_candidate(self.attacker, &seeker, &target, list)?;
        Ok(gap.map(|gap| crate::unit_ai::AttackStat { id, gap, moving }))
    }

    /// The quick path (slot 0x3e0) within the unit's range against the object (slot 0x238), with
    /// its master's terrain mask and radius on x.
    fn choose(&mut self, stats: &[crate::unit_ai::AttackStat]) -> Result<i32, Unsupported> {
        if stats.is_empty() {
            return Ok(-1);
        }
        let range = self.attacker.range_against()?;
        let terrain = self
            .spatial
            .terrain
            .as_ref()
            .ok_or(Unsupported("a quick path without the map's terrain"))?;
        let restriction = usize::try_from(self.restriction)
            .map_err(|_| Unsupported("a quick path of a master without a terrain restriction"))?;
        let mask = terrain.restriction_mask(restriction);
        let (spatial, me, radius) = (self.spatial, self.me, self.attacker.radius[0]);
        crate::unit_ai::choose_attack(stats, &mut |id| {
            crate::hull_path::quick_reach(spatial, me, id, mask, radius, range)
        })
    }
}

impl crate::unit_ai::UnitSide for UnitView<'_> {
    /// Slot 0x278 (`FUN_141158720`, [`Spatial::point_clear`]): the unit, of its radius and its
    /// restriction's terrain classes, could stand at `point`.
    fn can_stand(&mut self, point: [f32; 2]) -> Result<bool, Unsupported> {
        let unit = &*self.unit;
        let mover = unit
            .mover
            .as_ref()
            .ok_or(Unsupported("a stand test of a unit without movement state"))?;
        self.spatial.point_clear(
            unit.id,
            point,
            mover.stats.radius[0],
            mover.stats.restriction_mask,
        )
    }

    /// The base handler's case 0x216: the unit's closest spot beside the foundation `site`
    /// ([`Spatial::spot_off`] with the notify's radii; none without the foundation), its height
    /// and class, its owner, the foundation and its current action's target as orders name them.
    fn off_footprint(
        &mut self,
        site: i32,
        radius: [f32; 2],
    ) -> Result<crate::unit_ai::OffFootprint, Unsupported> {
        let unit = &*self.unit;
        let master = self
            .masters
            .and_then(|m| m.get(unit.life.master_id))
            .ok_or(Unsupported(
                "a unit sent off of a master the slice does not have",
            ))?;
        let (player, others, foreign) = (self.player, self.others, &self.foreign);
        let named = |id: i32| {
            let found = if id == unit.id {
                Some((player, unit))
            } else {
                others
                    .iter()
                    .find(|u| u.id == id)
                    .map(|u| (player, u))
                    .or_else(|| foreign.find(id))
            };
            found.map(|(p, u)| crate::unit_ai::Named {
                id,
                master: i32::from(u.life.master_id),
                owner: p as i32,
                vector: [u.position[0], u.position[2]],
            })
        };
        let spot = match self.spatial.get(site) {
            Some(placed) => {
                let mover = unit
                    .mover
                    .as_ref()
                    .ok_or(Unsupported("a unit sent off without movement state"))?;
                self.spatial.spot_off(
                    unit.position,
                    (unit.id, mover.stats.radius[0]),
                    mover.stats.restriction_mask,
                    (placed.position, radius),
                    false,
                )?
            }
            None => None,
        };
        let target = unit.top_target().filter(|&t| t != -1);
        Ok(crate::unit_ai::OffFootprint {
            spot,
            z: unit.position[2],
            class: master.class,
            computer: self.computer,
            player: player as i32,
            site: (site != -1).then(|| named(site)).flatten(),
            action_target: target.and_then(named),
        })
    }

    /// `FUN_140c1c7c0`: the flood ([`crate::unit_ai::flood_search`]) from the unit's tile over the
    /// larger of its search radius (slot 0x2c8) and its line of sight, truncated, testing each tile
    /// it reaches (`FUN_140c2b950`: the first object of its tile's list, in order, of type 80 or
    /// more in state 0 of the unit's player); −1 for none.
    fn foundation_search(&mut self) -> Result<i32, Unsupported> {
        let unit = &*self.unit;
        let master = self
            .masters
            .and_then(|m| m.get(unit.life.master_id))
            .ok_or(Unsupported("a builder of a master the slice does not have"))?;
        let r = if master.search_radius <= master.line_of_sight {
            master.line_of_sight
        } else {
            master.search_radius
        } as i32;
        let (ux, uy) = (unit.position[0] as i32, unit.position[1] as i32);
        let (player, others, masters, spatial) =
            (self.player, self.others, self.masters, &*self.spatial);
        let foundation_on = |x: i32, y: i32| -> Option<i32> {
            spatial.objects_on(x, y).find_map(|(id, p)| {
                if p.player != player {
                    return None;
                }
                let u = others.iter().find(|u| u.id == id)?;
                let kind = masters.and_then(|m| m.get(u.life.master_id))?.kind;
                (80 <= kind && u.life.state == 0).then_some(id)
            })
        };
        Ok(crate::unit_ai::flood_search((ux, uy), r, foundation_on).unwrap_or(-1))
    }

    fn seen_unit(&mut self, id: i32) -> Result<Option<crate::unit_ai::UnitSeen>, Unsupported> {
        let Some(p) = self.spatial.get(id) else {
            return Ok(None);
        };
        let (owner, u) = self
            .others
            .iter()
            .find(|u| u.id == id)
            .map(|u| (self.player, u))
            .or_else(|| self.foreign.find(id))
            .ok_or(Unsupported("an object the slice does not hold"))?;
        let m = self
            .every
            .get(owner)
            .and_then(|ms| ms.get(u.life.master_id))
            .ok_or(Unsupported("an object of a master the slice does not have"))?;
        let me = self
            .masters
            .and_then(|ms| ms.get(self.unit.life.master_id))
            .ok_or(Unsupported(
                "a unit AI's unit of a master the slice does not have",
            ))?;
        Ok(Some(crate::unit_ai::UnitSeen {
            id,
            class: m.class,
            owner,
            combat_level: m.combat_level,
            state: u.life.state,
            target: u.ai.as_ref().map(|a| a.current_target),
            gap_squared: footprint_gap_squared(self.unit.position, me.radius, p.position, p.radius),
            position: p.position,
        }))
    }

    fn closest_resource(
        &mut self,
        kind: u8,
        cap: i32,
        site: i32,
        current: i32,
    ) -> Result<Option<crate::unit_ai::Gatherable>, Unsupported> {
        let mut w = ResourceWorld {
            player: self.player,
            spatial: self.spatial,
            zones: self.zones,
            foreign: self.foreign,
            every: self.every,
            masters: self.masters,
            plan: Planning {
                pathing: self.pathing,
                now: self.world_time,
                computer: self.computer,
                object: self.object,
                construction_rate: self.construction_rate,
                groups: self.groups,
            },
        };
        let (id, _) = closest_resource_of(
            self.unit,
            &mut w,
            self.others,
            (kind, cap, site),
            (current, 2),
        )?;
        if id == -1 {
            return Ok(None);
        }
        let (owner, u) = self
            .others
            .iter()
            .find(|u| u.id == id)
            .map(|u| (self.player, u))
            .or_else(|| self.foreign.find(id))
            .ok_or(Unsupported("a resource found that the slice does not hold"))?;
        let class = self
            .every
            .get(owner)
            .and_then(|m| m.get(u.life.master_id))
            .ok_or(Unsupported("a resource found of a master the slice lacks"))?
            .class;
        Ok(Some(crate::unit_ai::Gatherable {
            id,
            position: u.position,
            class,
        }))
    }

    fn reaches(&mut self, target: i32, range: f32) -> Result<bool, Unsupported> {
        let mut plan = Planning {
            pathing: self.pathing,
            now: self.world_time,
            computer: self.computer,
            object: self.object,
            construction_rate: self.construction_rate,
            groups: self.groups,
        };
        let unit = &*self.unit;
        let walker = Me {
            id: unit.id,
            position: unit.position,
            group: unit.group.id,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let mover = unit
            .mover
            .as_ref()
            .ok_or(Unsupported("a reach test of a unit that does not move"))?;
        mover.reaches_object(
            &walker,
            self.spatial,
            &mut plan,
            target,
            range,
            unit.life.master_id,
        )
    }

    fn closest(&mut self, search: &Search<'_>) -> Result<crate::unit_ai::Found, Unsupported> {
        let seeker = match search.pick {
            crate::unit_ai::Pick::Attack { seeker, .. } => Some(seeker),
            _ => None,
        };
        let mut picker = WorldPicker {
            attacker: &self.attacker,
            seeker,
            me: self.unit.id,
            player: self.player,
            others: self.others,
            foreign: self.foreign,
            spatial: self.spatial,
            every: self.every,
            restriction: self
                .masters
                .and_then(|m| m.get(self.unit.life.master_id))
                .map_or(-1, |m| m.terrain_restriction),
        };
        let (others, foreign, every) = (self.others, self.foreign, self.every);
        // ClosestObject's reachability check (slot 0x3d0) walks the unit's paths
        let mut plan = Planning {
            pathing: self.pathing,
            now: self.world_time,
            computer: self.computer,
            object: self.object,
            construction_rate: self.construction_rate,
            groups: self.groups,
        };
        let unit = &*self.unit;
        let walker = Me {
            id: unit.id,
            position: unit.position,
            group: unit.group.id,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let (mover, master_id, spatial) =
            (unit.mover.as_ref(), unit.life.master_id, &*self.spatial);
        let mut reach = |id: i32, range: f32| {
            let m = mover.ok_or(Unsupported("a reach test of a unit that does not move"))?;
            m.reaches_object(&walker, spatial, &mut plan, id, range, master_id)
        };
        let moving = |id: i32| herded(others, foreign, id);
        // a class-10 searcher passes master 440 and the masters with object flag 0x10
        let shuns = |p: &Placed| {
            let m = every
                .get(p.player)
                .and_then(|ms| ms.get(p.master))
                .ok_or(Unsupported(
                    "a found object of a master the slice does not have",
                ))?;
            Ok(p.master == PETARD || m.object_flags & 0x10 != 0)
        };
        // a farm's workers (`+0x644`) and its pasture (`+0x790`)
        let farm = |id: i32| {
            others
                .iter()
                .find(|u| u.id == id)
                .or_else(|| foreign.find(id).map(|(_, u)| u))
                .map(|u| (u.workers, u.pasture))
        };
        spatial.search(
            unit.id,
            self.player,
            search,
            &mut picker,
            &mut reach,
            &moving,
            &shuns,
            &farm,
        )
    }

    fn may_attack(
        &mut self,
        target: i32,
        seeker: &crate::unit_ai::Seeker,
    ) -> Result<Option<(i16, [f32; 3])>, Unsupported> {
        let picker = WorldPicker {
            attacker: &self.attacker,
            seeker: Some(*seeker),
            me: self.unit.id,
            player: self.player,
            others: self.others,
            foreign: self.foreign,
            spatial: self.spatial,
            every: self.every,
            restriction: -1,
        };
        let held =
            self.foreign.find(target).is_some() || self.others.iter().any(|u| u.id == target);
        if !held {
            return Ok(None);
        }
        let (t, _) = picker.target(target)?;
        Ok(
            crate::unit_ai::can_attack(&self.attacker, seeker, &t)?
                .then_some((t.class, t.position)),
        )
    }

    /// The attack command (slot 0x338, `FUN_14120cea0`; classes 51 and 54 are not modelled) ->
    /// `FUN_141081a80`: a target the world does not hold fails; `+0x648` takes the world's time
    /// (not held); the unit's own Attack on the target answers true unless forced (its slot 0x48
    /// then, not modelled); a villager's master switch (slot 0x90) is not modelled. Then the new
    /// Attack ([`crate::attack::Attack::command`]): the constructor takes its target (slot 0x178,
    /// the caller's), the action list clears (the old actions' teardowns), and the install's start
    /// clears `+0x900`, puts the unit on its standing sprite (slot 0x68 with `+0x80`) and runs the
    /// range test; a `MoveTo` that starts arrived shows the standing sprite. A notify posted while
    /// the AI runs (its buffer, `FUN_140c55270`) is not modelled.
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
        // the attack command's Attack (`FUN_141081a80`) takes no MoveTo sprite: the master's
        let arms = crate::attack::arms(master, self.graphics, [-1; 4], -1);
        let standing = arms
            .idle_sprite
            .ok_or(Unsupported("an attacker without its standing sprite"))?;
        if unit.sprite.is_none() {
            return Err(Unsupported(
                "an attack command of a unit whose sprite the slice does not hold",
            ));
        }
        // the action list's clearing (FUN_140a98620): an Attack on another target tears down (its
        // MoveTo's stops the unit) and lets its targets go (slot 0x180)
        if let Some(mut a) = unit.attack.take() {
            a.tear_down(unit.mover.as_mut())?;
            let by = Releaser {
                id: unit.id,
                class: master.class,
                player: self.player,
            };
            for t in [a.target, a.target2] {
                if t != -1 {
                    self.releases.push((t, by));
                }
            }
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
            mut attack,
            arrived,
            posted,
            ready,
            failed,
        } = crate::attack::Attack::command(target, &e, arms, visible, t.player, mover)?;
        if failed {
            attack.fail_at_start(&mut unit.sprite, &mut unit.anim_speed, self.sprite)?;
        }
        if ready {
            return Err(Unsupported(
                "an attack command of a unit that cannot move (its set_state 6)",
            ));
        }
        if arrived {
            // the MoveTo's start in state 1 (`0x140aa29e0`, not quiet): the master's standing
            // sprite with its speed, `+0x7b8` cleared; the next update's state 4 ends the MoveTo
            mover.shows_standing();
            unit.shows_standing_sprite(self.masters, self.graphics, self.sprite)?;
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

    fn stop(&mut self) -> Result<(), Unsupported> {
        let masters = self.masters;
        let by = Releaser {
            id: self.unit.id,
            class: masters
                .and_then(|m| m.get(self.unit.life.master_id))
                .map_or(-1, |m| m.class),
            player: self.player,
        };
        self.unit
            .stop_with(self.releases, by, masters, self.graphics, self.sprite)
    }

    fn seek_unexplored(
        &mut self,
        ai: &UnitAi,
        rng: &mut Well512,
    ) -> Result<Option<[i32; 2]>, Unsupported> {
        let tc = town_centre(self.others)?;
        let (at, computer) = (self.unit.position, self.computer);
        // the search about the town centre only while the player has no scout map
        let scout_map = self.spatial.scout_map(self.player).is_some();
        let (mut area, map, line_of_sight) = self.scout_area()?;
        if !scout_map && let Some(p) = crate::explore::unexplored_area(tc, map, &mut area)? {
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
        let (at, player) = (self.unit.position, self.player);
        let scouted = self.spatial.scout_map(player).is_some();
        let (mut area, map, line_of_sight) = self.scout_area()?;
        area.scouted = scouted;
        let found = crate::explore::beyond_sight(&mut area, at, line_of_sight, map, rng)?;
        // the point found is marked in the scout map (FUN_140f4d5e0 with 1)
        if let Some([x, y]) = found {
            self.spatial.scout(player, x, y);
        }
        Ok(found)
    }

    /// The player's scout map (`FUN_140c589c0`'s case 0x202 when the auto scout found nothing):
    /// a new `InfluenceMap` of 480 by 480 (`FUN_140f4a240`, the old one goes, `+0x1368` 1) marks
    /// every tile the player sees now (its visible map's `FUN_141285350` 0xf) with 1.
    fn make_scout_map(&mut self) -> Result<(), Unsupported> {
        let size = crate::explore::SCOUT_MAP_SIZE;
        let mut m = crate::explore::InfluenceMap::new([size, size], 0);
        let [w, h] = self.spatial.map;
        for y in 0..h {
            for x in 0..w {
                if self.spatial.scout_sight(self.player, x, y) == Some(0xf) {
                    m.set(x, y, 1);
                }
            }
        }
        self.spatial.set_scout_map(self.player, m);
        Ok(())
    }

    /// The unit's slot 0x388 (`FUN_141226fc0`) on the foundation `target`: the world's object
    /// (`FUN_1415417a0`; the owner's sleeping list, which the unit does not see here, holds
    /// only objects of a type below 30, never a building), then its slot 0x820 with any action
    /// type (`0xffff`) and the switch ([`task_search`] on the target and its main building);
    /// with a task the new wait replaces the unit's actions, which the caller makes after the
    /// update ([`Wait`], [`Turns::wait_for_foundation`]).
    fn wait_solidify(&mut self, target: i32) -> Result<bool, Unsupported> {
        let unit = &*self.unit;
        let (player, others, foreign) = (self.player, self.others, self.foreign);
        let find = |id: i32| {
            if id == unit.id {
                return Some((player, unit));
            }
            others
                .iter()
                .find(|u| u.id == id)
                .map(|u| (player, u))
                .or_else(|| foreign.find(id))
        };
        let (Some(t), Some(general)) = (
            task_target_with(target, 1, &find, self.every)?,
            task_target_with(target, 2, &find, self.every)?,
        ) else {
            return Ok(false);
        };
        let (masters, own) = self
            .masters
            .and_then(|m| Some((m, m.get(unit.life.master_id)?)))
            .ok_or(Unsupported(
                "a unit AI's unit of a master the slice does not have",
            ))?;
        let actor = Actor {
            id: unit.id,
            master: own,
            player,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: self.attributes.get(95).copied().unwrap_or(0.0),
            civ: self.civ,
        };
        let dip = SideWorld {
            player,
            relations: self.relations,
            foreign,
            drop_sites: self.drop_sites,
        };
        let Some((task, switch)) = task_search(masters, &actor, &t, &general, -1, &dip)? else {
            return Ok(false);
        };
        self.waits.push(Wait {
            target,
            task,
            switch: switch.map(|m| m.id),
        });
        Ok(true)
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
            group: unit.group.id,
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
                groups: self.groups,
            },
            mover,
            me,
            player: self.player,
            master: master.id,
            radius: master.radius,
            reach: master.line_of_sight - LOS_MARGIN,
            scouted: false,
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
                    groups: self.groups,
                }),
            },
            sprite,
        )
    }

    /// A walk with the Sprite and the Action streams (a farmer's stops draw from the second).
    fn walk_sprites_action<'b>(
        &'b mut self,
        mover: Option<&'b mut Mover>,
        me: Me,
    ) -> (Walk<'b>, &'b mut Well512, &'b mut Well512) {
        let [object, sprite, action] =
            self.streams
                .three(Stream::Object, Stream::Sprite, Stream::Action);
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
                    groups: self.groups,
                }),
            },
            sprite,
            action,
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
                    groups: self.groups,
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
                groups: self.groups,
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
    /// The search reads the player's scout map for the tiles seen (`FUN_140c4b130` with one).
    scouted: bool,
}

impl crate::explore::Area for ScoutArea<'_> {
    fn explored(&self, x: i32, y: i32) -> Result<bool, Unsupported> {
        if self.scouted {
            return self
                .spatial
                .scout_map(self.player)
                .and_then(|m| m.get(x, y))
                .map(|v| v == 1)
                .ok_or(Unsupported("a tile off the player's scout map"));
        }
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
            .or_else(|| self.gather.map(|g| g.target).filter(|&t| t != -1))
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
            waits: m.waiting != 0 && !self.top_action().is_some_and(|a| matches!(a.state, 6 | 7)),
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

    /// Its top action's slots 0x190 and 0x288 (`FUN_14106d090`, `FUN_141082dc0`): the action has
    /// neither a target nor a point (`+0x30`, `+0x38` −1), or there is none. An Artifact holds
    /// them while it walks a move ([`crate::artifact::Artifact::idle`]); a `MoveTo` always holds
    /// its point.
    ///
    /// # Errors
    ///
    /// A top action the slice does not model.
    pub fn top_action_idle(&self) -> Result<bool, Unsupported> {
        if self.unmodelled_action.is_some() {
            return Err(Unsupported(
                "slots 0x190 and 0x288 of an action the slice does not model",
            ));
        }
        match self.top_action_kind() {
            None => Ok(true),
            Some(crate::artifact::ARTIFACT_TYPE) => Ok(self.artifact.is_some_and(|a| a.idle())),
            // a Make has no target, but its point (`+0x38`) is the base constructor's 0.0
            Some(crate::move_to::MOVE_TO | crate::make::MAKE) => Ok(false),
            Some(_) => Err(Unsupported(
                "slots 0x190 and 0x288 of an action the slice does not model",
            )),
        }
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
        .or_else(|| self.gather.map(|g| action(crate::gather::GATHER, g.state)))
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
        .or_else(|| self.make.map(|m| action(crate::make::MAKE, m.state)))
        .or_else(|| {
            self.wait_solidify
                .map(|w| action(crate::wait_solidify::WAIT_SOLIDIFY, w.state))
        })
    }

    /// The target of its top action ([`Unit::top_action`]), −1 for an action without one (an
    /// Artifact not moving to an object, a Fly, an Explore, a wait for a foundation, which keeps
    /// the foundation at `+0x68`); None without an action the slice holds.
    #[must_use]
    pub fn top_target(&self) -> Option<i32> {
        if let Some(a) = self.artifact {
            return Some(a.target);
        }
        None.or_else(|| self.build.map(|b| b.target))
            .or_else(|| self.move_to.map(|m| m.target))
            .or_else(|| self.fly.map(|_| -1))
            .or_else(|| self.hunt.map(|h| h.target))
            .or_else(|| self.gather.map(|g| g.target))
            .or_else(|| self.attack.map(|a| a.target))
            .or_else(|| self.explore.map(|_| -1))
            .or_else(|| self.enter.map(|e| e.target))
            .or_else(|| {
                self.missile
                    .as_ref()
                    .and_then(|m| m.action.as_ref())
                    .map(|a| a.target)
            })
            .or_else(|| self.make.map(|_| -1))
            .or_else(|| self.wait_solidify.map(|_| -1))
    }

    /// The second target of its top action (`+0x34`, `FUN_140a78430`), −1 for none: a Hunt's
    /// or a Gather's drop site, a missile's shooter; the slice's other actions hold none (an import checks the
    /// save's). None without an action the slice holds.
    #[must_use]
    pub fn top_second_target(&self) -> Option<i32> {
        match self.top_action_kind()? {
            crate::hunt::HUNT => self.hunt.map(|h| h.carry_to),
            crate::gather::GATHER => self.gather.map(|g| g.carry_to),
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
    pub(crate) fn interruptible(&self) -> Result<bool, Unsupported> {
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
                sharing: c.sharing,
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
                    (&w.spatial.relations, w.sharing),
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
        let by = self.releaser(c);
        let sprite = c.streams.get(Stream::Sprite);
        self.stop_with(&mut c.releases, by, c.masters, c.graphics, sprite)
    }

    /// [`Unit::stop_action`] on the parts of the world it changes: the targets its actions let
    /// go (`by` the unit), its masters and sprites, and the Sprite stream.
    fn stop_with(
        &mut self,
        releases: &mut Vec<(i32, Releaser)>,
        by: Releaser,
        masters: Option<&Masters>,
        graphics: &std::collections::BTreeMap<i16, Graphic>,
        sprite: &mut Well512,
    ) -> Result<(), Unsupported> {
        // its own Attack's teardown: the MoveTo's stops the unit; it lets its target go
        if let Some(mut a) = self.attack.take() {
            a.tear_down(self.mover.as_mut())?;
            for t in [a.target, a.target2] {
                if t != -1 {
                    releases.push((t, by));
                }
            }
        }
        // the Hunt's teardown: its sub-actions go (an Attack's or MoveTo's stops the unit) and the
        // base teardown lets its target go
        if let Some(mut h) = self.hunt.take() {
            h.tear_down(self.mover.as_mut())?;
            if h.target != -1 {
                releases.push((h.target, by));
            }
        }
        // the Gather's teardown: its MoveTo's stops the unit and the base teardown lets its
        // target, then its second target go
        if let Some(mut g) = self.gather.take() {
            g.tear_down(self.mover.as_mut())?;
            for t in [g.target, g.carry_to] {
                if t != -1 {
                    releases.push((t, by));
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
            releases.push((e.target, by));
        }
        self.clear_own_actions()?;
        if self.life.state == 2
            && let Some(m) = self.mover.as_mut()
        {
            m.shows_standing();
        }
        if self.life.state == 2 && self.sprite.is_some() {
            let standing = masters
                .and_then(|m| m.get(self.life.master_id))
                .map(|m| m.standing_sprite)
                .and_then(|id| graphics.get(&id).copied())
                .ok_or(Unsupported("a stopped unit without its standing sprite"))?;
            crate::attack::set_sprite(&mut self.sprite, standing, sprite);
        }
        Ok(())
    }

    /// The action list's clearing (`FUN_140a98620`): each action's teardown (a `MoveTo`'s, also
    /// an Explore's, stops the unit, slot 0x5d0). A Build waiting without work and a wait for a
    /// foundation go; the others the slice holds are not modelled.
    fn clear_own_actions(&mut self) -> Result<(), Unsupported> {
        // a wait for a foundation goes: its teardown is the base one's, which lets no target go
        self.wait_solidify = None;
        if self
            .build
            .as_ref()
            .is_some_and(|b| b.state == 3 && b.target == -1 && b.move_to.is_none())
        {
            self.build = None;
        }
        let held = [
            (self.artifact.is_some(), "an Artifact"),
            (self.build.is_some(), "a Build"),
            (self.fly.is_some(), "a Fly"),
            (self.hunt.is_some(), "a Hunt"),
            (self.gather.is_some(), "a Gather"),
            (self.attack.is_some(), "an Attack"),
            (self.enter.is_some(), "an Enter"),
            (self.unmodelled_action.is_some(), "an action not held"),
        ];
        if let Some(&(_, what)) = held.iter().find(|(h, _)| *h) {
            return Err(Unsupported(match what {
                "an Artifact" => "a unit AI's stop of an action list with an Artifact",
                "a Build" => "a unit AI's stop of an action list with a Build at work",
                "a Fly" => "a unit AI's stop of an action list with a Fly",
                "a Hunt" => "a unit AI's stop of an action list with a Hunt",
                "a Gather" => "a unit AI's stop of an action list with a Gather",
                "an Attack" => "a unit AI's stop of an action list with an Attack",
                "an Enter" => "a unit AI's stop of an action list with an Enter",
                _ => "a unit AI's stop of an action list the slice does not model",
            }));
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
                Effect::MoveAside(m) => c.move_aside.push(m),
                Effect::OrderOnTarget(order) => {
                    if c.order_on_target.replace((order, None)).is_some() {
                        return Err(Unsupported("two orders on targets in one update"));
                    }
                }
                Effect::AfterOrder(after) => {
                    let (_, tail) = c.order_on_target.as_mut().ok_or(Unsupported(
                        "a unit AI's tail after an order it did not take",
                    ))?;
                    *tail = Some(after);
                }
                Effect::HuntObject(target) => c.hunt_objects.push(target),
                Effect::GatherObject(target) => c.gather_objects.push(target),
                Effect::BuildObject(target) => {
                    if c.build_object.replace(target).is_some() {
                        return Err(Unsupported("two BuildObjects in one update"));
                    }
                }
                Effect::Explore(point) => self.explore_to(point, c)?,
                Effect::MoveToObject { target, range } => self.move_to_object(target, range, c)?,
            }
        }
        Ok(())
    }

    /// The Hunt's `set_state(3)` (`0x140ac60e0` case 3) after its target went empty when it faced
    /// it: the target goes (slot 0x68 with none: its slot 0x180), the unit AI is told
    /// ([`Unit::hunt_search_notify`]), the unit takes the sprite it stands with (slot 0x68 with
    /// `FUN_140ac5210`'s) and the timer starts at −10 s. A hunt of class 10 would end its search
    /// (`set_state` 2), not modelled.
    fn hunt_set_search(&mut self, c: &mut Context<'_>, others: &[Self]) -> Result<(), Unsupported> {
        let hunt = self.hunt.as_mut().expect("the caller's hunt");
        if hunt.target_class != 10 {
            let target = std::mem::replace(&mut hunt.target, -1);
            self.hunt_release(c, [target]);
        }
        self.hunt_search_state(c, others)
    }

    /// The Hunt's `set_state(3)` itself (`0x140ac60e0` case 3; its target stays): the unit AI is
    /// told ([`Unit::hunt_search_notify`]), the unit takes the sprite it stands with and the timer
    /// starts at −10 s. A human player's hunt of class 10 is done instead (`set_state` 2: the
    /// target and the second target go), a computer player's would end (1), not modelled.
    fn hunt_search_state(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), Unsupported> {
        let hunt = self.hunt.as_mut().expect("the caller's hunt");
        let task = hunt.task;
        if hunt.target_class == 10 {
            // a human player's hunt is done (set_state 2), a computer player's ends (1)
            if c.computer {
                return Err(Unsupported(
                    "a computer player's hunt of class 10's search (set_state 1)",
                ));
            }
            let released = hunt.set_done();
            self.hunt_stand(c, &task)?;
            self.hunt_release(c, released);
            return Ok(());
        }
        hunt.timer = SEARCH_AGAIN;
        self.hunt_search_notify(c, others)?;
        self.hunt_stand(c, &task)
    }

    /// The hunter takes the sprite its hunt stands with (slot 0x68 with `FUN_140ac5210`'s, by the
    /// task and what the unit carries; [`crate::hunt::stand_sprite`]).
    fn hunt_stand(
        &mut self,
        c: &mut Context<'_>,
        task: &crate::hunt::HuntTask,
    ) -> Result<(), Unsupported> {
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(Unsupported("a hunter of a master the slice does not have"))?;
        let stand = crate::hunt::stand_sprite(
            task,
            master.standing_sprite,
            self.life.attribute_type,
            self.life.amount,
            c.graphics,
        );
        if let Some(s) = stand {
            crate::attack::set_sprite(&mut self.sprite, s, c.streams.get(Stream::Sprite));
        }
        Ok(())
    }

    /// The objects a hunt let go (−1 for none), released in order after the update.
    fn hunt_release<const N: usize>(&self, c: &mut Context<'_>, ids: [i32; N]) {
        for id in ids.into_iter().filter(|&id| id != -1) {
            c.releases.push((id, self.releaser(c)));
        }
    }

    /// The Hunt in state 3 tells its unit AI to look for the next target (`0x140ac60e0` case 3,
    /// `0x140ac7a00` case 2): notify 0x202 with the hunt action (0x265), its target class (`+0x68`)
    /// and its drop site: the second target; −1 for a master without drop sites (`+0x7a0`); else
    /// the closest (`FUN_1411671e0` as `set_state(8)` searches, [`Unit::drop_site`]), with none
    /// found 0, and the hunt's class goes (−1).
    fn hunt_search_notify(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), Unsupported> {
        let hunt = self.hunt.as_ref().expect("the caller's hunt");
        let (carry_to, class) = (hunt.carry_to, i32::from(hunt.target_class));
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(Unsupported("a hunter of a master the slice does not have"))?;
        let site = if carry_to != -1 {
            carry_to
        } else if master.drop_sites.first().is_none_or(|&d| d == -1) {
            -1
        } else {
            let mut w = DropWorld {
                masters: c.masters,
                spatial: c.spatial,
                zones: c.zones,
                pathing: c.pathing,
                object: c.streams.get(Stream::Object),
                world_time: c.world_time,
                computer: c.computer,
                construction_rate: c.construction_rate,
                groups: c.groups,
            };
            let (site, _) = self.drop_site(&mut w, others, -1, crate::hunt::SEARCH)?;
            if site == -1 {
                self.hunt.as_mut().expect("the caller's hunt").target_class = -1;
                0
            } else {
                site
            }
        };
        self.post_own(SEARCH_NEEDED, [crate::unit_ai::HUNT_ACTION, class, site])
    }

    /// The hunt's state 8 after its load went (`0x140ac7a00` case 7, after `FUN_14119d310`): a
    /// hunt with a target walks back to it (`set_state` 4), not modelled. One without asks its unit
    /// AI: the queued orders (gather, hunt, on a target) whose target is gone or dead go
    /// (`FUN_140c5de60`); orders left end the hunt (the AI's order and action clear, `set_state`
    /// 1, the action list removes it; its teardown's release of the drop site is not held, as its
    /// take is not). The target class (the AI's target's, else the hunt's `+0x68`) the
    /// civilian's slot 0x1b0 knows (`FUN_140c1c4f0`) takes the search by kind (slot 0x80); a hunt
    /// of class 10 searches nothing; a unit in a pasture (`FUN_1411c7670`) and a farm found are
    /// not modelled. Else `ClosestObject` (`FUN_140c488b0`, every relation, below one worker for
    /// class 49) of the class over the AI's search (`FUN_140c63e20`: its sight plus a tile, at
    /// least its search radius, plus the civilian's growth; enemies found are not modelled, else
    /// the alert mode clears). The object found becomes the hunt's target (slot 0x48,
    /// [`HuntAction::retarget`], which takes it: its workers count one more); with none a human
    /// player's hunt walks to its point to search again (`set_state(11)`,
    /// [`HuntAction::search_walk`]). A hunt that fails at once takes the standing sprite (slot
    /// 0x68 with `FUN_140ac5210`'s: the unit carries nothing now).
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
            let moving = hunt
                .task
                .walk_sprite_id(self.life.attribute_type, self.life.amount);
            let arms = crate::attack::arms(master, c.graphics, hunt.task.sprites, moving);
            let (Some(hunt), Some(mover)) = (self.hunt.as_mut(), self.mover.as_mut()) else {
                return Err(Unsupported("a hunter without its hunt or movement state"));
            };
            hunt.carry_to = -1;
            if hunt.chase(&quarry, c.player, &engagement, arms, mover)? {
                return Err(Unsupported(
                    "a hunt's Attack whose MoveTo starts arrived after its drop-off",
                ));
            }
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
        let kind = crate::unit_ai::civilian_kind(class);
        let found = if kind == -1 && hunt.target_class == 10 {
            // a hunt of class 10 searches nothing
            -1
        } else if kind == -1 {
            if self.my_pasture != -1 {
                return Err(Unsupported("a hunter tending a pasture (FUN_1411c7670)"));
            }
            let mover = self
                .mover
                .as_ref()
                .ok_or(Unsupported("a hunter without movement state"))?;
            let radius =
                crate::unit_ai::civilian_search_radius(master.line_of_sight, master.search_radius);
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
                    spared: -1,
                },
            };
            let me = Me {
                id: self.id,
                position: self.position,
                group: self.group.id,
                action_state: hunt.state,
            };
            let mut plan = Planning {
                pathing: c.pathing,
                now: c.world_time,
                computer: c.computer,
                object: c.streams.get(Stream::Object),
                construction_rate: c.construction_rate,
                groups: c.groups,
            };
            let master_id = self.life.master_id;
            let mut reach = |id: i32, range: f32| {
                mover.reaches_object(&me, spatial, &mut plan, id, range, master_id)
            };
            let mut no_attack = crate::unit_ai::NoAttack;
            let foreign = c.foreign;
            let moving = |id: i32| herded(others, foreign, id);
            let shuns = |_: &Placed| -> Result<bool, Unsupported> {
                Err(Unsupported("a class-10 searcher in a hunt's search"))
            };
            let farm = |id: i32| {
                others
                    .iter()
                    .find(|u| u.id == id)
                    .or_else(|| foreign.find(id).map(|(_, u)| u))
                    .map(|u| (u.workers, u.pasture))
            };
            let result = spatial.search(
                self.id,
                c.player,
                &search,
                &mut no_attack,
                &mut reach,
                &moving,
                &shuns,
                &farm,
            )?;
            if result.fresh {
                if result.enemies {
                    return Err(Unsupported(
                        "a hunt's search after its drop-off finding enemies",
                    ));
                }
                ai.alert_mode = false;
            }
            result.id
        } else {
            // the unit AI's ClosestResourceObject (slot 0x80) of the kind, at most 2 workers for
            // wood, no drop site, state 2, no point (the probe of 4533's call)
            let cap = if kind == 0 { 2 } else { -1 };
            self.resource_search(c, others, kind as u8, cap)?
        };
        if found == -1 {
            // LAB_140ac9509 with nothing found: a computer player's hunt of a kind but wood
            // would end (set_state 1); one with its point walks there to search again
            // (set_state 11, its second target kept); without a point it would wait
            // (set_state 2)
            if c.computer && kind != 0 {
                return Err(Unsupported(
                    "a computer player's hunt that found nothing after its drop-off (set_state 1)",
                ));
            }
            let hunt = self.hunt.as_mut().expect("the caller's hunt");
            if hunt.point[0] == -1.0 || hunt.point[1] == -1.0 {
                return Err(Unsupported(
                    "a hunt without a point that found nothing after its drop-off (set_state 2)",
                ));
            }
            let mover = self
                .mover
                .as_mut()
                .ok_or(Unsupported("a hunter without movement state"))?;
            return hunt.search_walk(mover);
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
        let (sprites, moving) = self.hunt.as_ref().map_or(([-1; 4], -1), |h| {
            let moving = h
                .task
                .walk_sprite_id(self.life.attribute_type, self.life.amount);
            (h.task.sprites, moving)
        });
        let arms = crate::attack::arms(master, c.graphics, sprites, moving);
        let drops_here = master.drop_sites.contains(&t.life.master_id);
        let held = self.life.amount;
        let (Some(hunt), Some(mover)) = (self.hunt.as_mut(), self.mover.as_mut()) else {
            return Err(Unsupported("a hunter without its hunt or movement state"));
        };
        if let Some(arrived) = hunt.retarget(
            &quarry,
            c.player,
            held,
            drops_here,
            &engagement,
            arms,
            mover,
        )? {
            if arrived {
                crate::attack::show_arrived(
                    &arms,
                    &mut self.sprite,
                    &mut self.anim_speed,
                    c.streams.get(Stream::Sprite),
                )?;
            }
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
            groups: c.groups,
        };
        self.drop_off_in(w, others)
    }

    /// The Gather's update (slot 6, `0x140a8e1d0`). It first checks its targets: a target gone
    /// from the world's table goes (slot 0x68 with none), one past state 6 is let go (slot 0x180)
    /// and goes; a second target gone goes (`FUN_140a7a190` with none). Then state 8 walks the
    /// load to its drop site ([`crate::gather::GatherAction::update_drop`]): the load goes to the
    /// player, the drop site goes (`FUN_140a7a190` with none) and the gather walks back to its
    /// target (`set_state(4)`); without a target the unit AI's search is not modelled. The other
    /// states ([`crate::gather::GatherAction::update`]) may ask for the drop-off
    /// ([`Unit::gather_drop_off`]).
    fn gather_update(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
        me: Me,
    ) -> Result<(), Unsupported> {
        let by = self.releaser(c);
        let gather = self.gather.as_mut().expect("the caller's gather");
        if gather.target != -1 {
            let state = c
                .victim
                .as_deref()
                .filter(|v| v.id == gather.target)
                .map(|v| v.state);
            let gone = match state {
                Some(s) => 6 < s,
                None if c.spatial.get(gather.target).is_some() => {
                    return Err(Unsupported("a gather's target the slice cannot read"));
                }
                None => true,
            };
            if gone {
                if state.is_some() {
                    c.releases.push((gather.target, by));
                }
                gather.target = -1;
            }
        }
        if gather.carry_to != -1 && c.spatial.get(gather.carry_to).is_none() {
            gather.carry_to = -1;
        }
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(Unsupported(
                "a gatherer of a master the slice does not have",
            ))?;
        let g = |id: i16| (0 <= id).then(|| c.graphics.get(&id).copied()).flatten();
        let task = gather.task;
        let sprites = HuntSprites {
            moving: g(task.sprites[0]),
            working: g(task.sprites[2]),
            carrying: g(task.sprites[3]),
            standing: g(master.standing_sprite),
        };
        // the gatherer's view: the master's work rate (+0x798) plus the powerup (slot 0x6e8)
        let productivity = usize::try_from(task.productivity)
            .ok()
            .and_then(|i| c.attributes.get(i))
            .copied()
            .filter(|&v| 0.0 < v)
            .unwrap_or(1.0);
        let (work_rate, capacity, class) = (
            self.work_rate_powerup + master.work_rate,
            f32::from(master.capacity),
            master.class,
        );
        let stance = self.ai.as_ref().map_or(0, |ai| ai.stance);
        let (world_time, farm_stops, dt) = (c.world_time, c.farm_stops, c.dt_seconds);
        let stop_productivity = crate::gather::productivity_of(task.productivity, c.attributes);
        let food_sold = c
            .attributes
            .get(crate::gather::FARM_FOOD_SOLD)
            .copied()
            .unwrap_or(0.0);
        let (player, computer) = (c.player, c.computer);
        if gather.state == crate::gather::DROP {
            let me_at = c
                .spatial
                .get_known(self.id)?
                .ok_or(Unsupported("a gatherer without a place"))?;
            let (me_position, me_radius) = (me_at.position, me_at.radius);
            let site = match c.spatial.get_known(gather.carry_to)? {
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
            let (walk, sprite_rng, action_rng) = c.walk_sprites_action(self.mover.as_mut(), me);
            let mut work = crate::gather::Gatherer {
                held_kind: &mut self.life.attribute_type,
                held: &mut self.life.amount,
                work_rate,
                capacity,
                productivity,
                sprites,
                sprite: &mut self.sprite,
                sprite_rng,
                class,
                master: self.life.master_id,
                stance,
                world_time,
                farm_stops,
                posted: Vec::new(),
                action_rng,
                player,
                computer,
                stop_productivity,
                food_sold,
                sold: 0.0,
                anim_stopped: false,
            };
            let before = [gather.target, gather.carry_to];
            let dropped = gather.update_drop(walk, &mut work, attributes, site)?;
            let (posted, anim_stopped) = (std::mem::take(&mut work.posted), work.anim_stopped);
            for (kind, p) in posted {
                self.post_own(kind, p)?;
            }
            if anim_stopped && let Some(a) = self.anim_speed.as_mut() {
                *a = 0.0;
            }
            let (kind, amount) = match dropped {
                crate::hunt::Dropped::Going => return Ok(()),
                crate::hunt::Dropped::Load(kind, amount) => (kind, amount),
                // set_state(8) again: the drop site kept or searched
                crate::hunt::Dropped::Again => return self.gather_drop_off(c, others),
                // set_state(2): the target, then the drop site go (slot 0x68, FUN_140a7a190)
                crate::hunt::Dropped::Idle => {
                    for id in before.into_iter().filter(|&id| id != -1) {
                        c.releases.push((id, by));
                    }
                    return Ok(());
                }
            };
            // the player's slot 0xf8 (FUN_141616820) with 1 and 1, as the hunt's drop-off
            give_load(
                (&c.spatial.relations, c.sharing),
                c.player,
                &mut c.attribute_adds,
                kind,
                amount,
            )?;
            let gather = self.gather.as_mut().expect("the caller's gather");
            if gather.target == -1 {
                return self.gather_after_drop(c, others);
            }
            // back to the target: the drop site goes (FUN_140a7a190 with none), set_state(4)
            c.releases.push((gather.carry_to, by));
            gather.carry_to = -1;
            let t = c
                .spatial
                .get_known(gather.target)?
                .ok_or(Unsupported("a gather's target without a place"))?;
            let gap = footprint_gap(
                self.position,
                [me_radius[0], me_radius[1]],
                t.position,
                t.radius,
            );
            let mover = self
                .mover
                .as_mut()
                .ok_or(Unsupported("a gatherer without movement state"))?;
            let rng = c.streams.get(Stream::Action);
            return gather.set_walk(t.position, t.radius, gap, mover, rng);
        }
        // the target the caller holds, back in the context once the update is done
        let mut victim = c.victim.take();
        let (old_target, old_second) = (gather.target, gather.carry_to);
        let (next, posted, sold) = {
            let (walk, sprite_rng, action_rng) = c.walk_sprites_action(self.mover.as_mut(), me);
            let mut work = crate::gather::Gatherer {
                held_kind: &mut self.life.attribute_type,
                held: &mut self.life.amount,
                work_rate,
                capacity,
                productivity,
                sprites,
                sprite: &mut self.sprite,
                sprite_rng,
                class,
                master: self.life.master_id,
                stance,
                world_time,
                farm_stops,
                posted: Vec::new(),
                action_rng,
                player,
                computer,
                stop_productivity,
                food_sold,
                sold: 0.0,
                anim_stopped: false,
            };
            let target = victim.as_deref_mut().filter(|v| v.id == gather.target);
            let next = gather.update(walk, target, &mut work, dt);
            (
                next,
                std::mem::take(&mut work.posted),
                (work.sold, work.anim_stopped),
            )
        };
        let (sold, anim_stopped) = sold;
        c.victim = victim;
        let next = next?;
        for (kind, p) in posted {
            self.post_own(kind, p)?;
        }
        // a random wait stops the unit's animation (FUN_141161850: its +0x7b8 0)
        if anim_stopped && let Some(a) = self.anim_speed.as_mut() {
            *a = 0.0;
        }
        // a farm's food sold at once: the player's slot 0xf8 with 1 (food), as a drop-off's
        if 0.0 < sold {
            give_load(
                (&c.spatial.relations, c.sharing),
                c.player,
                &mut c.attribute_adds,
                0,
                sold,
            )?;
        }
        // the targets an end or an idle let go (slot 0x68, FUN_140a7a190 with none); state 1's
        // update returns 1: the action list removes the Gather, whose teardown lets its targets go
        if next == crate::gather::Next::Done {
            let g = self.gather.take().expect("the caller's gather");
            for t in [g.target, g.carry_to] {
                if t != -1 {
                    c.releases.push((t, by));
                }
            }
            return Ok(());
        }
        if let Some(g) = self.gather.as_ref() {
            for (old, new) in [(old_target, g.target), (old_second, g.carry_to)] {
                if old != new && old != -1 {
                    c.releases.push((old, by));
                }
            }
        }
        match next {
            crate::gather::Next::Drop => self.gather_drop_off(c, others)?,
            crate::gather::Next::Search => self.gather_search(c, others)?,
            crate::gather::Next::Build => {
                let target = self.gather.as_ref().map_or(-1, |g| g.target);
                if c.build_command.replace(target).is_some() {
                    return Err(Unsupported("two build commands in one update"));
                }
            }
            crate::gather::Next::Stay | crate::gather::Next::Done => {}
        }
        Ok(())
    }

    /// The Gather's search in state 3 (`0x140a8cd40` case 3 and its repeat, `0x140a8e1d0` case
    /// 2): the unit's notify 0x202 to its unit AI (slot 0x328) with the gather action (`0x261`),
    /// the Gather's target type (`+0x68`) and a drop site: the second target's, else for a master
    /// with drop sites (`+0x7a0`) the one `FUN_1411671e0` finds from the unit (inside others too:
    /// [`Unit::drop_site_from`]), 0 when it finds none; −1 for a master without.
    fn gather_search(&mut self, c: &mut Context<'_>, others: &[Self]) -> Result<(), Unsupported> {
        let gather = self.gather.as_ref().expect("the caller's gather");
        let (carry_to, target_type, state) = (gather.carry_to, gather.target_type, gather.state);
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(Unsupported(
                "a gatherer of a master the slice does not have",
            ))?;
        let site = if carry_to != -1 {
            carry_to
        } else if master.drop_sites.first().is_some_and(|&d| d != -1) {
            let mut w = DropWorld {
                masters: c.masters,
                spatial: c.spatial,
                zones: c.zones,
                pathing: c.pathing,
                object: c.streams.get(Stream::Object),
                world_time: c.world_time,
                computer: c.computer,
                construction_rate: c.construction_rate,
                groups: c.groups,
            };
            self.drop_site_from(&mut w, others, state, false)?
                .unwrap_or(0)
        } else {
            -1
        };
        self.post_own(
            crate::unit_ai::REQUIRES_SEARCH,
            [crate::unit_ai::GATHER_ACTION, target_type, site],
        )
    }

    /// The Gather's state 8 after the deposit without a target (`0x140a8e1d0` case 7; its drop
    /// site stays): the unit AI drops the queued orders on targets gone (`FUN_140c5de60`) and,
    /// with orders left (`FUN_140c564a0`), ends its order and action (`FUN_140c64b60`,
    /// `FUN_140c64b50`) and the Gather is done (`set_state(1)`: its update returns 1, the list
    /// removes it). Else the gather group of its target class (the AI's `+0x58`, else the
    /// Gather's `mTargetType`; slot 0x1b0, the civilian's) names the unit AI's
    /// `ClosestResourceObject` (slot 0x80: no drop site, any workers, state 2, no point), and the
    /// Gather takes what it finds (slot 0x48, [`crate::gather::GatherAction::retarget`]). Without
    /// one a Gather with its point would walk there to search again (`set_state(11)`, not
    /// modelled, or 0xf3 for a farmer); without a point a computer player's Gather is done, a
    /// human's idles (`set_state(2)`).
    fn gather_after_drop(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<(), Unsupported> {
        let by = self.releaser(c);
        let spatial: &Spatial = c.spatial;
        let gone = |id: i32| match spatial.get(id) {
            None => true,
            Some(_) => spatial
                .sight
                .as_ref()
                .and_then(|v| v.seer(id))
                .is_some_and(|s| 5 < s.state),
        };
        if let Some(ai) = self.ai.as_mut() {
            ai.orders
                .retain(|o| !(matches!(o.order, 0x2c5 | 0x2c9 | 0x2db) && gone(o.target)));
            if !ai.orders.is_empty() {
                ai.end_order_and_action()?;
                let mut gather = self.gather.take().expect("the caller's gather");
                gather.tear_down(self.mover.as_mut())?;
                if gather.carry_to != -1 {
                    c.releases.push((gather.carry_to, by));
                }
                return Ok(());
            }
        }
        let gather = self.gather.as_ref().expect("the caller's gather");
        let found = match self.ai.as_ref() {
            Some(ai) => {
                let class = if ai.target_type == -1 {
                    gather.target_type
                } else {
                    ai.target_type
                };
                if ai.ai_type != crate::unit_ai::CIVILIAN {
                    return Err(Unsupported(
                        "slot 0x1b0 of another unit AI than the civilian's",
                    ));
                }
                let kind = crate::unit_ai::civilian_kind(class);
                if kind == -1 {
                    -1
                } else {
                    self.resource_search(c, others, kind as u8, -1)?
                }
            }
            None => -1,
        };
        if found == -1 {
            let farm_waits = crate::gather::waits_for_a_farm(
                self.life.master_id,
                self.gather.as_ref().map_or(-1, |g| g.target_type),
            );
            let gather = self.gather.as_mut().expect("the caller's gather");
            if gather.point[0] != -1.0 && gather.point[1] != -1.0 {
                if farm_waits {
                    return Err(Unsupported(
                        "a farmer that found nothing after its drop-off (set_state 0xf3)",
                    ));
                }
                // set_state(11) without a target: to the point within the task's range
                let mover = self
                    .mover
                    .as_mut()
                    .ok_or(Unsupported("a gatherer without movement state"))?;
                return gather.walk_back(0.0, mover);
            }
            // LAB_140a8ed55: a computer player's Gather is done, a human's idles
            let carry_to = gather.carry_to;
            if c.computer {
                let mut gather = self.gather.take().expect("the caller's gather");
                gather.tear_down(self.mover.as_mut())?;
            } else {
                let master =
                    c.masters
                        .and_then(|m| m.get(self.life.master_id))
                        .ok_or(Unsupported(
                            "a gatherer of a master the slice does not have",
                        ))?;
                let standing = (0 <= master.standing_sprite)
                    .then(|| c.graphics.get(&master.standing_sprite).copied())
                    .flatten();
                let carrying = (0 <= gather.task.sprites[3])
                    .then(|| c.graphics.get(&gather.task.sprites[3]).copied())
                    .flatten();
                let shows = match carrying {
                    Some(g)
                        if gather.task.resource != -1
                            && self.life.attribute_type == gather.task.resource
                            && 1 <= self.life.amount as i32 =>
                    {
                        Some(g)
                    }
                    _ => standing,
                };
                gather.tear_down(self.mover.as_mut())?;
                gather.state = crate::gather::IDLE;
                gather.timer = 0.0;
                gather.target = -1;
                gather.carry_to = -1;
                gather.point = [-1.0; 3];
                if let Some(g) = shows {
                    crate::attack::set_sprite(&mut self.sprite, g, c.streams.get(Stream::Sprite));
                }
            }
            if carry_to != -1 {
                c.releases.push((carry_to, by));
            }
            return Ok(());
        }
        // slot 0x48 on what it found
        let foreign = c.foreign;
        let (owner, t) = match others.iter().find(|u| u.id == found) {
            Some(t) => (c.player, t),
            None => foreign
                .find(found)
                .ok_or(Unsupported("a gather's new target the slice does not hold"))?,
        };
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(Unsupported(
                "a gatherer of a master the slice does not have",
            ))?;
        let t_master = c
            .every_masters
            .get(owner)
            .and_then(|m| m.get(t.life.master_id))
            .ok_or(Unsupported(
                "a gather's target of a master the slice does not have",
            ))?;
        let resource = crate::gather::Resource {
            id: found,
            position: t.position,
            class: t_master.class,
            master: t_master.id,
            player: owner,
            workers: t.workers,
            drop_site: master.drop_sites.contains(&t.life.master_id),
            gap: footprint_gap(self.position, master.radius, t.position, t_master.radius),
            radius: t_master.radius,
        };
        let (computer, farm_stops, held) = (c.computer, c.farm_stops, self.life.amount);
        let rate = crate::gather::StopRate {
            work_rate: self.work_rate_powerup + master.work_rate,
            capacity: f32::from(master.capacity),
            productivity: crate::gather::productivity_of(
                self.gather.map_or(-1, |g| g.task.productivity),
                c.attributes,
            ),
        };
        let (Some(gather), Some(mover)) = (self.gather.as_mut(), self.mover.as_mut()) else {
            return Err(Unsupported(
                "a gatherer without its gather or movement state",
            ));
        };
        let (old_target, old_second) = (gather.target, gather.carry_to);
        let rng = c.streams.get(Stream::Action);
        let _ = gather.retarget(
            &resource,
            c.player,
            computer,
            held,
            (farm_stops, rate),
            mover,
            rng,
        )?;
        let (new_target, new_second) = (gather.target, gather.carry_to);
        for (old, new) in [(old_target, new_target), (old_second, new_second)] {
            if old != new {
                if old != -1 {
                    c.releases.push((old, by));
                }
                if new != -1 {
                    if owner == c.player && new == found {
                        let u = others
                            .iter_mut()
                            .find(|u| u.id == found)
                            .expect("found above");
                        if u.workers < 550 {
                            u.workers += 1;
                        }
                        u.life.goto_sleep = false;
                    } else {
                        c.takes.push(new);
                    }
                }
            }
        }
        Ok(())
    }

    /// The Gather's `set_state(8)` (`0x140a8cd40` case 8): the current stop clears; a gatherer
    /// carrying nothing would search (`set_state` 3, not modelled); else its drop site
    /// ([`Unit::drop_site`]) becomes its second target (`FUN_140a7a190`: slot 0x178) and a quiet
    /// `MoveTo` walks there ([`crate::gather::GatherAction::set_drop`]). Without one the unit AI's
    /// part (notify 0x203) is not modelled.
    fn gather_drop_off(&mut self, c: &mut Context<'_>, others: &[Self]) -> Result<(), Unsupported> {
        let w = DropWorld {
            masters: c.masters,
            spatial: c.spatial,
            zones: c.zones,
            pathing: c.pathing,
            object: c.streams.get(Stream::Object),
            world_time: c.world_time,
            computer: c.computer,
            construction_rate: c.construction_rate,
            groups: c.groups,
        };
        if let Some(site) = self.gather_drop_off_in(w, others)? {
            c.takes.push(site);
        }
        Ok(())
    }

    /// [`Unit::gather_drop_off`] in `w`, from the update or a command: the drop site it took
    /// when it is a new second target (the caller's to take, slot 0x178).
    pub(crate) fn gather_drop_off_in(
        &mut self,
        mut c: DropWorld<'_>,
        others: &[Self],
    ) -> Result<Option<i32>, Unsupported> {
        if self.life.amount <= 0.0 {
            return Err(Unsupported(
                "a gather's drop-off carrying nothing (set_state 3)",
            ));
        }
        let gather = self.gather.as_ref().expect("the caller's gather");
        let (carry_to, state) = (gather.carry_to, gather.state);
        let (site, gap) = self.drop_site(&mut c, others, carry_to, state)?;
        let (Some(gather), Some(mover)) = (self.gather.as_mut(), self.mover.as_mut()) else {
            return Err(Unsupported(
                "a gatherer without its gather or movement state",
            ));
        };
        gather.carry_to = site;
        gather.set_drop(gap, mover)?;
        Ok((site != carry_to && site != -1).then_some(site))
    }

    /// [`Unit::hunt_drop_off`] in `w`, from the update or a command.
    pub(crate) fn drop_off_in(
        &mut self,
        mut c: DropWorld<'_>,
        others: &[Self],
    ) -> Result<(), Unsupported> {
        if self.life.amount <= 0.0 {
            return Err(Unsupported(
                "a hunt's drop-off carrying nothing (set_state 3)",
            ));
        }
        let hunt = self.hunt.as_ref().expect("the caller's hunt");
        let (carry_to, state) = (hunt.carry_to, hunt.state);
        let (site, gap) = self.drop_site(&mut c, others, carry_to, state)?;
        if site == -1 {
            return Err(Unsupported(
                "a hunt without a drop site (notify 0x203, set_state 2)",
            ));
        }
        let (Some(hunt), Some(mover)) = (self.hunt.as_mut(), self.mover.as_mut()) else {
            return Err(Unsupported("a hunter without its hunt or movement state"));
        };
        hunt.carry_to = site;
        hunt.set_drop(gap, mover)
    }

    /// The drop site a gatherer's `set_state(8)` walks to, its second target `carry_to` or, with
    /// none and a master with drop sites (`+0x7a0`), the closest of them (`FUN_1411671e0`: first
    /// one it reaches, [`Unit::drop_site_from`] outside other objects, then any); −1 for none.
    /// With the footprint gap to it (`FUN_14118f680`).
    fn drop_site(
        &self,
        c: &mut DropWorld<'_>,
        others: &[Self],
        carry_to: i32,
        state: u8,
    ) -> Result<(i32, f32), Unsupported> {
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(Unsupported(
                "a gatherer of a master the slice does not have",
            ))?;
        let mover = self
            .mover
            .as_ref()
            .ok_or(Unsupported("a gatherer without movement state"))?;
        let mut site = carry_to;
        if site == -1 && master.drop_sites.first().is_some_and(|&d| d != -1) {
            let masters = c
                .masters
                .ok_or(Unsupported("a drop-off of a player without masters"))?;
            let sites = &master.drop_sites;
            site = match self.drop_site_from(c, others, state, true)? {
                Some(s) => Some(s),
                None => drop_site_search(others, masters, sites, self.position, None, true)?,
            }
            .unwrap_or(-1);
        }
        if site == -1 {
            return Ok((-1, 0.0));
        }
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        let s = c
            .spatial
            .get_known(site)?
            .ok_or(Unsupported("a drop site without a place"))?;
        Ok((
            site,
            footprint_gap(self.position, ground, s.position, s.radius),
        ))
    }

    /// `FUN_1411671e0` from the unit (`state` its action's, which the planner reads) over its
    /// master's drop sites (`+0x7a0`): [`drop_site_search`] with the unit as the seeker, by path
    /// for a human player (its argument 9, `FUN_141659cf0` false), skipping objects inside others
    /// when `outside` (its argument 10).
    fn drop_site_from(
        &self,
        c: &mut DropWorld<'_>,
        others: &[Self],
        state: u8,
        outside: bool,
    ) -> Result<Option<i32>, Unsupported> {
        let masters = c
            .masters
            .ok_or(Unsupported("a drop-off of a player without masters"))?;
        let master = masters.get(self.life.master_id).ok_or(Unsupported(
            "a gatherer of a master the slice does not have",
        ))?;
        let mover = self
            .mover
            .as_ref()
            .ok_or(Unsupported("a gatherer without movement state"))?;
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
            group: self.group.id,
            action_state: state,
        };
        let by_path = !c.computer;
        let mut plan = Planning {
            pathing: &mut *c.pathing,
            now: c.world_time,
            computer: c.computer,
            object: &mut *c.object,
            construction_rate: c.construction_rate,
            groups: c.groups,
        };
        let seeker = Seeker {
            me,
            mover,
            plan: &mut plan,
            spatial: c.spatial,
            zones,
            class: master.class,
            by_path,
        };
        drop_site_search(
            others,
            masters,
            &master.drop_sites,
            self.position,
            Some(seeker),
            outside,
        )
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

    /// The death's `set_state(3)` (from `before`, the state the object had) and the static
    /// update's dying sprite after it (slot 0x7a0, then slot 0x68, `FUN_141217000`). A building's
    /// (`FUN_1411dbb70`) first sets its annexes' states (the caller's after the update), then a
    /// built one dies ([`built_death`]) and a foundation runs the completion's first part and its
    /// construction's start ([`Unit::foundation_died`]); then the combat object's
    /// (`FUN_14121cc40`, [`combat_dies`]), whose refund from state 2 ([`refund_of`]) the caller
    /// takes from its owner's attributes; then the base one's (`FUN_14119d710`, whose storage and
    /// sight are the caller's). Not modelled: a dying sprite of another type than 70; not held:
    /// the particles and the sounds.
    fn died(&mut self, c: &mut Context<'_>, before: u8) -> Result<(), Unsupported> {
        let Some(master) = c.masters.and_then(|m| m.get(self.life.master_id)) else {
            if self.life.dying_sprite.is_some() {
                return Err(Unsupported(
                    "a dying object of a master the slice does not have",
                ));
            }
            return Ok(());
        };
        if master.kind == 80 {
            // FUN_1411dbb70: the annexes' slot 0xa0 with the same state, before anything else
            for annex in c.spatial.annexes_of(self.id) {
                c.annex_states.push((annex, 3));
            }
            if before == 2 {
                let death = built_death(self, master, c.computer)?;
                c.attribute_adds.extend(death.adds);
                c.unwatched.push(self.id);
                if let (Some(v), Some(at)) = (c.spatial.sight.as_mut(), death.unlink) {
                    v.unlink_building(at, master.radius);
                }
                c.building_death = Some(death.building);
            } else if before == 0 {
                self.foundation_died(c, master)?;
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
        combat_dies(self, master, c.spatial, c.pathing)?;
        if before == 2 {
            c.refund = refund_of(master, self.annex_of);
        }
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

    /// A foundation's death (`FUN_1411dbb70` to state 3 from state 0), before the combat
    /// object's: the completion's first part (`FUN_1411c6bd0`, [`crate::build::completion_part`]:
    /// a farm's bonus once its build points reached its build time, a mule cart's search for
    /// resources, the caller's), then, as the state is still 0 and 3 lies within 1 to 6, its
    /// construction's start (`FUN_1411e5670`, [`Construction::begin`]: a desolid one gets its
    /// record and sight, its missiles per volley its master's; its owner watches it, the
    /// caller's).
    fn foundation_died(&mut self, c: &mut Context<'_>, master: &Master) -> Result<(), Unsupported> {
        let construction = self
            .construction
            .as_mut()
            .ok_or(crate::build::NO_CONSTRUCTION)?;
        let completion = Completion {
            master: Some(master),
            annexed: false,
            attribute_204: false,
            mode_680: c.mode_680,
            farm_bonus: farm_bonus(c.attributes),
            masters: c.masters,
            computer: c.computer,
            ai: self.ai.is_some(),
            main: self.annex_of.is_some(),
            now: c.world_time,
            rice: self.rice,
        };
        let part = completion.first_part(construction)?;
        // its tracker part (FUN_14104eac0): the foundation leaves the pending lists
        let k = crate::tracker::key(c.masters, c.lines, self.life.master_id);
        c.tracker.complete(self.id, k);
        c.attribute_adds.extend(part.attributes);
        if part.cart {
            c.carts.push((self.id, 0));
        }
        let started = Construction::begin(
            self.id,
            construction,
            self.mover.as_mut(),
            c.spatial,
            c.pathing,
            c.world_time,
        )?;
        if started {
            construction.started = false;
            c.started.push(self.id);
            self.firepower = master.volley_fire;
        }
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
        let restriction = mover.stats.restriction;
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
        let target = self.resource_search(c, others, kind, cap)?;
        let spatial: &Spatial = c.spatial;
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

    /// The hunt's search for another target (`FUN_140ac4670`) when the class its unit AI targets
    /// (else the hunt's) is one the AI's slot 0x1b0 gives 0 (the civilian's `FUN_140c1c4f0`:
    /// trees, 15): with `within` (its first argument above 0; 8 when the target went on the walk
    /// to it) only a hunt within it of its point searches (the 3D distance, its components summed
    /// x, height, y); the unit AI's `ClosestResourceObject` with gather group 0, the hunt's second
    /// target as the drop site, at most 2 workers ([`Unit::resource_search_from`]) names the next
    /// tree; with `near` (its second argument above 0; 5 when the Attack's `MoveTo` failed) a tree
    /// farther than it from the unit (the same 3D distance) does not count. The hunt takes it
    /// (slot 0x48: [`crate::hunt::HuntAction::retarget`], the new target taken). True when it
    /// found one.
    fn hunt_tree_search(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
        within: Option<f32>,
        near: Option<f32>,
    ) -> Result<bool, Unsupported> {
        // NaN compares false as the game's `comiss` does
        let beyond = |a: [f32; 3], b: [f32; 3], limit: f32| {
            let (dx, dz, dy) = (a[0] - b[0], a[2] - b[2], a[1] - b[1]);
            let d = crate::fmath::sqrt(dx * dx + dz * dz + dy * dy);
            d.partial_cmp(&limit) != Some(std::cmp::Ordering::Less)
        };
        let hunt = self.hunt.as_ref().expect("the caller's");
        if let Some(within) = within
            && beyond(hunt.point, self.position, within)
        {
            return Ok(false);
        }
        let site = hunt.carry_to;
        let found = self.resource_search_from(c, others, 0, 2, site)?;
        if found == -1 {
            return Ok(false);
        }
        let (tp, t) = others
            .iter()
            .find(|u| u.id == found)
            .map(|u| (c.player, u))
            .or_else(|| c.foreign.find(found))
            .ok_or(Unsupported("a hunt's next tree the slice does not hold"))?;
        if let Some(near) = near {
            let (dx, dz, dy) = (
                self.position[0] - t.position[0],
                self.position[2] - t.position[2],
                self.position[1] - t.position[1],
            );
            if near < crate::fmath::sqrt(dx * dx + dz * dz + dy * dy) {
                return Ok(false);
            }
        }
        let t_master = c
            .every_masters
            .get(tp)
            .and_then(|m| m.get(t.life.master_id))
            .ok_or(Unsupported(
                "a hunt's next tree of a master the slice lacks",
            ))?;
        let own = c
            .spatial
            .relations
            .get(c.player)
            .ok_or(Unsupported("a hunter's player without relations"))?;
        let quarry = crate::hunt::Quarry {
            id: found,
            position: t.position,
            class: t_master.class,
            kind: t_master.kind,
            state: t.life.state,
            player: tp,
            amount: t.life.amount,
            allied: crate::tasks::allied(own, c.player, tp, true),
            top_action: t.top_action_kind(),
            drop_site: false,
        };
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(Unsupported("a hunter of a master the slice does not have"))?;
        let drops_here = master.drop_sites.contains(&t.life.master_id);
        let at = |id| {
            c.spatial
                .get_known(id)?
                .ok_or(Unsupported("a hunt's object without a place"))
        };
        let (me_at, them) = (at(self.id)?, at(found)?);
        let speed = self.mover.as_ref().map_or(0.0, |m| m.stats.speed);
        let engagement =
            crate::attack::engagement(self.ai.as_ref(), speed, master, me_at, them, c.world_time)?;
        let hunt = self.hunt.as_mut().expect("the caller's");
        let old = hunt.target;
        let moving = hunt
            .task
            .walk_sprite_id(self.life.attribute_type, self.life.amount);
        let arms = crate::attack::arms(master, c.graphics, hunt.task.sprites, moving);
        let mover = self
            .mover
            .as_mut()
            .ok_or(Unsupported("a hunter without movement state"))?;
        if let Some(arrived) = hunt.retarget(
            &quarry,
            c.player,
            self.life.amount,
            drops_here,
            &engagement,
            arms,
            mover,
        )? {
            if arrived {
                crate::attack::show_arrived(
                    &arms,
                    &mut self.sprite,
                    &mut self.anim_speed,
                    c.streams.get(Stream::Sprite),
                )?;
            }
            // slot 0x68 (FUN_140a7a230): the old target goes, the new one is taken
            self.hunt_release(c, [old]);
            c.takes.push(found);
        }
        Ok(true)
    }

    /// The unit AI's `ClosestResourceObject` (slot 0x80, `FUN_140c49760`) with no drop site, no
    /// point and `param_6` 2 ([`crate::gather_search::closest_resource`]): the closest object of
    /// gather group `kind` with fewer than `cap` workers (−1 any) the unit reaches (slot 0x3d0
    /// with range 0), from the tiles the player explored that the unit's restriction lets it
    /// stand on; −1 for none.
    fn resource_search(
        &self,
        c: &mut Context<'_>,
        others: &[Self],
        kind: u8,
        cap: i32,
    ) -> Result<i32, Unsupported> {
        self.resource_search_from(c, others, kind, cap, -1)
    }

    /// [`Unit::resource_search`] from the drop site `site` (−1 for none;
    /// [`crate::gather_search::closest_resource_at`]): the site's place, its master's radii and
    /// line of sight, and whether the search starts from its footprint (the site's tile in the
    /// unit's zone of its restriction, or a dock: masters 45 and 2172, `0x140c49d17`).
    fn resource_search_from(
        &self,
        c: &mut Context<'_>,
        others: &[Self],
        kind: u8,
        cap: i32,
        site: i32,
    ) -> Result<i32, Unsupported> {
        let current = self
            .ai
            .as_ref()
            .ok_or(Unsupported("a resource search without a unit AI"))?
            .current_target;
        let mut w = ResourceWorld {
            player: c.player,
            spatial: c.spatial,
            zones: c.zones,
            foreign: c.foreign,
            every: c.every_masters,
            masters: c.masters,
            plan: Planning {
                pathing: c.pathing,
                now: c.world_time,
                computer: c.computer,
                object: c.streams.get(Stream::Object),
                construction_rate: c.construction_rate,
                groups: c.groups,
            },
        };
        closest_resource_of(self, &mut w, others, (kind, cap, site), (current, 2)).map(|(id, _)| id)
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
        let in_the_way = ai
            .notifies
            .iter()
            .filter(|n| n.kind == crate::unit_ai::IN_THE_WAY)
            .map(|n| n.p[0]);
        let mut ids: Vec<i32> = ai
            .attackers
            .iter()
            .copied()
            .chain(damaged)
            .chain(in_the_way)
            .collect();
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

    pub(crate) fn post_own(&mut self, kind: i32, p: [i32; 3]) -> Result<(), Unsupported> {
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
            in_the_way: Vec::new(),
            sprite_facets: facets,
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
                let before = self.life.state;
                self.life.hp = 0.0;
                self.life.goto_sleep = false;
                self.life.state = 7;
                c.spatial.sight_state(self.id, 7);
                if before < 4 {
                    let master = c.masters.and_then(|m| m.get(self.life.master_id));
                    forget_resource(c.spatial, master, self.id, self.position);
                }
            }
        }
        Ok(())
    }

    /// The gate's update (`FUN_1411e7730`, `TRIBE_Building_Object::update` for a master of class
    /// 39 outside the walls' branch): a gate in state 2 whose master transforms
    /// (`transform_unit`, `+0x9ba`) clears its first update (its endpoints, not held). Unless it
    /// is shut (its master's height above [`GATE_EPSILON`]) and locked, it counts the units in it
    /// ([`Unit::gate_count`]): a shut, unlocked gate with one of an ally (its player counting the
    /// unit's an ally) in it opens (its player's slot 0x340, `FUN_14162f7e0`: a gate of a height
    /// of at least 1, [`Turns::toggle_gate`]) and its close timer is 0; an open one (a height
    /// below the epsilon) with nothing in it starts its close timer at 0.5 s, or runs it down by
    /// the world's dt, and at 0 shuts (slot 0x338, `FUN_1416193e0`: a height of at most 0).
    fn gate_update(&mut self, c: &mut Context<'_>, others: &[Self]) -> Result<(), Unsupported> {
        let Some(master) = c.masters.and_then(|m| m.get(self.life.master_id)) else {
            return Ok(());
        };
        let transforms = master
            .building
            .as_ref()
            .is_some_and(|b| b.transform_unit != -1);
        if master.class != crate::spatial::GATE_CLASS || !transforms || self.life.state != 2 {
            return Ok(());
        }
        let height = master.height;
        let gate = self
            .gate
            .as_mut()
            .ok_or(Unsupported("a gate without its gate state"))?;
        gate.first_update = false;
        let shut = GATE_EPSILON < height;
        if gate.locked && shut {
            return Ok(());
        }
        let locked = gate.locked;
        let (total, allied) = self.gate_count(c, others)?;
        let gate = self.gate.as_mut().expect("held above");
        if shut && 0 < allied && !locked {
            c.gate_toggle = GATE_SHUT_HEIGHT <= height;
            gate.close_timer = 0.0;
            return Ok(());
        }
        if height < GATE_EPSILON && total == 0 {
            if gate.close_timer <= 0.0 {
                gate.close_timer = GATE_CLOSE_DELAY;
            } else {
                gate.close_timer -= c.dt_seconds;
                if gate.close_timer <= 0.0 {
                    gate.close_timer = 0.0;
                    c.gate_toggle = height <= 0.0;
                }
            }
        }
        Ok(())
    }

    /// [`gate_units`] of this gate, its annexes the units of `others` that are its (in id order,
    /// as made): the units in it and how many of them are of an ally.
    fn gate_count(&self, c: &Context<'_>, others: &[Self]) -> Result<(usize, usize), Unsupported> {
        let (units, allied) = gate_units(c.spatial, self.id, &annexes_of(self.id, others))?;
        Ok((units.len(), allied))
    }

    /// `TRIBE_Building_Object::update`'s farm part for a foundation (state 0) with a build time
    /// (`0x1411ca2ef`): its stage, its build points times 3 over its build time (truncated, at
    /// most 2), lays terrain 29 plus the stage under a farm (master 50) each update
    /// (`FUN_1411ce250` -> `FUN_1411ddd90`, [`crate::terrain::Terrain::cover`]); the rice farms'
    /// (1187, 1889: their foundation terrain plus 2, or 65, plus the stage by `+0xcd0`) are not
    /// modelled.
    fn farm_stage(&self, c: &mut Context<'_>) -> Result<(), Unsupported> {
        /// The stages over the build time (`DAT_142eb1c98`).
        const STAGES: f32 = crate::image::turn::FARM_STAGES;
        /// A farm's first stage terrain (`0x1d`).
        const FIRST_STAGE: u8 = 29;
        let Some(cons) = self
            .construction
            .filter(|k| self.life.state == 0 && 0 < k.build_time)
        else {
            return Ok(());
        };
        let Some(m) = c.masters.and_then(|m| m.get(self.life.master_id)) else {
            return Ok(());
        };
        match m.id {
            50 => {}
            1187 | 1889 => return Err(Unsupported("a rice farm's stage terrain (+0xcd0)")),
            _ => return Ok(()),
        }
        let stage = ((cons.progress * STAGES) / f32::from(cons.build_time)) as i32 as u8;
        let f = crate::terrain::Foundation {
            position: self.position,
            clearance: m.clearance,
            terrain: FIRST_STAGE + stage.min(2),
            snow: false,
            master: m.id,
            class: m.class,
            terrain_bonus: m.terrain_bonus,
            mode_680: c.mode_680,
        };
        c.spatial
            .terrain
            .as_mut()
            .ok_or(Unsupported("a farm's foundation without the map's terrain"))?
            .cover(&f)
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
        self.farm_stage(c)?;
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
            firepower: Some(&mut self.firepower),
            roo_called: Some(&mut self.roo_called),
            store: Some(&mut self.store),
            completion: Completion {
                master,
                annexed,
                attribute_204: c.attribute_204,
                mode_680: c.mode_680,
                farm_bonus: farm_bonus(c.attributes),
                masters: c.masters,
                computer: c.computer,
                ai: self.ai.is_some(),
                main: self.annex_of.is_some(),
                now: c.world_time,
                rice: self.rice,
            },
            chain: crate::build::WorkChain::default(),
        };
        let built = Construction::complete(&mut site, c.spatial, c.pathing)?;
        // the completion's first part's tracker part (FUN_1411c6bd0 -> FUN_14104eac0)
        let k = crate::tracker::key(c.masters, c.lines, self.life.master_id);
        c.tracker.complete(self.id, k);
        // its set_state(2) took its annexes along first (FUN_1411dbb70)
        for annex in c.spatial.annexes_of(self.id) {
            c.annex_states.push((annex, 2));
        }
        c.attribute_adds.extend(built.attributes);
        c.techs_done.extend(built.tech);
        if let Some(state) = built.cart {
            c.carts.push((self.id, state));
        }
        if built.started {
            if let Some(k) = self.construction.as_mut() {
                k.started = false;
            }
            c.started.push(self.id);
        }
        if built.stack.is_some() || built.vanishes {
            c.completions.push((self.id, built.stack, built.vanishes));
        }
        Ok(())
    }

    /// `TRIBE_Building_Object::update`'s whole seconds of a building in state 2 (`0x1411c9000`,
    /// before its combat update): `FUN_1411c39b0` (attribute 255 naming its master: a terrain
    /// effect around it, not modelled); then for each whole second since `last_garrison_time`
    /// (`+0xbe0`, which moves on by them): a monastery's (master 104) relics give their goods
    /// (not modelled with relics inside); each unit inside (from the last) whose
    /// `raider_build_countdown` (`+0x9a0`) runs would be let out at its end (not modelled); the
    /// units inside (from the first) heal by the master's `garrison_heal_rate` below their
    /// master's hit points (slot 0x110, [`Unit::heal`]); with villagers inside, a damaged
    /// building with a `garrison_repair_rate` repairs itself at a cost (`FUN_1411d33e0`, not
    /// modelled). The `damaged_lately_timer` (`+0x30d`) a second takes off is not held.
    fn building_seconds(
        &mut self,
        c: &Context<'_>,
        others: &mut [Self],
    ) -> Result<(), Unsupported> {
        if self.life.state != 2 {
            return Ok(());
        }
        let Some(master) = c.masters.and_then(|m| m.get(self.life.master_id)) else {
            return Ok(());
        };
        let Some(bm) = master.building else {
            return Ok(());
        };
        // at most a fifth of its hit points (DAT_142eae8d0), it lets its garrison out
        // (FUN_1411e7220 for each from the last, then their slots 0x1a8 and 0xd0); its master's
        // slot 0x110 only sets a sound flag
        if self.life.hp <= f32::from(master.hit_points) * EJECT_SHARE && !self.garrison.is_empty() {
            return Err(Unsupported(
                "a damaged building lets its garrison out (a fifth of its hit points)",
            ));
        }
        if c.attributes.get(BUILDING_TERRAIN) == Some(&f32::from(master.id)) {
            return Err(Unsupported(
                "a building whose master attribute 255 names (FUN_1411c39b0)",
            ));
        }
        let seconds = c.world_time.wrapping_sub(self.garrison_time) / 1000;
        if seconds == 0 {
            return Ok(());
        }
        self.garrison_time = self.garrison_time.wrapping_add(seconds * 1000);
        if master.id == MONASTERY && self.relics != [0; 2] {
            return Err(Unsupported("a monastery's relics' goods (its seconds)"));
        }
        let inside = |id: i32, others: &[Self]| others.iter().position(|u| u.id == id);
        for &id in self.garrison.iter().rev() {
            let Some(i) = inside(id, others) else {
                continue;
            };
            let kind = c
                .masters
                .and_then(|m| m.get(others[i].life.master_id))
                .map(|m| m.kind);
            if kind == Some(70) && 0 < others[i].raider_countdown {
                return Err(Unsupported(
                    "a unit inside whose raider_build_countdown runs (FUN_1411e7220)",
                ));
            }
        }
        let rate = bm.garrison_heal_rate;
        let mut villagers = 0;
        for &id in &self.garrison {
            let Some(i) = inside(id, others) else {
                continue;
            };
            let m = c
                .masters
                .and_then(|m| m.get(others[i].life.master_id))
                .ok_or(Unsupported(
                    "a unit inside of a master the slice does not have",
                ))?;
            if m.class == VILLAGER_CLASS {
                villagers += 1;
            }
            if 0.0 < rate && others[i].life.hp < f32::from(m.hit_points) {
                others[i].heal(m, rate)?;
            }
        }
        if 0 < villagers
            && 0.0 < bm.garrison_repair_rate
            && self.life.hp < f32::from(master.hit_points)
        {
            return Err(Unsupported(
                "a building its villagers inside repair (FUN_1411d33e0)",
            ));
        }
        Ok(())
    }

    /// Slot 0x110 of a combat object (`FUN_141213b10`): the classes 2, 20, 21, 22 and 53 repair
    /// at a cost (not modelled); any other heals (`FUN_141193b30`): slot 0xb8 (`FUN_141203be0`)
    /// adds `amount` up to its hit points with its powerups' room, an overflow taken off the
    /// powerup (`+0xa38`, never below 0; the hit point transforms of `FUN_141204450`, empty in
    /// the recordings, are not held); healed to its full hit points (slot 0xc0 at least slot
    /// 200), it holds exactly its master's. The heal timer `+0x9e4` a full heal clears is not held.
    fn heal(&mut self, master: &Master, amount: f32) -> Result<(), Unsupported> {
        if matches!(master.class, 2 | 20 | 21 | 22 | 53) {
            return Err(Unsupported("a heal at a cost (FUN_141213b10's classes)"));
        }
        let full = f32::from(master.hit_points);
        let [mut up, room] = self.hp_powerups;
        let most = full + room;
        if room < up {
            up = room;
        }
        if most <= self.life.hp {
            if 0.0 < up {
                up = (up - amount).max(0.0);
            }
        } else {
            let healed = self.life.hp + amount;
            self.life.hp = healed;
            if most < healed {
                self.life.hp = most;
                if 0.0 < up {
                    up = (up - (healed - most)).max(0.0);
                }
            }
        }
        self.hp_powerups[0] = up;
        // slot 0xc0 (the hit points with the powerups' room) at least slot 200 (the most)
        if most <= (room - up) + self.life.hp {
            self.life.hp = full;
        }
        Ok(())
    }

    /// One update, and whether the object stays in its list, leaves it or goes to sleep. `others`
    /// are the rest of the owner's objects, whose foundations a builder works on.
    /// The animated object's idle swap (the end of `RGE_Animated_Object::update`,
    /// `0x141071880`): an object whose master has a second standing sprite (`+0x478`) and whose
    /// slot 0x5d8 answers 0 (an animated object's always; a moving one's while its master's move
    /// algorithm, `+0x768`, has bit 0 clear), showing either standing sprite, whose sprite's node
    /// looped in this update (the sprite list's slot 0x40, the node's slot 12), draws on the
    /// Sprite stream ("Update"); above 0x3fff it shows the other (`FUN_141189760` with the other
    /// index and 0x7fff: `+0x78c` takes the index, slot 0x68 the master's sprite of that index,
    /// [`crate::attack::set_sprite`], and the list's slot 0x28 puts its node back to its first
    /// frame, [`crate::sprite::ActiveSprite::reset_frame`]).
    fn idle_swap(
        &mut self,
        master: &Master,
        graphics: &std::collections::BTreeMap<i16, Graphic>,
        sprite_rng: &mut Well512,
    ) {
        let Some(second) = master.standing_sprite_2 else {
            return;
        };
        if self
            .mover
            .as_ref()
            .is_some_and(|m| m.stats.move_algorithm & 1 != 0)
        {
            return;
        }
        let Some(shown) = self.sprite.as_ref() else {
            return;
        };
        let id = shown.graphic.id;
        if (id != master.standing_sprite && id != second) || !shown.looped {
            return;
        }
        if sprite_rng.rand15() <= 0x3fff {
            return;
        }
        self.idle_variant = i32::from(self.idle_variant == 0);
        let next = if self.idle_variant == 0 {
            master.standing_sprite
        } else {
            second
        };
        if let Some(&g) = graphics.get(&next) {
            crate::attack::set_sprite(&mut self.sprite, g, sprite_rng);
        }
        if let Some(s) = self.sprite.as_mut() {
            s.reset_frame();
        }
    }

    fn update(&mut self, c: &mut Context<'_>, others: &mut [Self]) -> Result<Step, Unsupported> {
        if let Some(resume) = c.ai_resume.take() {
            return self.ai_part(c, others, resume);
        }
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
        // the yaw and the facing read the current sprite's facets
        if let (Some(m), Some(s)) = (self.mover.as_mut(), self.sprite) {
            m.sprite_facets = Some(crate::facing::SpriteFacets {
                count: s.graphic.facets,
                flag_1: s.graphic.flags & 2 != 0,
            });
        }
        self.gate_update(c, others)?;
        self.complete_on_update(c, others)?;
        self.building_seconds(c, others)?;
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
            let at = mover.step_velocity(
                self.position,
                c.dt_seconds,
                c.spatial.map,
                c.spatial.terrain.as_ref(),
                c.world_time,
            );
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
            // set_state(3)'s storage (FUN_1411a56c0 with 0): each of kinds 2 and 16 of an
            // attribute leaves the player (its slot 0xf8 with the amount negated)
            if let Some(m) = c.masters.and_then(|m| m.get(self.life.master_id)) {
                for &(attribute, amount, kind) in &m.storage {
                    if matches!(kind, 2 | 16) && 0 <= attribute {
                        c.attribute_adds.push((attribute, -amount));
                    }
                }
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
            self.died(c, before)?;
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
            if before < 4 && 6 < after {
                let master = c.masters.and_then(|m| m.get(self.life.master_id));
                forget_resource(c.spatial, master, self.id, self.position);
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
        // the animated object's update (0x141071880): its sprite's clock at its animation speed,
        // then its idle swap
        if let (Some(sprite), Some(speed)) = (self.sprite.as_mut(), self.anim_speed) {
            sprite.update(c.dt_seconds, speed);
            // the swap reads the loop this update's clock made
            if let Some(master) = c.masters.and_then(|m| m.get(self.life.master_id)) {
                self.idle_swap(master, c.graphics, c.streams.get(Stream::Sprite));
            }
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
            // a Make's teardown is the base one's: it holds no target; a wait for a foundation's
            // too
            self.make = None;
            self.wait_solidify = None;
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
        let mut makes_from_now = false;
        if self.life.state == 2
            && self.top_action_kind().is_none()
            && self.production.is_none()
            && std::mem::take(&mut self.default_make)
        {
            let task = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .and_then(Master::default_task)
                .copied()
                .ok_or(Unsupported("a Make whose master the slice does not have"))?;
            self.make = Some(crate::make::MakeAction::start(
                task,
                c.attributes,
                &mut crate::make::Held {
                    attribute_type: &mut self.life.attribute_type,
                    amount: &mut self.life.amount,
                },
            ));
            makes_from_now = true;
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
            group: self.group.id,
            action_state: self.top_action().map_or(0, |a| a.state),
        };
        let mut captured = None;
        if let Some(artifact) = self.artifact.as_mut().filter(|_| c.tick > 1) {
            let dt = c.dt;
            let ended = artifact.walk(Some(c.walk(self.mover.as_mut(), me)))?;
            // an ended MoveTo (codes 1 to 5) goes (FUN_140a98620): its teardown stops the unit
            if ended.is_some()
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
        // what the footprint clearing of the Build's set_state(7) reads (FUN_1415c3620) of
        // the objects on the tiles about a desolid foundation, as the update starts
        let mut looks = std::collections::BTreeMap::new();
        if let Some(f) = self
            .build
            .filter(|b| matches!(b.state, 6 | 222))
            .and_then(|b| others.iter().find(|u| u.id == b.target))
            .filter(|u| u.construction.is_some_and(|k| k.desolid))
        {
            let [fx, fy, _] = f.position;
            let reach = crate::build::CLEARING_REACH + 1.0;
            for (id, _) in c
                .spatial
                .objects_within(fx - reach, fy - reach, fx + reach, fy + reach)
            {
                let (owner, u) = if id == self.id {
                    (c.player, &*self)
                } else if let Some(u) = others.iter().find(|u| u.id == id) {
                    (c.player, u)
                } else if let Some((q, u)) = c.foreign.find(id) {
                    (q, u)
                } else {
                    continue;
                };
                if let Some(l) = clear_look_of(u, c.every_masters.get(owner)) {
                    let _ = looks.insert(id, l);
                }
            }
        }
        let by = self.releaser(c);
        if let Some(build) = self.build.as_mut() {
            let target = build.target;
            let annexed = others.iter().any(|u| u.annex_of == Some(target));
            let masters = c.masters;
            let (attribute_204, mode_680, computer) = (c.attribute_204, c.mode_680, c.computer);
            let world_time = c.world_time;
            let farm_bonus = farm_bonus(c.attributes);
            let built = others
                .iter()
                .find(|u| u.id == target)
                .map(|u| u.life.master_id);
            // the foundations its unit AI's queued build orders name, as they stand
            let build_targets: Vec<(i32, Option<u8>)> =
                self.ai.as_ref().map_or_else(Vec::new, |ai| {
                    ai.orders
                        .iter()
                        .filter(|o| o.order == BUILD_ORDER)
                        .map(|o| {
                            let state = others
                                .iter()
                                .find(|u| u.id == o.target)
                                .map(|u| u.life.state);
                            (o.target, state)
                        })
                        .collect()
                });
            // FUN_1411cc430's way to the building that takes the work: while the foundation is an
            // annex of a main building in state 0 with at least 1 hit point (COMISS: a NaN too),
            // that main building; then the foundation's annexes of a building type, which its
            // work starts first (none when the work goes on to a main building: the caller's)
            let mut worked = target;
            let mut main_work = None;
            while let Some(m) = others
                .iter()
                .find(|u| u.id == worked)
                .and_then(|u| u.annex_of)
                .and_then(|m| others.iter().find(|u| u.id == m))
                .filter(|m| m.life.state == 0 && crate::fmath::at_most_or_unordered(1.0, m.life.hp))
            {
                worked = m.id;
                main_work = Some(crate::build::MainWork {
                    construction: m.construction.ok_or(crate::build::NO_CONSTRUCTION)?,
                    hp: m.life.hp,
                });
            }
            let annex_ids = c.spatial.annexes_of(target);
            let mut site = None;
            let mut starts = Vec::new();
            for u in others.iter_mut() {
                if u.id == target {
                    site = Some(u);
                    continue;
                }
                if main_work.is_some() || !annex_ids.contains(&u.id) {
                    continue;
                }
                let Some(am) = masters
                    .and_then(|m| m.get(u.life.master_id))
                    .filter(|m| BUILDING_KIND <= m.kind)
                else {
                    continue;
                };
                let construction = u
                    .construction
                    .as_mut()
                    .ok_or(crate::build::NO_CONSTRUCTION)?;
                starts.push(crate::build::AnnexStart {
                    id: u.id,
                    construction,
                    mover: u.mover.as_mut(),
                    firepower: &mut u.firepower,
                    volley: am.volley_fire,
                });
            }
            // the slot order: the constructor made them, and their ids, in it
            starts.sort_by_key(|a| a.id);
            let site = site.and_then(|u| {
                let master = masters.and_then(|m| m.get(u.life.master_id));
                let (ai, main, rice) = (u.ai.is_some(), u.annex_of.is_some(), u.rice);
                Some(Site {
                    id: u.id,
                    life: &mut u.life,
                    construction: u.construction.as_mut()?,
                    mover: u.mover.as_mut(),
                    firepower: Some(&mut u.firepower),
                    roo_called: Some(&mut u.roo_called),
                    store: Some(&mut u.store),
                    completion: Completion {
                        master,
                        annexed,
                        attribute_204,
                        mode_680,
                        farm_bonus,
                        masters,
                        computer,
                        ai,
                        main,
                        now: world_time,
                        rice,
                    },
                    chain: crate::build::WorkChain {
                        annexes: starts,
                        main: main_work,
                    },
                })
            });
            let class = masters
                .and_then(|m| m.get(self.life.master_id))
                .map_or(-1, |m| m.class);
            // FUN_140fde180: the foundation's master is a drop site (the masters 562, 584, 621,
            // 68, 1808, 50, 1187 and 199, or one the drop-site table has for every civ or the
            // builder's)
            let drop_site = built.is_some_and(|m| {
                DROP_SITE_MASTERS.contains(&m)
                    || c.drop_sites
                        .iter()
                        .any(|d| d.building == m && (d.civs.is_empty() || d.civs.contains(&c.civ)))
            });
            // FUN_140aadc50 drops only a kind the player has (FUN_140efe5b0)
            let held_kind = self.life.attribute_type;
            let has_kind = usize::try_from(held_kind).is_ok_and(|k| k < c.attributes.len());
            let builder = Builder {
                class,
                master: self.life.master_id,
                held: self.life.amount,
                held_kind,
                drop_site: drop_site && has_kind,
                ai: self.ai.as_mut(),
                waits: &mut self.footprint_waits,
                build_targets: &build_targets,
                look: crate::build::Looks(&|id| looks.get(&id).copied()),
            };
            let step = build.update(c.walk(self.mover.as_mut(), me), site, builder)?;
            c.cleared.extend(step.cleared.iter().copied());
            if !step.occupants.is_empty() {
                c.evacuations.push((target, step.occupants.clone()));
            }
            // the constructions that started, its annexes' first: their owner watches each
            // (FUN_1411e5670), the caller's
            for id in annex_ids.iter().copied().chain([target]) {
                if let Some(cons) = others
                    .iter_mut()
                    .find(|u| u.id == id)
                    .and_then(|u| u.construction.as_mut())
                    .filter(|cons| cons.started)
                {
                    cons.started = false;
                    c.started.push(id);
                }
            }
            // the work an annex's main building takes, the caller's
            if let Some(amount) = step.main_work {
                c.main_works.push((target, amount));
            }
            if let Some(built) = step.completed {
                // the completion's first part's tracker part (FUN_1411c6bd0 ->
                // FUN_14104eac0); the foundation is the builder's player's
                if let Some(m) = others.iter().find(|u| u.id == target) {
                    let k = crate::tracker::key(c.masters, c.lines, m.life.master_id);
                    c.tracker.complete(target, k);
                }
                // its set_state(2) took its annexes along first (FUN_1411dbb70)
                for annex in c.spatial.annexes_of(target) {
                    c.annex_states.push((annex, 2));
                }
                c.attribute_adds.extend(built.attributes);
                c.techs_done.extend(built.tech);
                if let Some(state) = built.cart {
                    c.carts.push((target, state));
                }
                if built.stack.is_some() || built.vanishes {
                    c.completions.push((target, built.stack, built.vanishes));
                }
            }
            // the load dropped at the end of the work (FUN_140aadc50: the player's slot 0xf8 with
            // 1 and 1, then the builder carries nothing, FUN_14119d310)
            if let Some((kind, amount)) = step.dropped {
                give_load(
                    (&c.spatial.relations, c.sharing),
                    c.player,
                    &mut c.attribute_adds,
                    kind,
                    amount,
                )?;
                self.life.attribute_type = -1;
                self.life.amount = 0.0;
            }
            if step.done {
                // the action list removes it; its destructor (FUN_140aadbf0) finds it out of the
                // builders (set_state left them) and releases the foundation it still holds
                // (slot 0x180)
                let holds = self.build.is_some_and(|b| b.target != -1);
                self.build = None;
                if holds {
                    release_own(others, masters, target, by)?;
                }
            }
            if step.released {
                release_own(others, masters, target, by)?;
            }
            if step.notified {
                self.post_own(SEARCH_NEEDED, [BUILD_ACTION, 0, 0])?;
            }
            if step.works_it {
                // slot 0x7e8 on the building it built, at its place (the caller's work command)
                let point = c
                    .spatial
                    .get(target)
                    .ok_or(Unsupported("a builder's building without a place"))?
                    .position;
                c.work_order = Some(crate::order::Work {
                    target,
                    point,
                    carry_test: false,
                    filter: -1,
                    notified: true,
                });
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
                    self.post_own(SEARCH_NEEDED, [BUILD_ACTION, 0, 0])?;
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
        // the wait for a foundation (FUN_140ae8b70) on its build points, none when the world
        // does not hold it; its 1 removes it (FUN_140a98e20: the base teardown lets no target go,
        // and no action follows it, the wait having replaced the list)
        if let Some(w) = self.wait_solidify
            && w.update(wait_progress(w.target, others, c.foreign))
        {
            self.wait_solidify = None;
        }
        if self.missile.as_ref().is_some_and(|m| m.action.is_some()) {
            self.fly(c, others)?;
        }
        let step = match self.move_to.as_mut() {
            Some(move_to) => move_to.update(c.walk(self.mover.as_mut(), me))?,
            None => crate::move_to::MoveStep::default(),
        };
        if step.gave_up {
            // LAB_140aa320c: a human's villager waits once (set_state 2, 1 s or 2 s by its
            // stance); any other unit's MoveTo fails (set_state 13), which its unit AI's check
            // between turns may interrupt with its next order the same update
            let villager = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .is_some_and(|m| m.class == 4);
            let stance = self.ai.as_ref().map_or(0, |ai| ai.stance);
            if let Some(m) = self.move_to.as_mut() {
                m.state = m.wait.give_up(villager && !c.computer, stance);
            }
        }
        if step.target_moved {
            self.post_own(TARGET_MOVED, [0, 0, 0])?;
        }
        if step.done || step.failed {
            // its update in state 1 posts its completion (0x1fa with the move action value; an
            // own MoveTo is never quiet), in state 13 its failure (0x1f9) and returns 3; the
            // action list removes it (FUN_140a98e20): its teardown (0x140aa1f30) stops the unit
            // (slot 0x5d0)
            let notify = if step.failed {
                crate::unit_ai::ACTION_FAILED
            } else {
                ACTION_COMPLETED
            };
            self.post_own(notify, [crate::unit_ai::MOVE_ACTION, 0, 0])?;
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
                    // the motion step giving up: a human player's villager waits once, any
                    // other unit's MoveTo fails (LAB_140aa320c)
                    let villager = c
                        .masters
                        .and_then(|m| m.get(self.life.master_id))
                        .is_some_and(|m| m.class == 4);
                    let waits = villager && !c.computer;
                    let stance = self.ai.as_ref().map_or(0, |ai| ai.stance);
                    let code = match explore.move_to {
                        Some(MoveTo::Start) => {
                            if explore.move_wait.state == 1 {
                                explore.move_wait.state = 2;
                            }
                            explore.move_to = Some(MoveTo::Moving);
                            let goal = crate::move_to::Goal::Point(explore.point);
                            let walk = c.walk(self.mover.as_mut(), me);
                            explore.move_to = Some(
                                match crate::move_to::start_walk_or_give_up(walk, goal, 0.0, quiet)?
                                {
                                    Some(state) => state,
                                    None => explore.move_wait.give_up(waits, stance),
                                },
                            );
                            0
                        }
                        Some(MoveTo::Moving) => {
                            let walk = c.walk(self.mover.as_mut(), me);
                            explore.move_to = Some(match crate::move_to::walk_or_give_up(walk)? {
                                Some(state) => state,
                                None => explore.move_wait.give_up(waits, stance),
                            });
                            0
                        }
                        // case 2: the wait runs by dt, then the MoveTo asks again (set_state 3)
                        Some(MoveTo::Waiting) => {
                            explore.move_wait.timer += c.dt_seconds;
                            if explore.move_wait.delay <= explore.move_wait.timer {
                                explore.move_wait.timer = 0.0;
                                explore.move_to = Some(MoveTo::Start);
                            }
                            0
                        }
                        // its update in state 13 returns 3 (quiet: no notify)
                        Some(MoveTo::Failed) => 3,
                        Some(MoveTo::Arrived) | None => 1,
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
            let step = fly.update(walk, sky)?;
            if step.stopped {
                mover.stop()?;
            }
            // a failed flight's MoveTo posts its failure (notify 0x1f9, the move action value)
            if step.failed {
                self.post_own(
                    crate::unit_ai::ACTION_FAILED,
                    [crate::unit_ai::MOVE_ACTION, 0, 0],
                )?;
            }
        }
        if let Some(make) = self.make.as_mut().filter(|_| !makes_from_now) {
            let master = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(Unsupported("a Make of a master the slice does not have"))?;
            let farm = crate::make::Farm {
                master_id: master.id,
                kind: master.kind,
                class: master.class,
                work_rate: master.work_rate + self.work_rate_powerup,
                capacity: master.capacity,
                dt: c.dt_seconds,
            };
            let step = make.update(
                &mut crate::make::Held {
                    attribute_type: &mut self.life.attribute_type,
                    amount: &mut self.life.amount,
                },
                farm,
            );
            if let Some(add) = step.to_player {
                c.attribute_adds.push(add);
            }
            // slot 0xd8 (FUN_14118f580): no hit points left; the object is in its list, so not
            // asleep, and the special-object list (+0x30c bit 0) is not held
            if step.exhausted {
                self.life.hp = 0.0;
            }
            // the action list's result 1 removes it (FUN_140a98e20)
            if step.done {
                self.make = None;
            }
        }
        if self.gather.is_some() {
            self.gather_update(c, others, me)?;
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
        // a hunt whose target went on the walk to it (state 4, its MoveTo quiet: code 5), or
        // whose MoveTo failed (code 3), and whose class is a tree's searches for the next tree
        // (FUN_140ac4670 with 8 and -1 both times)
        let tree_lost = self.hunt.as_ref().is_some_and(|h| {
            h.state == crate::hunt::WALK
                && 0.0 <= h.timer
                && ((h.target == -1 && h.move_to.is_some()) || h.move_to == Some(MoveTo::Failed))
        }) && self.ai.as_ref().is_some_and(|ai| {
            let class = if ai.target_type == -1 {
                self.hunt.map_or(-1, |h| i32::from(h.target_class))
            } else {
                ai.target_type
            };
            class == crate::hunt::TREE_CLASS
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
            let shows = hunt_walk_sprite(hunt, c.graphics, &self.life);
            let who = crate::hunt::PointWalker {
                computer: c.computer,
                villager: master.class == 4,
                stance: self.ai.as_ref().map_or(0, |ai| ai.stance),
            };
            let (walk, rng) = c.walk_and_sprites(self.mover.as_mut(), me);
            let shown = crate::hunt::Shown {
                sprite: &mut self.sprite,
                rng,
                graphic: shows,
            };
            match hunt.update_drop(
                walk,
                &mut self.life.attribute_type,
                &mut self.life.amount,
                attributes,
                site,
                shown,
                who,
                &mut self.anim_speed,
            )? {
                crate::hunt::Dropped::Going => {}
                crate::hunt::Dropped::Load(kind, amount) => {
                    // the player's slot 0xf8 (FUN_141616820) with 1 and 1: the resource, and its
                    // gathered total (166 to 168; gold's 49 and 169); the statistics (slot 0xb8)
                    // and the local player's tracker (FUN_14097daf0) are not held; a sharing
                    // player's allies' share (+0x1426) is not modelled
                    give_load(
                        (&c.spatial.relations, c.sharing),
                        c.player,
                        &mut c.attribute_adds,
                        kind,
                        amount,
                    )?;
                    self.hunt_after_drop(c, others)?;
                }
                crate::hunt::Dropped::Again => {
                    // the drop site goes (FUN_140a7a190 with none), then set_state(8)
                    let hunt = self.hunt.as_mut().expect("above");
                    let site = std::mem::replace(&mut hunt.carry_to, -1);
                    self.hunt_release(c, [site]);
                    self.hunt_drop_off(c, others)?;
                }
                crate::hunt::Dropped::Idle => {
                    return Err(Unsupported("a hunt's drop-off idling (set_state 2)"));
                }
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
                if others.iter().any(|u| u.id == target) {
                    let by = self.releaser(c);
                    release_own(others, c.masters, target, by)?;
                } else {
                    // another player's (Gaia's deer, a neighbour's sheep): its slot 0x180 after
                    // this update ([`Turns::release`]), as the other teardowns' releases
                    self.hunt_release(c, [target]);
                }
            }
        } else if let Some(hunt) = self
            .hunt
            .as_mut()
            .filter(|h| matches!(h.state, crate::hunt::WALK_POINT | crate::hunt::WALK_SEARCH))
        {
            // states 15 and 11: the walk to a point; state 15's end is set_state(2): the unit
            // takes the hunt's sprite to stand with (slot 0x68 with FUN_140ac5210's), the target
            // and the second target go; state 11's is set_state(3), the search
            let master = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(Unsupported("a hunter of a master the slice does not have"))?;
            let who = crate::hunt::PointWalker {
                computer: c.computer,
                villager: master.class == 4,
                stance: self.ai.as_ref().map_or(0, |ai| ai.stance),
            };
            let shows = hunt_walk_sprite(hunt, c.graphics, &self.life);
            let (walk, rng) = c.walk_and_sprites(self.mover.as_mut(), me);
            let shown = crate::hunt::Shown {
                sprite: &mut self.sprite,
                rng,
                graphic: shows,
            };
            match hunt.update_point_walk(walk, who, shown)? {
                crate::hunt::PointWalk::Walking => {}
                crate::hunt::PointWalk::Search => self.hunt_search_state(c, others)?,
                crate::hunt::PointWalk::Done { target, second } => {
                    let task = hunt.task;
                    self.hunt_stand(c, &task)?;
                    self.hunt_release(c, [target, second]);
                }
            }
        } else if let Some(hunt) = self
            .hunt
            .as_mut()
            .filter(|h| h.state == crate::hunt::SEARCH)
        {
            // state 3 (case 2): the timer runs by dt; at its end it starts again at −10 s and the
            // unit AI is told again (notify 0x202)
            if hunt.timer < 0.0 {
                hunt.timer += c.dt_seconds;
                if 0.0 <= hunt.timer {
                    hunt.timer = SEARCH_AGAIN;
                    self.hunt_search_notify(c, others)?;
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
            // case 3 waiting after a given-up MoveTo, its target gone: the timer runs by dt,
            // then the walk to the point (set_state 11)
            let mover = self
                .mover
                .as_mut()
                .ok_or(Unsupported("a hunter without movement state"))?;
            hunt.wait_then_walk(c.dt_seconds, mover)?;
        } else if self
            .hunt
            .as_ref()
            .is_some_and(|h| h.state == crate::hunt::LEAD && h.target == -1)
        {
            // state 10 without its target (0x140ac7a00 case 9): set_state(3), the search
            self.hunt.as_mut().expect("above").state = crate::hunt::SEARCH;
            self.hunt_set_search(c, others)?;
        } else if tree_lost && self.hunt_tree_search(c, others, Some(TREE_SEARCH_WITHIN), None)? {
            // the hunt took the next tree (slot 0x48): its MoveTo's code 5 ends the update
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
                tree_searched: tree_lost,
                predator_hunts_end: c.predator_hunts_end,
                searches: false,
                leads: None,
                attack_failed: false,
                let_go: None,
            };
            // what set_state(6) on live livestock reads (0x140ac60e0 case 6), and the lead's
            // update (state 10, 0x140ac7a00 case 9: whatever the target, a dead one too)
            let faces = hunt.state == crate::hunt::WALK && hunt.move_to == Some(MoveTo::Arrived);
            let herd = if (faces && victim.class == crate::hunt::LIVESTOCK && victim.state <= 2)
                || hunt.state == crate::hunt::LEAD
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
                let moving = hunt.task.walk_sprite_id(*gather.held_kind, *gather.held);
                let arms = crate::attack::arms(master, c.graphics, hunt.task.sprites, moving);
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
                // the animal's top action without a target or a point (slots 0x190, 0x288)
                let animal = others
                    .iter()
                    .find(|u| u.id == hunt.target)
                    .or_else(|| c.foreign.find(hunt.target).map(|(_, u)| u));
                let idle = match animal {
                    Some(u) => u.top_action_idle()?,
                    None => false,
                };
                Some(crate::hunt::Herd {
                    own,
                    drop_site,
                    engagement,
                    arms,
                    idle,
                    special_graphic: if master.kind == 70 && master.special_graphic {
                        Some(
                            c.graphics
                                .get(&master.special_sprite)
                                .copied()
                                .ok_or(Unsupported(
                                    "a special graphic the data file does not have",
                                ))?,
                        )
                    } else {
                        None
                    },
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
                ai_type: self.ai.as_ref().map(|ai| ai.ai_type),
                computer,
                armed: true,
                volley_under_way: self.next_volley != 0,
                let_go: None,
                forgets: false,
                sends_home: false,
            };
            let full = hunt.update(walk, computer, &mut strike, &mut gather, herd)?;
            let searches = gather.searches;
            let (attack_failed, let_go) = (gather.attack_failed, gather.let_go);
            let posted = std::mem::take(&mut strike.posted);
            let leads = gather.leads.map(|site| (hunt.target, site));
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
            for (kind, p) in posted {
                self.post_own(kind, p)?;
            }
            c.own_killed = self.hit_back(own)?;
            if full {
                self.hunt_drop_off(c, others)?;
            }
            if searches {
                self.hunt_set_search(c, others)?;
            }
            if let Some(target) = let_go {
                self.hunt_release(c, [target]);
            }
            if attack_failed {
                // FUN_140ac4670 with -1 and 5: the class its unit AI targets (else the hunt's)
                // that the civilian's slot 0x1b0 gives 0 (trees) takes the next tree within 5
                // of the unit; else set_state(3) with the target kept
                let tree = self.ai.as_ref().is_some_and(|ai| {
                    let class = if ai.target_type == -1 {
                        self.hunt.map_or(-1, |h| i32::from(h.target_class))
                    } else {
                        ai.target_type
                    };
                    class == crate::hunt::TREE_CLASS
                });
                if !(tree && self.hunt_tree_search(c, others, None, Some(TREE_NEAR))?) {
                    let hunt = self.hunt.as_mut().expect("above");
                    hunt.tear_down(self.mover.as_mut())?;
                    hunt.state = crate::hunt::SEARCH;
                    self.hunt_search_state(c, others)?;
                }
            }
            c.leads = leads;
        } else if let Some(attack) = self.attack.as_mut().filter(|a| a.target == -1) {
            // the unit's own Attack after its target went ([`Attack::lost`])
            let master = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(Unsupported(
                    "an attacker of a master the slice does not have",
                ))?;
            let armed = armed(master, self.firepower);
            let (code, disarmed) =
                attack.update_targetless(c.dt_seconds, armed, &mut self.anim_speed)?;
            if disarmed {
                let posted = attack.disarmed(
                    &mut self.sprite,
                    &mut self.anim_speed,
                    c.streams.get(Stream::Sprite),
                )?;
                if let Some((kind, p)) = posted {
                    self.post_own(kind, p)?;
                }
            }
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
                ai_type: self.ai.as_ref().map(|ai| ai.ai_type),
                computer,
                armed,
                volley_under_way: self.next_volley != 0,
                let_go: None,
                forgets: false,
                sends_home: false,
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
            let sends_home = strike.sends_home;
            c.blasted = blasted;
            if let Some(t) = let_go {
                c.releases.push((t, self.releaser(c)));
            }
            if forgets && let Some(ai) = self.ai.as_mut() {
                ai.remove_target();
            }
            if sends_home && let Some(home) = self.ai.as_mut().and_then(|ai| ai.home.as_mut()) {
                home.go_home = true;
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
            if !m.in_the_way.is_empty() {
                return Err(Unsupported(
                    "a walk's unit in the way (notify 0x215) the action does not pass on",
                ));
            }
        }
        let resume = AiResume { captured, leaves };
        // a builder's work found in its Build's update (slot 0x7e8 from case 0xdf) runs inside the
        // action update; the caller gives it (`Turns::order_work`) and then runs the rest of this
        // update ([`Unit::ai_part`])
        if c.work_order.is_some() || !c.completions.is_empty() {
            c.ai_resume = Some(resume);
            return Ok(Step::Stay);
        }
        self.ai_part(c, others, resume)
    }

    /// The end of [`Unit::update`] after the actions: `RGE_Action_Object::update`'s end
    /// (`FUN_141163080`: the action object's shape), `RGE_Combat_Object::update`'s sight mask,
    /// then the unit AI's full update on its owner's turn, else its check between turns, and the
    /// effects it has on the unit; then the step the update ends with.
    fn ai_part(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
        resume: AiResume,
    ) -> Result<Step, Unsupported> {
        let AiResume { captured, leaves } = resume;
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
                combat_level: master.map_or(0, |m| m.combat_level),
                attack_priority: master.map_or(0, |m| m.attack_priority),
                size: master.map_or([0.0; 2], |m| m.radius),
                los: master.map_or(0.0, |m| m.line_of_sight),
                search_radius: master.map_or(0.0, |m| m.search_radius),
                weapon_range: master.map_or(0.0, |m| m.weapon_range),
                interruptible,
                task_0x99: master.is_some_and(|m| m.tasks.iter().any(|t| t.action_type == 0x99)),
                garrisoned: self.life.inside.is_some(),
                state: self.life.state,
                kind: master.map_or(0, |m| m.kind),
                ranged: master.is_some_and(|m| 0 <= m.missile),
                firepower: self.firepower,
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
                group: c
                    .groups
                    .get(self.group.id)
                    .map(|g| crate::unit_ai::GroupState {
                        commanded: g.current_command != -1,
                        idle: g.is_idle(),
                        patrol: g.patrols(),
                        moving: g.motion.moving(),
                        member_moving: self.mover.as_ref().is_some_and(Mover::is_moving),
                    }),
            };
            // the unit AI runs on its own; what it asks of its unit comes back through `UnitView`
            let mut ai = self.ai.take().expect("filtered above");
            let effects = {
                {
                    let attacker = crate::unit_ai::Attacker {
                        ai_type: ai.ai_type,
                        player,
                        computer,
                        state: self.life.state,
                        held: self.life.attribute_type,
                        top_action: self.top_action_kind().map(i32::from),
                        master: self.life.master_id,
                        kind: master.map_or(0, |m| m.kind),
                        class: master.map_or(-1, |m| m.class),
                        attack_priority: master.map_or(0, |m| m.attack_priority),
                        missile: master.map_or(-1, |m| m.missile),
                        range: master.map_or(0.0, |m| m.weapon_range),
                        min_range: master.map_or(0.0, |m| m.min_range),
                        charge_type: master.map_or(0, |m| m.charge_type),
                        area_effect: master.map_or(0.0, |m| m.area_effect),
                        blast_level: master.map_or(0, |m| m.blast_level),
                        los: master.map_or(0.0, |m| m.line_of_sight),
                        position: self.position,
                        radius: master.map_or([0.0; 2], |m| m.radius),
                        speed: self.mover.as_ref().map_or(0.0, |m| m.stats.speed),
                        scenario: c.scenario,
                        firepower: self.firepower,
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
                            groups: c.groups,
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
                            releases: &mut c.releases,
                            waits: &mut c.waits,
                            relations: c.relations,
                            attributes: c.attributes,
                            civ: c.civ,
                            drop_sites: c.drop_sites,
                        };
                        match turn {
                            Some(dt) => {
                                ai.update(dt, c.world_time, computer, &at, ai_stream, &mut side)
                            }
                            None => ai.off_turn(&at, &mut side),
                        }
                    };
                    self.ai = Some(ai);
                    result?
                }
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
        if self.artifact.as_mut().is_some_and(Artifact::stop) {
            self.mover
                .as_mut()
                .ok_or(Unsupported("a walking animal without movement state"))?
                .stop()?;
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
            wait: crate::move_to::MoveWait::default(),
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
/// the resources would go to the allies too when the player shares (`sharing`, `+0x1426`,
/// `FUN_14165d040`): a sharing player with allies is not modelled.
fn give_load(
    (relations, sharing): (&[Vec<u8>], bool),
    player: usize,
    adds: &mut Vec<(i16, f32)>,
    kind: i16,
    amount: f32,
) -> Result<(), Unsupported> {
    let own = relations.get(player).map_or(&[][..], |r| &r[..]);
    let allied = (1..own.len()).any(|q| q != player && crate::tasks::allied(own, player, q, true));
    if sharing && allied {
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
    /// The owner shares its gathered resources (`+0x1426`).
    pub sharing: bool,
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
                        if unit.artifact.as_mut().is_some_and(Artifact::stop) {
                            unit.mover
                                .as_mut()
                                .ok_or(Unsupported("a walking animal without movement state"))?
                                .stop()?;
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

/// What a unit's update hands its unit AI part when that runs after the caller gave the work its
/// builder found ([`Unit::ai_part`]): a takeover and the step the update ends with.
#[derive(Clone, Debug, PartialEq, Eq)]
struct AiResume {
    captured: Option<(usize, Vec<i32>)>,
    leaves: Step,
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
    /// The nearest is by path (`FUN_1411671e0`'s argument 9: a human player's); else by the
    /// squared distance of the centres, of the objects the planner reaches.
    by_path: bool,
}

/// A search for drop sites (`FUN_1411671e0` as the Gather's `set_state` 8 and 3 call it: in state
/// 2, not at work on task `0x7d` or `0x7c` (an object of type 40 up whose top action is one), not
/// inside another when `outside`): of the player's live objects (`units`, in list order) of a
/// master in `sites`, the nearest to `at`. With the `seeker`, only objects in its zone count (its
/// restriction's zone of its tile against each object's tile) and only those the tile planner
/// reaches (its goal or its range; radii the seeker's and the object's, [`Mover::plan_length`]),
/// the nearest by path ([`Seeker::by_path`]): the path's length, within 10 tiles the shorter of
/// it and the seeker's short path (slot 0x3e8 within 0.1, [`Mover::short_reach`]); else by the
/// squared distance of the centres. Without a planner the game keeps the distance it last
/// measured (of an earlier object, else the largest float) by path. Without a seeker, by the
/// squared distance of the centres. The seeker's class 21 (its gap test) and a villager tending
/// a pasture (its pasture's place) are not modelled. The first of equals wins.
fn drop_site_search(
    units: &[Unit],
    masters: &Masters,
    sites: &[i16],
    at: [f32; 3],
    mut seeker: Option<Seeker<'_, '_>>,
    outside: bool,
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
        if at_work || (outside && o.life.inside.is_some()) {
            continue;
        }
        let (dx, dy) = (x - o.position[0], y - o.position[1]);
        let squared = dy * dy + dx * dx;
        let d = if let Some(s) = seeker.as_mut() {
            let radii = s.mover.stats.radius[0] + master.radius[0];
            let to = [o.position[0], o.position[1]];
            match s.mover.plan_length(&s.me, s.spatial, s.plan, to, radii)? {
                Some((code, length)) if code < 2 => {
                    if s.by_path {
                        last = length;
                        if length <= SHORT_DROP_REACH
                            && let Some(short) =
                                s.mover.short_reach(&s.me, s.spatial, o.id, DROP_REACH)?
                            && short < length
                        {
                            last = short;
                        }
                        last
                    } else {
                        squared
                    }
                }
                // the planner's other results pass the object over
                Some(_) => continue,
                None if s.by_path => last,
                None => squared,
            }
        } else {
            squared
        };
        if d < best_d {
            best = Some(o.id);
            best_d = d;
        }
    }
    Ok(best)
}

/// What `ClosestResourceObject` reads of the world, as the unit's update or its unit AI sees it.
struct ResourceWorld<'a, 'b> {
    player: usize,
    spatial: &'a Spatial,
    zones: &'a mut crate::zones::Zones,
    foreign: Foreign<'a>,
    every: &'a [Masters],
    masters: Option<&'a Masters>,
    plan: Planning<'b>,
}

/// [`crate::gather_search::closest_resource_at`] for `unit` of gather group `kind` with fewer
/// than `cap` workers (−1 any) from the drop site `site` (−1 for none; one the world does not hold
/// counts as none, `FUN_1415417a0`), its unit AI's current target `current` passed over: the
/// site's place, its master's radii and line of sight, and whether the search starts from its
/// footprint (the site's tile in the unit's zone of its restriction, or a dock: masters 45 and
/// 2172, `0x140c49d17`); the answer and the out argument's steps.
fn closest_resource_of(
    unit: &Unit,
    c: &mut ResourceWorld<'_, '_>,
    others: &[Unit],
    (kind, cap, site): (u8, i32, i32),
    (current, min_state): (i32, u8),
) -> Result<(i32, Option<i32>), Unsupported> {
    use crate::gather_search::{self, Candidate};
    /// The site masters whose search starts from their footprint in any zone.
    const DOCKS: [i16; 2] = [45, 2172];
    let site = if site == -1 || c.spatial.get(site).is_none() {
        None
    } else {
        // the unit itself (a mule cart searches from its own place) or another object
        let (sp, s) = std::iter::once(unit)
            .chain(others.iter())
            .find(|u| u.id == site)
            .map(|u| (c.player, u))
            .or_else(|| c.foreign.find(site))
            .ok_or(Unsupported(
                "a resource search's drop site the slice does not hold",
            ))?;
        let m = c
            .every
            .get(sp)
            .and_then(|m| m.get(s.life.master_id))
            .ok_or(Unsupported(
                "a resource search's drop site of a master the slice lacks",
            ))?;
        let footprint = DOCKS.contains(&m.id) || {
            let restriction = unit
                .mover
                .as_ref()
                .ok_or(Unsupported("a resource search without movement state"))?
                .stats
                .restriction;
            let [w, h] = c.spatial.map;
            let terrain = c
                .spatial
                .terrain
                .as_ref()
                .ok_or(Unsupported("the map's zones without its terrain"))?;
            let values = terrain
                .restrictions
                .get(restriction)
                .ok_or(Unsupported("a restriction the slice's data has not got"))?;
            let zones = c.zones.of(w, h, &terrain.kinds, values);
            let zone = |p: [f32; 3]| zones.zone(p[0] as i32, p[1] as i32);
            zone(s.position) == zone(unit.position)
        };
        Some(gather_search::SearchSite {
            position: s.position,
            radius: m.radius,
            line_of_sight: m.line_of_sight,
            footprint,
        })
    };
    let masters = c.masters.ok_or(Unsupported(
        "a resource search without its player's masters",
    ))?;
    let master = masters.get(unit.life.master_id).ok_or(Unsupported(
        "a searcher of a master the slice does not have",
    ))?;
    let mover = unit
        .mover
        .as_ref()
        .ok_or(Unsupported("a resource search without movement state"))?;
    let mask = mover.stats.restriction_mask;
    let (player, me_id, foreign, every) = (c.player, unit.id, c.foreign, c.every);
    let candidate = |id: i32, owner: usize, u: &Unit, position: [f32; 3]| {
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
    let me = candidate(me_id, player, unit, unit.position)?;
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
    let seeker = gather_search::Seeker {
        player,
        position: unit.position,
        line_of_sight: master.line_of_sight,
        state: unit.life.state,
        current_target: current,
        my_pasture: unit.my_pasture,
        min_state,
    };
    let me_walk = Me {
        id: unit.id,
        position: unit.position,
        group: unit.group.id,
        action_state: unit.top_action().map_or(0, |a| a.state),
    };
    let master_id = unit.life.master_id;
    let plan = &mut c.plan;
    let blocked = u64::from(mask ^ 0xff) | crate::spatial::BLOCKED;
    gather_search::closest_resource_at(
        &seeker,
        site.as_ref(),
        kind,
        cap,
        spatial.map,
        &|x, y| spatial.explored(player, x, y),
        &|x, y| spatial.tiles().word(x, y).is_some_and(|w| w & blocked == 0),
        &objects,
        &mut |id| mover.reaches_object(&me_walk, spatial, plan, id, 0.0, master_id),
    )
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
    /// `mMinColumnDistance` (`+0x6b8`): a group's move along a longer path forms a column
    /// (`commandMoveTo`).
    pub min_column_distance: i32,
    /// `mColumnToLineDistance` (`+0x6bc`): where a column on its last waypoint turns to a line
    /// (`UpdateDesiredPositions`).
    pub column_to_line_distance: i32,
}

/// One player's objects in live-list order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Player {
    /// Its live objects, in their list's order.
    pub objects: Vec<Unit>,
    /// `player+0x10 == 3`.
    pub computer: bool,
    /// `player+0x10`, the player's type (the save's class byte): 1 human, 2 Gaia, 3 computer
    /// (0 the scenario editor's phantom).
    pub kind: u8,
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
    /// The players whose town centres the player's reveal (attribute 209) flared
    /// (`mTCFlared[i]`, saved; [`crate::vision`]).
    pub tc_flared: Vec<bool>,
    /// `mSharingIsCaring` (`+0x1426`, `FUN_141626780`, saved): the player's gathered resources go
    /// to its allies too.
    pub sharing: bool,
    /// The objects whose ghosts the player's list follows (`+0x78`, [`crate::ghost`]): not
    /// saved; the load builds it ([`Turns::build_watched`]).
    pub watched: Vec<crate::ghost::Watched>,
    /// The player's ghosts (its doppelganger list `+0x40`), in the list's order.
    pub ghosts: Vec<crate::ghost::Ghost>,
    /// Its history (`+0xdb8`, saved, [`crate::history`]).
    pub history: crate::history::History,
    /// Its object tracker (`+0x18`, [`crate::tracker`]): not saved; the load files the objects
    /// ([`Turns::load_trackers`]).
    pub tracker: crate::tracker::Tracker,
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
                    flags: master.object_flags,
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
        // a hunt that stopped its decay gave it a master of its own (FUN_140ac44b0)
        if let Some(m) = u.life.master.as_mut() {
            m.decay = v.decay;
        }
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
    /// villager first counts for its owner (`FUN_141082e80`): an attacker of class 9 or 10 (an
    /// animal) in attribute 227, and by the attacker's player type a human's in 229, Gaia's in
    /// 226, a computer's in 228. Gaia takes a player's livestock (slot 0x70): its place and sight, its Artifact's
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
        let ap = kill.attacker_player;
        if master.class == 4 {
            let (bp, by) = self.object(kill.attacker).ok_or(Unsupported(
                "a killed villager's attacker the slice does not hold",
            ))?;
            if matches!(self.master_of(bp, by.life.master_id)?.class, 9 | 10) {
                self.players[q].add_attribute(227, 1.0)?;
            }
            let by_kind = match self.players[ap].kind {
                1 => Some(229),
                2 => Some(226),
                3 => Some(228),
                _ => None,
            };
            if let Some(kind) = by_kind {
                self.players[q].add_attribute(kind, 1.0)?;
            }
        }
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
    /// update ([`Unit::change_owner`]'s parts, the class counters ([`count_class`]) and the list
    /// and storage moves of a capture).
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
        let own = self.masters.get(q).and_then(|m| m.get(master_id));
        let storage = own.map(|m| m.storage.clone()).unwrap_or_default();
        // the class counters leave the old owner and come to Gaia (FUN_141226060 with -1, then 1)
        if let Some(m) = own {
            count_class(&mut self.players[q].attributes, m, -1.0);
            count_class(&mut self.players[0].attributes, m, 1.0);
        }
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
        // the old owner's remove and Gaia's add (slots 0x2d8, 0x2d0) move it between the
        // trackers; a building whose build points reach its build time completes in Gaia's
        // (`FUN_1411cd860`, `COMISS`/`JC`: a NaN does not)
        let built = self
            .masters
            .first()
            .and_then(|m| m.get(master_id))
            .is_some_and(|m| m.kind == BUILDING_KIND)
            && unit
                .construction
                .is_some_and(|c| f32::from(c.build_time) <= c.progress);
        self.players[0].objects.push(unit);
        self.track_remove(q, id, master_id);
        self.track_add(0, id, master_id);
        if built {
            self.track_complete(0, id, master_id);
        }
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
    /// The world's unit groups, when the slice's differ.
    pub groups: Option<crate::group::UnitGroups>,
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
        if let Some(groups) = game.groups.as_ref() {
            self.groups.clone_from(groups);
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
                    self.track_remove(p, gone.id, gone.life.master_id);
                    let master = self.masters.get(p).and_then(|m| m.get(gone.life.master_id));
                    forget_resource(&mut self.spatial, master, gone.id, gone.position);
                    if let Some(m) = master.filter(|m| 70 <= m.kind) {
                        count_class(&mut self.players[p].attributes, m, -1.0);
                    }
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
                    if before < 4 && 6 < state {
                        let master = self
                            .masters
                            .get(p)
                            .and_then(|m| m.get(forced.life.master_id));
                        forget_resource(&mut self.spatial, master, id, forced.position);
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

impl Turns {
    /// A command or a pass the slice does not model for player `p`: the player stops
    /// ([`Player::stop`]), and a subscriber hears why ([`crate::trace`]).
    pub(crate) fn stop_player(&mut self, p: usize, why: Unsupported) {
        crate::trace::player_stops(self.ticks, p, why);
        self.players[p].stop(self.ticks, why);
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
    /// The game's unit lines (`unitlines.json`, [`crate::lines`]); empty when not loaded.
    pub lines: crate::lines::UnitLines,
    /// The game's linked technologies (`linkedTechs.json`, the table at the game settings'
    /// `+0x18` that `FUN_140f39f40` looks up), by technology; empty when not loaded.
    pub linked_techs: std::collections::BTreeMap<i16, crate::tech::Linked>,
    /// The game settings' test `FUN_141302150` (their `+0x1070` flag, then a table entry's
    /// `+0x9c`): a hunter drops another resource it carries. The tick oracle read the flag as 0 in
    /// MarvinBlue/426048731; where the save or the lobby holds it is not known yet.
    pub carry_rule: bool,
    /// The game's setting at `FUN_140c87b90()`'s `+0x69`: farmers walk their farm's stops (a
    /// farm's Gather of four stops), and a Gather's `set_state(7)` leaves `mGatherX`, `mGatherY`
    /// at −1. The tick oracle shows it set in records/425906646 (the forager 31139's `+0x8c`
    /// stays −1); where the save or the lobby holds it is not known yet.
    pub farm_stops: bool,
    /// The game's setting that `FUN_140ec0cd0` reads at `FUN_140c87b90()`'s `+0x98` is 1: a hunt
    /// whose Attack on a predator (class 10) ended ends too (`set_state` 1) when its unit AI has
    /// orders queued. records/425906646 shows it unset (its 31118 walks to the wolf in update
    /// 4461); where the save or the lobby holds it is not known yet.
    pub predator_hunts_end: bool,
    /// The game's setting that `FUN_140ec0cd0` reads at its `+0x680` (a value and a kind); a boar
    /// does not wander when it is 1. The tick oracle reads 0 in MarvinBlue/426048731; where the
    /// save or the lobby holds it is not known yet (the save's `mGameMode` sits elsewhere).
    pub mode_680: i32,
    /// The game mode (the save's `mGameMode`, options `+0x34`).
    pub game_mode: i32,
    /// The Empire Wars start's pass is due ([`crate::start_work`]): the world's `+0x6c6` is
    /// clear in game mode 13, which a game's start has.
    pub start_work_due: bool,
    /// The game setting `+0x1074` (`FUN_1412ffe00`): players may order each other's units. The
    /// tick oracle reads 0 in MarvinBlue/426049433; where the save or the lobby holds it is not
    /// known yet.
    pub allied_control: bool,
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
    /// The world's unit groups ([`crate::group`]).
    pub groups: crate::group::UnitGroups,
    /// The current update's time in seconds (`world+0x164`, set before the commands run).
    pub dt_seconds: f32,
    /// The history period (`world+0x8b8`, `time_slice`): the `.dat`'s, which the world reads
    /// after its constructor's [`HISTORY_PERIOD`] ([`crate::history::History::update`]).
    pub history_period: i32,
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

/// The players' relations and the drop sites as the task finder reads them while a unit's
/// owner's list is out for its update ([`UnitView`]): the owner's relations, the others'.
struct SideWorld<'a> {
    player: usize,
    relations: &'a [u8],
    foreign: Foreign<'a>,
    drop_sites: &'a [DropSite],
}

impl Diplomacy for SideWorld<'_> {
    /// [`crate::tasks::allied`] with `from`'s diplomacy.
    fn allied(&self, from: usize, to: usize, with_ally: bool) -> bool {
        let relations = if from == self.player {
            self.relations
        } else {
            self.foreign
                .player(from)
                .map_or(&[][..], |p| &p.relations[..])
        };
        crate::tasks::allied(relations, from, to, with_ally)
    }

    fn drops_at(&self, building: i16, actor: &Actor<'_>) -> bool {
        crate::tasks::drops_at(self.drop_sites, building, actor)
    }
}

/// The build points (`+0xc50`) a wait for a foundation reads of object `id`
/// ([`crate::wait_solidify::WaitSolidify::update`]) among its owner's other objects and the
/// other players' lists: None when the world does not hold it. Its target is the building that
/// sent the unit off (the unit AI's notify 0x216), whose build points are its construction's;
/// the world never gives its id to another object, so an object without a construction is not
/// met.
fn wait_progress(id: i32, others: &[Unit], foreign: Foreign<'_>) -> Option<f32> {
    others
        .iter()
        .find(|u| u.id == id)
        .or_else(|| foreign.find(id).map(|(_, u)| u))
        .and_then(|u| u.construction.map(|k| k.progress))
}

/// What the task finder reads of object `id`, as `find` holds the objects (with their owners)
/// and `every` the players' masters; an annex stands for its main building (`+0xbb8`), `steps`
/// times (`Turns::task_target`'s reading). None when `find` does not hold it.
fn task_target_with<'a>(
    id: i32,
    steps: usize,
    find: &impl Fn(i32) -> Option<(usize, &'a Unit)>,
    every: &'a [Masters],
) -> Result<Option<TaskTarget<'a>>, Unsupported> {
    let mut id = id;
    for _ in 0..steps {
        match find(id).and_then(|(_, u)| u.annex_of) {
            Some(main) => id = main,
            None => break,
        }
    }
    let Some((player, u)) = find(id) else {
        return Ok(None);
    };
    let master = every
        .get(player)
        .and_then(|m| m.get(u.life.master_id))
        .ok_or(Unsupported("an object of a master the slice does not have"))?;
    let [powerup, max_powerup] = u.hp_powerups;
    Ok(Some(TaskTarget {
        id,
        master,
        player,
        state: u.life.state,
        held: (u.life.attribute_type, u.life.amount),
        hp: u.life.hp + (max_powerup - powerup),
        max_hp: f32::from(master.hit_points) + max_powerup,
        attackable: u.attackable,
        pasture: u.pasture,
        top_action: u.top_action_kind(),
    }))
}

/// The search of a unit's slot 0x820 (`FUN_14106c6a0`) on target `t` (`general` the target the
/// general search reads, its main building's) with `filter`: the list of its own master
/// (`actor.master`) gives the task ([`crate::tasks::find_task`] in mode 0); without one, a
/// master with a task swap group (`+0x7b8`) tries its player's masters (`masters`) in id order
/// of its class, type and group, its own too, with any filter, and the first with a task gives
/// it. The task, with the master that gave it when the swap group did: with its `param_7` the
/// routine switches the unit to that master (slot 0x90 with 0), its own too, which is the
/// caller's. None: no task. The terrain under the target's place, which only a task for a
/// terrain reads, is not passed (as `Turns::find_work_task`, the same routine's port in
/// `crate::order`, which this module cannot call).
fn task_search<'m>(
    masters: &'m Masters,
    actor: &Actor<'_>,
    t: &TaskTarget<'_>,
    general: &TaskTarget<'_>,
    filter: i16,
    dip: &impl Diplomacy,
) -> Result<Option<(crate::masters::Task, Option<&'m Master>)>, Unsupported> {
    let own = actor.master;
    let find = |list: &Master, filter: i16| {
        crate::tasks::find_task(list, actor, Some(t), Some(general), None, filter, dip)
            .map(Option::<&crate::masters::Task>::copied)
    };
    if let Some(task) = find(own, filter)? {
        return Ok(Some((task, None)));
    }
    if own.task_by_group == 0 {
        return Ok(None);
    }
    for candidate in masters.iter() {
        if candidate.class != own.class
            || candidate.kind != own.kind
            || candidate.task_by_group != own.task_by_group
        {
            continue;
        }
        if let Some(task) = find(candidate, -1)? {
            return Ok(Some((task, Some(candidate))));
        }
    }
    Ok(None)
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
            lines: crate::lines::UnitLines::default(),
            linked_techs: std::collections::BTreeMap::new(),
            carry_rule: false,
            farm_stops: false,
            predator_hunts_end: false,
            start_work_due: false,
            allied_control: false,
            mode_680: 0,
            game_mode: 0,
            game_state: 0,
            victory_conquest: false,
            unit_cost_246: false,
            zones: crate::zones::Zones::default(),
            recycling: Recycling::default(),
            streams: Streams::default(),
            techs: Vec::new(),
            groups: crate::group::UnitGroups::default(),
            dt_seconds: 0.0,
            history_period: HISTORY_PERIOD,
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
                let master = self.farm_on_rice_terrain(master, position);
                // the pending debit first (FUN_141630d10 with 0), then the builders' test, the
                // placement test and the payment's: a refused BUILD places nothing
                if let Err(e) = self.debit_listed(player, master) {
                    self.stop_player(player, e);
                    return;
                }
                match self.build_has_builder(player, master, builders) {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(e) => {
                        self.stop_player(player, e);
                        return;
                    }
                }
                match self.build_allowed(player, master, [position[0], position[1]]) {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(e) => {
                        self.stop_player(player, e);
                        return;
                    }
                }
                let built =
                    self.found(player, master, unit_ai, position)
                        .and_then(|(id, position)| {
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
                    self.stop_player(player, e);
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
                    self.stop_player(player, e);
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
                    self.stop_player(player, e);
                }
            }
            Command::Unmodelled { player, why } => {
                self.stop_player(player, why);
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
                    self.stop_player(player, e);
                }
            }
            Command::CommandQueue {
                player,
                ref ids,
                target,
                point,
            } => {
                if let Err(e) = self.command_queue(player, ids, target, point) {
                    self.stop_player(player, e);
                }
            }
            Command::BuildWall(ref w) => {
                if let Err(e) = self.build_wall(w) {
                    self.stop_player(w.player, e);
                }
            }
            Command::Delete { player, id } => {
                if let Err(e) = self.delete(player, id) {
                    self.stop_player(player, e);
                }
            }
            Command::GoBackToWork { player, ref ids } => {
                if let Err(e) = self.go_back_to_work(player, ids) {
                    self.stop_player(player, e);
                }
            }
            Command::TownBell {
                player,
                id,
                activate,
            } => {
                if let Err(e) = self.town_bell(player, id, activate) {
                    self.stop_player(player, e);
                }
            }
            Command::Flare {
                player,
                target,
                point,
                players,
            } => {
                if let Err(e) = self.flare(target, point, players) {
                    self.stop_player(player, e);
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
                    self.stop_player(player, e);
                }
            }
            Command::UnitOrder {
                player,
                ref ids,
                target,
                through_ai,
                nearby,
                action,
                param,
                human,
            } => {
                if human {
                    self.human_order(ids);
                }
                let result = match action {
                    5 => self.garrison_order(player, ids, target, through_ai, nearby),
                    4 => self.cancel_order(ids, param, through_ai),
                    _ => self.unit_order(player, ids, action),
                };
                if let Err(e) = result {
                    self.stop_player(player, e);
                }
            }
            Command::TransformObject { player, id, master } => {
                if let Err(e) = self.transform_object(player, id, master) {
                    self.stop_player(player, e);
                }
            }
            Command::Resign { player, dropped } => {
                if let Err(e) = self.resign(player, dropped) {
                    self.stop_player(player, e);
                }
            }
            Command::AutoScout { player, ref ids } => {
                if let Err(e) = self.auto_scout(player, ids) {
                    self.stop_player(player, e);
                }
            }
            Command::Stop { player, ref ids } => {
                if let Err(e) = self.stop_command(player, ids) {
                    self.stop_player(player, e);
                }
            }
            Command::SeekShelter { player, ref ids } => {
                if let Err(e) = self.seek_shelter(player, ids) {
                    self.stop_player(player, e);
                }
            }
            Command::UnitAiState {
                player,
                ref ids,
                state,
            } => {
                if let Err(e) = self.unit_ai_state(player, ids, state) {
                    self.stop_player(player, e);
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
                    self.stop_player(player, e);
                }
            }
            Command::Order {
                player,
                ref units,
                order,
            } => {
                // the command's check (MOVE's FUN_140e553f0, INTERACT's FUN_140e55250): a unit the
                // world does not hold, or of another player without the setting +0x1074, becomes
                // -1; a human player's command left without a unit does nothing
                let checked: Vec<i32>;
                let units: &[i32] = if matches!(order, Some(Order::Move(_) | Order::Interact(_))) {
                    checked = self.checked_units(player, units);
                    if checked.iter().all(|&id| id == -1) && !self.players[player].computer {
                        return;
                    }
                    &checked
                } else {
                    units
                };
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
                self.reset_order_units(units);
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
                    self.stop_player(player, e);
                }
            }
        }
    }

    /// A new object of player `p`'s `master` placed at `at` (the static `new_position`,
    /// `0x1411a58b0`, slot 0x60's `FUN_141193a10`): its height the ground's by the master's fly
    /// mode; without the map's elevations the height stays.
    pub(crate) fn on_ground(&self, p: usize, master: i16, at: [f32; 3]) -> [f32; 3] {
        let fly_mode = self
            .masters
            .get(p)
            .and_then(|m| m.get(master))
            .map_or(0, |m| m.fly_mode);
        self.spatial
            .terrain
            .as_ref()
            .and_then(|t| t.ground(at[0], at[1]).ok())
            .map_or(at, |g| {
                [
                    at[0],
                    at[1],
                    crate::missile::grounded(at[2], g, false, fly_mode),
                ]
            })
    }

    /// The command check's units (`FUN_140e553f0`, `FUN_140e55250`): each unit the world holds
    /// that is the player's own, or any while players may order each other's units
    /// ([`Turns::allied_control`]); −1 for the others.
    pub(crate) fn checked_units(&self, player: usize, units: &[i32]) -> Vec<i32> {
        units
            .iter()
            .map(|&id| match self.object(id) {
                Some((p, _)) if p == player || self.allied_control => id,
                _ => -1,
            })
            .collect()
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

    /// The new foundation of BUILD and of `BUILD_WALL`'s tiles (the master's slot 0x108,
    /// [`Turns::make_building`]), then the player pays the master's cost (each entry flagged
    /// paid). Its id and position.
    pub(crate) fn found(
        &mut self,
        player: usize,
        master: i16,
        unit_ai: Result<Option<u32>, Unsupported>,
        position: [f32; 3],
    ) -> Result<(i32, [f32; 3]), Unsupported> {
        let made = self.make_building(player, master, unit_ai, position)?;
        let cost = self.master_of(player, master)?.cost.clone();
        let owner = &mut self.players[player];
        for &(resource, amount, paid) in &cost {
            let slot = usize::try_from(resource)
                .ok()
                .and_then(|r| owner.resources.get_mut(r))
                .ok_or(Unsupported("a cost in a resource other than the four"))?;
            if paid {
                *slot -= f32::from(amount);
            }
        }
        Ok(made)
    }

    /// A new building (the master's slot 0x40 or 0x108, `FUN_1411ac5c0` / `FUN_1411aca00` with
    /// their init: the static, animated, moving and combat setups, then the constructor
    /// `FUN_1411de5f0`) of `master` for `player` at `position`, the ground's height: its id
    /// (`mNextObjectId`), its unit AI of the given type if its class gets one, the combat setup's
    /// class counters ([`count_class`]), its powerup offset (the Object stream's "setup" draw)
    /// and a building with a train location its paid cost (attribute 247); the static init's
    /// storage; its place and sight (its place links its tiles to its anchor, `FUN_1411e6780`).
    /// The constructor: masters 50 and 1889 on snow keep their snow (`+0xc61` 2); a master
    /// neither of classes 51, 54 nor 357, 1188, nor an annex (`+0x9b8` −1) takes its build time
    /// as a foundation (a build time above 0: hit points 1, a farm's terrain,
    /// [`Turns::construction_terrain`]; build points 0), the others are built already (build
    /// points their train time, their master's hit points); one with build points 0 and a
    /// build time above 0, not of classes 51 or 54, turns desolid (`FUN_1411cf770`: no record,
    /// its tiles unlinked); its footprint is cleared ([`Turns::clear_footprint`]); each annex
    /// slot (`+0x990`) of a master the player has makes its annex the same way at the slot's
    /// offset (`+0x998`, `+0x9a8`), depth first, linked to the building (`+0xbb8`), out of its
    /// owner's watched list unless the building is a gate (39), desolid when its master is tall,
    /// its own footprint cleared; `+0xcd0` notes mangrove or rice terrain under it
    /// (`FUN_141188db0`); last its owner watches it (`FUN_1411cc980`, [`Turns::watch_started`]).
    /// Not held: the construction sprite, the facets, the world's `+0x154`, slot 0x840's
    /// `+0xba0` and `+0xbf8`, and the wonder's `FUN_1411665c0`.
    ///
    /// # Errors
    ///
    /// A master the player has not got or without its building part, an annex of another type,
    /// and what the clearing and the attributes do not model.
    pub(crate) fn make_building(
        &mut self,
        player: usize,
        master: i16,
        unit_ai: Result<Option<u32>, Unsupported>,
        position: [f32; 3],
    ) -> Result<(i32, [f32; 3]), Unsupported> {
        let made = self.master_of(player, master)?.clone();
        let building = made.building.ok_or(Unsupported(
            "a building constructor of a master without its building part",
        ))?;
        let given_z = position[2];
        // the new object's new_position: the ground's height (slot 0x60)
        let position = self.on_ground(player, master, position);
        let id = self.next_object_id;
        self.next_object_id += 1;
        // what its first update as a built building starts (slot 0x7f0)
        let (default_make, default_task) = self
            .masters
            .get(player)
            .and_then(|m| m.get(master))
            .map_or((false, None), Master::default_start);
        let ai = unit_ai.and_then(|t| {
            t.map(|t| UnitAi::constructed(t, position, self.streams.get(Stream::Ai)))
                .transpose()
        });
        let ai = ai.unwrap_or_else(|e| {
            // the constructor's draws are unknown, so the turn player's are too
            crate::trace::player_unsupported(self.ticks, self.player_turn, e);
            let turn = &mut self.players[self.player_turn];
            let at = turn.objects.len();
            let _ = turn.unsupported.get_or_insert((self.ticks, at, e));
            turn.stopped = true;
            None
        });
        // the combat setup (FUN_14121df60): its class counters (FUN_141226060), its powerup
        // offset (`+0xa54`, the "setup" draw) and, for a building with a train location (an
        // entry of `+0x898` other than -1), its paid cost in attribute 247 (slot 0xf8)
        if let Some(m) = self.masters.get(player).and_then(|m| m.get(master)) {
            count_class(&mut self.players[player].attributes, m, 1.0);
        }
        let _ = self.streams.get(Stream::Object).rand15();
        if let Some(m) = self
            .masters
            .get(player)
            .and_then(|m| m.get(master))
            .filter(|m| m.kind == BUILDING_KIND && m.trained_at.iter().any(|&t| t != -1))
        {
            let paid: i32 = m
                .cost
                .iter()
                .filter(|&&(resource, _, paid)| 0 <= resource && paid)
                .map(|&(_, amount, _)| i32::from(amount))
                .sum();
            self.players[player].add_attribute(BUILDING_COST, paid as f32)?;
        }
        // the static init (FUN_1411a45a0): the last storage entry of flag 0 is held
        // (FUN_141193710), those of flags 1, 2 and 32 go to the owner (FUN_141193860)
        let storage = self
            .masters
            .get(player)
            .and_then(|m| m.get(master))
            .map(|m| m.storage.clone())
            .unwrap_or_default();
        let mut held = (-1, 0.0);
        for &(kind, amount, flag) in &storage {
            if flag == 0 && kind != -1 {
                held = (kind, amount);
            }
        }
        // what its death reads of the master: its dying sprite, death spawns and decay
        let death = self.masters.get(player).and_then(|m| m.get(master)).map_or(
            (None, false, -1, None),
            |m| {
                (
                    (0 <= m.death_sprite).then_some(m.death_sprite),
                    0 <= m.death_spawns[0],
                    m.death_spawns[1],
                    m.life,
                )
            },
        );
        let owner = &mut self.players[player];
        for &(kind, amount, flag) in &storage {
            if 0 <= kind && matches!(flag, 1 | 2 | 32) {
                owner.add_attribute(kind, amount)?;
            }
        }
        // a building that moves (a mule cart: its master's speed above 0) gets the moving
        // object's init (FUN_1411580a0: facing x, its right -z, not waiting); the other
        // buildings' movement state is never read
        let moving = self
            .masters
            .get(player)
            .and_then(|m| m.get(master))
            .and_then(|m| m.moving)
            .filter(|m| 0.0 < m.speed);
        let world_time = self.world_time;
        let mover = moving.map(|stats| Mover {
            forward: [1.0, 0.0, 0.0],
            right: [0.0, 0.0, crate::spawn::START_RIGHT_Z],
            velocity: None,
            paths: Vec::new(),
            factor: 1.0,
            stats,
            waiting: 0,
            wait_delays: 0,
            attack_moving: false,
            last_move: world_time,
            moved_at: position,
            substitute_count: 0,
            ignore_units: Vec::new(),
            last_turn: 0.0,
            angle: 0.0,
            move_to_point: None,
            speed_stopped: false,
            pauses: Vec::new(),
            in_the_way: Vec::new(),
            sprite_facets: None,
        });
        // the constructor (FUN_1411de5f0)
        let [x, y, _] = position;
        let (tx, ty) = (
            crate::checkpoint::cvttss2si(x),
            crate::checkpoint::cvttss2si(y),
        );
        let terrain = self.spatial.terrain.as_ref();
        // masters 50 and 1889 on snow keep their snow (+0xc61 2)
        let snow = matches!(master, FARM | PASTURE_1889)
            && -1 < tx
            && -1 < ty
            && terrain
                .map(|t| t.snow_under(tx, ty))
                .transpose()?
                .unwrap_or(false);
        // +0xcd0 (FUN_141188db0): mangrove or rice terrain under its place
        let rice = terrain.is_some_and(|t| matches!(t.kind(tx, ty), Some(54 | 55 | 63..=67)));
        let foundation = !matches!(made.class, 51 | 54)
            && !matches!(master, 357 | 1188)
            && building.head_unit == -1;
        let hp = if foundation && 0 < made.build_time {
            1.0
        } else {
            f32::from(made.hit_points)
        };
        let progress = if foundation {
            0.0
        } else {
            f32::from(made.build_time)
        };
        // FUN_1411cf770 with no master copy of its own and no annex link yet
        let desolid = 0 < made.build_time && !matches!(made.class, 51 | 54) && progress == 0.0;
        let owner = &mut self.players[player];
        owner.objects.push(Unit {
            id,
            footprint_waits: 0,
            position,
            life: Lifecycle {
                class: Class::Animated,
                state: 0,
                hp,
                master_id: master,
                attribute_type: held.0,
                amount: held.1,
                dying_sprite: death.0,
                death_spawn: death.1,
                remains: death.2,
                master: death.3,
                ..Lifecycle::default()
            },
            ai,
            artifact: None,
            unmodelled_action: None,
            production: None,
            gather_point: None,
            build: None,
            mover,
            move_to: None,
            fly: None,
            construction: Some(Construction {
                progress,
                desolid,
                builders: 0,
                hit_points: made.hit_points,
                build_time: made.build_time,
                record: crate::spatial::Obstruction {
                    kind: made.obstruction_type,
                    class: made.obstruction_class,
                },
                snow,
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
            undeletable: false,
            capture_flag: 0,
            annex_of: None,
            group: crate::group::GroupLink::default(),
            hp_powerups: [0.0; 2],
            hunt: None,
            gather: None,
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
            garrison_time: 0,
            relics: [0; 2],
            raider_countdown: 0,
            // every building holds a gate's lock, open
            gate: Some(Gate {
                first_update: true,
                ..Gate::default()
            }),
            workers: 0,
            combat: true,
            default_task,
            default_fly: None,
            default_make,
            make: None,
            wait_solidify: None,
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
            idle_variant: 0,
            roo_called: false,
            rice,
            store: Vec::new(),
        });
        // the static init's add to its player (slot 0x2d0): it waits in the tracker's pending
        // lists
        self.track_add(player, id, master);
        self.building_place(player, id, &made, position, desolid);
        if foundation && 0 < made.build_time {
            self.construction_terrain(player, master, position)?;
        }
        self.clear_footprint(player, master, [position[0], position[1]], false)?;
        for (slot, &annex) in building.annexes.iter().enumerate() {
            if annex == -1 {
                continue;
            }
            // a master the player has not got makes nothing (its slot 0x40 is not called)
            let Some(am) = self.masters.get(player).and_then(|m| m.get(annex)).cloned() else {
                continue;
            };
            // the .dat's annexes are buildings (type 80): another type's master has no building
            // part, which the constructor below refuses
            let [ox, oy] = building.annex_offsets[slot];
            let ai = crate::unit_ai::factory_type(am.class, annex, am.volley_missile.unwrap_or(-1));
            let at = [position[0] + ox, position[1] + oy, given_z];
            let (aid, at) = self.make_building(player, annex, ai, at)?;
            self.link_annex(player, id, aid, &made, &am)?;
            self.clear_footprint(player, annex, [at[0], at[1]], false)?;
        }
        self.watch_started(player, id)?;
        Ok((id, position))
    }

    /// The building constructor's link of a new annex `aid` (master `am`) to its building `id`
    /// (master `made`): the building's slot and the annex's `+0xbb8`; its owner's watched list
    /// lets the annex go (`FUN_141091310`) unless the building is a gate (39); a tall annex
    /// master makes it desolid (`FUN_1411cf770`, which an annex's link lets through: its record
    /// and mesh shape go, its own master copy has no height, the watched list lets it go; its
    /// tiles keep their links, as an annex's do).
    fn link_annex(
        &mut self,
        player: usize,
        id: i32,
        aid: i32,
        made: &Master,
        am: &Master,
    ) -> Result<(), Unsupported> {
        let not_held = crate::build::NOT_HELD;
        let (_, a) = self.object_mut(aid).ok_or(not_held)?;
        a.annex_of = Some(id);
        self.spatial.set_annex(aid, Some(id));
        if made.class != GATE {
            self.unwatch(player, aid);
        }
        if 0.0 < am.height {
            let (_, a) = self.object_mut(aid).ok_or(not_held)?;
            let c = a
                .construction
                .as_mut()
                .ok_or(crate::build::NO_CONSTRUCTION)?;
            if !c.desolid {
                c.desolid = true;
                self.pathing.removed(aid);
                self.spatial.drop_record(aid);
                self.spatial.set_tall(aid, false);
                self.pathing.tiles_changed(self.spatial.take_changes());
                self.unwatch(player, aid);
            }
        }
        Ok(())
    }

    /// BUILD's first step (`FUN_140e7a400`): a farm (master 50) whose point's tile (truncated) is
    /// mangrove shallows or forest (terrains 54, 55, `FUN_1413fc100`) or a rice farm's (63 to 67,
    /// `FUN_1413fc220`) is a rice farm (1187); a point off the map keeps the farm.
    fn farm_on_rice_terrain(&self, master: i16, position: [f32; 3]) -> i16 {
        if master != FARM {
            return master;
        }
        let (x, y) = (position[0] as i32, position[1] as i32);
        let terrain = self.spatial.terrain.as_ref().and_then(|t| t.kind(x, y));
        match terrain {
            Some(54 | 55 | 63..=67) => RICE_FARM,
            _ => master,
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

    /// BUILD's builders' test (`FUN_140e7a400` after the pending debit): a human player's BUILD
    /// needs one unit of the packet that can build the master ([`Turns::builds`]; one the world
    /// holds, `FUN_140e6bad0`, follows from it), else it places nothing; a computer's passes.
    fn build_has_builder(
        &self,
        player: usize,
        master: i16,
        builders: &[i32],
    ) -> Result<bool, Unsupported> {
        if self.players[player].computer {
            return Ok(true);
        }
        let trained_at = self.master_of(player, master)?.trained_at.clone();
        for &id in builders {
            if self.builds((master, &trained_at), id)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether BUILD (`FUN_140e7a400`) places a foundation of `master` at `at` for `player`: its
    /// placement test ([`crate::spawn::build_placement`], the master's slot 0x50), then the
    /// payment's (`FUN_14162fb60` -> `FUN_141618db0`: every cost entry of a resource, paid or
    /// not, at most what the player holds). A refused BUILD places, pays and draws nothing (the
    /// player's slot 0x380 shows why). A cost in population (attribute 4) is not modelled.
    fn build_allowed(&self, player: usize, master: i16, at: [f32; 2]) -> Result<bool, Unsupported> {
        let human = !self.players[player].computer;
        if self.placement_code(player, master, at, human)? != 0 {
            return Ok(false);
        }
        self.affords(player, master)
    }

    /// The master's placement test (slot 0x50, [`crate::spawn::build_placement`]) of a
    /// foundation of `master` for `player` at `at`, `human` its explored test (`p6`): 0 when it
    /// may go there, else the game's code.
    pub(crate) fn placement_code(
        &self,
        player: usize,
        master: i16,
        at: [f32; 2],
        human: bool,
    ) -> Result<u8, Unsupported> {
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
            human,
            clearance: site.clearance,
            class: site.class,
            obstructs: !matches!(site.obstruction_type, 0 | 12),
            restriction,
            hill_mode: site.hill_mode,
            placement: site.placement,
            zones: false,
        };
        let map = crate::spawn::WorldMap { turns: self };
        crate::spawn::build_placement(&build, &map, at)
    }

    /// The payment's test of a foundation of `master` for `player` (`FUN_14162fb60` ->
    /// `FUN_141618db0`): every cost entry of a resource, paid or not, at most what the player
    /// holds. A cost in population (attribute 4) is not modelled.
    pub(crate) fn affords(&self, player: usize, master: i16) -> Result<bool, Unsupported> {
        let site = self.master_of(player, master)?;
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

    /// `FUN_14123bb20`'s sending off (with `DAT_1439e60e2` 1) of the units on `foundation`'s
    /// footprint: each of type 30 or more in state 2 whose top action is not on the foundation,
    /// that stands (`FUN_14114f740`: Gaia's of classes 9 and 10, or not walking, its animation
    /// speed `+0x7b8` at most 0, and its path level `+0x858` absent or in state 0 or 6) and
    /// whose unit AI's action is not 0x2bf or 0x2c0 (moving off already) is sent off: its unit AI
    /// gets notify 0x216 (slot 0x38) with the foundation master's radii times 100 (truncated) and
    /// the foundation ([`crate::unit_ai::SENT_OFF`]). The foundation owner's own units of type 70
    /// and more of a formation category (`+0x8bc`) would go together to its farthest free spot
    /// (`FUN_1411566b0` with 1, `FUN_1416123a0`'s group move, then 0x21b each), which is not
    /// modelled.
    fn evacuate(&mut self, foundation: i32, units: &[i32]) -> Result<(), Unsupported> {
        /// The notify's radii are hundredths (`DAT_14330ebcc`).
        const RADIUS_SCALE: f32 = crate::image::unit_ai::RADIUS_SCALE;
        let (site_owner, site) = self.object(foundation).ok_or(Unsupported(
            "a footprint's foundation the slice does not hold",
        ))?;
        let radius = self.master_of(site_owner, site.life.master_id)?.radius;
        let p = [
            (radius[0] * RADIUS_SCALE) as i32,
            (radius[1] * RADIUS_SCALE) as i32,
            foundation,
        ];
        let mut together = Vec::new();
        for &id in units {
            let Some((owner, u)) = self.object(id) else {
                continue;
            };
            let master = self.master_of(owner, u.life.master_id)?;
            if master.kind < 30 || u.life.state != 2 || u.top_target() == Some(foundation) {
                continue;
            }
            let walking = u.anim_speed.is_some_and(|s| 0.0 < s);
            let level = u
                .mover
                .as_ref()
                .and_then(|m| m.paths.first())
                .map(|l| l.state);
            let stands = (owner == 0 && matches!(master.class, 9 | 10))
                || (!walking && level.is_none_or(|s| matches!(s, 0 | 6)));
            let busy = u.ai.as_ref().is_none_or(|ai| {
                matches!(
                    ai.current_action,
                    crate::unit_ai::MOVE_ASIDE_ACTION | crate::unit_ai::GROUP_ASIDE_ACTION
                )
            });
            if !stands || busy {
                continue;
            }
            if owner == site_owner && 70 <= master.kind && master.formation_category != 0 {
                together.push(id);
                continue;
            }
            let state = u.life.state;
            let Some(crate::order::Place::Live(q, i)) = self.place(id) else {
                return Err(Unsupported("a sleeping unit sent off a footprint"));
            };
            if let Some(ai) = self.players[q].objects[i].ai.as_mut() {
                ai.post(Notify::own(id, crate::unit_ai::SENT_OFF, p), state)?;
            }
        }
        if !together.is_empty() {
            self.evacuate_together(foundation, radius, &together)?;
        }
        Ok(())
    }

    /// `FUN_14123bb20`'s owner's units of a formation category (`+0x8bc`) on `foundation`'s
    /// footprint (master radii `radius`), in the tiles' order: the farthest free spot beside the
    /// footprint from the first of them (`FUN_1411566b0` with 1, [`Spatial::spot_off`]); then, with
    /// one, `FUN_1416123a0`: the order splitter on them ([`Turns::loose_units`]: a unit group it
    /// forms, its group move `FUN_14100efd0`, is not modelled) and each unit it leaves loose takes
    /// the action of moving off a footprint (`0x2bf`) and moves to the spot at its own height
    /// (slot 0x138 without a target); last each of them whose unit AI's action is not `0x2bf` gets
    /// notify `0x21b` ([`crate::unit_ai::SENT_OFF_GROUP`]).
    fn evacuate_together(
        &mut self,
        foundation: i32,
        radius: [f32; 2],
        units: &[i32],
    ) -> Result<(), Unsupported> {
        let site_at = self
            .spatial
            .get(foundation)
            .ok_or(Unsupported("a footprint's foundation without a place"))?
            .position;
        let (_, first) = self
            .object(units[0])
            .ok_or(Unsupported("a unit sent off that is gone"))?;
        let mover = first
            .mover
            .as_ref()
            .ok_or(Unsupported("a unit sent off without movement state"))?;
        let spot = self.spatial.spot_off(
            first.position,
            (first.id, mover.stats.radius[0]),
            mover.stats.restriction_mask,
            (site_at, radius),
            true,
        )?;
        let Some([x, y]) = spot else {
            return Ok(());
        };
        let loose = self.loose_units(units)?;
        for id in loose {
            let Some(crate::order::Place::Live(q, i)) = self.place(id) else {
                continue;
            };
            let z = self.players[q].objects[i].position[2];
            if let Some(ai) = self.players[q].objects[i].ai.as_mut() {
                ai.set_action(crate::unit_ai::MOVE_ASIDE_ACTION);
            }
            self.move_point_order(q, i, [x, y, z])?;
        }
        for &id in units {
            let Some(crate::order::Place::Live(q, i)) = self.place(id) else {
                continue;
            };
            let unit = &mut self.players[q].objects[i];
            let state = unit.life.state;
            if let Some(ai) = unit.ai.as_mut()
                && ai.current_action != crate::unit_ai::MOVE_ASIDE_ACTION
            {
                ai.post(
                    Notify::own(id, crate::unit_ai::SENT_OFF_GROUP, [0, 0, 0]),
                    state,
                )?;
            }
        }
        Ok(())
    }

    /// The building constructor's terrain (`FUN_1411de5f0`, for a master with a build time, not
    /// of class 51 or 54, nor 357 or 1188): a farm (master 50) lays terrain 29 under its
    /// clearance, a rice farm (1187) terrain 65 (`FUN_1411ce250` -> `FUN_1411ddd90`,
    /// [`crate::terrain::Terrain::cover`]), before the footprint clearing.
    fn construction_terrain(
        &mut self,
        player: usize,
        master: i16,
        position: [f32; 3],
    ) -> Result<(), Unsupported> {
        /// The farms and the terrain their foundations lay (`0x1d`, `0x41`).
        const LAID: [(i16, u8); 2] = [(50, 29), (1187, 65)];
        let Some(&(_, laid)) = LAID.iter().find(|(m, _)| *m == master) else {
            return Ok(());
        };
        let m = self.master_of(player, master)?;
        if m.build_time <= 0 {
            return Ok(());
        }
        let f = crate::terrain::Foundation {
            position,
            clearance: m.clearance,
            terrain: laid,
            snow: false,
            master,
            class: m.class,
            terrain_bonus: m.terrain_bonus,
            mode_680: self.mode_680,
        };
        self.spatial
            .terrain
            .as_mut()
            .ok_or(Unsupported("a farm's foundation without the map's terrain"))?
            .cover(&f)
    }

    /// The footprint clearing (`FUN_1415c3620` with `param_7` 0) of a new building of `master`
    /// of `player` at `at`: the building constructor's (`FUN_1411de5f0`, `trees` false) and a new
    /// object's (`FUN_14162ce00` with its flag, `trees` true). The objects it takes
    /// ([`crate::build::footprint_clearing`]), each in the tiles' order: [`Turns::clear_object`].
    /// Nothing for a master the player has not got.
    ///
    /// # Errors
    ///
    /// What the choice and the deletes do not model.
    pub(crate) fn clear_footprint(
        &mut self,
        player: usize,
        master: i16,
        at: [f32; 2],
        trees: bool,
    ) -> Result<(), Unsupported> {
        let Some(m) = self.masters.get(player).and_then(|m| m.get(master)) else {
            return Ok(());
        };
        let site = crate::build::ClearingSite {
            player,
            master,
            class: m.class,
            radius: m.radius,
            at,
            trees,
        };
        let look = |id: i32| self.clear_look(id);
        let cleared = crate::build::footprint_clearing(&self.spatial, &site, &look)?;
        for cl in cleared {
            self.clear_object(cl, false)?;
            if !trees {
                continue;
            }
            // FUN_141189670 after its destroy: unless +0x686 is set, a tall master's (COMISS: a
            // NaN height too) or a recorded object's record goes; then +0x686 (FUN_141189b60)
            let look = self.clear_look(cl.id).ok_or(Unsupported(
                "a footprint clearing of an object the slice does not hold",
            ))?;
            let recorded = self
                .spatial
                .get(cl.id)
                .is_some_and(|o| o.obstruction.is_some());
            if !look.roo_called && (crate::fmath::below_or_unordered(0.0, look.height) || recorded)
            {
                self.pathing.removed(cl.id);
                self.spatial.drop_record(cl.id);
                self.pathing.tiles_changed(self.spatial.take_changes());
            }
            if let Some((_, u)) = self.object_mut(cl.id) {
                u.roo_called = true;
            }
        }
        Ok(())
    }

    /// What the footprint clearing reads of object `id` ([`crate::build::ClearLook`]).
    pub(crate) fn clear_look(&self, id: i32) -> Option<crate::build::ClearLook> {
        let (p, u) = self.object(id)?;
        clear_look_of(u, self.masters.get(p))
    }

    /// An object the footprint clearing deletes ([`crate::build::Cleared`]): a wall first goes
    /// through the building's delete core (`FUN_1411ccbb0`, [`Turns::remove_building`]), then
    /// its destroy (slot 0xd0, [`Turns::destroy`]). The builder's clearing (`roo_done`) dropped
    /// its record in the Build's update already (`FUN_141189670`): `+0x686` is set
    /// (`FUN_141189b60` with 1).
    ///
    /// # Errors
    ///
    /// An object the slice does not hold, and what the deletes do not model.
    pub(crate) fn clear_object(
        &mut self,
        cl: crate::build::Cleared,
        roo_done: bool,
    ) -> Result<(), Unsupported> {
        let (owner, _) = self.object(cl.id).ok_or(Unsupported(
            "a footprint clearing of an object the slice does not hold",
        ))?;
        if cl.wall {
            self.remove_building(owner, cl.id)?;
        }
        self.destroy(owner, cl.id)?;
        if roo_done && let Some((_, u)) = self.object_mut(cl.id) {
            u.roo_called = true;
        }
        Ok(())
    }

    /// A new building's place (its static init's `new_position`): it takes its place on the map,
    /// with its obstruction record unless it is desolid (`FUN_1411cf770` drops the one its place
    /// gave it; its first builder brings it back), and its sight, which waits for its
    /// construction to start (its place links its tiles to its anchor, `FUN_1411e6780`, and its
    /// desolid flag unlinks them).
    fn building_place(
        &mut self,
        player: usize,
        id: i32,
        made: &Master,
        position: [f32; 3],
        desolid: bool,
    ) {
        let record = crate::spatial::Obstruction {
            kind: made.obstruction_type,
            class: made.obstruction_class,
        };
        let placed = Placed {
            position,
            radius: made.radius,
            class: made.class,
            player,
            master: made.id,
            obstruction_type: made.obstruction_type,
            buildable_on: made.buildable_on,
            tall: made.tall,
            speed: 0.0,
            unknown: None,
            obstruction: (!desolid && record.recorded()).then_some(record),
        };
        self.spatial.insert(id, placed);
        if placed.obstruction.is_some() {
            self.pathing.moved(id, &placed, false);
            self.pathing.tiles_changed(self.spatial.take_changes());
        }
        if let Some(v) = self.spatial.sight.as_mut() {
            let umv = v.link_building(position, made.radius);
            if desolid {
                v.unlink_building(position, made.radius);
            }
            v.add_seer(
                id,
                Seer {
                    player,
                    master: made.id,
                    los: made.line_of_sight,
                    state: 0,
                    explored: false,
                    building: Some(BuildingSight::default()),
                    umv: umv as u8,
                    capture: capture_kind(player, made.class, made.id),
                    kind: 80,
                },
            );
        }
    }

    /// BUILD's builders (the cost was paid with the foundation, [`Turns::found`]; the pending
    /// debit came before, [`Turns::debit_listed`]), the handler's last loop: each unit of the
    /// packet the world holds (`FUN_1415417a0`: any player's, asleep or not), in the packet's
    /// order, of a master of type 30 and up that moves (its slot 0xe8; COMISS: a NaN speed too),
    /// or any unit of a queued BUILD: its ignore list empties (`FUN_141197da0`); with a unit AI,
    /// a queued BUILD, or one that `commits` while the AI holds an order not ready
    /// (`FUN_140c4dfa0`), goes into its queue as the build order (slot 0x30 into slot 0x28:
    /// [`Turns::queue_build`], issued by the unit's own player); else its slot 0x128 on the
    /// foundation: a static object's (types below 40, `FUN_141071bd0`) does nothing, a unit's
    /// takes the Build ([`Turns::build_task`], [`Turns::build_directly`]), another player's
    /// whatever task its slot 0x7e8 finds on the foundation ([`Turns::order_work`]). A unit the world no
    /// longer holds is passed over. Not modelled: the readiness pass of a committing BUILD with
    /// orders queued (`FUN_140c42b60`).
    fn task_builders(
        &mut self,
        player: usize,
        foundation: i32,
        master: i16,
        position: [f32; 3],
        builders: &[i32],
        (queued, commits): (bool, bool),
    ) -> Result<(), Unsupported> {
        // the builders go through the order splitter, which counts the command
        self.groups.count_command();
        let site = (player, foundation, master);
        for &id in builders {
            let Some((q, unit)) = self.object(id) else {
                continue;
            };
            let m = self.master_of(q, unit.life.master_id)?;
            let speed = m.moving.map_or(0.0, |s| s.speed);
            let kind = m.kind;
            let moves = 0x1d < kind && crate::fmath::below_or_unordered(0.0, speed);
            if !moves && !queued {
                continue;
            }
            let to_queue = unit
                .ai
                .as_ref()
                .is_some_and(|ai| queued || (commits && ai.has_unready_order()));
            let Some(crate::order::Place::Live(q, idx)) = self.place(id) else {
                // a sleeping object is a static one without a unit AI: its slot 0x128 does
                // nothing
                continue;
            };
            if to_queue {
                self.queue_build(q, idx, site, position)?;
                if commits {
                    return Err(Unsupported(
                        "the readiness pass of a committing BUILD with orders queued (FUN_140c42b60)",
                    ));
                }
            } else if STATIC_TYPES_END <= kind && q == player {
                let task = self.build_task(q, id, site, position)?;
                self.build_directly(q, idx, task, site, position)?;
            } else if STATIC_TYPES_END <= kind {
                // another player's unit: slot 0x128's slot 0x7e8 finds whatever task it has on
                // the foundation (a Build on an ally's), the work command's general path
                let work = crate::order::Work {
                    target: foundation,
                    point: [position[0], position[1], 0.0],
                    carry_test: false,
                    filter: -1,
                    notified: true,
                };
                self.order_work(q, idx, &work)?;
            } else if let Some(m) = self.players[q].objects[idx].mover.as_mut() {
                m.ignore_units.clear();
            }
        }
        Ok(())
    }

    /// What slot 0x128 on `foundation` (of `master`, player `fp`'s, at `position`) takes for the
    /// builder `id` of player `q`: the master it builds with ([`Masters::work_master`], with
    /// `q`'s masters and attributes) and the Build action of that master's task. A builder
    /// whose action would be replaced, without a Build task for it or with one not enabled, is
    /// not modelled.
    pub(crate) fn build_task(
        &self,
        q: usize,
        id: i32,
        (fp, foundation, master): (usize, i32, i16),
        position: [f32; 3],
    ) -> Result<(i16, BuildAction), Unsupported> {
        let masters = self
            .masters
            .get(q)
            .ok_or(Unsupported("BUILD tasks the builders: no master table"))?;
        let site = self
            .masters
            .get(fp)
            .and_then(|m| m.get(master))
            .ok_or(Unsupported("BUILD of a master the player does not have"))?;
        let target = TaskTarget {
            id: foundation,
            master: site,
            player: fp,
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
        let owner = &world.players[q];
        let unit = owner
            .objects
            .iter()
            .find(|o| o.id == id)
            .ok_or(Unsupported("a builder the slice does not have"))?;
        if unit.artifact.is_some() || unit.production.is_some() || unit.fly.is_some() {
            return Err(Unsupported("a builder with an action (its replacement)"));
        }
        let actor = Actor {
            id,
            master: masters
                .get(unit.life.master_id)
                .ok_or(Unsupported("a unit of a master the player does not have"))?,
            player: q,
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
        Ok((with, BuildAction::start(foundation, gap, task)))
    }

    /// The queue path of a builder at `idx` of player `q` (slot 0x30 into slot 0x28): its
    /// ignore list empties (`FUN_141197da0`) and its unit AI's queue takes the build order on
    /// `foundation` (of `master`, player `fp`'s) at `position`, issued by `q`, priority 100, the
    /// point shown, ready, as the game's saves hold it ([`Turns::build_order`] runs it).
    pub(crate) fn queue_build(
        &mut self,
        q: usize,
        idx: usize,
        (fp, foundation, master): (usize, i32, i16),
        position: [f32; 3],
    ) -> Result<(), Unsupported> {
        let unit = &mut self.players[q].objects[idx];
        if let Some(m) = unit.mover.as_mut() {
            m.ignore_units.clear();
        }
        let state = unit.life.state;
        let order = crate::unit_ai::QueuedOrder {
            issuer: q as i32,
            order: BUILD_ORDER,
            priority: 100,
            target: foundation,
            target_owner: fp as i32,
            point: [position[0], position[1], 0.0],
            range: 0.0,
            shown: [position[0], position[1]],
            ready: true,
            target_type: i32::from(master),
        };
        let ai = unit
            .ai
            .as_mut()
            .ok_or(Unsupported("a queued build order without a unit AI"))?;
        let _ = ai.queue_order(state, order, false, false)?;
        Ok(())
    }

    /// The direct path of a builder at `idx` of player `q`, slot 0x128 on `foundation` (player
    /// `fp`'s) at `position` with what [`Turns::build_task`] found: its ignore list empties
    /// (`FUN_141197da0`), the town-bell record (slot 0x7e8 with task filter −1), then the work
    /// command (`FUN_14106ff70`): the master changes, the Build takes its foundation, the unit
    /// AI's notify, the action list clears and holds the Build.
    pub(crate) fn build_directly(
        &mut self,
        player: usize,
        idx: usize,
        (with, build): (i16, BuildAction),
        (fp, foundation, master): (usize, i32, i16),
        position: [f32; 3],
    ) -> Result<(), Unsupported> {
        let site_class = i32::from(self.master_of(fp, master)?.class);
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
        Ok(())
    }

    /// The build order (`0x2be`) a unit AI's queue runs (slot 0x230's case: slot 0xf0 with 1 on its
    /// foundation, [`Turns::build_object`]), then the tail takes the order (702) with its
    /// priority.
    pub(crate) fn build_order(
        &mut self,
        player: usize,
        idx: usize,
        order: crate::unit_ai::QueuedOrder,
    ) -> Result<(), Unsupported> {
        self.build_object(player, idx, order.target)?;
        let ai = self.players[player].objects[idx]
            .ai
            .as_mut()
            .ok_or(Unsupported("a build order without a unit AI"))?;
        // the tail (`LAB_140c5d432`)
        ai.current_order = BUILD_ORDER;
        ai.order_priority = order.priority;
        Ok(())
    }

    /// `BuildObject` (the unit AI's slot 0xf0, `FUN_140c47d80`, with 1) on `foundation`: the
    /// unit's build command ([`Turns::build_command`]), then the AI's action becomes the build
    /// action (602; the last one kept) and its target the foundation (the last one kept) at its
    /// place with its master's class. Unlike BUILD's own path neither the town-bell record nor
    /// the work's notify.
    pub(crate) fn build_object(
        &mut self,
        player: usize,
        idx: usize,
        foundation: i32,
    ) -> Result<(), Unsupported> {
        let (class, f_at) = self.build_command(player, idx, foundation)?;
        let ai = self.players[player].objects[idx]
            .ai
            .as_mut()
            .ok_or(Unsupported("a BuildObject without a unit AI"))?;
        // FUN_140c47d80
        ai.last_action = ai.current_action;
        ai.current_action = BUILD_ACTION;
        ai.target_point = f_at;
        ai.last_target = ai.current_target;
        ai.last_target_type = ai.target_type;
        ai.current_target = foundation;
        ai.target_type = class;
        Ok(())
    }

    /// The unit's slot 0x388 (`FUN_141226fc0`) after its unit AI's update, with what it found
    /// there ([`Wait`]): slot 0x820's switch to the master its swap group gave (slot 0x90 with 0,
    /// [`Turns::change_master`]; a villager's candidates are of its own type, never a
    /// building's), the action list cleared (`FUN_140a98620`, [`Turns::clear_actions`]; a wait
    /// it held goes too, its teardown the base one's) and the new wait in it (slot 0x808,
    /// `FUN_14106e990`: the wait's slot 0x70 with no action and its start both do nothing).
    pub(crate) fn wait_for_foundation(
        &mut self,
        player: usize,
        idx: usize,
        wait: Wait,
    ) -> Result<(), Unsupported> {
        if let Some(master) = wait.switch {
            self.change_master(player, idx, master)?;
        }
        self.clear_actions(player, idx)?;
        self.players[player].objects[idx].wait_solidify = Some(
            crate::wait_solidify::WaitSolidify::new(wait.task, wait.target),
        );
        Ok(())
    }

    /// The unit's build command (slot 0x380, `FUN_14120d410`, forced) on `foundation`: a Build on
    /// top takes the foundation (its slot 0x48, [`Turns::build_take`]); without one its slot
    /// 0x820 with any action type and the switch ([`task_search`] on the foundation and its main
    /// building; the switch, slot 0x90 with 0, [`Turns::change_master`]) finds a task, and a
    /// Build action on it (`FUN_140aad4e0` with that task, whatever its type) replaces the
    /// unit's actions. The foundation's master's class and its place, for the unit AI. A
    /// foundation that is gone or without a task (slot 0x380's 0) is not modelled.
    pub(crate) fn build_command(
        &mut self,
        player: usize,
        idx: usize,
        foundation: i32,
    ) -> Result<(i32, [f32; 3]), Unsupported> {
        if self.players[player].objects[idx].top_action_kind() == Some(BUILD) {
            let class = self.build_take(player, idx, foundation)?;
            let f_at = self
                .object(foundation)
                .map(|(_, f)| f.position)
                .ok_or(Unsupported("a BuildObject on a foundation that is gone"))?;
            return Ok((class, f_at));
        }
        let gone = Unsupported("a build order on a foundation that is gone");
        let (fp, f) = self.object(foundation).ok_or(gone)?;
        let (site, f_at) = (self.master_of(fp, f.life.master_id)?, f.position);
        let find = |id: i32| self.object(id);
        let t = task_target_with(foundation, 1, &find, &self.masters)?.ok_or(gone)?;
        let general = task_target_with(foundation, 2, &find, &self.masters)?.ok_or(gone)?;
        let masters = self
            .masters
            .get(player)
            .ok_or(Unsupported("a build order: no master table"))?;
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
        // slot 0x820 with any action type (0xffff): whatever task it finds, the Build takes
        // (FUN_140aad4e0 on it), its type and its enablement unread
        let (task, switch) = task_search(masters, &actor, &t, &general, -1, &world)?
            .ok_or(Unsupported("a build order with no task for its foundation"))?;
        let builder = switch.unwrap_or(actor.master);
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
        if let Some(m) = switch.map(|m| m.id) {
            self.change_master(player, idx, m)?;
        }
        self.take(foundation);
        self.clear_actions(player, idx)?;
        self.players[player].objects[idx].build = Some(build);
        Ok((class, f_at))
    }

    /// A mule cart's completion (`FUN_1411c6bd0`'s tail, a building whose master has a task of
    /// action type 0x99, [`crate::build::Built::cart`]) with a unit AI: with no current target a
    /// human player's runs its AI's slot 0x1e8 (`FUN_140c16240`; its world test `FUN_141533b60`
    /// and the game mode's, `FUN_140c87f20` not 4, hold in every recording probed): per task of
    /// its master in order, the target class of the last of type 0x99 so far names a gather group
    /// (`FUN_140fdd5c0` with 1: 1 for 7, 3 for 8, 0 for 15, 4 for 32 and 63), each searched by
    /// `ClosestResourceObject` (slot 0x80) from the cart as the drop site, any workers, from
    /// state 0 on; of those found, the nearest by the squared footprint gap (`FUN_14118f8c0`,
    /// below 99999, `DAT_142ef9008`); then slot 0x140 (`FUN_141070c90`): the work command toward
    /// it at its place, else at the cart's own (slot 0x7e8: no carried-resource test, any task;
    /// `+0x888`'s count is not held). A cart with a target moves to it (slot 0x138): not modelled.
    pub(crate) fn cart_search(&mut self, p: usize, id: i32, state: u8) -> Result<(), Unsupported> {
        const CART_REACH: f32 = crate::image::turn::CART_REACH;
        let Some(idx) = self.players[p].objects.iter().position(|u| u.id == id) else {
            return Ok(());
        };
        let Some(current) = self.players[p].objects[idx]
            .ai
            .as_ref()
            .map(|ai| ai.current_target)
        else {
            return Ok(());
        };
        let found = if current == -1 && !self.players[p].computer {
            if self.players[p].objects[idx].mover.is_none() {
                return Err(Unsupported(
                    "a mule cart's search without its movement state (a moving building's setup)",
                ));
            }
            let master = self.master_of(p, self.players[p].objects[idx].life.master_id)?;
            let mut kinds = Vec::new();
            let mut class = -1;
            for t in &master.tasks {
                if t.action_type == crate::build::CART_TASK {
                    class = t.object_group;
                }
                let kind = match class {
                    7 => 1,
                    8 => 3,
                    15 => 0,
                    32 | 63 => 4,
                    _ => continue,
                };
                kinds.push(kind);
            }
            let cart = std::mem::take(&mut self.players[p].objects[idx]);
            let result = (|| {
                let (before, rest) = self.players.split_at_mut(p);
                let (player, after) = rest.split_first_mut().expect("p is a player");
                let mut w = ResourceWorld {
                    player: p,
                    spatial: &self.spatial,
                    zones: &mut self.zones,
                    foreign: Foreign {
                        before: &*before,
                        after: &*after,
                    },
                    every: &self.masters,
                    masters: self.masters.get(p),
                    plan: Planning {
                        pathing: &mut self.pathing,
                        now: self.world_time,
                        computer: player.computer,
                        object: self.streams.get(Stream::Object),
                        construction_rate: player.construction_rate,
                        groups: &self.groups,
                    },
                };
                let mut found = Vec::new();
                for kind in kinds {
                    let (f, _) = closest_resource_of(
                        &cart,
                        &mut w,
                        &player.objects,
                        (kind, -1, id),
                        (current, 0),
                    )?;
                    if f != -1 {
                        found.push(f);
                    }
                }
                Ok::<_, Unsupported>(found)
            })();
            self.players[p].objects[idx] = cart;
            let found = result?;
            let cart_master = self.master_of(p, self.players[p].objects[idx].life.master_id)?;
            let at = self.players[p].objects[idx].position;
            let mut best: Option<(i32, [f32; 3])> = None;
            let mut best_d = CART_REACH;
            for f in found {
                let Some(o) = self.spatial.get(f) else {
                    continue;
                };
                let d = footprint_gap_squared(at, cart_master.radius, o.position, o.radius);
                if d < best_d {
                    best = Some((f, o.position));
                    best_d = d;
                }
            }
            Some(best.unwrap_or((-1, at)))
        } else {
            None
        };
        // all of it runs inside the completion's set_state(2), before the cart is in state 2
        let built = std::mem::replace(&mut self.players[p].objects[idx].life.state, state);
        let result = (|| {
            if let Some((target, point)) = found {
                let work = crate::order::Work {
                    target,
                    point,
                    carry_test: false,
                    filter: -1,
                    notified: true,
                };
                self.order_work(p, idx, &work)?;
            }
            // with a target (the one the work command gave the unit AI) the cart moves to it
            // (slot 0x138 at its place)
            let target = self.players[p].objects[idx]
                .ai
                .as_ref()
                .map_or(-1, |ai| ai.current_target);
            if target != -1
                && let Some(at) = self.spatial.get(target).map(|o| o.position)
            {
                self.move_to_object(p, idx, target, at)?;
            }
            Ok::<_, Unsupported>(())
        })();
        self.players[p].objects[idx].life.state = built;
        result
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
            let unit = &mut self.players[player].objects[i];
            unit.gather_point = gather;
            // the unit AI's slot 0x210 (FUN_140c16790): its command queue empties
            if let Some(ai) = unit.ai.as_mut() {
                ai.building_orders.clear();
            }
        }
        Ok(())
    }

    /// `BUILDING_COMMAND_QUEUE`'s handler (`0x140e64310`) for `player`: each object of `ids`
    /// the world holds with a unit AI queues the step (its slot 0x208, `FUN_140c15740`: the
    /// player, the order `0x2db` on a target or `0x2c6` on the point, priority −1, the target,
    /// its owner and class, the point with height −1, range 0, not ready). A building first puts
    /// its gather point in the queue when it has one, the queue is empty (slot 0x218) and the
    /// point is not on itself, then the gather point clears (`FUN_1411c7ff0` with −1). A target
    /// the world does not hold queues the point alone. The view's marker (`FUN_1414cd6d0`) is not
    /// held.
    pub(crate) fn command_queue(
        &mut self,
        player: usize,
        ids: &[i32],
        target: i32,
        [x, y]: [f32; 2],
    ) -> Result<(), Unsupported> {
        let step = |turns: &Self,
                    t: i32,
                    at: [f32; 2]|
         -> Result<crate::unit_ai::QueuedOrder, Unsupported> {
            let named = if t == -1 { None } else { turns.object(t) };
            let (target, owner, class) = match named {
                Some((tp, u)) => (
                    t,
                    tp as i32,
                    i32::from(turns.master_of(tp, u.life.master_id)?.class),
                ),
                None => (-1, -1, -1),
            };
            Ok(crate::unit_ai::QueuedOrder {
                issuer: player as i32,
                order: if target < 0 {
                    crate::unit_ai::MOVE_ORDER
                } else {
                    crate::unit_ai::ORDER_ON_TARGET
                },
                priority: -1,
                target,
                target_owner: owner,
                point: [at[0], at[1], -1.0],
                range: 0.0,
                shown: [-1.0, -1.0],
                ready: false,
                target_type: class,
            })
        };
        for &id in ids {
            let Some(crate::order::Place::Live(p, i)) = self.place(id) else {
                if self.place(id).is_some() {
                    return Err(Unsupported("a command queue on a sleeping object"));
                }
                continue;
            };
            let unit = &self.players[p].objects[i];
            let Some(ai) = unit.ai.as_ref() else {
                continue;
            };
            let building = self.master_of(p, unit.life.master_id)?.kind == crate::order::BUILDING;
            let mut queued = Vec::new();
            if building
                && let Some(g) = unit.gather_point
                && ai.building_orders.is_empty()
                && g.target != unit.id
            {
                // the gather point's own target id stays even when the world lost it
                let mut first = step(self, g.target, [g.point[0], g.point[1]])?;
                if first.target == -1 && g.target != -1 {
                    first.target = g.target;
                    first.order = crate::unit_ai::ORDER_ON_TARGET;
                }
                queued.push(first);
            }
            queued.push(step(self, target, [x, y])?);
            let unit = &mut self.players[p].objects[i];
            if building {
                unit.gather_point = None;
            }
            if let Some(ai) = unit.ai.as_mut() {
                ai.building_orders.extend(queued);
            }
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
    /// takes the action value 0x26c. A technology of `linkedTechs.json` ([`Turns::linked_techs`],
    /// the game settings' table `FUN_140f39f40` looks up) is not modelled (`FUN_140f3a0f0`,
    /// `FUN_140f39e80`). Not held: the interface's view and selection (`FUN_140e6de20`), the
    /// local player's statistics flag. Not modelled: the computer players' parts (`player+0x7a8`, `FUN_1411d5520`), the player's
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
        if self.linked_techs.contains_key(&r.tech) {
            return Err(Unsupported(
                "RESEARCH of a linked technology (FUN_140f3a0f0, FUN_140f39e80)",
            ));
        }
        // a technology that is not available (state 1) and may not be queued again is
        // researchable only when it is linked (`FUN_140f39e80`): no building takes it
        if state.state != 1 && !repeatable {
            return Ok(());
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
    /// did; a building that produced nothing yet gets its queue at its work rate. Not held: the
    /// building's queue entries as the game groups them (the slice's queue holds one item per
    /// research), their costs (`FUN_1411c81a0`, `FUN_1411c3190`) and serials (`+0xcd8`).
    fn queue_research(
        &mut self,
        player: usize,
        i: usize,
        tech: i16,
        shift: bool,
    ) -> Result<bool, Unsupported> {
        let t = tech as usize;
        let def = &self.techs[t];
        // a building that produced nothing yet gets its queue now, at its work rate (its
        // `mWorkRatePowerup` and its master's `work_rate`), which the research's update reads
        let rate = {
            let b = &self.players[player].objects[i];
            b.work_rate_powerup + self.master_of(player, b.life.master_id)?.work_rate
        };
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
        let production = building.production.get_or_insert_with(|| Production {
            queue: std::collections::VecDeque::default(),
            action: false,
            make_paid: false,
            progress: 0.0,
            rate,
        });
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

    /// QUEUE at one building. The player's part (`FUN_141614450`) first caps `amount` at the
    /// building's room (15 queued, a computer player's 2), then lowers it while the player cannot
    /// pay for that many: each cost entry but the population's, paid or not, needs its amount
    /// times the count. With none left nothing is queued and nothing drawn. Then the building's
    /// part (`FUN_1411cbc40`): per unit (it recurses, so the last unit drawn is queued first) the
    /// villager's male or female draw, then the cost, the queued units' population (attribute
    /// 80, per villager, any other unit's for all at once) and the queue entry, which keeps the
    /// master's three cost entries (resource and amount) for a cancel's refund. A unit the player
    /// cannot pay for (`FUN_14162fb60` -> `FUN_141618db0`) is not queued; its draw stays. The
    /// limited units' counts (`FUN_1416116c0`, `FUN_141611520`, the master's `+0x5f1` flags) and
    /// a building that cannot train the unit (`FUN_141263610`) are not modelled.
    fn queue(
        &mut self,
        player: usize,
        q: &QueueAt,
        cost: &[(i16, i16, bool)],
    ) -> Result<(), Unsupported> {
        if self.pathing.game_type == ONE_UNIT_GAME {
            return Err(Unsupported("QUEUE in the one-unit game type"));
        }
        let owner = &self.players[player];
        let index = owner
            .objects
            .iter()
            .position(|o| o.id == q.building)
            .ok_or(Unsupported("QUEUE at a building the slice does not have"))?;
        let queued = owner.objects[index]
            .production
            .as_ref()
            .map_or(0, |p| p.queue.len());
        let limit = if owner.computer {
            COMPUTER_QUEUE_LIMIT
        } else {
            QUEUE_LIMIT
        };
        let mut amount = usize::from(q.amount).min(limit.saturating_sub(queued));
        let can_pay = |count: usize| -> Result<bool, Unsupported> {
            let mut ok = true;
            for &(resource, each, _) in cost.iter().filter(|c| 0 <= c.0 && c.0 != POPULATION) {
                let held = owner
                    .resources
                    .get(resource as usize)
                    .ok_or(Unsupported("a cost in a resource the slice does not track"))?;
                // the game compares in f32: (float)amount * (float)count
                ok &= f32::from(each) * count as f32 <= *held;
            }
            Ok(ok)
        };
        while amount > 0 && !can_pay(amount)? {
            amount -= 1;
        }
        let action = self.streams.get(Stream::Action);
        let masters: Vec<i16> = (0..amount)
            .map(|_| match q.unit {
                VILLAGER | VILLAGER_ANY => villager(action.raw64()),
                other => other,
            })
            .collect();
        let pop = |master: i16| {
            self.masters
                .get(player)
                .and_then(|m| m.get(master))
                .map_or(0.0, |m| crate::production::population(&m.storage))
        };
        let pops: Vec<f32> = masters.iter().map(|&m| pop(m)).collect();
        let villagers = matches!(q.unit, VILLAGER | VILLAGER_ANY);
        let mut queued = 0u16;
        let owner = &mut self.players[player];
        for (master, pop) in masters.into_iter().zip(pops).rev() {
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
            // the queued units' population (attribute 80, the player's slot 0xf8 with the
            // master's population, FUN_14163b0b0): a villager's per unit, as the routine recurses
            // for each; a population of 0 changes nothing
            if villagers && pop != 0.0 {
                owner.add_attribute(crate::production::QUEUED, pop)?;
            }
            queued += 1;
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
        // any other unit's: its population times the count paid for, at once
        let pop = pop(q.unit) * f32::from(queued);
        if !villagers && 0 < queued && pop != 0.0 {
            self.players[player].add_attribute(crate::production::QUEUED, pop)?;
        }
        Ok(())
    }

    /// Player `q`'s gate list (`+0x1360`, `FUN_141616200` adds to its front): its live objects
    /// of class 39 whose master transforms ([`crate::masters::BuildingMaster::transform_unit`]),
    /// newest first.
    fn gate_list(&self, q: usize) -> Vec<usize> {
        let masters = self.masters.get(q);
        let owner = &self.players[q];
        (0..owner.objects.len())
            .rev()
            .filter(|&i| {
                masters
                    .and_then(|m| m.get(owner.objects[i].life.master_id))
                    .is_some_and(|m| {
                        m.class == crate::spatial::GATE_CLASS
                            && m.building.as_ref().is_some_and(|b| 0 <= b.transform_unit)
                    })
            })
            .collect()
    }

    /// `FUN_140e6cfc0`: each of an order's units (INTERACT, MOVE, RETREAT) the world holds,
    /// sleeping ones too (`FUN_1415417a0`), with a unit AI resets it for the order
    /// ([`UnitAi::reset_for_order`]), in list order.
    fn reset_order_units(&mut self, units: &[i32]) {
        for &id in units {
            let Some(place) = self.place(id) else {
                continue;
            };
            let (p, i) = match place {
                crate::order::Place::Live(p, i) | crate::order::Place::Sleeping(p, i) => (p, i),
            };
            let player = &mut self.players[p];
            let unit = match place {
                crate::order::Place::Live(..) => &mut player.objects[i],
                crate::order::Place::Sleeping(..) => &mut player.sleeping[i],
            };
            if let Some(ai) = unit.ai.as_mut() {
                ai.reset_for_order(self.streams.get(Stream::Ai));
            }
        }
    }

    /// A gate's slot 0x678 (`FUN_1411d4a30`): player `p` may pass object `i` of player `q` when it
    /// stands (state below 3), its master is of class 39 and transforms (`+0x9ba` not -1), it is
    /// not locked (`mGateLocked`) and its owner and `p` count each other as allies
    /// (`FUN_141663dc0`, `FUN_141663d90`; a player counts itself).
    fn gate_passes(&self, q: usize, i: usize, p: usize) -> bool {
        let gate = &self.players[q].objects[i];
        let transforming = self
            .masters
            .get(q)
            .and_then(|m| m.get(gate.life.master_id))
            .is_some_and(|m| {
                m.class == crate::spatial::GATE_CLASS
                    && m.building.as_ref().is_some_and(|b| b.transform_unit != -1)
            });
        gate.life.state < 3
            && transforming
            && !gate.gate.is_some_and(|g| g.locked)
            && self.spatial.allied_with(q, p)
            && self.spatial.allied_with(p, q)
    }

    /// `BaseWorld::update` before player `p` updates (not Gaia): the gates of each player `q`
    /// that is `p` or counts `p` an ally and is counted one by it, unless `q`'s are open already
    /// (`open`, the update's flags), open for `p` (`q`'s slot 0x318, `FUN_14162faf0`): each gate
    /// `p` may pass ([`Turns::gate_passes`]) loses its record, its tiles' marks and its mesh shape
    /// (`FUN_141055b40`), so `p`'s units walk and plan through it.
    fn open_gates(&mut self, p: usize, open: &mut [bool]) -> Result<(), Unsupported> {
        if p == 0 {
            return Ok(());
        }
        let players = self.players.len();
        for (q, is_open) in open.iter_mut().enumerate().take(players).skip(1) {
            if *is_open
                || (q != p && !(self.spatial.allied_with(p, q) && self.spatial.allied_with(q, p)))
            {
                continue;
            }
            for i in self.gate_list(q) {
                if !self.gate_passes(q, i, p) {
                    continue;
                }
                let id = self.players[q].objects[i].id;
                if self
                    .spatial
                    .get(id)
                    .is_some_and(|g| g.obstruction.is_some_and(|o| o.kind == 10))
                {
                    return Err(Unsupported(
                        "a gate of obstruction kind 10 opens (FUN_140f65580)",
                    ));
                }
                self.spatial.drop_record(id);
                self.pathing.removed(id);
            }
            *is_open = true;
        }
        self.pathing.tiles_changed(self.spatial.take_changes());
        Ok(())
    }

    /// `BaseWorld::update` after player `p` updated (not Gaia): each open player `q`'s gates close
    /// (`q`'s slot 0x320, `FUN_141619480`) unless the next player counts `q` an ally and is
    /// counted one by it (they stay open for it): each gate `p` may pass gets its record, its
    /// tiles' marks and its mesh shape again (`FUN_141056460`), but for a part of obstruction
    /// kind 0 or 12 or of class 0 ([`crate::spatial::Obstruction::recorded`]).
    fn close_gates(&mut self, p: usize, open: &mut [bool]) {
        if p == 0 {
            return;
        }
        let (next, players) = (p + 1, self.players.len());
        for (q, is_open) in open.iter_mut().enumerate().take(players).skip(1) {
            if !*is_open {
                continue;
            }
            if next < players
                && self.spatial.allied_with(next, q)
                && self.spatial.allied_with(q, next)
            {
                continue;
            }
            for i in self.gate_list(q) {
                if !self.gate_passes(q, i, p) {
                    continue;
                }
                let gate = &self.players[q].objects[i];
                let (id, master) = (gate.id, gate.life.master_id);
                let Some(m) = self.masters.get(q).and_then(|t| t.get(master)) else {
                    continue;
                };
                let record = crate::spatial::Obstruction {
                    kind: m.obstruction_type,
                    class: m.obstruction_class,
                };
                // a gate part of obstruction kind 0 (the gate's middle, master 91) gets none
                if record.recorded()
                    && self
                        .spatial
                        .get(id)
                        .is_some_and(|g| g.obstruction.is_none())
                {
                    self.spatial.add_record(id, record);
                    if let Some(placed) = self.spatial.get(id).copied() {
                        self.pathing.place_shape(id, &placed);
                    }
                }
            }
            *is_open = false;
        }
        self.pathing.tiles_changed(self.spatial.take_changes());
    }

    /// `Process_New_Tiles` (`FUN_1412899a0`, the end of `WorldPlayerBase::update`): on each tile
    /// player `p`'s map explored since its last update, in order, each object (the tile's list,
    /// in its order) of a master that gathers by itself (`enable_auto_gather`, `+0x681`) joins
    /// the player's list of its group (`+0x683`) at its place ([`VisibleResources::add`]). A
    /// ghost's master (243) does not gather; no recyclable master does, so the objects have ids of
    /// 0 and up (`FUN_141288830` reads them from the world's table `DAT_143f69168`).
    ///
    /// # Errors
    ///
    /// An object of a master the slice does not have.
    ///
    /// [`VisibleResources::add`]: crate::visibility::VisibleResources::add
    pub(crate) fn process_new_tiles(&mut self, p: usize) -> Result<(), Unsupported> {
        let Some(sight) = self.spatial.sight.as_ref() else {
            return Ok(());
        };
        let (Some(tiles), Some(lists)) = (sight.new_tiles.get(p), sight.resources.get(p)) else {
            return Ok(());
        };
        if lists.lists.is_empty() {
            return Ok(());
        }
        let mut found = Vec::new();
        for &[x, y] in tiles {
            for (id, _) in self.spatial.objects_on(x, y) {
                let Some((owner, unit)) = self.object(id) else {
                    continue;
                };
                let master = self.master_of(owner, unit.life.master_id)?;
                if master.auto_gather {
                    found.push((id, master.gather_group, unit.position));
                }
            }
        }
        if let Some(r) = self
            .spatial
            .sight
            .as_mut()
            .and_then(|v| v.resources.get_mut(p))
        {
            for (id, group, [x, y, _]) in found {
                r.add(id, group, x, y);
            }
        }
        Ok(())
    }

    /// `WorldPlayer::make_new_object`'s visible resources (`FUN_14162ce00`, player slot 0x1c0,
    /// after the master made the object, the last of player `p`'s list): an object of a master
    /// that gathers by itself (`+0x681`) joins, at its place, the lists of its group (`+0x683`)
    /// of the players, in order, that see or explored its tile (`FUN_141663500` on its x and y
    /// truncated, [`Spatial::explored`]); the world's state (`+0x88`) is 0 in a game.
    ///
    /// # Errors
    ///
    /// A master the slice does not have, or no players' sight.
    pub(crate) fn made_new_object(&mut self, p: usize) -> Result<(), Unsupported> {
        let Some(u) = self.players[p].objects.last() else {
            return Ok(());
        };
        let (id, [x, y, _]) = (u.id, u.position);
        let master = self.master_of(p, u.life.master_id)?;
        if !master.auto_gather {
            return Ok(());
        }
        let group = master.gather_group;
        let (tx, ty) = (x as i32, y as i32);
        let mut explored = Vec::new();
        for q in 0..self.players.len() {
            explored.push(self.spatial.explored(q, tx, ty)?);
        }
        if let Some(v) = self.spatial.sight.as_mut() {
            for (q, _) in explored.iter().enumerate().filter(|&(_, &e)| e) {
                if let Some(r) = v.resources.get_mut(q) {
                    r.add(id, group, x, y);
                }
            }
        }
        Ok(())
    }

    /// `World::load_world`'s planners (`FUN_140f7d310`): the restrictions of Gaia's masters 83
    /// (the villager's) and 535 (the galley's), each unless one of the same values has one; any
    /// other restriction finds none (its units walk without the tile pathfinder).
    pub fn register_planners(&mut self) {
        for master in PLANNED_MASTERS {
            if let Some(r) = self
                .masters
                .first()
                .and_then(|m| m.get(master))
                .and_then(|m| usize::try_from(m.terrain_restriction).ok())
            {
                self.pathing.register(r);
            }
        }
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

    /// An update's start up to the end of its command phase (`BaseWorld::update` up to
    /// `RGE_Command__do_commands`, before `FUN_141552990`): the update's count and the world's
    /// time go on by `dt`, the streams take the frame seed and `commands` run; no player updates.
    /// For a check of the commands against the game's state there (the tick oracle's
    /// `snapshot_at` at `0x141552990`); [`Turns::tick_with`] runs it first.
    pub fn command_phase(&mut self, dt: u32, commands: &[Command]) {
        self.ticks += 1;
        self.world_time = self.world_time.wrapping_add(dt);
        self.dt_seconds = seconds(dt);
        self.streams.seed_all(self.frame_seed);
        self.sync_moving();
        for command in commands {
            tracing::debug!(target: crate::trace::UPDATE, ?command, "command");
            self.run(command);
        }
        self.sync_moving();
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
            self.spatial.task_0x99 = self
                .masters
                .iter()
                .map(|m| {
                    m.iter()
                        .filter(|m| m.tasks.iter().any(|t| t.action_type == 0x99))
                        .map(|m| m.id)
                        .collect()
                })
                .collect();
        }
    }

    /// One world update of `dt` game milliseconds, with the commands recorded in it.
    ///
    /// # Panics
    ///
    /// Never: each player updates in turn, so the one it splits off exists.
    pub fn tick_with(&mut self, dt: u32, commands: &[Command]) {
        let _update = tracing::debug_span!(
            target: crate::trace::UPDATE,
            "update",
            tick = self.ticks + 1,
            world_time = self.world_time.wrapping_add(dt)
        )
        .entered();
        self.command_phase(dt, commands);
        // which players' gates are open (BaseWorld::update's flags)
        let mut gates_open = vec![false; self.players.len()];
        // the Empire Wars start's pass, once (the world's +0x6c6 set before the players update)
        let start_work = std::mem::take(&mut self.start_work_due);
        for p in 0..self.players.len() {
            let _player =
                tracing::debug_span!(target: crate::trace::UPDATE, "player", player = p).entered();
            self.player_time[p] = self.player_time[p].wrapping_add(dt);
            let turn = (p == self.player_turn).then_some(self.player_time[p]);
            if let Err(e) = self.open_gates(p, &mut gates_open) {
                self.stop_player(p, e);
            }
            if start_work && let Err(e) = self.start_work(p) {
                self.stop_player(p, e);
            }
            self.pathing.tiles_changed(self.spatial.take_changes());
            // the player's groups (BaseWorld::update at 0x141551796), then its own update (slot
            // 0x100 at 0x1415518f0, WorldPlayerBase::update): its plans start over (`+0x67c`),
            // then its objects update (FUN_141166550 on its list)
            if let Err(e) = self.update_groups(p) {
                self.stop_player(p, e);
            }
            self.pathing.reset_plans(p);
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
            // a unit whose update stopped before its unit AI for its work order runs that part
            // again once the work is given ([`Unit::ai_part`])
            let mut resume: Option<AiResume> = None;
            while i < self.players[p].objects.len() {
                let player = &self.players[p];
                let stopped = player.stopped || player.objects[i].stopped;
                if stopped && player.objects[i].life.is_alive() {
                    self.spatial.freeze(player.objects[i].id);
                    resume = None;
                    i += 1;
                    continue;
                }
                let mut unit = std::mem::take(&mut self.players[p].objects[i]);
                let _object = tracing::trace_span!(
                    target: crate::trace::UPDATE,
                    "object",
                    id = unit.id,
                    master = unit.life.master_id
                )
                .entered();
                let watch = crate::trace::Watch::of(&unit);
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
                    groups: &self.groups,
                    construction_rate: player.construction_rate,
                    masters: self.masters.get(p),
                    victim: victim.as_mut(),
                    ai_target,
                    blasted,
                    own_killed: None,
                    remains: None,
                    refund: Vec::new(),
                    building_death: None,
                    pauses: Vec::new(),
                    releases: Vec::new(),
                    takes: Vec::new(),
                    made: Vec::new(),
                    ghost_deaths: Vec::new(),
                    started: Vec::new(),
                    unwatched: Vec::new(),
                    annex_states: Vec::new(),
                    cleared: Vec::new(),
                    evacuations: Vec::new(),
                    recycling: &mut self.recycling,
                    mode_1: self.mode_680 == 1,
                    player: p,
                    civ: player.civ,
                    sharing: player.sharing,
                    farm_stops: self.farm_stops,
                    predator_hunts_end: self.predator_hunts_end,
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
                    tracker: &mut player.tracker,
                    lines: &self.lines,
                    attribute_adds: Vec::new(),
                    techs_done: Vec::new(),
                    carts: Vec::new(),
                    completions: Vec::new(),
                    work_order: None,
                    gate_toggle: false,
                    ai_resume: resume.take(),
                    order_on_target: None,
                    hunt_objects: Vec::new(),
                    gather_objects: Vec::new(),
                    build_object: None,
                    build_command: None,
                    move_aside: Vec::new(),
                    waits: Vec::new(),
                    main_works: Vec::new(),
                    relations: &player.relations,
                    leads: None,
                    zones: &mut self.zones,
                    foreign: Foreign { before, after },
                };
                let mut result = unit.update(&mut c, &mut player.objects);
                if let Some(w) = watch {
                    w.report(&unit, c.tick, p);
                }
                let (adds, ordered) = (
                    std::mem::take(&mut c.attribute_adds),
                    c.order_on_target.take(),
                );
                let hunt_objects = std::mem::take(&mut c.hunt_objects);
                let gather_objects = std::mem::take(&mut c.gather_objects);
                let leads = c.leads.take();
                let build_object = c.build_object.take();
                let build_command = c.build_command.take();
                let move_aside = std::mem::take(&mut c.move_aside);
                let waits = std::mem::take(&mut c.waits);
                let techs_done = std::mem::take(&mut c.techs_done);
                let carts = std::mem::take(&mut c.carts);
                let completions = std::mem::take(&mut c.completions);
                let work_order = c.work_order.take();
                let (blasted, own_killed) = (std::mem::take(&mut c.blasted), c.own_killed.take());
                let (remains, refund) = (c.remains.take(), std::mem::take(&mut c.refund));
                let (annex_states, cleared) = (
                    std::mem::take(&mut c.annex_states),
                    std::mem::take(&mut c.cleared),
                );
                let main_works = std::mem::take(&mut c.main_works);
                let building_death = c.building_death.take();
                let pauses = std::mem::take(&mut c.pauses);
                let gate_toggle = std::mem::take(&mut c.gate_toggle);
                let ai_resume = c.ai_resume.take();
                let releases = std::mem::take(&mut c.releases);
                let (takes, made) = (std::mem::take(&mut c.takes), std::mem::take(&mut c.made));
                let (ghost_deaths, started, unwatched, evacuations) = (
                    std::mem::take(&mut c.ghost_deaths),
                    std::mem::take(&mut c.started),
                    std::mem::take(&mut c.unwatched),
                    std::mem::take(&mut c.evacuations),
                );
                if let Some(e) = self.spatial.take_sight_unsupported() {
                    result = result.and(Err(e));
                }
                self.spatial.set_gait(unit.id, unit.gait());
                player.objects[i] = unit;
                // each made object's add to its player (slot 0x2d0) filed it in the tracker
                for m in &made {
                    let k = crate::tracker::key(self.masters.get(p), &self.lines, m.life.master_id);
                    player.tracker.add(m.id, k);
                }
                player.objects.extend(made);
                // the gate's player opens or shuts it, in the gate's update before the rest
                if gate_toggle && let Err(e) = self.toggle_gate(p, i) {
                    result = result.and(Err(e));
                }
                // the work a builder gave an annex goes to its main building (FUN_1411cc430)
                for (annex, amount) in main_works {
                    if let Err(e) = self.work_building(annex, amount) {
                        result = result.and(Err(e));
                    }
                }
                // a building's set_state took its annexes along first (FUN_1411dbb70)
                for (annex, state) in annex_states {
                    if let Some((q, _)) = self.object(annex)
                        && let Err(e) = self.set_state(q, annex, state)
                    {
                        result = result.and(Err(e));
                    }
                }
                // the objects a builder's footprint clearing deleted (FUN_1415c3620)
                for cl in cleared {
                    if let Err(e) = self.clear_object(cl, true) {
                        result = result.and(Err(e));
                    }
                }
                for (kind, amount) in adds {
                    if let Err(e) = self.players[p].add_attribute(kind, amount) {
                        result = result.and(Err(e));
                    }
                }
                // a completed mule cart's search for resources (FUN_1411c6bd0's tail, before
                // the completion's technology)
                for (cart, state) in carts {
                    if let Err(e) = self.cart_search(p, cart, state) {
                        result = result.and(Err(e));
                    }
                }
                // a completed building's stack unit before its technology (FUN_1411dbb70)
                for &(building, stack, _) in &completions {
                    if let Some(stack) = stack
                        && let Err(e) = self.stack_unit(p, building, stack)
                    {
                        result = result.and(Err(e));
                    }
                }
                for t in techs_done {
                    if let Err(e) = self.research_built(p, t) {
                        result = result.and(Err(e));
                    }
                }
                // then a building that disappears when built is destroyed (slot 0xd0)
                for (building, _, vanishes) in completions {
                    if vanishes && let Err(e) = self.destroy(p, building) {
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
                for (foundation, units) in evacuations {
                    if let Err(e) = self.evacuate(foundation, &units) {
                        result = result.and(Err(e));
                    }
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
                // a dead building's count leaves its attribute, at least 0 (FUN_1411dbb70)
                if let Some(BuildingDeath {
                    count: Some(kind), ..
                }) = building_death
                {
                    let owner = &mut self.players[p];
                    if let Err(e) = owner.add_attribute(kind, -1.0) {
                        result = result.and(Err(e));
                    } else if owner.attributes[kind as usize] < 0.0 {
                        owner.attributes[kind as usize] = 0.0;
                    }
                }
                // a dead unit's paid cost leaves attributes 98 and 164 (a building's 98 and 165),
                // each at least 0 (FUN_14121cc40: slots 0xf8, then 0xf0 with 0)
                for (kind, cost) in refund {
                    let owner = &mut self.players[p];
                    if let Err(e) = owner.add_attribute(kind, -cost) {
                        result = result.and(Err(e));
                    } else if owner.attributes[kind as usize] < 0.0 {
                        owner.attributes[kind as usize] = 0.0;
                    }
                }
                // the last building of a master with a technology withdraws it (FUN_141166ab0,
                // FUN_1413d4ec0), after the combat part's state 3
                if let Some(BuildingDeath {
                    tech: Some((master, tech)),
                    ..
                }) = building_death
                    && let Err(e) = self.withdraw_tech(p, master, tech)
                {
                    result = result.and(Err(e));
                }
                // the remains of a dying sprite's end, a new object of the owner (FUN_1411a4e90)
                if let Some((master, at, facet)) = remains
                    && let Err(e) = self.remains(p, master, at, facet)
                {
                    result = result.and(Err(e));
                }
                // TRIBE_Building_Object::update: the production after the combat update (once,
                // with the update's unit AI part)
                if matches!(result, Ok(Step::Stay))
                    && ai_resume.is_none()
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
                // the unit AI's HuntObjects: each the unit's stop, then its hunt command
                // a hunt's set_state(6) on its own livestock: the animal's slot 0x138 to the drop
                // site and the herd around it (FUN_140ac4c60)
                if let Some((sheep, site)) = leads
                    && let Err(e) = self.lead_herd(p, sheep, site)
                {
                    result = Err(e);
                }
                for target in hunt_objects {
                    if !matches!(result, Ok(Step::Stay)) {
                        result = result.and(Err(Unsupported(
                            "a HuntObject of a unit that leaves its list",
                        )));
                        break;
                    }
                    if let Err(e) = self.hunt_command(p, i, target) {
                        result = Err(e);
                    }
                }
                // the unit AI's GatherObjects: each the unit's gather command
                for target in gather_objects {
                    if !matches!(result, Ok(Step::Stay)) {
                        result = result.and(Err(Unsupported(
                            "a GatherObject of a unit that leaves its list",
                        )));
                        break;
                    }
                    if let Err(e) = self.gather_command(p, i, target) {
                        result = Err(e);
                    }
                }
                // the unit AI's moves off footprints (slot 0x148)
                for m in move_aside {
                    if !matches!(result, Ok(Step::Stay)) {
                        result = result.and(Err(Unsupported(
                            "a move off a footprint of a unit that leaves its list",
                        )));
                    } else if let Err(e) = self.move_aside(p, i, m) {
                        result = Err(e);
                    }
                }
                // the waits for foundations the unit AI's orders found (slot 0x388)
                for w in waits {
                    // a unit that ran its orders stays in its list: the full update runs them
                    // only for a unit alive and not inside another, between turns only for a
                    // unit of a type below 70 or outside, and nothing after the queue in the
                    // unit's update kills, captures or houses it
                    debug_assert!(matches!(result, Ok(Step::Stay)) || result.is_err());
                    if !matches!(result, Ok(Step::Stay)) {
                        break;
                    }
                    if let Err(e) = self.wait_for_foundation(p, i, w) {
                        result = Err(e);
                    }
                }
                // a Gather's build command on its target not built yet (slot 0x380)
                if let Some(foundation) = build_command {
                    if !matches!(result, Ok(Step::Stay)) {
                        result = result.and(Err(Unsupported(
                            "a build command of a unit that leaves its list",
                        )));
                    } else if let Err(e) = self.build_command(p, i, foundation) {
                        result = Err(e);
                    }
                }
                // the unit AI's BuildObject on a foundation its builder's search found
                if let Some(foundation) = build_object {
                    if !matches!(result, Ok(Step::Stay)) {
                        result = result.and(Err(Unsupported(
                            "a BuildObject of a unit that leaves its list",
                        )));
                    } else if let Err(e) = self.build_object(p, i, foundation) {
                        result = Err(e);
                    }
                }
                if let (Ok(Step::Stay), Some((order, tail))) = (&result, ordered) {
                    let ran = self
                        .order_on_target(p, i, order)
                        .and_then(|()| tail.map_or(Ok(()), |after| self.after_order(p, i, after)));
                    if let Err(e) = ran {
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
                // a construction start's clearing of the desolid foundations it overlaps
                // (FUN_1411cd0d0), after the builder's update
                for id in self.spatial.take_started() {
                    if let Err(e) = self.clear_foundations_under(id) {
                        result = result.and(Err(e));
                    }
                }
                self.pathing.tiles_changed(self.spatial.take_changes());
                if matches!(result, Ok(Step::Stay)) && ai_resume.is_some() {
                    resume = ai_resume;
                    continue;
                }
                let player = &mut self.players[p];
                match result {
                    Ok(Step::Leave) => {
                        let gone = player.objects.swap_remove(i);
                        // its deletion (FUN_14116b5e0: its player's remove, slot 0x2d8, takes it
                        // out of the tracker), and a combat object's destructor's class counters
                        // (FUN_141200450)
                        let k = crate::tracker::key(
                            self.masters.get(p),
                            &self.lines,
                            gone.life.master_id,
                        );
                        player.tracker.remove(gone.id, k);
                        let master = self.masters.get(p).and_then(|m| m.get(gone.life.master_id));
                        forget_resource(&mut self.spatial, master, gone.id, gone.position);
                        if let Some(m) = master.filter(|m| 70 <= m.kind) {
                            count_class(&mut player.attributes, m, -1.0);
                        }
                        if let Err(e) = self.spatial.leave(gone.id) {
                            crate::trace::player_unsupported(self.ticks, p, e);
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
                        // that object's update waits for the next tick; the player's remove and
                        // add (slots 0x2d8, 0x2d0) file it last in the tracker
                        let asleep = player.objects.swap_remove(i);
                        let k = crate::tracker::key(
                            self.masters.get(p),
                            &self.lines,
                            asleep.life.master_id,
                        );
                        player.tracker.remove(asleep.id, k);
                        player.tracker.add(asleep.id, k);
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
                            crate::trace::player_unsupported(self.ticks, p, e);
                            let _ = self.players[p]
                                .unsupported
                                .get_or_insert((self.ticks, i, e));
                        }
                        // each one's slot 0x70: the old owner's remove (slot 0x2d8), the new
                        // owner's add (slot 0x2d0), each tracker by its own masters
                        for u in &moved {
                            let m = u.life.master_id;
                            let old = crate::tracker::key(self.masters.get(p), &self.lines, m);
                            self.players[p].tracker.remove(u.id, old);
                            let k = crate::tracker::key(self.masters.get(new), &self.lines, m);
                            self.players[new].tracker.add(u.id, k);
                        }
                        self.players[new].objects.extend(moved);
                        i += 1;
                    }
                    Ok(Step::Stay) => i += 1,
                    Err(e) => {
                        crate::trace::object_freezes(self.ticks, p, player.objects[i].id, e);
                        player.freezes.push((self.ticks, player.objects[i].id, e));
                        let _ = player.unsupported.get_or_insert((self.ticks, i, e));
                        player.objects[i].stopped = true;
                        self.spatial.freeze(player.objects[i].id);
                        i += 1;
                    }
                }
            }
            // the end of the player's own update: the tiles its map explored give their
            // resources
            if let Err(e) = self.process_new_tiles(p) {
                self.stop_player(p, e);
            }
            self.close_gates(p, &mut gates_open);
            // its tile list empties (FUN_141661030)
            if let Some(v) = self.spatial.sight.as_mut() {
                v.clear_new_tiles(p);
            }
        }
        self.spatial.grow_unknown(seconds(dt));
        let s1 = self.streams.get(Stream::World).rand15();
        // every player's ghosts (FUN_141166550 on +0x40), then every player's watched list
        // (FUN_1410905e0); the objects' visibility masks (FUN_14107d280 on +0x80, before the
        // draw) are the registry's, kept as the objects update
        for p in 0..self.players.len() {
            if let Err(e) = self.update_ghosts(p) {
                self.stop_player(p, e);
            }
        }
        for p in 0..self.players.len() {
            if let Err(e) = self.watch_pass(p) {
                self.stop_player(p, e);
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

/// The owner's attributes 237, 238 and 239: a built farm's bonus ([`Completion::farm_bonus`]);
/// −1 for those the player has not got.
pub(crate) fn farm_bonus(attributes: &[f32]) -> [f32; 3] {
    let at = |i: usize| attributes.get(i).copied().unwrap_or(-1.0);
    [at(237), at(238), at(239)]
}

/// `FUN_1411980b0`'s visible resources, from `set_state` leaving a state below 4 for one above 6
/// (`FUN_14119d710`) and from the object's deletion: an object of id 0 and up whose master
/// gathers by itself (`+0x681`) leaves the lists of the players that explored its tile
/// ([`Visibility::forget_resource`]); the world's state (`+0x88`) is 0 in a game. The deletion's
/// call is the destructors' (`FUN_14116b5e0`). The routine's other parts are the callers': a
/// master of `create_doppelganger_on_death` 2 leaves its owner's watched list
/// (`FUN_141091310`).
///
/// [`Visibility::forget_resource`]: crate::visibility::Visibility::forget_resource
pub(crate) fn forget_resource(
    spatial: &mut Spatial,
    master: Option<&Master>,
    id: i32,
    position: [f32; 3],
) {
    if id < 0 {
        return;
    }
    if let (Some(m), Some(v)) = (master.filter(|m| m.auto_gather), spatial.sight.as_mut()) {
        v.forget_resource(id, position, m.gather_group);
    }
}

/// The combat object's death part (`FUN_14121cc40`'s block for a state of 3 or more from below
/// 3), of a unit of type 70 and of a building: its pasture lets it go (`FUN_141189ca0` on its
/// `+0x794`), its hit points go to 0, and unless its record went already (`mROOAlreadyCalled`,
/// `+0x686`): a unit's, or a building's not desolid whose master is tall (`FUN_1411c3f70`:
/// COMISS, a NaN height is not), mesh shape and obstruction record go (`FUN_141055b40`); one of
/// type 70 and up has no height after (its own master copy, `FUN_141186d10`: no longer tall); a
/// building on a tile that is no annex lets its tiles go (`FUN_140f65580`); then `+0x686` is
/// set. Not modelled: a unit in a pasture, the artifact class (1, `FUN_141218020`), class 43
/// (Gaia's object 0x11d at its place) and class 13's units inside. Not held: the damage sprites
/// (`FUN_1413b9e20`) and the hero attachments (`FUN_141201930`).
///
/// # Errors
///
/// What it does not model.
pub(crate) fn combat_dies(
    u: &mut Unit,
    master: &Master,
    spatial: &mut Spatial,
    pathing: &mut Pathing,
) -> Result<(), Unsupported> {
    if u.pasture != -1 || u.my_pasture != -1 {
        return Err(Unsupported("a dying unit in a pasture (FUN_141189ca0)"));
    }
    if matches!(master.class, 1 | 13 | 43) {
        return Err(Unsupported("a dying unit of class 1, 13 or 43"));
    }
    u.life.hp = 0.0;
    if u.roo_called {
        return Ok(());
    }
    let desolid = u.construction.is_some_and(|c| c.desolid);
    if master.kind < 80 || (!desolid && 0.0 < master.height) {
        pathing.removed(u.id);
        spatial.drop_record(u.id);
    }
    if 0x45 < master.kind {
        spatial.set_tall(u.id, false);
    }
    if master.kind == 80
        && u.annex_of.is_none()
        && u.life.inside.is_none()
        && let Some(v) = spatial.sight.as_mut()
    {
        v.unlink_building(u.position, master.radius);
    }
    u.roo_called = true;
    Ok(())
}

/// What the combat object's `set_state` to 3 from state 2 (`FUN_14121cc40`) takes from its
/// owner's attributes: its paid cost (the cost entries of a resource flagged paid, each as a
/// float, summed in order) from 98 and 164 for a unit; from 98 and 165 for a building (98 alone
/// for the castle, 82, and the wonder, 276), nothing for an annex (`+0xbb8`). Each attribute is
/// held at 0 or more after (slot 0xf0 with 0), the caller's.
#[must_use]
pub(crate) fn refund_of(master: &Master, main: Option<i32>) -> Vec<(i16, f32)> {
    let mut cost = 0.0f32;
    for &(resource, amount, paid) in &master.cost {
        if 0 <= resource && paid {
            cost += f32::from(amount);
        }
    }
    if master.kind != 80 {
        return vec![(98, cost), (164, cost)];
    }
    if main.is_some() {
        return Vec::new();
    }
    if matches!(master.id, CASTLE | WONDER) {
        return vec![(98, cost)];
    }
    vec![(98, cost), (165, cost)]
}

/// What a built building's death (`FUN_1411dbb70` to state 3 from 2) does before the combat
/// object's part, for its caller to apply: the attributes its storage gave its owner
/// ([`storage_leaves`]), the place whose tiles it lets go (`FUN_140f65580`, a building on a tile
/// that is no annex), the count it lowers and its master's technology ([`BuildingDeath`]); its
/// owner's watched list lets it go (`FUN_141091310`, the caller's).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BuiltDeath {
    pub(crate) adds: Vec<(i16, f32)>,
    pub(crate) unlink: Option<[f32; 3]>,
    pub(crate) building: BuildingDeath,
}

/// [`BuiltDeath`] of the building `u` of `master`: the monastery (104), the wonder (276) and the
/// castle (82) count one less in attributes 52, 42 and 134 (slot 0xf8 with −1, then slot 0xf0
/// with 0 below 0); its master's technology (`+0x98c`) goes when no other building of the master
/// stands ([`Turns::withdraw_tech`]). A foundation terrain stays. Not modelled: a computer
/// player's (`FUN_1411e7ca0`), one with a pasture or its workers (`+0xcb8`), units inside, the
/// terrain 1 a foundation terrain of 28 lays at the death (`FUN_1411ce250`). Not held: its
/// linked objects (`+0xb78`, none held), `+0xca4` and the snow decal (`FUN_1413b9e20`).
///
/// # Errors
///
/// What it does not model.
pub(crate) fn built_death(
    u: &mut Unit,
    master: &Master,
    computer: bool,
) -> Result<BuiltDeath, Unsupported> {
    let building = master.building.ok_or(Unsupported(
        "a dying building of a master without its building part",
    ))?;
    if computer || u.pasture != -1 || u.my_pasture != -1 || !u.garrison.is_empty() {
        return Err(Unsupported(
            "a dying building of a computer player, with a pasture or units inside",
        ));
    }
    let adds = storage_leaves(u, master);
    if building.foundation_terrain == RESTORED_FOUNDATION {
        return Err(Unsupported(
            "a dying building of foundation terrain 28 (its terrain 1, FUN_1411ce250)",
        ));
    }
    let unlink = (u.annex_of.is_none() && u.life.inside.is_none()).then_some(u.position);
    let count = match master.id {
        MONASTERY => Some(MONASTERIES),
        WONDER => Some(WONDERS),
        CASTLE => Some(CASTLES),
        _ => None,
    };
    let tech = (0 <= building.tech).then_some((master.id, building.tech));
    Ok(BuiltDeath {
        adds,
        unlink,
        building: BuildingDeath { count, tech },
    })
}

/// What a built building's storage gave its owner leaves (`FUN_1411e63f0`, then
/// `FUN_1411e6580`): each entry of mode 4 with an attribute and an amount above 0 (the
/// population headroom of a house, a town centre or a castle; slot 0xf8 with the amount
/// negated), then the population it holds itself (`+0x308` 4: the amount leaves, and it holds
/// nothing after, `FUN_14119d310` with −1 and 0); then each entry of mode 16 leaves its own store
/// ([`Unit::store`]) when the store holds its attribute.
pub(crate) fn storage_leaves(u: &mut Unit, master: &Master) -> Vec<(i16, f32)> {
    let mut adds = Vec::new();
    for &(kind, amount, flag) in &master.storage {
        if 0 <= kind && 0.0 < amount && flag == 4 {
            adds.push((kind, -amount));
        }
    }
    if u.life.attribute_type == 4 {
        adds.push((4, -u.life.amount));
        u.life.attribute_type = -1;
        u.life.amount = 0.0;
    }
    // FUN_1411e6580: each entry of mode 16 leaves its own store, when the store holds it
    for &(kind, amount, flag) in &master.storage {
        if 0 <= kind
            && 0.0 < amount
            && flag & 0x10 != 0
            && let Some(e) = u.store.iter_mut().find(|e| e.0 == kind)
        {
            e.1 -= amount;
        }
    }
    adds
}

/// What the footprint clearing reads of the object `u` ([`crate::build::ClearLook`]) with its
/// owner's `masters`; None without its master.
fn clear_look_of(u: &Unit, masters: Option<&Masters>) -> Option<crate::build::ClearLook> {
    let m = masters?.get(u.life.master_id)?;
    Some(crate::build::ClearLook {
        hp: u.life.hp,
        height: m.height,
        roo_called: u.roo_called,
        kind: m.kind,
        state: u.life.state,
        desolid: u.construction.is_some_and(|c| c.desolid),
    })
}

/// The classes a combat object of which counts in its player's attribute 40
/// (`FUN_141226060`).
const MILITARY_CLASSES: [i16; 24] = [
    0, 6, 12, 13, 18, 22, 23, 24, 26, 28, 35, 36, 43, 44, 45, 46, 47, 50, 51, 53, 54, 55, 56, 57,
];
/// The masters whose objects count in attribute 235 besides 40 (`FUN_141226060`).
const MILITARY_235: [i16; 2] = [1663, 1697];
/// The classes that count in attribute 37 (`FUN_141226060`); the villagers' (4) too.
const CIVILIAN_CLASSES: [i16; 4] = [2, 19, 20, 21];

/// `FUN_141226060`: a combat object (type 70 and up) of `master` counts `by` (1 or -1) in its
/// player's `attributes` by its class: a military class (0, 6, 12, 13, 18, 22 to 24, 26, 28, 35,
/// 36, 43 to 47, 50, 51, 53 to 57) in attribute 40, masters 1663 and 1697 also in 235; a
/// villager (class 4) in 38 when its master's task swap group (`+0x7b8`) is 1 or 2, and in 37
/// as the classes 2, 19, 20 and 21 do. The attributes take it as they are (no player slot).
/// Its callers: the combat setup (`FUN_14121df60`, 1), the destructors (`FUN_141200450`,
/// `FUN_1411ec020`, -1), the ownership change (`FUN_14120f800`, -1 then 1 for the new owner) and
/// the master change (`FUN_141186580` through `FUN_141209d90`, -1 then 1 with the new master).
pub(crate) fn count_class(attributes: &mut [f32], master: &Master, by: f32) {
    let mut add = |i: usize| {
        if let Some(v) = attributes.get_mut(i) {
            *v += by;
        }
    };
    if MILITARY_CLASSES.contains(&master.class) {
        add(40);
        if MILITARY_235.contains(&master.id) {
            add(235);
        }
        return;
    }
    if master.class == crate::spawn::VILLAGER && matches!(master.task_by_group, 1 | 2) {
        add(38);
    }
    if master.class == crate::spawn::VILLAGER || CIVILIAN_CLASSES.contains(&master.class) {
        add(37);
    }
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
        workers: u.workers,
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
            // a villager moves (its master's slot 0xe8), which BUILD's last loop asks
            moving: Some(crate::movement::MoveStats {
                fly_mode: 0,
                speed: 0.8,
                turn_rate: f32::MAX,
                turn_radius: 0.0,
                turn_radius_speed: f32::MAX,
                radius: [0.2, 0.2, 0.5],
                restriction: 7,
                restriction_mask: 6,
                turn_time: 0.0,
                move_algorithm: 0,
                min_collision: 1.0,
                fast_turn: false,
                walk_speed: 1.0,
                stand_speed: 1.0,
                run_speed: 1.0,
                facets: None,
                weapon_range: 0.0,
            }),
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
                // built by the villagers (the builders' test, FUN_141262270)
                trained_at: vec![118],
                // a house's building part: no annexes, no stack unit, not an annex
                building: Some(crate::masters::BuildingMaster {
                    stack_unit: -1,
                    foundation_terrain: -1,
                    tech: -1,
                    annexes: [-1; 4],
                    head_unit: -1,
                    transform_unit: -1,
                    ..crate::masters::BuildingMaster::default()
                }),
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
        terrain.set_elevations(vec![0; n]);
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

    /// `Process_New_Tiles`: player 1's newly explored tiles, in order, give the objects of its
    /// masters that gather by themselves to its lists by group, at their places.
    #[test]
    fn a_player_s_new_tiles_give_their_resources_to_its_lists() {
        use crate::masters::{Master, Masters};
        use crate::visibility::{VisibleResource, VisibleResources};
        let object = |id, master, x, y| Unit {
            id,
            position: [x, y, 0.0],
            life: Lifecycle {
                class: Class::Static,
                state: 2,
                master_id: master,
                ..Lifecycle::default()
            },
            ..Unit::default()
        };
        let mut t = Turns::new(
            0,
            1,
            1,
            vec![0, 0],
            vec![
                Player {
                    sleeping: vec![
                        object(501, 411, 10.5, 10.25),
                        object(502, 412, 10.5, 10.5),
                        object(503, 413, 11.5, 10.75),
                    ],
                    ..Player::default()
                },
                Player::default(),
            ],
        );
        let master = |id, auto_gather, gather_group| Master {
            id,
            kind: 10,
            auto_gather,
            gather_group,
            ..Master::default()
        };
        t.masters = vec![
            Masters::new([
                master(411, true, 0),
                master(412, false, 0),
                master(413, true, 4),
            ]),
            Masters::default(),
        ];
        t.spatial = Spatial::with_tiles(
            [120, 120],
            crate::spatial::Tiles::new(120, 120, vec![0; 120 * 120]),
        );
        placeable(&mut t);
        for u in &t.players[0].sleeping {
            t.spatial.insert(
                u.id,
                Placed {
                    position: u.position,
                    radius: [0.5, 0.5],
                    class: 15,
                    player: 0,
                    master: u.life.master_id,
                    obstruction_type: 0,
                    buildable_on: 0,
                    tall: false,
                    speed: 0.0,
                    unknown: None,
                    obstruction: None,
                },
            );
        }
        let v = t.spatial.sight.as_mut().unwrap();
        v.resources[1] = VisibleResources {
            capacities: vec![32; 5],
            lists: vec![Vec::new(); 5],
        };
        v.new_tiles[1] = vec![[11, 10], [10, 10]];
        t.process_new_tiles(1).unwrap();
        let rec = |id, x, y| VisibleResource {
            id,
            x,
            y,
            ..VisibleResource::default()
        };
        let v = t.spatial.sight.as_ref().unwrap();
        assert_eq!(v.resources[1].lists[0], [rec(501, 10.5, 10.25)]);
        assert_eq!(v.resources[1].lists[4], [rec(503, 11.5, 10.75)]);
        assert_eq!(v.resources[0], VisibleResources::default());
    }

    /// `FUN_1411980b0`: a static resource that runs out (state 3 to 7) leaves the players'
    /// visible-resource lists at once; an animated one (state 5 to 7, not below 4) only when it
    /// is deleted (state 8 leaving its list, `FUN_14116b5e0`).
    #[test]
    fn a_resource_leaves_the_lists_when_it_runs_out_or_is_deleted() {
        use crate::lifecycle::MasterData;
        use crate::masters::{Master, Masters};
        use crate::visibility::VisibleResources;
        let dead = |id, class, state, master| Unit {
            id,
            position: [10.5, 10.5, 0.0],
            life: Lifecycle {
                class,
                state,
                master_id: master,
                attribute_type: -1,
                master: Some(MasterData {
                    decay: 0.0,
                    undead: false,
                }),
                ..Lifecycle::default()
            },
            ..Unit::default()
        };
        let mut t = Turns::new(
            0,
            1,
            1,
            vec![0, 0],
            vec![
                Player {
                    objects: vec![
                        dead(501, Class::Static, 3, 411),
                        dead(502, Class::Animated, 5, 412),
                    ],
                    ..Player::default()
                },
                Player::default(),
            ],
        );
        let master = |id, gather_group| Master {
            id,
            kind: 10,
            auto_gather: true,
            gather_group,
            ..Master::default()
        };
        t.masters = vec![
            Masters::new([master(411, 0), master(412, 1)]),
            Masters::default(),
        ];
        placeable(&mut t);
        let v = t.spatial.sight.as_mut().unwrap();
        for p in 0..2 {
            v.resources[p] = VisibleResources {
                capacities: vec![32; 5],
                lists: vec![Vec::new(); 5],
            };
            v.resources[p].add(501, 0, 10.5, 10.5);
            v.resources[p].add(502, 1, 10.5, 10.5);
        }
        let held = |t: &Turns| -> Vec<(usize, usize)> {
            let v = t.spatial.sight.as_ref().unwrap();
            (0..2)
                .map(|p| (v.resources[p].lists[0].len(), v.resources[p].lists[1].len()))
                .collect()
        };
        t.tick(10);
        assert_eq!(t.players[0].objects[0].life.state, 7);
        assert_eq!(held(&t), [(0, 1), (0, 1)]);
        t.tick(10);
        assert_eq!(held(&t), [(0, 1), (0, 1)]);
        t.tick(10);
        assert_eq!(t.players[0].objects, []);
        assert_eq!(held(&t), [(0, 0), (0, 0)]);
    }

    /// `FUN_14162ce00`: a new object of a master that gathers by itself joins the lists of the
    /// players that explored its tile; one of another master joins none.
    #[test]
    fn a_new_resource_joins_the_lists_of_the_players_that_explored_its_tile() {
        use crate::masters::{Master, Masters};
        use crate::visibility::{VisibleResource, VisibleResources};
        let made = |id, master| Unit {
            id,
            position: [20.75, 30.25, 0.0],
            life: Lifecycle {
                state: 2,
                master_id: master,
                ..Lifecycle::default()
            },
            ..Unit::default()
        };
        let mut t = Turns::new(
            0,
            1,
            1,
            vec![0, 0, 0],
            vec![Player::default(), Player::default(), Player::default()],
        );
        let master = |id, auto_gather| Master {
            id,
            kind: 30,
            auto_gather,
            gather_group: 2,
            ..Master::default()
        };
        t.masters = vec![Masters::new([master(53, true), master(54, false)]); 3];
        placeable(&mut t);
        let v = t.spatial.sight.as_mut().unwrap();
        // players 0 and 2 explored the tile, player 1 did not
        v.unified[30 * 120 + 20] = 1 << 32 | 1 << 34;
        for r in &mut v.resources {
            *r = VisibleResources {
                capacities: vec![32; 5],
                lists: vec![Vec::new(); 5],
            };
        }
        t.players[0].objects.push(made(700, 53));
        t.made_new_object(0).unwrap();
        t.players[0].objects.push(made(701, 54));
        t.made_new_object(0).unwrap();
        let fish = VisibleResource {
            id: 700,
            x: 20.75,
            y: 30.25,
            ..VisibleResource::default()
        };
        let v = t.spatial.sight.as_ref().unwrap();
        let lists: Vec<_> = v.resources.iter().map(|r| r.lists[2].clone()).collect();
        assert_eq!(lists, [vec![fish], vec![], vec![fish]]);
    }

    /// `FUN_141226060`: the class counters by class and master.
    #[test]
    fn a_combat_object_counts_in_its_player_s_class_attributes() {
        use crate::masters::Master;
        let counted = |class, id, task_by_group| {
            let mut a = vec![0.0f32; 240];
            let m = Master {
                id,
                class,
                task_by_group,
                ..Master::default()
            };
            count_class(&mut a, &m, 1.0);
            count_class(&mut a, &m, 1.0);
            [a[37], a[38], a[40], a[235]]
        };
        assert_eq!(counted(6, 4, 0), [0.0, 0.0, 2.0, 0.0]);
        assert_eq!(counted(13, 1663, 0), [0.0, 0.0, 2.0, 2.0]);
        assert_eq!(counted(57, 1697, 0), [0.0, 0.0, 2.0, 2.0]);
        assert_eq!(counted(4, 83, 1), [2.0, 2.0, 0.0, 0.0]);
        assert_eq!(counted(4, 83, 2), [2.0, 2.0, 0.0, 0.0]);
        assert_eq!(counted(4, 83, 3), [2.0, 0.0, 0.0, 0.0]);
        assert_eq!(counted(19, 128, 0), [2.0, 0.0, 0.0, 0.0]);
        assert_eq!(counted(58, 1663, 1), [0.0; 4]);
        assert_eq!(counted(3, 109, 0), [0.0; 4]);
        let mut a = vec![5.0f32; 41];
        count_class(
            &mut a,
            &Master {
                class: 6,
                ..Master::default()
            },
            -1.0,
        );
        assert_eq!(a[40], 4.0);
    }

    /// `0x141071880`'s idle swap: a standing sprite that looped draws on the Sprite stream and,
    /// above 0x3fff, swaps to the other standing sprite at its first frame; a sprite that did not
    /// loop, another sprite or a master without a second standing sprite draws nothing.
    #[test]
    fn an_idle_object_swaps_its_standing_sprite_on_a_high_draw() {
        use crate::sprite::{ActiveSprite, Graphic};
        let g = |id| Graphic {
            id,
            frames: 10,
            facets: 8,
            speed_multiplier: 1.0,
            frame_duration: 0.1,
            replay_delay: 0.0,
            flags: 1,
        };
        let graphics: std::collections::BTreeMap<i16, Graphic> =
            [(100, g(100)), (101, g(101))].into_iter().collect();
        let master = Master {
            standing_sprite: 100,
            standing_sprite_2: Some(101),
            ..Master::default()
        };
        let shown = |id, looped| ActiveSprite {
            frame: 4,
            looped,
            ..ActiveSprite::at_rest(g(id), 4)
        };
        // a seed whose first draw is above 0x3fff, another whose first is not
        let rng = |seed| {
            let mut r = Well512::default();
            r.set_seed(seed);
            r
        };
        let high = (0..100u32)
            .find(|&s| 0x3fff < rng(s).rand15())
            .expect("a high draw");
        let low = (0..100u32)
            .find(|&s| rng(s).rand15() <= 0x3fff)
            .expect("a low draw");
        let mut u = Unit {
            sprite: Some(shown(100, true)),
            ..Unit::default()
        };
        let mut r = rng(high);
        u.idle_swap(&master, &graphics, &mut r);
        assert_eq!(u.idle_variant, 1);
        let s = u.sprite.unwrap();
        assert_eq!((s.graphic.id, s.frame), (101, 0));
        assert_eq!(r.draws, 1 + rng(high).draws);
        // a low draw keeps the sprite
        let mut u = Unit {
            sprite: Some(shown(100, true)),
            ..Unit::default()
        };
        u.idle_swap(&master, &graphics, &mut rng(low));
        assert_eq!((u.idle_variant, u.sprite.unwrap().graphic.id), (0, 100));
        // no loop, another sprite, no second standing sprite: no draw
        for (sprite, m) in [
            (shown(100, false), master.clone()),
            (shown(7, true), master.clone()),
            (
                shown(100, true),
                Master {
                    standing_sprite_2: None,
                    ..master.clone()
                },
            ),
        ] {
            let mut u = Unit {
                sprite: Some(sprite),
                ..Unit::default()
            };
            let mut r = rng(high);
            u.idle_swap(&m, &graphics, &mut r);
            assert_eq!(r.draws, rng(high).draws);
        }
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
                    attributes: vec![0.0; 250],
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
        // the house's paid cost, as a building with a train location, in attribute 247
        assert_eq!(p.attributes[247], 25.0);
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

    /// census v5's 257 stops "a builder of another player or a sleeping one": BUILD's last
    /// loop (`FUN_140e7a400`) takes the packet's units the world holds whatever their player: a
    /// queued BUILD puts the build order into another player's villager's queue, issued by its
    /// own player and naming the BUILD's player as the target's; a unit of a master that does
    /// not move is passed over unless the BUILD is queued.
    #[test]
    fn a_queued_build_queues_its_order_for_another_player_s_villager() {
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
            vec![0, 0, 0],
            vec![
                Player::default(),
                Player {
                    objects: vec![villager(9175, 83, 30.0, 89.0)],
                    resources: [200.0, 100.0, 200.0, 100.0],
                    attributes: vec![0.0; 250],
                    ..Player::default()
                },
                Player {
                    objects: vec![villager(9618, 293, 29.0, 99.0)],
                    attributes: vec![0.0; 250],
                    ..Player::default()
                },
            ],
        );
        t.next_object_id = 10028;
        t.masters = vec![Masters::default(), builder_masters(), builder_masters()];
        placeable(&mut t);
        // the command phase alone (the units' updates would run the orders)
        t.run(&Command::Build {
            player: 1,
            master: 70,
            unit_ai: Ok(None),
            position: [30.0, 88.0, 0.0],
            builders: vec![9618, 9175],
            queued: true,
            commits: false,
        });
        let order = |p: usize| {
            t.players[p].objects[0]
                .ai
                .as_ref()
                .and_then(|ai| ai.orders.first().copied())
                .map(|o| (o.issuer, o.order, o.target, o.target_owner))
        };
        assert_eq!(order(2), Some((2, BUILD_ORDER, 10028, 1)));
        assert_eq!(order(1), Some((1, BUILD_ORDER, 10028, 1)));
    }

    /// A world of player 1's villager 9175 (master 83, which has no Build task) beside its
    /// house's foundation 10028 (master 70, state 0, no build points yet), with the unit AI's
    /// queue `orders`.
    fn sent_off_world(orders: Vec<crate::unit_ai::QueuedOrder>) -> Turns {
        let villager = Unit {
            id: 9175,
            position: [29.5, 89.5, 0.0],
            life: Lifecycle {
                class: Class::Animated,
                state: 2,
                hp: 25.0,
                master_id: 83,
                ..Lifecycle::default()
            },
            ai: Some(UnitAi {
                ai_type: crate::unit_ai::CIVILIAN,
                current_order: -1,
                current_action: -1,
                current_target: -1,
                last_target: -1,
                orders,
                ..UnitAi::default()
            }),
            pasture: -1,
            my_pasture: -1,
            ..Unit::default()
        };
        let foundation = Unit {
            id: 10028,
            position: [30.0, 88.0, 0.0],
            life: Lifecycle {
                class: Class::Animated,
                state: 0,
                hp: 1.0,
                master_id: 70,
                ..Lifecycle::default()
            },
            construction: Some(Construction {
                progress: 0.0,
                desolid: true,
                builders: 0,
                hit_points: 550,
                build_time: 25,
                record: crate::spatial::Obstruction { kind: 2, class: 2 },
                snow: false,
                sprite_override: false,
                started: false,
            }),
            pasture: -1,
            my_pasture: -1,
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
                    objects: vec![villager, foundation],
                    attributes: vec![0.0; 250],
                    relations: vec![1, 0],
                    ..Player::default()
                },
            ],
        );
        t.masters = vec![builder_masters(), builder_masters()];
        t
    }

    fn task(action_type: i16) -> crate::masters::Task {
        crate::masters::Task {
            action_type,
            object_group: -1,
            ..crate::masters::Task::default()
        }
    }

    /// Slot 0x820's search (`FUN_14106c6a0`): the own master's list first; without a task there
    /// the player's masters by id of the unit's class, type and swap group, with any filter,
    /// the first with a task giving it with the master to switch to; none without a group.
    #[test]
    fn slot_0x820_s_search_takes_the_own_task_else_the_first_master_of_the_swap_group() {
        let t = sent_off_world(vec![]);
        let masters = &t.masters[1];
        let find = |id: i32| t.object(id);
        let site = task_target_with(10028, 1, &find, &t.masters)
            .unwrap()
            .unwrap();
        let world = World {
            players: &t.players,
            drop_sites: &[],
        };
        let search = |own: &Master| {
            let actor = Actor {
                id: 9175,
                master: own,
                player: 1,
                held: (-1, 0.0),
                attribute_95: 0.0,
                civ: 0,
            };
            task_search(masters, &actor, &site, &site, -1, &world)
                .unwrap()
                .map(|(task, switch)| (task.action_type, switch.map(|m| m.id)))
        };
        // a villager of master 83 builds as the builder 118, the first of its group with a task
        assert_eq!(search(masters.get(83).unwrap()), Some((101, Some(118))));
        // the builders find it in their own lists
        assert_eq!(search(masters.get(118).unwrap()), Some((101, None)));
        assert_eq!(search(masters.get(212).unwrap()), Some((101, None)));
        // without a swap group there is no other list to try
        let alone = Master {
            task_by_group: 0,
            ..masters.get(83).unwrap().clone()
        };
        assert_eq!(search(&alone), None);
    }

    /// The task finder's target (`Turns::task_target`'s reading): an annex stands for its main
    /// building, once per step; an object not held is none.
    #[test]
    fn a_task_target_reads_an_annex_as_its_main_building() {
        let mut t = sent_off_world(vec![]);
        t.players[1].objects[0].annex_of = Some(10028);
        let find = |id: i32| t.object(id);
        let at = |steps| {
            task_target_with(9175, steps, &find, &t.masters)
                .unwrap()
                .map(|x| (x.id, x.master.id, x.state))
        };
        assert_eq!(at(0), Some((9175, 83, 2)));
        assert_eq!(at(1), Some((10028, 70, 0)));
        assert_eq!(at(2), Some((10028, 70, 0)));
        assert!(
            task_target_with(77, 1, &find, &t.masters)
                .unwrap()
                .is_none()
        );
    }

    /// The unit's slot 0x388 after its unit AI's update (`FUN_141226fc0`): the switch to the
    /// master slot 0x820's search gave (slot 0x90), the actions cleared and the wait (type 0x9a,
    /// state 10) the only one, its target −1 (the foundation is at its `+0x68`).
    #[test]
    fn a_wait_for_a_foundation_switches_the_villager_and_holds_the_task() {
        let mut t = sent_off_world(vec![]);
        let wait = Wait {
            target: 10028,
            task: task(101),
            switch: Some(118),
        };
        t.wait_for_foundation(1, 0, wait).unwrap();
        let v = &t.players[1].objects[0];
        assert_eq!(v.life.master_id, 118);
        assert_eq!(
            v.wait_solidify,
            Some(crate::wait_solidify::WaitSolidify::new(task(101), 10028))
        );
        assert_eq!(
            v.top_action(),
            Some(TopAction {
                kind: 0x9a,
                state: 10
            })
        );
        assert_eq!(v.top_target(), Some(-1));
        // without a switch the master stays
        let mut kept = sent_off_world(vec![]);
        kept.wait_for_foundation(
            1,
            0,
            Wait {
                switch: None,
                ..wait
            },
        )
        .unwrap();
        assert_eq!(kept.players[1].objects[0].life.master_id, 83);
    }

    /// While a player's list is out for its unit's update, the others are read around it, and
    /// the task finder reads the owner's diplomacy from the update and another player's from its
    /// own record.
    #[test]
    fn a_unit_s_side_reads_the_other_players_relations_around_its_own() {
        let players: Vec<Player> = (0..3)
            .map(|q| Player {
                relations: vec![q as u8; 3],
                ..Player::default()
            })
            .collect();
        let foreign = Foreign {
            before: &players[..1],
            after: &players[2..],
        };
        assert_eq!(foreign.player(0).map(|p| p.relations[0]), Some(0));
        assert!(foreign.player(1).is_none());
        assert_eq!(foreign.player(2).map(|p| p.relations[0]), Some(2));
        assert!(foreign.player(3).is_none());
        // the owner (1) counts player 2 as an ally (0), player 2 counts the owner an enemy (2)
        let world = SideWorld {
            player: 1,
            relations: &[1, 1, 0],
            foreign,
            drop_sites: &[],
        };
        assert!(world.allied(1, 2, false));
        assert!(!world.allied(2, 1, false));
        assert!(world.allied(1, 1, false));
    }

    /// The wait's foundation as its update reads it: its build points among the owner's other
    /// objects or the other players', none when the world does not hold it.
    #[test]
    fn a_wait_reads_its_foundation_s_build_points_wherever_it_is() {
        let t = sent_off_world(vec![]);
        let (none, players) = (&t.players[..0], &t.players[..]);
        let own = Foreign {
            before: none,
            after: none,
        };
        assert_eq!(wait_progress(10028, &t.players[1].objects, own), Some(0.0));
        assert_eq!(wait_progress(77, &t.players[1].objects, own), None);
        // another player's (player 0 updating, player 1 after it)
        let other = Foreign {
            before: none,
            after: &players[1..],
        };
        assert_eq!(wait_progress(10028, &[], other), Some(0.0));
    }

    /// `FUN_1411cc430` on an annex whose main building is in state 0 with a hit point (the
    /// builder's work deferred to the caller, [`crate::build::Worked::Main`]): the main
    /// building's annexes start first (the annex itself among them, desolid), then the main
    /// building starts and takes the work; the annex takes none.
    #[test]
    fn the_work_on_an_annex_goes_to_its_main_building_after_its_annexes_start() {
        let mut t = sent_off_world(vec![]);
        let annex = Unit {
            id: 10031,
            annex_of: Some(10028),
            ..t.players[1].objects[1].clone()
        };
        t.players[1].objects.push(annex);
        t.spatial = Spatial::with_tiles(
            [120, 120],
            crate::spatial::Tiles::new(120, 120, vec![4; 14400]),
        );
        for (id, x) in [(10028, 30.0), (10031, 32.0)] {
            t.spatial.insert(
                id,
                Placed {
                    position: [x, 88.0, 0.0],
                    radius: [1.0, 1.0],
                    class: 3,
                    player: 1,
                    master: 70,
                    obstruction_type: 2,
                    buildable_on: 0,
                    tall: true,
                    speed: 0.0,
                    unknown: None,
                    obstruction: None,
                },
            );
        }
        t.spatial.set_annex(10031, Some(10028));
        t.work_building(10031, 0.021).unwrap();
        let c = |id: i32| t.object(id).unwrap().1.construction.unwrap();
        let started = f32::from_bits(0x2edb_e6ff);
        assert!(!c(10031).desolid);
        assert_eq!(c(10031).progress, started);
        assert!(!c(10028).desolid);
        assert_eq!(c(10028).progress, started + 0.021);
        assert_eq!(t.object(10031).unwrap().1.life.hp, 1.0);
        assert!(1.0 < t.object(10028).unwrap().1.life.hp);
    }

    /// The whole of it: a villager the foundation sent off (its queue the foundation's order)
    /// runs the order on its turn (slot 0x108 forced: its slot 0x388 finds the builder's task by
    /// the swap group), becomes the builder with the wait as its only action, and keeps waiting
    /// while the foundation has no build points; once they pass 0.01 its next update drops the
    /// wait, the unit AI's action value staying 0x27a.
    #[test]
    fn a_villager_sent_off_a_foundation_waits_for_it_as_a_builder() {
        use crate::unit_ai::{FOUNDATION_ORDER, QueuedOrder, WAIT_SOLIDIFY_ACTION};
        let order = QueuedOrder {
            issuer: 1,
            order: FOUNDATION_ORDER,
            priority: 100,
            target: 10028,
            target_owner: 1,
            point: [30.0, 0.0, 0.0],
            range: 0.0,
            shown: [30.0, 0.0],
            ready: true,
            target_type: 70,
        };
        let mut t = sent_off_world(vec![order]);
        t.tick(50);
        let v = &t.players[1].objects[0];
        assert!(!v.stopped, "{:?}", t.players[1].objects[0]);
        assert_eq!(v.life.master_id, 118);
        assert_eq!(v.top_action_kind(), Some(0x9a));
        let ai = v.ai.as_ref().unwrap();
        assert_eq!(
            (ai.current_action, ai.last_action, ai.current_order),
            (WAIT_SOLIDIFY_ACTION, WAIT_SOLIDIFY_ACTION, FOUNDATION_ORDER)
        );
        assert_eq!(ai.orders, vec![]);
        // player 0's update, then player 1's: the wait reads 0 build points and stays
        t.tick(50);
        t.tick(50);
        assert_eq!(t.players[1].objects[0].top_action_kind(), Some(0x9a));
        t.players[1].objects[1]
            .construction
            .as_mut()
            .unwrap()
            .progress = 0.5;
        t.tick(50);
        t.tick(50);
        assert_eq!(t.players[1].freezes, vec![]);
        let v = &t.players[1].objects[0];
        assert_eq!(v.wait_solidify, None);
        assert_eq!(
            v.ai.as_ref().map(|ai| ai.current_action),
            Some(WAIT_SOLIDIFY_ACTION)
        );
    }

    #[test]
    fn a_build_creates_a_foundation_whose_unit_ai_draws_before_the_turn_players_units() {
        // MarvinBlue/426048731 tick 281: player 1's BUILD draws three AI values in the command
        // phase, so player 2's units, on their turn, get the draws after them (a computer's
        // BUILD here: it needs no builder that can build it)
        let world = |objects: Vec<Unit>| {
            let mut t = Turns::new(
                0,
                29910,
                1,
                vec![0, 0],
                vec![
                    Player {
                        resources: [0.0, 100.0, 0.0, 0.0],
                        attributes: vec![0.0; 250],
                        computer: true,
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

    /// Build 158041's farm (master 50): its default task 0 is a Make of the farm food kind 16
    /// at `work_val1` 1.0, its work rate 0.4 a second, its capacity 15.
    fn farm_master() -> Master {
        Master {
            id: 50,
            kind: 80,
            class: crate::gather::FARM_CLASS,
            work_rate: 0.4,
            capacity: 15,
            default_task: Some(0),
            tasks: vec![crate::masters::Task {
                action_type: crate::make::MAKE,
                attribute_type: crate::make::FARM_FOOD_KIND,
                attribute_type2: 0,
                work_val1: 1.0,
                ..crate::masters::Task::default()
            }],
            ..Master::default()
        }
    }

    #[test]
    fn a_farm_starts_its_make_in_its_first_update_and_fills_from_the_next() {
        let farm = Unit {
            id: 40,
            life: Lifecycle {
                master_id: 50,
                attribute_type: 0,
                amount: 15.0,
                ..Lifecycle::default()
            },
            default_make: true,
            ..Unit::default()
        };
        let mut attributes = vec![0.0; 100];
        attributes[crate::make::FARM_FOOD] = 175.0;
        let mut t = Turns::new(
            0,
            1,
            0,
            vec![0],
            vec![Player {
                objects: vec![farm],
                attributes,
                ..Player::default()
            }],
        );
        t.masters = vec![Masters::new([farm_master()])];
        t.tick(104);
        let f = &t.players[0].objects[0];
        let make = f.make.unwrap();
        assert_eq!((make.state, make.remaining, make.total), (6, 175.0, 175.0));
        assert_eq!((f.life.attribute_type, f.life.amount), (16, 0.0));
        assert_eq!(
            f.top_action(),
            Some(TopAction {
                kind: crate::make::MAKE,
                state: 6
            })
        );
        t.tick(104);
        let f = &t.players[0].objects[0];
        let made = 0.4f32 * 1.0 * seconds(104);
        assert_eq!(
            (f.life.amount, f.make.unwrap().remaining),
            (made, 175.0 - made)
        );
        assert!(t.players[0].unsupported.is_none());
    }

    #[test]
    fn a_built_farm_s_master_starts_a_make_and_another_default_task_is_reported() {
        assert_eq!(farm_master().default_start(), (true, None));
        let mut other = farm_master();
        other.tasks[0].action_type = crate::fly::FLY;
        assert!(other.default_start().1.is_some());
        // a type the factories make nothing of starts nothing (a wild animal's Explore, 6)
        other.tasks[0].action_type = 6;
        assert_eq!(other.default_start(), (false, None));
        other.default_task = None;
        assert_eq!(other.default_start(), (false, None));
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
    fn a_unit_inside_heals_up_to_its_hit_points() {
        // records/425906646's update 2191: villager 27260 inside TC 27150 heals by the TC's
        // garrison heal rate once a second
        let master = Master {
            id: 592,
            class: 4,
            hit_points: 25,
            ..Master::default()
        };
        let mut v = Unit {
            id: 27260,
            life: Lifecycle {
                hp: 11.0,
                ..Lifecycle::default()
            },
            ..Unit::default()
        };
        v.heal(&master, 0.1).unwrap();
        assert_eq!(v.life.hp, 11.1);
        // a heal past the hit points stops at them, the powerup's room taking the rest
        v.life.hp = 24.95;
        v.hp_powerups = [1.0, 2.0];
        v.heal(&master, 0.1).unwrap();
        assert_eq!((v.life.hp, v.hp_powerups), (24.95f32 + 0.1, [1.0, 2.0]));
        // past them with the room: the overflow comes off the powerup, then a full unit holds
        // exactly its master's hit points
        v.life.hp = 26.95;
        v.heal(&master, 0.1).unwrap();
        assert_eq!(v.life.hp, 25.0);
        assert_eq!(v.hp_powerups[0], 1.0 - (26.95f32 + 0.1 - 27.0));
        // a paid class heals at a cost: not modelled
        let ship = Master {
            class: 22,
            ..master
        };
        assert!(v.heal(&ship, 0.1).is_err());
    }

    /// `FUN_140e6cfc0` resets every listed unit the world holds, a sleeping one too, and passes
    /// over an id it does not hold.
    #[test]
    fn an_order_resets_its_units_sleeping_ones_too() {
        let unit = |id: i32| Unit {
            id,
            ai: Some(UnitAi {
                patrol_path: true,
                patrol_waypoint: 2,
                adjusted_idle_timeout: 100,
                secondary_timer: 900,
                ..UnitAi::default()
            }),
            ..Unit::default()
        };
        let player = Player {
            objects: vec![unit(1)],
            sleeping: vec![unit(2)],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0, 0], vec![Player::default(), player]);
        t.reset_order_units(&[2, 9, 1]);
        for u in t.players[1].objects.iter().chain(&t.players[1].sleeping) {
            let ai = u.ai.as_ref().unwrap();
            assert_eq!((ai.patrol_path, ai.secondary_timer), (false, 750));
        }
    }

    #[test]
    fn a_load_goes_to_its_player_alone_unless_the_player_shares() {
        // records/425906646's update 818: player 6, with allies, does not share (+0x1426 0)
        let relations = vec![vec![0, 3, 3], vec![3, 0, 0], vec![3, 0, 0]];
        let mut adds = Vec::new();
        give_load((&relations, false), 1, &mut adds, 0, 10.0).unwrap();
        assert_eq!(adds, [(0, 10.0), (166, 10.0)]);
        assert!(give_load((&relations, true), 1, &mut adds, 3, 10.0).is_err());
    }

    /// A gate's slot 0x678 (`FUN_1411d4a30`): a standing, unlocked object of class 39 whose
    /// master transforms (`+0x9ba` not -1), between allies; any other object answers false.
    #[test]
    fn only_a_standing_unlocked_transforming_gate_lets_an_ally_pass() {
        let object = |id: i32, master: i16, state: u8, locked: bool| Unit {
            id,
            life: Lifecycle {
                master_id: master,
                state,
                ..Lifecycle::default()
            },
            gate: Some(Gate {
                locked,
                ..Gate::default()
            }),
            ..Unit::default()
        };
        let player = |objects: Vec<Unit>| Player {
            objects,
            relations: vec![0, 0],
            attributes: vec![0.0; 250],
            ..Player::default()
        };
        let mut t = Turns::new(
            0,
            1,
            0,
            vec![0; 2],
            vec![
                player(Vec::new()),
                player(vec![
                    object(1, 64, 2, false),
                    object(2, 65, 2, false),
                    object(3, 66, 2, false),
                    object(4, 64, 3, false),
                    object(5, 64, 2, true),
                ]),
            ],
        );
        let master = |id: i16, class: i16, transform_unit: i16| Master {
            id,
            class,
            building: Some(crate::masters::BuildingMaster {
                transform_unit,
                ..crate::masters::BuildingMaster::default()
            }),
            ..Master::default()
        };
        let masters = Masters::new([master(64, 39, 63), master(65, 39, -1), master(66, 4, 63)]);
        t.masters = vec![masters.clone(), masters];
        let passes: Vec<bool> = (0..5).map(|i| t.gate_passes(1, i, 1)).collect();
        assert_eq!(passes, [true, false, false, false, false]);
    }

    #[test]
    fn a_gate_opens_for_its_allies_while_they_update() {
        // records/425906646's update 636: player 2's gate 27687 (master 64, class 39 with a
        // transform unit) has no record, marks or shape while its team updates, so 27274's plan
        // passes it; the next player, an enemy, finds it closed again
        let gate = Unit {
            id: 27687,
            life: Lifecycle {
                master_id: 64,
                state: 2,
                ..Lifecycle::default()
            },
            gate: Some(Gate::default()),
            ..Unit::default()
        };
        let player = |objects: Vec<Unit>, relations: Vec<u8>| Player {
            objects,
            relations,
            attributes: vec![0.0; 250],
            ..Player::default()
        };
        let mut t = Turns::new(
            0,
            1,
            0,
            vec![0; 4],
            vec![
                player(Vec::new(), vec![0, 3, 3, 3]),
                player(Vec::new(), vec![3, 0, 0, 3]),
                player(vec![gate], vec![3, 0, 0, 3]),
                player(Vec::new(), vec![3, 3, 3, 0]),
            ],
        );
        let master = Master {
            id: 64,
            class: 39,
            obstruction_type: 2,
            obstruction_class: 5,
            building: Some(crate::masters::BuildingMaster {
                transform_unit: 63,
                ..crate::masters::BuildingMaster::default()
            }),
            ..Master::default()
        };
        t.masters = (0..4).map(|_| Masters::new([master.clone()])).collect();
        t.spatial = Spatial::with_tiles([20, 20], crate::spatial::Tiles::new(20, 20, vec![1; 400]));
        t.spatial.relations = t.players.iter().map(|p| p.relations.clone()).collect();
        let record = crate::spatial::Obstruction { kind: 2, class: 5 };
        t.spatial.insert(
            27687,
            Placed {
                position: [10.0, 10.5, 0.0],
                radius: [1.0, 0.5],
                class: 39,
                player: 2,
                master: 64,
                obstruction_type: 2,
                buildable_on: 0,
                tall: true,
                speed: 0.0,
                unknown: None,
                obstruction: Some(record),
            },
        );
        let blocked = |t: &Turns| {
            t.spatial
                .tiles()
                .word(9, 10)
                .is_some_and(|w| w & crate::spatial::BLOCKED != 0)
        };
        assert!(blocked(&t));
        let mut open = vec![false; 4];
        // player 1, an ally, opens both its own and player 2's gates
        t.open_gates(1, &mut open).unwrap();
        assert_eq!(open, [false, true, true, false]);
        assert!(t.spatial.get(27687).unwrap().obstruction.is_none());
        assert!(!blocked(&t));
        // player 2 is next and an ally of both: they stay open
        t.close_gates(1, &mut open);
        assert_eq!(open, [false, true, true, false]);
        t.open_gates(2, &mut open).unwrap();
        // player 3, next, is an enemy: they close
        t.close_gates(2, &mut open);
        assert_eq!(open, [false; 4]);
        assert_eq!(t.spatial.get(27687).unwrap().obstruction, Some(record));
        assert!(blocked(&t));
        t.open_gates(3, &mut open).unwrap();
        assert_eq!(open, [false, false, false, true]);
        assert!(blocked(&t));
        // a locked gate stays closed for its allies
        t.close_gates(3, &mut open);
        t.players[2].objects[0].gate = Some(Gate {
            locked: true,
            ..Gate::default()
        });
        t.open_gates(1, &mut open).unwrap();
        assert!(blocked(&t));
    }

    /// records/425906646's gate 27745 of player 3 (annexes 27746 and 27747) in update 2865: an
    /// ally walking between the posts opens it; a standing Gaia animal does not count, an enemy
    /// counts but is no ally, a wall next to it neither.
    #[test]
    fn a_gate_counts_the_moving_units_between_its_posts() {
        let mut s =
            Spatial::with_tiles([40, 40], crate::spatial::Tiles::new(40, 40, vec![1; 1600]));
        s.relations = vec![vec![0, 3, 3], vec![3, 0, 3], vec![3, 3, 0]];
        let placed = |x: f32, class, player, speed, kind| Placed {
            position: [x, 20.0, 0.0],
            radius: [0.2, 0.2],
            class,
            player,
            master: 0,
            obstruction_type: kind,
            buildable_on: 0,
            tall: true,
            speed,
            unknown: None,
            obstruction: Some(crate::spatial::Obstruction { kind, class: 2 }),
        };
        s.insert(27745, placed(20.0, 39, 1, 0.0, 2));
        s.insert(27746, placed(18.5, 39, 1, 0.0, 2));
        s.insert(27747, placed(21.5, 39, 1, 0.0, 2));
        s.insert(1, placed(19.6, 4, 1, 0.8, 5));
        let (units, allied) = gate_units(&s, 27745, &[27746, 27747]).unwrap();
        assert_eq!((units, allied), (vec![1], 1));
        s.insert(2, placed(20.4, 10, 0, 0.0, 5));
        s.insert(3, placed(20.8, 6, 2, 0.9, 5));
        s.insert(4, placed(21.0, 27, 1, 0.0, 2));
        let (units, allied) = gate_units(&s, 27745, &[27746, 27747]).unwrap();
        assert_eq!((units, allied), (vec![1, 3], 1));
        assert!(gate_units(&s, 27745, &[27746]).is_err());
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
        // too little food for the villager (46 here): no entry, no cost, no draw (records/
        // 425906646's update 31: player 5's refused QUEUE leaves player 6's draws alone)
        let mut poor = one_player(vec![town_centre(9)], [40.0, 0.0, 0.0, 0.0]);
        poor.tick_with(104, &[queue(9, 83, 1)]);
        assert!(poor.players[0].unsupported.is_none());
        assert_eq!(poor.players[0].resources[0], 40.0);
        let tc = &poor.players[0].objects[0];
        assert!(tc.production.as_ref().is_none_or(|p| p.queue.is_empty()));
        let mut idle = one_player(vec![town_centre(9)], [40.0, 0.0, 0.0, 0.0]);
        idle.tick_with(104, &[]);
        assert_eq!(
            poor.streams.get(Stream::Action).draws,
            idle.streams.get(Stream::Action).draws
        );
        let mut missing = one_player(vec![town_centre(9)], [200.0; 4]);
        missing.tick_with(104, &[queue(9 - 1, 83, 1)]);
        assert!(missing.players[0].unsupported.is_some());
    }

    /// `FUN_1411cbc40`: a queue adds its units' population to attribute 80 (the queued units),
    /// which keeps it until the unit spawns; the make action's start counts the one it makes in
    /// 81.
    #[test]
    fn a_queue_counts_its_units_population_as_queued() {
        let mut t = one_player(vec![town_centre(9)], [1000.0; 4]);
        t.tick_with(104, &[queue(9, 83, 3)]);
        let a = &t.players[0].attributes;
        assert_eq!((a[80], a[81]), (3.0, 1.0));
        // a unit without population (master 99, no storage of kind 4) adds none
        let mut other = one_player(vec![town_centre(9)], [1000.0; 4]);
        other.tick_with(104, &[queue(9, 99, 2)]);
        assert_eq!(other.players[0].attributes[80], 0.0);
    }

    #[test]
    fn a_queue_takes_as_many_as_the_room_and_the_resources_allow() {
        // food for two of three: two entries, two draws
        let mut two = one_player(vec![town_centre(9)], [100.0, 0.0, 0.0, 0.0]);
        two.tick_with(104, &[queue(9, 83, 3)]);
        let mut one = one_player(vec![town_centre(9)], [100.0, 0.0, 0.0, 0.0]);
        one.tick_with(104, &[queue(9, 83, 1)]);
        let queued = |t: &Turns| {
            t.players[0].objects[0]
                .production
                .as_ref()
                .unwrap()
                .queue
                .len()
        };
        assert_eq!(queued(&two), 2);
        assert_eq!(two.players[0].resources[0], 100.0 - 2.0 * 46.0);
        assert_eq!(
            two.streams.get(Stream::Action).draws,
            one.streams.get(Stream::Action).draws + 1
        );
        // 14 queued: room for one of five
        let mut full = one_player(vec![town_centre(9)], [1000.0; 4]);
        full.tick_with(104, &[queue(9, 99, 14), queue(9, 99, 5)]);
        assert_eq!(queued(&full), 15);
        // a computer player fills to 2
        let mut computer = one_player(vec![town_centre(9)], [1000.0; 4]);
        computer.players[0].computer = true;
        computer.tick_with(104, &[queue(9, 99, 5)]);
        assert_eq!(queued(&computer), 2);
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

    /// `BUILDING_COMMAND_QUEUE` on a town centre with a gather point: the gather point goes
    /// into the queue first and clears, then the packet's step; a second packet adds its step
    /// alone; SET GATHER POINT empties the queue.
    #[test]
    fn a_command_queue_takes_the_gather_point_first_then_the_steps() {
        let mut tc = town_centre(9149);
        tc.life.master_id = 109;
        tc.ai = Some(UnitAi {
            ai_type: 17,
            current_order: -1,
            current_action: -1,
            current_target: -1,
            last_target: -1,
            ..UnitAi::default()
        });
        tc.gather_point = Some(GatherPoint {
            point: [30.5, 40.5, 0.0],
            target: -1,
            target_master: -1,
        });
        let mut t = one_player(vec![tc], [200.0; 4]);
        t.masters = vec![Masters::new([Master {
            id: 109,
            kind: 80,
            garrison: 15,
            ..Master::default()
        }])];
        let step = |point| Command::CommandQueue {
            player: 0,
            ids: vec![9149],
            target: -1,
            point,
        };
        t.tick_with(25, &[step([50.0, 60.0])]);
        t.tick_with(25, &[step([70.0, 80.0])]);
        assert_eq!(t.players[0].unsupported, None);
        let tc = &t.players[0].objects[0];
        assert_eq!(tc.gather_point, None);
        let steps: Vec<_> = tc
            .ai
            .as_ref()
            .unwrap()
            .building_orders
            .iter()
            .map(|o| (o.order, o.target, o.point, o.priority, o.ready))
            .collect();
        let at = |x, y| (crate::unit_ai::MOVE_ORDER, -1, [x, y, -1.0], -1, false);
        assert_eq!(steps, vec![at(30.5, 40.5), at(50.0, 60.0), at(70.0, 80.0)]);
        t.tick_with(
            25,
            &[Command::GatherPoint {
                player: 0,
                ids: vec![9149],
                point: [-1.0, -1.0],
                target: -1,
                target_master: -1,
                own: false,
            }],
        );
        let left = &t.players[0].objects[0].ai.as_ref().unwrap().building_orders;
        assert_eq!(left.len(), 0);
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
