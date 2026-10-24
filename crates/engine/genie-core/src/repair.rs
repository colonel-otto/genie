//! A repairer's Repair action (`TRIBE_Action_Repair`, type 106, vtable `0x142ed7b88`): the
//! action walks to its target (state 4, a `MoveTo` sub-action within the unit's weapon range),
//! faces it (state 6) and repairs it each update (state 7) until the target holds its most hit
//! points or the player cannot pay for more; a target out of reach makes it wait (state 10) or
//! fail (state 13).
//!
//! The work of one update is the unit's work rate times the task's work value times the world's
//! dt, of which each repairer past the first adds half ([`work_amount`]). The target's own repair
//! (a building's slot 0x8c8, `FUN_1411d37d0`; a unit's, `FUN_1412143b0`) pays a share of its
//! master's cost ([`crate::production::pay`]) and heals it (`FUN_141193b30`); the caller runs it
//! ([`RepairView::repair`]), since it reaches other objects and the player's resources.

use crate::move_to::{Goal, MoveTo, MoveWait, Walk, start_walk_or_give_up, walk_or_give_up};
use crate::movement::footprint_gap;
use crate::unit_ai::GenieError;

/// The Repair action's type, which word 9 adds, and its tasks' action type.
pub const REPAIR: i16 = 0x6a;
/// The repair's action value (`0x26a`), which its notifies to the unit AI carry.
pub const REPAIR_ACTION: i32 = 0x26a;

/// The gap within which a repairer reaches its target (`DAT_142eb5690`).
pub const REACH: f32 = crate::image::repair::REACH;
/// What a moving target (`FUN_140fdd080`) or a scenario's game mode adds to the reach
/// (`DAT_142e8c150`).
const MOVING_REACH: f32 = crate::image::repair::MOVING_REACH;
/// The hit points below which a target in state 2 is not repaired (`DAT_142e7f384`), and the
/// share each repairer past the first adds to the work (the same constant).
const LEAST_HP: f32 = crate::image::repair::LEAST_HP;
const EXTRA_REPAIRER: f32 = crate::image::repair::EXTRA_REPAIRER;
/// The wait of state 10 (`0x3f800000`), and the repairer count's first (`DAT_14330eb8c`).
const ONE: f32 = crate::image::repair::ONE;

/// The Repair action's states.
const DONE: u8 = 1;
const HALTED: u8 = 2;
const IDLE: u8 = 3;
const MOVE: u8 = 4;
const FACE: u8 = 6;
const WORK: u8 = 7;
const WAIT: u8 = 10;
const WALK_POINT: u8 = 11;
const FAILED: u8 = 13;

/// The ship classes whose repair meets them at the shore (`FUN_140ad9120`: 20, 21, 22, 2, 53).
pub const SHIPS: [i16; 5] = [20, 21, 22, 2, 53];

/// A ship's repair: its meeting point (`FUN_140ad9120`: the unit's slot 0x428 toward the
/// shore, the ship's slot 0x340 there and its command count `+0x888`) is not modelled.
pub const SHIP_REPAIR: GenieError =
    GenieError::unsupported("the task finder's repair (106) of a ship (FUN_140ad9120)");

/// What a Repair works with: its task's id and work value (`+0x28`, `work_val1`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RepairTask {
    /// The task's id.
    pub id: i16,
    /// Its work value.
    pub work_value: f32,
}

/// The Repair action of a repairer (`TRIBE_Action_Repair`, type 106).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RepairAction {
    /// The target's id (`+0x30`), −1 for none.
    pub target: i32,
    /// The action state (`+0x18`), which word 9 adds.
    pub state: u8,
    /// Its `MoveTo` sub-action's state.
    pub move_to: Option<MoveTo>,
    /// Its `MoveTo`'s wait after the motion step gave up ([`MoveWait`]).
    pub move_wait: MoveWait,
    /// The point a `MoveTo` to a point walks to (`set_state(11)`).
    pub goal: Option<[f32; 3]>,
    /// `+0x38`..`+0x40`: the target's place as the action last took it (the constructor, states
    /// 4, 6 and 7), or the point the unit was sent to.
    pub point: [f32; 3],
    /// `+0x44`: state 10's wait.
    pub timer: f32,
    /// The task it repairs with (`+0x48`).
    pub task: RepairTask,
    /// The `MoveTo`'s range toward the target: the unit's master's weapon range (`+0x828`).
    pub range: f32,
    /// `+0x6c`: the action counts among its target's repairers (`+0x9e8` of the object
    /// `FUN_140ad9050` gives: the target, an annex's main building).
    pub counted: bool,
    /// `+0x60`: its end posts no notify 0x1fa.
    pub quiet: bool,
}

/// What a Repair's update reads of its target (the object its id names).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Repairee {
    /// Its state (`+0x640`).
    pub state: u8,
    /// Its hit points (`+0x278`).
    pub hp: f32,
    /// Its owner's id (`player+0x718`).
    pub owner: usize,
    /// The repairer's player counts its owner as its own or an ally (`FUN_141663dc0` with the
    /// ally flag).
    pub allied: bool,
    /// Its place and its master's radii.
    pub position: [f32; 3],
    /// Its master's radii.
    pub radius: [f32; 2],
    /// Its master's class.
    pub class: i16,
    /// It moves (`FUN_140fdd080`: a master of type 30 and up with a speed above 0).
    pub moves: bool,
}

/// What a Repair's update reads of the world and runs on it.
pub struct RepairView<'a> {
    /// The target, None when the world holds no object of its id.
    pub target: Option<Repairee>,
    /// The repairer's place and its master's radii.
    pub radius: [f32; 2],
    /// The game mode is one of `FUN_141302540`'s ([`crate::turn::scenario_mode`]).
    pub scenario: bool,
    /// The repairer's work rate: its `mWorkRatePowerup` (`+0xa30`, slot 0x6e8) plus its master's
    /// `work_rate` (`+0x798`), in that order.
    pub rate: f32,
    /// The repairers of the object `FUN_140ad9050` gives (its `+0x9e8`), None without one.
    pub repairers: Option<u8>,
    /// The world's time, which a turn toward the target reads.
    pub now: u32,
    /// The motion step giving up makes the `MoveTo` wait once (a human player's villager), and
    /// the unit AI's stance, which sets the wait ([`MoveWait::give_up`]).
    pub waits: bool,
    /// The unit AI's stance (`+0x1a9`).
    pub stance: u8,
    /// The unit's quick path to the target within [`REACH`] (slot 0x3e0, `canQuickPath`) over
    /// the world's objects.
    pub can_path: &'a mut dyn FnMut(&crate::spatial::Spatial) -> Result<bool, GenieError>,
    /// The target's repair by `amount` (a building's or a combat unit's slot 0x8c8, any other's
    /// slot 0x110): 0 to go on, 1 once it holds its most hit points, 2 when the player cannot pay.
    pub repair: &'a mut dyn FnMut(f32) -> Result<u8, GenieError>,
}

impl std::fmt::Debug for RepairView<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepairView")
            .field("target", &self.target)
            .field("rate", &self.rate)
            .field("repairers", &self.repairers)
            .finish_non_exhaustive()
    }
}

/// What a Repair's update leaves to its unit and player.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepairStep {
    /// The update returned 1 or 3: the action list removes the action, whose destructor
    /// (`FUN_140ad8fd0`) leaves its target's repairers ([`RepairStep::repairers`]).
    pub done: bool,
    /// The action let its target go (slot 0x68 with none, `FUN_140ada470` -> `FUN_140a7a230`:
    /// the target's slot 0x180): the caller's.
    pub released: bool,
    /// The notifies the unit's slot 0x328 posts to its unit AI, each with the repair's action
    /// value ([`REPAIR_ACTION`], 0, 0), in order: 0x1fa its end, 0x202 its search, 0x1f9 its
    /// failure, 0x1fb the player's want of resources.
    pub notifies: Vec<i32>,
    /// The unit takes its master's standing sprite (slot 0x68 with `+0x470`): the caller's.
    pub stands: bool,
    /// What the action did to its target's repairers (`+0x9e8` of `FUN_140ad9050`'s object).
    pub repairers: i8,
    /// The repair took the target to its most hit points, and a target of a type above 69 has
    /// the armor its charge's hits took back (`FUN_1412066a0`: `+0x9e4` zeroed): the caller's.
    pub restored: bool,
}

/// The work of one update (`0x140adaa20` case 6): the work `rate` times the task's
/// `work_value` times the world's `dt`; with more than one repairer (`repairers`, `COMISS` with
/// `JBE`) it is scaled by `((n − 1) × 0.5 + 1) / n`.
#[must_use]
pub fn work_amount(rate: f32, work_value: f32, dt: f32, repairers: Option<u8>) -> f32 {
    let mut amount = rate * work_value * dt;
    if let Some(n) = repairers {
        let n = f32::from(n);
        if ONE < n {
            amount *= ((n - ONE) * EXTRA_REPAIRER + ONE) / n;
        }
    }
    amount
}

impl RepairAction {
    /// A Repair from the work command (the factory `FUN_140aca4b0` case 0x6a, `FUN_140ad7fa0`):
    /// its target, whose place is its point, the task, not counted, not quiet; the start (slot
    /// 5, `FUN_140ad9030` with its force flag) then turns to state 4, whose `set_state` makes
    /// the `MoveTo` to the target within `range`.
    #[must_use]
    pub const fn start(target: i32, at: [f32; 3], task: RepairTask, range: f32) -> Self {
        Self {
            target,
            state: MOVE,
            move_to: Some(MoveTo::Start),
            move_wait: MoveWait {
                state: 0,
                timer: 0.0,
                delay: 0.0,
            },
            goal: None,
            point: at,
            timer: 0.0,
            task,
            range,
            counted: false,
            quiet: false,
        }
    }

    /// One update (slot 6, `0x140adaa20`). A target the world no longer holds ends it (slot 0x68
    /// with none, `set_state(1)`); so does one no longer in state 2, one below
    /// [`LEAST_HP`] hit points (`COMISS` with `JA`: a NaN is repaired), or one neither its
    /// player's own, an ally's nor Gaia's. Then by its state: 1 is done (the update returns 1);
    /// 4 walks ([`RepairAction::walking`]); 6 turns the unit toward the target, then works (7);
    /// 7 repairs ([`RepairAction::work`]); 10 waits a second, then works or waits again; 11
    /// walks to its point, then ends; 13 posts notify 0x1f9, halts and returns 3; any other
    /// does nothing.
    ///
    /// # Errors
    ///
    /// A ship's meeting point, and what the walk and the repair do not model.
    pub fn update(
        &mut self,
        mut walk: Walk<'_>,
        mut view: RepairView<'_>,
    ) -> Result<RepairStep, GenieError> {
        let mut step = RepairStep::default();
        if self.target != -1 && view.target.is_none() {
            // the target is gone: FUN_140ad9050 finds no object to leave the repairers of
            self.counted = false;
            self.target = -1;
            self.set_state(&mut walk, DONE, &view, &mut step)?;
            return Ok(step);
        }
        if let Some(t) = view.target
            && (t.state != 2 || LEAST_HP > t.hp || (!t.allied && t.owner != 0))
        {
            self.drop_target(&mut step);
            self.set_state(&mut walk, DONE, &view, &mut step)?;
            return Ok(step);
        }
        match self.state {
            DONE => step.done = true,
            MOVE => self.walking(&mut walk, &mut view, &mut step)?,
            FACE => {
                let t = view.target.ok_or(GenieError::unreachable(
                    "a Repair facing its target without one",
                ))?;
                let at = walk
                    .world
                    .get(walk.me.id)
                    .map_or(walk.me.position, |p| p.position);
                let mover = walk.mover.as_deref_mut().ok_or(GenieError::unreachable(
                    "a repairer without movement state (an action object always has one)",
                ))?;
                if crate::attack::face_target(mover, at, t.position, view.now)? {
                    self.set_state(&mut walk, WORK, &view, &mut step)?;
                }
            }
            WORK => self.work(&mut walk, view, &mut step)?,
            WAIT => {
                self.timer -= walk.dt;
                if crate::fmath::below_or_unordered(0.0, self.timer) {
                    return Ok(step);
                }
                if self.reaches(&walk, &view) {
                    self.set_state(&mut walk, FACE, &view, &mut step)?;
                } else {
                    self.timer = ONE;
                }
            }
            WALK_POINT => {
                let goal = self.goal.ok_or(GenieError::unreachable(
                    "a Repair's walk to its point without the point",
                ))?;
                // at the point (1), failed (3) or gone (the list empty, 1): the end
                if self.step_walk(&mut walk, &view, Goal::Point(goal), (0.0, true))? != 0 {
                    self.set_state(&mut walk, DONE, &view, &mut step)?;
                }
            }
            FAILED => {
                step.notifies.push(crate::unit_ai::ACTION_FAILED);
                self.set_state(&mut walk, HALTED, &view, &mut step)?;
                step.done = true;
            }
            // states 2, 3, 5, 8, 9, 12 and above 13: the update returns 0
            _ => {}
        }
        Ok(step)
    }

    /// State 4 (`0x140adaa20` case 3): the `MoveTo` toward the target walks (its start in its
    /// first update, [`start_walk`], then [`walk_on`]); once it is at it, waits or failed (or
    /// gone), the unit within reach of the target ([`RepairAction::reaches`]) faces it (6), one
    /// with a quick path to it waits (10), any other fails (13).
    fn walking(
        &mut self,
        walk: &mut Walk<'_>,
        view: &mut RepairView<'_>,
        step: &mut RepairStep,
    ) -> Result<(), GenieError> {
        let goal = Goal::Target(self.target);
        if self.step_walk(walk, view, goal, (self.range, false))? == 0 {
            return Ok(());
        }
        if self.reaches(walk, view) {
            self.set_state(walk, FACE, view, step)?;
        } else {
            let next = if (view.can_path)(walk.world)? {
                WAIT
            } else {
                FAILED
            };
            self.set_state(walk, next, &*view, step)?;
        }
        Ok(())
    }

    /// The `MoveTo`'s update and its code (`FUN_140a992b0`): its start asks for a path toward
    /// `goal` within `range` (`quiet` its `+0x60`), then it walks; the motion step giving up makes
    /// it wait once ([`MoveWait::give_up`]: a human player's villager), whose wait runs by the
    /// world's dt before it asks again, or fail. 0 while it walks or waits, 1 once it is at the
    /// goal (or gone), 3 once it failed.
    fn step_walk(
        &mut self,
        walk: &mut Walk<'_>,
        view: &RepairView<'_>,
        goal: Goal,
        (range, quiet): (f32, bool),
    ) -> Result<u8, GenieError> {
        let give_up = |w: &mut MoveWait| w.give_up(view.waits, view.stance);
        Ok(match self.move_to {
            Some(MoveTo::Start) => {
                if self.move_wait.state == 1 {
                    self.move_wait.state = 2;
                }
                self.move_to = Some(MoveTo::Moving);
                self.move_to = Some(
                    match start_walk_or_give_up(walk.reborrow(), goal, range, quiet)? {
                        Some(state) => state,
                        None => give_up(&mut self.move_wait),
                    },
                );
                0
            }
            Some(MoveTo::Moving) => {
                self.move_to = Some(match walk_or_give_up(walk.reborrow())? {
                    Some(state) => state,
                    None => give_up(&mut self.move_wait),
                });
                0
            }
            Some(MoveTo::Waiting) => {
                self.move_wait.timer += walk.dt;
                if self.move_wait.delay <= self.move_wait.timer {
                    self.move_wait.timer = 0.0;
                    self.move_to = Some(MoveTo::Start);
                }
                0
            }
            Some(MoveTo::Failed) => 3,
            Some(MoveTo::Arrived) | None => 1,
        })
    }

    /// The reach test of states 4 and 10: the footprints' gap (`FUN_14118f680`) less
    /// [`MOVING_REACH`] for a moving target or a scenario's game mode at most [`REACH`]
    /// (`COMISS REACH, gap` with `JC`/`JNC`: a NaN gap is out of reach).
    fn reaches(&self, walk: &Walk<'_>, view: &RepairView<'_>) -> bool {
        let Some(t) = view.target else {
            return false;
        };
        let gap = Self::gap(walk, view, t);
        !crate::fmath::below_or_unordered(REACH, gap)
    }

    /// The footprints' gap less the moving target's or the scenario's margin.
    fn gap(walk: &Walk<'_>, view: &RepairView<'_>, t: Repairee) -> f32 {
        let margin = if view.scenario || t.moves {
            MOVING_REACH
        } else {
            0.0
        };
        let at = walk
            .world
            .get(walk.me.id)
            .map_or(walk.me.position, |p| p.position);
        footprint_gap(at, view.radius, t.position, t.radius) - margin
    }

    /// State 7 (`0x140adaa20` case 6): a target that left the action's point (`UCOMISS` on x
    /// and y: a NaN has) within reach (`COMISS gap, REACH` with `JBE`: a NaN is) gives the point
    /// its place; out of reach the unit walks again (4). Then the work of the update
    /// ([`work_amount`]) goes to the target's repair ([`RepairView::repair`]): done (1) the
    /// target of a type above 69 has its armor back ([`RepairStep::restored`]) and the action
    /// ends (`set_state(1)`); the player short of resources (2) posts notify 0x1fb (the game's
    /// event 0x6b to its interface, `FUN_140c898a0`, is not held) and ends it too.
    fn work(
        &mut self,
        walk: &mut Walk<'_>,
        view: RepairView<'_>,
        step: &mut RepairStep,
    ) -> Result<(), GenieError> {
        let t = view.target.ok_or(GenieError::unreachable(
            "a Repair working without its target",
        ))?;
        let moved = t.position[0] != self.point[0] || t.position[1] != self.point[1];
        if moved {
            let gap = Self::gap(walk, &view, t);
            if REACH < gap {
                self.set_state(walk, MOVE, &view, step)?;
                return Ok(());
            }
            self.point = t.position;
        }
        let amount = work_amount(view.rate, self.task.work_value, walk.dt, view.repairers);
        match (view.repair)(amount)? {
            1 => {
                step.restored = true;
                self.set_state(walk, DONE, &view, step)?;
            }
            2 => {
                step.notifies.push(crate::unit_ai::ACTION_INVALIDATED);
                self.set_state(walk, DONE, &view, step)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Slot 0x68 with none (`FUN_140ada470`): a counted action leaves its target's repairers
    /// (`FUN_140ad9050`'s object, which exists), then the base lets the target go (the caller's
    /// release).
    const fn drop_target(&mut self, step: &mut RepairStep) {
        if self.counted {
            step.repairers -= 1;
            self.counted = false;
        }
        if self.target != -1 {
            step.released = true;
        }
        self.target = -1;
    }

    /// `set_state` (`0x140ad9c60`): a counted action leaves its target's repairers, the
    /// sub-actions go (a walking `MoveTo`'s teardown stops the unit), then by the new state: 1
    /// lets the target go (slot 0x68 with none) and posts notify 0x1fa unless quiet; 1, 2 and 3
    /// (with notify 0x202) give the unit its standing sprite; 4 takes the target's place and a
    /// `MoveTo` to the target; 6 takes its place (the moving sprite is not held); 7 counts the
    /// action among the target's repairers (the work sprite and sound are not held); 10 waits a
    /// second with the standing sprite (a ship target's move to the unit, [`SHIP_REPAIR`], is
    /// not modelled); 11 walks to the point (a quiet `MoveTo`).
    fn set_state(
        &mut self,
        walk: &mut Walk<'_>,
        state: u8,
        view: &RepairView<'_>,
        step: &mut RepairStep,
    ) -> Result<(), GenieError> {
        if self.counted {
            if view.repairers.is_some() {
                step.repairers -= 1;
            }
            self.counted = false;
        }
        if self.move_to.take().is_some()
            && let Some(m) = walk.mover.as_deref_mut()
        {
            m.stop()?;
        }
        self.goal = None;
        self.state = state;
        match state {
            DONE => {
                self.drop_target(step);
                if !self.quiet {
                    step.notifies.push(crate::unit_ai::ACTION_COMPLETED);
                }
                step.stands = true;
            }
            HALTED => step.stands = true,
            IDLE => {
                step.notifies.push(crate::unit_ai::REQUIRES_SEARCH);
                step.stands = true;
            }
            MOVE => {
                if let Some(t) = view.target {
                    self.point = t.position;
                }
                self.move_to = Some(MoveTo::Start);
                self.move_wait = MoveWait::default();
            }
            FACE => {
                if let Some(t) = view.target {
                    self.point = t.position;
                }
            }
            WORK => {
                if view.repairers.is_some() {
                    step.repairers += 1;
                    self.counted = true;
                }
            }
            WAIT => {
                step.stands = true;
                self.timer = ONE;
                if view.target.is_some_and(|t| SHIPS.contains(&t.class)) {
                    return Err(SHIP_REPAIR);
                }
            }
            WALK_POINT => {
                self.goal = Some(self.point);
                self.move_to = Some(MoveTo::Start);
                self.move_wait = MoveWait::default();
            }
            _ => {}
        }
        Ok(())
    }

    /// `set_state(4)` from the work command's new target (slot 0x48, the caller's prelude):
    /// the `MoveTo` goes (a walking one stops the unit) and a new one to the target starts.
    ///
    /// # Errors
    ///
    /// The unit's stop that the motion step does not model.
    pub fn walk_again(
        &mut self,
        mover: Option<&mut crate::movement::Mover>,
    ) -> Result<(), GenieError> {
        if self.move_to.take().is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        self.goal = None;
        self.state = MOVE;
        self.move_to = Some(MoveTo::Start);
        self.move_wait = MoveWait::default();
        Ok(())
    }

    /// Slot 8 (`FUN_140ad9700`), a move to `point` (its target argument unread): the point is
    /// the action's, slot 0x68 with none (a counted repairer leaves the repairers, the target
    /// goes), then `set_state(11)` (the `MoveTo` goes, a walking one stopping the unit; a quiet
    /// one to the point starts). Returns the target let go and whether it counted.
    ///
    /// # Errors
    ///
    /// The unit's stop that the motion step does not model.
    pub fn take_point(
        &mut self,
        point: [f32; 3],
        mover: Option<&mut crate::movement::Mover>,
    ) -> Result<(i32, bool), GenieError> {
        self.point = point;
        let counted = std::mem::replace(&mut self.counted, false);
        let old = std::mem::replace(&mut self.target, -1);
        if self.move_to.take().is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        self.state = WALK_POINT;
        self.goal = Some(point);
        self.move_to = Some(MoveTo::Start);
        self.move_wait = MoveWait::default();
        Ok((old, counted))
    }

    /// Slot 7 (`FUN_140adaa00`) as the unit's stop calls it: `set_state(2)`. Returns whether it
    /// counted among its target's repairers before (the caller's to leave them) and the target
    /// it still holds.
    ///
    /// # Errors
    ///
    /// The unit's stop that the motion step does not model.
    pub fn halt(&mut self, mover: Option<&mut crate::movement::Mover>) -> Result<bool, GenieError> {
        let counted = std::mem::replace(&mut self.counted, false);
        if self.move_to.take().is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        self.goal = None;
        self.state = HALTED;
        Ok(counted)
    }
}

/// The classes whose repair pays its unit share (`FUN_1412143b0`: the ships 20, 21, 22, 2 and
/// 53, the siege 13 and the classes 51, 54 and 55).
const COSTED: [i16; 9] = [20, 21, 22, 2, 53, 13, 51, 54, 55];
/// The trebuchets' classes (51, 54), whose building repair is the unit's.
const TREBUCHETS: [i16; 2] = [51, 54];
/// The owner's attributes of a unit's and a building's repair cost (270, 271: `player+0x100`
/// at `+0x870` and `+0x878`).
pub const UNIT_REPAIR_COST: usize = 270;
/// The building repair cost attribute (271).
pub const BUILDING_REPAIR_COST: usize = 271;

/// One object a repair reaches (the target, an annex's main building, a main building's
/// annexes) as the repair reads and changes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Repaired {
    /// The object.
    pub id: i32,
    /// Its master's type.
    pub kind: u8,
    /// Its master's class.
    pub class: i16,
    /// Its master.
    pub master: i16,
    /// Its master's hit points (`+0x530`).
    pub master_hp: i16,
    /// Its hit points (`+0x278`).
    pub hp: f32,
    /// Its hit point powerup and its room (`+0xa38`, `+0xa3c`).
    pub hp_powerups: [f32; 2],
    /// An annex's main building (`+0xbb8`).
    pub main: Option<i32>,
    /// A main building's annexes (`+0xbc0`, its four slots in their order).
    pub annexes: Vec<i32>,
    /// Its owner's attributes 270 and 271 ([`UNIT_REPAIR_COST`], [`BUILDING_REPAIR_COST`]).
    pub cost_rates: [f32; 2],
}

/// What a repair pays with: the repairer's player's masters and attributes (`FUN_14162fb60`
/// with the repairer's player).
pub struct Payer<'a> {
    /// The repairer's player's masters.
    pub masters: &'a crate::masters::Masters,
    /// The repairer's player's attributes.
    pub attributes: &'a mut dyn crate::tech::Attributes,
}

impl std::fmt::Debug for Payer<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Payer").finish_non_exhaustive()
    }
}

/// The object of `set` with id `id`.
fn find(set: &[Repaired], id: i32) -> Result<usize, GenieError> {
    set.iter()
        .position(|o| o.id == id)
        .ok_or(GenieError::unreachable("a repair's object outside its set"))
}

/// A building's slot 0x8c8 (`FUN_1411d37d0`) by `amount` for the repairer (`payer`): a
/// trebuchet's class repairs as a unit ([`repair_unit`]); one below 1 hit point (`COMISS`: a NaN
/// is) takes nothing (2). From its repairer (`top`) an annex's repair is its main building's.
/// Else each of its annexes (a combat unit's or a building's type) is repaired by the amount
/// first (not `top`); then the amount, at most what its hit points lack of its master's, costs
/// its owner's building repair rate times the amount over its master's hit points of its
/// master's costs ([`crate::production::pay`]); paid, it heals as a unit's does, else 2.
///
/// # Errors
///
/// An object outside `set`, and an attribute the player does not have.
pub fn repair_building(
    set: &mut [Repaired],
    idx: usize,
    amount: f32,
    top: bool,
    payer: &mut Payer<'_>,
) -> Result<u8, GenieError> {
    if TREBUCHETS.contains(&set[idx].class) {
        return repair_unit(set, idx, amount, payer);
    }
    if crate::fmath::below_or_unordered(set[idx].hp, ONE) {
        return Ok(2);
    }
    if top && let Some(main) = set[idx].main {
        let m = find(set, main)?;
        return repair_building(set, m, amount, true, payer);
    }
    for annex in set[idx].annexes.clone() {
        let a = find(set, annex)?;
        match set[a].kind {
            70 => {
                let _ = repair_unit(set, a, amount, payer)?;
            }
            80 => {
                let _ = repair_building(set, a, amount, false, payer)?;
            }
            _ => {}
        }
    }
    let o = &set[idx];
    let most = f32::from(o.master_hp);
    let mut amount = amount;
    if most < o.hp + amount {
        amount = most - o.hp;
    }
    let share = o.cost_rates[1] * amount / most;
    if crate::production::pay(payer.masters, o.master, share, &mut *payer.attributes)? {
        return repair_unit(set, idx, amount, payer);
    }
    Ok(2)
}

/// A combat unit's slot 0x8c8 (`FUN_1412143b0`, also a building's last step) by `amount`: a
/// class of [`COSTED`] first takes at most what its hit points lack of its most (slots 0xc0 and
/// 200) and pays its owner's unit repair rate times the amount over its most of its master's
/// costs (unpaid: 2); then it heals (`FUN_141193b30`, [`crate::damage::heal_hp`]): 1 once full,
/// else 0.
///
/// # Errors
///
/// An attribute the player does not have.
pub fn repair_unit(
    set: &mut [Repaired],
    idx: usize,
    amount: f32,
    payer: &mut Payer<'_>,
) -> Result<u8, GenieError> {
    let o = &mut set[idx];
    let mut amount = amount;
    if COSTED.contains(&o.class) {
        let [up, room] = o.hp_powerups;
        let most = f32::from(o.master_hp) + room;
        if most < ((room - up) + o.hp) + amount {
            amount = most - ((room - up) + o.hp);
        }
        let share = o.cost_rates[0] * amount / most;
        if !crate::production::pay(payer.masters, o.master, share, &mut *payer.attributes)? {
            return Ok(2);
        }
    }
    let full = crate::damage::heal_hp(&mut o.hp, &mut o.hp_powerups, o.master_hp, amount);
    Ok(u8::from(full))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement::Me;
    use crate::spatial::Spatial;

    const TASK: RepairTask = RepairTask {
        id: 7,
        work_value: 0.5,
    };

    /// A villager's master work rate 1 with a powerup of 0: half a hit point per second per the
    /// task, each repairer past the first half of it.
    #[test]
    fn the_work_counts_half_for_each_repairer_past_the_first() {
        assert_eq!(work_amount(1.0, 0.5, 0.05, None), 1.0 * 0.5 * 0.05);
        assert_eq!(work_amount(1.0, 0.5, 0.05, Some(1)), 1.0 * 0.5 * 0.05);
        let alone = 1.0f32 * 0.5 * 0.05;
        assert_eq!(
            work_amount(1.0, 0.5, 0.05, Some(3)),
            alone * (((3.0 - 1.0) * 0.5 + 1.0) / 3.0)
        );
    }

    /// The repairer's player's attributes as a list (food, wood, stone, gold at 0 to 3).
    struct Held(Vec<f32>);

    impl crate::tech::Attributes for Held {
        fn get(&self, i: i16) -> Result<f32, GenieError> {
            Ok(self.0[i as usize])
        }
        fn set(&mut self, i: i16, v: f32) -> Result<(), GenieError> {
            self.0[i as usize] = v;
            Ok(())
        }
    }

    /// A house (70, class 3, 550 hit points, 25 wood) and a ram (35, class 13, 175 hit points,
    /// 160 wood, 75 gold) of the repairer's player.
    fn masters() -> crate::masters::Masters {
        crate::masters::Masters::new([
            crate::masters::Master {
                id: 70,
                kind: 80,
                class: 3,
                hit_points: 550,
                cost: vec![(1, 25, true)],
                ..crate::masters::Master::default()
            },
            crate::masters::Master {
                id: 35,
                kind: 70,
                class: 13,
                hit_points: 175,
                cost: vec![(1, 160, true), (3, 75, true)],
                ..crate::masters::Master::default()
            },
        ])
    }

    fn house(id: i32, hp: f32) -> Repaired {
        Repaired {
            id,
            kind: 80,
            class: 3,
            master: 70,
            master_hp: 550,
            hp,
            hp_powerups: [0.0; 2],
            main: None,
            annexes: Vec::new(),
            cost_rates: [0.5, 0.15],
        }
    }

    /// `FUN_1411d37d0` on a house: the amount, at most what its hit points lack, costs 0.15 of
    /// its 25 wood per its 550 hit points; the last of it heals it full (1); a player without
    /// the wood pays nothing and heals nothing (2); a house below 1 hit point takes nothing.
    #[test]
    fn a_building_s_repair_pays_its_share_of_the_cost_and_heals() {
        let masters = masters();
        let mut wood = Held(vec![0.0, 100.0, 0.0, 0.0]);
        let mut set = vec![house(1, 500.0)];
        let mut payer = Payer {
            masters: &masters,
            attributes: &mut wood,
        };
        assert_eq!(repair_building(&mut set, 0, 10.0, true, &mut payer), Ok(0));
        assert_eq!(set[0].hp, 510.0);
        let share = 0.15f32 * 10.0 / 550.0;
        assert_eq!(wood.0[1], 100.0 - 25.0 * share);
        let mut payer = Payer {
            masters: &masters,
            attributes: &mut wood,
        };
        assert_eq!(repair_building(&mut set, 0, 50.0, true, &mut payer), Ok(1));
        assert_eq!(set[0].hp, 550.0);
        let mut broke = Held(vec![0.0; 4]);
        let mut set = vec![house(1, 500.0)];
        let mut payer = Payer {
            masters: &masters,
            attributes: &mut broke,
        };
        assert_eq!(repair_building(&mut set, 0, 10.0, true, &mut payer), Ok(2));
        assert_eq!(set[0].hp, 500.0);
        let mut set = vec![house(1, 0.5)];
        assert_eq!(repair_building(&mut set, 0, 10.0, true, &mut payer), Ok(2));
    }

    /// From its repairer an annex's repair is its main building's, which repairs its annexes
    /// (here the annex itself) by the whole amount first, then itself.
    #[test]
    fn an_annex_s_repair_goes_to_its_main_building_and_its_annexes() {
        let masters = masters();
        let mut wood = Held(vec![0.0, 100.0, 0.0, 0.0]);
        let mut set = vec![
            Repaired {
                main: Some(2),
                ..house(1, 500.0)
            },
            Repaired {
                annexes: vec![1],
                ..house(2, 540.0)
            },
        ];
        let mut payer = Payer {
            masters: &masters,
            attributes: &mut wood,
        };
        assert_eq!(repair_building(&mut set, 0, 20.0, true, &mut payer), Ok(1));
        assert_eq!((set[0].hp, set[1].hp), (520.0, 550.0));
    }

    /// `FUN_1412143b0` on a ram (class 13): the amount, at most what it lacks of its most, costs
    /// its owner's unit repair rate of its costs per its most hit points; then it heals.
    #[test]
    fn a_siege_unit_s_repair_pays_its_unit_share() {
        let masters = masters();
        let mut held = Held(vec![0.0, 1000.0, 0.0, 1000.0]);
        let mut set = vec![Repaired {
            kind: 70,
            class: 13,
            master: 35,
            master_hp: 175,
            ..house(5, 100.0)
        }];
        let mut payer = Payer {
            masters: &masters,
            attributes: &mut held,
        };
        assert_eq!(repair_unit(&mut set, 0, 10.0, &mut payer), Ok(0));
        let share = 0.5f32 * 10.0 / 175.0;
        assert_eq!(set[0].hp, 110.0);
        assert_eq!(
            (held.0[1], held.0[3]),
            (1000.0 - 160.0 * share, 1000.0 - 75.0 * share)
        );
    }

    fn me() -> Me {
        Me {
            id: 9,
            position: [10.0, 10.0, 0.0],
            group: -1,
            action_state: 0,
        }
    }

    fn walk<'a>(world: &'a mut Spatial, mover: Option<&'a mut crate::movement::Mover>) -> Walk<'a> {
        Walk {
            mover,
            me: me(),
            dt: 0.05,
            world,
            plan: None,
        }
    }

    fn ward(x: f32) -> Repairee {
        Repairee {
            state: 2,
            hp: 300.0,
            owner: 1,
            allied: true,
            position: [x, 10.0, 0.0],
            radius: [1.0, 1.0],
            class: 3,
            moves: false,
        }
    }

    /// One update of `r` with the target `target`, the repairers `repairers`, a quick path
    /// `paths` and the repair's answer `answer` (recording its amounts).
    fn update(
        r: &mut RepairAction,
        target: Option<Repairee>,
        (repairers, paths, answer): (Option<u8>, bool, u8),
        amounts: &mut Vec<f32>,
    ) -> RepairStep {
        let mut world = Spatial::default();
        let mut mover = crate::hunt::tests::walker();
        let mut can_path = |_: &Spatial| Ok(paths);
        let mut repair = |a: f32| {
            amounts.push(a);
            Ok(answer)
        };
        let view = RepairView {
            target,
            radius: [0.2, 0.2],
            scenario: false,
            rate: 1.0,
            repairers,
            now: 0,
            waits: false,
            stance: 0,
            can_path: &mut can_path,
            repair: &mut repair,
        };
        r.update(walk(&mut world, Some(&mut mover)), view).unwrap()
    }

    /// A target the world no longer holds, another player's or a dead one ends the Repair
    /// (`set_state(1)`: notify 0x1fa, the standing sprite); the next update removes it.
    #[test]
    fn a_repair_ends_with_its_target_gone_foreign_or_dead() {
        let mut amounts = Vec::new();
        for target in [
            None,
            Some(Repairee {
                allied: false,
                owner: 2,
                ..ward(11.0)
            }),
            Some(Repairee {
                state: 3,
                ..ward(11.0)
            }),
            Some(Repairee {
                hp: 0.25,
                ..ward(11.0)
            }),
        ] {
            let mut r = RepairAction::start(20, [11.0, 10.0, 0.0], TASK, 0.0);
            let step = update(&mut r, target, (None, true, 0), &mut amounts);
            assert_eq!((r.state, r.target), (DONE, -1));
            assert_eq!(step.notifies, vec![crate::unit_ai::ACTION_COMPLETED]);
            assert!(step.stands && !step.done);
            assert_eq!(step.released, target.is_some());
            let step = update(&mut r, None, (None, true, 0), &mut amounts);
            assert!(step.done);
        }
        // Gaia's object is repaired
        let mut r = RepairAction {
            state: FACE,
            move_to: None,
            ..RepairAction::start(20, [11.0, 10.0, 0.0], TASK, 0.0)
        };
        let gaia = Repairee {
            allied: false,
            owner: 0,
            ..ward(11.0)
        };
        let step = update(&mut r, Some(gaia), (Some(0), true, 0), &mut amounts);
        assert_eq!((r.state, step.repairers), (WORK, 1));
    }

    /// State 4 once the `MoveTo` ended: within reach the unit faces the target (6); out of reach
    /// it waits (10) with a quick path to it, else fails (13), whose update posts 0x1f9.
    #[test]
    fn a_repairer_at_its_target_faces_it_or_waits_or_fails() {
        let mut amounts = Vec::new();
        let arrived = || RepairAction {
            move_to: Some(MoveTo::Arrived),
            ..RepairAction::start(20, [11.0, 10.0, 0.0], TASK, 0.0)
        };
        let mut r = arrived();
        let _ = update(&mut r, Some(ward(11.0)), (Some(0), true, 0), &mut amounts);
        assert_eq!((r.state, r.move_to), (FACE, None));
        let mut r = arrived();
        let step = update(&mut r, Some(ward(20.0)), (Some(0), true, 0), &mut amounts);
        assert_eq!((r.state, r.timer, step.stands), (WAIT, 1.0, true));
        let mut r = arrived();
        let _ = update(&mut r, Some(ward(20.0)), (Some(0), false, 0), &mut amounts);
        assert_eq!(r.state, FAILED);
        let step = update(&mut r, Some(ward(20.0)), (Some(0), false, 0), &mut amounts);
        assert_eq!(
            (r.state, step.notifies, step.done, step.stands),
            (HALTED, vec![crate::unit_ai::ACTION_FAILED], true, true)
        );
        assert_eq!(amounts, Vec::<f32>::new());
    }

    /// State 7: each update's work goes to the target's repair; full (1) the Repair ends, its
    /// counted repairer out of the repairers, the armor back; the player short of resources (2)
    /// posts 0x1fb and ends it too; a target that walked out of reach sends the unit walking.
    #[test]
    fn a_repairer_works_until_the_target_is_full_or_the_player_cannot_pay() {
        let working = || RepairAction {
            state: WORK,
            move_to: None,
            counted: true,
            ..RepairAction::start(20, [11.0, 10.0, 0.0], TASK, 0.0)
        };
        let mut amounts = Vec::new();
        let mut r = working();
        let step = update(&mut r, Some(ward(11.0)), (Some(2), true, 0), &mut amounts);
        assert_eq!((r.state, step.repairers), (WORK, 0));
        assert_eq!(amounts, vec![work_amount(1.0, 0.5, 0.05, Some(2))]);
        let step = update(&mut r, Some(ward(11.0)), (Some(2), true, 1), &mut amounts);
        assert_eq!((r.state, step.repairers, step.restored), (DONE, -1, true));
        assert_eq!(step.notifies, vec![crate::unit_ai::ACTION_COMPLETED]);
        let mut r = working();
        let step = update(&mut r, Some(ward(11.0)), (Some(1), true, 2), &mut amounts);
        assert_eq!(
            step.notifies,
            vec![
                crate::unit_ai::ACTION_INVALIDATED,
                crate::unit_ai::ACTION_COMPLETED
            ]
        );
        let mut r = working();
        let _ = update(&mut r, Some(ward(20.0)), (Some(1), true, 0), &mut amounts);
        assert_eq!(
            (r.state, r.move_to, r.counted),
            (MOVE, Some(MoveTo::Start), false)
        );
    }

    /// State 10 waits a second (the world's dt off the timer), then faces a target within reach
    /// or waits a second again.
    #[test]
    fn a_waiting_repairer_looks_again_each_second() {
        let mut amounts = Vec::new();
        let mut r = RepairAction {
            state: WAIT,
            move_to: None,
            timer: 0.1,
            ..RepairAction::start(20, [11.0, 10.0, 0.0], TASK, 0.0)
        };
        let _ = update(&mut r, Some(ward(20.0)), (Some(0), true, 0), &mut amounts);
        assert_eq!((r.state, r.timer), (WAIT, 0.1 - 0.05));
        let _ = update(&mut r, Some(ward(20.0)), (Some(0), true, 0), &mut amounts);
        assert_eq!((r.state, r.timer), (WAIT, 1.0));
        r.timer = 0.0;
        let _ = update(&mut r, Some(ward(11.0)), (Some(0), true, 0), &mut amounts);
        assert_eq!(r.state, FACE);
    }

    /// Slot 8 (`FUN_140ad9700`): the point, the target let go (a counted repairer leaving its
    /// repairers), the walk there; its end ends the Repair.
    #[test]
    fn a_move_sends_the_repairer_to_its_point() {
        let mut mover = crate::hunt::tests::walker();
        let mut r = RepairAction {
            state: WORK,
            move_to: None,
            counted: true,
            ..RepairAction::start(20, [11.0, 10.0, 0.0], TASK, 0.0)
        };
        assert_eq!(
            r.take_point([30.0, 5.0, 0.0], Some(&mut mover)),
            Ok((20, true))
        );
        assert_eq!(
            (r.state, r.target, r.goal, r.counted),
            (WALK_POINT, -1, Some([30.0, 5.0, 0.0]), false)
        );
        let mut amounts = Vec::new();
        r.move_to = Some(MoveTo::Arrived);
        let step = update(&mut r, None, (None, true, 0), &mut amounts);
        assert_eq!(
            (r.state, step.notifies),
            (DONE, vec![crate::unit_ai::ACTION_COMPLETED])
        );
    }
}
