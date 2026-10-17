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
use crate::attack::{Strike, Victim};
use crate::build::{BUILD, BuildAction, BuildTask, Construction, Site, Walk};
use crate::fly::Fly;
use crate::hunt::HuntAction;
use crate::lifecycle::{Class, Lifecycle, seconds};
use crate::masters::Masters;
use crate::move_to::MoveToAction;
use crate::movement::{Me, Mover, Planning, footprint_gap};
use crate::order::{Interact, MoveOrder};
use crate::pathing::Pathing;
use crate::production::{Production, Queued};
use crate::rng::Well512;
use crate::rng::{Stream, Streams};
use crate::spatial::{Placed, Spatial};
use crate::sprite::{ActiveSprite, Graphic};
use crate::tasks::{Actor, Diplomacy, DropSite, TaskTarget};
use crate::unit_ai::{AiMove, BUILD_ACTION, BUILD_ORDER, Surroundings, UnitAi, Unsupported};

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
    /// `+0x644`, `worker_num`: the actions that have the object as their target.
    pub workers: i32,
    /// The slice could not model one of its updates: it freezes (its place is known only within
    /// its speed since) while the rest of its player's objects go on.
    pub stopped: bool,
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
    /// The owner's attributes 211 and 212 (with more than 212 attributes, else 0): an attack's
    /// elevation bonus.
    elevation: [f32; 2],
    /// The target of the unit's attack, read before its update and written back after it.
    victim: Option<&'a mut Victim>,
    /// The game's setting at `+0x680` is 1 ([`Turns::mode_680`]).
    mode_1: bool,
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
    /// The target of its attack, if it attacks (a hunt's Attack sub-action).
    fn attack_target(&self) -> Option<i32> {
        self.hunt.and_then(|h| h.attack).map(|a| a.target)
    }

    /// The type of its top action, if it has one.
    pub fn top_action_kind(&self) -> Option<i16> {
        None.or(self.artifact.map(|_| crate::artifact::ARTIFACT_TYPE))
            .or(self.build.map(|_| crate::build::BUILD))
            .or(self.move_to.map(|_| crate::move_to::MOVE_TO))
            .or(self.fly.map(|_| crate::fly::FLY))
            .or(self.hunt.map(|_| crate::hunt::HUNT))
    }

    /// One update; true when the object leaves its list. `others` are the rest of the owner's
    /// objects, whose foundations a builder works on.
    fn update(&mut self, c: &mut Context<'_>, others: &mut [Unit]) -> Result<bool, Unsupported> {
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
        }
        let leaves = self.life.update(c.dt_seconds)?;
        // the animated object's update (0x141071880): its sprite's clock at its animation speed
        if let (Some(sprite), Some(speed)) = (self.sprite.as_mut(), self.anim_speed) {
            sprite.update(c.dt_seconds, speed);
        }
        if let Some(e) = self.unmodelled_action.filter(|_| self.life.state == 2) {
            return Err(e);
        }
        if let Some(production) = self.production.as_mut().filter(|_| self.life.state == 2) {
            production.update(c.dt_seconds)?;
        }
        if !self.life.is_alive() {
            // RGE_Action_Object::update deletes a dead object's action list
            self.artifact = None;
        }
        // The default action exists from tick 1 on, but its first update is in tick 2: fitted to
        // 16 checkpoints of idle players with livestock (workspace FD-008).
        if let Some(artifact) = self.artifact.as_mut().filter(|_| c.tick > 1) {
            artifact.update(c.dt);
        }
        let me = Me {
            id: self.id,
            position: self.position,
        };
        if let Some(build) = self.build.as_mut() {
            let site = others
                .iter_mut()
                .find(|u| u.id == build.target)
                .and_then(|u| {
                    Some(Site {
                        id: u.id,
                        life: &mut u.life,
                        construction: u.construction.as_mut()?,
                    })
                });
            build.update(c.walk(self.mover.as_mut(), me), site)?;
        }
        if let Some(move_to) = self.move_to.as_mut() {
            move_to.update(c.walk(self.mover.as_mut(), me))?;
        }
        if let Some(fly) = self.fly.as_mut() {
            fly.update(c.walk(self.mover.as_mut(), me))?;
        }
        if let Some(hunt) = self.hunt.as_mut() {
            let computer = c.computer;
            let master = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(Unsupported("a hunter of a master the slice does not have"))?;
            let elevation = c.elevation;
            let victim = c.victim.take().ok_or(Unsupported(
                "an attack's target the slice does not hold live",
            ))?;
            let (walk, sprite_rng) = c.walk_and_sprites(self.mover.as_mut(), me);
            let mut strike = Strike {
                sprite: &mut self.sprite,
                anim_speed: &mut self.anim_speed,
                attack_timer: &mut self.attack_timer,
                reload: self.reload_stinger + (master.reload - self.reload_powerup),
                weapons: &master.weapons,
                hit_chance: master.hit_chance,
                class: master.class,
                elevation,
                z: self.position[2],
                victim,
                sprite_rng,
            };
            hunt.update(walk, computer, &mut strike)?;
        }
        // the motion step may have put the unit onto a waypoint it reached
        if let Some(p) = c.spatial.get(self.id) {
            self.position = p.position;
        }
        let mut moved = None;
        if let (Some(dt), Some(ai)) = (c.turn, self.ai.as_mut().filter(|_| self.life.is_alive())) {
            if ai.current_order == -1 && ai.current_action == -1 && ai.current_target == -1 {
                let at = Surroundings {
                    position: self.position,
                    mode_1: c.mode_1,
                };
                moved =
                    ai.update_idle(dt, c.world_time, c.computer, &at, c.streams.get(Stream::Ai))?;
            } else if (self.build.is_some() || self.hunt.is_some()) && !self.life.inside {
                ai.update_working(dt, c.computer, c.streams.get(Stream::Ai))?;
            } else {
                return Err(Unsupported("unit is not idle"));
            }
        }
        if let Some(m) = moved {
            let class = c
                .masters
                .and_then(|ms| ms.get(self.life.master_id))
                .map(|ms| ms.class)
                .ok_or(Unsupported(
                    "an AI move of a master the slice does not have",
                ))?;
            self.ai_move(m, class)?;
        }
        Ok(leaves)
    }

    /// The unit's slot 0x340 (`FUN_141216c50`) on its unit AI's move: a unit of class 51 or 54
    /// (packing) is not modelled; any other runs `FUN_1410866e0`. Without `force`, a MoveTo to the
    /// same point stands; a current action's slot 0x40 may take the point (neither modelled, so a
    /// unit with an action is not). Otherwise a MoveTo to the point within the range
    /// (`FUN_140aa0d30`; its sprite, the master's walking or running one, is not held) replaces
    /// the action list (`FUN_140a98620`) and starts (slot 0x808, `FUN_14106e990`; its start
    /// `0x140aa2100` gives a unit far from the point state 3).
    fn ai_move(&mut self, m: AiMove, class: i16) -> Result<(), Unsupported> {
        if matches!(class, 51 | 54) {
            return Err(Unsupported("an AI move of a unit of class 51 or 54"));
        }
        if self.top_action_kind().is_some() || self.unmodelled_action.is_some() {
            return Err(Unsupported(
                "an AI move of a unit with an action (its slot 0x40)",
            ));
        }
        self.move_to = Some(MoveToAction {
            state: crate::move_to::MoveTo::Start,
            goal: Some(m.point),
            range: m.range,
        });
        Ok(())
    }
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
    /// A command the slice could not model stopped the whole player: its objects freeze.
    pub stopped: bool,
}

impl Turns {
    /// What an attack reads of its target ([`Victim`]): a live object the slice holds.
    fn victim(&self, id: i32) -> Option<Victim> {
        let (p, u) =
            self.players.iter().enumerate().find_map(|(p, player)| {
                player.objects.iter().find(|u| u.id == id).map(|u| (p, u))
            })?;
        let kind = self.masters.get(p)?.get(u.life.master_id)?.kind;
        let attributes = &self.players[p].attributes;
        let at = |i: usize| attributes.get(i).copied().unwrap_or(0.0);
        Some(Victim {
            id,
            hp: u.life.hp,
            amount: u.life.amount,
            state: u.life.state,
            z: u.position[2],
            kind,
            goto_sleep: u.life.goto_sleep,
            elevation: [at(272), at(273)],
        })
    }

    /// An attack's hit on its target, written back.
    fn set_victim(&mut self, v: &Victim) {
        let found = self
            .players
            .iter_mut()
            .flat_map(|p| p.objects.iter_mut())
            .find(|u| u.id == v.id);
        if let Some(u) = found {
            u.life.hp = v.hp;
            u.life.amount = v.amount;
            u.life.goto_sleep = v.goto_sleep;
        }
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
                    t.map(|t| UnitAi::constructed(t, self.streams.get(Stream::Ai)))
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
                        }),
                    work_rate_powerup: 0.0,
                    attackable: true,
                    pasture: -1,
                    my_pasture: -1,
                    annex_of: None,
                    hp_powerups: [0.0; 2],
                    hunt: None,
                    workers: 0,
                    stopped: false,
                    sprite: None,
                    anim_speed: None,
                    attack_timer: 0.0,
                    reload_stinger: 0.0,
                    reload_powerup: 0.0,
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
            Command::Order {
                player,
                ref units,
                order,
            } => {
                for &id in units {
                    let ai = self
                        .players
                        .iter_mut()
                        .flat_map(|p| p.objects.iter_mut())
                        .find(|o| o.id == id)
                        .and_then(|o| o.ai.as_mut());
                    if let Some(ai) = ai {
                        ai.reset_for_order(self.streams.get(Stream::Ai));
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
            chosen.push((id, with, BuildAction::start(foundation, gap, task)));
        }
        for _ in &chosen {
            // the Build action takes its foundation (FUN_140a7a230, slot 0x178)
            self.take(foundation);
        }
        let owner = &mut self.players[player];
        for (id, with, build) in chosen {
            let unit = owner
                .objects
                .iter_mut()
                .find(|o| o.id == id)
                .expect("found above");
            unit.life.master_id = with;
            unit.build = Some(build);
            // the work command's notify (slot 0x828): the build order and the foundation, with
            // the point slot 0x128 passes (BUILD's x and y, on the ground)
            if let Some(ai) = unit.ai.as_mut() {
                ai.notify_work(
                    BUILD_ORDER,
                    BUILD_ACTION,
                    Some((foundation, site_class)),
                    [position[0], position[1], 0.0],
                    self.streams.get(Stream::Ai),
                );
            }
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

    /// One world update of `dt` game milliseconds, with the commands recorded in it.
    pub fn tick_with(&mut self, dt: u32, commands: &[Command]) {
        self.ticks += 1;
        self.world_time = self.world_time.wrapping_add(dt);
        self.streams.seed_all(self.frame_seed);
        for command in commands {
            self.run(command);
        }
        for p in 0..self.players.len() {
            self.player_time[p] = self.player_time[p].wrapping_add(dt);
            let turn = (p == self.player_turn).then_some(self.player_time[p]);
            let attributes = &self.players[p].attributes;
            let elevation = if 212 < attributes.len() {
                [attributes[211], attributes[212]]
            } else {
                [0.0; 2]
            };
            self.pathing.tiles_changed(self.spatial.take_changes());
            // An object whose update is not modelled freezes, and so do all live objects of a
            // player a command stopped; dead objects still run their state steps, which draw
            // nothing and depend on nothing the stop changed.
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
                let mut victim = unit.attack_target().and_then(|t| self.victim(t));
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
                    elevation,
                    victim: victim.as_mut(),
                    mode_1: self.mode_680 == 1,
                };
                let result = unit.update(&mut c, &mut player.objects);
                player.objects[i] = unit;
                if let Some(v) = victim {
                    self.set_victim(&v);
                }
                self.pathing.tiles_changed(self.spatial.take_changes());
                let player = &mut self.players[p];
                match result {
                    Ok(true) => {
                        let gone = player.objects.swap_remove(i);
                        self.spatial.remove(gone.id);
                        self.pathing.removed(gone.id);
                        self.pathing.tiles_changed(self.spatial.take_changes());
                    }
                    Ok(false) => i += 1,
                    Err(e) => {
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
        odd.ai_type = 2;
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
        let first = UnitAi::constructed(17, ai).unwrap();
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

    fn one_player(objects: Vec<Unit>, resources: [f32; 4]) -> Turns {
        Turns::new(
            0,
            29910,
            0,
            vec![0],
            vec![Player {
                objects,
                resources,
                ..Player::default()
            }],
        )
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

    #[test]
    fn a_unit_due_to_spawn_stops_the_player() {
        let mut t = one_player(vec![town_centre(9)], [200.0; 4]);
        let quick = Command::Queue {
            player: 0,
            building: 9,
            unit: 4,
            amount: 1,
            train_time: 2.0,
            rate: 1.0,
            cost: vec![],
        };
        t.tick_with(1000, &[quick]);
        assert!(t.players[0].unsupported.is_none());
        t.tick(1000);
        assert!(t.players[0].unsupported.is_some());
    }
}
