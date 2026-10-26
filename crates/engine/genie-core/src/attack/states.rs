//! The Attack's `set_state` (`0x140a820c0`): the move, the teardown and prelude, a lost
//! target, one out of sight, the finish and the halt.

use crate::GenieError;
use crate::attack::{
    ARRIVED, Attack, BOAR_AI, DONE, Engagement, FAILED, HALF, LION_AI, MOVE, NO_RANGE, NONE,
    PREDATOR, Strike, VILLAGER, set_sprite,
};
use crate::math::rng::Well512;
use crate::move_to::{MoveTo, Walk};
use crate::movement::Mover;
use crate::sprite::ActiveSprite;

impl Attack {
    /// A `MoveTo` of the attack that starts arrived (its `set_state` 1, not quiet): the unit shows
    /// its idle sprite (slot 0x68) and its `+0x7b8` clears.
    pub(crate) fn arrived_sprite(&self, s: &mut Strike<'_>) -> Result<(), GenieError> {
        let g = self
            .arms
            .idle_sprite
            .ok_or(crate::turn::NO_STANDING_SPRITE)?;
        set_sprite(s.sprite, g, s.sprite_rng);
        if let Some(a) = s.anim_speed.as_mut() {
            *a = 0.0;
        }
        Ok(())
    }

    /// How far the unit reaches: the range, half a tile more without one.
    pub(super) fn reach(&self) -> f32 {
        if self.range <= NO_RANGE {
            self.range + HALF
        } else {
            self.range
        }
    }

    /// `set_state(4)` (`0x140a820c0` case 4): the sub-actions go (a `MoveTo`'s teardown stops the unit,
    /// slot 0x5d0), the range is slot 0x238's again, and a `MoveTo` to the target with that range
    /// starts (`FUN_140aa0df0`, start `FUN_140aa2100`): arrived within 0.01, else in state 3 for a
    /// unit with speed. True when it arrived.
    pub(super) fn set_move(
        &mut self,
        e: &Engagement,
        mover: &mut Mover,
    ) -> Result<bool, GenieError> {
        if self.move_to.take().is_some() {
            mover.stop()?;
        }
        self.state = MOVE;
        Ok(self.moving(e.range, e.gap))
    }

    /// `set_state(4)`'s own part: the range (slot 0x238's), the timer, the `MoveTo` to the target,
    /// which starts arrived (state 1, true) within 0.01 of a `gap` away.
    pub(super) fn moving(&mut self, range: f32, gap: f32) -> bool {
        self.range = range;
        self.timer = 0.0;
        let arrived = gap <= ARRIVED;
        self.move_to = Some(if arrived {
            MoveTo::Arrived
        } else {
            MoveTo::Start
        });
        arrived
    }

    /// The teardown of the sub-actions (`FUN_140a73fc0` on the action list): a `MoveTo` stops the
    /// unit.
    pub fn tear_down(&mut self, mover: Option<&mut Mover>) -> Result<(), GenieError> {
        if self.move_to.take().is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        Ok(())
    }

    /// The start of every `set_state` (`0x140a820c0`): the sub-actions go (a `MoveTo`'s teardown stops
    /// the unit and its animation, `+0x7b8`), then the state.
    pub(super) fn enter(
        &mut self,
        state: u8,
        walk: &mut Walk<'_>,
        s: &mut Strike<'_>,
    ) -> Result<(), GenieError> {
        if self.move_to.take().is_some() {
            walk.mover
                .as_deref_mut()
                .ok_or(crate::turn::ATTACKER_UNMOVING)?
                .stop()?;
            if let Some(a) = s.anim_speed.as_mut() {
                *a = 0.0;
            }
        }
        self.state = state;
        Ok(())
    }

    /// A unit's own Attack whose target died (the update's start, not quiet: `+0x60` 0): its unit
    /// AI forgets the target (`FUN_140c5e1e0`; a villager keeps a Gaia target), the target goes
    /// (slot 0x68 with none) and the point; a human player's unit whose unit AI has an order and
    /// an empty queue (or no unit AI) posts 0x1fa with 600 (the action completed), any other
    /// with a unit AI whose queue is empty 0x1fb with 600 and 0x202; it ends (`set_state` 1) and
    /// returns 5, which keeps it in the list. A dead tree (class 15) up to state 5, which keeps
    /// a non-villager at it, and a mover of stance 0 that waits instead (`set_state` 5) are not
    /// modelled.
    pub(super) fn lost(
        &mut self,
        walk: &mut Walk<'_>,
        s: &mut Strike<'_>,
    ) -> Result<u8, GenieError> {
        if s.ai_order.is_some() && (s.victim.player != 0 || s.own.class != VILLAGER) {
            s.forgets = true;
        }
        s.let_go = Some(self.target);
        self.target = -1;
        self.point = [NONE; 3];
        let human = s.own.player != 0 && !s.computer;
        if human && s.ai_order.is_none_or(|o| o != -1 && s.queue_empty) {
            let speed = walk.mover.as_ref().map_or(0.0, |m| m.stats.speed);
            if s.stance == 0 && 0.0 < speed && s.own.class != VILLAGER {
                return Err(GenieError::unsupported(
                    "an Attack whose target died waiting for another (set_state 5)",
                ));
            }
            s.posted.push((0x1fa, [600, 0, 0]));
        } else if s.ai_order.is_some() && s.queue_empty {
            s.posted.push((0x1fb, [600, 0, 0x202]));
        }
        self.finish(walk, s)?;
        Ok(5)
    }

    /// The update's answer to a target out of sight ([`Attack::update`]): the target goes (slot
    /// 0x68 with none), the attack fails (`set_state` 13, as 1), the unit AI forgets the target
    /// (`FUN_140c5e1e0`), a predator's unit AI's slot 0x1f0 runs (the boar's sends it home at its
    /// next idle check, `sends_home`; the lion's does nothing) and a unit's own Attack whose unit AI's queue is empty posts 0x1f9 with 600;
    /// returns 3, which deletes it.
    pub(super) fn out_of_sight(
        &mut self,
        walk: &mut Walk<'_>,
        s: &mut Strike<'_>,
    ) -> Result<u8, GenieError> {
        s.let_go = Some(self.target);
        self.target = -1;
        self.finish_in(FAILED, walk, s)?;
        s.forgets = s.ai_order.is_some();
        if s.own.class == PREDATOR {
            match s.ai_type {
                Some(BOAR_AI) => s.sends_home = true,
                Some(LION_AI) | None => {}
                Some(_) => {
                    return Err(GenieError::unsupported(
                        "a predator's failed Attack whose unit AI is not a boar's or a lion's (slot 0x1f0)",
                    ));
                }
            }
        }
        if !self.sub_action && s.queue_empty {
            s.posted.push((crate::unit_ai::ACTION_FAILED, [600, 0, 0]));
        }
        Ok(3)
    }

    /// `set_state(1)` (case 1): the unit's animation runs at speed 1 and it takes the sprite it
    /// stands with.
    pub(super) fn finish(
        &mut self,
        walk: &mut Walk<'_>,
        s: &mut Strike<'_>,
    ) -> Result<(), GenieError> {
        self.finish_in(DONE, walk, s)
    }

    /// Slot 0x38 (`FUN_140a82ac0`) as the unit's stop calls it: `set_state(1)` (the
    /// sub-actions go, a `MoveTo`'s teardown stopping the unit and its animation; the animation
    /// runs at speed 1 and the unit takes the sprite it stands with, [`Attack::finish`]); its
    /// answer is true (the caller's), so the stop keeps the action list.
    ///
    /// # Errors
    ///
    /// An attacker walking without movement state, or without a standing sprite.
    pub(crate) fn halt(
        &mut self,
        mover: Option<&mut Mover>,
        anim_speed: &mut Option<f32>,
        sprite: &mut Option<ActiveSprite>,
        rng: &mut Well512,
    ) -> Result<(), GenieError> {
        if self.move_to.take().is_some() {
            mover.ok_or(crate::turn::ATTACKER_UNMOVING)?.stop()?;
            if let Some(a) = anim_speed.as_mut() {
                *a = 0.0;
            }
        }
        self.state = DONE;
        *anim_speed = Some(1.0);
        let idle = self.arms.idle_sprite.ok_or(GenieError::unsupported(
            "an Attack's unit without a standing sprite",
        ))?;
        set_sprite(sprite, idle, rng);
        Ok(())
    }

    /// `set_state` cases 1, 13 and 14: as [`Attack::finish`], in `state`.
    pub(super) fn finish_in(
        &mut self,
        state: u8,
        walk: &mut Walk<'_>,
        s: &mut Strike<'_>,
    ) -> Result<(), GenieError> {
        self.enter(state, walk, s)?;
        *s.anim_speed = Some(1.0);
        let idle = self.arms.idle_sprite.ok_or(GenieError::unsupported(
            "an Attack's unit without a standing sprite",
        ))?;
        set_sprite(s.sprite, idle, s.sprite_rng);
        Ok(())
    }
}
