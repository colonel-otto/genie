//! The install of a unit's own `MoveTo` (slot 0x808 with it, `FUN_14106e990`): from the world
//! and from the unit's step, then its start on the unit.

use crate::GenieResult;
use crate::map::Distance;
use crate::math::rng::Stream;
use crate::math::rng::Well512;
use crate::movement::ai_moves::MoveSprites;
use crate::movement::move_to::MoveToAction;
use crate::objects::ListIndex;
use crate::objects::layers::Action;
use crate::objects::layers::MotionMut;
use crate::objects::masters::OwnerMasters;
use crate::objects::masters::PlayersMastersExt;
use crate::objects::unit::Unit;
use crate::players::PlayerId;
use crate::world::World;
use crate::world::slots::PlayerSlotsExt;
use crate::world::step::Step;

impl World {
    /// Slot 0x808 with a new `MoveTo` of its own (`move_to`, its constructor's) of object `idx` of
    /// player `player`, outside the unit's update: the move to a point or an object
    /// (`FUN_14106d340`) and the move off a footprint (`FUN_1410866e0`). The install
    /// ([`World::set_only_action`]: the list's deletion, then its add) deletes the `MoveTo`
    /// unstarted for a unit in state 3 or later: its teardown (`FUN_140aa1f30`) stops the unit
    /// and lets its target go. Else the add starts it ([`Unit::start_own_move_to`]) by its gap to
    /// its target ([`MoveToAction::start_gap`], from the unit's master's radii) and the unit's
    /// top speed after the deletion (slot 0x208), and the list holds it.
    ///
    /// # Errors
    ///
    /// The install's, a master the unit's player lacks, a target the world lacks, and the
    /// standing sprite's.
    pub(crate) fn install_move_to(
        &mut self,
        player: PlayerId,
        idx: ListIndex,
        move_to: MoveToAction,
    ) -> GenieResult<()> {
        let Some(move_to) = self.set_only_action(player, idx, Some(move_to))? else {
            return Ok(());
        };
        let unit = self.players.live(player, idx)?;
        let ground = unit.master(self.master_tables(), player)?.radius;
        let gap = move_to.start_gap(&self.spatial, unit.position(), ground)?;
        let top_speed = self.top_speed_terms(player, unit)?;
        let id = unit.id;
        self.players.live_mut(player, idx)?.start_own_move_to(
            self.parts.actions.of_mut(id)?,
            self.parts.motion.layer_mut(id),
            move_to,
            gap,
            MoveSprites {
                masters: OwnerMasters::new(&self.parts.own_master, self.model.masters.of(player)),
                graphics: &self.model.graphics,
                look: self.parts.combat.of(id).abilities.look.as_ref(),
                roles: &self.model.roles,
                sprite_tuning: self.model.tuning.sprite,
                move_to_tuning: &self.model.tuning.move_to,
                top_speed,
            },
            self.streams.get_mut(Stream::Sprite),
        )
    }
}

impl Unit {
    /// [`World::install_move_to`] on the unit's own list from its update (`FUN_1410866e0`,
    /// `FUN_141086440`): the install ([`Unit::set_only_own_action`]), then the start of the
    /// `MoveTo` it hands back ([`Unit::start_own_move_to`]).
    ///
    /// # Errors
    ///
    /// As [`World::install_move_to`].
    pub(crate) fn install_own_move_to(
        &mut self,
        actions: &mut Action,
        move_to: MoveToAction,
        step: &mut Step<'_>,
    ) -> GenieResult<()> {
        let Some(move_to) = self.set_only_own_action(actions, Some(move_to), step)? else {
            return Ok(());
        };
        let ground = self.master(step.world.master_tables(), step.player)?.radius;
        let gap = move_to.start_gap(&step.world.spatial, self.position(), ground)?;
        let top_speed = step.world.top_speed_terms(step.player, self)?;
        self.start_own_move_to(
            actions,
            step.world.parts.motion.layer_mut(self.id),
            move_to,
            gap,
            MoveSprites {
                masters: OwnerMasters::new(
                    &step.world.parts.own_master,
                    step.world.model.masters.of(step.player),
                ),
                graphics: &step.world.model.graphics,
                look: step.world.parts.combat.of(self.id).abilities.look.as_ref(),
                roles: &step.world.model.roles,
                sprite_tuning: step.world.model.tuning.sprite,
                move_to_tuning: &step.world.model.tuning.move_to,
                top_speed,
            },
            step.world.streams.get_mut(Stream::Sprite),
        )
    }

    /// The start (slot 5 with 1, `0x140aa2100`) of the `MoveTo` of its own that the install
    /// handed back (`move_to`, `gap` its start's gap to its target), on the unit with its action
    /// layer `actions` and its motion layer `motion`: its state by the gap and the unit's top
    /// speed (slot 0x208 with the terms in `sprites`, [`MoveToAction::start`]); one that ends at
    /// its start shows the standing sprite of the unit's master ([`Unit::shows_standing_sprite`],
    /// a draw of `sprite_rng`); then the list holds it.
    ///
    /// # Errors
    ///
    /// A master the owner's masters lack, and the standing sprite's.
    pub(crate) fn start_own_move_to(
        &mut self,
        actions: &mut Action,
        mut motion: MotionMut<'_>,
        mut move_to: MoveToAction,
        gap: Distance,
        sprites: MoveSprites<'_>,
        sprite_rng: &mut Well512,
    ) -> GenieResult<()> {
        let top = sprites
            .top_speed
            .of_object(motion.mover(), self.position().ground());
        move_to.start(gap, top, sprites.move_to_tuning);
        if move_to.shows_standing() {
            self.shows_standing_sprite(
                motion.reborrow(),
                sprites.masters,
                sprites.graphics,
                sprite_rng,
                sprites.sprite_tuning,
            )?;
        }
        actions.list.set_move_to(Some(move_to));
        Ok(())
    }
}

#[cfg(test)]
mod tests;
