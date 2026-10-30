//! The Build's `MoveTo`s (`0x140aae200` cases 4, 8, 11 and 223) through [`MoveToReads`]: the one to
//! its foundation, and the sub-action list's update.

use crate::GenieError;
use crate::construction::build::{BuildAction, BuildReads, BuildTask, SITE_GONE};
use crate::movement::move_to::{MoveTo, MoveToAction, MoveToReads, Shown, TargetMoved, WalkTo};
use crate::movement::{Me, PathAsk};
use crate::objects::unit::Unit;
use crate::unit_ai::Inbox;
use crate::world::step::Step;

impl BuildTask {
    /// The `MoveTo` of `set_state(4)` with this task (`0x140aae200` case 4: `FUN_140aa0df0` with
    /// the task's range `+0x30` and moving sprite `+0x40`) to the foundation `to`, or without
    /// one ([`MoveToReads::to_nothing`]), with what it reads of the builder (`move_reads`).
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
            || move_reads.to_nothing(self.range, self.moving),
            |to| move_reads.to_object(to, self.range, self.moving, TargetMoved::Follows),
        )
    }
}

impl BuildAction {
    /// The `MoveTo` of `set_state(4)` toward the Build's foundation (its place and the
    /// builder's footprint gap to it, its state as the world holds it now), or without one
    /// ([`BuildTask::walk`]).
    ///
    /// # Errors
    ///
    /// A builder without movement state or its master, a foundation without a place, a sprite
    /// the table lacks.
    pub(super) fn walk_to_foundation(
        &self,
        c: &Step<'_>,
        builder: &Unit,
    ) -> Result<(Option<WalkTo>, Option<MoveToAction>), GenieError> {
        let move_reads = builder.move_to_reads(&c.w.masters, c.player, &c.w.graphics)?;
        let to = if self.target == -1 {
            None
        } else {
            Some(move_reads.walk_to(c.w, self.target)?.ok_or(SITE_GONE)?)
        };
        Ok((to, self.task.walk(&move_reads, to)?))
    }

    /// The sub-action list's update (`FUN_140a992b0`: its first action's, the quiet `MoveTo`'s,
    /// [`MoveToAction::sub_update`]) and its code: 0 while it walks or waits, 1 once it ended,
    /// 3 once it failed, 5 once its target was let go; 1 for an empty list ("There is no
    /// list."). Its start shows its sprite; the motion step gives up as [`BuildAction::gives_up`]
    /// says.
    ///
    /// # Errors
    ///
    /// The walk's and the notifies'.
    pub(super) fn step_walk(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        me: Me,
        reads: &BuildReads,
    ) -> Result<u8, GenieError> {
        let gives_up = Self::gives_up(c, builder, reads);
        let Some(move_to) = &mut self.move_to else {
            return Ok(1);
        };
        let held = c.w.object_state(move_to.target);
        let (mut walk, beside) = c.w.walk_beside(c.pace(), &mut builder.body, me);
        let shown = (move_to.state == MoveTo::Start).then_some(Shown {
            sprite: &mut builder.sprite,
            rng: beside.sprite_rng,
            graphic: move_to.sprite,
        });
        let mut inbox = Inbox::of(&mut builder.ai, builder.id, builder.life.state);
        let ask = PathAsk::default();
        move_to.sub_update(&mut walk, ask, gives_up, held, shown, &mut inbox)
    }
}
