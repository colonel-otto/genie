//! One object's step with its context ([`World::run_object`]) and a unit AI run outside its
//! update.

use crate::GenieError;
use crate::construction::building_update::blasts_at_death;
use crate::objects::lifecycle::seconds;
use crate::objects::unit::{Step, Unit};
use crate::unit_ai::unit::AiResume;
use crate::world::context::{Context, ObjectOutputs};
use crate::world::{PlayerFrame, World, scenario_mode, standard_mode};

impl World {
    /// One object's step with its Context (`f`: the unit's update, or one part of its unit AI
    /// for a command): object `i` of player `p` leaves its list (its stand-in holds the slot),
    /// and `f` gets it and the Context, which holds the world (FD-045-D28; the objects it
    /// makes join the list at their making). It goes back to the slot its stand-in holds then
    /// ([`World::put_back_own`]). Returns `f`'s result (an error of the put-back, then of the
    /// sight's registry first) and what the Context gathered for the world
    /// ([`World::step_object`]).
    #[allow(clippy::too_many_lines)]
    pub(super) fn run_object(
        &mut self,
        (p, i): (usize, usize),
        (dt, turn): (u32, Option<u32>),
        frame: &PlayerFrame,
        resume: Option<AiResume>,
        f: impl FnOnce(&mut Unit, &mut Context<'_>) -> Result<Step, GenieError>,
    ) -> (Result<Step, GenieError>, ObjectOutputs) {
        // the unit AI's target as its update starts, read before the unit leaves its list: a unit
        // may be its own target (an unload's)
        let ai_target = self.players[p].objects[i]
            .ai
            .as_ref()
            .map(|ai| ai.current_target)
            .filter(|&t| t != -1)
            .map_or(Ok(None), |t| self.seen(t, p));
        let mut unit = self.players[p].objects.take_out(i);
        let watch = crate::trace::Watch::of(&unit);
        // the objects a hit's blast may reach about its target (one a hit can read), or about
        // the unit for its blast at death (TRIBE_Combat_Object::update)
        let target = unit
            .attack_target()
            .filter(|&t| self.victim(t, p).is_some());
        let blasted = match target {
            Some(t) => self.blasted(&unit, p, t),
            None if self
                .masters
                .get(p)
                .and_then(|m| m.get(unit.life.master_id))
                .is_some_and(|m| blasts_at_death(m, &unit.life)) =>
            {
                self.blasted(&unit, p, -1)
            }
            None => Vec::new(),
        };
        let player = &self.players[p];
        let (computer, construction_rate, civ) =
            (player.computer, player.construction_rate, player.civ);
        let (ai_settings, sharing, resources) =
            (player.ai_settings, player.sharing, player.resources);
        let settings = &self.settings;
        let (farm_stops, predator_hunts_end) =
            (settings.walkable_farms, settings.predator_hunts_end());
        let (carry_rule, game_mode) = (settings.carry_rule(), settings.game_mode);
        let (tick, world_time, mode_680, player_turn) =
            (self.ticks, self.world_time, self.mode_680, self.player_turn);
        let me = unit.id;
        let mut c = Context {
            w: self,
            me,
            dt,
            dt_seconds: seconds(dt),
            tick,
            turn,
            world_time,
            computer,
            construction_rate,
            ai_target,
            blasted,
            kills: Vec::new(),
            remains: None,
            refund: Vec::new(),
            building_death: None,
            releases: Vec::new(),
            takes: Vec::new(),
            builder_leaves: Vec::new(),
            left_group: None,
            ghost_deaths: Vec::new(),
            started: Vec::new(),
            unwatched: Vec::new(),
            mode_1: mode_680 == 1,
            player: p,
            civ,
            ai_settings,
            sharing,
            farm_stops,
            predator_hunts_end,
            resources,
            attribute_97: &frame.attribute_97,
            game_status: &frame.game_status,
            player_turn,
            computers: &frame.computers,
            dodge_chances: &frame.dodge_chances,
            keep_distance_chances: &frame.keep_distance_chances,
            attribute_204: frame.attribute_204,
            mode_680,
            carry_rule,
            scenario: scenario_mode(game_mode),
            standard: standard_mode(game_mode),
            sight_takers: &frame.sight_takers,
            attribute_adds: Vec::new(),
            history_events: Vec::new(),
            techs_done: Vec::new(),
            pastures: Vec::new(),
            carts: Vec::new(),
            completions: Vec::new(),
            work_order: None,
            transport_unload: false,
            gate_toggle: false,
            ai_resume: resume,
            taken_over: None,
            building_handover: false,
            order_on_target: None,
            works: Vec::new(),
            build_command: None,
            reseed: None,
            move_aside: Vec::new(),
            transport_moves: Vec::new(),
            starts: Vec::new(),
            leads: None,
        };
        let mut result = f(&mut unit, &mut c);
        if let Some(w) = watch {
            w.report(&unit, c.tick, p);
        }
        let out = c.outputs(me);
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
        (result, out)
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
        ) -> Result<Vec<crate::unit_ai::Effect>, GenieError>,
    ) -> Result<(), GenieError> {
        let frame = self.player_frame();
        let stepped = self.step_object((p, i), (0, None, false), &frame, None, |unit, c| {
            unit.run_ai(c, f).map(|()| Step::Stay)
        })?;
        match stepped.step {
            Step::Stay => Ok(()),
            _ => Err(GenieError::unsupported(
                "a unit AI run outside its update that takes the unit out of its list",
            )),
        }
    }
}
