//! An object's step and what follows it ([`World::step_object`]): what its Context gathered,
//! applied to the world in the order the game's update makes them.

use crate::GenieError;
use crate::combat::damage::Victim;
use crate::objects::lifecycle::seconds;
use crate::objects::unit::{Step, Unit};
use crate::unit_ai::AfterOrder;
use crate::unit_ai::unit::AiResume;
use crate::unit_ai::world_view::{AiStart, AiWork};
use crate::world::context::{Context, ObjectOutputs};
use crate::world::{BuildingDeath, LEAVES_ITS_LIST, Player, PlayerFrame, World};

/// An effect for a unit that stays in its list: `leaves` when the step took the unit out.
fn in_list<T>(
    step: &Step,
    leaves: GenieError,
    effect: impl FnOnce() -> Result<T, GenieError>,
) -> Result<(), GenieError> {
    if !matches!(step, Step::Stay) {
        return Err(leaves);
    }
    effect().map(drop)
}

/// Takes `amount` of attribute `kind` from `owner`, at least 0 left (slot 0xf8, then slot 0xf0
/// with 0).
fn take_at_least_zero(owner: &mut Player, kind: i16, amount: f32) -> Result<(), GenieError> {
    owner.add_attribute(kind, -amount)?;
    if owner.attributes[kind as usize] < 0.0 {
        owner.attributes[kind as usize] = 0.0;
    }
    Ok(())
}

impl World {
    /// An object's step ([`World::run_object`], `f`: the unit's update, or one part of its unit
    /// AI for a command) and what follows it: what its Context gathered, applied in the order the
    /// game's update makes them (the gate, the annexes' and main buildings'
    /// work, the completions, the attributes, the deaths and remains, the regeneration, passive
    /// abilities and production of a full update, the unit AI's works, moves, waits and order on
    /// a target, the hit's write-backs, the takes and releases). Returns the step, the resume of a
    /// unit AI part the update stopped before, and the tail after an order on a target.
    /// Without `full` (a unit AI run outside the object's update) the parts that only its update
    /// runs (regeneration, passive abilities, production) do not.
    ///
    /// # Errors
    ///
    /// The step's, or the first of what follows it; the object's outputs after it do not apply.
    pub(super) fn step_object(
        &mut self,
        (p, i): (usize, usize),
        (dt, turn, full): (u32, Option<u32>, bool),
        frame: &PlayerFrame,
        resume: Option<AiResume>,
        f: impl FnOnce(
            &mut Unit,
            &mut Context<'_>,
            &mut crate::world::others::Others<'_>,
        ) -> Result<Step, GenieError>,
    ) -> Result<(Step, Option<AiResume>, Option<AfterOrder>), GenieError> {
        let id = self.players[p].objects[i].id;
        let (step, mut out, mut victim) = self.run_object((p, i), (dt, turn), frame, resume, f);
        let step = step?;
        // the tail after an order on a target, which stops the unit AI's rest
        let after_order = out.ordered.as_ref().and_then(|&(_, tail)| tail);
        self.apply_object_writes((p, i), &mut out)?;
        self.apply_completions(p, &mut out)?;
        self.apply_deaths(p, &mut out)?;
        let own_update = full && out.ai_resume.is_none();
        self.finish_own_update(
            (p, i, id),
            (dt, own_update),
            &mut out,
            &mut victim,
            after_order,
            &step,
        )?;
        self.apply_ai_works((p, i), &mut out, &step)?;
        self.apply_ai_starts((p, i), &mut out, &step)?;
        self.apply_hits_and_releases((p, i), &mut out, victim)?;
        Ok((step, out.ai_resume, after_order))
    }

    /// The step's writes on other objects and on its own building: a repair's writes, the
    /// tracker's promotions, the flares, the gate, the annexes' work for their main buildings, a
    /// dying building's annexes and their states, a builder's footprint clearing and foundation
    /// removals, and the history events.
    fn apply_object_writes(
        &mut self,
        (p, i): (usize, usize),
        out: &mut ObjectOutputs,
    ) -> Result<(), GenieError> {
        // the tracker's promotions of the units whose master changed while their unit AI ran
        for (id, old, new) in std::mem::take(&mut out.promotions) {
            self.track_promote(p, id, old, new);
        }
        // the flares a hit on a player's object made for that player
        for (q, at, doubled) in std::mem::take(&mut out.flares) {
            self.make_flare(q, at, doubled)?;
        }
        // the gate's player opens or shuts it, in the gate's update before the rest
        if out.gate_toggle {
            self.toggle_gate(p, i)?;
        }
        // the work a builder gave an annex goes to its main building (FUN_1411cc430)
        for (annex, amount) in std::mem::take(&mut out.main_works) {
            self.work_building(annex, amount)?;
        }
        // a building's set_state took its annexes along first (FUN_1411dbb70)
        for (annex, state) in std::mem::take(&mut out.annex_states) {
            if let Some((q, _)) = self.object(annex) {
                self.set_state(q, annex, state)?;
            }
        }
        // the objects a builder's footprint clearing deleted (FUN_1415c3620)
        for cl in std::mem::take(&mut out.cleared) {
            self.clear_object(cl, true)?;
        }
        // the foundations a builder found another player's marks on: a human owner's goes
        // (FUN_1411ccbb0)
        for site in std::mem::take(&mut out.site_removals) {
            if let Some((q, _)) = self.object(site)
                && !self.players[q].computer
            {
                self.remove_building(q, site)?;
            }
        }
        // a completed castle's or wonder's history event, the owner's id as data1
        for kind in std::mem::take(&mut out.history_events) {
            let now = self.world_time;
            self.players[p]
                .history
                .add_event(kind, p as f32, 0.0, 0, now);
        }
        Ok(())
    }

    /// The owner's attribute changes and what a completion sets off: a pasture's livestock, a
    /// mule cart's search, a building's stack unit, its technology, and a building that
    /// disappears when built.
    fn apply_completions(&mut self, p: usize, out: &mut ObjectOutputs) -> Result<(), GenieError> {
        for (kind, amount) in std::mem::take(&mut out.adds) {
            self.players[p].add_attribute(kind, amount)?;
        }
        // a completed pasture's livestock (FUN_1411c8410 in FUN_1411c6bd0, after the farm's
        // bonus)
        for pasture in std::mem::take(&mut out.pastures) {
            self.setup_pasture(p, pasture)?;
        }
        // a completed mule cart's search for resources (FUN_1411c6bd0's tail, before
        // the completion's technology)
        for (cart, state) in std::mem::take(&mut out.carts) {
            self.cart_search(p, cart, state)?;
        }
        // a completed building's stack unit before its technology (FUN_1411dbb70)
        let completions = std::mem::take(&mut out.completions);
        for &(building, stack, _) in &completions {
            if let Some(stack) = stack {
                self.stack_unit(p, building, stack)?;
            }
        }
        for t in std::mem::take(&mut out.techs_done) {
            self.research_built(p, t)?;
        }
        // then a building that disappears when built is destroyed (slot 0xd0)
        for (building, _, vanishes) in completions {
            if vanishes {
                self.destroy(p, building)?;
            }
        }
        Ok(())
    }

    /// What the step's deaths and construction starts set off: the watched lists, the evacuations, the ghosts, a dead building's count and technology, a
    /// dead object's refund, and the remains of a dying sprite's end.
    fn apply_deaths(&mut self, p: usize, out: &mut ObjectOutputs) -> Result<(), GenieError> {
        // a dead building leaves its owner's watched list (FUN_141091310), then the
        // ghosts of a death (FUN_14118e990, while the game state is not 2) and the
        // watch of a started construction (FUN_1411e5670)
        for id in std::mem::take(&mut out.unwatched) {
            self.unwatch(p, id);
        }
        for (foundation, units) in std::mem::take(&mut out.evacuations) {
            self.evacuate(foundation, &units)?;
        }
        for id in std::mem::take(&mut out.ghost_deaths) {
            if self.game_state != 2 {
                self.ghosts_at_death(p, id)?;
            }
        }
        for id in std::mem::take(&mut out.started) {
            if let Some((owner, _)) = self.object(id) {
                self.watch_started(owner, id)?;
            }
        }
        // a dead building's count leaves its attribute, at least 0 (FUN_1411dbb70)
        if let Some(BuildingDeath {
            count: Some(kind), ..
        }) = out.building_death
        {
            take_at_least_zero(&mut self.players[p], kind, 1.0)?;
        }
        // a dead unit's paid cost leaves attributes 98 and 164 (a building's 98 and 165),
        // each at least 0 (FUN_14121cc40: slots 0xf8, then 0xf0 with 0)
        for (kind, cost) in std::mem::take(&mut out.refund) {
            take_at_least_zero(&mut self.players[p], kind, cost)?;
        }
        // the last building of a master with a technology withdraws it (FUN_141166ab0,
        // FUN_1413d4ec0), after the combat part's state 3
        if let Some(BuildingDeath {
            tech: Some((master, tech)),
            ..
        }) = out.building_death
        {
            self.withdraw_tech(p, master, tech)?;
        }
        // the remains of a dying sprite's end, a new object of the owner (FUN_1411a4e90)
        if let Some((master, at, facet)) = out.remains {
            self.remains(p, master, at, facet)?;
        }
        Ok(())
    }

    /// The parts of the object's own update after its actions, once, with the update's unit AI
    /// part (`own_update`): the regeneration, the passive abilities with the hit point powerup
    /// check, and a building's production. Then the Transport's unload.
    fn finish_own_update(
        &mut self,
        (p, i, id): (usize, usize, i32),
        (dt, own_update): (u32, bool),
        out: &mut ObjectOutputs,
        victim: &mut Option<Victim>,
        after_order: Option<AfterOrder>,
        step: &Step,
    ) -> Result<(), GenieError> {
        // TRIBE_Combat_Object::update's regeneration after RGE_Combat_Object::update
        if own_update {
            self.regenerate_object((p, i, id), seconds(dt));
        }
        // TRIBE_Combat_Object::update's passive abilities (FUN_1412072f0) as the object's owner
        // after its update; a removal reward's command comes back as the recording's own
        if own_update {
            let (owner, leaves) = match step {
                Step::Captured(k) if k.main => (k.new, false),
                Step::Leave => (p, true),
                _ => (p, false),
            };
            let mut pending = crate::combat::abilities::Pending {
                victim: victim.as_mut(),
                blasted: &mut out.blasted,
            };
            let _ = self.abilities((id, p, i), owner, leaves, &mut pending)?;
            // then its hit point powerup check
            self.clamp_object_powerup((p, i, id));
        }
        // TRIBE_Building_Object::update: the production after the combat update (after the
        // unit AI's rest, when an order on a target stopped it)
        if matches!(step, Step::Stay) && own_update && after_order.is_none() {
            self.produce(p, i, seconds(dt))?;
        }
        // the Transport's unload in its update, before the unit AI part
        if out.transport_unload {
            in_list(step, LEAVES_ITS_LIST, || self.transport_unload(p, i))?;
        }
        Ok(())
    }

    /// The unit AI's works in the order it gave them: a builder's work, a hunt's lead of its
    /// livestock, the hunts, gathers, builds and works, and the moves off footprints.
    fn apply_ai_works(
        &mut self,
        (p, i): (usize, usize),
        out: &mut ObjectOutputs,
        step: &Step,
    ) -> Result<(), GenieError> {
        if let Some(work) = out.work_order.take() {
            in_list(
                step,
                GenieError::unsupported("a builder's work order of a unit that leaves its list"),
                || self.order_work(p, i, &work),
            )?;
        }
        // a hunt's set_state(6) on its own livestock: the animal's slot 0x138 to the drop site
        // and the herd around it (FUN_140ac4c60)
        if let Some((sheep, site)) = out.leads {
            self.lead_herd(p, sheep, site)?;
        }
        // the unit AI's HuntObjects (the unit's stop, then its hunt command),
        // GatherObjects (its gather command), BuildObjects (its build command) and works
        // (slot 0x7e8), in the order it gave them
        for work in std::mem::take(&mut out.works) {
            if !matches!(step, Step::Stay) {
                return Err(GenieError::unsupported(
                    "a HuntObject or GatherObject of a unit that leaves its list",
                ));
            }
            match work {
                AiWork::Hunt(target) => self.hunt_command(p, i, target),
                AiWork::Gather(target) => self.gather_command(p, i, target),
                AiWork::Work(work) => self.order_work(p, i, &work),
                AiWork::Build(foundation) => self.build_command(p, i, foundation).map(drop),
                AiWork::DropElsewhere(site, back) => self.drop_elsewhere(p, i, site, back),
            }?;
        }
        // the unit AI's moves off footprints (slot 0x148)
        for m in std::mem::take(&mut out.move_aside) {
            in_list(
                step,
                GenieError::unsupported("a move off a footprint of a unit that leaves its list"),
                || self.move_aside(p, i, m),
            )?;
        }
        Ok(())
    }

    /// The unit AI's entries and starts after its works: the transports' meeting points, the waits and repairs, a Gather's build command and reseed,
    /// and the order on a target.
    fn apply_ai_starts(
        &mut self,
        (p, i): (usize, usize),
        out: &mut ObjectOutputs,
        step: &Step,
    ) -> Result<(), GenieError> {
        // the transports an Enter sent to where they meet the unit (slot 0x340)
        for (ship, to) in std::mem::take(&mut out.transport_moves) {
            self.ship_move(ship, to)?;
        }
        // the waits for foundations (slot 0x388) and the repairs (slot 0x378) the unit AI's
        // orders gave
        for start in std::mem::take(&mut out.starts) {
            // a unit that ran its orders stays in its list: the full update runs them
            // only for a unit alive and not inside another, between turns only for a
            // unit of a type below 70 or outside, and nothing after the queue in the
            // unit's update kills, captures or houses it
            debug_assert!(matches!(step, Step::Stay));
            if !matches!(step, Step::Stay) {
                break;
            }
            match start {
                AiStart::Wait(w) => self.wait_for_foundation(p, i, w),
                AiStart::Repair(r) => self.repair_order(p, i, r),
                AiStart::Enter(e) => self.enter_order(p, i, e),
                AiStart::Transport(to) => self.transport_order(p, i, to),
            }?;
        }
        // a Gather's build command on its target not built yet (slot 0x380)
        if let Some(foundation) = out.build_command {
            in_list(step, LEAVES_ITS_LIST, || {
                self.build_command(p, i, foundation)
            })?;
        }
        // a Gather's reseed after its walk back (state 0xf2): a build command too
        if let Some(arrived) = out.reseed {
            in_list(step, LEAVES_ITS_LIST, || {
                self.reseed_after_walk(p, i, arrived)
            })?;
        }
        if let Some((order, _)) = out.ordered {
            if !matches!(step, Step::Stay) {
                return Err(GenieError::unsupported(
                    "an order on a target of a unit that leaves its list",
                ));
            }
            self.order_on_target(p, i, order)?;
        }
        Ok(())
    }

    /// A hit's write-backs in the order the hit made them (the blast's objects, the unit itself
    /// for a retaliation, its target), an entered unit's leave of its group, the takes, the
    /// builders' leaves and the releases, then a construction start's clearing and the changed
    /// tiles.
    fn apply_hits_and_releases(
        &mut self,
        (p, i): (usize, usize),
        out: &mut ObjectOutputs,
        victim: Option<Victim>,
    ) -> Result<(), GenieError> {
        for v in &out.blasted {
            self.set_victim(v)?;
        }
        if let Some(kill) = out.own_killed.take() {
            self.killed(p, i, kill)?;
        }
        if let Some(v) = victim {
            self.set_victim(&v)?;
        }
        // an entered unit's removeUnit from its group (FUN_14102c160), when the world
        // holds the group (FUN_14152ff40)
        if let Some((g, id, building)) = out
            .left_group
            .filter(|&(g, ..)| self.groups.get(g).is_some())
        {
            let _ = self.remove_unit(g, id, building, false)?;
        }
        for (id, by) in std::mem::take(&mut out.takes) {
            self.take(id, by);
        }
        for id in std::mem::take(&mut out.builder_leaves) {
            self.leave_builders(id)?;
        }
        for (id, by) in std::mem::take(&mut out.releases) {
            self.release(id, by)?;
        }
        // a construction start's clearing of the desolid foundations it overlaps
        // (FUN_1411cd0d0), after the builder's update
        for id in self.spatial.take_started() {
            self.clear_foundations_under(id)?;
        }
        self.pathing.tiles_changed(self.spatial.take_changes());
        Ok(())
    }
}
