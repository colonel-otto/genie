//! An object's step and what follows it ([`World::step_object`]): what its Context gathered,
//! applied to the world in the order the game's update makes them.

use crate::GenieError;
use crate::objects::lifecycle::seconds;
use crate::objects::unit::{Step, Unit};
use crate::unit_ai::AfterOrder;
use crate::unit_ai::unit::AiResume;
use crate::unit_ai::world_view::{AiStart, AiWork};
use crate::world::context::{Context, ObjectOutputs};
use crate::world::{LEAVES_ITS_LIST, PlayerFrame, World};

/// What an object's step left for its list's loop ([`World::step_object`]).
pub(super) struct Stepped {
    /// The step.
    pub(super) step: Step,
    /// The resume of a unit AI part the update stopped before.
    pub(super) resume: Option<AiResume>,
    /// The tail after an order on a target.
    pub(super) after: Option<AfterOrder>,
    /// The object's slot in its owner's live list after the step: a removal that put the list's
    /// last object in another's slot (`FUN_1411664a0`) may have moved it.
    pub(super) slot: usize,
}

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

impl World {
    /// An object's step ([`World::run_object`], `f`: the unit's update, or one part of its unit
    /// AI for a command) and what follows it: what its Context gathered, applied in the order the
    /// game's update makes them (the regeneration, passive abilities and production of a full
    /// update, the unit AI's works, moves, waits and order on
    /// a target, the own kill, the takes and releases). Returns the step, the resume of a unit AI
    /// part the update stopped before, the tail after an order on a target and the object's
    /// slot, which it looks up by id after each part that may move it.
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
        f: impl FnOnce(&mut Unit, &mut Context<'_>) -> Result<Step, GenieError>,
    ) -> Result<Stepped, GenieError> {
        let id = self.players[p].objects[i].id;
        let (step, mut out) = self.run_object((p, i), (dt, turn), frame, resume, f);
        let step = step?;
        // the tail after an order on a target, which stops the unit AI's rest
        let after_order = out.ordered.as_ref().and_then(|&(_, tail)| tail);
        let own_update = full && out.ai_resume.is_none();
        let at = (p, self.slot_of(p, id)?, id);
        self.finish_own_update(at, (dt, own_update), &out, after_order, &step)?;
        self.apply_ai_works((p, self.slot_of(p, id)?), &mut out, &step)?;
        self.apply_ai_starts((p, self.slot_of(p, id)?), &mut out, &step)?;
        self.apply_hits_and_releases(&mut out)?;
        Ok(Stepped {
            step,
            resume: out.ai_resume,
            after: after_order,
            slot: self.slot_of(p, id)?,
        })
    }

    /// The slot of object `id` in player `p`'s live list, after its step.
    ///
    /// # Errors
    ///
    /// The object left the list in its own step.
    fn slot_of(&self, p: usize, id: i32) -> Result<usize, GenieError> {
        self.players[p].objects.position_of(id).ok_or_else(|| {
            GenieError::unreachable("a stepped object left its list in its step").with("object", id)
        })
    }

    /// The parts of the object's own update after its actions, once, with the update's unit AI
    /// part (`own_update`): the regeneration, the passive abilities with the hit point powerup
    /// check, and a building's production. Then the Transport's unload.
    fn finish_own_update(
        &mut self,
        (p, i, id): (usize, usize, i32),
        (dt, own_update): (u32, bool),
        out: &ObjectOutputs,
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
            let _ = self.abilities((id, p, i), owner, leaves)?;
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

    /// The unit AI's starts after its works: the waits and repairs, a Gather's build command and
    /// reseed, and the order on a target.
    fn apply_ai_starts(
        &mut self,
        (p, i): (usize, usize),
        out: &mut ObjectOutputs,
        step: &Step,
    ) -> Result<(), GenieError> {
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

    /// What the hits' write-backs left, in the order the hits made it (the blast's objects, the
    /// unit itself for a retaliation, its target): the kills and razings; then an entered unit's
    /// leave of its group, the takes, the builders' leaves and the releases, a construction
    /// start's clearing and the changed tiles.
    fn apply_hits_and_releases(&mut self, out: &mut ObjectOutputs) -> Result<(), GenieError> {
        for k in std::mem::take(&mut out.kills) {
            self.kill_left(&k)?;
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
