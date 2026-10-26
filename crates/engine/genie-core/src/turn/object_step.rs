//! One object's step with its context ([`Turns::run_object`]), what follows it and a unit
//! AI run outside its update.

use crate::GenieError;
use crate::combat::damage::Victim;
use crate::lifecycle::seconds;
use crate::turn::unit::{AiResume, Step, Unit};
use crate::turn::{
    BuildingDeath, LEAVES_ITS_LIST, PlayerFrame, Turns, scenario_mode, standard_mode, unit,
};
use crate::unit_ai::world_view::{AiStart, AiWork, Context, Foreign, ObjectOutputs};

impl Turns {
    /// One object's step with its Context (`f`: the unit's update, or one part of its unit AI
    /// for a command): object `i` of player `p` leaves its list, `f` gets it, the Context and the
    /// rest of the list; it comes back with the objects it made (each filed in its owner's
    /// tracker). Returns `f`'s result (an error of the sight's registry first), what the Context
    /// gathered for the world ([`Turns::finish_object`]) and the victim of its attack.
    #[allow(clippy::too_many_lines)]
    pub(super) fn run_object<R>(
        &mut self,
        (p, i): (usize, usize),
        (dt, turn): (u32, Option<u32>),
        frame: &PlayerFrame,
        resume: Option<AiResume>,
        f: impl FnOnce(&mut Unit, &mut Context<'_>, &mut Vec<Unit>) -> Result<R, GenieError>,
    ) -> (Result<R, GenieError>, ObjectOutputs, Option<Victim>) {
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
                .is_some_and(|m| unit::blasts_at_death(m, &unit.life)) =>
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
            repair_writes: Vec::new(),
            relations: &player.relations,
            leads: None,
            zones: &mut self.zones,
            foreign: Foreign {
                before,
                after,
                left: None,
            },
        };
        let mut result = f(&mut unit, &mut c, &mut player.objects);
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

    /// What follows an object's step ([`Turns::run_object`]): what its Context gathered, applied
    /// in the order the game's update makes them (the gate, the annexes' and main buildings'
    /// work, the completions, the attributes, the deaths and remains, the regeneration, passive
    /// abilities and production of a full update, the unit AI's works, moves, waits and order on
    /// a target, the hit's write-backs, the takes and releases). Returns the result, the resume
    /// of a unit AI part the update stopped before, and the tail after an order on a target.
    /// Without `full` (a unit AI run outside the object's update) the parts that only its update
    /// runs (regeneration, passive abilities, production) do not.
    #[allow(clippy::too_many_lines)]
    pub(super) fn finish_object(
        &mut self,
        (p, i, id): (usize, usize, i32),
        (dt, full): (u32, bool),
        out: ObjectOutputs,
        mut victim: Option<Victim>,
        mut result: Result<Step, GenieError>,
    ) -> (
        Result<Step, GenieError>,
        Option<AiResume>,
        Option<crate::unit_ai::AfterOrder>,
    ) {
        let ObjectOutputs {
            adds,
            ordered,
            history_events,
            works,
            leads,
            build_command,
            reseed,
            move_aside,
            transport_moves,
            entered,
            starts,
            techs_done,
            pastures,
            carts,
            completions,
            work_order,
            transport_unload,
            mut blasted,
            own_killed,
            remains,
            refund,
            annex_states,
            cleared,
            site_removals,
            flares,
            promotions,
            released_annexes,
            main_works,
            building_death,
            pauses,
            gate_toggle,
            ai_resume,
            releases,
            builder_leaves,
            left_group,
            takes,
            made: _,
            ghost_deaths,
            started,
            unwatched,
            evacuations,
            repair_writes,
        } = out;
        // a repair's changes of another player's objects (the hit points, the repairers, the
        // armor back), in the order it made them
        for w in repair_writes {
            if let Some((_, u)) = self.object_mut(w.id) {
                if let Some((hp, powerups)) = w.hp {
                    u.life.hp = hp;
                    u.hp_powerups = powerups;
                }
                let n = u.repairers_mut();
                *n = n.wrapping_add_signed(w.repairers);
                if w.restored {
                    u.armor_removed = [0; 2];
                }
            }
        }
        // the tracker's promotions of the units whose master changed while their unit AI ran
        for (id, old, new) in promotions {
            self.track_promote(p, id, old, new);
        }
        // the flares a hit on a player's object made for that player
        for (q, at, doubled) in flares {
            if let Err(e) = self.make_flare(q, at, doubled) {
                result = result.and(Err(e));
            }
        }
        // the gate's player opens or shuts it, in the gate's update before the rest
        if gate_toggle && let Err(e) = self.toggle_gate(p, i) {
            result = result.and(Err(e));
        }
        // the work a builder gave an annex goes to its main building (FUN_1411cc430)
        for (annex, amount) in main_works {
            if let Err(e) = self.work_building(annex, amount) {
                result = result.and(Err(e));
            }
        }
        // a dying building's annexes, let go with its hit points (0x14118ad50)
        for (annex, hp) in released_annexes {
            if let Some((_, a)) = self.object_mut(annex) {
                a.annex_of = None;
                a.life.hp = hp;
            }
        }
        // a building's set_state took its annexes along first (FUN_1411dbb70)
        for (annex, state) in annex_states {
            if let Some((q, _)) = self.object(annex)
                && let Err(e) = self.set_state(q, annex, state)
            {
                result = result.and(Err(e));
            }
        }
        // the objects a builder's footprint clearing deleted (FUN_1415c3620)
        for cl in cleared {
            if let Err(e) = self.clear_object(cl, true) {
                result = result.and(Err(e));
            }
        }
        // the foundations a builder found another player's marks on: a human owner's goes
        // (FUN_1411ccbb0)
        for site in site_removals {
            if let Some((q, _)) = self.object(site)
                && !self.players[q].computer
                && let Err(e) = self.remove_building(q, site)
            {
                result = result.and(Err(e));
            }
        }
        // a completed castle's or wonder's history event, the owner's id as data1
        for kind in history_events {
            let now = self.world_time;
            self.players[p]
                .history
                .add_event(kind, p as f32, 0.0, 0, now);
        }
        for (kind, amount) in adds {
            if let Err(e) = self.players[p].add_attribute(kind, amount) {
                result = result.and(Err(e));
            }
        }
        // a completed pasture's livestock (FUN_1411c8410 in FUN_1411c6bd0, after the farm's
        // bonus)
        for pasture in pastures {
            if let Err(e) = self.setup_pasture(p, pasture) {
                result = result.and(Err(e));
            }
        }
        // a completed mule cart's search for resources (FUN_1411c6bd0's tail, before
        // the completion's technology)
        for (cart, state) in carts {
            if let Err(e) = self.cart_search(p, cart, state) {
                result = result.and(Err(e));
            }
        }
        // a completed building's stack unit before its technology (FUN_1411dbb70)
        for &(building, stack, _) in &completions {
            if let Some(stack) = stack
                && let Err(e) = self.stack_unit(p, building, stack)
            {
                result = result.and(Err(e));
            }
        }
        for t in techs_done {
            if let Err(e) = self.research_built(p, t) {
                result = result.and(Err(e));
            }
        }
        // then a building that disappears when built is destroyed (slot 0xd0)
        for (building, _, vanishes) in completions {
            if vanishes && let Err(e) = self.destroy(p, building) {
                result = result.and(Err(e));
            }
        }
        // slot 0x5f0 on each unit the walk paused: its walked level's pause
        for (id, pause) in pauses {
            self.pause(id, pause);
        }
        // a dead building leaves its owner's watched list (FUN_141091310), then the
        // ghosts of a death (FUN_14118e990, while the game state is not 2) and the
        // watch of a started construction (FUN_1411e5670)
        for id in unwatched {
            self.unwatch(p, id);
        }
        for (foundation, units) in evacuations {
            if let Err(e) = self.evacuate(foundation, &units) {
                result = result.and(Err(e));
            }
        }
        for id in ghost_deaths {
            if self.game_state != 2
                && let Err(e) = self.ghosts_at_death(p, id)
            {
                result = result.and(Err(e));
            }
        }
        for id in started {
            if let Some((owner, _)) = self.object(id)
                && let Err(e) = self.watch_started(owner, id)
            {
                result = result.and(Err(e));
            }
        }
        // a dead building's count leaves its attribute, at least 0 (FUN_1411dbb70)
        if let Some(BuildingDeath {
            count: Some(kind), ..
        }) = building_death
        {
            let owner = &mut self.players[p];
            if let Err(e) = owner.add_attribute(kind, -1.0) {
                result = result.and(Err(e));
            } else if owner.attributes[kind as usize] < 0.0 {
                owner.attributes[kind as usize] = 0.0;
            }
        }
        // a dead unit's paid cost leaves attributes 98 and 164 (a building's 98 and 165),
        // each at least 0 (FUN_14121cc40: slots 0xf8, then 0xf0 with 0)
        for (kind, cost) in refund {
            let owner = &mut self.players[p];
            if let Err(e) = owner.add_attribute(kind, -cost) {
                result = result.and(Err(e));
            } else if owner.attributes[kind as usize] < 0.0 {
                owner.attributes[kind as usize] = 0.0;
            }
        }
        // the last building of a master with a technology withdraws it (FUN_141166ab0,
        // FUN_1413d4ec0), after the combat part's state 3
        if let Some(BuildingDeath {
            tech: Some((master, tech)),
            ..
        }) = building_death
            && let Err(e) = self.withdraw_tech(p, master, tech)
        {
            result = result.and(Err(e));
        }
        // the remains of a dying sprite's end, a new object of the owner (FUN_1411a4e90)
        if let Some((master, at, facet)) = remains
            && let Err(e) = self.remains(p, master, at, facet)
        {
            result = result.and(Err(e));
        }
        // TRIBE_Combat_Object::update's regeneration after RGE_Combat_Object::update
        // (once, with the update's unit AI part)
        if full && ai_resume.is_none() && result.is_ok() {
            self.regenerate_object((p, i, id), seconds(dt));
        }
        // TRIBE_Combat_Object::update's passive abilities (FUN_1412072f0, once, with the
        // update's unit AI part) as the object's owner after its update; a removal
        // reward's command comes back as the recording's own
        if full
            && ai_resume.is_none()
            && let Ok(step) = &result
        {
            let (owner, leaves) = match step {
                Step::Captured(k) if k.main => (k.new, false),
                Step::Leave => (p, true),
                _ => (p, false),
            };
            let mut pending = crate::combat::abilities::Pending {
                victim: victim.as_mut(),
                blasted: &mut blasted,
            };
            if let Err(e) = self.abilities((id, p, i), owner, leaves, &mut pending) {
                result = Err(e);
            }
            // then its hit point powerup check
            self.clamp_object_powerup((p, i, id));
        }
        // TRIBE_Building_Object::update: the production after the combat update (once,
        // with the update's unit AI part)
        // (after the unit AI's rest, when an order on a target stopped it)
        let after_order = ordered.as_ref().and_then(|&(_, tail)| tail);
        if matches!(result, Ok(Step::Stay))
            && full
            && ai_resume.is_none()
            && after_order.is_none()
            && let Err(e) = self.produce(p, i, seconds(dt))
        {
            result = Err(e);
        }
        // the Transport's unload in its update, before the unit AI part
        if transport_unload {
            if !matches!(result, Ok(Step::Stay)) {
                result = result.and(Err(LEAVES_ITS_LIST));
            } else if let Err(e) = self.transport_unload(p, i) {
                result = Err(e);
            }
        }
        if let Some(work) = work_order {
            if !matches!(result, Ok(Step::Stay)) {
                result = result.and(Err(GenieError::unsupported(
                    "a builder's work order of a unit that leaves its list",
                )));
            } else if let Err(e) = self.order_work(p, i, &work) {
                result = Err(e);
            }
        }
        // the unit AI's HuntObjects: each the unit's stop, then its hunt command
        // a hunt's set_state(6) on its own livestock: the animal's slot 0x138 to the drop
        // site and the herd around it (FUN_140ac4c60)
        if let Some((sheep, site)) = leads
            && let Err(e) = self.lead_herd(p, sheep, site)
        {
            result = Err(e);
        }
        // the unit AI's HuntObjects (the unit's stop, then its hunt command),
        // GatherObjects (its gather command), BuildObjects (its build command) and works
        // (slot 0x7e8), in the order it gave them
        for work in works {
            if !matches!(result, Ok(Step::Stay)) {
                result = result.and(Err(GenieError::unsupported(
                    "a HuntObject or GatherObject of a unit that leaves its list",
                )));
                break;
            }
            let done = match work {
                AiWork::Hunt(target) => self.hunt_command(p, i, target),
                AiWork::Gather(target) => self.gather_command(p, i, target),
                AiWork::Work(work) => self.order_work(p, i, &work),
                AiWork::Build(foundation) => self.build_command(p, i, foundation).map(|_| ()),
                AiWork::DropElsewhere(site, back) => self.drop_elsewhere(p, i, site, back),
            };
            if let Err(e) = done {
                result = Err(e);
            }
        }
        // the unit AI's moves off footprints (slot 0x148)
        for m in move_aside {
            if !matches!(result, Ok(Step::Stay)) {
                result = result.and(Err(GenieError::unsupported(
                    "a move off a footprint of a unit that leaves its list",
                )));
            } else if let Err(e) = self.move_aside(p, i, m) {
                result = Err(e);
            }
        }
        // another player's transport or building the unit entered
        if let Some((target, firepower)) = entered
            && let Some((_, b)) = self.object_mut(target)
        {
            b.firepower += firepower;
            b.garrison.push(id);
        }
        // the transports an Enter sent to where they meet the unit (slot 0x340)
        for (ship, to) in transport_moves {
            if let Err(e) = self.ship_move(ship, to) {
                result = result.and(Err(e));
            }
        }
        // the waits for foundations (slot 0x388) and the repairs (slot 0x378) the unit AI's
        // orders gave
        for start in starts {
            // a unit that ran its orders stays in its list: the full update runs them
            // only for a unit alive and not inside another, between turns only for a
            // unit of a type below 70 or outside, and nothing after the queue in the
            // unit's update kills, captures or houses it
            debug_assert!(matches!(result, Ok(Step::Stay)) || result.is_err());
            if !matches!(result, Ok(Step::Stay)) {
                break;
            }
            let made = match start {
                AiStart::Wait(w) => self.wait_for_foundation(p, i, w),
                AiStart::Repair(r) => self.repair_order(p, i, r),
                AiStart::Enter(e) => self.enter_order(p, i, e),
                AiStart::Transport(to) => self.transport_order(p, i, to),
            };
            if let Err(e) = made {
                result = Err(e);
            }
        }
        // a Gather's build command on its target not built yet (slot 0x380)
        if let Some(foundation) = build_command {
            if !matches!(result, Ok(Step::Stay)) {
                result = result.and(Err(LEAVES_ITS_LIST));
            } else if let Err(e) = self.build_command(p, i, foundation) {
                result = Err(e);
            }
        }
        // a Gather's reseed after its walk back (state 0xf2): a build command too
        if let Some(arrived) = reseed {
            if !matches!(result, Ok(Step::Stay)) {
                result = result.and(Err(LEAVES_ITS_LIST));
            } else if let Err(e) = self.reseed_after_walk(p, i, arrived) {
                result = Err(e);
            }
        }
        if let (Ok(Step::Stay), Some((order, _))) = (&result, ordered) {
            if let Err(e) = self.order_on_target(p, i, order) {
                result = Err(e);
            }
        } else if ordered.is_some() && result.is_ok() {
            result = Err(GenieError::unsupported(
                "an order on a target of a unit that leaves its list",
            ));
        }
        // a hit's write-backs in the order the hit made them: the blast's objects, the
        // unit itself (a retaliation), its target
        for v in &blasted {
            if let Err(e) = self.set_victim(v) {
                result = result.and(Err(e));
            }
        }
        if let Some(kill) = own_killed
            && let Err(e) = self.killed(p, i, kill)
        {
            result = result.and(Err(e));
        }
        if let Some(v) = victim
            && let Err(e) = self.set_victim(&v)
        {
            result = result.and(Err(e));
        }
        // an entered unit's removeUnit from its group (FUN_14102c160), when the world
        // holds the group (FUN_14152ff40)
        if let Some((g, id, building)) = left_group.filter(|&(g, ..)| self.groups.get(g).is_some())
            && let Err(e) = self.remove_unit(g, id, building, false)
        {
            result = result.and(Err(e));
        }
        for (id, by) in takes {
            self.take(id, by);
        }
        for id in builder_leaves {
            if let Err(e) = self.leave_builders(id) {
                result = result.and(Err(e));
            }
        }
        for (id, by) in releases {
            if let Err(e) = self.release(id, by) {
                result = result.and(Err(e));
            }
        }
        // a construction start's clearing of the desolid foundations it overlaps
        // (FUN_1411cd0d0), after the builder's update
        for id in self.spatial.take_started() {
            if let Err(e) = self.clear_foundations_under(id) {
                result = result.and(Err(e));
            }
        }
        self.pathing.tiles_changed(self.spatial.take_changes());
        (result, ai_resume, after_order)
    }

    /// Runs `f` on the unit AI of object `i` of player `p` outside its update (a command's
    /// immediate order, `FUN_140c55c90`): the object's step ([`Turns::run_object`]) whose only
    /// part is the AI's run with what it reads of the world ([`Unit::run_ai`]), then what that
    /// gathered ([`Turns::finish_object`] without the update's own parts).
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
        let id = self.players[p].objects[i].id;
        let (result, out, victim) =
            self.run_object((p, i), (0, None), &frame, None, |unit, c, objects| {
                unit.run_ai(c, objects, f).map(|()| Step::Stay)
            });
        let (result, ..) = self.finish_object((p, i, id), (0, false), out, victim, result);
        match result? {
            Step::Stay => Ok(()),
            _ => Err(GenieError::unsupported(
                "a unit AI run outside its update that takes the unit out of its list",
            )),
        }
    }
}
