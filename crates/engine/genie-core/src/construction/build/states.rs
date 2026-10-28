//! The states the update and the unit's stop set: the prelude, the work's end and what
//! follows it, the restart, the idle, a retarget, the halt and the end.

use crate::GenieError;
use crate::construction::build::{
    AfterWork, BUILDER_UNMOVING, BuildAction, BuildStep, Builder, DONE, Dropped, FARM, FINISHED,
    FISH_TRAP, FISHING_SHIPS, HALTED, IDLE, LAST_TAKEN, Load, MOVE, NO_TARGET_POINT, SITE_GONE,
    Site, VILLAGER, WORK, drop_done_orders,
};
use crate::movement::footprint_gap;
use crate::movement::move_to::{MoveTo, Walk};
use crate::unit_ai::OrderKind;

impl BuildAction {
    /// The prelude of every `set_state` (`0x140aae200`): a builder counted among the builders
    /// of `FUN_140aaddb0`'s building (`builders`, [`Site::builders`]: none once its foundation
    /// is gone) leaves them, and the sub-actions go (`FUN_140a98620`: a `MoveTo`'s teardown
    /// stops the unit, slot 0x5d0).
    pub(super) fn leave(
        &mut self,
        walk: &mut Walk<'_>,
        builders: Option<&mut u8>,
    ) -> Result<(), GenieError> {
        if self.added {
            if let Some(b) = builders {
                *b = b.wrapping_sub(1);
            }
            self.added = false;
        }
        if self.move_to.take().is_some()
            && let Some(m) = walk.mover.as_deref_mut()
        {
            m.stop()?;
        }
        self.goal = None;
        self.walk_target = -1;
        Ok(())
    }

    /// `set_state(224)` (`0x140aae200` case 0xe0): the work ended. A builder carrying something
    /// (`+0x310` above 0, its kind not −1) on a drop site of its civ (`FUN_140fde180`) drops it
    /// (`FUN_140aadc50`): returns the load its player gets ([`BuildStep::dropped`]).
    pub(super) fn finish(
        &mut self,
        walk: &mut Walk<'_>,
        builders: Option<&mut u8>,
        load: Load,
    ) -> Result<Dropped, GenieError> {
        if self.added && builders.is_none() {
            return Err(GenieError::unsupported(
                "the end of a Build counted among another player's builders",
            ));
        }
        self.leave(walk, builders)?;
        self.state = FINISHED;
        if !(load.drop_site && 0.0 < load.amount && 0 <= load.kind) {
            return Ok(None);
        }
        // FUN_140aadc50: the food kinds become food
        let kind = if matches!(load.kind, 15..=17) {
            0
        } else {
            load.kind
        };
        Ok(Some((kind, load.amount)))
    }

    /// Case 0xdf of the update, the work ended (224): a villager works the farm it built (class
    /// 49 but the fish trap, `FUN_141194b20`: a pasture too), a fishing ship its fish trap
    /// ([`AfterWork::WorksIt`]); another builder of a pasture (1893, 1897, slot 0x780 against
    /// its owner's attribute 290) is not modelled; the builder's unit AI drops its build orders on foundations gone or
    /// built (`FUN_140c5dcd0`); with orders left in its queue (`FUN_140c564a0`) it has no order
    /// and no action (`FUN_140c64b60`, `FUN_140c64b50` with −1) and the Build ends (`set_state(1)`,
    /// then the update returns 1: Some(true)). A builder without queued orders looks for more
    /// work around the building (`FUN_1415cc4e0`, Some(false): [`BuildStep::search`], the
    /// caller's). A computer player's builder is not modelled.
    pub(super) fn after_work(
        &mut self,
        walk: &mut Walk<'_>,
        site: Option<&Site<'_>>,
        computer: bool,
        builder: Builder<'_>,
        step: &mut BuildStep,
    ) -> Result<AfterWork, GenieError> {
        if computer {
            return Err(GenieError::unsupported(
                "a computer player's builder after its work",
            ));
        }
        let master = site
            .and_then(|s| s.completion.master)
            .ok_or(GenieError::unsupported(
                "the end of a Build on a foundation the slice does not read",
            ))?;
        // LAB_140ab09af: a villager works the farm it built (FUN_141194b20, FUN_141195750), a
        // fishing ship its fish trap (FUN_141194640, FUN_1411945a0): its unit AI drops its build
        // orders (FUN_140c5e020 with 0x2be), then slot 0x7e8 on the building (the caller's)
        let farm = master.class == FARM && master.id != FISH_TRAP;
        let works_it = (farm && builder.class == VILLAGER)
            || (master.id == FISH_TRAP && FISHING_SHIPS.contains(&builder.master));
        // a pasture is a farm to the test, so a villager works it as any farm; another builder
        // works it when its livestock's workers (slot 0x780, FUN_1411c5dd0) are its owner's
        // attribute 290
        if !works_it && matches!(master.id, 1893 | 1897) {
            return Err(GenieError::unsupported(
                "the end of a Build on a pasture (its animals, slot 0x780)",
            ));
        }
        let ai = builder.ai.ok_or(GenieError::unsupported(
            "a builder without a unit AI after its work",
        ))?;
        if works_it {
            ai.orders.retain(|o| o.order != OrderKind::Build);
            return Ok(AfterWork::WorksIt);
        }
        // FUN_140c5dcd0 (the building just completed is in state 2 now)
        drop_done_orders(
            ai,
            site.map(|s| (s.id, s.life.state)),
            builder.build_targets,
        );
        if ai.orders.is_empty() {
            return Ok(AfterWork::Search);
        }
        ai.end_order_and_action()?;
        self.end(walk, None, DONE, step)?;
        Ok(AfterWork::Done)
    }

    /// `set_state(4)` (`0x140aae200` case 4) toward its foundation `self.target`: the prelude
    /// (it leaves the foundation's builders, its `MoveTo` goes: the unit stops), then a new
    /// `MoveTo` to the foundation (`FUN_140aa0df0`), arrived when the footprints touch; its point
    /// (`+0x38`..`+0x40`, the foundation's place) is not held. Without a foundation the point
    /// stays and the `MoveTo` has no target: it walks to its zeroed point ([`NO_TARGET_POINT`]),
    /// far from it (`FUN_14118f680` without a target: `FLT_MAX`), in state 13 for a unit without
    /// speed (`FUN_140aa2100`). A foundation whose `state` is above 6 the `MoveTo` refuses
    /// ([`BuildAction::refuse_walk_target`]); None when the caller does not hold its state.
    ///
    /// # Errors
    ///
    /// A builder without movement state, a foundation without a place.
    pub(super) fn restart(
        &mut self,
        walk: &mut Walk<'_>,
        builders: Option<&mut u8>,
        state: Option<u8>,
    ) -> Result<(), GenieError> {
        self.leave(walk, builders)?;
        let mover = walk.mover.as_deref().ok_or(BUILDER_UNMOVING)?;
        let moves = mover.stats.speed != 0.0;
        if self.target == -1 {
            let mut new = self.restarted(-1, self.point, f32::MAX);
            new.goal = Some(NO_TARGET_POINT);
            new.walk_target = -1;
            new.move_to = Some(if moves { MoveTo::Start } else { MoveTo::Failed });
            *self = new;
            return Ok(());
        }
        let at = walk.world.get_known(self.target)?.ok_or(SITE_GONE)?;
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        let gap = footprint_gap(walk.me.position, ground, at.position, at.radius);
        *self = self.restarted(self.target, at.position, gap);
        if state.is_some_and(|s| LAST_TAKEN < s) {
            self.refuse_walk_target(moves);
        }
        Ok(())
    }

    /// `FUN_140a7a230` on the new `MoveTo`'s target: a foundation in a state above
    /// [`LAST_TAKEN`] is refused (no pointer, the id −1). The `MoveTo` keeps the point its
    /// constructor took from the foundation (`FUN_140aa0df0`) and, without a target, starts far
    /// from it (`FUN_140aa2100`: state 3, 13 for a unit without speed).
    pub(super) const fn refuse_walk_target(&mut self, moves: bool) {
        self.walk_target = -1;
        self.goal = Some(self.point);
        self.move_to = Some(if moves { MoveTo::Start } else { MoveTo::Failed });
    }
}

impl BuildAction {
    /// `set_state(3)` (`0x140aae200` case 3): the prelude (out of the foundation's builders, the
    /// `MoveTo`'s teardown), then with a unit AI its build orders on foundations gone or no
    /// longer in state 0 leave its queue (`FUN_140c5dcd0`); with orders left (`FUN_140c564a0`)
    /// it has no order and no action (`FUN_140c64b60`, `FUN_140c64b50` with −1) and the Build
    /// ends (`set_state(1)`: the update after removes it); with none it posts notify 0x202 to
    /// its unit AI ([`BuildStep::notified`]) and takes its standing sprite (slot 0x68,
    /// [`BuildStep::stands`]).
    ///
    /// # Errors
    ///
    /// A unit AI on a patrol order.
    pub(super) fn idle(
        &mut self,
        walk: &mut Walk<'_>,
        site: Option<Site<'_>>,
        builder: Builder<'_>,
        step: &mut BuildStep,
    ) -> Result<(), GenieError> {
        let built = site.as_ref().map(|s| (s.id, s.life.state));
        self.leave(walk, site.map(Site::into_builders))?;
        self.state = IDLE;
        let mut left = 0;
        if let Some(ai) = builder.ai {
            drop_done_orders(ai, built, builder.build_targets);
            if !ai.orders.is_empty() {
                ai.end_order_and_action()?;
                self.end(walk, None, DONE, step)?;
            }
            left = ai.orders.len();
        }
        if left == 0 {
            step.notified = true;
            step.stands = true;
        }
        Ok(())
    }

    /// Slot 0x48 (`FUN_140ab1b30`), the work command's target for the Build's own task, here its
    /// own foundation again (slot 0x68 with the same target does nothing): working (state 6) it
    /// stays, as does a computer player's builder walking to it (state 4); else `set_state(4)`:
    /// it leaves the builders of `FUN_140aaddb0`'s building (`builders`, an annex's main
    /// building's) and its `MoveTo` (the unit stops, slot 0x5d0), and a new `MoveTo` to the
    /// foundation starts, arrived when the footprints touch (`gap`).
    ///
    /// # Errors
    ///
    /// A builder counted among the builders of a foundation the caller does not hold.
    pub fn retake(
        &mut self,
        (at, gap): ([f32; 3], f32),
        mover: Option<&mut crate::movement::Mover>,
        builders: Option<&mut u8>,
        computer: bool,
    ) -> Result<(), GenieError> {
        if self.state == WORK || (self.state == MOVE && computer) {
            return Ok(());
        }
        if self.added {
            let b = builders.ok_or(GenieError::unsupported(
                "a builder retaking a foundation the slice does not hold",
            ))?;
            *b = b.wrapping_sub(1);
        }
        if self.move_to.is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        *self = self.restarted(self.target, at, gap);
        Ok(())
    }

    /// Slot 0x48 (`FUN_140ab1b30`) on another foundation of the builder's player: slot 0x68
    /// (`FUN_140ab0300`) takes it (the caller's builder count, release and take; the Build is no
    /// longer among the builders) and `set_state(4)` starts over: its `MoveTo` goes (the unit
    /// stops, slot 0x5d0) and a new one to the foundation starts, arrived when the footprints
    /// touch (`gap`).
    ///
    /// # Errors
    ///
    /// The unit's stop that the motion step does not model.
    pub fn retarget(
        &mut self,
        (target, at): (i32, [f32; 3]),
        gap: f32,
        mover: Option<&mut crate::movement::Mover>,
    ) -> Result<(), GenieError> {
        if self.move_to.is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        *self = self.restarted(target, at, gap);
        Ok(())
    }

    /// Slot 0x38 (`FUN_140ab0350`) as the unit's stop calls it: `set_state(2)` (`0x140aae200`):
    /// a counted builder leaves its foundation's builders first (the caller's,
    /// [`crate::world::World::leave_builders`]: true when it counted), the sub-actions go (a
    /// `MoveTo`'s teardown stops the unit), the unit takes its standing sprite (slot 0x68 with the
    /// master's `+0x470`, the caller's), the foundation and the second target go (slot 0x68 and
    /// `FUN_140a7a190` with none: the caller releases both) and the point is −1. The answer is
    /// true, so the stop keeps the list; returns whether it counted, the foundation and the
    /// second target (−1 for none).
    ///
    /// # Errors
    ///
    /// The unit's stop that the motion step does not model.
    pub(crate) fn halt(
        &mut self,
        mover: Option<&mut crate::movement::Mover>,
    ) -> Result<(bool, [i32; 2]), GenieError> {
        let counted = std::mem::replace(&mut self.added, false);
        if self.move_to.take().is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        self.goal = None;
        self.walk_target = -1;
        self.point = [-1.0; 3];
        self.state = HALTED;
        let target = std::mem::replace(&mut self.target, -1);
        let site = std::mem::replace(&mut self.site, -1);
        Ok((counted, [target, site]))
    }

    /// `set_state(2)` from the update ([`BuildAction::halt`]): a counted builder leaves the
    /// builders of `site` (its foundation's, [`Site::builders`]), the unit takes its standing
    /// sprite ([`BuildStep::stands`]), and the foundation and the second target go
    /// ([`BuildStep::released`], [`BuildStep::site_let_go`]: the caller's).
    pub(super) fn halt_in(
        &mut self,
        walk: &mut Walk<'_>,
        site: Option<Site<'_>>,
        step: &mut BuildStep,
    ) -> Result<(), GenieError> {
        let (counted, [target, second]) = self.halt(walk.mover.as_deref_mut())?;
        if counted && let Some(b) = site.map(Site::into_builders) {
            *b = b.wrapping_sub(1);
        }
        step.stands = true;
        step.released = target != -1;
        if second != -1 {
            step.site_let_go = Some(second);
        }
        Ok(())
    }

    /// The search for more work found nothing (`0x140ab0f00`): slot 0x68 with none lets the
    /// foundation go (the caller's) and `set_state(3)` waits (the caller's notify 0x202 and
    /// standing sprite: its unit AI has no orders, or the search would not have run).
    ///
    /// # Errors
    ///
    /// A Build with its `MoveTo` (`set_state`'s teardown is not modelled for it).
    pub const fn no_more_work(&mut self) -> Result<(), GenieError> {
        if self.move_to.is_some() {
            return Err(GenieError::unsupported("a Build waiting with its MoveTo"));
        }
        self.target = -1;
        self.state = IDLE;
        Ok(())
    }

    /// `set_state(1)` and `set_state(13)` (`0x140aae200` cases 1 and 0xd, one case): the prelude
    /// ([`BuildAction::leave`]; out of the builders of `builders`), then the builder takes its
    /// standing sprite (slot 0x68 with its master's `+0x470`, [`BuildStep::stands`], the
    /// caller's) and, with a target, its owner's slot 0x380 hears of it (`FUN_141618d30`: only a
    /// computer player's AI listens).
    pub(super) fn end(
        &mut self,
        walk: &mut Walk<'_>,
        builders: Option<&mut u8>,
        state: u8,
        step: &mut BuildStep,
    ) -> Result<(), GenieError> {
        self.leave(walk, builders)?;
        self.state = state;
        step.stands = true;
        Ok(())
    }
}
