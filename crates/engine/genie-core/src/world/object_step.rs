//! One object's step with its context ([`World::run_object`]) and a unit AI run outside its
//! update.

use crate::GenieError;
use crate::combat::damage::Victim;
use crate::construction::building_update::blasts_at_death;
use crate::objects::lifecycle::seconds;
use crate::objects::unit::{Step, Unit};
use crate::unit_ai::unit::AiResume;
use crate::world::context::{Context, ObjectOutputs};
use crate::world::others::Others;
use crate::world::{PlayerFrame, World, scenario_mode, standard_mode};

impl World {
    /// One object's step with its Context (`f`: the unit's update, or one part of its unit AI
    /// for a command): object `i` of player `p` leaves its list, `f` gets it, the Context and the
    /// rest of the list; it comes back with the objects it made (each filed in its owner's
    /// tracker). Returns `f`'s result (an error of the sight's registry first), what the Context
    /// gathered for the world ([`World::step_object`]) and the victim of its attack.
    #[allow(clippy::too_many_lines)]
    pub(super) fn run_object(
        &mut self,
        (p, i): (usize, usize),
        (dt, turn): (u32, Option<u32>),
        frame: &PlayerFrame,
        resume: Option<AiResume>,
        f: impl FnOnce(&mut Unit, &mut Context<'_>, &mut Others<'_>) -> Result<Step, GenieError>,
    ) -> (Result<Step, GenieError>, ObjectOutputs, Option<Victim>) {
        // the unit AI's target as its update starts, read before the unit leaves its list: a unit
        // may be its own target (an unload's)
        let ai_target = self.players[p].objects[i]
            .ai
            .as_ref()
            .map(|ai| ai.current_target)
            .filter(|&t| t != -1)
            .map_or(Ok(None), |t| self.seen(t, p));
        let mut unit = std::mem::replace(&mut self.players[p].objects[i], Unit::taken_out());
        let watch = crate::trace::Watch::of(&unit);
        let mut victim = unit.attack_target().and_then(|t| self.victim(t, p));
        // the objects a hit's blast may reach about its target, or about the unit for
        // its blast at death (TRIBE_Combat_Object::update)
        let blasted = match victim.as_ref() {
            Some(v) => self.blasted(&unit, p, v.id),
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
        let (before, rest) = self.players.split_at_mut(p);
        let (player, after) = rest.split_first_mut().expect("the player updating");

        let mut c = Context {
            dt,
            dt_seconds: seconds(dt),
            tick: self.ticks,
            turn,
            world_time: self.world_time,
            computer: player.computer,
            streams: &mut self.streams,
            spatial: &mut self.spatial,
            pathing: &mut self.pathing,
            groups: &self.groups,
            construction_rate: player.construction_rate,
            masters: self.masters.get(p),
            victim: victim.as_mut(),
            ai_target,
            blasted,
            own_killed: None,
            remains: None,
            refund: Vec::new(),
            building_death: None,
            pauses: Vec::new(),
            releases: Vec::new(),
            takes: Vec::new(),
            builder_leaves: Vec::new(),
            left_group: None,
            made: Vec::new(),
            ghost_deaths: Vec::new(),
            started: Vec::new(),
            unwatched: Vec::new(),
            annex_states: Vec::new(),
            released_annexes: Vec::new(),
            cleared: Vec::new(),
            site_removals: Vec::new(),
            flares: Vec::new(),
            promotions: Vec::new(),
            evacuations: Vec::new(),
            recycling: &mut self.recycling,
            mode_1: self.mode_680 == 1,
            player: p,
            civ: player.civ,
            ai_settings: player.ai_settings,
            sharing: player.sharing,
            farm_stops: self.settings.walkable_farms,
            predator_hunts_end: self.settings.predator_hunts_end(),
            attributes: &player.attributes,
            resources: player.resources,
            graphics: &self.graphics,
            drop_sites: &self.drop_sites,
            every_masters: &self.masters,
            attribute_97: &frame.attribute_97,
            game_status: &frame.game_status,
            player_turn: self.player_turn,
            player_time: &self.player_time,
            computers: &frame.computers,
            dodge_chances: &frame.dodge_chances,
            keep_distance_chances: &frame.keep_distance_chances,
            attribute_204: frame.attribute_204,
            mode_680: self.mode_680,
            carry_rule: self.settings.carry_rule(),
            scenario: scenario_mode(self.settings.game_mode),
            standard: standard_mode(self.settings.game_mode),
            sight_takers: &frame.sight_takers,
            tracker: &mut player.tracker,
            lines: &self.lines,
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
            entered: None,
            starts: Vec::new(),
            main_works: Vec::new(),
            relations: &player.relations,
            leads: None,
            zones: &mut self.zones,
        };
        let lists = (&mut player.objects, &mut player.sleeping);
        let mut others = Others::new(p, lists, (before, after));
        let mut result = f(&mut unit, &mut c, &mut others);
        if let Some(w) = watch {
            w.report(&unit, c.tick, p);
        }
        let mut out = c.outputs(unit.id);
        let made = std::mem::take(&mut out.made);
        if let Some(e) = self.spatial.take_sight_unsupported()
            && result.is_ok()
        {
            result = Err(e);
        }
        self.spatial.set_gait(unit.id, unit.gait());
        player.objects[i] = unit;
        // each made object's add to its player (slot 0x2d0) filed it in the tracker
        for m in &made {
            let k =
                crate::players::tracker::key(self.masters.get(p), &self.lines, m.life.master_id);
            player.tracker.add(m.id, k);
        }
        player.objects.extend(made);
        (result, out, victim)
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
        let (step, ..) =
            self.step_object((p, i), (0, None, false), &frame, None, |unit, c, others| {
                unit.run_ai(c, others.view(), f).map(|()| Step::Stay)
            })?;
        match step {
            Step::Stay => Ok(()),
            _ => Err(GenieError::unsupported(
                "a unit AI run outside its update that takes the unit out of its list",
            )),
        }
    }
}
