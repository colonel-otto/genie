//! State 8: the drop-off at the drop site; the target let go, the teardown and a case 6
//! without a target.

use crate::GenieError;
use crate::gathering::hunt::{
    ARRIVED, DROP, DropOff, HuntAction, Lead, PointWalker, SEARCH, Shown, asked_drop_off,
    resource_kind,
};
use crate::movement::Mover;
use crate::movement::move_to::{
    Goal, MoveTo, MoveWait, Walk, start_walk_or_give_up, target_moved, walk_or_give_up,
};

/// What a drop-off's update did ([`HuntAction::update_drop`],
/// [`crate::gathering::gather::GatherAction::update_drop`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dropped {
    /// It walks or waits.
    Going,
    /// The load went to the player: its resource and amount, which the caller adds.
    Load(i16, f32),
    /// Its wait ended: it tries again (`set_state(8)`, the caller's).
    Again,
    /// The drop site does not take the load: the action idles (`set_state(2)`; the caller lets
    /// the targets go).
    Idle,
}

/// The wait of a hunt's drop-off that cannot go on (`0xc0400000`).
pub(super) const DROP_WAIT: f32 = -3.0;
/// A hunt's drop-off reaches its drop site within this squared footprint gap (`DAT_14330eb8c`).
pub(super) const DROP_REACH: f32 = crate::math::image::hunt::DROP_REACH;

impl HuntAction {
    /// `set_state(8)` (`0x140ac60e0` case 8) with a load and its drop site (`carry_to`; without one
    /// the drop site search `FUN_1411671e0` would find it, not modelled): the sub-actions go and a
    /// quiet `MoveTo` to the drop site within the task's range starts (`FUN_140aa0df0`, as
    /// [`HuntAction::set_walk`]'s: within 0.01 of its footprint (`gap`) the unit is there already).
    pub(crate) fn set_drop(&mut self, gap: f32, mover: &mut Mover) -> Result<(), GenieError> {
        self.tear_down(Some(mover))?;
        self.state = DROP;
        if self.carry_to == -1 {
            return Err(GenieError::unsupported(
                "a drop-off without its drop site (its search)",
            ));
        }
        self.walk_range = self.task.range;
        self.move_wait = MoveWait::default();
        self.move_to = Some(if ARRIVED < gap {
            if mover.stats.speed == 0.0 {
                return Err(GenieError::unsupported(
                    "a MoveTo of a unit without speed (state 13)",
                ));
            }
            MoveTo::Start
        } else {
            MoveTo::Arrived
        });
        Ok(())
    }

    /// State 8's update (`0x140ac7a00` case 7): while its timer is below 0 the hunt waits (the
    /// timer runs by dt); at its end the drop site goes (`FUN_140a7a190` with none) and the hunt
    /// tries again (`set_state(8)`, [`Dropped::Again`], the caller's). Else the `MoveTo` to the
    /// drop site walks, waits after the motion step gave up (a human player's villager once:
    /// [`HuntAction::give_up_walk`]) or fails; a failed one (codes 3 to 5) makes the hunt wait 3
    /// seconds and stops the unit's animation (`FUN_141161850`). Once it ended (its update in
    /// state 1 returns 1: the list deletes it and its destructor stops the unit) the load goes to
    /// the player at a drop site that takes it within a tile (`site`; one that does not,
    /// `set_state` 2, is not modelled; a farther one makes the hunt wait 3 seconds too): the
    /// resource of what the unit carries (`FUN_140fdd4f0`: food for 15 to 17, nothing outside 0
    /// to 3, 9 and 56, then 0 beyond the player's attributes, `FUN_140efe5b0`), which the caller
    /// adds; the unit carries nothing after (`FUN_14119d310` with -1). A hunt with a target walks
    /// back to it (`set_state` 4), one without searches for a new one (the caller's).
    #[allow(clippy::too_many_arguments)]
    pub fn update_drop(
        &mut self,
        mut walk: Walk<'_>,
        held_kind: &mut i16,
        held: &mut f32,
        attributes: usize,
        site: Option<DropOff>,
        mut shown: Shown<'_>,
        who: PointWalker,
    ) -> Result<Dropped, GenieError> {
        if self.timer < 0.0 {
            self.timer += walk.dt;
            if self.timer < 0.0 {
                return Ok(Dropped::Going);
            }
            self.timer = 0.0;
            return Ok(Dropped::Again);
        }
        match self.move_to {
            Some(MoveTo::Start) => {
                self.move_to = Some(MoveTo::Moving);
                let range = self.walk_range;
                walk.me.action_state = self.state;
                let walked = start_walk_or_give_up(walk, Goal::Target(self.carry_to), range, true)?;
                shown.walking(walked.unwrap_or(MoveTo::Moving));
                match walked {
                    Some(state) => self.move_to = Some(state),
                    None => self.give_up_walk(who),
                }
                return Ok(Dropped::Going);
            }
            Some(MoveTo::Moving) => {
                if target_moved(&walk, self.carry_to, true)?.again() {
                    self.move_to = Some(MoveTo::Start);
                    return Ok(Dropped::Going);
                }
                walk.me.action_state = self.state;
                match walk_or_give_up(walk)? {
                    Some(m) => self.move_to = Some(m),
                    None => self.give_up_walk(who),
                }
                return Ok(Dropped::Going);
            }
            Some(MoveTo::Waiting) => {
                self.move_wait.timer += walk.dt;
                if self.move_wait.delay <= self.move_wait.timer {
                    self.move_wait.timer = 0.0;
                    self.move_to = Some(MoveTo::Start);
                }
                return Ok(Dropped::Going);
            }
            Some(MoveTo::Failed) => {
                self.timer = DROP_WAIT;
                walk.set_anim_speed(0.0);
                return Ok(Dropped::Going);
            }
            Some(MoveTo::Arrived) => {
                self.move_to = None;
                walk.body
                    .mover_mut()
                    .ok_or(crate::world::HUNTER_UNMOVING)?
                    .stop()?;
            }
            None => {
                return Err(GenieError::unsupported(
                    "a Hunt in state 8 without its MoveTo",
                ));
            }
        }
        let site = site.ok_or(GenieError::unsupported(
            "a drop-off whose drop site is gone",
        ))?;
        if !site.takes {
            return Err(GenieError::unsupported(
                "a drop site that does not take the load (set_state 2)",
            ));
        }
        if DROP_REACH < site.gap_squared {
            self.timer = DROP_WAIT;
            walk.set_anim_speed(0.0);
            return Ok(Dropped::Going);
        }
        let kind = resource_kind(*held_kind, attributes);
        let amount = *held;
        *held_kind = -1;
        *held = 0.0;
        Ok(Dropped::Load(kind, amount))
    }

    /// Slot 0x78 (`FUN_140a78060`, the base action's; `UNIT_ORDER`'s drop-off) of a unit carrying
    /// some of `held_kind` (`+0x310` above 0, which the caller checked): [`asked_drop_off`] on the
    /// task's resource.
    #[must_use]
    pub fn drop_off_asked(&self, held_kind: i16, attributes: usize) -> bool {
        asked_drop_off(self.task.resource, held_kind, attributes)
    }

    /// Slot 0x68 with none (`FUN_140ac7980`, [`HuntAction::set_target`]): the target goes; the
    /// old target, which the caller releases (slot 0x180).
    pub const fn let_go(&mut self) -> i32 {
        let target = self.target;
        self.set_target(-1);
        target
    }

    /// Slot 0x68 (`FUN_140ac7980`): the base's change of target (`FUN_140a7a230`, whose release
    /// and take are the caller's), then the lead clears ([`Lead::NEW`]: `moved_sheep` and
    /// `sheep_still_time` 0, the start −1).
    pub const fn set_target(&mut self, target: i32) {
        self.target = target;
        self.lead = Lead::NEW;
    }

    /// The teardown (`FUN_140a73fc0`): the sub-actions go; the caller releases the target (slot
    /// 0x180).
    pub fn tear_down(&mut self, mut mover: Option<&mut Mover>) -> Result<(), GenieError> {
        if let Some(mut a) = self.attack.take() {
            a.tear_down(mover.as_deref_mut())?;
        }
        if self.move_to.take().is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        Ok(())
    }

    /// Case 6 without a target (`0x140ac7a00` at `0x140ac9029`): a load is carried off
    /// (`set_state(8)`, true, the caller's); with none, or one that is not a number (`COMISS`
    /// with `JBE`), the hunt searches (`set_state(3)`: [`SEARCH`] here, the rest the caller's).
    pub(super) fn ends_without_target(&mut self, held: f32) -> bool {
        if crate::math::fmath::at_most_or_unordered(held, 0.0) {
            self.state = SEARCH;
            false
        } else {
            true
        }
    }
}
