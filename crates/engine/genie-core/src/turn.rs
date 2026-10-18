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
use crate::masters::Masters;
use crate::move_to::MoveToAction;
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
    OWNER_CHANGED, Search, Surroundings, UnitAi, Unsupported,
};
use crate::visibility::{BuildingSight, Seer, capture_kind};

/// What the slice tracks of one object.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Unit {
    /// The object's id (`+0x8`), which names it across ticks.
    pub id: i32,
    /// x, y, z (`+0x3c8`, `+0x4c8`, `+0x5a8`, obfuscated in the game's memory).
    pub position: [f32; 3],
    pub life: Lifecycle,
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
    /// A MoveTo as the object's own top action (a wandering animal).
    pub move_to: Option<MoveToAction>,
    /// A bird's Fly action, its top action.
    pub fly: Option<Fly>,
    /// A building's construction.
    pub construction: Option<Construction>,
    /// `+0xa30`, `mWorkRatePowerup`: what the unit adds to its master's work rate.
    pub work_rate_powerup: f32,
    /// `+0x68a`, `unit_attackable_flag`.
    pub attackable: bool,
    /// `+0x790`, `parent_pasture_id`: the pasture it is in; `+0x794`, `my_pasture_id`: the one it
    /// tends. -1 for none.
    pub pasture: i32,
    pub my_pasture: i32,
    /// An annex's main building (`+0xbb8`, the save's linked owner).
    pub annex_of: Option<i32>,
    /// `mHitpointsPowerup` and `mHitpointsMaxPowerup` (`+0xa38`, `+0xa3c`), which the task finder
    /// adds to the hit points.
    pub hp_powerups: [f32; 2],
    /// A villager's Hunt action, its top action.
    pub hunt: Option<HuntAction>,
    /// Its active sprite and animation speed (`+0x7b8`, the save's `current_speed`), held from
    /// an attack's swing on; None while the slice does not know them.
    pub sprite: Option<ActiveSprite>,
    pub anim_speed: Option<f32>,
    /// `+0x91c`, `attack_timer`: the time until it may attack again; `mReloadStinger` and
    /// `mReloadPowerup` (`+0xb40`, `+0xa28`), which change its reload time.
    pub attack_timer: f32,
    pub reload_stinger: f32,
    pub reload_powerup: f32,
    /// What a hit reads of its unit and its target besides the masters: `+0xa24`
    /// (`mAttackPowerup`, the attacker's slot 0x6d8), the armor powerups (`+0xa48` melee,
    /// `+0xa4c` pierce, slots 0x708 and 0x710; whole numbers), the armor attacks removed
    /// (`+0x9e4`, `+0x9e6`), the charge (`+0x9e0`) and `mHitSomethingValidForChargeAttack`
    /// (`+0x8a8`: its last hit was its charge's).
    pub attack_powerup: f32,
    pub armor_powerups: [i16; 2],
    pub armor_removed: [i16; 2],
    pub charge: f32,
    pub charge_hit: bool,
    /// `+0x644`, `worker_num`: the actions that have the object as their target.
    pub workers: i32,
    /// `+0x650`, `mLastHumanOrderTime`: the world time of the last order a human gave it
    /// (`FUN_140e5e780`, for a unit with a unit AI); an animal's idle check lowers it to 5000.
    pub last_human_order: u32,
    /// The object is a combat object (types 50 and up): its update keeps its sight mask and its
    /// records in the objects-in-sight registry.
    pub combat: bool,
    /// What its master's default task would start once its action list is empty, when the
    /// slice does not model it (None: nothing, as a wild animal's Explore).
    pub default_task: Option<Unsupported>,
    /// The slice could not model one of its updates: it freezes (its place is known only within
    /// its speed since) while the rest of its player's objects go on.
    pub stopped: bool,
}

/// A building's gather point (`+0xc30`..`+0xc44`: it exists, x, y, z, the target and the target's
/// master; the save's `gather_point_*`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GatherPoint {
    pub point: [f32; 3],
    /// The object it is on, −1 for none.
    pub target: i32,
    pub target_master: i16,
}

/// A recorded command, run in the command phase of its tick: after the frame seed's SeedAll, before
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
        player: usize,
        master: i16,
        unit_ai: Result<Option<u32>, Unsupported>,
        position: [f32; 3],
        builders: Vec<i32>,
    },
    /// SET GATHER POINT (0x78, `FUN_140e7e9a0`): each of the player's objects of `ids` that takes
    /// a gather point (its master garrisons, `+0x5d4`; the list of units it trains,
    /// `FUN_141625a20`, is not modelled) takes `point` and `target` with its master, or with
    /// `own` itself (`FUN_1411c7ff0`); a target of −1 at (−1, −1) clears it. The ids of other
    /// players' objects are left out (the game setting `+0x1074` that lets them is not modelled),
    /// and the unit AI's command queue (`+0x1f0`, which its slot 0x210 empties) is not held.
    GatherPoint {
        player: usize,
        ids: Vec<i32>,
        point: [f32; 2],
        target: i32,
        target_master: i16,
        own: bool,
    },
    /// QUEUE (0x81, the player's queue `FUN_141614450`, the building's `FUN_1411cbc40`): `amount`
    /// units of master `unit` at one building. Each paid `cost` entry `(resource, amount, paid)`
    /// comes off the player's resources per unit; a villager's master is drawn male or female from
    /// the Action stream. The make action then works `rate` seconds of `train_time` per second.
    Queue {
        player: usize,
        building: i32,
        unit: i16,
        amount: u16,
        train_time: f32,
        rate: f32,
        cost: Vec<(i16, i16, bool)>,
    },
    /// A command the slice runs part of or none, for a reason the decoder names: it stops the
    /// issuing player's lane.
    Unmodelled { player: usize, why: Unsupported },
    /// INTERACT (0x00), MOVE (0x03) or RETREAT (0x23): before the order runs, each listed unit's AI
    /// resets its idle timeout ([`UnitAi::reset_for_order`]), in list order, wherever the unit is.
    /// INTERACT's order then runs ([`Interact`]), and MOVE's ([`MoveOrder`]); RETREAT is not
    /// modelled.
    Order {
        player: usize,
        units: Vec<i32>,
        order: Option<Order>,
    },
}

/// What an order command gives its units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Order {
    Interact(Interact),
    Move(MoveOrder),
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

/// A queued villager's master: male when the 64-bit Action draw times 100 (wrapping) is below 50
/// modulo 100.
fn villager(draw: u64) -> i16 {
    if draw.wrapping_mul(100) % 100 < 50 {
        VILLAGER
    } else {
        VILLAGER_FEMALE
    }
}

/// What a tick's update of one object needs from the world.
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
    blasted: Vec<Victim>,
    /// The unit was killed in its update (a retaliation), which the caller handles after it.
    own_killed: Option<crate::damage::Kill>,
    /// What the unit's death leaves for the caller: the remains its dying sprite's end makes
    /// (`FUN_1411a4e90`: the master and the place) and the paid cost its owner takes back
    /// (attributes 98 and 164).
    remains: Option<(i16, [f32; 3])>,
    refund: Option<f32>,
    /// The pauses the unit's walk gave units it met ([`Mover::pauses`]).
    pauses: Vec<(i32, f32)>,
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
    /// The attributes the owner gains from the object's update (a building it completed), which
    /// the caller adds after it.
    attribute_adds: Vec<(i16, f32)>,
    /// The unit AI's order on a target (`0x2db`) the unit took from its queue, which the caller
    /// runs after the update ([`Turns::order_on_target`]).
    order_on_target: Option<crate::unit_ai::QueuedOrder>,
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
    fn walk<'b>(&'b mut self, mover: Option<&'b mut Mover>, me: Me) -> Walk<'b> {
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

impl Unit {
    /// The target of its hunt, which it attacks or gathers from.
    fn attack_target(&self) -> Option<i32> {
        self.hunt.map(|h| h.target)
    }

    /// What the walk reads of it when it meets it ([`crate::spatial::Gait`]).
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
        })
    }

    /// The type of its top action, if it has one.
    pub fn top_action_kind(&self) -> Option<i16> {
        self.top_action().map(|a| a.kind)
    }

    /// Its top action as the slice models it, type and state.
    pub fn top_action(&self) -> Option<TopAction> {
        let action = |kind, state| TopAction { kind, state };
        None.or(self
            .artifact
            .map(|a| action(crate::artifact::ARTIFACT_TYPE, a.state)))
            .or(self.build.map(|b| action(crate::build::BUILD, b.state)))
            .or(self
                .move_to
                .map(|m| action(crate::move_to::MOVE_TO, m.state.number())))
            .or(self.fly.map(|f| action(crate::fly::FLY, f.state)))
            .or(self.hunt.map(|h| action(crate::hunt::HUNT, h.state)))
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

    /// stopAction (slot 0x3c0, `FUN_14108c160`): the action list goes (`FUN_140a98620`; a
    /// MoveTo's teardown stops the unit) and a unit in state 2 takes its standing sprite (slot
    /// 0x68 with slot 0x798's, [`crate::attack::set_sprite`]) where the slice holds its sprite.
    /// A Build waiting without its foundation (state 3) goes with nothing to release
    /// (`FUN_140aadbf0`); other actions' teardowns are not modelled.
    fn stop_action(&mut self, c: &mut Context<'_>) -> Result<(), Unsupported> {
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
            || self.unmodelled_action.is_some()
        {
            return Err(Unsupported(
                "a unit AI's stop of an action list the slice does not model",
            ));
        }
        if self.move_to.take().is_some() {
            self.mover
                .as_mut()
                .ok_or(Unsupported("a walker without movement state"))?
                .stop()?;
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
            }
        }
        Ok(())
    }

    /// A stop since the last look cleared the animation speed (`+0x7b8`, [`Mover::speed_stopped`]).
    fn take_stopped_speed(&mut self) {
        if let Some(m) = self.mover.as_mut().filter(|m| m.speed_stopped) {
            m.speed_stopped = false;
            self.anim_speed = Some(0.0);
        }
    }

    /// The death's set_state(3) of a combat object of type 70 (TRIBE_Combat_Object's,
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
        if master.kind != 70 {
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

    /// A notify the unit posts to its own unit AI (slot 0x328).
    fn post_own(&mut self, kind: i32, p: [i32; 3]) -> Result<(), Unsupported> {
        let (id, state) = (self.id, self.life.state);
        match self.ai.as_mut() {
            Some(ai) => ai.post(Notify::own(id, kind, p), state),
            None => Ok(()),
        }
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

    /// One update, and whether the object stays in its list, leaves it or goes to sleep. `others`
    /// are the rest of the owner's objects, whose foundations a builder works on.
    fn update(&mut self, c: &mut Context<'_>, others: &mut [Unit]) -> Result<Step, Unsupported> {
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
        self.take_stopped_speed();
        // RGE_Combat_Object::update (0x14107dfb0): the attack timer runs down by dt times
        // `+0x87c` (1.0 for every unit the slice has met; its writer is not ported)
        if 0.0 < self.attack_timer {
            self.attack_timer -= c.dt_seconds;
            if self.attack_timer < 0.0 {
                self.attack_timer = 0.0;
            }
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
                c.remains = Some((self.life.remains, self.position));
            }
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
        }
        // RGE_Action_Object::update with no action in state 2 starts the master's default task
        // (slot 0x7f0)
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
            artifact.walk(Some(c.walk(self.mover.as_mut(), me)))?;
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
            if let Some(new) = new {
                captured = Some((new, self.capture(c, others, new)?));
            }
        }
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
            let civ = c.civ;
            let more_work = built.is_some_and(|b| {
                c.drop_sites.iter().any(|e| {
                    e.building == b
                        && e.worker_class == class
                        && (e.civs.is_empty() || e.civs.contains(&civ))
                })
            });
            let builder = Builder {
                class,
                held: self.life.amount,
                ai: self.ai.as_mut(),
                more_work,
            };
            let step = build.update(c.walk(self.mover.as_mut(), me), site, builder)?;
            if let Some(adds) = step.completed {
                c.attribute_adds.extend(adds);
            }
            if step.done {
                // the action list removes it; its destructor (FUN_140aadbf0) finds it out of the
                // builders (set_state left them) and releases the foundation (slot 0x180)
                self.build = None;
                release_own(others, masters, target)?;
            }
            if step.released {
                release_own(others, masters, target)?;
                self.post_own(BUILD_IDLE, [BUILD_ACTION, 0, 0])?;
            }
        }
        if let Some(move_to) = self.move_to.as_mut()
            && move_to.update(c.walk(self.mover.as_mut(), me))?
        {
            // its update in state 1 posts its completion (0x1fa with the move action value; an
            // own MoveTo is never quiet), then the action list removes it (FUN_140a98e20): its
            // teardown (0x140aa1f30) stops the unit (slot 0x5d0)
            self.post_own(ACTION_COMPLETED, [crate::unit_ai::MOVE_ACTION, 0, 0])?;
            self.move_to = None;
            if let Some(m) = self.mover.as_mut() {
                m.stop()?;
            }
        }
        if let Some(fly) = self.fly.as_mut() {
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
        // the hunter as a combat object, which its hits read and change
        let own = self.hunt.as_ref().and_then(|_| {
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
        if let Some(hunt) = self.hunt.as_mut() {
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
                sprites,
            };
            // what set_state(6) on live livestock reads (0x140ac60e0 case 6)
            let herd = if hunt.state == crate::hunt::WALK
                && hunt.move_to == Some(crate::move_to::MoveTo::Arrived)
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
                let engagement =
                    crate::attack::engagement(self.ai.as_ref(), speed, master, me_at, them)?;
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
            let (walk, sprite_rng) = c.walk_and_sprites(self.mover.as_mut(), me);
            let mut strike = Strike {
                sprite: &mut self.sprite,
                anim_speed: &mut self.anim_speed,
                attack_timer: &mut self.attack_timer,
                reload: self.reload_stinger + (master.reload - self.reload_powerup),
                hit_chance: master.hit_chance,
                victim,
                blasted: &mut blasted,
                sprite_rng,
                own,
                executed: None,
                can_move,
                stance,
            };
            hunt.update(walk, computer, &mut strike, &mut gather, herd)?;
            let (executed, own) = (strike.executed, std::mem::take(&mut strike.own));
            c.blasted = blasted;
            if let Some(target) = executed {
                self.post_own(ACTION_EXECUTED, [600, target, 0])?;
            }
            c.own_killed = self.hit_back(own)?;
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
            && let Some(p) = c.spatial.get(self.id).cloned()
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
        // FUN_140c68980 for a unit not inside another
        let runs = turn.is_some() || !self.life.inside;
        if let Some(orders) = self
            .ai
            .as_ref()
            .filter(|_| self.life.is_alive() && runs)
            .map(|ai| !ai.orders.is_empty())
        {
            if turn.is_some() && busy && self.life.inside {
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
                interruptible,
                task_0x99: master.is_some_and(|m| m.tasks.iter().any(|t| t.action_type == 0x99)),
                garrisoned: self.life.inside,
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
            };
            let me = self.id;
            let ai = self.ai.as_mut().expect("filtered above");
            let effects = match turn {
                Some(dt) => {
                    let spatial: &Spatial = c.spatial;
                    let mut closest = |s: &Search<'_>| spatial.closest_object(me, player, s);
                    ai.update(
                        dt,
                        c.world_time,
                        computer,
                        &at,
                        c.streams.get(Stream::Ai),
                        &mut closest,
                    )?
                }
                None => ai.off_turn(&at)?,
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
        others: &mut [Unit],
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
        if self.life.inside {
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
    /// (packing) is not modelled; any other runs `FUN_1410866e0`. Without `force`, a MoveTo to the
    /// same point (its `+0x38`..`+0x40`) stands. Then the current action's slot 0x40 may take the
    /// point: an Artifact's (`FUN_140aaca30`, [`Artifact::take_point`]; a MoveTo it walked goes and
    /// stops the unit) does, a MoveTo's (`FUN_140a78490`) never (other actions are not modelled).
    /// Otherwise a MoveTo to the point within the range (`FUN_140aa0d30`; its sprite, the master's
    /// walking or running one, is not held) replaces the action list (`FUN_140a98620`: the old
    /// MoveTo stops the unit) and starts (slot 0x808, `FUN_14106e990`; its start `0x140aa2100`
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
            state: crate::move_to::MoveTo::Start,
            goal: Some(m.point),
            range: m.range,
        });
        Ok(())
    }
}

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

/// Slot 0x180 (`Turns::release`) on an object of the unit's own live list, from inside the unit's
/// update: its worker count falls, and a static object no one works on any more that is alive is
/// flagged to sleep.
fn release_own(others: &mut [Unit], masters: Option<&Masters>, id: i32) -> Result<(), Unsupported> {
    let u = others.iter_mut().find(|u| u.id == id).ok_or(Unsupported(
        "a release of an object not in the unit's live list",
    ))?;
    let kind = masters
        .and_then(|m| m.get(u.life.master_id))
        .ok_or(Unsupported(
            "a released object of a master the slice does not have",
        ))?
        .kind;
    if 0 < u.workers {
        u.workers -= 1;
    }
    let alive = (u.life.state == 2 && 0.0 <= u.life.hp) || u.life.state == 6;
    if kind < 30 && u.workers == 0 && alive {
        u.life.goto_sleep = true;
    }
    Ok(())
}

/// One player's objects in live-list order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Player {
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
    /// The first update this slice could not model, with the tick (1-based) and object index.
    pub unsupported: Option<(u32, usize, Unsupported)>,
    /// Every object the slice stopped updating, with the tick and why.
    pub freezes: Vec<(u32, i32, Unsupported)>,
    /// A command the slice could not model stopped the whole player: its objects freeze.
    pub stopped: bool,
}

impl Turns {
    /// What a hit of an attacker of player `attacker` reads of the object `id` ([`Victim`]): a
    /// live object the slice holds, with its relations to the attacker's player and whether its
    /// place is a guess.
    fn victim(&self, id: i32, attacker: usize) -> Option<Victim> {
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
                self.victim(id, p).unwrap_or(Victim {
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
        match v.killed {
            Some(kill) => self.killed(q, j, kill),
            None => Ok(()),
        }
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
        const PEACEFUL: [i16; 26] = [
            1, 3, 5, 7, 8, 9, 10, 11, 14, 15, 16, 27, 29, 30, 31, 32, 33, 34, 38, 39, 40, 48, 52,
            58, 60, 63,
        ];
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
        if u.life.inside {
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
    pub frame_seed: u32,
    pub players: Vec<Vec<Unit>>,
    pub resources: Vec<[f32; 4]>,
    /// Objects the slice modelled but got wrong this tick: they take the game's state too.
    pub resync: Vec<i32>,
    /// The players' visible maps and the unified map, when the slice's differ.
    pub sight: Option<(Vec<crate::visibility::VisibleMap>, Vec<u64>)>,
}

impl Turns {
    /// The forced replay's step after a tick: the frame seed is the game's, and so is every
    /// object the slice did not model (it froze, or its player stopped) or got wrong (`resync`),
    /// and the players' sight where it differs: its unit is the game's
    /// (with what the save does not hold, a hunted animal's origin and a boar's flag, kept),
    /// placed where the game has it, and it runs again from the next tick; a frozen object the
    /// game no longer holds goes. A stopped player takes the game's resources and runs again.
    /// Returns what could not be forced (an object that changed hands or masters, or that the
    /// game made and the slice did not). Not part of a replay: a dev hook to see past a freeze.
    pub fn force(&mut self, game: &GameState) -> Vec<(usize, i32, Unsupported)> {
        let mut notes = Vec::new();
        self.frame_seed = game.frame_seed;
        if let (Some((maps, unified)), Some(sight)) =
            (game.sight.as_ref(), self.spatial.sight.as_mut())
        {
            sight.maps.clone_from(maps);
            sight.unified.clone_from(unified);
        }
        for p in 0..self.players.len() {
            let Some(theirs) = game.players.get(p) else {
                continue;
            };
            let stopped = self.players[p].stopped;
            if stopped {
                if let Some(r) = game.resources.get(p) {
                    self.players[p].resources = *r;
                }
                self.players[p].stopped = false;
                self.players[p].unsupported = None;
            }
            let mut i = 0;
            while i < self.players[p].objects.len() {
                let unit = &self.players[p].objects[i];
                let (id, frozen) = (
                    unit.id,
                    stopped || unit.stopped || game.resync.contains(&unit.id),
                );
                if !frozen || !unit.life.is_alive() {
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
                if let Some(placed) = self.spatial.get(id).cloned() {
                    let moving = forced.mover.as_ref().is_some_and(|m| m.is_moving());
                    self.pathing.moved(id, &placed, moving);
                }
                forced.stopped = false;
                self.players[p].objects[i] = forced;
                i += 1;
            }
            for g in theirs {
                let known = self
                    .players
                    .iter()
                    .any(|q| q.objects.iter().chain(&q.sleeping).any(|u| u.id == g.id));
                if !known {
                    notes.push((
                        p,
                        g.id,
                        Unsupported("an object the game made and the slice did not"),
                    ));
                }
            }
        }
        self.pathing.tiles_changed(self.spatial.take_changes());
        notes
    }
}

impl Player {
    /// A command the slice does not model for this player: the player stops.
    fn stop(&mut self, tick: u32, why: Unsupported) {
        self.unsupported.get_or_insert((tick, 0, why));
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
    pub world_time: u32,
    /// `world+0x7d0`.
    pub frame_seed: u32,
    /// `world+0x188`, `player_turn` in the save.
    pub player_turn: usize,
    /// `world+0x18c[p]`, the time each player accumulated since its last turn.
    pub player_time: Vec<u32>,
    pub players: Vec<Player>,
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
    /// The flag a new combat object's setup reads at `FUN_140982ff0()`'s `+0x921`: the unit's
    /// cost also counts in attribute 246. The tick oracle reads 1 in MarvinBlue/426048731 (the
    /// attribute grows by a villager's cost at its spawn); where the save holds it is not known.
    pub unit_cost_246: bool,
    pub(crate) streams: Streams,
}

/// The players' relations and the drop sites, as the task finder reads them.
pub(crate) struct World<'a> {
    pub(crate) players: &'a [Player],
    pub(crate) drop_sites: &'a [DropSite],
}

impl Diplomacy for World<'_> {
    /// `FUN_141663dc0`: one's own player counts; Gaia never does; else relation 0 does, and 4
    /// with the ally flag; relation 1 counts only under allied victory (`FUN_1415e8e10`), which
    /// the slice does not model.
    fn allied(&self, from: usize, to: usize, with_ally: bool) -> bool {
        if from == to {
            return true;
        }
        if to == 0 {
            return false;
        }
        match self.players.get(from).and_then(|p| p.relations.get(to)) {
            Some(0) => true,
            Some(4) => with_ally,
            _ => false,
        }
    }

    fn drops_at(&self, building: i16, actor: &Actor<'_>) -> bool {
        crate::tasks::drops_at(self.drop_sites, building, actor)
    }
}

impl Turns {
    pub fn new(
        world_time: u32,
        frame_seed: u32,
        player_turn: usize,
        player_time: Vec<u32>,
        players: Vec<Player>,
    ) -> Self {
        Turns {
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
            graphics: Default::default(),
            drop_sites: Vec::new(),
            carry_rule: false,
            mode_680: 0,
            unit_cost_246: false,
            streams: Streams::default(),
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
            } => {
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
                    turn.unsupported.get_or_insert((self.ticks, at, e));
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
                        }),
                    work_rate_powerup: 0.0,
                    attackable: true,
                    last_human_order: 0,
                    pasture: -1,
                    my_pasture: -1,
                    annex_of: None,
                    hp_powerups: [0.0; 2],
                    hunt: None,
                    workers: 0,
                    combat: true,
                    default_task: None,
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
                if let Err(e) = self.task_builders(player, id, master, position, builders) {
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
            Command::Unmodelled { player, why } => {
                let owner = &mut self.players[player];
                owner.stop(self.ticks, why);
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
                // +0x15: each unit with a unit AI takes the world time as its last human order
                // time (FUN_140e5e780), after the resets
                let human = match order {
                    Some(Order::Interact(o)) => o.human,
                    Some(Order::Move(o)) => o.human,
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
                    for &id in units {
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
                let result = match order {
                    Some(Order::Interact(o)) => self.interact(player, units, &o),
                    Some(Order::Move(o)) => self.move_order(player, units, &o),
                    None => Err(Unsupported("an order moves its units")),
                };
                if let Err(e) = result {
                    let owner = &mut self.players[player];
                    owner.stop(self.ticks, e);
                }
            }
        }
    }

    /// BUILD's part for its own player: the cost, paid when the foundation is placed, and per
    /// builder the work command on the foundation (slot 37, `FUN_14106ff70`): the master it builds
    /// with ([`Masters::work_master`]), then the Build action from that master's task.
    fn task_builders(
        &mut self,
        player: usize,
        foundation: i32,
        master: i16,
        position: [f32; 3],
        builders: &[i32],
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
        for &id in builders {
            let owner = &world.players[player];
            let unit = owner
                .objects
                .iter()
                .find(|o| o.id == id)
                .ok_or(Unsupported("a builder the slice does not have"))?;
            if unit.artifact.is_some()
                || unit.production.is_some()
                || unit.build.is_some()
                || unit.move_to.is_some()
                || unit.fly.is_some()
            {
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
            let sight = (builder.radius, builder.class, builder.line_of_sight);
            chosen.push((id, with, BuildAction::start(foundation, gap, task), sight));
        }
        for _ in &chosen {
            // the Build action takes its foundation (FUN_140a7a230, slot 0x178)
            self.take(foundation);
        }
        let owner = &mut self.players[player];
        for (id, with, build, (radius, class, los)) in chosen {
            let unit = owner
                .objects
                .iter_mut()
                .find(|o| o.id == id)
                .expect("found above");
            unit.life.master_id = with;
            self.spatial.sight_master(id, radius, class, with, los)?;
            unit.build = Some(build);
            // the work command's notify (slot 0x828): the build order and the foundation, with
            // the point slot 0x128 passes (BUILD's x and y, on the ground)
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
        }
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

    /// QUEUE at one building: per unit (the game recurses, so the last unit drawn is queued first)
    /// the villager's male or female draw, then the cost and the queue entry.
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
            for &(resource, amount, _) in cost.iter().filter(|c| c.2) {
                let slot = owner
                    .resources
                    .get_mut(resource as usize)
                    .ok_or(Unsupported("a cost in a resource the slice does not track"))?;
                if *slot < f32::from(amount) {
                    return Err(Unsupported("QUEUE the player cannot pay"));
                }
                *slot -= f32::from(amount);
            }
            let production = owner.objects[index]
                .production
                .get_or_insert_with(|| Production {
                    queue: Default::default(),
                    action: false,
                    progress: 0.0,
                    rate: q.rate,
                });
            production.queue.push_back(Queued {
                master,
                train_time: q.train_time,
            });
        }
        Ok(())
    }

    /// One world update of `dt` game milliseconds without commands.
    pub fn tick(&mut self, dt: u32) {
        self.tick_with(dt, &[]);
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
                let blasted = victim
                    .as_ref()
                    .map_or_else(Vec::new, |v| self.blasted(&unit, p, v.id));
                let player = &mut self.players[p];
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
                    blasted,
                    own_killed: None,
                    remains: None,
                    refund: None,
                    pauses: Vec::new(),
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
                    attribute_adds: Vec::new(),
                    order_on_target: None,
                };
                let mut result = unit.update(&mut c, &mut player.objects);
                let (adds, ordered) = (
                    std::mem::take(&mut c.attribute_adds),
                    c.order_on_target.take(),
                );
                let (blasted, own_killed) = (std::mem::take(&mut c.blasted), c.own_killed.take());
                let (remains, refund) = (c.remains.take(), c.refund.take());
                let pauses = std::mem::take(&mut c.pauses);
                if let Some(e) = self.spatial.take_sight_unsupported() {
                    result = result.and(Err(e));
                }
                self.spatial.set_gait(unit.id, unit.gait());
                player.objects[i] = unit;
                for (kind, amount) in adds {
                    if let Err(e) = self.players[p].add_attribute(kind, amount) {
                        result = result.and(Err(e));
                    }
                }
                // slot 0x5f0 on each unit the walk paused: its walked level's pause
                for (id, pause) in pauses {
                    self.pause(id, pause);
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
                if let Some((master, at)) = remains
                    && let Err(e) = self.remains(p, master, at)
                {
                    result = result.and(Err(e));
                }
                // TRIBE_Building_Object::update: the production after the combat update
                if matches!(result, Ok(Step::Stay))
                    && let Err(e) = self.produce(p, i, seconds(dt))
                {
                    result = Err(e);
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
                self.pathing.tiles_changed(self.spatial.take_changes());
                let player = &mut self.players[p];
                match result {
                    Ok(Step::Leave) => {
                        let gone = player.objects.swap_remove(i);
                        if let Err(e) = self.spatial.leave(gone.id) {
                            player.unsupported.get_or_insert((self.ticks, i, e));
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
                                failed.get_or_insert(e);
                            }
                            if matches!(flag, 1 | 2 | 32)
                                && let Err(e) = self.players[new].add_attribute(kind, amount)
                            {
                                failed.get_or_insert(e);
                            }
                        }
                        if let Some(e) = failed {
                            self.players[p]
                                .unsupported
                                .get_or_insert((self.ticks, i, e));
                        }
                        self.players[new].objects.extend(moved);
                        i += 1;
                    }
                    Ok(Step::Stay) => i += 1,
                    Err(e) => {
                        player.freezes.push((self.ticks, player.objects[i].id, e));
                        player.unsupported.get_or_insert((self.ticks, i, e));
                        player.objects[i].stopped = true;
                        self.spatial.freeze(player.objects[i].id);
                        i += 1;
                    }
                }
            }
        }
        self.spatial.grow_unknown(seconds(dt));
        let s1 = self.streams.get(Stream::World).rand15();
        self.streams.seed_all(s1);
        self.player_time[self.player_turn] = 0;
        self.player_turn = (self.player_turn + 1) % self.players.len();
        self.frame_seed = self.streams.get(Stream::World).rand15();
    }
}

/// What a hit reads of the object `u` of player `p` ([`Victim`]): its master's values and its
/// own, its player's elevation attributes, the terrain defense of its tile (`terrain`, the map's;
/// 1 for an object inside another, which has no tile) and the game's setting `mode_680`. Its
/// relations to an attacker (`allied`) and whether its place is a guess (`uncertain`) are the
/// caller's. The slice holds neither its unit AI's alert mode and attackers nor its facing.
pub(crate) fn victim_of(
    u: &Unit,
    master: &crate::masters::Master,
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
    let defense = if master.terrain_bonus < 0 || u.life.inside {
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
        storage: master.storage.first().map_or(0.0, |s| s.1),
        capacity: master.capacity,
        blast_defense: master.blast_defense,
        goto_sleep: u.life.goto_sleep,
        decay: u.life.master.map_or(0.0, |m| m.decay),
        player: p,
        elevation: [at(272), at(273)],
        attack_elevation,
        allied: [false; 2],
        uncertain: false,
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
        assert_eq!(
            with_dead.players[0].objects[0].ai,
            dead_building().ai,
            "the dead building's timers stay"
        );
        assert_eq!(
            with_dead.players[0].objects[1].ai,
            alone.players[0].objects[0].ai
        );
    }

    /// The masters of MarvinBlue/426048731's player 2 that its BUILD at tick 679 uses, shortened
    /// from build 158041's save and `.dat`.
    fn builder_masters() -> crate::masters::Masters {
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
                ..Master::default()
            },
        ])
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
        t.masters = vec![Default::default(), builder_masters()];
        t.tick_with(
            50,
            &[Command::Build {
                player: 1,
                master: 70,
                unit_ai: Ok(None),
                position: [30.0, 88.0, 0.0],
                builders: vec![9175, 9181],
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
                    Player::default(),
                    Player {
                        objects,
                        ..Player::default()
                    },
                ],
            );
            t.next_object_id = 100;
            t
        };
        let mut built = world(vec![live(2)]);
        built.tick_with(
            40,
            &[Command::Build {
                player: 0,
                master: 70,
                unit_ai: Ok(Some(17)),
                position: [0.0; 3],
                builders: vec![],
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
        assert!(built.players[0].unsupported.is_some());
        // the same tick without the BUILD, with three AI draws taken by hand after the reseed
        let mut by_hand = world(vec![live(2)]);
        let mut streams = Streams::default();
        streams.seed_all(29910);
        let ai = streams.get(Stream::Ai);
        let first = UnitAi::constructed(17, [0.0; 3], ai).unwrap();
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
        t.force(&game);
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
        let villager = |id| crate::masters::Master {
            id,
            class: 4,
            storage: vec![(4, -1.0, 2), (11, 1.0, 2)],
            ..crate::masters::Master::default()
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
    fn a_queue_the_player_cannot_pay_or_a_missing_building_stops_the_player() {
        let mut poor = one_player(vec![town_centre(9)], [40.0, 0.0, 0.0, 0.0]);
        poor.tick_with(104, &[queue(9, 83, 1)]);
        assert!(poor.players[0].unsupported.is_some());
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
            ai: Some(crate::unit_ai::UnitAi::default()),
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
        let master = crate::masters::Master {
            id: 705,
            class: 58,
            storage: vec![(16, 100.0, 1)],
            ..crate::masters::Master::default()
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
        t.masters = vec![Masters::new([crate::masters::Master {
            id: 109,
            garrison: 15,
            ..crate::masters::Master::default()
        }])];
        let set = |target, point, own| Command::GatherPoint {
            player: 0,
            ids: vec![9149],
            point,
            target,
            target_master: 705,
            own,
        };
        t.tick_with(25, &[set(9520, [107.2451, 46.075863], false)]);
        assert_eq!(t.players[0].unsupported, None);
        assert_eq!(
            t.players[0].objects[0].gather_point,
            Some(GatherPoint {
                point: [107.2451, 46.075863, 0.0],
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
