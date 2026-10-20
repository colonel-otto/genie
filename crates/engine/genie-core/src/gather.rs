//! The Gather action (`RGE_Action_Gather`, type 5, vtable `0x142ed4f28`): a forager sent to a
//! resource walks to it (state 4, a quiet `MoveTo`), faces it (6), gathers (7), takes the load to
//! a drop site (8) and walks back to the resource (4).
//!
//! 158041: constructor `FUN_140a88310`, start (slot 5) `0x140a8b3a0`, update (slot 6)
//! `0x140a8e1d0`, `set_state` (slot 16) `0x140a8cd40`; slot 0x40 `0x140a8b740`, slot 0x48
//! `0x140a903a0`.
//!
//! Not modelled yet: a farm's stops (`mNumberStops` above 1: the walk to the next stop's random
//! point, the time per stop), the searches (`set_state` 3, 0xf3), the walks to a point (11, 15),
//! the waits after a failed walk or a drop-off out of reach (the Action stream's `"update"`
//! draws) and a gather left without a target and a load.

use crate::attack::set_sprite;
use crate::damage::Victim;
use crate::hunt::{HuntSprites, HuntTask, walk_sprite};
use crate::move_to::{
    Goal, MoveTo, MoveWait, Walk, start_walk, start_walk_or_give_up, target_moved, walk_or_give_up,
};
use crate::movement::Mover;
use crate::rng::Well512;
use crate::sprite::ActiveSprite;
use crate::unit_ai::Unsupported;

/// The Gather action's type, which word 9 adds.
pub const GATHER: i16 = 5;

/// State 4: walking to the target.
pub const WALK: u8 = 4;
/// State 6: facing it.
pub const FACE: u8 = 6;
/// State 7: gathering from it.
pub const WORK: u8 = 7;
/// State 8: taking the load to its drop site.
pub const DROP: u8 = 8;

/// A `MoveTo` starts in place within this gap of its target (`0x140aa2100`).
const ARRIVED: f32 = 0.01;
/// Below this a target's or a load's remainder counts as nothing (`DAT_142eb5690`).
const REMAINDER: f32 = crate::image::gather::REMAINDER;
/// A target's last bit adds its master's first storage amount times this (`DAT_142eb1e9c`).
const LAST_BIT: f32 = crate::image::gather::LAST_BIT;
/// A drop-off reaches its drop site within this squared footprint gap (`DAT_14330eb8c`).
const DROP_REACH: f32 = crate::image::gather::DROP_REACH;
/// The farm class, whose gather the slice does not model yet.
pub const FARM_CLASS: i16 = 49;
/// The villagers' class, whose `MoveTo` waits once when the motion step gives up.
const VILLAGER_CLASS: i16 = 4;
/// The masters a gatherer may gather from while they hold nothing (the combat object's slot
/// 0x2b8, `FUN_141216390`: the farm, the rice farm and master 45).
const REGATHERED: [i16; 3] = [50, 1187, 45];
/// Masters the constructor and `set_state(6)` treat apart (357 and 1188, `FUN_140a8b140`).
const SPECIAL_TARGETS: [i16; 2] = [357, 1188];

/// The Gather action of a gatherer (`RGE_Action_Gather`, type 5), with the save's fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GatherAction {
    /// The target object (`+0x20`, its id at `+0x30`), −1 for none.
    pub target: i32,
    /// `+0x28` (its id at `+0x34`), the second target: the drop site the load goes to in state 8,
    /// −1 for none.
    pub carry_to: i32,
    /// The action state (`+0x18`), which word 9 adds.
    pub state: u8,
    /// `+0x38`..`+0x40`: where the target stood.
    pub point: [f32; 3],
    /// `+0x44`: the state's timer.
    pub timer: f32,
    /// `+0x48`: its task.
    pub task: HuntTask,
    /// `+0x68`, `mTargetType`: the target's class.
    pub target_type: i32,
    /// `+0x6c`, `mNumberStops`: 1, or a farm's stops.
    pub stops: u8,
    /// `+0x6d`, `mCurrentStop`.
    pub current_stop: u8,
    /// `+0x70`, `mTimePerStop`.
    pub time_per_stop: f32,
    /// `+0x74`, `mTimeAtCurrentStop`.
    pub time_at_stop: f32,
    /// `+0x78`, `+0x7c`: `mLastStopX`, `mLastStopY`.
    pub last_stop: [f32; 2],
    /// `+0x80`: the gathering started (`set_state(7)`); not saved.
    pub worked: bool,
    /// `+0x84`, `+0x88`: `mFarmX`, `mFarmY`, a farm's place.
    pub farm: [f32; 2],
    /// `+0x8c`, `+0x90`: `mGatherX`, `mGatherY`, where the gathering started (−1 while the
    /// game's farm setting holds them, [`Gatherer::farm_stops`]).
    pub gather_at: [f32; 2],
    /// The sub-action of states 4 and 8: a `MoveTo` to the target or the drop site within the
    /// task's range, quiet (its `+0x60` and `+0x61` set).
    pub move_to: Option<MoveTo>,
    /// Its `MoveTo`'s wait after the motion step gave up.
    pub move_wait: MoveWait,
}

/// What the gather reads of its target when it takes it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resource {
    /// The target.
    pub id: i32,
    /// Its place.
    pub position: [f32; 3],
    /// Its master's class.
    pub class: i16,
    /// Its master's id.
    pub master: i16,
    /// Its owner.
    pub player: usize,
    /// How many actions work on it (`+0x644`).
    pub workers: i32,
    /// The gatherer's master has its master (a building annex's main building's, `+0xbb8`) among
    /// its drop sites (`FUN_14109d4f0`).
    pub drop_site: bool,
    /// The footprint gap between the gatherer and it (`FUN_14118f680`), which the `MoveTo`'s start
    /// reads.
    pub gap: f32,
}

/// What the gathering reads of its unit and changes.
#[derive(Debug)]
pub struct Gatherer<'a> {
    /// `+0x308`: the resource the unit carries.
    pub held_kind: &'a mut i16,
    /// `+0x310`: how much.
    pub held: &'a mut f32,
    /// The master's work rate (`+0x798`) plus the unit's powerup (slot 0x6e8, `+0xa30`).
    pub work_rate: f32,
    /// The master's `attribute_max_amount` (`+0x61c`): what the unit carries at most.
    pub capacity: f32,
    /// The player's attribute the task's `attribute_type2` names, when above 0; else 1.
    pub productivity: f32,
    /// The sprites the gather shows.
    pub sprites: HuntSprites,
    /// The unit's sprite.
    pub sprite: &'a mut Option<ActiveSprite>,
    /// The Sprite stream.
    pub sprite_rng: &'a mut Well512,
    /// The unit's master's class: a villager's `MoveTo` waits once when the motion step gives up.
    pub class: i16,
    /// The unit AI's stance (`+0x1a9`), 0 without a unit AI.
    pub stance: u8,
    /// The world's time.
    pub world_time: u32,
    /// The game's setting at `FUN_140c87b90()`'s `+0x69` (farmers walk their farm's stops); with
    /// it `set_state(7)` leaves `mGatherX`, `mGatherY` as they are.
    pub farm_stops: bool,
    /// The notifies the walk posts to the unit AI (the units in its way).
    pub posted: Vec<(i32, [i32; 3])>,
}

/// What an update asks of the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// Nothing.
    Stay,
    /// `set_state(8)`: the drop site search, then the walk there
    /// ([`GatherAction::set_drop`]).
    Drop,
}

/// Food and the food kinds (0, 15 to 17): a gatherer keeps a load of one for another.
const fn food(kind: i16) -> bool {
    kind == 0 || (15 <= kind && kind <= 17)
}

impl GatherAction {
    /// The factory's constructor with a target (`FUN_140a88310`; the target's add, slot 0x178, is
    /// the caller's): the target's place is the point and its class the target type; a farm (class
    /// 49) or masters 357 and 1188 keep their place as the farm's. One stop, `mGatherX` and
    /// `mGatherY` at −1. A farm of master 50 or 1187 with the game's farm setting has four stops,
    /// which the slice does not model yet.
    ///
    /// # Errors
    ///
    /// The farm's stops.
    pub fn new(task: HuntTask, target: &Resource, farm_stops: bool) -> Result<Self, Unsupported> {
        let farm = if target.class == FARM_CLASS || SPECIAL_TARGETS.contains(&target.master) {
            [target.position[0], target.position[1]]
        } else {
            [0.0; 2]
        };
        if farm_stops && target.class == FARM_CLASS && matches!(target.master, 50 | 1187) {
            return Err(Unsupported("a farm's gather in stops (mNumberStops 4)"));
        }
        Ok(Self {
            target: target.id,
            carry_to: -1,
            state: 0,
            point: target.position,
            timer: 0.0,
            task,
            target_type: i32::from(target.class),
            stops: 1,
            current_stop: 0,
            time_per_stop: 0.0,
            time_at_stop: 0.0,
            last_stop: [0.0; 2],
            worked: false,
            farm,
            gather_at: [-1.0; 2],
            move_to: None,
            move_wait: MoveWait::default(),
        })
    }

    /// The install's start (slot 5 with 1, `0x140a8b3a0`): a gatherer carrying another food keeps
    /// the amount as the task's kind (`FUN_14119d310`); one carrying another resource drops it
    /// when the game settings' test `FUN_141302150` (`carry_rule`) says so or it is a computer
    /// player's carrying nothing. A loaded gatherer sent to one of its own drop sites carries the
    /// load there: the target becomes the second target (the caller takes it, then releases it as
    /// the first), the point clears and `set_state(8)` walks there. Else `set_state(4)` walks to
    /// the target. A farm another farmer works (`set_state` 11) is not modelled.
    ///
    /// # Errors
    ///
    /// What the start does not model.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &mut self,
        target: &Resource,
        player: usize,
        computer: bool,
        carry_rule: bool,
        held: &mut (i16, f32),
        mover: &mut Mover,
    ) -> Result<(), Unsupported> {
        let kind = self.task.resource;
        let both_food = food(kind) && food(held.0);
        let drops = computer && held.1 == 0.0;
        if (!carry_rule && !drops) || held.0 == kind || both_food {
            if held.0 != kind && both_food {
                held.0 = kind;
            }
        } else {
            *held = (kind, 0.0);
        }
        if self.target_type == i32::from(FARM_CLASS) && 1 < target.workers {
            return Err(Unsupported("a farm another farmer works (set_state 11)"));
        }
        if target.player == player && target.drop_site && 0.0 < held.1 {
            self.target = -1;
            self.point = [-1.0; 3];
            self.carry_to = target.id;
            return self.set_drop(target.gap, mover);
        }
        self.set_walk(target.position, target.gap, mover)
    }

    /// Slot 0x48 (`0x140a903a0`) on the work command's new `target` (one the world holds that is
    /// no building, or a farm; the caller's tests): its present target in state 6, 7 or 15, or in
    /// 4 or 11 for a computer player, it keeps as it is. A loaded gatherer (`held`) sent to one of
    /// its player's drop sites carries its load there: the timer clears, the target goes (slot
    /// 0x68 with none) with the point, the drop site becomes the second target (`FUN_140a7a190`)
    /// and `set_state(8)` walks there. Any other target becomes the target (slot 0x68) with its
    /// place as the point and its class as the target type, the second target goes and
    /// `set_state(4)` walks to it; the caller takes and lets go the targets that changed. With
    /// the game's farm setting a fish trap (master 199) keeps its place as the farm's. Always
    /// true: the gather took the target. Not modelled: a farm of master 50 or 1187 with the farm
    /// setting (its four stops) and a farm another farmer works (slots 0x80 and 0x68).
    ///
    /// # Errors
    ///
    /// What it does not model; a unit that cannot move.
    pub fn retarget(
        &mut self,
        target: &Resource,
        player: usize,
        computer: bool,
        held: f32,
        farm_stops: bool,
        mover: &mut Mover,
    ) -> Result<bool, Unsupported> {
        if target.id == self.target {
            match self.state {
                FACE | WORK | 15 => return Ok(true),
                WALK | 11 if computer => return Ok(true),
                _ => {}
            }
        }
        if target.player == player && target.drop_site && 0.0 < held {
            self.timer = 0.0;
            self.target = -1;
            self.point = [-1.0; 3];
            self.carry_to = target.id;
            self.set_drop(target.gap, mover)?;
            return Ok(true);
        }
        // slot 0x68 takes the target: its workers count it once it is new
        let workers = target.workers + i32::from(target.id != self.target);
        self.target = target.id;
        self.point = target.position;
        self.target_type = i32::from(target.class);
        if farm_stops {
            if matches!(target.master, 50 | 1187) {
                return Err(Unsupported("a gather retargeted to a farm in stops"));
            }
            if target.master == 199 {
                self.farm = [target.position[0], target.position[1]];
            }
        }
        if target.class == FARM_CLASS && 1 < workers {
            return Err(Unsupported(
                "a gather retargeted to a farm another farmer works",
            ));
        }
        self.carry_to = -1;
        self.set_walk(target.position, target.gap, mover)?;
        Ok(true)
    }

    /// `set_state(4)` (`0x140a8cd40` case 4) with its target at `position`, `gap` from the unit:
    /// the sub-actions go (a `MoveTo`'s teardown stops the unit), the timer clears and the target's
    /// place becomes the point; with one stop a quiet `MoveTo` to the target within the task's
    /// range starts (`FUN_140aa0df0`, its start `0x140aa2100`: a unit within 0.01 of the target's
    /// footprint is there already, state 1; else state 3).
    ///
    /// # Errors
    ///
    /// A farm's stops; a unit that cannot move (the `MoveTo`'s state 13).
    pub fn set_walk(
        &mut self,
        position: [f32; 3],
        gap: f32,
        mover: &mut Mover,
    ) -> Result<(), Unsupported> {
        self.tear_down(Some(mover))?;
        self.state = WALK;
        self.timer = 0.0;
        if self.target == -1 {
            return Err(Unsupported(
                "a gather's walk without its target (set_state 13)",
            ));
        }
        self.point = position;
        if 1 < self.stops {
            return Err(Unsupported("a farm's walk to its next stop (set_state 4)"));
        }
        self.start_move(gap, mover)
    }

    /// `set_state(8)` (`0x140a8cd40` case 8) with a load and its drop site (`carry_to`, the
    /// caller's search, `FUN_1411671e0`): the current stop clears, the sub-actions go and a quiet
    /// `MoveTo` to the drop site within the task's range starts (`FUN_140aa0df0`).
    ///
    /// # Errors
    ///
    /// No drop site (the unit AI's part); a unit that cannot move.
    pub fn set_drop(&mut self, gap: f32, mover: &mut Mover) -> Result<(), Unsupported> {
        self.tear_down(Some(mover))?;
        self.state = DROP;
        self.current_stop = 0;
        if self.carry_to == -1 {
            return Err(Unsupported(
                "a gather's drop-off without a drop site (notify 0x203)",
            ));
        }
        self.start_move(gap, mover)
    }

    /// A new quiet `MoveTo` within the task's range: arrived within 0.01 of its goal's footprint.
    fn start_move(&mut self, gap: f32, mover: &Mover) -> Result<(), Unsupported> {
        self.move_wait = MoveWait::default();
        self.move_to = Some(if ARRIVED < gap {
            if mover.stats.speed == 0.0 {
                return Err(Unsupported("a MoveTo of a unit without speed (state 13)"));
            }
            MoveTo::Start
        } else {
            MoveTo::Arrived
        });
        Ok(())
    }

    /// The teardown (`FUN_140a73fc0`) and `set_state`'s first step (`FUN_140a98620`): the
    /// `MoveTo` goes and stops the unit (slot 0x5d0); the caller releases the targets.
    ///
    /// # Errors
    ///
    /// The unit's stop that the motion step does not model.
    pub fn tear_down(&mut self, mover: Option<&mut Mover>) -> Result<(), Unsupported> {
        if self.move_to.take().is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        Ok(())
    }

    /// The sprite the walks show (`FUN_140a8b660`, slot 0x68 with it): the task's carrying sprite
    /// while the unit carries at least 1 of the task's resource, else its moving one.
    const fn walk_sprite(&self, g: &Gatherer<'_>) -> Option<crate::sprite::Graphic> {
        walk_sprite(self.task.resource, *g.held_kind, *g.held, &g.sprites)
    }

    /// One update (slot 6, `0x140a8e1d0`) in states 4, 6 and 7, `target` the target the caller
    /// holds (None once it went). State 4 walks the `MoveTo` (`FUN_140a992b0`); its end (codes 1
    /// and 2) is `set_state(6)`. State 6 faces the target (`FUN_141162db0`) and, while it holds
    /// something, `set_state(7)`. State 7 gathers ([`GatherAction::gather`]). Without a target a
    /// loaded gatherer takes its load off ([`Next::Drop`]).
    ///
    /// # Errors
    ///
    /// What the update does not model.
    pub fn update(
        &mut self,
        mut walk: Walk<'_>,
        target: Option<&mut Victim>,
        g: &mut Gatherer<'_>,
        dt: f32,
    ) -> Result<Next, Unsupported> {
        let Some(target) = target.filter(|_| self.target != -1) else {
            if matches!(self.state, WALK | FACE | WORK) {
                if 0.0 < *g.held {
                    return Ok(Next::Drop);
                }
                return Err(Unsupported(
                    "a gather without a target or a load (set_state 3)",
                ));
            }
            return Err(Unsupported(
                "a Gather action in a state the slice does not model",
            ));
        };
        match self.state {
            WALK if self.timer < 0.0 => Err(Unsupported("a gather's wait after a failed walk")),
            WALK => self.update_walk(walk, target, g).map(|()| Next::Stay),
            FACE => {
                if target.class == FARM_CLASS {
                    return Err(Unsupported("a gather facing a farm"));
                }
                // case 5: FUN_141162db0 turns the unit to face its target
                let at = walk
                    .world
                    .get(walk.me.id)
                    .map_or(walk.me.position, |p| p.position);
                let mover = walk
                    .mover
                    .as_deref_mut()
                    .ok_or(Unsupported("a gatherer without movement state"))?;
                if !crate::attack::face_target(mover, at, target.position, g.world_time)? {
                    return Err(Unsupported(
                        "a gatherer that turns over time to face its target",
                    ));
                }
                if 0.0 < target.amount || REGATHERED.contains(&target.master_id) {
                    self.set_work(walk.me.position, g);
                    self.time_at_stop = 0.0;
                    return Ok(Next::Stay);
                }
                Err(Unsupported(
                    "a gather's target empty when faced (set_state 3)",
                ))
            }
            WORK => self.gather(target, g, dt),
            _ => Err(Unsupported(
                "a Gather action in a state the slice does not model",
            )),
        }
    }

    /// State 4's `MoveTo` (case 3): it starts, walks, waits after the motion step gave up (a
    /// villager once) and ends: its update in state 1 returns 1 (the list deletes it and its
    /// destructor stops the unit), then `set_state(6)`. A failed `MoveTo` (code 3) is not
    /// modelled.
    fn update_walk(
        &mut self,
        mut walk: Walk<'_>,
        target: &Victim,
        g: &mut Gatherer<'_>,
    ) -> Result<(), Unsupported> {
        match self.move_to {
            Some(MoveTo::Start) => {
                if self.move_wait.state == 1 {
                    self.move_wait.state = 2;
                }
                self.move_to = Some(MoveTo::Moving);
                walk.me.action_state = self.state;
                let range = self.task.range;
                let walked =
                    start_walk_or_give_up(walk.reborrow(), Goal::Target(self.target), range, true)?;
                self.post_in_the_way(&mut walk, g);
                // the MoveTo's set_state(4) shows the walking sprite
                if walked.is_none_or(|w| w == MoveTo::Moving)
                    && let Some(s) = self.walk_sprite(g)
                {
                    set_sprite(g.sprite, s, g.sprite_rng);
                }
                match walked {
                    Some(state) => self.move_to = Some(state),
                    None => self.give_up(g),
                }
                Ok(())
            }
            Some(MoveTo::Moving) => {
                if target_moved(&walk, self.target, true)?.again() {
                    self.move_to = Some(MoveTo::Start);
                    return Ok(());
                }
                walk.me.action_state = self.state;
                let walked = walk_or_give_up(walk.reborrow())?;
                self.post_in_the_way(&mut walk, g);
                match walked {
                    Some(m) => self.move_to = Some(m),
                    None => self.give_up(g),
                }
                Ok(())
            }
            Some(MoveTo::Waiting) => {
                self.move_wait.timer += walk.dt;
                if self.move_wait.delay <= self.move_wait.timer {
                    self.move_wait.timer = 0.0;
                    self.move_to = Some(MoveTo::Start);
                }
                Ok(())
            }
            Some(MoveTo::Arrived) => {
                self.move_to = None;
                walk.mover
                    .as_deref_mut()
                    .ok_or(Unsupported("a gatherer without movement state"))?
                    .stop()?;
                self.set_face(target, g)
            }
            Some(MoveTo::Failed) => Err(Unsupported(
                "a gather's failed walk (its wait or its search, set_state 3)",
            )),
            None => Err(Unsupported("a Gather in state 4 without its MoveTo")),
        }
    }

    /// The units in the way the motion step told the unit AI of ([`crate::unit_ai::IN_THE_WAY`]).
    fn post_in_the_way(&self, walk: &mut Walk<'_>, g: &mut Gatherer<'_>) {
        if let Some(m) = walk.mover.as_deref_mut() {
            g.posted.extend(
                m.in_the_way
                    .drain(..)
                    .map(|id| (crate::unit_ai::IN_THE_WAY, [id, -1, -1])),
            );
        }
    }

    /// The `MoveTo`'s answer to the motion step giving up ([`MoveWait::give_up`]; quiet). The
    /// computer player's test is not held: its villagers would fail at once.
    const fn give_up(&mut self, g: &Gatherer<'_>) {
        let waits = g.class == VILLAGER_CLASS;
        self.move_to = Some(self.move_wait.give_up(waits, g.stance));
    }

    /// `set_state(6)` (case 6) on a target: a farm, masters 357 and 1188 and a target not yet
    /// built (state below 2, the unit's slot 0x380) are not modelled; the timer clears and the
    /// unit takes the walking sprite (`FUN_140a8b660`; slot 0x68 takes the standing one for
    /// none).
    fn set_face(&mut self, target: &Victim, g: &mut Gatherer<'_>) -> Result<(), Unsupported> {
        self.state = FACE;
        if target.class == FARM_CLASS || SPECIAL_TARGETS.contains(&target.master_id) {
            return Err(Unsupported("a gather's set_state 6 on a farm"));
        }
        if target.state < 2 {
            return Err(Unsupported(
                "a gather's target not built yet (the unit's slot 0x380)",
            ));
        }
        if (target.master_id - 0x765) & !4 == 0 {
            return Err(Unsupported("a gather's set_state 6 on master 1893 or 1897"));
        }
        self.timer = 0.0;
        if let Some(s) = self.walk_sprite(g).or(g.sprites.standing) {
            set_sprite(g.sprite, s, g.sprite_rng);
        }
        Ok(())
    }

    /// `set_state(7)` (case 7): the gathering starts (`+0x80`), the timer clears; without the
    /// game's farm setting the unit's place is `mGatherX`, `mGatherY`. A load of another resource
    /// is dropped unless both are food (`FUN_14119d310` with 0; then the unit keeps the kind it
    /// carries). The unit takes the working sprite (slot 0x68; the standing one for none); the
    /// gathering sound (slot 0x4c0) is not held.
    fn set_work(&mut self, at: [f32; 3], g: &mut Gatherer<'_>) {
        self.state = WORK;
        self.worked = true;
        self.timer = 0.0;
        if !g.farm_stops {
            self.gather_at = [at[0], at[1]];
        }
        let kind = self.task.resource;
        if *g.held_kind != kind && !(food(kind) && food(*g.held_kind)) {
            *g.held_kind = kind;
            *g.held = 0.0;
        }
        if let Some(s) = g.sprites.working.or(g.sprites.standing) {
            set_sprite(g.sprite, s, g.sprite_rng);
        }
    }

    /// State 7 (case 6) with one stop: the work, `(work rate) × work_val1 × dt`, at most what the
    /// target holds, times the productivity. A load that would pass the capacity ends the
    /// gathering ([`Next::Drop`]); else the target loses the work, its last bit (above 0, below
    /// 0.1) going with a thousandth of its master's first storage amount, the unit gains, and a
    /// remainder below 0.1 of the capacity rounds the load up. A full load, or an empty target
    /// that cannot be gathered empty (the combat object's slot 0x2b8), ends it. Not modelled: a
    /// farm's food sold at once (the player's attribute 96 on class 49 and food kind 16) and the
    /// game's `+0xc14` (`FUN_140c884c0`, 0 as the tick oracle holds it).
    fn gather(
        &self,
        target: &mut Victim,
        g: &mut Gatherer<'_>,
        dt: f32,
    ) -> Result<Next, Unsupported> {
        if 1 < self.stops {
            return Err(Unsupported("a farm's gathering in stops"));
        }
        let work = (g.work_rate * self.task.work_value) * dt;
        let take = if work <= target.amount {
            work
        } else {
            target.amount
        };
        let capacity = g.capacity;
        let mut gain = g.productivity * take;
        if capacity < gain + *g.held {
            return Ok(Next::Drop);
        }
        target.amount += -take;
        if target.amount < REMAINDER && 0.0 < target.amount {
            gain = gain + target.storage * LAST_BIT + target.amount;
            target.amount = 0.0;
        }
        *g.held += gain;
        let over = (*g.held - capacity).abs();
        if *g.held < capacity && over < REMAINDER && over <= target.amount {
            target.amount += -over;
            *g.held += over;
        }
        if target.class == FARM_CLASS && *g.held_kind == 16 {
            return Err(Unsupported(
                "a farm's food sold at once (the player's attribute 96)",
            ));
        }
        let empty = target.amount <= 0.0 && !REGATHERED.contains(&target.master_id);
        Ok(if over < REMAINDER || empty {
            Next::Drop
        } else {
            Next::Stay
        })
    }

    /// State 8's update (case 7): the `MoveTo` to the drop site walks; once it ended (its update
    /// in state 1 returns 1: the list deletes it and its destructor stops the unit) the load goes
    /// to the player at a drop site that takes it (`site`) within `DROP_REACH` (squared): the
    /// resource of what the unit carries ([`crate::hunt::resource_kind`]), which the caller adds;
    /// the unit carries nothing after (`FUN_14119d310` with −1). Returns the resource and the
    /// amount; the caller lets the drop site go and walks back to the target (`set_state(4)`).
    /// Not modelled: the timer's wait, a `MoveTo` that waits or failed (codes 3 to 5), a site that
    /// does not take the load (`set_state` 2) or is out of reach (the wait).
    ///
    /// # Errors
    ///
    /// What the drop-off does not model.
    pub fn update_drop(
        &mut self,
        mut walk: Walk<'_>,
        g: &mut Gatherer<'_>,
        attributes: usize,
        site: Option<crate::hunt::DropOff>,
    ) -> Result<Option<(i16, f32)>, Unsupported> {
        if self.timer < 0.0 {
            return Err(Unsupported("a gather's drop-off waiting to try again"));
        }
        match self.move_to {
            Some(MoveTo::Start) => {
                self.move_to = Some(MoveTo::Moving);
                walk.me.action_state = self.state;
                let range = self.task.range;
                let state = start_walk(walk.reborrow(), Goal::Target(self.carry_to), range, true)?;
                if state == MoveTo::Moving
                    && let Some(s) = self.walk_sprite(g)
                {
                    set_sprite(g.sprite, s, g.sprite_rng);
                }
                self.move_to = Some(state);
                return Ok(None);
            }
            Some(MoveTo::Moving) => {
                if target_moved(&walk, self.carry_to, true)?.again() {
                    self.move_to = Some(MoveTo::Start);
                    return Ok(None);
                }
                walk.me.action_state = self.state;
                self.move_to = Some(walk_or_give_up(walk.reborrow())?.ok_or(Unsupported(
                    "a gather's drop-off walk the motion step gave up",
                ))?);
                self.post_in_the_way(&mut walk, g);
                return Ok(None);
            }
            Some(MoveTo::Arrived) => {
                self.move_to = None;
                walk.mover
                    .as_deref_mut()
                    .ok_or(Unsupported("a gatherer without movement state"))?
                    .stop()?;
            }
            Some(MoveTo::Waiting | MoveTo::Failed) => {
                return Err(Unsupported(
                    "a gather's drop-off whose MoveTo waits or failed",
                ));
            }
            None => return Err(Unsupported("a Gather in state 8 without its MoveTo")),
        }
        let site = site.ok_or(Unsupported("a gather's drop-off whose drop site is gone"))?;
        if !site.takes {
            return Err(Unsupported(
                "a drop site that does not take the gather's load (set_state 2)",
            ));
        }
        if DROP_REACH < site.gap_squared {
            return Err(Unsupported(
                "a gather's drop-off out of reach of its drop site (its wait)",
            ));
        }
        let kind = crate::hunt::resource_kind(*g.held_kind, attributes);
        let amount = *g.held;
        *g.held_kind = -1;
        *g.held = 0.0;
        Ok(Some((kind, amount)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A forager's task: food (kind 17, berries), range 0.25, work value 1 (the 4v4's 31139).
    fn task() -> HuntTask {
        HuntTask {
            id: 2,
            range: 0.25,
            resource: 17,
            sprites: [-1; 4],
            work_value: 1.0,
            productivity: -1,
            kind4: -1,
        }
    }

    /// The 4v4's berry bush 28222 (master 59, class 7) as the builder 31139 takes it in update
    /// 4863.
    fn bush() -> Resource {
        Resource {
            id: 28222,
            position: [23.5, 93.5, 0.0],
            class: 7,
            master: 59,
            player: 0,
            workers: 0,
            drop_site: false,
            gap: 1.2,
        }
    }

    fn target(amount: f32) -> Victim {
        Victim {
            id: 28222,
            amount,
            state: 2,
            class: 7,
            master_id: 59,
            storage: 125.0,
            ..Victim::default()
        }
    }

    #[test]
    fn a_forager_walks_to_its_bush() {
        let mut mover = crate::hunt::tests::walker();
        let mut g = GatherAction::new(task(), &bush(), true).unwrap();
        assert_eq!((g.stops, g.gather_at, g.target_type), (1, [-1.0; 2], 7));
        let mut held = (2, 0.0);
        g.start(&bush(), 7, false, false, &mut held, &mut mover)
            .unwrap();
        assert_eq!(
            (g.state, g.move_to, held),
            (WALK, Some(MoveTo::Start), (2, 0.0))
        );
        // a load of another food becomes the task's kind
        let mut g = GatherAction::new(task(), &bush(), true).unwrap();
        let mut held = (15, 3.0);
        g.start(&bush(), 7, false, false, &mut held, &mut mover)
            .unwrap();
        assert_eq!(held, (17, 3.0));
    }

    /// Slot 0x48 on the work command's target: the 4v4's forager 31156 walking to bush 28222 (state
    /// 4) is sent to it again in update 5026 and walks to it anew (`set_state(4)`); facing or
    /// gathering it, it keeps on; a computer player's walking one too. A loaded one sent to its
    /// player's drop site takes its load there (`set_state(8)`, the site its second target).
    #[test]
    fn a_gatherer_sent_again_to_its_bush_walks_to_it_anew() {
        let mut mover = crate::hunt::tests::walker();
        let walking = |state| GatherAction {
            state,
            move_to: Some(MoveTo::Moving),
            ..GatherAction::new(task(), &bush(), true).unwrap()
        };
        let mut g = walking(WALK);
        assert_eq!(
            g.retarget(&bush(), 7, false, 0.0, true, &mut mover),
            Ok(true)
        );
        assert_eq!(
            (
                g.state,
                g.move_to,
                g.target,
                g.carry_to,
                g.point,
                g.target_type
            ),
            (WALK, Some(MoveTo::Start), 28222, -1, [23.5, 93.5, 0.0], 7)
        );
        for (state, computer) in [(FACE, false), (WORK, false), (WALK, true)] {
            let mut g = walking(state);
            assert_eq!(
                g.retarget(&bush(), 7, computer, 0.0, true, &mut mover),
                Ok(true)
            );
            assert_eq!((g.state, g.move_to), (state, Some(MoveTo::Moving)));
        }
        let mill = Resource {
            id: 500,
            player: 7,
            drop_site: true,
            ..bush()
        };
        let mut g = walking(WALK);
        assert_eq!(g.retarget(&mill, 7, false, 3.0, true, &mut mover), Ok(true));
        assert_eq!(
            (g.state, g.target, g.carry_to, g.point),
            (DROP, -1, 500, [-1.0; 3])
        );
    }

    /// The forager's gathering: `(work rate) × work_val1 × dt` a tick into its load, the bush
    /// losing as much; the last bit below 0.1 goes with a thousandth of the master's storage; a
    /// load that would pass the capacity asks for the drop-off.
    #[test]
    fn a_forager_gathers_until_its_load_is_full() {
        let g = GatherAction {
            state: WORK,
            ..GatherAction::new(task(), &bush(), true).unwrap()
        };
        let (mut kind, mut held) = (17i16, 0.0f32);
        let mut sprite = None;
        let mut rng = Well512::default();
        let mut gatherer = Gatherer {
            held_kind: &mut kind,
            held: &mut held,
            work_rate: 0.31,
            capacity: 10.0,
            productivity: 1.0,
            sprites: HuntSprites::default(),
            sprite: &mut sprite,
            sprite_rng: &mut rng,
            class: 4,
            stance: 0,
            world_time: 0,
            farm_stops: true,
            posted: Vec::new(),
        };
        let mut bush = target(100.0);
        assert_eq!(g.gather(&mut bush, &mut gatherer, 0.5), Ok(Next::Stay));
        assert_eq!((*gatherer.held, bush.amount), (0.155, 100.0 - 0.155));
        // 0.2 less the work leaves a last bit below 0.1: it goes with the storage's thousandth
        let mut bush = target(0.2);
        let last = 0.2f32 - 0.155;
        assert_eq!(g.gather(&mut bush, &mut gatherer, 0.5), Ok(Next::Drop));
        assert_eq!(bush.amount, 0.0);
        assert_eq!(*gatherer.held, 0.155 + (0.155 + 125.0 * LAST_BIT + last));
        *gatherer.held = 9.9;
        assert_eq!(
            g.gather(&mut target(100.0), &mut gatherer, 0.5),
            Ok(Next::Drop)
        );
        assert_eq!(*gatherer.held, 9.9);
    }
}
