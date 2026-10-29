//! The Build action's start (state 4 toward the foundation) and its update, which runs the
//! action's state; the AI's build orders whose foundation is gone.

use crate::GenieError;
use crate::construction::build::{
    ARRIVED, BUILDER_UNMOVING, BuildAction, BuildReads, BuildTask, CLEAR, DONE, DROP, FAILED,
    FINISHED, HALTED, IDLE, MASTER_621, MASTER_1889, MOVE, PLACE, WAIT, WALK_POINT, WORK,
};
use crate::movement::move_to::{
    Goal, MoveDefaults, MoveTo, MoveWait, Sprites, start_walk_shown, walk_or_give_up,
};
use crate::movement::{Me, PathAsk};
use crate::objects::lifecycle::seconds;
use crate::objects::masters::class;
use crate::objects::teardown::HeadCode;
use crate::objects::unit::Unit;
use crate::unit_ai::OrderKind;
use crate::unit_ai::UnitAi;
use crate::world::step::Step;

impl BuildAction {
    /// The action's start (slot 5, `0x140aadd80`) on the foundation `target` at `at`: state 4
    /// for a target, whose `set_state` (`0x140aae200`) takes the foundation's place as the point
    /// and creates the `MoveTo` to it (`FUN_140aa0df0` with the task's moving sprite); the
    /// `MoveTo` starts arrived if the footprints touch (`gap`).
    #[must_use]
    pub fn start(target: i32, at: [f32; 3], gap: f32, task: BuildTask) -> Self {
        Self {
            target,
            state: MOVE,
            move_to: Some(if gap > ARRIVED {
                MoveTo::Start
            } else {
                MoveTo::Arrived
            }),
            goal: None,
            walk_target: target,
            point: at,
            task,
            added: false,
            timer: 0.0,
            move_wait: MoveWait::default(),
            move_sprite: task.moving,
            site: -1,
            drop_range: 0.0,
        }
    }

    /// The sprite its `MoveTo`'s `set_state(4)` shows: the one it was made with
    /// ([`BuildAction::move_sprite`]), else its constructor's default from `defaults`
    /// (`FUN_140aa0df0`'s toward the foundation or a drop site, states 4 and 8;
    /// `FUN_140aa0d30`'s toward a point, states 11 and 223).
    #[must_use]
    pub const fn move_shown(&self, defaults: MoveDefaults) -> Option<i16> {
        match self.state {
            MOVE | DROP => defaults.to_object(self.move_sprite),
            _ => defaults.to_point(self.move_sprite),
        }
    }

    /// `set_state(4)` (and slot 0x48 or 0x68 before it) on `target` at `at`: a new `MoveTo` as
    /// [`BuildAction::start`]'s; the second target and the drop-off range stay.
    #[must_use]
    pub(super) fn restarted(&self, target: i32, at: [f32; 3], gap: f32) -> Self {
        let mut new = Self::start(target, at, gap, self.task);
        new.site = self.site;
        new.drop_range = self.drop_range;
        new
    }

    /// One update (slot 6, `0x140ab0370`) of `builder` (`me`) through its step, with what it
    /// reads as it starts (`reads`), which first updates the `MoveTo` (`0x140aa2a70`); a moving
    /// `MoveTo` calls the motion step (object slot 193). Before its state's case it checks its
    /// foundation: one the world lost ([`BuildAction::lost_foundation`]), a human player's
    /// pasture already built ([`BuildAction::pasture_built`]), and one no longer under
    /// construction ([`BuildAction::foundation_done`]); then the end of the work (224), the
    /// `MoveTo`'s target, the drop-off (8) and the state's case ([`BuildAction::update_state`]).
    /// Each effect on the builder and the world happens where the game makes it; the code is
    /// the update's (its 1 and 3: the action list deletes the Build).
    ///
    /// # Errors
    ///
    /// What the states, the walks and the work do not model.
    pub(crate) fn update(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        me: Me,
        reads: &BuildReads,
    ) -> Result<HeadCode, GenieError> {
        // state 3 without a target: the update does nothing (0x140ab0370's default case)
        if self.state == IDLE && self.target == -1 {
            return Ok(HeadCode::Keeps);
        }
        if self.lost_foundation(c, reads) {
            self.idle(c, builder, reads)?;
            return Ok(HeadCode::Keeps);
        }
        // the second target's id names no object of the world: FUN_140a7a190 with none, whose
        // release finds nothing to call
        if self.site != -1 && c.w.spatial.get(self.site).is_none() {
            self.site = -1;
        }
        let mut walked_from = None;
        if reads.site.is_some() {
            if self.pasture_built(c, builder, reads)? {
                return Ok(HeadCode::Deletes);
            }
            walked_from = self.foundation_done(c, builder, reads)?;
        }
        if self.state == FINISHED {
            return self.after_work(c, builder, reads);
        }
        // the MoveTo's update first (`0x140aa2a70` through `FUN_140a992b0`, states 4 and 223):
        // its target in a state above 6 lets it go (slot 0x68 with none) and the quiet MoveTo
        // returns 5. A target the world removed is no longer the MoveTo's: its removal let the
        // MoveTo go (slot 0x180's release clears the pointer and the id), which walks on.
        if self.walk_target != -1 && reads.walk_target_state.is_none() {
            self.walk_target = -1;
        }
        if self.state == DROP {
            return self.update_drop(c, builder, me, reads);
        }
        if matches!(self.state, MOVE | CLEAR)
            && self.move_to.is_some()
            && self.walk_target != -1
            && reads.walk_target_state.is_some_and(|s| 6 < s)
        {
            self.walk_target = -1;
            self.lose_foundation(c, builder, reads)?;
            return Ok(HeadCode::Keeps);
        }
        self.update_state(c, builder, me, reads, walked_from)
    }

    /// The target's id names no object of the world (`FUN_1415417a0`; the builder's player
    /// does not hold it either, `reads`): slot 0x68 with none (its builders' count went with
    /// it); then a Build in state 2 or 8 goes on, any other waits (true: the caller's
    /// `set_state(3)`, and the update returns 0).
    fn lost_foundation(&mut self, c: &Step<'_>, reads: &BuildReads) -> bool {
        if self.target == -1 || reads.site.is_some() || c.w.spatial.get(self.target).is_some() {
            return false;
        }
        self.added = false;
        self.target = -1;
        !matches!(self.state, 2 | 8)
    }

    /// A human player's builder of a pasture (master 1889) already built: its unit AI has no
    /// order and no action (`FUN_140c64b60`, `FUN_140c64b50` with −1), the unit tells it (notify
    /// 0x21d, the build action's value and the pasture the stack unit made, `FUN_1411c5f70`; 0
    /// when the slice does not hold it), the foundation goes (slot 0x68 with none: out of the
    /// builders, the foundation let go), the load goes to the player (`FUN_140aadc50`: a kind
    /// the player has, the foods as food) and the Build ends (`set_state(1)`); the update
    /// returns 1. True when it ended.
    fn pasture_built(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        reads: &BuildReads,
    ) -> Result<bool, GenieError> {
        let computer = c.w.players[c.player].computer;
        let site = Self::site_unit(c, reads);
        let pasture = site
            .and_then(|u| u.master(&c.w.masters, c.player).ok())
            .is_some_and(|m| m.id == MASTER_1889);
        let built = site.is_some_and(|u| u.life.state != 0);
        if computer || !pasture || !built {
            return Ok(false);
        }
        let Some(ai) = builder.ai.as_mut() else {
            return Ok(false);
        };
        ai.end_order_and_action()?;
        builder.post_own(
            crate::unit_ai::PASTURE_BUILT,
            [
                crate::unit_ai::ActionKind::Build.value(),
                reads.stack_made.unwrap_or(0),
                0,
            ],
        )?;
        self.uncount(c)?;
        if self.target != -1 {
            let target = std::mem::replace(&mut self.target, -1);
            crate::world::lets::release(c, builder, target, reads.by)?;
        }
        let (kind, amount) = (builder.life.attribute_type, builder.life.amount);
        let attributes = c.w.players[c.player].attributes.len();
        if usize::try_from(kind).is_ok_and(|k| k < attributes) {
            let kind = if matches!(kind, 15..=17) { 0 } else { kind };
            Self::drop_load(c, builder, kind, amount)?;
        }
        self.end(c, builder, DONE)?;
        Ok(true)
    }

    /// A foundation no longer under construction (a state above 0) unless the Build already
    /// ended (224, 4, 8): master 621 and the gates (class 39) end the work once built
    /// (`set_state(224)`), as any other below state 3 does; else `set_state(4)` toward it, then
    /// slot 0x68 with none lets it go. The foundation the `MoveTo` walks to unless it refused
    /// it.
    fn foundation_done(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        reads: &BuildReads,
    ) -> Result<Option<i32>, GenieError> {
        let Some(site) = Self::site_unit(c, reads) else {
            return Ok(None);
        };
        let state = site.life.state;
        if state == 0 || matches!(self.state, FINISHED | MOVE | 8) {
            return Ok(None);
        }
        let master = site.master(&c.w.masters, c.player).ok();
        let special = master.is_some_and(|m| m.id == MASTER_621 || m.class == class::GATE);
        let unbuilt = site
            .construction
            .is_some_and(|k| k.progress < f32::from(k.build_time));
        if (special || state < 3) && !(special && 2 < state && unbuilt) {
            self.finish(c, builder, reads)?;
            return Ok(None);
        }
        self.restart(c, builder, Some(state))?;
        let walked_from = (self.walk_target != -1).then_some(self.target);
        let target = std::mem::replace(&mut self.target, -1);
        crate::world::lets::release(c, builder, target, reads.by)?;
        Ok(walked_from)
    }

    /// The update's case by the state and its `MoveTo` (`walked_from`: the foundation a
    /// `set_state(4)` of this update walks to), and the update's code.
    fn update_state(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        me: Me,
        reads: &BuildReads,
        walked_from: Option<i32>,
    ) -> Result<HeadCode, GenieError> {
        match (self.state, self.move_to) {
            // the MoveTo at its end (1), or no sub-action (the list's update returns 1)
            (4, Some(MoveTo::Arrived) | None) => self.walk_ended(c, builder, reads)?,
            (4, Some(MoveTo::Start)) => {
                self.walk_to_foundation(c, builder, me, reads, walked_from)?;
            }
            (4 | CLEAR | WALK_POINT, Some(MoveTo::Moving)) => {
                self.walk_on(c, builder, me, reads)?;
            }
            (4 | CLEAR | WALK_POINT, Some(MoveTo::Waiting)) => self.walk_wait(c.dt),
            // its update returned 3
            (4 | CLEAR, Some(MoveTo::Failed)) => self.walk_failed(c, builder, reads)?,
            // case 0: the update returns 1, the action list removes it
            (DONE, _) => return Ok(HeadCode::Deletes),
            // case 1: set_state(13); the update returns 0
            (HALTED, _) => self.end(c, builder, FAILED)?,
            (FAILED, _) => return self.failed(c, builder, reads),
            (WORK, _) => self.face_and_place(c, builder, reads)?,
            (WAIT, _) => self.wait_out(c, builder, reads)?,
            (PLACE, _) => self.work(c, builder, reads)?,
            (CLEAR | WALK_POINT, Some(MoveTo::Start)) => {
                self.walk_to_goal(c, builder, me, reads)?;
            }
            // case 0xde: the MoveTo ended (its update in state 1 returns 1): a villager goes
            // back to the foundation (set_state 4)
            // (its update returned 1, or the list's without a sub-action: in reach or not)
            (CLEAR, Some(MoveTo::Arrived) | None) => self.back(c, builder)?,
            // case 10, the walk to the point: the MoveTo at it (1), failed (3) or gone (the list
            // empty, 1) makes the Build wait (set_state(3)); its target gone (4) cannot be, as
            // it has none
            (WALK_POINT, Some(MoveTo::Arrived | MoveTo::Failed) | None) => {
                self.idle(c, builder, reads)?;
            }
            // the default case (states 0, 3, 5, 8 to 10, 12 and 14 to 221): the update returns 0
            _ => {}
        }
        Ok(HeadCode::Keeps)
    }

    /// The walking `MoveTo` (its state 4) of states 4, 7 and 10: the motion step; its giving up
    /// makes the `MoveTo` wait or fail ([`MoveWait::give_up`]).
    ///
    /// # Errors
    ///
    /// The motion step's.
    fn walk_on(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        me: Me,
        reads: &BuildReads,
    ) -> Result<(), GenieError> {
        let (waits, stance) = Self::gives_up(c, builder, reads);
        self.move_to = Some(match walk_or_give_up(c.walk(&mut builder.body, me))? {
            Some(state) => state,
            None => self.move_wait.give_up(waits, stance),
        });
        Ok(())
    }

    /// The waiting `MoveTo` (its state 2): the world's `dt` ms onto its timer; at its end it asks
    /// again (`set_state(3)`; COMISS, JC: a NaN timer waits on); its update returns 0.
    fn walk_wait(&mut self, dt: u32) {
        self.move_wait.timer += seconds(dt);
        if self.move_wait.delay <= self.move_wait.timer {
            self.move_wait.timer = 0.0;
            self.move_to = Some(MoveTo::Start);
        }
    }

    /// Case 0xc: the unit's slot 0x328 posts notify 0x1f9 (the Build's action value, 0, 0) to its
    /// unit AI; `set_state(2)` ([`BuildAction::halt_in`]: out of the builders, the `MoveTo`'s
    /// teardown, the unit's standing sprite, the targets let go) and the update returns 3: the
    /// action list removes the Build (`FUN_140a98e20`), as for 1.
    ///
    /// # Errors
    ///
    /// The notify's and the halt's.
    fn failed(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        reads: &BuildReads,
    ) -> Result<HeadCode, GenieError> {
        builder.post_own(
            crate::unit_ai::ACTION_FAILED,
            [crate::unit_ai::ActionKind::Build.value(), 0, 0],
        )?;
        self.halt_in(c, builder, reads)?;
        Ok(HeadCode::Deletes)
    }

    /// Case 0xdd: the wait runs down by the world's `dt`, then `set_state(7)` again
    /// ([`BuildAction::place`]); the update returns 0 either way.
    ///
    /// # Errors
    ///
    /// The placing's.
    fn wait_out(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        reads: &BuildReads,
    ) -> Result<(), GenieError> {
        self.timer -= seconds(c.dt);
        if 0.0 < self.timer {
            return Ok(());
        }
        self.timer = 0.0;
        self.place(c, builder, reads)
    }

    /// The `MoveTo`'s answer to the motion step giving up ([`MoveWait::give_up`]): a human
    /// player's villager waits (by its unit AI's stance).
    pub(super) fn gives_up(c: &Step<'_>, builder: &Unit, reads: &BuildReads) -> (bool, u8) {
        let computer = c.w.players[c.player].computer;
        let waits = reads.class == class::VILLAGER && !computer;
        (waits, builder.ai.as_ref().map_or(0, |ai| ai.stance))
    }

    /// The quiet `MoveTo` `set_state(4)` made (`FUN_140a7a170` with 1) walks to the foundation
    /// (`walked_from`, the one this update let go, else its target), within the task's range; one
    /// without a target (a `set_state(4)` without a foundation, or one that refused it) to its
    /// point. Its `set_state(4)` shows its sprite ([`BuildAction::move_shown`]) before the
    /// motion step. The motion step giving up makes it wait or fail ([`MoveWait::give_up`]).
    fn walk_to_foundation(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        me: Me,
        reads: &BuildReads,
        walked_from: Option<i32>,
    ) -> Result<(), GenieError> {
        let (waits, stance) = Self::gives_up(c, builder, reads);
        let shown = self.move_shown(reads.defaults);
        let (walk, beside) = c.w.walk_beside(c.pace(), &mut builder.body, me);
        let mut sprites = Sprites {
            sprite: &mut builder.sprite,
            rng: beside.sprite_rng,
            graphics: beside.graphics,
        };
        let shown = sprites.shown(shown)?;
        self.asked_again();
        self.move_to = Some(MoveTo::Moving);
        let range = self.task.range;
        let to = match (walked_from, self.goal) {
            (None, Some(p)) if self.target == -1 || self.walk_target == -1 => Goal::Point(p),
            _ => Goal::Target(walked_from.unwrap_or(self.target)),
        };
        let ask = PathAsk::default();
        self.move_to = Some(
            match start_walk_shown(walk, (to, range, true), ask, Some(shown))? {
                Some(state) => state,
                None => self.move_wait.give_up(waits, stance),
            },
        );
        Ok(())
    }

    /// The quiet `MoveTo` to the point of `set_state(223)` and `set_state(11)`, which shows its
    /// sprite ([`BuildAction::move_shown`]) as it turns to state 4; the motion step giving up
    /// makes it wait or fail ([`MoveWait::give_up`]).
    ///
    /// # Errors
    ///
    /// A walk to the point without one (`set_state(223)` and `set_state(11)` set it), and what
    /// the walk does not model.
    fn walk_to_goal(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        me: Me,
        reads: &BuildReads,
    ) -> Result<(), GenieError> {
        let (waits, stance) = Self::gives_up(c, builder, reads);
        let shown = self.move_shown(reads.defaults);
        let (walk, beside) = c.w.walk_beside(c.pace(), &mut builder.body, me);
        let mut sprites = Sprites {
            sprite: &mut builder.sprite,
            rng: beside.sprite_rng,
            graphics: beside.graphics,
        };
        let shown = sprites.shown(shown)?;
        self.asked_again();
        self.move_to = Some(MoveTo::Moving);
        let goal = self.goal.ok_or(GenieError::unreachable(
            "a Build's walk to its point without one (set_state 223, 11)",
        ))?;
        let ask = PathAsk::default();
        self.move_to = Some(
            match start_walk_shown(walk, (Goal::Point(goal), 0.0, true), ask, Some(shown))? {
                Some(state) => state,
                None => self.move_wait.give_up(waits, stance),
            },
        );
        Ok(())
    }

    /// Case 5: `FUN_141162db0` turns the builder to face its foundation (a builder that turns over
    /// time is not modelled), then `set_state(7)` places it ([`BuildAction::place`]).
    fn face_and_place(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        reads: &BuildReads,
    ) -> Result<(), GenieError> {
        let at = builder.position();
        let to =
            c.w.spatial
                .get_known(self.target)?
                .ok_or(GenieError::unsupported(
                    "a builder's foundation without a place",
                ))?
                .position;
        let now = c.w.world_time;
        let mover = builder.body.mover_mut().ok_or(BUILDER_UNMOVING)?;
        if !crate::combat::attack::face_target(mover, at, to, now)? {
            return Err(GenieError::unsupported(
                "a builder that turns over time to face its foundation",
            ));
        }
        self.place(c, builder, reads)
    }
}

/// `FUN_140c5dcd0`: the build orders (`0x2be`) of `ai`'s queue whose foundation is gone or no
/// longer in state 0 leave it; `site` is the Build's own foundation with its state now, the
/// others' states are as the update started (`build_targets`, None for one that is gone).
pub(super) fn drop_done_orders(
    ai: &mut UnitAi,
    site: Option<(i32, u8)>,
    build_targets: &[(i32, Option<u8>)],
) {
    ai.orders.retain(|o| {
        if o.order != OrderKind::Build {
            return true;
        }
        let state = match site {
            Some((id, state)) if id == o.target => Some(state),
            _ => build_targets
                .iter()
                .find(|&&(id, _)| id == o.target)
                .and_then(|&(_, s)| s),
        };
        state == Some(0)
    });
}
