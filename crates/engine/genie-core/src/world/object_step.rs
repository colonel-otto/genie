//! One object's step with its [`Step`] ([`World::run_object`]) and a unit AI run outside its
//! update.

use crate::GenieError;
use crate::construction::building_update::blasts_at_death;
use crate::objects::unit::{Outcome, Unit};
use crate::world::World;
use crate::world::step::Step;

impl World {
    /// One object's step with its `Step` (`f`: the unit's update, or one part of its unit AI
    /// for a command): object `i` of player `p` leaves its list (its stand-in holds the slot),
    /// and `f` gets it and the `Step`, which holds the world (FD-045-D28; the objects it
    /// makes join the list at their making). It goes back to the slot its stand-in holds then
    /// ([`World::put_back_own`]). Returns `f`'s result (an error of the put-back, then of the
    /// sight's registry first).
    #[allow(clippy::too_many_lines)]
    pub(super) fn run_object(
        &mut self,
        (p, i): (usize, usize),
        (dt, turn): (u32, Option<u32>),
        f: impl FnOnce(&mut Unit, &mut Step<'_>) -> Result<Outcome, GenieError>,
    ) -> Result<Outcome, GenieError> {
        let mut unit = self.players[p].objects.take_out(i);
        let watch = crate::trace::Watch::of(&unit);
        // the objects a hit's blast may reach about its target (one a hit can read), or about
        // the unit for its blast at death (TRIBE_Combat_Object::update)
        let target = unit
            .attack_target()
            .filter(|&t| self.victim(t, p).is_some());
        let blasted = match target {
            Some(t) => self.blasted(&unit, p, t),
            None if unit
                .master(&self.masters, p)
                .is_ok_and(|m| blasts_at_death(m, &unit.life)) =>
            {
                self.blasted(&unit, p, -1)
            }
            None => Vec::new(),
        };
        let (tick, me) = (self.ticks, unit.id);
        let mut c = Step {
            w: self,
            me,
            dt,
            tick,
            turn,
            blasted,
            player: p,
            taken_over: None,
            building_handover: false,
            side_notifies: None,
        };
        let mut result = f(&mut unit, &mut c);
        if let Some(w) = watch {
            w.report(&unit, c.tick, p);
        }
        if let Some(e) = self.spatial.take_sight_unsupported()
            && result.is_ok()
        {
            result = Err(e);
        }
        self.spatial.set_gait(me, unit.gait());
        // back to its stand-in's slot, which a removal in the step may have moved
        if let Err(e) = self.put_back_own(p, unit) {
            result = Err(e);
        }
        result
    }

    /// Runs `f` on object `i` of player `p` with its step outside its update: a command's part
    /// the game runs on the object at once (`UNIT_ORDER`'s drop-off), [`World::step_object`]
    /// without the update's own parts.
    ///
    /// # Errors
    ///
    /// `f`'s, and what its effects do not model.
    pub(crate) fn run_on_step(
        &mut self,
        p: usize,
        i: usize,
        f: impl FnOnce(&mut Unit, &mut Step<'_>) -> Result<(), GenieError>,
    ) -> Result<(), GenieError> {
        let _ = self.step_object((p, i), (0, None, false), |unit, c| {
            f(unit, c).map(|()| Outcome::Stay)
        })?;
        Ok(())
    }

    /// Runs `f` on the unit AI of object `i` of player `p` outside its update (a command's
    /// immediate order, `FUN_140c55c90`): the object's step ([`World::step_object`] without the
    /// update's own parts) whose only part is the AI's run with what it reads of the world
    /// ([`Unit::run_ai`]).
    ///
    /// # Errors
    ///
    /// What the AI's run and its effects do not model.
    pub(crate) fn run_unit_ai(
        &mut self,
        (p, i): (usize, usize),
        f: impl FnOnce(
            &mut crate::unit_ai::UnitAi,
            &crate::unit_ai::Surroundings,
            &mut dyn crate::unit_ai::UnitSide,
            bool,
            &mut crate::math::rng::Well512,
        ) -> Result<(), GenieError>,
    ) -> Result<(), GenieError> {
        let stepped = self.step_object((p, i), (0, None, false), |unit, c| {
            unit.run_ai(c, f).map(|()| Outcome::Stay)
        })?;
        match stepped.outcome {
            Outcome::Stay => Ok(()),
            _ => Err(GenieError::unsupported(
                "a unit AI run outside its update that takes the unit out of its list",
            )),
        }
    }
}
