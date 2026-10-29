//! The states the update and the unit's stop set: the prelude, the work's end and what
//! follows it, the restart, the idle, a retarget, the halt and the end; the builder count and
//! the load the work's end drops.

use crate::GenieError;
use crate::construction::build::{
    BUILDER_UNMOVING, BuildAction, BuildReads, DONE, FARM, FINISHED, FISH_TRAP, FISHING_SHIPS,
    HALTED, IDLE, LAST_TAKEN, MOVE, NO_TARGET_POINT, SITE_GONE, VILLAGER, WORK, drop_done_orders,
};
use crate::math::rng::Stream;
use crate::movement::footprint_gap;
use crate::movement::move_to::MoveTo;
use crate::objects::teardown::HeadCode;
use crate::objects::unit::Unit;
use crate::unit_ai::{ActionKind, OrderKind};
use crate::world::SEARCH_NEEDED;
use crate::world::step::Step;

impl BuildAction {
    /// The foundation as the update reads it ([`BuildReads::site`]): the builder's player's
    /// object, None once the player no longer holds it.
    pub(super) fn site_unit<'w>(c: &'w Step<'_>, reads: &BuildReads) -> Option<&'w Unit> {
        reads
            .site
            .and_then(|id| c.w.players[c.player].objects.by_id(id))
    }

    /// The builder count (`+0x9e8`) of the building `FUN_140aaddb0` gives a Build on `site`
    /// ([`crate::world::World::counted_building`]: an annex's main building) changes by `by`
    /// at once; nothing once the world no longer holds it.
    ///
    /// # Errors
    ///
    /// An object of a master its owner lacks.
    pub(super) fn count(c: &mut Step<'_>, site: i32, by: i8) -> Result<(), GenieError> {
        if let Some(id) = c.w.counted_building(site)?
            && let Some((_, u)) = c.w.object_mut(id)
        {
            let n = u.repairers_mut();
            *n = n.wrapping_add_signed(by);
        }
        Ok(())
    }

    /// The builders the work's share reads (`+0x9e8` of `FUN_140aaddb0`'s building for the
    /// foundation `site`), 0 once the world no longer holds it.
    ///
    /// # Errors
    ///
    /// An object of a master its owner lacks.
    pub(super) fn builders(c: &Step<'_>, site: i32) -> Result<u8, GenieError> {
        Ok(c.w
            .counted_building(site)?
            .and_then(|id| c.w.object(id))
            .map_or(0, |(_, u)| u.repairers()))
    }

    /// A counted builder leaves the builders of its foundation ([`BuildAction::count`]) and
    /// is no longer counted.
    ///
    /// # Errors
    ///
    /// [`BuildAction::count`]'s.
    pub(super) fn uncount(&mut self, c: &mut Step<'_>) -> Result<(), GenieError> {
        if self.added {
            Self::count(c, self.target, -1)?;
            self.added = false;
        }
        Ok(())
    }

    /// The prelude of every `set_state` (`0x140aae200`): a counted builder leaves its
    /// foundation's builders ([`BuildAction::uncount`]), and the sub-actions go
    /// (`FUN_140a98620`: a `MoveTo`'s teardown stops the unit, slot 0x5d0).
    ///
    /// # Errors
    ///
    /// The builder count's, and the unit's stop that the motion step does not model.
    pub(super) fn leave(&mut self, c: &mut Step<'_>, builder: &mut Unit) -> Result<(), GenieError> {
        self.uncount(c)?;
        if self.move_to.take().is_some()
            && let Some(m) = builder.body.mover_mut()
        {
            m.stop()?;
        }
        self.goal = None;
        self.walk_target = -1;
        Ok(())
    }

    /// The load a builder drops (`FUN_140aadc50`): its player's slot 0xf8 with 1 and 1 on
    /// `kind` and `amount` ([`Step::give_load`]), then it carries nothing (`FUN_14119d310` with
    /// −1 and 0).
    ///
    /// # Errors
    ///
    /// [`Step::give_load`]'s.
    pub(super) fn drop_load(
        c: &mut Step<'_>,
        builder: &mut Unit,
        kind: i16,
        amount: f32,
    ) -> Result<(), GenieError> {
        c.give_load(kind, amount)?;
        builder.life.attribute_type = -1;
        builder.life.amount = 0.0;
        Ok(())
    }

    /// `set_state(224)` (`0x140aae200` case 0xe0): the work ended. A builder carrying something
    /// (`+0x310` above 0, its kind not −1) on a drop site of its civ (`FUN_140fde180`,
    /// [`BuildReads::drop_site`]) drops it ([`BuildAction::drop_load`], the food kinds as food).
    ///
    /// # Errors
    ///
    /// The prelude's and the drop's.
    pub(super) fn finish(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        reads: &BuildReads,
    ) -> Result<(), GenieError> {
        self.leave(c, builder)?;
        self.state = FINISHED;
        let (amount, kind) = (builder.life.amount, builder.life.attribute_type);
        if !(reads.drop_site && 0.0 < amount && 0 <= kind) {
            return Ok(());
        }
        let kind = if matches!(kind, 15..=17) { 0 } else { kind };
        Self::drop_load(c, builder, kind, amount)
    }

    /// Case 0xdf of the update, the work ended (224): a villager works the farm it built (class
    /// 49 but the fish trap, `FUN_141194b20`: a pasture too), a fishing ship its fish trap (slot
    /// 0x7e8 on it, at its place: [`Step::work_order`]); another builder of a pasture (1893,
    /// 1897, slot 0x780 against its owner's attribute 290) is not modelled; the builder's unit
    /// AI drops its build orders on foundations gone or built (`FUN_140c5dcd0`); with orders
    /// left in its queue (`FUN_140c564a0`) it has no order and no action (`FUN_140c64b60`,
    /// `FUN_140c64b50` with −1) and the Build ends (`set_state(1)`, then the update returns 1).
    /// A builder without queued orders looks for more work around the building
    /// (`FUN_1415cc4e0`, [`BuildAction::search`]). A computer player's builder is not modelled.
    ///
    /// # Errors
    ///
    /// What it does not model.
    pub(super) fn after_work(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        reads: &BuildReads,
    ) -> Result<HeadCode, GenieError> {
        if c.w.players[c.player].computer {
            return Err(GenieError::unsupported(
                "a computer player's builder after its work",
            ));
        }
        let site = Self::site_unit(c, reads);
        let master = site
            .and_then(|u| u.master(&c.w.masters, c.player).ok())
            .ok_or(GenieError::unsupported(
                "the end of a Build on a foundation the slice does not read",
            ))?;
        // LAB_140ab09af: a villager works the farm it built (FUN_141194b20, FUN_141195750), a
        // fishing ship its fish trap (FUN_141194640, FUN_1411945a0): its unit AI drops its build
        // orders (FUN_140c5e020 with 0x2be), then slot 0x7e8 on the building
        let farm = master.class == FARM && master.id != FISH_TRAP;
        let works_it = (farm && reads.class == VILLAGER)
            || (master.id == FISH_TRAP && FISHING_SHIPS.contains(&builder.life.master_id));
        // a pasture is a farm to the test, so a villager works it as any farm; another builder
        // works it when its livestock's workers (slot 0x780, FUN_1411c5dd0) are its owner's
        // attribute 290
        if !works_it && matches!(master.id, 1893 | 1897) {
            return Err(GenieError::unsupported(
                "the end of a Build on a pasture (its animals, slot 0x780)",
            ));
        }
        let built = site.map(|s| (s.id, s.life.state));
        let ai = builder.ai.as_mut().ok_or(GenieError::unsupported(
            "a builder without a unit AI after its work",
        ))?;
        if works_it {
            ai.orders.retain(|o| o.order != OrderKind::Build);
            self.works_it(c)?;
            return Ok(HeadCode::Keeps);
        }
        // FUN_140c5dcd0 (the building just completed is in state 2 now)
        drop_done_orders(ai, built, &reads.build_targets);
        if ai.orders.is_empty() {
            self.search(c, builder, reads)?;
            return Ok(HeadCode::Keeps);
        }
        ai.end_order_and_action()?;
        self.end(c, builder, DONE)?;
        Ok(HeadCode::Deletes)
    }

    /// Slot 0x7e8 on the building the builder built, at its place: the work the caller gives
    /// after the update ([`Step::work_order`]).
    fn works_it(&self, c: &mut Step<'_>) -> Result<(), GenieError> {
        let point =
            c.w.spatial
                .get(self.target)
                .ok_or(GenieError::unsupported(
                    "a builder's building without a place",
                ))?
                .position;
        c.work_order = Some(crate::orders::Work {
            target: self.target,
            point,
            carry_test: false,
            filter: -1,
            notified: true,
        });
        Ok(())
    }

    /// The Build's case 0xdf after the work: the builder's search for more work around the
    /// building it finished ([`Unit::more_work`]: the work the caller gives after the update,
    /// [`Step::work_order`]); without one the Build lets the building go and waits
    /// ([`BuildAction::no_more_work`]; `set_state(3)` without orders: notify 0x202, then the
    /// standing sprite).
    ///
    /// # Errors
    ///
    /// What the search does not model.
    fn search(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        reads: &BuildReads,
    ) -> Result<(), GenieError> {
        // the caller read the foundation, so the player holds it
        let built = reads.built.ok_or(GenieError::unreachable(
            "a builder's search for more work without its building",
        ))?;
        let target = self.target;
        if let Some(work) = builder.more_work(c, target, built)? {
            c.work_order = Some(work);
            return Ok(());
        }
        self.no_more_work()?;
        crate::world::lets::release(c, builder, target, reads.by)?;
        builder.post_own(SEARCH_NEEDED, [ActionKind::Build.value(), 0, 0])?;
        Self::stand(c, builder)
    }

    /// The unit's slot 0x68 with its master's standing sprite (`+0x470`,
    /// [`Unit::show_standing`]) at once.
    ///
    /// # Errors
    ///
    /// [`Unit::show_standing`]'s.
    pub(super) fn stand(c: &mut Step<'_>, builder: &mut Unit) -> Result<(), GenieError> {
        builder.show_standing(
            c.w.masters.get(c.player),
            &c.w.graphics,
            c.w.streams.get(Stream::Sprite),
        )
    }

    /// `set_state(4)` (`0x140aae200` case 4) toward its foundation `self.target`: the prelude
    /// (it leaves the foundation's builders, its `MoveTo` goes: the unit stops), then a new
    /// `MoveTo` to the foundation (`FUN_140aa0df0`), arrived when the footprints touch; its point
    /// (`+0x38`..`+0x40`, the foundation's place) is not held. Without a foundation the point
    /// stays and the `MoveTo` has no target: it walks to its zeroed point ([`NO_TARGET_POINT`]),
    /// far from it (`FUN_14118f680` without a target: `FLT_MAX`), in state 13 for a unit without
    /// speed (`FUN_140aa2100`). A foundation whose `state` is above 6 the `MoveTo` refuses
    /// ([`BuildAction::refuse_walk_target`]); None when the update does not read its state.
    ///
    /// # Errors
    ///
    /// A builder without movement state, a foundation without a place.
    pub(super) fn restart(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        state: Option<u8>,
    ) -> Result<(), GenieError> {
        self.leave(c, builder)?;
        let mover = builder.body.mover().ok_or(BUILDER_UNMOVING)?;
        let moves = mover.stats.speed != 0.0;
        if self.target == -1 {
            let mut new = self.restarted(-1, self.point, f32::MAX);
            new.goal = Some(NO_TARGET_POINT);
            new.walk_target = -1;
            new.move_to = Some(if moves { MoveTo::Start } else { MoveTo::Failed });
            *self = new;
            return Ok(());
        }
        let at = c.w.spatial.get_known(self.target)?.ok_or(SITE_GONE)?;
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        let gap = footprint_gap(builder.position(), ground, at.position, at.radius);
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
    /// longer in state 0 leave its queue (`FUN_140c5dcd0`, the foundation's state as it is now);
    /// with orders left (`FUN_140c564a0`) it has no order and no action (`FUN_140c64b60`,
    /// `FUN_140c64b50` with −1) and the Build ends (`set_state(1)`: the update after removes
    /// it); with none it posts notify 0x202 to its unit AI and takes its standing sprite (slot
    /// 0x68).
    ///
    /// # Errors
    ///
    /// A unit AI on a patrol order, and what the prelude and the sprite do not model.
    pub(super) fn idle(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        reads: &BuildReads,
    ) -> Result<(), GenieError> {
        let built = Self::site_unit(c, reads).map(|s| (s.id, s.life.state));
        self.leave(c, builder)?;
        self.state = IDLE;
        let mut left = 0;
        if let Some(ai) = builder.ai.as_mut() {
            drop_done_orders(ai, built, &reads.build_targets);
            left = ai.orders.len();
            if left != 0 {
                ai.end_order_and_action()?;
                self.end(c, builder, DONE)?;
            }
        }
        if left == 0 {
            builder.post_own(SEARCH_NEEDED, [ActionKind::Build.value(), 0, 0])?;
            Self::stand(c, builder)?;
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

    /// `set_state(2)` from the update ([`BuildAction::halt`]): a counted builder leaves its
    /// foundation's builders ([`BuildAction::count`]), the unit takes its standing sprite, and
    /// the foundation and the second target are let go (their slot 0x180), all at once.
    ///
    /// # Errors
    ///
    /// The builder count's, the sprite's and the releases'.
    pub(super) fn halt_in(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        reads: &BuildReads,
    ) -> Result<(), GenieError> {
        let (counted, [target, second]) = self.halt(builder.body.mover_mut())?;
        if counted {
            Self::count(c, target, -1)?;
        }
        Self::stand(c, builder)?;
        if target != -1 {
            crate::world::lets::release(c, builder, target, reads.by)?;
        }
        if second != -1 {
            crate::world::lets::release(c, builder, second, reads.by)?;
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
    /// ([`BuildAction::leave`]), then the builder takes its standing sprite (slot 0x68 with its
    /// master's `+0x470`) and, with a target, its owner's slot 0x380 hears of it
    /// (`FUN_141618d30`: only a computer player's AI listens).
    ///
    /// # Errors
    ///
    /// The prelude's and the sprite's.
    pub(super) fn end(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        state: u8,
    ) -> Result<(), GenieError> {
        self.leave(c, builder)?;
        self.state = state;
        Self::stand(c, builder)
    }
}
