//! The Build action's start (state 4 toward the foundation) and its update, which runs the
//! action's state; the AI's build orders whose foundation is gone.

use crate::GenieError;
use crate::construction::build::{
    BUILDER_UNMOVING, BuildAction, BuildReads, BuildTask, BuildWalks, CLEAR, DROP, FINISHED,
    HALTED, IDLE, MOVE, PLACE, WAIT, WALK_POINT, WORK, WalkTo,
};
use crate::movement::Me;
use crate::movement::move_to::{GiveUp, MoveToAction};
use crate::objects::lifecycle::seconds;
use crate::objects::masters::class;
use crate::objects::masters::ids::PASTURE_1889;
use crate::objects::masters::ids::TOWN_CENTRE_621;
use crate::objects::teardown::HeadCode;
use crate::objects::unit::Unit;
use crate::orders::action_state::DONE;
use crate::orders::action_state::FAILED;
use crate::unit_ai::OrderKind;
use crate::unit_ai::UnitAi;
use crate::world::step::Step;

impl BuildAction {
    /// The action's start (slot 5, `0x140aadd80`) on the foundation `to`: state 4, whose
    /// `set_state` (`0x140aae200`) takes the foundation's place as the point and makes the
    /// `MoveTo` to it ([`BuildWalks::to_foundation`] with what it reads of the builder, `walks`).
    ///
    /// # Errors
    ///
    /// A sprite the table lacks.
    pub fn start(to: WalkTo, task: BuildTask, walks: &BuildWalks<'_>) -> Result<Self, GenieError> {
        Ok(Self {
            target: to.id,
            state: MOVE,
            move_to: walks.to_foundation(&task, Some(to))?,
            point: to.at,
            task,
            added: false,
            timer: 0.0,
            site: -1,
        })
    }

    /// `set_state(4)` (and slot 0x48 or 0x68 before it) on `target` at `at`, with the `MoveTo`
    /// its case 4 makes ([`BuildWalks::to_foundation`]); the second target and the timer stay.
    #[must_use]
    pub(super) const fn restarted(
        &self,
        target: i32,
        at: [f32; 3],
        move_to: Option<MoveToAction>,
    ) -> Self {
        Self {
            target,
            state: MOVE,
            move_to,
            point: at,
            task: self.task,
            added: false,
            timer: self.timer,
            site: self.site,
        }
    }

    /// One update (slot 6, `0x140ab0370`) of `builder` (`me`) through its step, with what it
    /// reads as it starts (`reads`). Before its state's case it checks its foundation: one the
    /// world lost ([`BuildAction::lost_foundation`]), a human player's pasture already built
    /// ([`BuildAction::pasture_built`]), and one no longer under construction
    /// ([`BuildAction::foundation_done`]); then the end of the work (224), the drop-off (8) and
    /// the state's case ([`BuildAction::update_state`]); the walking states update their
    /// sub-action list ([`BuildAction::step_walk`]).
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
        if reads.site.is_some() {
            if self.pasture_built(c, builder, reads)? {
                return Ok(HeadCode::Deletes);
            }
            self.foundation_done(c, builder, reads)?;
        }
        if self.state == FINISHED {
            return self.after_work(c, builder, reads);
        }
        if self.state == DROP {
            return self.update_drop(c, builder, me, reads);
        }
        self.update_state(c, builder, me, reads)
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
            .is_some_and(|m| m.id == PASTURE_1889);
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
    /// slot 0x68 with none lets it go (its `MoveTo` keeps walking to it unless it refused it).
    fn foundation_done(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        reads: &BuildReads,
    ) -> Result<(), GenieError> {
        let Some(site) = Self::site_unit(c, reads) else {
            return Ok(());
        };
        let state = site.life.state;
        if state == 0 || matches!(self.state, FINISHED | MOVE | 8) {
            return Ok(());
        }
        let master = site.master(&c.w.masters, c.player).ok();
        let special = master.is_some_and(|m| m.id == TOWN_CENTRE_621 || m.class == class::GATE);
        let unbuilt = site
            .construction
            .is_some_and(|k| k.progress < f32::from(k.build_time));
        if (special || state < 3) && !(special && 2 < state && unbuilt) {
            return self.finish(c, builder, reads);
        }
        self.restart(c, builder)?;
        let target = std::mem::replace(&mut self.target, -1);
        crate::world::lets::release(c, builder, target, reads.by)?;
        Ok(())
    }

    /// The update's case by the state, and the update's code.
    fn update_state(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        me: Me,
        reads: &BuildReads,
    ) -> Result<HeadCode, GenieError> {
        match self.state {
            // case 0: the update returns 1, the action list removes it
            DONE => return Ok(HeadCode::Deletes),
            // case 1: set_state(13); the update returns 0
            HALTED => self.end(c, builder, FAILED)?,
            MOVE | CLEAR => self.walking(c, builder, me, reads)?,
            WORK => self.face_and_place(c, builder, reads)?,
            PLACE => self.work(c, builder, reads)?,
            WALK_POINT => self.walking_to_point(c, builder, me, reads)?,
            // 13 (`0xd`): the Build failed; its next update tells its unit AI and halts it
            FAILED => return self.failed(c, builder, reads),
            WAIT => self.wait_out(c, builder, reads)?,
            // the default case (states 0, 3, 5, 8 to 10, 12 and 14 to 221): the update returns 0
            _ => {}
        }
        Ok(HeadCode::Keeps)
    }

    /// Cases 3 and 0xde, the walks to the foundation (4) and off its footprint (223), by the
    /// sub-action list's code ([`BuildAction::step_walk`]): 1 and 2 end the walk
    /// ([`BuildAction::walk_ended`]), 3 failed it ([`BuildAction::walk_failed`]), 4 and 5 (its
    /// target let go) let the foundation go and walk to the point (slot 0x68 with none, then
    /// `set_state(11)`: [`BuildAction::lose_foundation`]); 0 (it walks or waits) and any code
    /// above 5 (none) leave the update's 0.
    ///
    /// # Errors
    ///
    /// The walk's and the next states'.
    fn walking(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        me: Me,
        reads: &BuildReads,
    ) -> Result<(), GenieError> {
        match self.step_walk(c, builder, me, reads)? {
            1 | 2 => self.walk_ended(c, builder, reads),
            3 => self.walk_failed(c, builder, reads),
            4 | 5 => self.lose_foundation(c, builder, reads),
            _ => Ok(()),
        }
    }

    /// Case 10, the walk to the point (11), by the sub-action list's code: at its end (1, 2),
    /// failed (3) or its target let go (5) the Build waits (`set_state(3)`,
    /// [`BuildAction::idle`]); its target let go with 4 lets the foundation go and walks to the
    /// point again (slot 0x68 with none, then `set_state(11)`); 0 (it walks or waits) and any
    /// code above 5 (none) leave the update's 0.
    ///
    /// # Errors
    ///
    /// The walk's and the next states'.
    fn walking_to_point(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        me: Me,
        reads: &BuildReads,
    ) -> Result<(), GenieError> {
        match self.step_walk(c, builder, me, reads)? {
            1..=3 | 5 => self.idle(c, builder, reads),
            4 => self.lose_foundation(c, builder, reads),
            _ => Ok(()),
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

    /// What the `MoveTo`'s answer to the motion step giving up reads ([`GiveUp`]): a human
    /// player's villager waits (by its unit AI's stance).
    pub(super) fn gives_up(c: &Step<'_>, builder: &Unit, reads: &BuildReads) -> GiveUp {
        let computer = c.w.players[c.player].computer;
        GiveUp {
            waits: reads.class == class::VILLAGER && !computer,
            stance: builder.ai.as_ref().map_or(0, |ai| ai.stance),
        }
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
