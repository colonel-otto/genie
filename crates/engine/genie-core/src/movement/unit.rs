//! The movement actions in a unit's update: the velocity step, the unit's own `MoveTo`, its
//! Explore and a bird's Fly.

use crate::GenieError;
use crate::math::rng::Stream;
use crate::movement::move_to::MoveTo;
use crate::movement::{GroupSpeed, Me};
use crate::objects::unit::Unit;
use crate::unit_ai::{ACTION_COMPLETED, TARGET_MOVED, UnitAi};
use crate::world::SPEED_EPSILON;
use crate::world::context::Context;

/// A group member's own move's reasons slot 0x5e0 tells apart (`FUN_141017590`): moved aside,
/// and none.
const ASIDE: i32 = 2;
const NO_OWN_MOVE: i32 = -1;

impl Unit {
    /// `RGE_Moving_Object::update` (`0x14114dbb0`) before the actions: the last human order's time
    /// counts down, the velocity step moves the unit (at the speed its group steers,
    /// [`Unit::group_speed`]), and a moving unit animates at its speed.
    ///
    /// # Errors
    ///
    /// A unit its group may steer whose master the slice does not have or whose action it does
    /// not model.
    pub(crate) fn motion_step(&mut self, c: &mut Context<'_>) -> Result<(), GenieError> {
        // RGE_Moving_Object::update counts the last human order time down before its velocity
        // step
        if self.body.mover().is_some() {
            self.count_down_human_order(c.dt, &mut c.w.spatial);
        }
        // the velocity step (FUN_140f9ba80) runs for a unit with velocity data (+0x868)
        let group = match self.body.mover().and_then(|m| m.velocity.as_ref()) {
            Some(_) => self.group_speed(&c.w.groups, c.w.masters.get(c.player))?,
            None => None,
        };
        if let Some(mover) = self.body.mover_mut() {
            let at = mover.step_velocity(
                self.position,
                c.dt_seconds,
                c.w.spatial.map,
                c.w.spatial.terrain.as_ref(),
                (c.world_time, group),
            );
            if at != self.position {
                self.position = at;
                c.w.spatial.set_position(self.id, at);
                if let Some(p) = c.w.spatial.get(self.id) {
                    c.w.pathing.moved(self.id, p, mover.is_moving());
                }
            }
            // RGE_Moving_Object::update: a unit that moves animates at its speed (+0x7b8)
            if let Some(v) = mover.velocity.as_ref() {
                let [x, y, z] = v.velocity;
                let speed = crate::math::fmath::sqrt(x * x + y * y + z * z);
                if SPEED_EPSILON < speed {
                    self.body.set_anim_speed(speed);
                }
            }
        }
        Ok(())
    }

    /// What slot 0x5e0 (`FUN_14114f300`) reads of the unit's group ([`GroupSpeed`]) among the
    /// world's `groups`, the unit's master among `masters`: none outside a group the world holds (`+0x6d0`, `FUN_14152ff40`: none for −1), nor when the
    /// group's motion stands (`FUN_141046600`) and the unit is in its place
    /// ([`UnitGroup::slot_in_place`]), nor for a unit not free for the group's moves
    /// ([`Unit::free_for_group`]). Else its own move's reason (`FUN_141017590`): 2 moved aside,
    /// −1 none, any other an own move at the group's top speed (`+0xbc`).
    ///
    /// # Errors
    ///
    /// A unit whose master the slice does not have, or whose action it does not model.
    ///
    /// [`UnitGroup::slot_in_place`]: crate::groups::UnitGroup::slot_in_place
    pub(crate) fn group_speed(
        &self,
        groups: &crate::groups::UnitGroups,
        masters: Option<&crate::objects::masters::Masters>,
    ) -> Result<Option<GroupSpeed>, GenieError> {
        let Some(g) = groups.get(self.group.id) else {
            return Ok(None);
        };
        if !g.motion.moving() && g.slot_in_place(self.id) {
            return Ok(None);
        }
        let master = masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(crate::world::OBJECT_MASTER)?;
        if !self.free_for_group(master)? {
            return Ok(None);
        }
        Ok(Some(match g.member_reason(self.id) {
            ASIDE => GroupSpeed::Aside,
            NO_OWN_MOVE => GroupSpeed::Free,
            _ => GroupSpeed::OwnMove(g.motion.maximum_velocity_forward),
        }))
    }

    /// The unit's own `MoveTo` in its update (`0x140aa2a70`): its update, then a give-up, a moved
    /// target, and its end (done or failed), which posts the notify, lets the target go and stops
    /// the unit.
    pub(crate) fn move_to_part(&mut self, c: &mut Context<'_>, me: Me) -> Result<(), GenieError> {
        let step = self.own_move_step(c, me)?;
        if step.gave_up {
            let villager =
                c.w.masters
                    .get(c.player)
                    .and_then(|m| m.get(self.life.master_id))
                    .is_some_and(|m| m.class == 4);
            let stance = self.ai.as_ref().map_or(0, |ai| ai.stance);
            self.own_move_gave_up(
                villager && !c.computer,
                stance,
                (
                    c.w.masters.get(c.player),
                    &c.w.graphics,
                    c.w.streams.get(Stream::Sprite),
                ),
            )?;
        }
        if step.target_moved {
            self.post_own(TARGET_MOVED, [0, 0, 0])?;
        }
        if step.done || step.failed {
            self.own_move_end(c, step.failed)?;
        }
        Ok(())
    }

    /// The own `MoveTo`'s update (`0x140aa2a70`): its state 3 asks the unit's AI for a path to
    /// its point (the goal, or the target's place); none without a `MoveTo`.
    fn own_move_step(
        &mut self,
        c: &mut Context<'_>,
        me: Me,
    ) -> Result<crate::movement::move_to::MoveStep, GenieError> {
        Ok(match self.actions.move_to_mut() {
            Some(move_to) => {
                // its state 3 reads the unit's AI for the path it asks (0x140aa2a70), with its
                // point: the goal, or the target's place (a target gone fails the update first)
                let point = match (move_to.target, move_to.goal) {
                    (-1, goal) => goal.map(|p| [p[0], p[1]]),
                    (id, _) => c.w.spatial.get(id).map(|t| [t.position[0], t.position[1]]),
                };
                let ask = match point {
                    Some(p) if move_to.state == MoveTo::Start => {
                        let at = [me.position[0], me.position[1]];
                        UnitAi::path_ask(self.ai.as_mut(), at, p)
                    }
                    _ => crate::movement::PathAsk::default(),
                };
                move_to.update(c.walk(self.body.mover_mut(), me), ask)?
            }
            None => crate::movement::move_to::MoveStep::default(),
        })
    }

    /// The own `MoveTo`'s end, done or `failed`: the notify, the target let go and the unit
    /// stopped.
    fn own_move_end(&mut self, c: &mut Context<'_>, failed: bool) -> Result<(), GenieError> {
        // its update in state 1 posts its completion (0x1fa with the move action value; an
        // own MoveTo is never quiet), in state 13 its failure (0x1f9) and returns 3; the
        // action list removes it (FUN_140a98e20): its teardown (0x140aa1f30) stops the unit
        // (slot 0x5d0)
        let notify = if failed {
            crate::unit_ai::ACTION_FAILED
        } else {
            ACTION_COMPLETED
        };
        self.post_own(notify, [crate::unit_ai::MOVE_ACTION, 0, 0])?;
        // ... and the base teardown (FUN_140a73fc0) lets its target go (slot 0x180)
        if let Some(t) = self
            .actions
            .take_move_to()
            .map(|m| m.target)
            .filter(|&t| t != -1)
        {
            let by = self.releaser(c);
            crate::world::lets::release(c, t, by)?;
        }
        if let Some(m) = self.body.mover_mut() {
            m.stop()?;
        }
        Ok(())
    }

    /// The unit's Explore in its update (`0x140a880c0`): its walk in state 11; states 1 (done) and
    /// 13 (failed) are not modelled, and the other states wait.
    pub(crate) fn explore_part(&mut self, c: &mut Context<'_>, me: Me) -> Result<(), GenieError> {
        if let Some(explore) = self.actions.explore_mut() {
            // 0x140a880c0: state 1 is done (the action list removes it, the default task's
            // restart is not modelled), 11 walks, 13 fails; the others wait
            match explore.state {
                crate::movement::explore::DONE => {
                    return Err(GenieError::unsupported(
                        "a done Explore (the action list after it)",
                    ));
                }
                crate::movement::explore::FAILED => {
                    return Err(GenieError::unsupported(
                        "a failed Explore (notify 0x1f9, set_state 2)",
                    ));
                }
                crate::movement::explore::WALK => {
                    // the MoveTo's update code (FUN_140a992b0): 1 once it arrived or without
                    // one, 3 once it failed (quiet: no notify)
                    let quiet = self
                        .ai
                        .as_ref()
                        .is_some_and(|ai| !matches!(ai.current_order, 711 | 733));
                    // the motion step giving up: a human player's villager waits once, any
                    // other unit's MoveTo fails (LAB_140aa320c)
                    let villager =
                        c.w.masters
                            .get(c.player)
                            .and_then(|m| m.get(self.life.master_id))
                            .is_some_and(|m| m.class == 4);
                    let waits = villager && !c.computer;
                    let stance = self.ai.as_ref().map_or(0, |ai| ai.stance);
                    let goal = crate::movement::move_to::Goal::Point(explore.point);
                    let code = crate::movement::move_to::sub_move_to_update(
                        (&mut explore.move_to, &mut explore.move_wait),
                        &mut c.walk(self.body.mover_mut(), me),
                        goal,
                        (0.0, quiet),
                        (waits, stance),
                    )?;
                    let walked = explore.move_to.is_some();
                    let owner = explore.after_walk(code)?;
                    if walked && self.actions.explore().is_some_and(|e| e.move_to.is_none()) {
                        self.stop_mover()?;
                    }
                    self.explore_owner(owner, c)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// A bird's Fly in its update: its step, the stop, and a failed flight's notify.
    pub(crate) fn fly_part(&mut self, c: &mut Context<'_>, me: Me) -> Result<(), GenieError> {
        if let Some(fly) = self.actions.fly_mut() {
            let mover = self
                .body
                .mover_mut()
                .ok_or(GenieError::unsupported("a bird without movement state"))?;
            let (speed, radius) = (
                mover.stats.speed,
                [mover.stats.radius[0], mover.stats.radius[1]],
            );
            let map = c.w.spatial.map;
            let (walk, action) = c.walk_with(Some(&mut *mover), me, Stream::Action);
            let sky = crate::movement::fly::Sky {
                speed,
                position: me.position,
                radius,
                map,
                action,
            };
            let step = fly.update(walk, sky)?;
            if step.stopped {
                mover.stop()?;
            }
            // a failed flight's MoveTo posts its failure (notify 0x1f9, the move action value)
            if step.failed {
                self.post_own(
                    crate::unit_ai::ACTION_FAILED,
                    [crate::unit_ai::MOVE_ACTION, 0, 0],
                )?;
            }
        }
        Ok(())
    }
}
