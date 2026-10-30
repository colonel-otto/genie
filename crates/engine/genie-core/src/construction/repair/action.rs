//! The Repair action's start, update and states ([`RepairAction`]): the walk, the face, the
//! wait, the work, a new target, a move and the halt.

use crate::GenieError;
use crate::construction::repair::{
    EXTRA_REPAIRER, FACE, HALTED, IDLE, LEAST_HP, MOVE, MOVING_REACH, ONE, REACH, RepairAction,
    RepairReads, RepairTask, Repairee, SHIP_REPAIR, SHIPS, WAIT, WALK_POINT, WORK,
};
use crate::math::rng::Stream;
use crate::movement::move_to::{
    GiveUp, MoveTo, MoveToAction, MoveToReads, Shown, Sprites, TargetMoved, WalkTo,
};
use crate::movement::{Me, PathAsk, footprint_gap};
use crate::objects::lifecycle::seconds;
use crate::objects::masters::master_type::COMBAT;
use crate::objects::teardown::HeadCode;
use crate::objects::unit::Unit;
use crate::orders::action_state::DONE;
use crate::orders::action_state::FAILED;
use crate::unit_ai::ActionKind;
use crate::unit_ai::Inbox;
use crate::world::step::Step;

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

/// The repairer's slot 0x328: notify `kind` with the repair's action value to its unit AI.
fn notify(repairer: &mut Unit, kind: i32) -> Result<(), GenieError> {
    repairer.post_own(kind, [ActionKind::Repair.value(), 0, 0])
}

/// Slot 0x68 of the repairer: sprite `id` shows (the master's standing one for none, `standing`),
/// its node in the unit's movement state.
fn show(
    c: &mut Step<'_>,
    repairer: &mut Unit,
    id: Option<i16>,
    standing: Option<i16>,
) -> Result<(), GenieError> {
    let mut sprites = Sprites {
        sprite: &mut repairer.sprite,
        rng: c.w.streams.get(Stream::Sprite),
        graphics: &c.w.graphics,
    };
    sprites
        .shown(id.or(standing))?
        .show(repairer.body.move_stats_mut());
    Ok(())
}

impl RepairAction {
    /// A Repair from the work command (the factory `FUN_140aca4b0` case 0x6a, `FUN_140ad7fa0`):
    /// its target `to`, whose place is its point, the task, not counted, not quiet; the start
    /// (slot 5, `FUN_140ad9030` with its force flag) then turns to state 4, whose `set_state`
    /// makes the `MoveTo` to the target within `range` ([`RepairAction::walk`] with what it
    /// reads of the repairer, `move_reads`).
    ///
    /// # Errors
    ///
    /// A sprite the table lacks.
    pub fn start(
        to: WalkTo,
        task: RepairTask,
        range: f32,
        move_reads: &MoveToReads<'_>,
    ) -> Result<Self, GenieError> {
        let mut repair = Self {
            target: to.id,
            state: MOVE,
            move_to: None,
            point: to.at,
            timer: 0.0,
            task,
            range,
            counted: false,
            quiet: false,
        };
        repair.move_to = repair.walk(move_reads, Some(to))?;
        Ok(repair)
    }

    /// The `MoveTo` of state 4 (`0x140ad9c60` case 4: `FUN_140aa0df0` with the action's range and
    /// the task's moving sprite `+0x40`, then quiet) to the target `to`, or without one
    /// ([`MoveToReads::to_nothing`]), as the sub-action list adds it.
    ///
    /// # Errors
    ///
    /// A sprite the table lacks.
    pub fn walk(
        &self,
        move_reads: &MoveToReads<'_>,
        to: Option<WalkTo>,
    ) -> Result<Option<MoveToAction>, GenieError> {
        to.map_or_else(
            || move_reads.to_nothing(self.range, self.task.moving),
            |to| move_reads.to_object(to, self.range, self.task.moving, TargetMoved::Follows),
        )
    }

    /// The quiet `MoveTo` of state 11 (`0x140ad9c60` case 0xb: `FUN_140aa0d30`, quiet) to
    /// `point` within 0 with the task's moving sprite, as the sub-action list adds it.
    ///
    /// # Errors
    ///
    /// A sprite the table lacks.
    pub fn to_point(
        &self,
        point: [f32; 3],
        move_reads: &MoveToReads<'_>,
    ) -> Result<Option<MoveToAction>, GenieError> {
        move_reads.to_point(point, 0.0, self.task.moving)
    }

    /// One update (slot 6, `0x140adaa20`) of `repairer` (`me`) through its step, with what it
    /// reads as it starts (`reads`). A target the world no longer holds ends it (slot 0x68 with
    /// none, `set_state(1)`); so does one no longer in state 2, one below [`LEAST_HP`] hit
    /// points (`COMISS` with `JA`: a NaN is repaired), or one neither its player's own, an
    /// ally's nor Gaia's. Then by its state: 1 is done (its 1: the action list deletes it); 4
    /// walks ([`RepairAction::walking`]); 6 turns the unit toward the target, then works (7); 7
    /// repairs ([`RepairAction::work`]); 10 waits a second, then works or waits again; 11 walks
    /// to its point, then ends; 13 posts notify 0x1f9, halts and returns 3 (deleted too); any
    /// other does nothing.
    ///
    /// # Errors
    ///
    /// A ship's meeting point, and what the walk and the repair do not model.
    pub(crate) fn update(
        &mut self,
        c: &mut Step<'_>,
        repairer: &mut Unit,
        me: Me,
        reads: &RepairReads,
    ) -> Result<HeadCode, GenieError> {
        if self.target != -1 && reads.target.is_none() {
            // the target is gone: FUN_140ad9050 finds no object to leave the repairers of
            self.counted = false;
            self.target = -1;
            self.set_state(c, repairer, DONE, reads)?;
            return Ok(HeadCode::Keeps);
        }
        if let Some(t) = reads.target
            && (t.state != 2 || LEAST_HP > t.hp || (!t.allied && t.owner != 0))
        {
            self.drop_target(c, repairer, reads)?;
            self.set_state(c, repairer, DONE, reads)?;
            return Ok(HeadCode::Keeps);
        }
        match self.state {
            DONE => return Ok(HeadCode::Deletes),
            MOVE => self.walking(c, repairer, me, reads)?,
            FACE => self.facing(c, repairer, reads)?,
            WORK => self.work(c, repairer, reads)?,
            WAIT => self.waiting(c, repairer, reads)?,
            WALK_POINT => self.walking_to_point(c, repairer, me, reads)?,
            FAILED => {
                notify(repairer, crate::unit_ai::ACTION_FAILED)?;
                self.set_state(c, repairer, HALTED, reads)?;
                return Ok(HeadCode::Deletes);
            }
            // states 2, 3, 5, 8, 9, 12 and above 13: the update returns 0
            _ => {}
        }
        Ok(HeadCode::Keeps)
    }

    /// State 6 (`0x140adaa20` case 5): the unit turns toward the target ([`face_target`] with its
    /// sprites' facets), then works (7).
    ///
    /// [`face_target`]: crate::combat::attack::face_target
    pub(super) fn facing(
        &mut self,
        c: &mut Step<'_>,
        repairer: &mut Unit,
        reads: &RepairReads,
    ) -> Result<(), GenieError> {
        let t = reads.target.ok_or(GenieError::unreachable(
            "a Repair facing its target without one",
        ))?;
        let at = repairer.body.position();
        let mover = repairer.body.mover_mut().ok_or(GenieError::unreachable(
            "a repairer without movement state (an action object always has one)",
        ))?;
        if crate::combat::attack::face_target(mover, at, t.position, c.w.world_time)? {
            self.set_state(c, repairer, WORK, reads)?;
        }
        Ok(())
    }

    /// State 10 (`0x140adaa20` case 9): the timer counts down by the update's seconds; once it
    /// is out (`COMISS` 0 with `JC`: a NaN is out) a unit within reach faces the target (6), any
    /// other waits a second more.
    pub(super) fn waiting(
        &mut self,
        c: &mut Step<'_>,
        repairer: &mut Unit,
        reads: &RepairReads,
    ) -> Result<(), GenieError> {
        self.timer -= seconds(c.dt);
        if crate::math::fmath::below_or_unordered(0.0, self.timer) {
            return Ok(());
        }
        if Self::reaches(repairer, reads) {
            self.set_state(c, repairer, FACE, reads)?;
        } else {
            self.timer = ONE;
        }
        Ok(())
    }

    /// State 11 (`0x140adaa20` case 10): the walk to the Repair's point; at it (1, 2), failed
    /// (3), gone (the list empty, 1) or its target let go (5) the Repair ends; a target let go
    /// with 4 lets the Repair's go too (slot 0x68 with none, [`RepairAction::drop_target`]) and
    /// walks to the point again (`set_state(11)`).
    pub(super) fn walking_to_point(
        &mut self,
        c: &mut Step<'_>,
        repairer: &mut Unit,
        me: Me,
        reads: &RepairReads,
    ) -> Result<(), GenieError> {
        match self.step_walk(c, repairer, me, reads)? {
            0 => Ok(()),
            4 => {
                self.drop_target(c, repairer, reads)?;
                self.set_state(c, repairer, WALK_POINT, reads)
            }
            _ => self.set_state(c, repairer, DONE, reads),
        }
    }

    /// State 4 (`0x140adaa20` case 3): the quiet `MoveTo` toward the target walks
    /// ([`RepairAction::step_walk`]); once it is at it, waits or failed (or gone: 1, 2, 3), the
    /// unit within reach of the target ([`RepairAction::reaches`]) faces it (6), one with a
    /// quick path to it within [`REACH`] (slot 0x3e0, `canQuickPath`) waits (10), any other
    /// fails (13). Its target let go (4, 5) lets the Repair's go (slot 0x68 with none,
    /// [`RepairAction::drop_target`]) and ends the Repair (`set_state(1)`).
    pub(super) fn walking(
        &mut self,
        c: &mut Step<'_>,
        repairer: &mut Unit,
        me: Me,
        reads: &RepairReads,
    ) -> Result<(), GenieError> {
        match self.step_walk(c, repairer, me, reads)? {
            0 => return Ok(()),
            4 | 5 => {
                self.drop_target(c, repairer, reads)?;
                return self.set_state(c, repairer, DONE, reads);
            }
            _ => {}
        }
        if Self::reaches(repairer, reads) {
            return self.set_state(c, repairer, FACE, reads);
        }
        let next = if self.can_path(c, repairer, reads)? {
            WAIT
        } else {
            FAILED
        };
        self.set_state(c, repairer, next, reads)
    }

    /// Slot 0x3e0 (`canQuickPath`) of the repairer to its target within [`REACH`]: the old
    /// short pather's quick path ([`crate::pathing::hull_path::quick_reach`]) with its master's
    /// x radius and its terrain restriction's mask.
    fn can_path(
        &self,
        c: &mut Step<'_>,
        repairer: &Unit,
        reads: &RepairReads,
    ) -> Result<bool, GenieError> {
        let spatial = &mut c.w.spatial;
        let terrain = spatial
            .terrain
            .as_ref()
            .ok_or(crate::pathing::hull_path::NO_TERRAIN)?;
        let r = reads
            .restriction
            .ok_or(crate::pathing::hull_path::NO_RESTRICTION)?;
        let mask = terrain.restriction_mask(r);
        crate::pathing::hull_path::quick_reach(
            spatial,
            repairer.id,
            self.target,
            mask,
            reads.radius[0],
            REACH,
        )
    }

    /// The quiet `MoveTo`'s update and its code (`FUN_140a992b0`, [`MoveToAction::sub_update`]),
    /// the villager test and the stance deciding a give-up: 0 while it walks or waits, 1 once it
    /// is at its goal (or gone), 3 once it failed, 5 once its target was let go (a target in a
    /// state above 6, [`MoveToAction::check_held`]). Its start shows its sprite.
    pub(super) fn step_walk(
        &mut self,
        c: &mut Step<'_>,
        repairer: &mut Unit,
        me: Me,
        reads: &RepairReads,
    ) -> Result<u8, GenieError> {
        let Some(move_to) = &mut self.move_to else {
            return Ok(1);
        };
        let held = c.w.object_state(move_to.target);
        let (mut walk, beside) = c.w.walk_beside(c.pace(), &mut repairer.body, me);
        let shown = (move_to.state == MoveTo::Start).then_some(Shown {
            sprite: &mut repairer.sprite,
            rng: beside.sprite_rng,
            graphic: move_to.sprite,
        });
        let gives_up = GiveUp {
            waits: reads.waits,
            stance: reads.stance,
        };
        let mut inbox = Inbox::of(&mut repairer.ai, repairer.id, repairer.life.state);
        let ask = PathAsk::default();
        move_to.sub_update(&mut walk, ask, gives_up, held, shown, &mut inbox)
    }

    /// The reach test of states 4 and 10: the footprints' gap (`FUN_14118f680`) less
    /// [`MOVING_REACH`] for a moving target or a scenario's game mode at most [`REACH`]
    /// (`COMISS REACH, gap` with `JC`/`JNC`: a NaN gap is out of reach).
    pub(super) fn reaches(repairer: &Unit, reads: &RepairReads) -> bool {
        let Some(t) = reads.target else {
            return false;
        };
        let gap = Self::gap(repairer, reads, t);
        !crate::math::fmath::below_or_unordered(REACH, gap)
    }

    /// The footprints' gap less the moving target's or the scenario's margin.
    pub(super) fn gap(repairer: &Unit, reads: &RepairReads, t: Repairee) -> f32 {
        let margin = if reads.scenario || t.moves {
            MOVING_REACH
        } else {
            0.0
        };
        let at = repairer.body.position();
        footprint_gap(at, reads.radius, t.position, t.radius) - margin
    }

    /// State 7 (`0x140adaa20` case 6): a target that left the action's point (`UCOMISS` on x
    /// and y: a NaN has) within reach (`COMISS gap, REACH` with `JBE`: a NaN is) gives the point
    /// its place; out of reach the unit walks again (4). Then the work of the update
    /// ([`work_amount`]) goes to the target's repair ([`crate::construction::repair::repair_target`]):
    /// done (1) a target of a type above 69 has the armor its charge's hits took back
    /// (`FUN_1412066a0`: `+0x9e4` zeroed) and the action ends (`set_state(1)`); the player short
    /// of resources (2) posts notify 0x1fb (the game's event 0x6b to its interface,
    /// `FUN_140c898a0`, is not held) and ends it too.
    pub(super) fn work(
        &mut self,
        c: &mut Step<'_>,
        repairer: &mut Unit,
        reads: &RepairReads,
    ) -> Result<(), GenieError> {
        let t = reads.target.ok_or(GenieError::unreachable(
            "a Repair working without its target",
        ))?;
        let moved = t.position[0] != self.point[0] || t.position[1] != self.point[1];
        if moved {
            let gap = Self::gap(repairer, reads, t);
            if REACH < gap {
                return self.set_state(c, repairer, MOVE, reads);
            }
            self.point = t.position;
        }
        let repairers = reads.counted_in.map(|(_, n)| n);
        let amount = work_amount(reads.rate, self.task.work_value, seconds(c.dt), repairers);
        let (answer, kind) = crate::construction::repair::repair_target(c, self.target, amount)?;
        match answer {
            1 => {
                if COMBAT <= kind
                    && let Some((_, u)) = c.w.object_mut(self.target)
                {
                    u.armor_removed = [0; 2];
                }
                self.set_state(c, repairer, DONE, reads)
            }
            2 => {
                notify(repairer, crate::unit_ai::ACTION_INVALIDATED)?;
                self.set_state(c, repairer, DONE, reads)
            }
            _ => Ok(()),
        }
    }

    /// The repairers of the object `FUN_140ad9050` counts them in (`+0x9e8`) change by `by` at
    /// once; nothing without one.
    fn count(c: &mut Step<'_>, reads: &RepairReads, by: i8) {
        if let Some((id, _)) = reads.counted_in
            && let Some((_, u)) = c.w.object_mut(id)
        {
            let n = u.repairers_mut();
            *n = n.wrapping_add_signed(by);
        }
    }

    /// Slot 0x68 with none (`FUN_140ada470`): a counted action leaves its target's repairers
    /// (`FUN_140ad9050`'s object), then the base lets the target go (`FUN_140a7a230`: the
    /// target's slot 0x180, [`crate::world::lets::release`]).
    pub(super) fn drop_target(
        &mut self,
        c: &mut Step<'_>,
        repairer: &mut Unit,
        reads: &RepairReads,
    ) -> Result<(), GenieError> {
        if self.counted {
            Self::count(c, reads, -1);
            self.counted = false;
        }
        let target = std::mem::replace(&mut self.target, -1);
        if target != -1 {
            let by = repairer.releaser(c);
            crate::world::lets::release(c, repairer, target, by)?;
        }
        Ok(())
    }

    /// `set_state` (`0x140ad9c60`): a counted action leaves its target's repairers, the
    /// sub-actions go (a walking `MoveTo`'s teardown stops the unit), then by the new state: 1
    /// lets the target go (slot 0x68 with none) and posts notify 0x1fa unless quiet; 1, 2 and 3
    /// (with notify 0x202) give the unit its standing sprite; 4 takes the target's place and a
    /// `MoveTo` to the target; 6 takes its place and shows the task's moving sprite; 7 shows its
    /// proceeding sprite (its sound, slot 0x4c0, is not held) and counts the action among the
    /// target's repairers; slot 0x68 shows the master's standing sprite for none. 10 waits a
    /// second with the standing sprite (a ship target's move to the unit, [`SHIP_REPAIR`], is
    /// not modelled); 11 walks to the point (a quiet `MoveTo`).
    pub(super) fn set_state(
        &mut self,
        c: &mut Step<'_>,
        repairer: &mut Unit,
        state: u8,
        reads: &RepairReads,
    ) -> Result<(), GenieError> {
        if self.counted {
            Self::count(c, reads, -1);
            self.counted = false;
        }
        if self.move_to.take().is_some()
            && let Some(m) = repairer.body.mover_mut()
        {
            m.stop()?;
        }
        self.state = state;
        self.enter(c, repairer, reads)
    }

    /// [`RepairAction::set_state`]'s part by the new state (`0x140ad9c60`'s switch).
    fn enter(
        &mut self,
        c: &mut Step<'_>,
        repairer: &mut Unit,
        reads: &RepairReads,
    ) -> Result<(), GenieError> {
        let standing = |c: &mut Step<'_>, repairer: &mut Unit| {
            repairer.show_standing(
                c.w.masters.get(c.player),
                &c.w.graphics,
                c.w.streams.get(Stream::Sprite),
            )
        };
        match self.state {
            DONE => {
                self.drop_target(c, repairer, reads)?;
                if !self.quiet {
                    notify(repairer, crate::unit_ai::ACTION_COMPLETED)?;
                }
                standing(c, repairer)?;
            }
            HALTED => standing(c, repairer)?,
            IDLE => {
                notify(repairer, crate::unit_ai::REQUIRES_SEARCH)?;
                standing(c, repairer)?;
            }
            MOVE => self.enter_walk(c, repairer, reads)?,
            FACE => {
                if let Some(t) = reads.target {
                    self.point = t.position;
                }
                show(c, repairer, self.task.moving, reads.standing)?;
            }
            WORK => {
                show(c, repairer, self.task.proceeding, reads.standing)?;
                if reads.counted_in.is_some() {
                    Self::count(c, reads, 1);
                    self.counted = true;
                }
            }
            WAIT => {
                standing(c, repairer)?;
                self.timer = ONE;
                if reads.target.is_some_and(|t| SHIPS.contains(&t.class)) {
                    return Err(SHIP_REPAIR);
                }
            }
            WALK_POINT => {
                let move_reads = repairer.move_to_reads(&c.w.masters, c.player, &c.w.graphics)?;
                self.move_to = self.to_point(self.point, &move_reads)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// `set_state(4)`'s case: the target's place is the point, and the `MoveTo` to it starts
    /// ([`RepairAction::walk`], none for a target the world does not hold).
    ///
    /// # Errors
    ///
    /// A repairer without its master, a target whose master its table lacks, a sprite the table
    /// lacks.
    fn enter_walk(
        &mut self,
        c: &Step<'_>,
        repairer: &Unit,
        reads: &RepairReads,
    ) -> Result<(), GenieError> {
        if let Some(t) = reads.target {
            self.point = t.position;
        }
        let move_reads = repairer.move_to_reads(&c.w.masters, c.player, &c.w.graphics)?;
        let to = move_reads.walk_to(c.w, self.target)?;
        self.move_to = self.walk(&move_reads, to)?;
        Ok(())
    }

    /// `set_state(4)` from the work command's new target `to` (slot 0x48, the caller's
    /// prelude): its place is the point, the `MoveTo` goes (a walking one stops the unit) and a
    /// new one to the target starts ([`RepairAction::walk`] with `move_reads`).
    ///
    /// # Errors
    ///
    /// A sprite the table lacks, and the unit's stop that the motion step does not model.
    pub fn walk_again(
        &mut self,
        to: WalkTo,
        mover: Option<&mut crate::movement::Mover>,
        move_reads: &MoveToReads<'_>,
    ) -> Result<(), GenieError> {
        let walk = self.walk(move_reads, Some(to))?;
        if self.move_to.take().is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        self.point = to.at;
        self.state = MOVE;
        self.move_to = walk;
        Ok(())
    }

    /// Slot 8 (`FUN_140ad9700`), a move to `point` (its target argument unread): the point is
    /// the action's, slot 0x68 with none (a counted repairer leaves the repairers, the target
    /// goes), then `set_state(11)` (the `MoveTo` goes, a walking one stopping the unit; a quiet
    /// one to the point starts, [`RepairAction::to_point`] with `move_reads`). Returns the target
    /// let go and whether it counted.
    ///
    /// # Errors
    ///
    /// A sprite the table lacks, and the unit's stop that the motion step does not model.
    pub fn take_point(
        &mut self,
        point: [f32; 3],
        mover: Option<&mut crate::movement::Mover>,
        move_reads: &MoveToReads<'_>,
    ) -> Result<(i32, bool), GenieError> {
        let walk = self.to_point(point, move_reads)?;
        self.point = point;
        let counted = std::mem::replace(&mut self.counted, false);
        let old = std::mem::replace(&mut self.target, -1);
        if self.move_to.take().is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        self.state = WALK_POINT;
        self.move_to = walk;
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
        self.state = HALTED;
        Ok(counted)
    }
}
