//! The movement actions in a unit's update: the velocity step, the unit's own `MoveTo`, its
//! Explore and a bird's Fly.

use crate::GenieError;
use crate::math::rng::Stream;
use crate::movement::Me;
use crate::movement::move_to::MoveTo;
use crate::objects::unit::Unit;
use crate::unit_ai::{ACTION_COMPLETED, TARGET_MOVED, UnitAi};
use crate::world::SPEED_EPSILON;
use crate::world::context::Context;

impl Unit {
    /// `RGE_Moving_Object::update` (`0x14114dbb0`) before the actions: the last human order's time
    /// counts down, the velocity step moves the unit, and a moving unit animates at its speed.
    pub(crate) fn motion_step(&mut self, c: &mut Context<'_>) {
        // RGE_Moving_Object::update counts the last human order time down before its velocity
        // step
        if self.mover.is_some() {
            self.count_down_human_order(c.dt, c.spatial);
        }
        if let Some(mover) = self.mover.as_mut() {
            let at = mover.step_velocity(
                self.position,
                c.dt_seconds,
                c.spatial.map,
                c.spatial.terrain.as_ref(),
                c.world_time,
            );
            if at != self.position {
                self.position = at;
                c.spatial.set_position(self.id, at);
                if let Some(p) = c.spatial.get(self.id) {
                    c.pathing.moved(self.id, p, mover.is_moving());
                }
            }
            // RGE_Moving_Object::update: a unit that moves animates at its speed (+0x7b8)
            if let Some(v) = mover.velocity.as_ref() {
                let [x, y, z] = v.velocity;
                let speed = crate::math::fmath::sqrt(x * x + y * y + z * z);
                if SPEED_EPSILON < speed {
                    self.anim_speed = Some(speed);
                }
            }
        }
    }

    /// The unit's own `MoveTo` in its update (`0x140aa2a70`): its update, then a give-up, a moved
    /// target, and its end (done or failed), which posts the notify, lets the target go and stops
    /// the unit.
    pub(crate) fn move_to_part(&mut self, c: &mut Context<'_>, me: Me) -> Result<(), GenieError> {
        let step = match self.move_to.as_mut() {
            Some(move_to) => {
                // its state 3 reads the unit's AI for the path it asks (0x140aa2a70), with its
                // point: the goal, or the target's place (a target gone fails the update first)
                let point = match (move_to.target, move_to.goal) {
                    (-1, goal) => goal.map(|p| [p[0], p[1]]),
                    (id, _) => c.spatial.get(id).map(|t| [t.position[0], t.position[1]]),
                };
                let ask = match point {
                    Some(p) if move_to.state == MoveTo::Start => {
                        let at = [me.position[0], me.position[1]];
                        UnitAi::path_ask(self.ai.as_mut(), at, p)
                    }
                    _ => crate::movement::PathAsk::default(),
                };
                move_to.update(c.walk(self.mover.as_mut(), me), ask)?
            }
            None => crate::movement::move_to::MoveStep::default(),
        };
        if step.gave_up {
            let villager = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .is_some_and(|m| m.class == 4);
            let stance = self.ai.as_ref().map_or(0, |ai| ai.stance);
            self.own_move_gave_up(
                villager && !c.computer,
                stance,
                (c.masters, c.graphics, c.streams.get(Stream::Sprite)),
            )?;
        }
        if step.target_moved {
            self.post_own(TARGET_MOVED, [0, 0, 0])?;
        }
        if step.done || step.failed {
            // its update in state 1 posts its completion (0x1fa with the move action value; an
            // own MoveTo is never quiet), in state 13 its failure (0x1f9) and returns 3; the
            // action list removes it (FUN_140a98e20): its teardown (0x140aa1f30) stops the unit
            // (slot 0x5d0)
            let notify = if step.failed {
                crate::unit_ai::ACTION_FAILED
            } else {
                ACTION_COMPLETED
            };
            self.post_own(notify, [crate::unit_ai::MOVE_ACTION, 0, 0])?;
            // ... and the base teardown (FUN_140a73fc0) lets its target go (slot 0x180)
            if let Some(t) = self.move_to.take().map(|m| m.target).filter(|&t| t != -1) {
                c.releases.push((t, self.releaser(c)));
            }
            if let Some(m) = self.mover.as_mut() {
                m.stop()?;
            }
        }
        Ok(())
    }

    /// The unit's Explore in its update (`0x140a880c0`): its walk in state 11; states 1 (done) and
    /// 13 (failed) are not modelled, and the other states wait.
    pub(crate) fn explore_part(&mut self, c: &mut Context<'_>, me: Me) -> Result<(), GenieError> {
        if let Some(explore) = self.explore.as_mut() {
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
                    // the MoveTo's update code (FUN_140a992b0): 1 once it arrived, 1 without it
                    let quiet = self
                        .ai
                        .as_ref()
                        .is_some_and(|ai| !matches!(ai.current_order, 711 | 733));
                    // the motion step giving up: a human player's villager waits once, any
                    // other unit's MoveTo fails (LAB_140aa320c)
                    let villager = c
                        .masters
                        .and_then(|m| m.get(self.life.master_id))
                        .is_some_and(|m| m.class == 4);
                    let waits = villager && !c.computer;
                    let stance = self.ai.as_ref().map_or(0, |ai| ai.stance);
                    let code = match explore.move_to {
                        Some(MoveTo::Start) => {
                            if explore.move_wait.state == 1 {
                                explore.move_wait.state = 2;
                            }
                            explore.move_to = Some(MoveTo::Moving);
                            let goal = crate::movement::move_to::Goal::Point(explore.point);
                            let walk = c.walk(self.mover.as_mut(), me);
                            explore.move_to = Some(
                                match crate::movement::move_to::start_walk_or_give_up(
                                    walk, goal, 0.0, quiet,
                                )? {
                                    Some(state) => state,
                                    None => explore.move_wait.give_up(waits, stance),
                                },
                            );
                            0
                        }
                        Some(MoveTo::Moving) => {
                            let walk = c.walk(self.mover.as_mut(), me);
                            explore.move_to =
                                Some(match crate::movement::move_to::walk_or_give_up(walk)? {
                                    Some(state) => state,
                                    None => explore.move_wait.give_up(waits, stance),
                                });
                            0
                        }
                        // case 2: the wait runs by dt, then the MoveTo asks again (set_state 3)
                        Some(MoveTo::Waiting) => {
                            explore.move_wait.timer += c.dt_seconds;
                            if explore.move_wait.delay <= explore.move_wait.timer {
                                explore.move_wait.timer = 0.0;
                                explore.move_to = Some(MoveTo::Start);
                            }
                            0
                        }
                        // its update in state 13 returns 3 (quiet: no notify)
                        Some(MoveTo::Failed) => 3,
                        Some(MoveTo::Arrived) | None => 1,
                    };
                    let walked = explore.move_to.is_some();
                    let owner = explore.after_walk(code)?;
                    if walked && self.explore.is_some_and(|e| e.move_to.is_none()) {
                        self.stop_mover()?;
                    }
                    self.explore_owner(owner, c)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// A bird's Fly in its update: its step, the stop, and a failed flight's notify. A Fly that
    /// this update started (`flies_from_now`) waits for the next update.
    pub(crate) fn fly_part(
        &mut self,
        c: &mut Context<'_>,
        me: Me,
        flies_from_now: bool,
    ) -> Result<(), GenieError> {
        if let Some(fly) = self.fly.as_mut().filter(|_| !flies_from_now) {
            let mover = self
                .mover
                .as_mut()
                .ok_or(GenieError::unsupported("a bird without movement state"))?;
            let (speed, radius) = (
                mover.stats.speed,
                [mover.stats.radius[0], mover.stats.radius[1]],
            );
            let map = c.spatial.map;
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
