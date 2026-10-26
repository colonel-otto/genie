//! A defeated player's objects stop (`FUN_141613ed0`): each object's stop and its top action's
//! halt by kind.

use crate::game_status::{
    FISHING_BOAT, KEEPING_TYPES, KEPT_ACTION, LIVESTOCK, TRADE_ACTION, TRADE_CART,
    UNSTOPPED_BUILDINGS, VILLAGER,
};
use crate::math::rng::Stream;
use crate::turn::{Releaser, Turns};
use crate::unit_ai::{Effect, GenieError};

impl Turns {
    /// `FUN_141613ed0`: a defeated player's objects in its live list's order. A villager, a
    /// fishing boat and an explorer (unit AI action `0x2bd` or 605) empty their ignore lists
    /// (`FUN_141197da0`), stop (slot 0x148, [`Turns::object_stop`]) and stand (slot 0x560:
    /// the stop of their movement, slot 0x5d0, and their walked level's reset,
    /// `FUN_140f98e80`); a building stops but for a few masters (`FUN_1411e6290`) and gives
    /// back its queue ([`Turns::cancel_queue`]); every unit AI's orders go (`FUN_140c5e010`). The
    /// slice holds no unit groups (`+0x6d0`).
    pub(super) fn stop_everything(&mut self, p: usize) -> Result<(), GenieError> {
        use crate::explore::EXPLORE_ACTION;
        use crate::unit_ai::AUTO_SCOUT_ACTION;
        for idx in 0..self.players[p].objects.len() {
            let unit = &self.players[p].objects[idx];
            let master = self.master_of(p, unit.life.master_id)?;
            let (class, id) = (master.class, master.id);
            let explorer = unit
                .ai
                .as_ref()
                .is_some_and(|ai| matches!(ai.current_action, AUTO_SCOUT_ACTION | EXPLORE_ACTION));
            if matches!(class, VILLAGER | FISHING_BOAT) || explorer {
                if let Some(m) = self.players[p].objects[idx].mover.as_mut() {
                    m.ignore_units.clear();
                }
                self.object_stop(p, idx)?;
                if let Some(m) = self.players[p].objects[idx].mover.as_mut() {
                    m.stand()?;
                }
            }
            if class == crate::garrison::BUILDING_CLASS {
                if !UNSTOPPED_BUILDINGS.contains(&id) {
                    self.object_stop(p, idx)?;
                }
                self.cancel_queue(p, idx)?;
            }
            if let Some(ai) = self.players[p].objects[idx].ai.as_mut() {
                ai.orders.clear();
            }
        }
        Ok(())
    }

    /// The object stop (slot 0x148, `FUN_14108c050`; a building's `FUN_1411e6290`) of the object
    /// at `idx` of player `p`: the action object's stop (`FUN_14106fa60`): its top action's
    /// slot 0x38 ([`Turns::halt_top_action`]); the action list goes ([`Turns::clear_actions`])
    /// unless that answered true for a unit that is no trade cart with goods; then a unit AI of
    /// a class other than 58 whose action value is not `0x2c2` and whose top action is no trade
    /// (0x6f) with a load stops (slot 0xb0 with 0: the order ends, and stopAction, slot 0x3c0)
    /// and forgets its target and its last target (`FUN_140c4dcb0`).
    pub(crate) fn object_stop(&mut self, p: usize, idx: usize) -> Result<(), GenieError> {
        let unit = &self.players[p].objects[idx];
        let master = self.master_of(p, unit.life.master_id)?;
        let class = master.class;
        if unit
            .top_action_kind()
            .is_some_and(|k| KEEPING_TYPES.contains(&k))
        {
            return Err(GenieError::unsupported(
                "a stop of an action of type 0x7c or 0x7d",
            ));
        }
        let keeps = self.halt_top_action(p, idx)?;
        if keeps && class == TRADE_CART {
            return Err(GenieError::unsupported(
                "a trade cart's goods after its stop (+0x7a0, FUN_141188160)",
            ));
        }
        // FUN_14106fa60: the list stays when the top action's slot 0x38 answered true and the
        // unit is no trade cart with goods (FUN_14106a940)
        if !keeps {
            self.clear_actions(p, idx)?;
        }
        let unit = &mut self.players[p].objects[idx];
        let place = ([unit.position[0], unit.position[1]], class);
        // slot 0x438 is the top action's type: a trade (0x6f) with a load held (COMISS 0, held;
        // JNC at 0x14108c11e: a NaN load counts) keeps the unit AI going
        let trading = unit.top_action_kind() == Some(TRADE_ACTION)
            && crate::math::fmath::below_or_unordered(0.0, unit.life.amount)
            && unit.life.attribute_type != -1;
        let Some(ai) = unit.ai.as_mut() else {
            return Ok(());
        };
        if class == LIVESTOCK || ai.current_action == KEPT_ACTION || trading {
            return Ok(());
        }
        let mut fx = Vec::new();
        ai.stop_with(false, Some(place), &mut fx)?;
        for e in fx {
            match e {
                Effect::Stop => self.command_stop(p, idx)?,
                _ => return Err(GenieError::unsupported("a stop's effect not modelled")),
            }
        }
        if let Some(ai) = self.players[p].objects[idx].ai.as_mut() {
            ai.forget_targets();
        }
        Ok(())
    }

    /// The top action's slot 0x38 before the stop clears the list (`FUN_140a97770`): false
    /// without one, or for a `MoveTo`, an Explore or an Enter (the base's, `FUN_140a7bbe0`). A
    /// human player's Hunt (`FUN_140ac79b0`) goes to state 2 (`0x140ac60e0` case 2): its
    /// sub-actions go, the unit stands with the hunt's sprite (slot 0x68 with
    /// `FUN_140ac5210`'s), its target goes (slot 0x68 with none, [`Turns::release`]) and its
    /// second target (`FUN_140a7a190` with none), its point and timer clear; then the pasture it
    /// tends lets it go (`FUN_141189ca0` with −1), and the answer is true. The Fly (Bird), the
    /// missile's, the Make and the wait for a foundation keep the base's too; the Gather, the
    /// Build, the Attack and the Artifact halt ([`Turns::halt_other`]).
    pub(super) fn halt_top_action(&mut self, p: usize, idx: usize) -> Result<bool, GenieError> {
        let computer = self.players[p].computer;
        let by = self.releaser(p, idx);
        let master_id = self.players[p].objects[idx].life.master_id;
        let standing = self.master_of(p, master_id)?.standing_sprite;
        let unit = &mut self.players[p].objects[idx];
        let Some(kind) = unit.top_action_kind() else {
            return Ok(false);
        };
        match kind {
            crate::move_to::MOVE_TO
            | crate::explore::EXPLORE
            | crate::garrison::ENTER
            | crate::fly::FLY
            | crate::missile::MISSILE
            | crate::make::MAKE
            | crate::wait_solidify::WAIT_SOLIDIFY => {
                return Ok(false);
            }
            crate::hunt::HUNT => {}
            _ => return self.halt_other(p, idx, kind),
        }
        if computer {
            return Err(GenieError::unsupported(
                "a computer player's hunt stopped (set_state 1)",
            ));
        }
        let hunt = unit.hunt.as_mut().expect("its top action");
        hunt.tear_down(unit.mover.as_mut())?;
        hunt.state = crate::hunt::IDLE;
        let sprite = crate::hunt::stand_sprite(
            &hunt.task,
            standing,
            unit.life.attribute_type,
            unit.life.amount,
            &self.graphics,
        );
        // the target, then the second target
        let released = [hunt.target, hunt.carry_to];
        hunt.target = -1;
        hunt.carry_to = -1;
        hunt.point = [-1.0; 3];
        hunt.timer = 0.0;
        if let Some(g) = sprite {
            crate::attack::set_sprite(&mut unit.sprite, g, self.streams.get(Stream::Sprite));
        }
        if unit.my_pasture != -1 {
            return Err(GenieError::unsupported(
                "a stopped hunter that tends a pasture",
            ));
        }
        for t in released {
            if t != -1 {
                self.release(t, by)?;
            }
        }
        Ok(true)
    }

    /// Slot 0x38 of the top actions with their own besides the Hunt's, each true: the Gather's
    /// (`FUN_140a8e1b0`, [`crate::gather::GatherAction::halt`]: its target and second target go,
    /// [`Turns::release`]), the Build's (`FUN_140ab0350`, [`crate::build::BuildAction::halt`]:
    /// a counted builder leaves its foundation's builders, [`Turns::leave_builders`], before the
    /// rest; the foundation goes), the Attack's (`FUN_140a82ac0`,
    /// [`crate::attack::Attack::halt`]) and the Artifact's (`FUN_140aad1a0`,
    /// [`crate::artifact::Artifact::stop`]). The Gather, the Build and the Artifact show the
    /// master's standing sprite (the Gather the task's carrying one while it carries at least 1 of
    /// the task's resource, `FUN_140a8b6b0`). [`crate::turn::Unit::top_action`] holds no other
    /// kind.
    pub(super) fn halt_other(
        &mut self,
        p: usize,
        idx: usize,
        kind: i16,
    ) -> Result<bool, GenieError> {
        let by = self.releaser(p, idx);
        let master_id = self.players[p].objects[idx].life.master_id;
        let standing = self.master_of(p, master_id)?.standing_sprite;
        match kind {
            crate::gather::GATHER => self.halt_gather(p, idx, standing, by)?,
            crate::build::BUILD => self.halt_build(p, idx, by)?,
            crate::repair::REPAIR => self.halt_repair(p, idx)?,
            crate::attack::ATTACK => {
                let unit = &mut self.players[p].objects[idx];
                let attack = unit.attack.as_mut().expect("its top action");
                attack.halt(
                    unit.mover.as_mut(),
                    &mut unit.anim_speed,
                    &mut unit.sprite,
                    self.streams.get(Stream::Sprite),
                )?;
            }
            crate::artifact::ARTIFACT_TYPE => self.halt_artifact(p, idx, by)?,
            _ => {
                return Err(GenieError::unsupported(
                    "a stop of an action whose slot 0x38 is not modelled",
                ));
            }
        }
        Ok(true)
    }

    /// [`Turns::halt_other`] of a Gather: it halts with `standing` (the task's
    /// carrying one while it carries at least 1 of the task's resource); its target and second
    /// target go, released by `by`. `standing` is the master's standing sprite.
    pub(super) fn halt_gather(
        &mut self,
        p: usize,
        idx: usize,
        standing: i16,
        by: Releaser,
    ) -> Result<(), GenieError> {
        let unit = &mut self.players[p].objects[idx];
        let gather = unit.gather.as_mut().expect("its top action");
        let stand = crate::hunt::stand_sprite(
            &gather.task,
            standing,
            unit.life.attribute_type,
            unit.life.amount,
            &self.graphics,
        );
        let released = gather.halt(
            unit.mover.as_mut(),
            stand,
            &mut unit.sprite,
            self.streams.get(Stream::Sprite),
        )?;
        for t in released {
            if t != -1 {
                self.release(t, by)?;
            }
        }
        Ok(())
    }

    /// [`Turns::halt_other`] of a Build: a counted builder leaves its foundation's builders, the
    /// unit stands, and the foundation and the second target go, released by `by`.
    pub(super) fn halt_build(
        &mut self,
        p: usize,
        idx: usize,
        by: Releaser,
    ) -> Result<(), GenieError> {
        let unit = &mut self.players[p].objects[idx];
        let build = unit.build.as_mut().expect("its top action");
        let foundation = build.target;
        let (counted, targets) = build.halt(unit.mover.as_mut())?;
        if counted {
            self.leave_builders(foundation)?;
        }
        self.standing_sprite(p, idx)?;
        for t in targets {
            if t != -1 {
                self.release(t, by)?;
            }
        }
        Ok(())
    }

    /// [`Turns::halt_other`] of a Repair (`FUN_140adaa00`): `set_state(2)`, a counted repairer
    /// out of its target's repairers, the standing sprite; the target stays.
    pub(super) fn halt_repair(&mut self, p: usize, idx: usize) -> Result<(), GenieError> {
        let unit = &mut self.players[p].objects[idx];
        let repair = unit.repair.as_mut().expect("its top action");
        let target = repair.target;
        if repair.halt(unit.mover.as_mut())? {
            self.leave_builders(target)?;
        }
        self.standing_sprite(p, idx)
    }

    /// [`Turns::halt_other`] of an Artifact: its sub-actions and its target go (released by
    /// `by`, the Enter's first), a walking animal stops, and it stands.
    pub(super) fn halt_artifact(
        &mut self,
        p: usize,
        idx: usize,
        by: Releaser,
    ) -> Result<(), GenieError> {
        let unit = &mut self.players[p].objects[idx];
        let artifact = unit.artifact.as_mut().expect("its top action");
        let (gone, released) = artifact.stop();
        if gone.stops {
            unit.mover
                .as_mut()
                .ok_or(GenieError::unsupported(
                    "a walking animal without movement state",
                ))?
                .stop()?;
        }
        for t in [gone.released, released].into_iter().flatten() {
            self.release(t, by)?;
        }
        self.standing_sprite(p, idx)
    }
}
