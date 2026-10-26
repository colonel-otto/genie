//! What the slice tracks of one object ([`Unit`]) and its update, action by action.

use crate::construction::build::{BuildAction, Construction};
use crate::gathering::hunt::{HuntAction, HuntSprites};
use crate::lifecycle::Lifecycle;
use crate::masters::Master;
use crate::movement::Mover;
use crate::movement::fly::Fly;
use crate::movement::move_to::MoveToAction;
use crate::ownership::artifact::Artifact;
use crate::production::Production;
use crate::sprite::{ActiveSprite, Graphic};
use crate::turn::command::GatherPoint;
use crate::turn::view::Context;
use crate::turn::{Gate, WONDER};
use crate::unit_ai::{GenieError, UnitAi};

mod ai;
mod death;
mod owner;
mod search;
mod stop;
mod top;
mod update;

pub(in crate::turn) use self::ai::AiResume;
pub(crate) use self::owner::PaidAttributes;
pub(crate) use self::owner::PointMove;
pub(crate) use self::owner::{Captured, HANDS_LEFT, LIVESTOCK, POPULATION, Step};
pub(in crate::turn) use crate::combat::volley::{Aimed, armed, same_members};
pub(in crate::turn) use crate::construction::building_update::blasts_at_death;
#[cfg(test)]
pub(in crate::turn) use crate::construction::building_update::{
    farm_stage_terrain, regeneration_rate,
};
pub(crate) use crate::garrison::unit::EnterWorld;
pub(crate) use crate::gathering::gather::unit::give_load;

/// A dying sprite the `.dat` does not have.
const NO_DYING_SPRITE: GenieError =
    GenieError::unsupported("a dying sprite the data file does not have");

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
    pub unmodelled_action: Option<GenieError>,
    /// A building's production queue and make action.
    pub production: Option<Production>,
    /// A building's gather point, which its new units go to.
    pub gather_point: Option<GatherPoint>,
    /// A builder's Build action, its top action.
    pub build: Option<BuildAction>,
    /// A repairer's Repair action, its top action ([`crate::construction::repair`]).
    pub repair: Option<crate::construction::repair::RepairAction>,
    /// `+0x9e8` (`num_builders`) of a combat object that is no building: the Repair actions
    /// counted among its repairers. A building's is its construction's
    /// ([`Construction::builders`]).
    pub builders: u8,
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
    /// Its unit group, if any ([`crate::groups::GroupLink`]).
    pub group: crate::groups::GroupLink,
    /// `mHitpointsPowerup` and `mHitpointsMaxPowerup` (`+0xa38`, `+0xa3c`), which the task finder
    /// adds to the hit points.
    pub hp_powerups: [f32; 2],
    /// `berserker_timer` (`+0xa10`): the seconds the regeneration holds back toward its next
    /// whole hit point ([`Unit::regenerate`]).
    pub regen_timer: f32,
    /// `mRegenerationRatePowerup` and `mRegenerationStinger` (`+0xa34`, `+0xb3c`), which the
    /// regeneration rate adds to its master's ([`regeneration_rate`]).
    ///
    /// [`regeneration_rate`]: crate::construction::building_update::regeneration_rate
    pub regen_powerups: [f32; 2],
    /// A villager's Hunt action, its top action.
    pub hunt: Option<HuntAction>,
    /// A forager's Gather action, its top action.
    pub gather: Option<crate::gathering::gather::GatherAction>,
    /// Its own Attack (type 9, not a Hunt's), its top action: a boar's on its attacker.
    pub attack: Option<crate::combat::attack::Attack>,
    /// An auto scout's Explore action, its top action.
    pub explore: Option<crate::movement::explore::ExploreAction>,
    /// A unit's Enter action (a garrison), its top action.
    pub enter: Option<crate::garrison::EnterAction>,
    /// A unit's Transport action (an unload command's), its top action.
    pub transport: Option<crate::garrison::transport::TransportAction>,
    /// A missile's own part with its action ([`crate::combat::missile`]).
    pub missile: Option<crate::combat::missile::Missile>,
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
    /// `damaged_lately_timer` (`+0x30d`, saved): 60 after a hit on a building (`FUN_1411cea90`),
    /// one off in each of its updates that passes whole seconds; the computer AI reads it.
    pub damaged_lately: u8,
    /// A building's `mLivestockSummation` (`+0xc28`, saved): the livestock inside it (their
    /// master's `+0x610`, `FUN_1411cb850`), whose food a mill makes ([`crate::players::world_player`]).
    pub livestock_summation: f32,
    /// A pasture's livestock (`+0xb78`, saved as `livestockId`): the objects its completion put
    /// on its edges ([`crate::turn::Turns::setup_pasture`]), in the order it made them.
    pub livestock: Vec<i32>,
    /// A pasture's workers (`+0xcb8`, saved as `pastureWorkerId`): the units that tend it
    /// ([`crate::turn::Turns::tend_pasture`]), in the order they came.
    pub pasture_workers: Vec<i32>,
    /// A building's `+0xc54`: the frame its BUILD command chose (`FUN_140e7a400`, the packet's
    /// `+0x1c`), which its standing sprite takes once built; 0 from its constructor and the load.
    pub build_frame: u8,
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
    /// What its master's passive abilities keep on it ([`crate::combat::abilities`]): the movement damage
    /// timer, the attack timer's rate and its terrain look.
    pub abilities: crate::combat::abilities::Abilities,
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
    /// `mMaxRangePowerup` (`+0xa40`, slot 0x700): what its shown range adds
    /// ([`crate::combat::damage::shown_range`]).
    pub range_powerup: f32,
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
    pub default_task: Option<GenieError>,
    /// The Fly task its master's default task starts once its action list is empty (a bird at
    /// a game's start, before its first update).
    pub default_fly: Option<crate::masters::Task>,
    /// Its master's default task is a Make, which starts once its action list is empty in
    /// state 2 (a farm or fish trap before its first update as a built building), with the task
    /// its master holds then.
    pub default_make: bool,
    /// Its master's default task is the Artifact action (livestock), which starts again once its
    /// action list is empty in state 2 (an animal out of a building whose Artifact went), its
    /// first update in the next tick, as a Fly's.
    pub default_artifact: bool,
    /// A farm's or fish trap's Make action, its top action ([`crate::gathering::make`]).
    pub make: Option<crate::gathering::make::MakeAction>,
    /// A villager's wait for a foundation (`TRIBE_Action_Wait_Solidify`), its only action
    /// ([`crate::construction::wait_solidify`]).
    pub wait_solidify: Option<crate::construction::wait_solidify::WaitSolidify>,
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
    /// A building's `+0xcb0` (`FUN_1411c5f70`): the id of the object its completion's stack unit
    /// made ([`crate::turn::Turns::stack_unit`]); None before (the save does not hold it).
    pub stack_made: Option<i32>,
    /// A building's own store (`+0xae0`, `FUN_1411abad0`'s map by attribute): what its storage
    /// entries of mode 16 put in at its completion and take out at its death
    /// (`FUN_1411e6580`).
    pub store: Vec<(i16, f32)>,
    /// `+0x6c0` (`FUN_141185010`): the master of its own its master pointer (`+0xa0`) reads, a
    /// copy (`FUN_141186d10`, the master's slot 0x48) of the one it read when it took it; None
    /// while it reads its player's. An ownership change gives it one ([`change_hands`]).
    ///
    /// [`change_hands`]: crate::turn::world::change_hands
    pub own_master: Option<Box<Master>>,
    /// `+0x6b8`: the copy of its old owner's death-spawn master (`+0x5e4`) an ownership change
    /// took ([`change_hands`]), which its death spawn makes its dead object of
    /// (`FUN_1411a5090`); None for none.
    ///
    /// [`change_hands`]: crate::turn::world::change_hands
    pub dead_master: Option<Box<Master>>,
    /// `+0xce0`, `attackingUnits` (saved): the units of players its owner is not allied with
    /// that took this foundation (a building below state 2 whose master has a stack unit,
    /// `+0x986`) as their target, once per take, in order ([`Unit::lists_taker`]); its stack
    /// unit's completion tells each of the new object.
    pub attacking_units: Vec<i32>,
}

/// The sprite a hunt's walk shows ([`crate::gathering::hunt::walk_sprite`]) from its task's sprites.
pub(in crate::turn) fn hunt_walk_sprite(
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
    crate::gathering::hunt::walk_sprite(
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

/// The second target's checks at the start of a Gather's or a Hunt's update (`0x140a8e1d0`,
/// `0x140ac7a00`) on `site` (−1 for none), which the world's table holds (`held`,
/// `FUN_1415417a0`) in `state`: one the table lacks goes (`FUN_140a7a190` with none, with nothing
/// to let go); one past state 6 is let go (slot 0x180) and goes. The game checks the second
/// target's absence before the first target's state; the two touch different objects. Returns
/// the second target after them and the object let go.
pub(crate) const fn second_target_checked(
    site: i32,
    held: bool,
    state: Option<u8>,
) -> (i32, Option<i32>) {
    if site == -1 || !held {
        return (-1, None);
    }
    match state {
        Some(s) if 6 < s => (-1, Some(site)),
        _ => (site, None),
    }
}

/// `RGE_Action_Object::update` (`0x14106ac90`): the action list of an object past state 2 goes
/// (`FUN_140a98620`), the Wonder's (master 276, `0x61b0` raw) only past state 5.
pub(in crate::turn) const fn loses_actions(life: &Lifecycle) -> bool {
    let last = if life.master_id == WONDER { 5 } else { 2 };
    last < life.state
}

/// The id of the stand-in a unit leaves in its list while it is taken out for its update or an
/// order ([`Unit::taken_out`]): no object has it, so a lookup by id during that time finds the
/// object itself, never the stand-in (the default unit's id 0 is a real object's, 428491175's
/// oak 0, which a petard's blast then read as an empty victim and felled).
pub(crate) const TAKEN_OUT: i32 = i32::MIN;

/// [`Unit::missile_hits`]'s hit mode 1 (a missed missile): true when it passes the object it
/// meets (its id, `player`, master type `kind` and blast defence `level`): one of the missile's
/// own player or a missile, an ally's, and, when it is not the `target`, Gaia's of a type above
/// 15 or one of a blast defence below the missile's (`blast_level`).
pub(crate) fn missed_passes(
    c: &Context<'_>,
    (id, player, kind, level): (i32, usize, u8, u8),
    target: i32,
    blast_level: u8,
) -> bool {
    if player == c.player || kind == 60 {
        return true;
    }
    let other = id != target;
    c.spatial.allied_with(c.player, player)
        || (other && player == 0 && 15 < kind)
        || (other && level < blast_level)
}

/// An unpacked trebuchet's class (`-0xe76` raw), whose slot 0x340 packs instead of moving.
const UNPACKED_TREBUCHET: i16 = 54;
