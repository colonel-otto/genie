//! The Attack's `set_state` (`0x140a820c0`): the move, the teardown and prelude, a lost
//! target, one out of sight, the finish and the halt.

use crate::GenieError;
use crate::combat::attack::update::let_go;
use crate::combat::attack::{
    ARRIVED, AiOrder, Attack, AttackReads, BOAR_AI, Engagement, Fight, HALF, LION_AI, MOVE,
    NO_RANGE, NONE, set_walker_anim,
};
use crate::math::rng::{Stream, Well512};
use crate::movement::Mover;
use crate::movement::move_to::MoveTo;
use crate::objects::layers::Body;
use crate::objects::masters::class;
use crate::objects::sprite::ActiveSprite;
use crate::objects::sprite::new_sprite;
use crate::objects::unit::Unit;
use crate::orders::action_state::DONE;
use crate::orders::action_state::FAILED;
use crate::world::step::Step;

impl Attack {
    /// A `MoveTo` of the attack that starts arrived (its `set_state` 1, not quiet): the unit shows
    /// its idle sprite (slot 0x68) and its `+0x7b8` clears.
    pub(crate) fn arrived_sprite(
        &self,
        unit: &mut Unit,
        sprite_rng: &mut Well512,
    ) -> Result<(), GenieError> {
        let g = self
            .arms
            .idle_sprite
            .ok_or(crate::world::NO_STANDING_SPRITE)?;
        new_sprite(&mut unit.sprite, unit.body.move_stats_mut(), g, sprite_rng);
        set_walker_anim(&mut unit.body, 0.0);
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
    pub(super) fn enter(&mut self, state: u8, unit: &mut Unit) -> Result<(), GenieError> {
        if self.move_to.take().is_some() {
            unit.body
                .mover_mut()
                .ok_or(crate::world::ATTACKER_UNMOVING)?
                .stop()?;
        }
        self.state = state;
        Ok(())
    }

    /// A unit's own Attack whose target died (the update's start, not quiet: `+0x60` 0): its unit
    /// AI forgets the target at once (`FUN_140c5e1e0`; a villager keeps a Gaia target), the
    /// target goes (slot 0x68 with none: let go at once) and the point; a human player's unit
    /// whose unit AI has an order and an empty queue (or no unit AI) posts 0x1fa with 600 (the
    /// action completed), any other with a unit AI whose queue is empty 0x1fb with 600 and
    /// 0x202; it ends (`set_state` 1) and returns 5, which keeps it in the list. A dead tree
    /// (class 15) up to state 5, which keeps a non-villager at it, and a mover of stance 0 that
    /// waits instead (`set_state` 5) are not modelled.
    pub(super) fn lost(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        f: &Fight,
        r: &AttackReads,
    ) -> Result<u8, GenieError> {
        if r.ai_order.has_ai() && (f.victim.player != 0 || f.own.class != class::VILLAGER) {
            forget_target(unit);
        }
        let target = std::mem::replace(&mut self.target, -1);
        let_go(c, unit, target)?;
        self.point = [NONE; 3];
        let human = f.own.player != 0 && !r.computer;
        let waits = match r.ai_order {
            AiOrder::Absent => true,
            AiOrder::Idle => false,
            AiOrder::Order(_) => r.queue_empty,
        };
        if human && waits {
            let speed = unit.body.mover().map_or(0.0, |m| m.stats.speed);
            if r.stance == 0 && 0.0 < speed && f.own.class != class::VILLAGER {
                return Err(GenieError::unsupported(
                    "an Attack whose target died waiting for another (set_state 5)",
                ));
            }
            unit.post_own(0x1fa, [600, 0, 0])?;
        } else if r.ai_order.has_ai() && r.queue_empty {
            unit.post_own(0x1fb, [600, 0, 0x202])?;
        }
        self.finish(unit, c.w.streams.get(Stream::Sprite))?;
        Ok(5)
    }

    /// The update's answer to a target out of sight ([`Attack::update`]): the target goes (slot
    /// 0x68 with none), the attack fails (`set_state` 13, as 1); a unit's own Attack lets the
    /// target go, its unit AI forgets it (`FUN_140c5e1e0`) and a predator's unit AI's slot 0x1f0
    /// runs (the boar's sends it home at its next idle check, `+0x1fd`; the lion's does nothing),
    /// each at once, and with its unit AI's queue empty it posts 0x1f9 with 600; returns 3,
    /// which deletes it. The Hunt's Attack, a sub-action, does none of these (MISSING.md, "The
    /// Hunt's Attack out of sight").
    pub(super) fn out_of_sight(
        &mut self,
        c: &mut Step<'_>,
        unit: &mut Unit,
        f: &Fight,
        r: &AttackReads,
    ) -> Result<u8, GenieError> {
        let target = std::mem::replace(&mut self.target, -1);
        self.finish_in(FAILED, unit, c.w.streams.get(Stream::Sprite))?;
        if f.own.class == class::PREDATOR && !matches!(r.ai_type, Some(BOAR_AI | LION_AI) | None) {
            return Err(GenieError::unsupported(
                "a predator's failed Attack whose unit AI is not a boar's or a lion's (slot 0x1f0)",
            ));
        }
        if self.sub_action {
            return Ok(3);
        }
        let_go(c, unit, target)?;
        if r.ai_order.has_ai() {
            forget_target(unit);
        }
        if f.own.class == class::PREDATOR
            && r.ai_type == Some(BOAR_AI)
            && let Some(home) = unit.ai.as_mut().and_then(|ai| ai.home.as_mut())
        {
            home.go_home = true;
        }
        if r.queue_empty {
            unit.post_own(crate::unit_ai::ACTION_FAILED, [600, 0, 0])?;
        }
        Ok(3)
    }

    /// `set_state(1)` (case 1): the unit's animation runs at speed 1 and it takes the sprite it
    /// stands with.
    pub(super) fn finish(
        &mut self,
        unit: &mut Unit,
        sprite_rng: &mut Well512,
    ) -> Result<(), GenieError> {
        self.finish_in(DONE, unit, sprite_rng)
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
        body: &mut Body,
        sprite: &mut Option<ActiveSprite>,
        rng: &mut Well512,
    ) -> Result<(), GenieError> {
        if self.move_to.take().is_some() {
            body.mover_mut()
                .ok_or(crate::world::ATTACKER_UNMOVING)?
                .stop()?;
        }
        self.state = DONE;
        body.set_anim_speed(1.0);
        let idle = self.arms.idle_sprite.ok_or(GenieError::unsupported(
            "an Attack's unit without a standing sprite",
        ))?;
        new_sprite(sprite, body.move_stats_mut(), idle, rng);
        Ok(())
    }

    /// `set_state` cases 1, 13 and 14: as [`Attack::finish`], in `state`.
    pub(super) fn finish_in(
        &mut self,
        state: u8,
        unit: &mut Unit,
        sprite_rng: &mut Well512,
    ) -> Result<(), GenieError> {
        self.enter(state, unit)?;
        set_walker_anim(&mut unit.body, 1.0);
        let idle = self.arms.idle_sprite.ok_or(GenieError::unsupported(
            "an Attack's unit without a standing sprite",
        ))?;
        new_sprite(
            &mut unit.sprite,
            unit.body.move_stats_mut(),
            idle,
            sprite_rng,
        );
        Ok(())
    }
}

/// `FUN_140c5e1e0` at once: the unit AI forgets its target ([`crate::unit_ai::UnitAi::remove_target`]).
const fn forget_target(unit: &mut Unit) {
    if let Some(ai) = unit.ai.as_mut() {
        ai.remove_target();
    }
}
