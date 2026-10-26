//! The Repair action's start, update and states ([`RepairAction`]): the walk, the face, the
//! wait, the work, a new target, a move and the halt.

use crate::move_to::{Goal, MoveTo, MoveWait, Walk, start_walk_or_give_up, walk_or_give_up};
use crate::movement::footprint_gap;
use crate::repair::{
    DONE, EXTRA_REPAIRER, FACE, FAILED, HALTED, IDLE, LEAST_HP, MOVE, MOVING_REACH, ONE, REACH,
    RepairAction, RepairStep, RepairTask, RepairView, Repairee, SHIP_REPAIR, SHIPS, WAIT,
    WALK_POINT, WORK,
};
use crate::unit_ai::GenieError;

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
            FACE => self.facing(&mut walk, &view, &mut step)?,
            WORK => self.work(&mut walk, view, &mut step)?,
            WAIT => self.waiting(&mut walk, &view, &mut step)?,
            WALK_POINT => self.walking_to_point(&mut walk, &view, &mut step)?,
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

    /// State 6 (`0x140adaa20` case 5): the unit turns toward the target ([`face_target`] with its
    /// sprites' facets), then works (7).
    ///
    /// [`face_target`]: crate::attack::face_target
    pub(super) fn facing(
        &mut self,
        walk: &mut Walk<'_>,
        view: &RepairView<'_>,
        step: &mut RepairStep,
    ) -> Result<(), GenieError> {
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
            self.set_state(walk, WORK, view, step)?;
        }
        Ok(())
    }

    /// State 10 (`0x140adaa20` case 9): the timer counts down by the update's seconds; once it
    /// is out (`COMISS` 0 with `JC`: a NaN is out) a unit within reach faces the target (6), any
    /// other waits a second more.
    pub(super) fn waiting(
        &mut self,
        walk: &mut Walk<'_>,
        view: &RepairView<'_>,
        step: &mut RepairStep,
    ) -> Result<(), GenieError> {
        self.timer -= walk.dt;
        if crate::math::fmath::below_or_unordered(0.0, self.timer) {
            return Ok(());
        }
        if self.reaches(walk, view) {
            self.set_state(walk, FACE, view, step)?;
        } else {
            self.timer = ONE;
        }
        Ok(())
    }

    /// State 11 (`0x140adaa20` case 10): the walk to the Repair's point; at it (1), failed (3)
    /// or gone (the list empty, 1) the Repair ends.
    pub(super) fn walking_to_point(
        &mut self,
        walk: &mut Walk<'_>,
        view: &RepairView<'_>,
        step: &mut RepairStep,
    ) -> Result<(), GenieError> {
        let goal = self.goal.ok_or(GenieError::unreachable(
            "a Repair's walk to its point without the point",
        ))?;
        if self.step_walk(walk, view, Goal::Point(goal), (0.0, true))? != 0 {
            self.set_state(walk, DONE, view, step)?;
        }
        Ok(())
    }

    /// State 4 (`0x140adaa20` case 3): the `MoveTo` toward the target walks (its start in its
    /// first update, [`start_walk_or_give_up`], then [`walk_or_give_up`]); once it is at it, waits or failed (or
    /// gone), the unit within reach of the target ([`RepairAction::reaches`]) faces it (6), one
    /// with a quick path to it waits (10), any other fails (13).
    pub(super) fn walking(
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
    pub(super) fn step_walk(
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
    pub(super) fn reaches(&self, walk: &Walk<'_>, view: &RepairView<'_>) -> bool {
        let Some(t) = view.target else {
            return false;
        };
        let gap = Self::gap(walk, view, t);
        !crate::math::fmath::below_or_unordered(REACH, gap)
    }

    /// The footprints' gap less the moving target's or the scenario's margin.
    pub(super) fn gap(walk: &Walk<'_>, view: &RepairView<'_>, t: Repairee) -> f32 {
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
    pub(super) fn work(
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
    pub(super) const fn drop_target(&mut self, step: &mut RepairStep) {
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
    pub(super) fn set_state(
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
