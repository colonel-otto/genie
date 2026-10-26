//! The Build action's start (state 4 toward the foundation) and its update, which runs the
//! action's state; the AI's build orders whose foundation is gone.

use crate::GenieError;
use crate::construction::build::{
    ARRIVED, AfterWork, BUILD_ORDER, BUILDER_UNMOVING, BuildAction, BuildStep, BuildTask, Builder,
    CLASS_39, CLEAR, DONE, DROP, FAILED, FINISHED, HALTED, IDLE, Load, MASTER_621, MASTER_1889,
    MOVE, PLACE, Site, VILLAGER, WAIT, WALK_POINT, WORK,
};
use crate::movement::move_to::{
    Goal, MoveTo, MoveWait, Walk, start_walk_or_give_up, walk_or_give_up,
};
use crate::unit_ai::UnitAi;

impl BuildAction {
    /// The action's start (slot 5, `0x140aadd80`) on the foundation `target` at `at`: state 4
    /// for a target, whose `set_state` (`0x140aae200`) takes the foundation's place as the point
    /// and creates the `MoveTo` to it; the `MoveTo` starts arrived if the footprints touch
    /// (`gap`).
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
            move_wait: MoveWait {
                state: 0,
                timer: 0.0,
                delay: 0.0,
            },
            site: -1,
            drop_range: 0.0,
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

    /// One update (slot 6, `0x140ab0370`), which first updates the `MoveTo` (`0x140aa2a70`); a
    /// moving `MoveTo` calls the motion step (object slot 193). `site` is the foundation, when it
    /// is the builder's player's. Before its state's case it checks its foundation: one that is
    /// no longer under construction (a state above 0) ends the work (`set_state(224)`) unless the
    /// Build already ended (224, 4, 8); one that is gone, a dead one, master 621, a builder of
    /// class 39 and master 1889 of a human player are not modelled.
    ///
    /// # Panics
    ///
    /// Never: a builder clearing the footprint (state 223) always has the point its `set_state`
    /// gave it.
    pub fn update(
        &mut self,
        mut walk: Walk<'_>,
        site: Option<Site<'_>>,
        mut builder: Builder<'_>,
    ) -> Result<BuildStep, GenieError> {
        let mut step = BuildStep::default();
        // state 3 without a target: the update does nothing (0x140ab0370's default case)
        if self.state == IDLE && self.target == -1 {
            return Ok(step);
        }
        let computer = walk.plan.as_ref().is_some_and(|p| p.computer);
        let load = Load {
            amount: builder.held,
            kind: builder.held_kind,
            drop_site: builder.drop_site,
        };
        let mut site = site;
        // the target's id names no object of the world (FUN_1415417a0): slot 0x68 with none (its
        // builders' count went with it); then a Build in state 2 or 8 goes on, any other waits
        // (set_state 3) and the update returns 0
        if self.target != -1 && site.is_none() && walk.world.get(self.target).is_none() {
            self.added = false;
            self.target = -1;
            if !matches!(self.state, 2 | 8) {
                self.idle(&mut walk, None, builder, &mut step)?;
                return Ok(step);
            }
        }
        // the second target's id names no object of the world: FUN_140a7a190 with none, whose
        // release finds nothing to call
        if self.site != -1 && walk.world.get(self.site).is_none() {
            self.site = -1;
        }
        let mut walked_from = None;
        if let Some(site) = site.as_mut() {
            let master = site.completion.master;
            let state = site.life.state;
            if !computer
                && master.is_some_and(|m| m.id == MASTER_1889)
                && state != 0
                && let Some(ai) = builder.ai.as_deref_mut()
            {
                // a human player's builder of a pasture already built: its unit AI has no order
                // and no action (FUN_140c64b60, FUN_140c64b50 with −1), the unit tells it
                // (notify 0x21d, the build action's value and the pasture the stack unit made,
                // FUN_1411c5f70), the foundation goes (slot 0x68 with none), the load goes to the
                // player (FUN_140aadc50: a kind the player has, the foods as food) and the Build
                // ends (set_state(1)); the update returns 1
                ai.end_order_and_action()?;
                step.pasture_made = Some(builder.stack_made.unwrap_or(0));
                if self.target != -1 {
                    self.target = -1;
                    step.released = true;
                }
                if (0..builder.attributes as i16).contains(&builder.held_kind) {
                    let kind = if matches!(builder.held_kind, 15..=17) {
                        0
                    } else {
                        builder.held_kind
                    };
                    step.dropped = Some((kind, builder.held));
                }
                self.end(&mut walk, Some(site.builders()), DONE, &mut step)?;
                step.done = true;
                return Ok(step);
            }
            if state != 0 && !matches!(self.state, FINISHED | MOVE | 8) {
                // 621 and the gates (class 39) end the work once built, else walk back
                let special = master.is_some_and(|m| m.id == MASTER_621 || m.class == CLASS_39);
                let c = &*site.construction;
                let unbuilt = c.progress < f32::from(c.build_time);
                if (special || state < 3) && !(special && 2 < state && unbuilt) {
                    step.dropped = self.finish(&mut walk, Some(site.builders()), load)?;
                } else {
                    // set_state(4) toward it, then slot 0x68 with none: the caller releases it;
                    // the MoveTo walks to it unless it refused it
                    self.restart(&mut walk, Some(site.builders()), Some(state))?;
                    walked_from = (self.walk_target != -1).then_some(self.target);
                    self.target = -1;
                    step.released = true;
                }
            }
        }
        if self.state == FINISHED {
            match self.after_work(&mut walk, site.as_ref(), computer, builder, &mut step)? {
                AfterWork::Done => step.done = true,
                AfterWork::Search => step.search = true,
                AfterWork::WorksIt => step.works_it = true,
            }
            return Ok(step);
        }
        let mut walk = walk;
        // the MoveTo's give-up waits for a human player's villager (by its unit AI's stance)
        let waits = builder.class == VILLAGER && !computer;
        let stance = builder.ai.as_ref().map_or(0, |ai| ai.stance);
        // the MoveTo's update first (`0x140aa2a70` through `FUN_140a992b0`, states 4 and 223):
        // its target in a state above 6 lets it go (slot 0x68 with none) and the quiet MoveTo
        // returns 5. A target the world removed is no longer the MoveTo's: its removal let the
        // MoveTo go (slot 0x180's release clears the pointer and the id), which walks on.
        if self.walk_target != -1 && builder.walk_target_state.is_none() {
            self.walk_target = -1;
        }
        if self.state == DROP {
            return self
                .update_drop(walk, site, builder, &mut step)
                .map(|()| step);
        }
        if matches!(self.state, MOVE | CLEAR)
            && self.move_to.is_some()
            && self.walk_target != -1
            && builder.walk_target_state.is_some_and(|s| 6 < s)
        {
            self.walk_target = -1;
            self.lose_foundation(&mut walk, site, &mut step)?;
            return Ok(step);
        }
        let r = match (self.state, self.move_to) {
            // the MoveTo at its end (1), or no sub-action (the list's update returns 1)
            (4, Some(MoveTo::Arrived) | None) => {
                return self
                    .walk_ended(&mut walk, site, builder, &mut step)
                    .map(|()| step);
            }
            // the MoveTo set_state(4) made walks to the foundation it let go; the motion step
            // giving up makes it wait or fail ([`MoveWait::give_up`])
            (4, Some(MoveTo::Start)) => {
                self.asked_again();
                self.move_to = Some(MoveTo::Moving);
                let range = self.task.range;
                // a MoveTo without a target (set_state(4) without a foundation, or one that
                // refused it) to its point
                let to = match (walked_from, self.goal) {
                    (None, Some(p)) if self.target == -1 || self.walk_target == -1 => {
                        Goal::Point(p)
                    }
                    _ => Goal::Target(walked_from.unwrap_or(self.target)),
                };
                self.move_to = Some(match start_walk_or_give_up(walk, to, range, false)? {
                    Some(state) => state,
                    None => self.move_wait.give_up(waits, stance),
                });
                Ok(())
            }
            (4 | CLEAR | WALK_POINT, Some(MoveTo::Moving)) => {
                self.move_to = Some(match walk_or_give_up(walk)? {
                    Some(state) => state,
                    None => self.move_wait.give_up(waits, stance),
                });
                Ok(())
            }
            // the waiting MoveTo (its state 2): the world's dt onto its timer; at its end it asks
            // again (set_state(3); COMISS, JC: a NaN timer waits on); its update returns 0
            (4 | CLEAR | WALK_POINT, Some(MoveTo::Waiting)) => {
                self.move_wait.timer += walk.dt;
                if self.move_wait.delay <= self.move_wait.timer {
                    self.move_wait.timer = 0.0;
                    self.move_to = Some(MoveTo::Start);
                }
                Ok(())
            }
            // its update returned 3
            (4 | CLEAR, Some(MoveTo::Failed)) => {
                self.walk_failed(&mut walk, site, builder, &mut step)
            }
            // case 0: the update returns 1, the action list removes it
            (DONE, _) => {
                step.done = true;
                Ok(())
            }
            // case 1: set_state(13); the update returns 0
            (HALTED, _) => self.end(&mut walk, site.map(Site::into_builders), FAILED, &mut step),
            // case 0xc: the unit's slot 0x328 posts notify 0x1f9 (the Build's action value, 0, 0)
            // to its unit AI; set_state(2) ([`BuildAction::halt`]: out of the builders, the
            // MoveTo's teardown, the unit's standing sprite, the target let go) and the update
            // returns 3: the action list removes the Build (FUN_140a98e20), as for 1
            (FAILED, _) => {
                step.failed = true;
                self.halt_in(&mut walk, site, &mut step)?;
                step.done = true;
                Ok(())
            }
            (WORK, _) => {
                let (waits, look) = (builder.waits, builder.look);
                // case 5: FUN_141162db0 turns the builder to face its foundation
                let at = walk
                    .world
                    .get(walk.me.id)
                    .map_or(walk.me.position, |p| p.position);
                let to = walk
                    .world
                    .get_known(self.target)?
                    .ok_or(GenieError::unsupported(
                        "a builder's foundation without a place",
                    ))?
                    .position;
                let mover = walk.mover.as_deref_mut().ok_or(BUILDER_UNMOVING)?;
                let now = walk.plan.as_ref().map_or(0, |p| p.now);
                if !crate::combat::attack::face_target(mover, at, to, now)? {
                    return Err(GenieError::unsupported(
                        "a builder that turns over time to face its foundation",
                    ));
                }
                self.place(&mut walk, site, (waits, look), &mut step)
            }
            // case 0xdd: the wait runs down by the world's dt, then set_state(7) again
            (WAIT, _) => {
                self.timer -= walk.dt;
                if 0.0 < self.timer {
                    return Ok(step);
                }
                self.timer = 0.0;
                self.place(&mut walk, site, (builder.waits, builder.look), &mut step)
            }
            (PLACE, _) => self.work(walk, site, load).map(|(c, d, m)| {
                step.completed = c;
                step.dropped = d;
                step.main_work = m;
            }),
            // the quiet MoveTo to a point of set_state(223) and set_state(11)
            (CLEAR | WALK_POINT, Some(MoveTo::Start)) => {
                self.asked_again();
                self.move_to = Some(MoveTo::Moving);
                let goal = self
                    .goal
                    .expect("set_state(223) and set_state(11) set the goal");
                self.move_to = Some(
                    match start_walk_or_give_up(walk, Goal::Point(goal), 0.0, true)? {
                        Some(state) => state,
                        None => self.move_wait.give_up(waits, stance),
                    },
                );
                Ok(())
            }
            // case 0xde: the MoveTo ended (its update in state 1 returns 1): a villager goes
            // back to the foundation (set_state 4)
            // (its update returned 1, or the list's without a sub-action: in reach or not)
            (CLEAR, Some(MoveTo::Arrived) | None) => self.back(&walk),
            // case 10, the walk to the point: the MoveTo at it (1), failed (3) or gone (the list
            // empty, 1) makes the Build wait (set_state(3)); its target gone (4) cannot be, as
            // it has none
            (WALK_POINT, Some(MoveTo::Arrived | MoveTo::Failed) | None) => {
                return self
                    .idle(&mut walk, site, builder, &mut step)
                    .map(|()| step);
            }
            // the default case (states 0, 3, 5, 8 to 10, 12 and 14 to 221): the update returns 0
            _ => Ok(()),
        };
        r.map(|()| step)
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
        if o.order != BUILD_ORDER {
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
