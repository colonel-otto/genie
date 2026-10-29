//! State 8: a builder that carries something walks to a drop site first ([`DropSearch`],
//! [`DropStart`]).

use crate::GenieError;
use crate::construction::build::{
    ARRIVED, BUILDER_UNMOVING, BuildAction, BuildStep, Builder, DONE, DROP, DROP_REACH, DROP_WAIT,
    DROPPED_ORDERS, NO_TARGET_POINT, Site, VILLAGER,
};
use crate::movement::footprint_gap;
use crate::movement::move_to::{
    Goal, MoveTo, MoveWait, Walk, start_walk_or_give_up, walk_or_give_up,
};

/// What the caller's search gives `set_state(8)` ([`BuildAction::set_drop`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DropSearch {
    /// The builder carries nothing (`+0x310` at most 0): no search.
    Empty,
    /// The drop-site table has no drop site for its load (`FUN_141658e20`).
    Nothing,
    /// The drop site the table gives, and the range of the `MoveTo` to it: the task the
    /// builder's slot 0x820 finds on it (any action type, no switch: its `+0x30`), else the
    /// Build task's.
    Site {
        /// The drop site.
        id: i32,
        /// The `MoveTo`'s range.
        range: f32,
    },
}

/// What `set_state(8)` leaves to its caller ([`BuildAction::set_drop`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropStart {
    /// The second target it let go (`FUN_140a7a190`: its slot 0x180), −1 for none.
    pub let_go: i32,
    /// The second target it took (`FUN_140a7a190`: its slot 0x178), −1 for none.
    pub took: i32,
    /// It found no drop site: the unit's slot 0x328 posts notify 0x203 (the Build's action
    /// value, 0, 0) to its unit AI.
    pub notified: bool,
}

impl BuildAction {
    /// `set_state(8)` (`0x140aae200` case 8) with what the caller's search found (`search`): the
    /// prelude ([`BuildAction::leave`]: out of the builders of `builders`, the `MoveTo`'s
    /// teardown stops the unit), then a builder that carries nothing walks to its foundation
    /// (`set_state(4)`, [`BuildAction::restart`]). Else the drop-site table's nearest drop site
    /// for its load (`FUN_141658e20`) becomes the second target (`FUN_140a7a190`: the old one let
    /// go, the new one taken, the caller's); with none the unit tells its unit AI (notify 0x203)
    /// and walks to its foundation (`set_state(4)`); with one a quiet `MoveTo` (`+0x61` 1) to it
    /// within the search's range starts (`FUN_140aa0df0` with its point the site's place; its
    /// sprite, the task's carrying one or the Build task's moving one, is not held): arrived
    /// within 0.01 of its footprint (`FUN_140aa2100`), in state 13 for a unit without speed.
    ///
    /// # Errors
    ///
    /// A builder without movement state, a drop site or foundation without a place.
    pub fn set_drop(
        &mut self,
        walk: &mut Walk<'_>,
        builders: Option<&mut u8>,
        search: DropSearch,
    ) -> Result<DropStart, GenieError> {
        self.leave(walk, builders)?;
        self.state = DROP;
        let mut out = DropStart {
            let_go: -1,
            took: -1,
            notified: false,
        };
        let found = match search {
            DropSearch::Empty => {
                self.restart(walk, None, None)?;
                return Ok(out);
            }
            DropSearch::Nothing => None,
            DropSearch::Site { id, range } => Some((id, range)),
        };
        let new = found.map_or(-1, |(id, _)| id);
        if self.site != new {
            out.let_go = self.site;
            out.took = new;
            self.site = new;
        }
        let Some((id, range)) = found else {
            out.notified = true;
            self.restart(walk, None, None)?;
            return Ok(out);
        };
        let mover = walk.body.mover().ok_or(BUILDER_UNMOVING)?;
        let at = walk.position();
        let to = walk
            .world
            .get_known(id)?
            .ok_or(GenieError::unsupported("a drop site without a place"))?;
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        let gap = footprint_gap(at, ground, to.position, to.radius);
        let moves = mover.stats.speed != 0.0;
        self.drop_range = range;
        self.walk_target = id;
        self.goal = Some(to.position);
        self.move_wait = MoveWait::default();
        self.move_to = Some(if gap <= ARRIVED {
            MoveTo::Arrived
        } else if moves {
            MoveTo::Start
        } else {
            MoveTo::Failed
        });
        Ok(out)
    }

    /// State 8's update (`0x140ab0370` case 7). While its timer is below 0 (COMISS: a NaN runs
    /// it out) it runs by dt; at its end the drop-off searches again (`set_state(8)`,
    /// [`BuildStep::drop_off`]). Else its quiet `MoveTo` walks to the drop site
    /// ([`BuildAction::walk_to_drop`]): its target in a state above 6 (code 5), a failed walk
    /// (codes 3 and 4) and a drop site gone or not in state 2 search again. At its end (codes 1
    /// and 2: the list deletes the `MoveTo`, whose destructor stops the unit) a drop site that
    /// does not take the load (`FUN_140fdd190`) halts the Build (`set_state(2)`); one beyond
    /// [`DROP_REACH`] (`FUN_14118f8c0`) makes it wait 3 seconds with the unit's animation stopped
    /// (`FUN_141161850`). Else the load goes to the player (slot 0xf8 with 1 and 1, the resource
    /// of what it carries, `FUN_140fdd4f0`: the caller's, [`BuildStep::dropped`]; its sound, slot
    /// 0x4c0, and the statistics are not held), the unit carries nothing (`FUN_14119d310`) and
    /// the Build goes on ([`BuildAction::after_deposit`]).
    pub(super) fn update_drop(
        &mut self,
        mut walk: Walk<'_>,
        site: Option<Site<'_>>,
        builder: Builder<'_>,
        step: &mut BuildStep,
    ) -> Result<(), GenieError> {
        if crate::math::fmath::below_or_unordered(self.timer, 0.0) {
            self.timer += walk.dt;
            if self.timer < 0.0 {
                return Ok(());
            }
            self.timer = 0.0;
            step.drop_off = true;
            return Ok(());
        }
        let computer = walk.plan.as_ref().is_some_and(|p| p.computer);
        let waits = builder.class == VILLAGER && !computer;
        let stance = builder.ai.as_ref().map_or(0, |ai| ai.stance);
        // the quiet MoveTo lets a target in a state above 6 go (slot 0x68 with none): code 5
        if self.move_to.is_some()
            && self.walk_target != -1
            && builder.walk_target_state.is_some_and(|s| 6 < s)
        {
            self.walk_target = -1;
            step.drop_off = true;
            return Ok(());
        }
        if self.walk_to_drop(&mut walk, (waits, stance), step)? {
            return Ok(());
        }
        if self.move_to.take().is_some()
            && let Some(m) = walk.body.mover_mut()
        {
            m.stop()?;
        }
        let Some((2, drop)) = builder.deposit.filter(|_| self.site != -1) else {
            step.drop_off = true;
            return Ok(());
        };
        if !drop.takes {
            return self.halt_in(&mut walk, site, step);
        }
        if DROP_REACH < drop.gap_squared {
            self.timer = DROP_WAIT;
            step.anim_stopped = true;
            return Ok(());
        }
        let kind = crate::gathering::hunt::resource_kind(builder.held_kind, builder.attributes);
        step.dropped = Some((kind, builder.held));
        self.after_deposit(walk, site, builder, computer, step)
    }

    /// State 8's quiet `MoveTo` to the drop site (its target, else its point): its start asks
    /// for a path within the drop-off range; on its way a drop site that left the `MoveTo`'s
    /// point makes it ask again, else it walks; the motion step giving up makes it wait or fail
    /// ([`MoveWait::give_up`] with `waits` and `stance`); a waiting one asks again at its delay;
    /// a failed one searches again. True while it walks, waits or failed (the update ends there);
    /// false at its end (codes 1 and 2) or without it.
    fn walk_to_drop(
        &mut self,
        walk: &mut Walk<'_>,
        (waits, stance): (bool, u8),
        step: &mut BuildStep,
    ) -> Result<bool, GenieError> {
        let to = if self.walk_target == -1 {
            Goal::Point(self.goal.unwrap_or(NO_TARGET_POINT))
        } else {
            Goal::Target(self.walk_target)
        };
        match self.move_to {
            Some(MoveTo::Start) => {
                self.asked_again();
                self.move_to = Some(MoveTo::Moving);
                let range = self.drop_range;
                self.move_to = Some(
                    match start_walk_or_give_up(walk.reborrow(), to, range, true)? {
                        Some(state) => state,
                        None => self.move_wait.give_up(waits, stance),
                    },
                );
            }
            Some(MoveTo::Moving) => {
                if self.walk_target != -1
                    && crate::movement::move_to::target_moved(walk, self.walk_target, true)?.again()
                {
                    self.move_to = Some(MoveTo::Start);
                    return Ok(true);
                }
                self.move_to = Some(match walk_or_give_up(walk.reborrow())? {
                    Some(state) => state,
                    None => self.move_wait.give_up(waits, stance),
                });
            }
            Some(MoveTo::Waiting) => {
                self.move_wait.timer += walk.dt;
                if self.move_wait.delay <= self.move_wait.timer {
                    self.move_wait.timer = 0.0;
                    self.move_to = Some(MoveTo::Start);
                }
            }
            Some(MoveTo::Failed) => step.drop_off = true,
            Some(MoveTo::Arrived) | None => return Ok(false),
        }
        Ok(true)
    }

    /// State 8 after the load went to the player: a Build with a foundation lets the drop site
    /// go and walks back to it (`set_state(4)`). Without one its unit AI drops its orders on
    /// targets gone (`FUN_140c5de60`); with orders left it has no order and no action and the
    /// Build is done (`set_state(1)`); with none it waits (`set_state(3)`). A unit without a unit
    /// AI and a point of −1 is done (a computer player's) or halts; with a point it walks there
    /// (`set_state(11)`).
    fn after_deposit(
        &mut self,
        mut walk: Walk<'_>,
        site: Option<Site<'_>>,
        mut builder: Builder<'_>,
        computer: bool,
        step: &mut BuildStep,
    ) -> Result<(), GenieError> {
        if self.target != -1 {
            step.site_let_go = Some(std::mem::replace(&mut self.site, -1));
            let state = site.as_ref().map(|s| s.life.state);
            return self.restart(&mut walk, site.map(Site::into_builders), state);
        }
        if let Some(ai) = builder.ai.as_deref_mut() {
            let gone = builder.gone_orders;
            ai.orders
                .retain(|o| !(DROPPED_ORDERS.contains(&o.order) && gone.contains(&o.target)));
            if !ai.orders.is_empty() {
                ai.end_order_and_action()?;
                self.end(&mut walk, None, DONE, step)?;
                step.done = true;
                return Ok(());
            }
            return self.idle(&mut walk, site, builder, step);
        }
        // UCOMISS against −1 (DAT_14330ec20): a NaN is not
        if self.point[0] == -1.0 || self.point[1] == -1.0 {
            if computer {
                return self.end(&mut walk, None, DONE, step);
            }
            return self.halt_in(&mut walk, site, step);
        }
        self.walk_to_point(&mut walk, None)
    }
}
