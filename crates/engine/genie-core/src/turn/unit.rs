//! What the slice tracks of one object ([`Unit`]) and its update, action by action.

use crate::artifact::{Artifact, Handover};
use crate::attack::Strike;
use crate::build::{BUILD, BuildAction, Builder, Completion, Construction, Site};
use crate::checkpoint::TopAction;
use crate::damage::Victim;
use crate::fly::Fly;
use crate::hunt::{Gatherer, HuntAction, HuntSprites};
use crate::lifecycle::{Class, Lifecycle};
use crate::masters::{Master, Masters};
use crate::move_to::{MoveTo, MoveToAction};
use crate::movement::{Me, Mover, Planning, footprint_gap, footprint_gap_squared};
use crate::pathing::Pathing;
use crate::production::Production;
use crate::rng::{Stream, Well512};
use crate::spatial::{Placed, Spatial};
use crate::sprite::{ActiveSprite, Graphic};
use crate::tasks::{Actor, DropSite};
use crate::turn::command::GatherPoint;
use crate::turn::view::{AiWork, Context, DropWorld, OwnerView, UnitView};
use crate::turn::world::{
    Releaser, ResourceWorld, Seeker, built_death, change_hands, clear_look_of, closest_resource_of,
    combat_dies, drop_site_search, farm_bonus, forget_resource, herded, nearest_drop_site,
    refund_of, release_own, state_of, victim_of, wait_progress,
};
use crate::turn::{
    ARMED_FIREPOWER, BUILDING_KIND, BUILDING_TERRAIN, DROP_SITE_MASTERS, EJECT_SHARE,
    GATE_CLOSE_DELAY, GATE_EPSILON, GATE_SHUT_HEIGHT, Gate, MONASTERY, SEARCH_AGAIN, SEARCH_NEEDED,
    SPEED_EPSILON, TREE_NEAR, TREE_SEARCH_WITHIN, VILLAGER_CLASS, WONDER, gate_units,
};
use crate::unit_ai::{
    ACTION_COMPLETED, ACTION_EXECUTED, AiMove, BUILD_ACTION, BUILD_ORDER, Effect, GenieError,
    Notify, Search, Surroundings, TARGET_MOVED, UnitAi,
};
use crate::visibility::{Seer, capture_kind};

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
    /// `damaged_lately_timer` (`+0x30d`, saved): 60 after a hit on a building (`FUN_1411cea90`),
    /// one off in each of its updates that passes whole seconds; the computer AI reads it.
    pub damaged_lately: u8,
    /// A building's `mLivestockSummation` (`+0xc28`, saved): the livestock inside it (their
    /// master's `+0x610`, `FUN_1411cb850`), whose food a mill makes ([`crate::world_player`]).
    pub livestock_summation: f32,
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
    /// What its master's passive abilities keep on it ([`crate::abilities`]): the movement damage
    /// timer, the attack timer's rate and its terrain look.
    pub abilities: crate::abilities::Abilities,
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
    pub default_task: Option<GenieError>,
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
    /// `+0x6c0` (`FUN_141185010`): the master of its own its master pointer (`+0xa0`) reads, a
    /// copy (`FUN_141186d10`, the master's slot 0x48) of the one it read when it took it; None
    /// while it reads its player's. An ownership change gives it one ([`change_hands`]).
    pub own_master: Option<Box<Master>>,
    /// `+0x6b8`: the copy of its old owner's death-spawn master (`+0x5e4`) an ownership change
    /// took ([`change_hands`]), which its death spawn makes its dead object of
    /// (`FUN_1411a5090`); None for none.
    pub dead_master: Option<Box<Master>>,
    /// `+0xce0`, `attackingUnits` (saved): the units of players its owner is not allied with
    /// that took this foundation (a building below state 2 whose master has a stack unit,
    /// `+0x986`) as their target, once per take, in order ([`Unit::lists_taker`]); its stack
    /// unit's completion tells each of the new object.
    pub attacking_units: Vec<i32>,
}

/// The sprite a hunt's walk shows ([`crate::hunt::walk_sprite`]) from its task's sprites.
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

/// The second target's checks at the start of a Gather's or a Hunt's update (`0x140a8e1d0`,
/// `0x140ac7a00`) on `site` (−1 for none), which the world's table holds (`held`,
/// `FUN_1415417a0`) in `state`: one the table lacks goes (`FUN_140a7a190` with none, with nothing
/// to let go); one past state 6 is let go (slot 0x180) and goes. The game checks the second
/// target's absence before the first target's state; the two touch different objects. Returns
/// the second target after them and the object let go.
pub(in crate::turn) const fn second_target_checked(
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

impl Unit {
    /// Whether a building's slot 0x178 or 0x180 (`FUN_1411dba70`, `FUN_1411d6020`) lists its taker
    /// among the attacking units (`+0xce0`, [`Unit::attacking_units`]): this object of `master` is a
    /// building (type 80) below state 2 whose master has a stack unit (`+0x986`), and the taker's
    /// player is not allied with its owner (`FUN_141663d90` with 1: `allied` false).
    pub(crate) fn lists_taker(&self, master: &Master, allied: bool) -> bool {
        master.kind == BUILDING_KIND
            && self.life.state < 2
            && master.building.is_some_and(|b| b.stack_unit != -1)
            && !allied
    }

    /// Slot 0x178 on this object of its player's live list, taken by the object `by` (`listed`,
    /// [`Unit::lists_taker`]). A building's (`FUN_1411dba70`) first lists the taker among its attacking
    /// units (`+0xce0`, [`Unit::attacking_units`]; once per take). Then the base take
    /// (`FUN_14119d3e0`): its worker count (`+0x644`) rises, to at most 550, and a live object flagged
    /// to sleep drops the flags (a sleeping one's wake is [`crate::turn::Turns::take`]'s). The
    /// building's wake from semi-asleep (`+0xc98`) after it never runs here: only the building's
    /// load (`FUN_1411df160` -> `FUN_1411ddd20`) sets that flag, from the save's
    /// `temp_semi_asleep`, which no saved building of the 158041 recordings has (1473 headers
    /// read); the slice does not hold it.
    pub(crate) fn taken(&mut self, listed: bool, by: i32) {
        if listed {
            self.attacking_units.push(by);
        }
        if self.workers < 550 {
            self.workers += 1;
        }
        self.life.goto_sleep = false;
    }

    /// The target of its hunt, which it attacks or gathers from.
    pub(in crate::turn) fn attack_target(&self) -> Option<i32> {
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

    /// `why` with what this object was doing in its context: its id and master, its top action
    /// (type and state) and its unit AI's order, action and target.
    #[must_use]
    pub fn in_context(&self, why: GenieError) -> GenieError {
        let mut why = why
            .with("object", self.id)
            .with("master", self.life.master_id);
        if let Some(a) = self.top_action() {
            why = why.with("action", format_args!("{} state {}", a.kind, a.state));
        }
        if let Some(ai) = &self.ai {
            why = why.with(
                "unit AI",
                format_args!(
                    "order {} action {} target {}",
                    ai.current_order, ai.current_action, ai.current_target
                ),
            );
        }
        why
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
            group: Some(self.group.id).filter(|&g| g != -1),
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
    pub fn top_action_idle(&self) -> Result<bool, GenieError> {
        if self.unmodelled_action.is_some() {
            return Err(GenieError::unsupported(
                "slots 0x190 and 0x288 of an action the slice does not model",
            ));
        }
        match self.top_action_kind() {
            None => Ok(true),
            Some(crate::artifact::ARTIFACT_TYPE) => Ok(self.artifact.is_some_and(|a| a.idle())),
            // a Make has no target, but its point (`+0x38`) is the base constructor's 0.0
            Some(crate::move_to::MOVE_TO | crate::make::MAKE) => Ok(false),
            Some(_) => Err(GenieError::unsupported(
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
    pub(crate) fn interruptible(&self) -> Result<bool, GenieError> {
        if self.unmodelled_action.is_some() {
            return Err(GenieError::unsupported(
                "slot 0x2d0 of an action the slice does not model",
            ));
        }
        let Some(TopAction { kind, state }) = self.top_action() else {
            return Ok(true);
        };
        Ok(match (kind, state) {
            (0x7e, _) => false,
            (0x6f, 4) => return Err(GenieError::unsupported("slot 0x2d0 of a trade in state 4")),
            (_, 0 | 1 | 3 | 0xd | 0xe) => true,
            (_, 2) => kind != crate::attack::ATTACK,
            (crate::artifact::ARTIFACT_TYPE, _) => self
                .artifact
                .is_some_and(|a| a.goal.is_none_or(|g| g[0] == -1.0)),
            _ => false,
        })
    }

    /// The Enter's update ([`crate::garrison::EnterAction::update`]) with what it reads of its
    /// target, one of its own player's (another player's is not modelled): room for the unit (its
    /// count, the units of the player inside it, plus 1 within its master's capacity) and the
    /// rest of [`crate::garrison::enter_view`]; a transport's reach test is
    /// [`crate::garrison::ship_reaches`]. Then its step: the enter (`FUN_140a867b0`) and the
    /// update's `set_state(1)`, with an enter again ([`crate::garrison::EnterStep::again`]); a
    /// wait's standing sprite; the transport's move, the caller's after the update
    /// ([`Context::transport_moves`]).
    pub(in crate::turn) fn enter_update(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
        me: Me,
    ) -> Result<(), GenieError> {
        let enter = self.enter.expect("checked by the caller");
        let found = others
            .iter()
            .find(|u| u.id == enter.target)
            .map(|t| (c.player, t))
            .or_else(|| c.foreign.find(enter.target));
        // the units inside the target, any player's (its slot 0x498)
        let foreign = c.foreign;
        let inside_of = |others: &[Self], id: i32| {
            let inside = |u: &&Self| u.life.inside == Some(id);
            let theirs: usize = (0..foreign.before.len() + 1 + foreign.after.len())
                .filter_map(|q| foreign.player(q))
                .map(|p| p.objects.iter().chain(&p.sleeping).filter(inside).count())
                .sum();
            others.iter().filter(inside).count() + theirs
        };
        let (building, walking, meeting) = match found {
            Some((tp, t)) => {
                let master = c
                    .every_masters
                    .get(tp)
                    .and_then(|m| m.get(t.life.master_id))
                    .ok_or(GenieError::unsupported(
                        "an Enter's building of a master the slice lacks",
                    ))?;
                let room = inside_of(others, t.id) < usize::from(master.garrison);
                let (mut view, walking, meeting) = crate::garrison::enter_view(
                    c.spatial,
                    c.zones,
                    c.every_masters,
                    self,
                    (t, master, room),
                    enter.range,
                )?;
                // FUN_141663dc0 with allies, or Gaia's
                view.allied = tp == 0 || crate::tasks::allied(c.relations, c.player, tp, true);
                (Some(view), walking, meeting)
            }
            None => (None, (0.0, 0.0), Ok(([0.0; 2], 0))),
        };
        let ship = found.map(|(_, t)| t);
        let walk = c.walk(self.mover.as_mut(), me);
        let step = self.enter.as_mut().expect("checked above").update(
            walk,
            building,
            walking,
            || meeting.clone(),
            |w, to| {
                let (Some(ship), Some(plan)) = (ship, w.plan.as_mut()) else {
                    return Ok(false);
                };
                crate::garrison::ship_reaches(ship, w.world, plan, to)
            },
        )?;
        if let Some(to) = step.order {
            c.transport_moves.push((enter.target, to));
        }
        if step.waits {
            self.shows_standing_sprite(c.masters, c.graphics, c.streams.get(Stream::Sprite))?;
        }
        if step.enter {
            let (tp, building) = others
                .iter()
                .find(|u| u.id == enter.target)
                .map(|t| (c.player, t))
                .or_else(|| c.foreign.find(enter.target))
                .expect("the update found it");
            let inside = inside_of(others, enter.target);
            let mut w = EnterWorld {
                spatial: c.spatial,
                pathing: c.pathing,
                masters: c.masters,
                target_masters: c.every_masters.get(tp),
                graphics: c.graphics,
                sprite_rng: c.streams.get(Stream::Sprite),
                drop_sites: c.drop_sites,
                attributes: c.attributes,
                civ: c.civ,
                sharing: c.sharing,
                player: c.player,
                attribute_adds: &mut c.attribute_adds,
                firepower: 0.0,
                left_group: None,
            };
            if !self.enter_building(building, inside, &mut w)? {
                return Err(GenieError::unsupported(
                    "an Enter that did not get in (state 2)",
                ));
            }
            // the update's own set_state(1) after the enter's
            self.entered(&mut w)?;
            if step.again {
                // FUN_140a867b0 on the unit inside: notify 0x1fa and its set_state(1), then the
                // update's
                self.post_own(
                    crate::ai_kinds::NOTIFY_ACTION_COMPLETED,
                    [crate::garrison::ENTER_ACTION, 0, 0],
                )?;
                self.entered(&mut w)?;
                self.entered(&mut w)?;
            }
            let firepower = w.firepower;
            c.left_group = w.left_group.map(|g| (g, enter.target));
            if let Some(b) = others.iter_mut().find(|u| u.id == enter.target) {
                b.firepower += firepower;
                b.garrison.push(self.id);
            } else {
                // another player's: the caller's after the update
                c.entered = Some((enter.target, firepower));
            }
        }
        if step.done {
            return Err(GenieError::unsupported("a done Enter outside its building"));
        }
        Ok(())
    }

    /// The Enter's `set_state(1)` (`FUN_140a87320` case 1): its sub-actions go (`FUN_140a98620`: a
    /// `MoveTo`'s teardown stops the unit, slot 0x5d0), the unit shows its master's standing
    /// sprite ([`Unit::shows_standing_sprite`]) and, the Enter not being quiet, its unit AI gets
    /// notify 0x1f9 with the enter action value.
    pub(crate) fn entered(&mut self, w: &mut EnterWorld<'_>) -> Result<(), GenieError> {
        let enter = self
            .enter
            .as_mut()
            .ok_or(GenieError::unsupported("an entered unit without its Enter"))?;
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
    /// manager (`FUN_141055b40`) and the mesh (`+0x660`, `FUN_141053840`), a member of a unit
    /// group leaves it (`FUN_14102c160`, the caller's: [`EnterWorld::left_group`]) and stands
    /// (slot 0x560), and it joins the
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
    ) -> Result<bool, GenieError> {
        let masters = w.masters.ok_or(crate::garrison::NO_MASTERS)?;
        let bm = w
            .target_masters
            .unwrap_or(masters)
            .get(building.life.master_id)
            .ok_or(GenieError::unsupported(
                "an enter into a building of a master the slice lacks",
            ))?;
        let me = masters
            .get(self.life.master_id)
            .ok_or(GenieError::unsupported(
                "an entering unit of a master the slice lacks",
            ))?;
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
            return Err(GenieError::unsupported(
                "an enter into a building at a fifth of its hit points (state 13)",
            ));
        }
        if me
            .tasks
            .iter()
            .find(|t| t.action_type == 0x88)
            .is_some_and(|t| t.object_id == bm.id)
        {
            return Err(GenieError::unsupported(
                "an enter of a master that transforms (task 0x88)",
            ));
        }
        if usize::from(bm.garrison) <= inside {
            return Ok(false);
        }
        self.go_inside(building, (masters, me, bm), w)?;
        self.post_own(
            crate::ai_kinds::NOTIFY_ACTION_COMPLETED,
            [crate::garrison::ENTER_ACTION, 0, 0],
        )?;
        self.entered(w)?;
        Ok(true)
    }

    /// The unit's slot 0x1a0 into `building` (`FUN_141212e10`; the room test, slot 0x198, is the
    /// caller's): its sight goes from its tile, it leaves its tile's list, the obstruction
    /// manager and the mesh, a member of a unit group leaves it and stands, it joins the
    /// building's, its records leave the registry's lists; the building's sight takes a circle
    /// of 5 and the villager's town-bell flag follows the garrison (`FUN_1412067f0` with 3).
    /// The building's `+0x9a8` (the units inside the unit), and a siege building's slot 0x1b0
    /// are not modelled.
    pub(crate) fn go_inside(
        &mut self,
        building: &Self,
        (masters, me, bm): (&Masters, &Master, &Master),
        w: &mut EnterWorld<'_>,
    ) -> Result<(), GenieError> {
        let target = building.id;
        if self.life.inside.is_some() {
            return Err(GenieError::unsupported("an enter of a unit inside another"));
        }
        // the building's `+0x9a8` takes the units inside the unit (slot 0x498)
        if !self.garrison.is_empty() {
            return Err(GenieError::unsupported(
                "an enter of a unit with units inside it",
            ));
        }
        let placed = w
            .spatial
            .get(self.id)
            .copied()
            .ok_or(GenieError::unsupported("an entering unit without a place"))?;
        if let Some(v) = w.spatial.sight.as_mut() {
            v.sight_off(self.id, self.position, placed.radius, false, false);
        }
        let at = building.position;
        w.spatial.go_inside(self.id, at);
        w.pathing.removed(self.id);
        // a member of a unit group (+0x6d0) leaves the group the world holds (removeUnit with the
        // building and 0, FUN_14102c160: the caller's, `left_group`), then stands (slot 0x560)
        if self.group.id != -1 {
            w.left_group = Some(self.group.id);
            if let Some(m) = self.mover.as_mut() {
                m.stand()?;
            }
        }
        let building_radius = w.spatial.get(target).map_or(bm.radius, |p| p.radius);
        self.life.inside = Some(target);
        // the building's slot 0x1c0 (FUN_1411cb850 -> FUN_14120b790): a unit (type 70) adds its
        // share to the building's volleys
        if me.kind == 70 {
            let own = w.target_masters.unwrap_or(masters);
            let rate = crate::garrison::attack_rate(bm, own, building.reload_powerup);
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
            // a building's sight takes a circle of 5 (FUN_141212e10, type 80 only)
            if bm.kind == crate::order::BUILDING {
                let _ = v.sight_on_los(target, at, building_radius, false, 5, false);
            }
        }
        // FUN_1412067f0 with 3, for a villager or master 1808: the town-bell record of its work
        // (FUN_141206a20 with the flag clear: the Enter's target at its place, no target class,
        // the unit AI's action, no second target), then the flag set for a building in state 2
        // (its own player's) that takes villagers; with 3 it is never cleared
        if me.class == 4 || me.id == 1808 {
            if !self.town_bell.flag {
                let action =
                    self.ai
                        .as_ref()
                        .map(|ai| ai.current_action)
                        .ok_or(GenieError::unsupported(
                            "a town-bell record of a unit without a unit AI",
                        ))?;
                self.town_bell.record(crate::town_bell::BackToWork {
                    target,
                    target_type: -1,
                    action,
                    target2: -1,
                    point: [at[0], at[1]],
                });
            }
            if bm.kind == crate::order::BUILDING
                && building.life.state == 2
                && bm.building.is_some_and(|b| b.garrison_type & 1 != 0)
            {
                self.town_bell.flag = true;
            }
        }
        Ok(())
    }

    /// stopAction (slot 0x3c0, `FUN_14108c160`): the action list goes ([`Unit::clear_own_actions`])
    /// and a unit in state 2 takes its standing sprite (slot 0x68 with slot 0x798's,
    /// [`crate::attack::set_sprite`]) where the slice holds its sprite.
    pub(in crate::turn) fn stop_action(&mut self, c: &mut Context<'_>) -> Result<(), GenieError> {
        let by = self.releaser(c);
        let sprite = c.streams.get(Stream::Sprite);
        let lets = (&mut c.releases, &mut c.builder_leaves);
        self.stop_with(lets, by, c.masters, c.graphics, sprite)
    }

    /// [`Unit::stop_action`] on the parts of the world it changes: the targets its actions let
    /// go and the foundations its Build leaves (`by` the unit, [`Unit::clear_own_actions`]), its
    /// masters and sprites, and the Sprite stream.
    pub(in crate::turn) fn stop_with(
        &mut self,
        (releases, builder_leaves): (&mut Vec<(i32, Releaser)>, &mut Vec<i32>),
        by: Releaser,
        masters: Option<&Masters>,
        graphics: &std::collections::BTreeMap<i16, Graphic>,
        sprite: &mut Well512,
    ) -> Result<(), GenieError> {
        self.tear_down_actions(releases, builder_leaves, by)?;
        if self.life.state == 2 {
            self.show_standing(masters, graphics, sprite)?;
        }
        Ok(())
    }

    /// The action list's clearing (`FUN_140a98620`) as the unit's own commands and stops run it
    /// ([`Unit::clear_own_actions`]); an action the slice does not hold has a teardown it cannot
    /// run.
    pub(in crate::turn) fn tear_down_actions(
        &mut self,
        releases: &mut Vec<(i32, Releaser)>,
        builder_leaves: &mut Vec<i32>,
        by: Releaser,
    ) -> Result<(), GenieError> {
        if self.unmodelled_action.is_some() {
            return Err(GenieError::unsupported(
                "a unit AI's stop of an action list the slice does not model",
            ));
        }
        self.clear_own_actions(releases, builder_leaves, by)
    }

    /// The unit's slot 0x68 with its master's standing sprite (slot 0x798's, the master's
    /// `+0x470`): its top speed becomes the standing sprite's ([`Mover::shows_standing`]) and,
    /// where the slice holds its sprite, the sprite starts ([`crate::attack::set_sprite`] on the
    /// Sprite stream). Its master is its own or its owner's (`masters`).
    pub(in crate::turn) fn show_standing(
        &mut self,
        masters: Option<&Masters>,
        graphics: &std::collections::BTreeMap<i16, Graphic>,
        sprite: &mut Well512,
    ) -> Result<(), GenieError> {
        if let Some(m) = self.mover.as_mut() {
            m.shows_standing();
        }
        if self.sprite.is_some() {
            let standing = self
                .own_master
                .as_deref()
                .or_else(|| masters.and_then(|m| m.get(self.life.master_id)))
                .map(|m| m.standing_sprite)
                .and_then(|id| graphics.get(&id).copied())
                .ok_or(GenieError::unsupported(
                    "a stopped unit without its standing sprite",
                ))?;
            crate::attack::set_sprite(&mut self.sprite, standing, sprite);
        }
        Ok(())
    }

    /// The action list's deletion (`FUN_140a98620`), the one every caller shares (the unit's
    /// death, its stops and commands, [`crate::turn::Turns::clear_actions`]): each action's
    /// destructor. A `MoveTo`'s (`FUN_140aa1f30`, an action's sub-action's too) stops the unit
    /// (slot 0x5d0). The base teardown (`FUN_140a73fc0`) lets the target go (slot 0x180) unless
    /// the action is quiet (`+0x60`; the Hunt's Attack is, so only the Hunt lets its target go),
    /// then the second target, into `releases` with the unit as `by`. The Build's
    /// (`FUN_140aadbf0`) first takes a counted builder (`+0x68`) out of its building's builders
    /// (`FUN_140aaddb0`, `+0x9e8`), its foundation into `builder_leaves`
    /// ([`crate::turn::Turns::leave_builders`]); the Artifact's (`FUN_140aab0d0`) lets its
    /// target go and its `MoveTo`'s stops the unit; the Fly's (`FUN_140a84750`) gives the unit
    /// its top speed as its animation speed (slot 0x208 into `+0x7b8`); the missile's lets its
    /// target and the object it was fired from go. The Make, the wait for a foundation
    /// (`0x140ae8780`) and the Explore (`0x140a87dd0`) hold no target. A building's production
    /// is a list of its own, which stays. Of the list's deletion itself, the next action's slot
    /// 0x28 for a unit below state 3 needs a second action, which the slice never holds.
    ///
    /// # Errors
    ///
    /// The unit's stop that the motion step does not model.
    pub(crate) fn clear_own_actions(
        &mut self,
        releases: &mut Vec<(i32, Releaser)>,
        builder_leaves: &mut Vec<i32>,
        by: Releaser,
    ) -> Result<(), GenieError> {
        let mut release = |t: i32| {
            if t != -1 {
                releases.push((t, by));
            }
        };
        self.wait_solidify = None;
        self.make = None;
        if let Some(mut a) = self.artifact.take() {
            let (walked, released) = a.tear_down();
            if walked && let Some(m) = self.mover.as_mut() {
                m.stop()?;
            }
            if let Some(t) = released {
                release(t);
            }
        }
        if let Some(f) = self.fly.take() {
            self.anim_speed = Some(self.mover.as_ref().map_or(0.0, |m| m.stats.speed));
            if f.move_to.is_some()
                && let Some(m) = self.mover.as_mut()
            {
                m.stop()?;
            }
        }
        if let Some(e) = self.enter.take() {
            if e.move_to.is_some()
                && let Some(m) = self.mover.as_mut()
            {
                m.stop()?;
            }
            release(e.target);
        }
        if let Some(b) = self.build.take() {
            if b.added {
                builder_leaves.push(b.target);
            }
            release(b.target);
            if b.move_to.is_some()
                && let Some(m) = self.mover.as_mut()
            {
                m.stop()?;
            }
        }
        if self.explore.take().is_some_and(|e| e.move_to.is_some())
            && let Some(m) = self.mover.as_mut()
        {
            m.stop()?;
        }
        if let Some(mut h) = self.hunt.take() {
            h.tear_down(self.mover.as_mut())?;
            release(h.target);
            release(h.carry_to);
        }
        if let Some(mut g) = self.gather.take() {
            g.tear_down(self.mover.as_mut())?;
            release(g.target);
            release(g.carry_to);
        }
        if let Some(mut a) = self.attack.take() {
            a.tear_down(self.mover.as_mut())?;
            release(a.target);
            release(a.target2);
        }
        if let Some(old) = self.move_to.take() {
            if let Some(m) = self.mover.as_mut() {
                m.stop()?;
            }
            release(old.target);
        }
        if let Some(a) = self.missile.as_mut().and_then(|m| m.action.take()) {
            release(a.target);
            release(a.fired_from);
        }
        Ok(())
    }

    /// Its unit AI's effects, in order.
    pub(in crate::turn) fn apply(
        &mut self,
        effects: Vec<Effect>,
        class: Option<i16>,
        c: &mut Context<'_>,
    ) -> Result<(), GenieError> {
        for e in effects {
            match e {
                Effect::Stop => self.stop_action(c)?,
                Effect::CapHumanOrder(cap) => {
                    self.last_human_order = self.last_human_order.min(cap);
                    c.spatial.set_gait(self.id, self.gait());
                }
                Effect::Move(_) | Effect::RestorePatrol => {
                    let class = class.ok_or(GenieError::unsupported(
                        "an AI move of a master the slice does not have",
                    ))?;
                    let m = match e {
                        Effect::Move(m) => Some(m),
                        _ => self.restore_patrol(),
                    };
                    if let Some(m) = m {
                        self.ai_move(m, class, c)?;
                    }
                }
                Effect::MoveAside(m) => c.move_aside.push(m),
                Effect::OrderOnTarget(order) => {
                    if c.order_on_target.replace((order, None)).is_some() {
                        return Err(GenieError::unsupported(
                            "two orders on targets in one update",
                        ));
                    }
                }
                Effect::AfterOrder(after) => {
                    let (_, tail) = c.order_on_target.as_mut().ok_or(GenieError::unsupported(
                        "a unit AI's tail after an order it did not take",
                    ))?;
                    *tail = Some(after);
                }
                Effect::HuntObject(target) => c.works.push(AiWork::Hunt(target)),
                Effect::GatherObject(target) => c.works.push(AiWork::Gather(target)),
                Effect::WorkOn { target, point } => {
                    // slot 0x7e8 with the carried-resource test, any task, +0xa14 0
                    c.works.push(AiWork::Work(crate::order::Work {
                        target,
                        point,
                        carry_test: true,
                        filter: -1,
                        notified: false,
                    }));
                }
                Effect::BuildObject(target) => c.works.push(AiWork::Build(target)),
                Effect::ChangeOwner(new) => self.ai_change_owner(new, c)?,
                Effect::Explore(point) => self.explore_to(point, c)?,
                Effect::MoveToObject {
                    target,
                    state,
                    range,
                } => self.move_to_object((target, state), range, c)?,
            }
        }
        Ok(())
    }

    /// The Hunt's `set_state(3)` (`0x140ac60e0` case 3) after its target went empty when it faced
    /// it: the target goes (slot 0x68 with none: its slot 0x180), the unit AI is told
    /// ([`Unit::hunt_search_notify`]), the unit takes the sprite it stands with (slot 0x68 with
    /// `FUN_140ac5210`'s) and the timer starts at −10 s. A hunt of class 10 would end its search
    /// (`set_state` 2), not modelled.
    pub(in crate::turn) fn hunt_set_search(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), GenieError> {
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
    pub(in crate::turn) fn hunt_search_state(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), GenieError> {
        let hunt = self.hunt.as_mut().expect("the caller's hunt");
        let task = hunt.task;
        if hunt.target_class == 10 {
            // a human player's hunt is done (set_state 2), a computer player's ends (1)
            if c.computer {
                return Err(GenieError::unsupported(
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
    pub(in crate::turn) fn hunt_stand(
        &mut self,
        c: &mut Context<'_>,
        task: &crate::hunt::HuntTask,
    ) -> Result<(), GenieError> {
        let master =
            c.masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(GenieError::unsupported(
                    "a hunter of a master the slice does not have",
                ))?;
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
    pub(in crate::turn) fn hunt_release<const N: usize>(&self, c: &mut Context<'_>, ids: [i32; N]) {
        for id in ids.into_iter().filter(|&id| id != -1) {
            c.releases.push((id, self.releaser(c)));
        }
    }

    /// The Hunt in state 3 tells its unit AI to look for the next target (`0x140ac60e0` case 3,
    /// `0x140ac7a00` case 2): notify 0x202 with the hunt action (0x265), its target class (`+0x68`)
    /// and its drop site: the second target; −1 for a master without drop sites (`+0x7a0`); else
    /// the closest (`FUN_1411671e0` as `set_state(8)` searches, [`Unit::drop_site`]), with none
    /// found 0, and the hunt's class goes (−1).
    pub(in crate::turn) fn hunt_search_notify(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), GenieError> {
        let hunt = self.hunt.as_ref().expect("the caller's hunt");
        let (carry_to, class) = (hunt.carry_to, i32::from(hunt.target_class));
        let master =
            c.masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(GenieError::unsupported(
                    "a hunter of a master the slice does not have",
                ))?;
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
    /// hunt with a target lets the drop site go (`FUN_140a7a190` with none: slot 0x180) and walks
    /// back to it (`set_state` 4). One without asks its unit AI: the queued orders (gather, hunt,
    /// on a target) whose target is gone or dead go (`FUN_140c5de60`); orders left end the hunt
    /// (the AI's order and action clear, `set_state` 1, the action list removes it: its teardown
    /// lets the drop site go). The target class (the AI's target's, else the hunt's `+0x68`) the
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
    pub(in crate::turn) fn hunt_after_drop(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<(), GenieError> {
        let hunt = self.hunt.as_ref().expect("the caller's hunt");
        let master =
            c.masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(GenieError::unsupported(
                    "a hunter of a master the slice does not have",
                ))?;
        if hunt.target != -1 {
            // back to the target: the drop site goes (FUN_140a7a190 with none), set_state(4)
            let v = c.victim.as_deref().ok_or(GenieError::unsupported(
                "a hunt's target the slice does not hold live",
            ))?;
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
                    .ok_or(GenieError::unsupported("a hunter without a place"))?,
                c.spatial
                    .get_known(v.id)?
                    .ok_or(GenieError::unsupported("a hunt's target without a place"))?,
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
            let hunt = self.hunt.as_mut().expect("the caller's hunt");
            let site = std::mem::replace(&mut hunt.carry_to, -1);
            let by = self.releaser(c);
            if site != -1 {
                c.releases.push((site, by));
            }
            let (Some(hunt), Some(mover)) = (self.hunt.as_mut(), self.mover.as_mut()) else {
                return Err(GenieError::unsupported(
                    "a hunter without its hunt or movement state",
                ));
            };
            if hunt.chase(&quarry, c.player, &engagement, arms, mover)? {
                return Err(GenieError::unsupported(
                    "a hunt's Attack whose MoveTo starts arrived after its drop-off",
                ));
            }
            if hunt.state == crate::hunt::FAILED {
                return Err(GenieError::unsupported(
                    "a hunt failing on its target after its drop-off",
                ));
            }
            return Ok(());
        }
        let ai = self.ai.as_mut().ok_or(GenieError::unsupported(
            "a hunt without a unit AI after its drop-off",
        ))?;
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
            // the base teardown lets the drop site go (its target is none by now)
            self.hunt_release(c, [hunt.target, hunt.carry_to]);
            return Ok(());
        }
        let class = if ai.target_type == -1 {
            i32::from(hunt.target_class)
        } else {
            ai.target_type
        };
        if ai.ai_type != crate::unit_ai::CIVILIAN {
            return Err(GenieError::unsupported(
                "slot 0x1b0 of another unit AI than the civilian's",
            ));
        }
        let kind = crate::unit_ai::civilian_kind(class);
        let found = if kind == -1 && hunt.target_class == 10 {
            // a hunt of class 10 searches nothing
            -1
        } else if kind == -1 {
            if self.my_pasture != -1 {
                return Err(GenieError::unsupported(
                    "a hunter tending a pasture (FUN_1411c7670)",
                ));
            }
            let mover = self
                .mover
                .as_ref()
                .ok_or(GenieError::unsupported("a hunter without movement state"))?;
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
                    reach: true,
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
            let shuns = |_: &Placed| -> Result<bool, GenieError> {
                Err(GenieError::unsupported(
                    "a class-10 searcher in a hunt's search",
                ))
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
                    return Err(GenieError::unsupported(
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
                return Err(GenieError::unsupported(
                    "a computer player's hunt that found nothing after its drop-off (set_state 1)",
                ));
            }
            let hunt = self.hunt.as_mut().expect("the caller's hunt");
            if hunt.point[0] == -1.0 || hunt.point[1] == -1.0 {
                return Err(GenieError::unsupported(
                    "a hunt without a point that found nothing after its drop-off (set_state 2)",
                ));
            }
            let mover = self
                .mover
                .as_mut()
                .ok_or(GenieError::unsupported("a hunter without movement state"))?;
            return hunt.search_walk(mover);
        }
        let foreign = c.foreign;
        let (owner, t) = match others.iter().find(|u| u.id == found) {
            Some(t) => (c.player, t),
            None => foreign.find(found).ok_or(GenieError::unsupported(
                "a hunt's new target the slice does not hold",
            ))?,
        };
        let t_master = c
            .every_masters
            .get(owner)
            .and_then(|m| m.get(t.life.master_id))
            .ok_or(GenieError::unsupported(
                "a hunt's target of a master the slice does not have",
            ))?;
        let relations = c
            .spatial
            .relations
            .get(c.player)
            .map_or(&[][..], |r| &r[..]);
        if matches!(t.life.master_id, 1893 | 1897) {
            return Err(GenieError::unsupported(
                "a hunt finding a farm (FUN_1411c7670)",
            ));
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
                .ok_or(GenieError::unsupported("a hunter without a place"))?,
            c.spatial
                .get_known(found)?
                .ok_or(GenieError::unsupported("a hunt's target without a place"))?,
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
        let by = self.releaser(c);
        let (Some(hunt), Some(mover)) = (self.hunt.as_mut(), self.mover.as_mut()) else {
            return Err(GenieError::unsupported(
                "a hunter without its hunt or movement state",
            ));
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
                // one of its own player's: allied, never listed
                let u = others
                    .iter_mut()
                    .find(|u| u.id == found)
                    .expect("found above");
                u.taken(false, by.id);
            } else {
                c.takes.push((found, by));
            }
        }
        if hunt.state == crate::hunt::FAILED
            && let Some(m) = self.mover.as_mut()
        {
            m.shows_standing();
        }
        if hunt.state == crate::hunt::FAILED && self.sprite.is_some() {
            let standing =
                c.graphics
                    .get(&master.standing_sprite)
                    .copied()
                    .ok_or(GenieError::unsupported(
                        "a hunter without its standing sprite",
                    ))?;
            crate::attack::set_sprite(&mut self.sprite, standing, c.streams.get(Stream::Sprite));
        }
        Ok(())
    }

    /// The hunt's `set_state(8)` after its gathering (`0x140ac60e0` case 8): a unit that carries
    /// nothing goes back to state 3 (not modelled). Without a drop site, its master's drop sites
    /// (`+0x7a0`; none when the first is −1) are searched among its player's objects, from the
    /// unit by path ([`drop_site_search`] with the unit), else by distance from its place (without
    /// it); none found is not modelled (notify 0x203, `set_state` 2). The drop site found is the
    /// second target (`FUN_140a7a190`, which takes it: slot 0x178 after the update) and a quiet
    /// `MoveTo` to it starts ([`HuntAction::set_drop`]).
    pub(in crate::turn) fn hunt_drop_off(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), GenieError> {
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
        if let Some(site) = self.drop_off_in(w, others)? {
            c.takes.push((site, self.releaser(c)));
        }
        Ok(())
    }

    /// The Gather's update (slot 6, `0x140a8e1d0`). It first checks its targets: a target gone
    /// from the world's table goes (slot 0x68 with none), one past state 6 is let go (slot 0x180)
    /// and goes; a second target gone goes (`FUN_140a7a190` with none), one past state 6 is let
    /// go and goes. Then state 8 walks the
    /// load to its drop site ([`crate::gather::GatherAction::update_drop`]): the load goes to the
    /// player, the drop site goes (`FUN_140a7a190` with none) and the gather walks back to its
    /// target (`set_state(4)`); without a target the unit AI's search is not modelled. The other
    /// states ([`crate::gather::GatherAction::update`]) may ask for the drop-off
    /// ([`Unit::gather_drop_off`]).
    pub(in crate::turn) fn gather_update(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
        me: Me,
    ) -> Result<(), GenieError> {
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
                    return Err(GenieError::unsupported(
                        "a gather's target the slice cannot read",
                    ));
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
        let site = gather.carry_to;
        let held = c.spatial.get(site).is_some();
        let (kept, let_go) = second_target_checked(site, held, state_of(site, others, c.foreign));
        gather.carry_to = kept;
        if let Some(t) = let_go {
            c.releases.push((t, by));
        }
        let master =
            c.masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(GenieError::unsupported(
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
                .ok_or(GenieError::unsupported("a gatherer without a place"))?;
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
                .ok_or(GenieError::unsupported("a gather's target without a place"))?;
            let gap = footprint_gap(
                self.position,
                [me_radius[0], me_radius[1]],
                t.position,
                t.radius,
            );
            let mover = self
                .mover
                .as_mut()
                .ok_or(GenieError::unsupported("a gatherer without movement state"))?;
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
                    return Err(GenieError::unsupported("two build commands in one update"));
                }
            }
            // state 0xf2's walk ended: the caller reseeds after the update
            crate::gather::Next::Reseed { arrived } => c.reseed = Some(arrived),
            crate::gather::Next::Stay | crate::gather::Next::Done => {}
        }
        Ok(())
    }

    /// The Gather's search in state 3 (`0x140a8cd40` case 3 and its repeat, `0x140a8e1d0` case
    /// 2): the unit's notify 0x202 to its unit AI (slot 0x328) with the gather action (`0x261`),
    /// the Gather's target type (`+0x68`) and a drop site: the second target's, else for a master
    /// with drop sites (`+0x7a0`) the one `FUN_1411671e0` finds from the unit (inside others too:
    /// [`Unit::drop_site_from`]), 0 when it finds none; −1 for a master without.
    pub(in crate::turn) fn gather_search(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), GenieError> {
        let gather = self.gather.as_ref().expect("the caller's gather");
        let (carry_to, target_type, state) = (gather.carry_to, gather.target_type, gather.state);
        let master =
            c.masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(GenieError::unsupported(
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
    pub(in crate::turn) fn gather_after_drop(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<(), GenieError> {
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
                    return Err(GenieError::unsupported(
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
                    return Err(GenieError::unsupported(
                        "a farmer that found nothing after its drop-off (set_state 0xf3)",
                    ));
                }
                // set_state(11) without a target: to the point within the task's range
                let mover = self
                    .mover
                    .as_mut()
                    .ok_or(GenieError::unsupported("a gatherer without movement state"))?;
                return gather.walk_back(0.0, mover);
            }
            // LAB_140a8ed55: a computer player's Gather is done, a human's idles
            let carry_to = gather.carry_to;
            if c.computer {
                let mut gather = self.gather.take().expect("the caller's gather");
                gather.tear_down(self.mover.as_mut())?;
            } else {
                let master = c.masters.and_then(|m| m.get(self.life.master_id)).ok_or(
                    GenieError::unsupported("a gatherer of a master the slice does not have"),
                )?;
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
            None => foreign.find(found).ok_or(GenieError::unsupported(
                "a gather's new target the slice does not hold",
            ))?,
        };
        let master =
            c.masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(GenieError::unsupported(
                    "a gatherer of a master the slice does not have",
                ))?;
        let t_master = c
            .every_masters
            .get(owner)
            .and_then(|m| m.get(t.life.master_id))
            .ok_or(GenieError::unsupported(
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
            return Err(GenieError::unsupported(
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
                        // one of its own player's: allied, never listed
                        let u = others
                            .iter_mut()
                            .find(|u| u.id == found)
                            .expect("found above");
                        u.taken(false, by.id);
                    } else {
                        c.takes.push((new, self.releaser(c)));
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
    pub(in crate::turn) fn gather_drop_off(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), GenieError> {
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
            c.takes.push((site, self.releaser(c)));
        }
        Ok(())
    }

    /// [`Unit::gather_drop_off`] in `w`, from the update or a command: the drop site it took
    /// when it is a new second target (the caller's to take, slot 0x178).
    pub(crate) fn gather_drop_off_in(
        &mut self,
        mut c: DropWorld<'_>,
        others: &[Self],
    ) -> Result<Option<i32>, GenieError> {
        if self.life.amount <= 0.0 {
            return Err(GenieError::unsupported(
                "a gather's drop-off carrying nothing (set_state 3)",
            ));
        }
        let gather = self.gather.as_ref().expect("the caller's gather");
        let (carry_to, state) = (gather.carry_to, gather.state);
        let (site, gap) = self.drop_site(&mut c, others, carry_to, state)?;
        let (Some(gather), Some(mover)) = (self.gather.as_mut(), self.mover.as_mut()) else {
            return Err(GenieError::unsupported(
                "a gatherer without its gather or movement state",
            ));
        };
        gather.carry_to = site;
        gather.set_drop(gap, mover)?;
        Ok((site != carry_to && site != -1).then_some(site))
    }

    /// [`Unit::hunt_drop_off`] in `w`, from the update or a command: the drop site it took when
    /// it is a new second target (`FUN_140a7a190`, the caller's to take, slot 0x178).
    pub(crate) fn drop_off_in(
        &mut self,
        mut c: DropWorld<'_>,
        others: &[Self],
    ) -> Result<Option<i32>, GenieError> {
        if self.life.amount <= 0.0 {
            return Err(GenieError::unsupported(
                "a hunt's drop-off carrying nothing (set_state 3)",
            ));
        }
        let hunt = self.hunt.as_ref().expect("the caller's hunt");
        let (carry_to, state) = (hunt.carry_to, hunt.state);
        let (site, gap) = self.drop_site(&mut c, others, carry_to, state)?;
        if site == -1 {
            return Err(GenieError::unsupported(
                "a hunt without a drop site (notify 0x203, set_state 2)",
            ));
        }
        let (Some(hunt), Some(mover)) = (self.hunt.as_mut(), self.mover.as_mut()) else {
            return Err(GenieError::unsupported(
                "a hunter without its hunt or movement state",
            ));
        };
        hunt.carry_to = site;
        hunt.set_drop(gap, mover)?;
        Ok((site != carry_to).then_some(site))
    }

    /// The drop site a gatherer's `set_state(8)` walks to, its second target `carry_to` or, with
    /// none and a master with drop sites (`+0x7a0`), the closest of them (`FUN_1411671e0`: first
    /// one it reaches, [`Unit::drop_site_from`] outside other objects, then any); −1 for none.
    /// With the footprint gap to it (`FUN_14118f680`).
    pub(in crate::turn) fn drop_site(
        &self,
        c: &mut DropWorld<'_>,
        others: &[Self],
        carry_to: i32,
        state: u8,
    ) -> Result<(i32, f32), GenieError> {
        let master =
            c.masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(GenieError::unsupported(
                    "a gatherer of a master the slice does not have",
                ))?;
        let mover = self
            .mover
            .as_ref()
            .ok_or(GenieError::unsupported("a gatherer without movement state"))?;
        let mut site = carry_to;
        if site == -1 && master.drop_sites.first().is_some_and(|&d| d != -1) {
            let masters = c.masters.ok_or(GenieError::unsupported(
                "a drop-off of a player without masters",
            ))?;
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
            .ok_or(GenieError::unsupported("a drop site without a place"))?;
        Ok((
            site,
            footprint_gap(self.position, ground, s.position, s.radius),
        ))
    }

    /// `FUN_1411671e0` from the unit (`state` its action's, which the planner reads) over its
    /// master's drop sites (`+0x7a0`): [`drop_site_search`] with the unit as the seeker, by path
    /// for a human player (its argument 9, `FUN_141659cf0` false), skipping objects inside others
    /// when `outside` (its argument 10).
    pub(in crate::turn) fn drop_site_from(
        &self,
        c: &mut DropWorld<'_>,
        others: &[Self],
        state: u8,
        outside: bool,
    ) -> Result<Option<i32>, GenieError> {
        let masters = c.masters.ok_or(GenieError::unsupported(
            "a drop-off of a player without masters",
        ))?;
        let master = masters
            .get(self.life.master_id)
            .ok_or(GenieError::unsupported(
                "a gatherer of a master the slice does not have",
            ))?;
        let mover = self
            .mover
            .as_ref()
            .ok_or(GenieError::unsupported("a gatherer without movement state"))?;
        let [w, h] = c.spatial.map;
        let terrain = c.spatial.terrain.as_ref().ok_or(GenieError::unsupported(
            "the map's zones without its terrain",
        ))?;
        let values =
            terrain
                .restrictions
                .get(mover.stats.restriction)
                .ok_or(GenieError::unsupported(
                    "a restriction the slice's data has not got",
                ))?;
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
    pub(in crate::turn) fn explore_to(
        &mut self,
        point: [f32; 2],
        c: &mut Context<'_>,
    ) -> Result<(), GenieError> {
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
                return Err(GenieError::unsupported(
                    "an Explore replacing another action",
                ));
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
    pub(in crate::turn) fn explore_owner(
        &mut self,
        owner: crate::explore::Owner,
        c: &mut Context<'_>,
    ) -> Result<(), GenieError> {
        use crate::explore::Owner;
        match owner {
            Owner::Nothing => return Ok(()),
            Owner::Stopped => {
                return Err(GenieError::unsupported(
                    "an Explore's set_state 2 (slots 0x68)",
                ));
            }
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
                .ok_or(GenieError::unsupported(
                    "an explorer without its standing sprite",
                ))?;
            crate::attack::set_sprite(&mut self.sprite, standing, c.streams.get(Stream::Sprite));
        }
        Ok(())
    }

    /// Slot 0x5d0 of a unit whose `MoveTo` went.
    pub(in crate::turn) fn stop_mover(&mut self) -> Result<(), GenieError> {
        self.mover
            .as_mut()
            .ok_or(GenieError::unsupported("a walker without movement state"))?
            .stop()
    }

    /// A stop since the last look cleared the animation speed (`+0x7b8`, [`Mover::speed_stopped`]).
    pub(in crate::turn) fn take_stopped_speed(&mut self) {
        if let Some(m) = self.mover.as_mut().filter(|m| m.speed_stopped) {
            m.speed_stopped = false;
            self.anim_speed = Some(0.0);
        }
    }

    /// The default task's Fly ([`crate::fly::Fly::start`]): a player's bird (its task's
    /// `+0x15`, not held) is not modelled; a master of fly mode 1 lifts the bird to the fly
    /// height (slot 0x58, the position setter: its tile stays, so do its counts).
    pub(in crate::turn) fn start_fly(
        &mut self,
        c: &mut Context<'_>,
        task: crate::masters::Task,
    ) -> Result<(), GenieError> {
        if c.player != 0 {
            return Err(GenieError::unsupported(
                "a player's bird's default Fly (its task's +0x15)",
            ));
        }
        let fly_mode = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .map(|m| m.fly_mode)
            .ok_or(GenieError::unsupported(
                "a bird of a master the slice does not have",
            ))?;
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
    pub(in crate::turn) fn died(
        &mut self,
        c: &mut Context<'_>,
        before: u8,
    ) -> Result<(), GenieError> {
        let Some(master) = c.masters.and_then(|m| m.get(self.life.master_id)) else {
            if self.life.dying_sprite.is_some() {
                return Err(GenieError::unsupported(
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
                return Err(GenieError::unsupported(
                    "a dying sprite of an object of another type than 70",
                ));
            }
            return Ok(());
        }
        combat_dies(self, master, c.spatial, c.pathing)?;
        if before == 2 {
            c.refund = refund_of(master, self.annex_of);
        }
        // the static update's sprite after the death (0x14118ad50): a building's by its build
        // points (its destruction sprite unbuilt), any other combat object's slot 0x7a0's
        let look = self.abilities.look.as_ref();
        let shown = if master.kind == 80 {
            let built = self.construction.ok_or(crate::build::NO_CONSTRUCTION)?;
            crate::sprite::building_death(master, look, built.progress)
        } else {
            crate::sprite::dying(master, look)
        };
        if let Some(id) = shown {
            let g = c.graphics.get(&id).copied().ok_or(GenieError::unsupported(
                "a dying sprite the data file does not have",
            ))?;
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
    pub(in crate::turn) fn foundation_died(
        &mut self,
        c: &mut Context<'_>,
        master: &Master,
    ) -> Result<(), GenieError> {
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
    pub(in crate::turn) fn more_work(
        &self,
        c: &mut Context<'_>,
        others: &[Self],
        building: i32,
        built: i16,
    ) -> Result<Option<crate::order::Work>, GenieError> {
        use crate::gather_search::{self, Candidate};
        use crate::image::gather_search::{REACH_CAP, REACH_FLOOR};
        /// The reach a line of sight below the floor takes (the Build's immediate 4.0).
        const REACH_LOW: f32 = 4.0;
        let masters = c.masters.ok_or(GenieError::unsupported(
            "a builder without its player's masters",
        ))?;
        let master = masters
            .get(self.life.master_id)
            .ok_or(GenieError::unsupported(
                "a builder of a master the slice does not have",
            ))?;
        let site = masters.get(built).ok_or(GenieError::unsupported(
            "a building of a master the slice does not have",
        ))?;
        let at = *c.spatial.get(building).ok_or(GenieError::unsupported(
            "a builder's building without a place",
        ))?;
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
            .ok_or(GenieError::unsupported("a builder without movement state"))?;
        let restriction = mover.stats.restriction;
        let (player, me_id, foreign, every) = (c.player, self.id, c.foreign, c.every_masters);
        let candidate = |id: i32, owner: usize, u: &Self, position: [f32; 3]| {
            let m = every
                .get(owner)
                .and_then(|m| m.get(u.life.master_id))
                .ok_or(GenieError::unsupported(
                    "an object of a master the slice does not have",
                ))?;
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
        let objects = |x: i32, y: i32| -> Result<Vec<Candidate>, GenieError> {
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
                        .ok_or(GenieError::unsupported(
                            "an object on a tile the slice does not hold",
                        ))?;
                    candidate(id, owner, u, p.position)
                })
                .collect()
        };
        let found =
            {
                let planner = c.pathing.planner(restriction, spatial.tiles()).ok_or(
                    GenieError::unsupported("a builder's restriction without a planner"),
                )?;
                let width = i64::from(planner.level_sizes().0[0]);
                let cells = planner.level(0);
                let blocked = move |x: i32, y: i32| {
                    cells[(width * i64::from(y) + i64::from(x)) as usize][0] == 0
                };
                gather_search::more_work(entry, &builder, spatial.map, Some(&blocked), &objects)?
            };
        let Some(found) = found else {
            return Ok(None);
        };
        if found.class == gather_search::FARM {
            return Err(GenieError::unsupported(
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
            .ok_or(GenieError::unsupported(
                "a builder's new work without a place",
            ))?
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
    pub(in crate::turn) fn hunt_tree_search(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
        within: Option<f32>,
        near: Option<f32>,
    ) -> Result<bool, GenieError> {
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
            .ok_or(GenieError::unsupported(
                "a hunt's next tree the slice does not hold",
            ))?;
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
            .ok_or(GenieError::unsupported(
                "a hunt's next tree of a master the slice lacks",
            ))?;
        let own = c
            .spatial
            .relations
            .get(c.player)
            .ok_or(GenieError::unsupported(
                "a hunter's player without relations",
            ))?;
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
        let master =
            c.masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(GenieError::unsupported(
                    "a hunter of a master the slice does not have",
                ))?;
        let drops_here = master.drop_sites.contains(&t.life.master_id);
        let at = |id| {
            c.spatial
                .get_known(id)?
                .ok_or(GenieError::unsupported("a hunt's object without a place"))
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
            .ok_or(GenieError::unsupported("a hunter without movement state"))?;
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
            c.takes.push((found, self.releaser(c)));
        }
        Ok(true)
    }

    /// The unit AI's `ClosestResourceObject` (slot 0x80, `FUN_140c49760`) with no drop site, no
    /// point and `param_6` 2 ([`crate::gather_search::closest_resource`]): the closest object of
    /// gather group `kind` with fewer than `cap` workers (−1 any) the unit reaches (slot 0x3d0
    /// with range 0), from the tiles the player explored that the unit's restriction lets it
    /// stand on; −1 for none.
    pub(in crate::turn) fn resource_search(
        &self,
        c: &mut Context<'_>,
        others: &[Self],
        kind: u8,
        cap: i32,
    ) -> Result<i32, GenieError> {
        self.resource_search_from(c, others, kind, cap, -1)
    }

    /// [`Unit::resource_search`] from the drop site `site` (−1 for none;
    /// [`crate::gather_search::closest_resource_at`]): the site's place, its master's radii and
    /// line of sight, and whether the search starts from its footprint (the site's tile in the
    /// unit's zone of its restriction, or a dock: masters 45 and 2172, `0x140c49d17`).
    pub(in crate::turn) fn resource_search_from(
        &self,
        c: &mut Context<'_>,
        others: &[Self],
        kind: u8,
        cap: i32,
        site: i32,
    ) -> Result<i32, GenieError> {
        let current = self
            .ai
            .as_ref()
            .ok_or(GenieError::unsupported(
                "a resource search without a unit AI",
            ))?
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
        let search = (current, 2, crate::unit_ai::NO_POINT);
        closest_resource_of(self, &mut w, others, (kind, cap, site), search).map(|(id, _)| id)
    }

    /// The unit as the releaser of its actions' targets (slot 0x180's argument).
    pub(in crate::turn) fn releaser(&self, c: &Context<'_>) -> Releaser {
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
    pub(in crate::turn) fn named_units(
        &self,
        c: &Context<'_>,
        view: &OwnerView<'_>,
        master: Option<&Master>,
    ) -> Result<Vec<crate::unit_ai::UnitSeen>, GenieError> {
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
            let found = view
                .others
                .iter()
                .find(|u| u.id == id)
                .map(|u| (view.player, u))
                .or_else(|| view.foreign.find(id));
            let (Some((owner, u)), Some(p)) = (found, c.spatial.get(id)) else {
                continue;
            };
            let m = c
                .every_masters
                .get(owner)
                .and_then(|ms| ms.get(u.life.master_id))
                .ok_or(GenieError::unsupported(
                    "a named object of a master the slice does not have",
                ))?;
            let me = master.ok_or(GenieError::unsupported(
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

    pub(crate) fn post_own(&mut self, kind: i32, p: [i32; 3]) -> Result<(), GenieError> {
        let (id, state) = (self.id, self.life.state);
        self.ai
            .as_mut()
            .map_or(Ok(()), |ai| ai.post(Notify::own(id, kind, p), state))
    }

    /// The unit's own part of its hit, written back: what a retaliation or reflected damage left
    /// of its hit points and powerups, its charge and `+0x8a8`, the damage notify (500) a
    /// retaliation posts to its unit AI; a retaliation that killed it is the caller's (Gaia
    /// taking a killed attacker is not modelled).
    pub(in crate::turn) fn hit_back(
        &mut self,
        own: Victim,
    ) -> Result<Option<crate::damage::Kill>, GenieError> {
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
            return Err(GenieError::unsupported(
                "a killed attacker that goes to Gaia",
            ));
        }
        Ok(own.killed)
    }

    /// The action list's deletion of the unit's own Attack (`FUN_140a98e20`): its teardown (a
    /// `MoveTo`'s stops the unit) and the base teardown (`FUN_140a73fc0`), which lets its target,
    /// then its second target go (slot 0x180).
    pub(in crate::turn) fn delete_attack(&mut self, c: &mut Context<'_>) -> Result<(), GenieError> {
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
    pub(in crate::turn) fn volley(
        &mut self,
        c: &mut Context<'_>,
        aimed: Aimed,
    ) -> Result<(), GenieError> {
        let master =
            c.masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(GenieError::unsupported(
                    "an attacker of a master the slice does not have",
                ))?;
        if (matches!(master.id, 527 | 528 | 1104 | 2133..=2135) || matches!(master.class, 35 | 64))
            && self.life.state == 2
        {
            return Err(GenieError::unsupported(
                "a self-destructing attacker's hit (its object 112 and its death)",
            ));
        }
        if matches!(master.charge_type, 6 | 7) {
            return Err(GenieError::unsupported(
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
                return Err(GenieError::unsupported(
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
            return Err(GenieError::unsupported(
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
    pub(in crate::turn) fn fire(
        &mut self,
        c: &mut Context<'_>,
        aimed: &Aimed,
    ) -> Result<(), GenieError> {
        self.attack_count += 1;
        let master =
            c.masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(GenieError::unsupported(
                    "a ranged unit of a master the slice does not have",
                ))?;
        if matches!(master.charge_type, 6 | 7) {
            return Err(GenieError::unsupported(
                "a charge projectile (charge types 6 and 7, slot 0x2f8)",
            ));
        }
        let facing = self
            .mover
            .as_ref()
            .map(|m| m.angle)
            .ok_or(GenieError::unsupported("a ranged unit that does not move"))?;
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
            return Err(GenieError::unsupported(
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
    pub(in crate::turn) fn volley_missile(
        &mut self,
        c: &mut Context<'_>,
        aimed: &Aimed,
        scatter: [f32; 2],
    ) -> Result<(), GenieError> {
        let masters = c
            .masters
            .ok_or(GenieError::unsupported("a volley without the masters"))?;
        let master = masters
            .get(self.life.master_id)
            .ok_or(GenieError::unsupported(
                "a ranged unit of a master the slice does not have",
            ))?;
        let facing = self
            .mover
            .as_ref()
            .map(|m| m.angle)
            .ok_or(GenieError::unsupported("a ranged unit that does not move"))?;
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
            None if 0 <= master.missile => masters.get(master.missile).ok_or(
                GenieError::unsupported("a missile its player's masters do not have"),
            )?,
            None => {
                return Err(GenieError::unsupported(
                    "a volley's extra missile of a master without a missile",
                ));
            }
        };
        if matches!(made.id, 527 | 528) {
            return Err(GenieError::unsupported(
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
    pub(in crate::turn) fn launch(
        &mut self,
        c: &mut Context<'_>,
        aimed: &Aimed,
        shot: &Shot,
    ) -> Result<(), GenieError> {
        let made = c
            .masters
            .and_then(|m| m.get(shot.made))
            .ok_or(GenieError::unsupported(
                "a missile its player's masters do not have",
            ))?;
        let stats = made.missile_stats.ok_or(GenieError::unsupported(
            "a missile master without its flight",
        ))?;
        if !made.recyclable {
            return Err(GenieError::unsupported(
                "a missile of a master that is not recyclable",
            ));
        }
        let moving = made.moving.ok_or(GenieError::unsupported(
            "a missile master that does not move",
        ))?;
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
            user_waypoints: Vec::new(),
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
                .ok_or(GenieError::unsupported(
                    "a missile without the map's terrain",
                ))?
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
        let missile = Releaser {
            id,
            class: made.class,
            player: c.player,
        };
        c.takes.push((aimed.target, missile));
        if shot.shooter {
            // the shooter, of the missile's own player: never listed
            self.taken(false, id);
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
    pub(in crate::turn) fn fly(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<(), GenieError> {
        let Some(mut missile) = self.missile.take() else {
            return Ok(());
        };
        let flown = self.fly_missile(&mut missile, c, others);
        self.missile = Some(missile);
        flown
    }

    pub(in crate::turn) fn fly_missile(
        &mut self,
        missile: &mut crate::missile::Missile,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<(), GenieError> {
        let master =
            c.masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(GenieError::unsupported(
                    "a missile of a master the slice does not have",
                ))?;
        let stats = master.missile_stats.ok_or(GenieError::unsupported(
            "a missile whose master has no flight",
        ))?;
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
                return Err(GenieError::unsupported(
                    "a missile's target the slice cannot read",
                ));
            }
            action.target = -1;
        }
        if action.fired_from != -1 && shooter_state.is_none() {
            if c.spatial.get(action.fired_from).is_some() {
                return Err(GenieError::unsupported(
                    "a missile's shooter the slice cannot read",
                ));
            }
            action.fired_from = -1;
        }
        if action.target != -1 && target_state.is_some_and(dead) {
            c.releases.push((action.target, self.releaser(c)));
            action.target = -1;
        }
        if action.fired_from != -1 && shooter_state.is_some_and(dead) {
            return Err(GenieError::unsupported(
                "a missile whose shooter died (its weapons, FUN_14113e170)",
            ));
        }
        match action.state {
            crate::missile::FLYING => {}
            crate::missile::DONE => {
                return Err(GenieError::unsupported(
                    "a missile action's update in state 1",
                ));
            }
            _ => {
                return Err(GenieError::unsupported(
                    "a missile action in a state but 1 and 4",
                ));
            }
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
            .ok_or(GenieError::unsupported("a missile without its moving part"))?;
        let mut at = mover.set_position(to, [w, h]);
        let ground = c
            .spatial
            .terrain
            .as_ref()
            .ok_or(GenieError::unsupported(
                "a missile without the map's terrain",
            ))?
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
    pub(in crate::turn) fn missile_hits(
        &mut self,
        action: &mut crate::missile::MissileAction,
        stats: &crate::missile::MissileStats,
        fired: Fired,
        old: [f32; 3],
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<(), GenieError> {
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
                .ok_or(GenieError::unsupported(
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
                        return Err(GenieError::unsupported(
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
                _ => return Err(GenieError::unsupported("a missile of hit mode 2")),
            }
        }
        if !met.is_empty() {
            action.met = met.iter().map(|m| m.0).collect();
        }
        Ok(())
    }

    /// The missile action's `set_state(1)` (`FUN_140a9de70`): the missile's slot 0xd8
    /// (`FUN_14118f580`), its hit points to 0.
    pub(in crate::turn) const fn end_missile(
        &mut self,
        action: &mut crate::missile::MissileAction,
    ) {
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
    pub(in crate::turn) fn missile_strike(
        &mut self,
        action: &crate::missile::MissileAction,
        fired: Fired,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<u8, GenieError> {
        if fired.charge {
            return Err(GenieError::unsupported("a charge projectile's own hit"));
        }
        if action.fired_from == -1 {
            return self.own_strike(action, fired.by, c);
        }
        let shooter = others
            .iter_mut()
            .find(|u| u.id == action.fired_from)
            .ok_or(GenieError::unsupported(
                "a missile's shooter outside its list",
            ))?;
        let master = c
            .masters
            .and_then(|m| m.get(shooter.life.master_id))
            .ok_or(GenieError::unsupported(
                "a missile's shooter of a master the slice does not have",
            ))?;
        if 0.0 < master.area_effect {
            return Err(GenieError::unsupported(
                "a missile of a shooter with an area effect",
            ));
        }
        if master.kind == crate::order::BUILDING {
            return Err(GenieError::unsupported(
                "a building's missile hit (its elevation factor, FUN_1411cc890)",
            ));
        }
        let terrain = c.spatial.terrain.as_ref();
        let mut own = victim_of(shooter, master, c.attributes, c.player, terrain, c.mode_680);
        let victim = c
            .victim
            .as_deref_mut()
            .filter(|v| v.id == action.target)
            .ok_or(GenieError::unsupported(
                "a missile's target the slice does not hold live",
            ))?;
        let hit = crate::attack::Hit {
            at: self.position,
            from_z: Some(action.origin.position[2]),
            missile: true,
            chance: master.hit_chance,
        };
        let object = c.streams.get(Stream::Object);
        let struck = crate::attack::strike(&mut own, victim, &mut c.blasted, hit, object)?;
        if let Some(target) = struck.executed {
            shooter.post_own(ACTION_EXECUTED, [600, target, 0])?;
        }
        if shooter.hit_back(own)?.is_some() {
            return Err(GenieError::unsupported(
                "a retaliation killing a missile's shooter",
            ));
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
    pub(in crate::turn) fn own_strike(
        &mut self,
        action: &crate::missile::MissileAction,
        fired_from: i32,
        c: &mut Context<'_>,
    ) -> Result<u8, GenieError> {
        if action.origin.owner != -1 {
            return Err(GenieError::unsupported(
                "a shooterless missile with an origin (its elevation, FUN_14113e550)",
            ));
        }
        let master =
            c.masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(GenieError::unsupported(
                    "a missile of a master the slice does not have",
                ))?;
        if 0.0 < master.area_effect {
            return Err(GenieError::unsupported(
                "a shooterless missile with an area effect",
            ));
        }
        let terrain = c.spatial.terrain.as_ref();
        let mut own = victim_of(self, master, c.attributes, c.player, terrain, c.mode_680);
        // the missile's own part is out of the unit while it flies
        own.source.fired_from = fired_from;
        let victim = c
            .victim
            .as_deref_mut()
            .filter(|v| v.id == action.target)
            .ok_or(GenieError::unsupported(
                "a missile's target the slice does not hold live",
            ))?;
        let hit = crate::attack::Hit {
            at: self.position,
            from_z: None,
            missile: true,
            chance: master.hit_chance,
        };
        let object = c.streams.get(Stream::Object);
        let struck = crate::attack::strike(&mut own, victim, &mut c.blasted, hit, object)?;
        if self.hit_back(own)?.is_some() {
            return Err(GenieError::unsupported("a retaliation killing a missile"));
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
    pub(in crate::turn) fn missile_lands(
        &mut self,
        missile: &mut crate::missile::Missile,
        stats: &crate::missile::MissileStats,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<(), GenieError> {
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
                .ok_or(GenieError::unsupported(
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
                .ok_or(GenieError::unsupported(
                    "a landed missile's blast of a master the slice does not have",
                ))?;
            if 0.0 < area {
                return Err(GenieError::unsupported("a landed missile's blast"));
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
                    return Err(GenieError::unsupported(
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
    pub(in crate::turn) fn gate_update(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), GenieError> {
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
            .ok_or(GenieError::unsupported("a gate without its gate state"))?;
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
    pub(in crate::turn) fn gate_count(
        &self,
        c: &Context<'_>,
        others: &[Self],
    ) -> Result<(usize, usize), GenieError> {
        let (units, allied) = gate_units(c.spatial, self.id, &annexes_of(self.id, others))?;
        Ok((units.len(), allied))
    }

    /// `TRIBE_Building_Object::update`'s farm part for a foundation (state 0) with a build time
    /// (`0x1411ca2ef`): its stage, its build points times 3 over its build time (truncated, at
    /// most 2), lays terrain 29 plus the stage under a farm (master 50) each update
    /// (`FUN_1411ce250` -> `FUN_1411ddd90`, [`crate::terrain::Terrain::cover`]); the rice farms'
    /// (1187, 1889: their foundation terrain plus 2, or 65, plus the stage by `+0xcd0`) are not
    /// modelled.
    pub(in crate::turn) fn farm_stage(&self, c: &mut Context<'_>) -> Result<(), GenieError> {
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
            1187 | 1889 => {
                return Err(GenieError::unsupported(
                    "a rice farm's stage terrain (+0xcd0)",
                ));
            }
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
            .ok_or(GenieError::unsupported(
                "a farm's foundation without the map's terrain",
            ))?
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
    pub(in crate::turn) fn complete_on_update(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), GenieError> {
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
            main_builders: None,
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
    /// modelled). Its `damaged_lately_timer` (`+0x30d`) loses one when whole seconds passed.
    pub(in crate::turn) fn building_seconds(
        &mut self,
        c: &Context<'_>,
        others: &mut [Self],
    ) -> Result<(), GenieError> {
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
            return Err(GenieError::unsupported(
                "a damaged building lets its garrison out (a fifth of its hit points)",
            ));
        }
        if c.attributes.get(BUILDING_TERRAIN) == Some(&f32::from(master.id)) {
            return Err(GenieError::unsupported(
                "a building whose master attribute 255 names (FUN_1411c39b0)",
            ));
        }
        let seconds = c.world_time.wrapping_sub(self.garrison_time) / 1000;
        if seconds == 0 {
            return Ok(());
        }
        self.garrison_time = self.garrison_time.wrapping_add(seconds * 1000);
        if self.damaged_lately != 0 {
            self.damaged_lately -= 1;
        }
        if master.id == MONASTERY && self.relics != [0; 2] {
            return Err(GenieError::unsupported(
                "a monastery's relics' goods (its seconds)",
            ));
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
                return Err(GenieError::unsupported(
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
                .ok_or(GenieError::unsupported(
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
            return Err(GenieError::unsupported(
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
    pub(in crate::turn) fn heal(&mut self, master: &Master, amount: f32) -> Result<(), GenieError> {
        if matches!(master.class, 2 | 20 | 21 | 22 | 53) {
            return Err(GenieError::unsupported(
                "a heal at a cost (FUN_141213b10's classes)",
            ));
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
    pub(in crate::turn) fn idle_swap(
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

    pub(in crate::turn) fn update(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<Step, GenieError> {
        if let Some(resume) = c.ai_resume.take() {
            return self.ai_part(c, others, resume);
        }
        // RGE_Static_Object::update (0x14118ad50): an object flagged to sleep (`+0x643`) drops
        // the flag, is marked asleep (`+0x641`) and moves to its player's sleeping list (player
        // slots 0x2d8 and 0x2d0), and its update ends; only static objects (type 10) are flagged
        if self.life.goto_sleep {
            if self.life.class != Class::Static || self.mover.is_some() || self.ai.is_some() {
                return Err(GenieError::unsupported(
                    "an object above the static type goes to sleep",
                ));
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
        // `+0x87c` (a terrain look's rate, [`crate::abilities`])
        if 0.0 < self.attack_timer {
            self.attack_timer -= c.dt_seconds * self.abilities.attack_rate;
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
            return Err(GenieError::unsupported(
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
                .ok_or(GenieError::unsupported(
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
                .ok_or(GenieError::unsupported(
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
        if let Some(e) = self
            .unmodelled_action
            .clone()
            .filter(|_| self.life.state == 2)
        {
            return Err(e);
        }
        // RGE_Action_Object::update (0x14106ac90), after the moving update: an object past
        // state 2 (the Wonder past state 5) loses its action list (FUN_140a98620)
        if loses_actions(&self.life) {
            let by = self.releaser(c);
            self.clear_own_actions(&mut c.releases, &mut c.builder_leaves, by)?;
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
                .ok_or(GenieError::unsupported(
                    "a Make whose master the slice does not have",
                ))?;
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
            && let Some(e) = self.default_task.clone()
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
        let by = self.releaser(c);
        if let Some(artifact) = self.artifact.as_mut().filter(|_| c.tick > 1) {
            let dt = c.dt;
            let ended = artifact.walk(Some(c.walk(self.mover.as_mut(), me)))?;
            // an ended MoveTo (codes 1 to 5) goes (FUN_140a98620): its teardown stops the unit,
            // quiet or not; the action lets its target go
            if ended.is_some()
                && let Some(m) = self.mover.as_mut()
            {
                m.stop()?;
            }
            if let Some(t) = ended.and_then(|e| e.released) {
                c.releases.push((t, by));
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
            // the check's sub-actions went: a walking MoveTo's teardown stops the unit
            if let Some(Handover::Taken { walked: true, .. }) = new
                && let Some(m) = self.mover.as_mut()
            {
                m.stop()?;
            }
            if let Some(kind) = ended.and_then(|e| e.notify) {
                self.post_own(kind, [crate::unit_ai::MOVE_ACTION, 0, 0])?;
            }
            match new {
                Some(Handover::Taken { new, .. }) => {
                    let (main, also) = self.capture(c, others, new)?;
                    captured = Some(Captured { new, main, also });
                }
                // state 100: the unit's stop (slot 0x148), then slot 0x70 to Gaia
                Some(Handover::ToGaia) => {
                    let class = self.placed_class(c)?;
                    self.capture_stop(c, class)?;
                    if self.change_owner(c, 0)? {
                        captured = Some(Captured {
                            new: 0,
                            main: true,
                            also: Vec::new(),
                        });
                    }
                }
                None => {}
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
            // FUN_140aaddb0: a building annex (`+0xbb8`) counts its builders in its main
            // building
            let counted_main = others
                .iter()
                .find(|u| u.id == target)
                .filter(|u| {
                    masters
                        .and_then(|m| m.get(u.life.master_id))
                        .is_some_and(|m| m.kind == BUILDING_KIND)
                })
                .and_then(|u| u.annex_of);
            let mut main_builders = None;
            let mut site = None;
            let mut starts = Vec::new();
            for u in others.iter_mut() {
                if u.id == target {
                    site = Some(u);
                    continue;
                }
                if Some(u.id) == counted_main {
                    let k = u
                        .construction
                        .as_mut()
                        .ok_or(crate::build::NO_CONSTRUCTION)?;
                    main_builders = Some(&mut k.builders);
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
            if counted_main.is_some() && main_builders.is_none() {
                return Err(crate::build::NOT_HELD);
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
                    main_builders,
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
                c.history_events.extend(built.history);
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
            if step.failed {
                self.post_own(crate::unit_ai::ACTION_FAILED, [BUILD_ACTION, 0, 0])?;
            }
            if step.stands {
                self.show_standing(c.masters, c.graphics, c.streams.get(Stream::Sprite))?;
            }
            if step.works_it {
                // slot 0x7e8 on the building it built, at its place (the caller's work command)
                let point = c
                    .spatial
                    .get(target)
                    .ok_or(GenieError::unsupported(
                        "a builder's building without a place",
                    ))?
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
                let built = built.ok_or(GenieError::unsupported(
                    "a builder's building the slice does not hold",
                ))?;
                if let Some(work) = self.more_work(c, others, target, built)? {
                    c.work_order = Some(work);
                } else {
                    if let Some(b) = self.build.as_mut() {
                        b.no_more_work()?;
                    }
                    release_own(others, masters, target, by)?;
                    // set_state(3) without orders: notify 0x202, then the standing sprite
                    self.post_own(SEARCH_NEEDED, [BUILD_ACTION, 0, 0])?;
                    self.show_standing(c.masters, c.graphics, c.streams.get(Stream::Sprite))?;
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
                    return Err(GenieError::unsupported(
                        "an inside unit's Enter with its MoveTo",
                    ));
                }
                c.releases.push((e.target, self.releaser(c)));
            } else if self.top_action_kind().is_some() || self.unmodelled_action.is_some() {
                return Err(GenieError::unsupported(
                    "the inside update of an action not an Enter",
                ));
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
            Some(move_to) => {
                // its state 3 reads the unit's AI for the path it asks (0x140aa2a70), with its
                // point: the goal, or the target's place (a target gone fails the update first)
                let point = match (move_to.target, move_to.goal) {
                    (-1, goal) => goal.map(|p| [p[0], p[1]]),
                    (id, _) => c.spatial.get(id).map(|t| [t.position[0], t.position[1]]),
                };
                let ask = match point {
                    Some(p) if move_to.state == MoveTo::Start => {
                        let at = [me.position[0], me.position[1]];
                        UnitAi::path_ask(self.ai.as_mut(), at, p)
                    }
                    _ => crate::movement::PathAsk::default(),
                };
                move_to.update(c.walk(self.mover.as_mut(), me), ask)?
            }
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
            // ... and the base teardown (FUN_140a73fc0) lets its target go (slot 0x180)
            if let Some(t) = self.move_to.take().map(|m| m.target).filter(|&t| t != -1) {
                c.releases.push((t, self.releaser(c)));
            }
            if let Some(m) = self.mover.as_mut() {
                m.stop()?;
            }
        }
        if let Some(explore) = self.explore.as_mut() {
            // 0x140a880c0: state 1 is done (the action list removes it, the default task's
            // restart is not modelled), 11 walks, 13 fails; the others wait
            match explore.state {
                crate::explore::DONE => {
                    return Err(GenieError::unsupported(
                        "a done Explore (the action list after it)",
                    ));
                }
                crate::explore::FAILED => {
                    return Err(GenieError::unsupported(
                        "a failed Explore (notify 0x1f9, set_state 2)",
                    ));
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
                .ok_or(GenieError::unsupported("a bird without movement state"))?;
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
            let master = c.masters.and_then(|m| m.get(self.life.master_id)).ok_or(
                GenieError::unsupported("a Make of a master the slice does not have"),
            )?;
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
        // the Hunt's update (0x140ac7a00) first checks its targets: one gone from the world's
        // table is dropped (slot 0x68 with none), one past state 6 is let go (slot 0x180, its
        // `+0x60` being 0) and dropped; then the second target ([`second_target_checked`])
        if let Some(target) = self.hunt.map(|h| h.target).filter(|&t| t != -1) {
            let state = c
                .victim
                .as_deref()
                .filter(|v| v.id == target)
                .map(|v| v.state);
            let gone = match state {
                Some(s) => 6 < s,
                None if c.spatial.get(target).is_some() => {
                    return Err(GenieError::unsupported(
                        "a hunt's target the slice cannot read",
                    ));
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
        if let Some(site) = self.hunt.map(|h| h.carry_to) {
            let held = c.spatial.get(site).is_some();
            let (kept, let_go) =
                second_target_checked(site, held, state_of(site, others, c.foreign));
            if let Some(h) = self.hunt.as_mut() {
                h.carry_to = kept;
            }
            if let Some(t) = let_go {
                self.hunt_release(c, [t]);
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
            // (FUN_140a7a190 with none: slot 0x180) and set_state(8) searches again
            hunt.timer += c.dt_seconds;
            if 0.0 <= hunt.timer {
                hunt.timer = 0.0;
                let site = std::mem::replace(&mut hunt.carry_to, -1);
                self.hunt_release(c, [site]);
                self.hunt_drop_off(c, others)?;
            }
        } else if let Some(hunt) = self.hunt.as_mut().filter(|h| h.state == crate::hunt::DROP) {
            // state 8 has no target: its drop site (the second target) is what it walks to
            let master = c.masters.and_then(|m| m.get(self.life.master_id)).ok_or(
                GenieError::unsupported("a hunter of a master the slice does not have"),
            )?;
            let me_at = c
                .spatial
                .get_known(self.id)?
                .ok_or(GenieError::unsupported("a hunter without a place"))?;
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
                    return Err(GenieError::unsupported(
                        "a hunt's drop-off idling (set_state 2)",
                    ));
                }
            }
        } else if let Some(hunt) = self
            .hunt
            .as_mut()
            .filter(|h| h.state == crate::hunt::FAILED)
        {
            // state 13 (0x140ac7a00 case 0xc): notify 0x1f9 with the hunt's action value, and the
            // update returns 3: the action list removes the hunt (FUN_140a98e20), whose teardown
            // lets its target, then its second target go
            let (target, site) = (hunt.target, hunt.carry_to);
            if hunt.attack.is_some() || hunt.move_to.is_some() {
                return Err(GenieError::unsupported("a failed hunt with a sub-action"));
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
            self.hunt_release(c, [site]);
        } else if let Some(hunt) = self
            .hunt
            .as_mut()
            .filter(|h| matches!(h.state, crate::hunt::WALK_POINT | crate::hunt::WALK_SEARCH))
        {
            // states 15 and 11: the walk to a point; state 15's end is set_state(2): the unit
            // takes the hunt's sprite to stand with (slot 0x68 with FUN_140ac5210's), the target
            // and the second target go; state 11's is set_state(3), the search
            let master = c.masters.and_then(|m| m.get(self.life.master_id)).ok_or(
                GenieError::unsupported("a hunter of a master the slice does not have"),
            )?;
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
                return Err(GenieError::unsupported(
                    "a done hunt of a tree's class (set_state 3)",
                ));
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
                .ok_or(GenieError::unsupported("a hunter without movement state"))?;
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
            let master = c.masters.and_then(|m| m.get(self.life.master_id)).ok_or(
                GenieError::unsupported("a hunter of a master the slice does not have"),
            )?;
            let victim = c.victim.take().ok_or(GenieError::unsupported(
                "an attack's target the slice does not hold live",
            ))?;
            let own = own.ok_or(GenieError::unsupported(
                "a hunter of a master the slice does not have",
            ))?;
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
                        .ok_or(GenieError::unsupported("a hunt's object without a place"))
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
                        return Err(GenieError::unsupported(
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
                        .map(|(id, _)| -> Result<(i32, f32), GenieError> {
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
                        Some(c.graphics.get(&master.special_sprite).copied().ok_or(
                            GenieError::unsupported(
                                "a special graphic the data file does not have",
                            ),
                        )?)
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
                state_position: self.ai.as_ref().map_or([0.0; 2], |ai| ai.state_position),
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
            let master = c.masters.and_then(|m| m.get(self.life.master_id)).ok_or(
                GenieError::unsupported("an attacker of a master the slice does not have"),
            )?;
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
            let master = c.masters.and_then(|m| m.get(self.life.master_id)).ok_or(
                GenieError::unsupported("an attacker of a master the slice does not have"),
            )?;
            let armed = armed(master, self.firepower);
            let computer = c.computers.get(c.player).copied().unwrap_or(false);
            let victim = c.victim.take().ok_or(GenieError::unsupported(
                "an attack's target the slice does not hold live",
            ))?;
            let own = own.ok_or(GenieError::unsupported(
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
                state_position: self.ai.as_ref().map_or([0.0; 2], |ai| ai.state_position),
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
                return Err(GenieError::unsupported(
                    "a walk's unit in the way (notify 0x215) the action does not pass on",
                ));
            }
        }
        let resume = AiResume {
            captured,
            leaves,
            after: None,
        };
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
    pub(in crate::turn) fn ai_part(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
        resume: AiResume,
    ) -> Result<Step, GenieError> {
        let AiResume {
            captured,
            leaves,
            after,
        } = resume;
        // the rest of a full update after its order on a target ran its part before that already
        if after.is_none()
            && let Some(m) = c
                .masters
                .and_then(|ms| ms.get(self.life.master_id))
                .filter(|m| 40 <= m.kind)
            && let Some(p) = c.spatial.get(self.id).copied()
        {
            let moving = self.mover.as_ref().is_some_and(Mover::is_moving);
            c.pathing.refresh(self.id, &p, moving, m.outline);
        }
        // RGE_Combat_Object::update after the actions: the object's sight mask; a changed mask of
        // an object others take over by seeing it (`+0x924` 1) gives it to the first player that
        // sees it ([`crate::visibility::sight_taker`]) when that one has its master
        let mut captured = captured;
        if after.is_none()
            && self.combat
            && let Some(mask) = c.spatial.combat_sight(self.id)?
            && let Some(q) =
                crate::visibility::sight_taker(mask, c.player, c.standard, c.sight_takers)
            && c.every_masters
                .get(q)
                .is_some_and(|m| m.get(self.life.master_id).is_some())
        {
            self.take_by_sight(c, q)?;
            captured = Some(Captured {
                new: q,
                main: true,
                also: Vec::new(),
            });
        }
        let busy = self.top_action_kind().is_some() || self.unmodelled_action.is_some();
        // the unit's master: its own (a captured unit's copy), else its owner's
        let master = self
            .own_master
            .as_deref()
            .or_else(|| c.masters.and_then(|ms| ms.get(self.life.master_id)));
        let class = master.map(|ms| ms.class);
        // a captured unit's unit AI runs as its new owner's (RGE_Combat_Object::update reads the
        // owner then): on that player's turn, with its time as it stands
        let (turn, player, computer) = match &captured {
            Some(k) if k.main => (
                (k.new == c.player_turn)
                    .then(|| c.player_time.get(k.new).copied())
                    .flatten(),
                k.new,
                c.computers.get(k.new).copied().unwrap_or(false),
            ),
            _ => (c.turn, c.player, c.computer),
        };
        let view = OwnerView::of(
            c,
            others,
            captured.as_ref().filter(|k| k.main).map(|k| k.new),
        );
        // RGE_Combat_Object::update: on the owner's turn the full update (slot 4), else
        // FUN_140c68980 for a unit not inside another; a dead unit's full update only adds the
        // world's dt to its time since the last enemy sighting (`+0x1b4`) before it returns
        if turn.is_some() && self.ai.is_some() && after.is_none() {
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
                return Err(GenieError::unsupported("a busy unit inside another"));
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
                        return Err(GenieError::unsupported(
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
                // a new owner sees the target with its own map
                target: match c.ai_target.clone()? {
                    Some(t) if view.player != c.player => {
                        let [x, y, _] = t.position;
                        let (tx, ty) = (
                            crate::checkpoint::cvttss2si(x),
                            crate::checkpoint::cvttss2si(y),
                        );
                        Some(crate::unit_ai::Seen {
                            visible: c.spatial.visible(view.player, x as i32, y as i32),
                            sight: c.spatial.tile_sight(view.player, tx, ty).unwrap_or(0),
                            ..t
                        })
                    }
                    t => t,
                },
                under_attack: self.under_attack,
                id: self.id,
                anim_speed: self.anim_speed,
                action_state: if self.unmodelled_action.is_some() {
                    None
                } else {
                    Some(self.top_action().map_or(0, |a| a.state))
                },
                attack_listed: self.attack.is_some(),
                units: self.named_units(c, &view, master)?,
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
                top_target: self.top_target_seen(c, others),
                min_range: master.map_or(0.0, |m| m.min_range),
                attack_moving: self.mover.as_ref().is_some_and(|m| m.attack_moving),
                dodge_chance: c.dodge_chances.get(player).copied().unwrap_or(0),
                keep_distance_chance: c.keep_distance_chances.get(player).copied().unwrap_or(0),
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
                        capture: at.capture,
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
                            construction_rate: view.construction_rate,
                            others: view.others,
                            zones: c.zones,
                            foreign: view.foreign,
                            every: c.every_masters,
                            masters: view.masters,
                            graphics: c.graphics,
                            takes: &mut c.takes,
                            releases: &mut c.releases,
                            builder_leaves: &mut c.builder_leaves,
                            waits: &mut c.waits,
                            relations: view.relations,
                            attributes: view.attributes,
                            civ: view.civ,
                            drop_sites: c.drop_sites,
                            mode_680: c.mode_680,
                        };
                        // an order on a target comes only from a full update, on the owner's turn
                        match (turn, after) {
                            (Some(_), Some(after)) => ai.after_order(
                                after,
                                c.world_time,
                                computer,
                                &at,
                                ai_stream,
                                &mut side,
                            ),
                            (Some(dt), None) => {
                                ai.update(dt, c.world_time, computer, &at, ai_stream, &mut side)
                            }
                            (None, _) => ai.off_turn(&at, &mut side, computer, ai_stream),
                        }
                    };
                    self.ai = Some(ai);
                    result?
                }
            };
            self.apply(effects, class, c)?;
        }
        if let Some(new) = c.taken_over.take() {
            captured = Some(Captured {
                new,
                main: true,
                also: Vec::new(),
            });
        }
        if let Some(k) = captured {
            return Ok(Step::Captured(k));
        }
        Ok(leaves)
    }

    /// Slot 400 (`FUN_14106d090`) as the unit AI reads it: its top action's target as the world
    /// holds it (its owner's other objects as they stand, the other players' lists), with its
    /// type, place and its master's radii; None without one.
    pub(in crate::turn) fn top_target_seen(
        &self,
        c: &Context<'_>,
        others: &[Self],
    ) -> Option<crate::unit_ai::TopTarget> {
        let id = self.top_target().filter(|&t| t != -1)?;
        let (owner, u) = others
            .iter()
            .find(|u| u.id == id)
            .map(|u| (c.player, u))
            .or_else(|| c.foreign.find(id))?;
        let m = c.every_masters.get(owner)?.get(u.life.master_id)?;
        Some(crate::unit_ai::TopTarget {
            id,
            kind: m.kind,
            position: u.position,
            radius: m.radius,
        })
    }

    /// `restorePatrolInformation`'s unit part ([`crate::unit_ai::Effect::RestorePatrol`]): its own
    /// waypoints go (slot 0x450) and become the saved patrol's points (slot 0x448 each, as x, y
    /// and height), and it moves to the saved waypoint (slot 0x340: range 0, forced, running; a
    /// waypoint outside the path is the game's static zero point): that move, the caller's
    /// ([`Unit::ai_move`]). None without a saved patrol.
    pub(in crate::turn) fn restore_patrol(&mut self) -> Option<AiMove> {
        let ai = self.ai.as_ref()?;
        let saved = ai.patrol_path.as_ref()?;
        let points = saved.path.points.clone();
        let w = usize::try_from(ai.patrol_waypoint)
            .ok()
            .and_then(|i| points.get(i))
            .copied()
            .unwrap_or([0.0; 3]);
        if let Some(m) = self.mover.as_mut() {
            m.clear_user_waypoints();
            for p in &points {
                m.add_user_waypoint(p[0], p[2], p[1]);
            }
        }
        Some(AiMove {
            point: [w[0], w[2], w[1]],
            range: 0.0,
            force: true,
            walk: false,
        })
    }

    /// `FUN_140aab570`: player `new` takes the animal over. Its stop (slot 0x148,
    /// `FUN_14108c050` -> `FUN_14106fa60`): its Artifact action's slot 0x38 (the unit AI of a
    /// class other than livestock would stop too, which is not modelled), then the ownership
    /// change (slot 0x70, [`Unit::change_owner`]); a livestock whose owner is then not Gaia has
    /// its new owner take Gaia's livestock in state 2 on the tiles it sees within 3 of the
    /// animal's tile, x then y, each the same way. Returns whether the animal changed hands and
    /// the others taken; the lists' moves, the storage and the class counters are the caller's.
    /// The new owner's computer AI part (`FUN_140aab130`), the local player's sound and
    /// statistics are not held.
    pub(in crate::turn) fn capture(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
        new: usize,
    ) -> Result<(bool, Vec<i32>), GenieError> {
        let class = self.placed_class(c)?;
        self.capture_stop(c, class)?;
        let changed = self.change_owner(c, new)?;
        // the scan reads the owner after the change: the old one when `new` lacks the master
        let mut taken = Vec::new();
        if class == LIVESTOCK && (if changed { new } else { c.player }) != 0 {
            scan_livestock(c, others, self.position, new, &mut taken)?;
        }
        Ok((changed, taken))
    }

    /// The master class the world files the captured object under.
    pub(in crate::turn) fn placed_class(&self, c: &Context<'_>) -> Result<i16, GenieError> {
        c.spatial
            .get(self.id)
            .map(|p| p.class)
            .ok_or(GenieError::unsupported(
                "a captured object the world does not hold",
            ))
    }

    /// The stop (slot 0x148, `FUN_14108c050`) of an animal or artifact of master class `class`
    /// from inside its update: the action object's stop (`FUN_14106fa60`: `+0x88a` 0, the top
    /// action's slot 0x38: the Artifact's, [`Artifact::stop`], keeps the action list: a walk's
    /// `MoveTo` goes and stops the unit, the target is let go, and the unit shows its standing
    /// sprite; without an Artifact the list goes, an animal or an artifact holding no Hunt,
    /// Gather, Build or Attack, whose slot 0x38 would keep it); then a unit AI of any class but
    /// livestock, not on action 0x2c2, stops (slot 0xb0 with 0: the order ends, the unit's
    /// stopAction) and forgets its targets (`FUN_140c4dcb0`), unless the unit's top action
    /// (slot 0x438) is a trade (0x6f) with a load of a kind (COMISS 0, amount; JNC: an amount
    /// that is not a number keeps it).
    pub(in crate::turn) fn capture_stop(
        &mut self,
        c: &mut Context<'_>,
        class: i16,
    ) -> Result<(), GenieError> {
        let by = self.releaser(c);
        if let Some(artifact) = self.artifact.as_mut() {
            let (walked, released) = artifact.stop();
            if walked {
                self.mover
                    .as_mut()
                    .ok_or(GenieError::unsupported(
                        "a walking animal without movement state",
                    ))?
                    .stop()?;
            }
            if let Some(t) = released {
                c.releases.push((t, by));
            }
            self.show_standing(c.masters, c.graphics, c.streams.get(Stream::Sprite))?;
        } else {
            // any other top action's slot 0x38 is the base's false for what an animal or an
            // artifact holds (a `MoveTo` of its own), and the list goes
            self.tear_down_actions(&mut c.releases, &mut c.builder_leaves, by)?;
        }
        let top = self.top_action_kind().map_or(-1, i32::from);
        let trading = top == TRADE_ACTION
            && crate::fmath::below_or_unordered(0.0, self.life.amount)
            && self.life.attribute_type != -1;
        if class != LIVESTOCK
            && !trading
            && let Some(ai) = self.ai.as_mut()
            && ai.current_action != CAPTURE_KEEPS_ACTION
        {
            let mut fx = Vec::new();
            let place = ([self.position[0], self.position[1]], class);
            ai.stop_with(false, Some(place), &mut fx)?;
            ai.forget_targets();
            self.apply(fx, Some(class), c)?;
        }
        Ok(())
    }

    /// The unit's slot 0x70 as its unit AI asks it ([`Effect::ChangeOwner`], the artifact's
    /// processIdle `FUN_140c1e410`): a combat unit's ([`Unit::change_owner`]); once it changed
    /// hands the caller moves it to the new owner's list after the update ([`Step::Captured`]).
    /// A building's (`FUN_1411cd860`) is not modelled.
    pub(in crate::turn) fn ai_change_owner(
        &mut self,
        new: usize,
        c: &mut Context<'_>,
    ) -> Result<(), GenieError> {
        let master = self
            .own_master
            .as_deref()
            .or_else(|| c.masters.and_then(|ms| ms.get(self.life.master_id)));
        if master.is_some_and(|m| m.kind == crate::masters::master_type::BUILDING) {
            return Err(HANDS_LEFT);
        }
        if self.change_owner(c, new)? {
            c.taken_over = Some(new);
        }
        Ok(())
    }

    /// `RGE_Combat_Object::update`'s takeover by sight of the unit by player `new`, which has its
    /// master: slot 0x70 ([`Unit::change_owner`]), then `+0x924` goes 0. The local player's
    /// sound and its capture notice (`FUN_1414c3380`) are not simulation. Not modelled: a
    /// building's slot 0x70 (`FUN_1411cd860`) and a siege weapon's (class 13) drop of the units
    /// it carries (`FUN_14120f800` with `FUN_141192d10`).
    pub(in crate::turn) fn take_by_sight(
        &mut self,
        c: &mut Context<'_>,
        new: usize,
    ) -> Result<(), GenieError> {
        let master = self
            .own_master
            .as_deref()
            .or_else(|| c.masters.and_then(|ms| ms.get(self.life.master_id)));
        if master.is_some_and(|m| m.kind == crate::masters::master_type::BUILDING) {
            return Err(HANDS_LEFT);
        }
        if master.is_some_and(|m| m.class == SIEGE_CLASS) && !self.garrison.is_empty() {
            return Err(HANDS_LEFT);
        }
        let _ = self.change_owner(c, new)?;
        self.capture_flag = 0;
        if let Some(s) = c.spatial.sight.as_mut().and_then(|v| v.seer_mut(self.id)) {
            s.capture = 0;
        }
        Ok(())
    }

    /// The ownership change (slot 0x70, [`change_hands`]) of the unit, from its owner to `new`:
    /// false when `new` has not its master.
    pub(in crate::turn) fn change_owner(
        &mut self,
        c: &mut Context<'_>,
        new: usize,
    ) -> Result<bool, GenieError> {
        change_hands(
            self,
            (c.player, new),
            c.every_masters,
            c.spatial,
            c.world_time,
        )
    }

    /// The unit's slot 0x340 on its unit AI's move ([`Unit::ai_move_with`]), what it lets go and
    /// the standing sprite of a `MoveTo` that fails at its start into the context `c`.
    pub(in crate::turn) fn ai_move(
        &mut self,
        m: AiMove,
        class: i16,
        c: &mut Context<'_>,
    ) -> Result<(), GenieError> {
        let by = self.releaser(c);
        let sprite_rng = c.streams.get(Stream::Sprite);
        self.ai_move_with(
            m,
            (class, c.player),
            (&mut c.releases, &mut c.builder_leaves, by),
            (c.masters, c.graphics, sprite_rng),
        )
    }

    /// The unit's slot 0x340 (`FUN_141216c50`) on a move to `m.point` its unit AI gives it (also
    /// its move off a footprint, [`crate::turn::Turns::move_aside`]), the unit of `class` of
    /// player `player`. An unpacked trebuchet (class 54) packs instead (its Pack action,
    /// `FUN_140ad4ff0` through slot 0x808), which is not modelled; a packed one (class 51) whose
    /// head action packs or unpacks (types `0x7c`, `0x7d`) does nothing, and the slice holds no such
    /// action, so it moves as any other: `FUN_1410866e0`. Without `m.force` a head `MoveTo` whose
    /// point (`+0x38`..`+0x40`, [`MoveToAction::point`]) is `m.point` stands: the move is done.
    /// Then the head action's slot 0x40 may take the point ([`Unit::head_takes_point`]; the
    /// target it lets go into `releases`). Otherwise a `MoveTo` to the point within `m.range`
    /// (`FUN_140aa0d30`, the master's walking sprite with `m.walk`, else its running one; the
    /// sprite is not held) is made, the action list is cleared (`FUN_140a98620`,
    /// [`Unit::clear_own_actions`]: the teardowns let their targets go into `releases` and a
    /// counted builder's foundation into `builder_leaves`, by `by`) and the `MoveTo` installed and
    /// started (slot 0x808, `FUN_14106e990`: its slot 0x70 sees an empty list; `FUN_140a977e0`
    /// starts an action of a unit below state 3, which the unit AI's moves are, and
    /// [`MoveToAction::to_point`]); one that ends at its start shows the standing sprite of the
    /// unit's master in `look` ([`Unit::shows_standing_sprite`]).
    pub(crate) fn ai_move_with(
        &mut self,
        m: AiMove,
        (class, player): (i16, usize),
        (releases, builder_leaves, by): (&mut Vec<(i32, Releaser)>, &mut Vec<i32>, Releaser),
        (masters, graphics, sprite_rng): (
            Option<&Masters>,
            &std::collections::BTreeMap<i16, Graphic>,
            &mut Well512,
        ),
    ) -> Result<(), GenieError> {
        if class == UNPACKED_TREBUCHET {
            return Err(GenieError::unsupported(
                "an AI move of an unpacked trebuchet (class 54: its Pack action, FUN_140ad4ff0)",
            ));
        }
        if !m.force
            && self.top_action_kind() == Some(crate::move_to::MOVE_TO)
            && self
                .move_to
                .and_then(|mt| mt.point(self.mover.as_ref()))
                .is_some_and(|p| p == m.point)
        {
            return Ok(());
        }
        match self.head_takes_point(m.point, player)? {
            PointMove::Taken(released) => {
                if let Some(t) = released {
                    releases.push((t, by));
                }
                return Ok(());
            }
            PointMove::Declined => {}
        }
        self.clear_own_actions(releases, builder_leaves, by)?;
        let move_to = MoveToAction::to_point(m.point, m.range, !m.walk, self.mover.as_mut());
        if move_to.shows_standing() {
            self.shows_standing_sprite(masters, graphics, sprite_rng)?;
        }
        self.move_to = Some(move_to);
        Ok(())
    }

    /// The head action's slot 0x40 on a move to `point` without a target object (its second
    /// argument 0), the unit of player `player`. An empty list takes nothing
    /// (`FUN_140a98730`), nor does the base slot (`FUN_140a78490`, which answers 0) of a `MoveTo`,
    /// an Explore, a Build (`0x140aade60`), an Attack, an Enter, a Fly, a missile, a Make or a
    /// wait for a foundation. The Artifact's (`FUN_140aaca30`) takes it
    /// ([`Artifact::take_point`]; a `MoveTo` it walked goes and stops the unit); the Hunt's
    /// (`FUN_140ac52c0`, [`HuntAction::take_move`]) unless its task has a work range; the
    /// Gather's (`0x140a8b740`, [`crate::gather::GatherAction::take_move`]) always. The Hunt and
    /// the Gather let their target go (slot 0x68 with none), which [`PointMove::Taken`] names
    /// for the caller to release; their second target stays.
    ///
    /// # Errors
    ///
    /// An action the slice does not hold, and what the Hunt's and the Gather's walks do not
    /// model.
    pub(crate) fn head_takes_point(
        &mut self,
        point: [f32; 3],
        player: usize,
    ) -> Result<PointMove, GenieError> {
        if self.unmodelled_action.is_some() {
            return Err(GenieError::unsupported(
                "slot 0x40 of an action the slice does not hold",
            ));
        }
        match self.top_action_kind() {
            None
            | Some(
                crate::move_to::MOVE_TO
                | crate::explore::EXPLORE
                | BUILD
                | crate::attack::ATTACK
                | crate::garrison::ENTER
                | crate::fly::FLY
                | crate::missile::MISSILE
                | crate::make::MAKE
                | crate::wait_solidify::WAIT_SOLIDIFY,
            ) => Ok(PointMove::Declined),
            Some(crate::artifact::ARTIFACT_TYPE) => {
                let walked = self.artifact.as_mut().is_some_and(|a| a.take_point(point));
                if walked {
                    self.mover
                        .as_mut()
                        .ok_or(GenieError::unsupported("a walker without movement state"))?
                        .stop()?;
                }
                Ok(PointMove::Taken(None))
            }
            Some(crate::hunt::HUNT) => {
                let (held, my_pasture) = (self.life.amount, self.my_pasture);
                let (Some(hunt), Some(mover)) = (self.hunt.as_mut(), self.mover.as_mut()) else {
                    return Err(GenieError::unsupported(
                        "a hunter without its hunt or movement state",
                    ));
                };
                let old = hunt.target;
                if !hunt.take_move(None, point, player, held, false, my_pasture, 0.0, mover)? {
                    return Ok(PointMove::Declined);
                }
                Ok(PointMove::Taken(Some(old).filter(|&t| t != -1)))
            }
            Some(crate::gather::GATHER) => {
                let held = self.life.amount;
                let (Some(gather), Some(mover)) = (self.gather.as_mut(), self.mover.as_mut())
                else {
                    return Err(GenieError::unsupported(
                        "a gatherer without its gather or movement state",
                    ));
                };
                let old = gather.target;
                gather.take_move(None, point, player, held, mover)?;
                Ok(PointMove::Taken(Some(old).filter(|&t| t != -1)))
            }
            Some(other) => {
                unreachable!("a top action of type {other}, which top_action never holds")
            }
        }
    }

    /// The unit's slot 0x348 (`FUN_141086440`), forced, on its unit AI's move to an object (in
    /// `state`): a `MoveTo` to the target within `range`, showing the master's running sprite (the
    /// walking one without it), takes the target it holds (`FUN_140aa0df0`, quiet 0: slot 0x178,
    /// [`crate::move_to::holds_object`]), then replaces its action list (`FUN_140a98620`,
    /// [`Unit::clear_own_actions`]: the old actions' teardowns let their targets go) and starts
    /// ([`MoveToAction::to_object`]).
    pub(in crate::turn) fn move_to_object(
        &mut self,
        (target, state): (i32, u8),
        range: f32,
        c: &mut Context<'_>,
    ) -> Result<(), GenieError> {
        if self.unmodelled_action.is_some() {
            return Err(GenieError::unsupported(
                "an AI move to an object of a unit with an action",
            ));
        }
        if self.mover.is_none() {
            return Err(GenieError::unsupported(
                "an AI move to an object of a unit that does not move",
            ));
        }
        let livestock = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .is_some_and(|m| m.class == LIVESTOCK);
        let master = c
            .spatial
            .get_known(target)?
            .ok_or(crate::move_to::TARGET_LACKING)?
            .master;
        if crate::move_to::holds_object(livestock, master, state) {
            c.takes.push((target, self.releaser(c)));
        }
        let by = self.releaser(c);
        self.clear_own_actions(&mut c.releases, &mut c.builder_leaves, by)?;
        let mover = self.mover.as_mut().expect("tested above");
        let move_to = MoveToAction::to_object(
            c.spatial,
            self.position,
            mover,
            (target, state),
            range,
            (true, livestock),
        )?;
        if move_to.shows_standing() {
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
    ) -> Result<(), GenieError> {
        if self.sprite.is_some() {
            let standing = masters
                .and_then(|m| m.get(self.life.master_id))
                .map(|m| m.standing_sprite)
                .and_then(|id| graphics.get(&id).copied())
                .ok_or(GenieError::unsupported(
                    "an arrived walker without its standing sprite",
                ))?;
            crate::attack::set_sprite(&mut self.sprite, standing, sprite_rng);
        }
        if let Some(a) = self.anim_speed.as_mut() {
            *a = 0.0;
        }
        Ok(())
    }
}

/// What the head action's slot 0x40 did with a move to a point ([`Unit::head_takes_point`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PointMove {
    /// It answered 0 (an empty list, the base slot, a Hunt whose task has a work range): the
    /// caller's `MoveTo` replaces the actions.
    Declined,
    /// It answered 1: the action walks there itself. The target it let go (slot 0x68 with none),
    /// if any, which the caller releases.
    Taken(Option<i32>),
}

/// An unpacked trebuchet's class (`-0xe76` raw), whose slot 0x340 packs instead of moving.
const UNPACKED_TREBUCHET: i16 = 54;

/// The player's slot 0xf8 (`FUN_141616820`) with 1 and 1 on a unit's load: the resource and its
/// gathered total (166 to 168; gold's 49 and 169), which the caller adds after the update; the
/// statistics (slot 0xb8) and the local player's tracker (`FUN_14097daf0`) are not held. With 1
/// the resources would go to the allies too when the player shares (`sharing`, `+0x1426`,
/// `FUN_14165d040`): a sharing player with allies is not modelled.
pub(crate) fn give_load(
    (relations, sharing): (&[Vec<u8>], bool),
    player: usize,
    adds: &mut Vec<(i16, f32)>,
    kind: i16,
    amount: f32,
) -> Result<(), GenieError> {
    let own = relations.get(player).map_or(&[][..], |r| &r[..]);
    let allied = (1..own.len()).any(|q| q != player && crate::tasks::allied(own, player, q, true));
    if sharing && allied {
        return Err(GenieError::unsupported(
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
    /// The target's owner's masters, which its master is (the owner's when None).
    pub target_masters: Option<&'a Masters>,
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
    /// The unit group the unit was a member of (`+0x6d0`): the caller runs the group's removeUnit
    /// (`FUN_14102c160` with the building and 0, [`crate::turn::Turns::remove_unit`]) when the
    /// world holds it (`FUN_14152ff40`).
    pub left_group: Option<i32>,
}

/// The population's resource, which a cost check skips (`FUN_141618db0` with its fifth argument
/// 1).
pub(in crate::turn) const POPULATION: i16 = 4;

/// The livestock class (`-0xe72` raw).
pub(in crate::turn) const LIVESTOCK: i16 = 58;

/// The siege class (`-0xe9f` raw), whose ownership change drops the units it carries
/// (`FUN_14120f800`).
pub(in crate::turn) const SIEGE_CLASS: i16 = 13;

/// The ownership changes the slice does not model: a building's (its slot 0x70,
/// `FUN_1411cd860`: its annexes', its production queue's and researches', its garrison's, its
/// object and technology on capture, then the combat object's), which a takeover by sight and
/// the artifact's ask for one reach, and a siege weapon's with units inside (`FUN_14120f800`'s
/// class 13 branch: `FUN_141192d10`, each unit's leave, slot 0x1a8, with the siege weapon's slot
/// 0x1b0, and the rest destroyed, slot 0xd0).
pub(in crate::turn) const HANDS_LEFT: GenieError = GenieError::unsupported(
    "an ownership change of a building or of a siege weapon with units inside",
);

/// The trade action's type (slot 0x438), whose load keeps a captured unit's unit AI going
/// (`FUN_14108c050`).
pub(in crate::turn) const TRADE_ACTION: i32 = 0x6f;

/// The unit AI's action that a capture's stop leaves alone (`FUN_14108c050`).
pub(in crate::turn) const CAPTURE_KEEPS_ACTION: i32 = 0x2c2;

/// `FUN_140aab570`'s scan after a livestock's capture: Gaia's livestock in state 2 whose tile
/// the new owner sees (`FUN_141663500`), within 3 tiles of `at` (x outer, y inner); each is taken
/// in turn, with its own scan. Two on one tile would need the tile's object list order.
pub(in crate::turn) fn scan_livestock(
    c: &mut Context<'_>,
    others: &mut [Unit],
    at: [f32; 3],
    new: usize,
    taken: &mut Vec<i32>,
) -> Result<(), GenieError> {
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
                        unit.capture_stop(c, LIVESTOCK)?;
                        unit.change_owner(c, new)
                    })();
                    let (id, position) = (unit.id, unit.position);
                    others[k] = unit;
                    // without the master the animal stays Gaia's, which scans nothing
                    if done? {
                        taken.push(id);
                        scan_livestock(c, others, position, new, taken)?;
                    }
                }
                _ => {
                    return Err(GenieError::unsupported(
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
pub(in crate::turn) enum Step {
    Stay,
    /// It leaves its list for good (`RGE_Object_List::update` deletes it).
    Leave,
    /// It moves to its player's sleeping list.
    Sleep,
    /// A player took it or other objects of its player over ([`Captured`]).
    Captured(Captured),
}

/// What a takeover in an object's update did ([`Step::Captured`]): player `new` took the object
/// (`main`; without the master it stays) and then `also`, other objects of its player, in order.
/// Those taken move to the end of the new owner's list; their storage and class counters move
/// with them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::turn) struct Captured {
    pub(in crate::turn) new: usize,
    pub(in crate::turn) main: bool,
    pub(in crate::turn) also: Vec<i32>,
}

/// What a unit's update hands its unit AI part when that runs after the caller gave the work its
/// builder found ([`Unit::ai_part`]): a takeover and the step the update ends with; or, with
/// `after`, the rest of the unit AI's full update once the caller gave the order on a target
/// the update took ([`crate::unit_ai::UnitAi::after_order`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::turn) struct AiResume {
    pub(in crate::turn) captured: Option<Captured>,
    pub(in crate::turn) leaves: Step,
    pub(in crate::turn) after: Option<crate::unit_ai::AfterOrder>,
}

/// What a ranged unit's fire reads of its target ([`Unit::fire`]): its id, place, master's radii
/// and class, and player.
pub(in crate::turn) struct Aimed {
    pub(in crate::turn) target: i32,
    pub(in crate::turn) position: [f32; 3],
    pub(in crate::turn) radius: [f32; 3],
    pub(in crate::turn) class: i16,
    pub(in crate::turn) player: usize,
}

/// `FUN_14120ea30` of an attacker of `master` with `firepower` (`+0xa00`): a building can attack
/// when of class 51 or 54, with a missile, or with a firepower above 1; any other attacker can.
pub(in crate::turn) fn armed(master: &Master, firepower: f32) -> bool {
    master.kind != crate::order::BUILDING
        || matches!(master.class, 51 | 54)
        || master.missile != -1
        || ARMED_FIREPOWER < firepower
}

/// Whether two lists hold the same objects, in any order.
pub(in crate::turn) fn same_members(a: &[i32], b: &[i32]) -> bool {
    let (mut a, mut b) = (a.to_vec(), b.to_vec());
    a.sort_unstable();
    b.sort_unstable();
    a == b
}

/// What a missile was fired as, which its hits read while its own part is out of the unit: a
/// charge's projectile (`+0x99c`) and what it was fired from (`+0x998`).
#[derive(Clone, Copy, Debug)]
pub(in crate::turn) struct Fired {
    pub(in crate::turn) charge: bool,
    pub(in crate::turn) by: i32,
}

/// A missile [`Unit::launch`] makes: its master, where it starts, the point its action aims at,
/// its reach (`+0x990`), whether its action has the object as its shooter (else none), what its
/// aim reads of the firing object (or the missile) and the attack count the aim sees.
pub(in crate::turn) struct Shot {
    pub(in crate::turn) made: i16,
    pub(in crate::turn) at: [f32; 3],
    pub(in crate::turn) point: [f32; 3],
    pub(in crate::turn) reach: f32,
    pub(in crate::turn) shooter: bool,
    pub(in crate::turn) aimer: crate::missile::Aimer,
    pub(in crate::turn) attack_count: i32,
}
