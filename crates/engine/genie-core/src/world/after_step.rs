//! An object's step and what follows it ([`World::step_object`]): what its `Step` gathered,
//! applied to the world in the order the game's update makes them.

use crate::GenieError;
use crate::objects::lifecycle::seconds;
use crate::objects::unit::{Outcome, Unit};
use crate::unit_ai::AfterOrder;
use crate::unit_ai::unit::AiResume;
use crate::unit_ai::world_view::AiStart;
use crate::world::step::{ObjectOutputs, Step};
use crate::world::{LEAVES_ITS_LIST, World};

/// What an object's step left for its list's loop ([`World::step_object`]).
pub(super) struct Stepped {
    /// The step.
    pub(super) outcome: Outcome,
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
    outcome: &Outcome,
    leaves: GenieError,
    effect: impl FnOnce() -> Result<T, GenieError>,
) -> Result<(), GenieError> {
    if !matches!(outcome, Outcome::Stay) {
        return Err(leaves);
    }
    effect().map(drop)
}

impl World {
    /// An object's step ([`World::run_object`], `f`: the unit's update, or one part of its unit
    /// AI for a command) and what follows it: what its `Step` gathered, applied in the order the
    /// game's update makes them (the regeneration, passive abilities and production of a full
    /// update, a builder's work order, the unit AI's waits and order on a target, the own kill).
    /// Returns the step, the resume of a unit AI
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
        resume: Option<AiResume>,
        f: impl FnOnce(&mut Unit, &mut Step<'_>) -> Result<Outcome, GenieError>,
    ) -> Result<Stepped, GenieError> {
        let id = self.players[p].objects[i].id;
        let (outcome, mut out) = self.run_object((p, i), (dt, turn), resume, f);
        let outcome = outcome?;
        // the tail after an order on a target, which stops the unit AI's rest
        let after_order = out.ordered.as_ref().and_then(|&(_, tail)| tail);
        let own_update = full && out.ai_resume.is_none();
        let at = (p, self.slot_of(p, id)?, id);
        self.finish_own_update(at, (dt, own_update), after_order, &outcome)?;
        self.apply_work_order((p, self.slot_of(p, id)?), &mut out, &outcome)?;
        self.apply_ai_starts((p, self.slot_of(p, id)?), &mut out, &outcome)?;
        self.apply_hits_and_releases(&mut out)?;
        Ok(Stepped {
            outcome,
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
            GenieError::unreachable("a stepped object left its list in its outcome")
                .with("object", id)
        })
    }

    /// The parts of the object's own update after its actions, once, with the update's unit AI
    /// part (`own_update`): the regeneration, the passive abilities with the hit point powerup
    /// check, and a building's production.
    fn finish_own_update(
        &mut self,
        (p, i, id): (usize, usize, i32),
        (dt, own_update): (u32, bool),
        after_order: Option<AfterOrder>,
        outcome: &Outcome,
    ) -> Result<(), GenieError> {
        // TRIBE_Combat_Object::update's regeneration after RGE_Combat_Object::update
        if own_update {
            self.regenerate_object((p, i, id), seconds(dt));
        }
        // TRIBE_Combat_Object::update's passive abilities (FUN_1412072f0) as the object's owner
        // after its update; a removal reward's command comes back as the recording's own
        if own_update {
            let (owner, leaves) = match outcome {
                Outcome::Captured(k) if k.main => (k.new, false),
                Outcome::Leave => (p, true),
                _ => (p, false),
            };
            let _ = self.abilities((id, p, i), owner, leaves)?;
            // then its hit point powerup check
            self.clamp_object_powerup((p, i, id));
        }
        // TRIBE_Building_Object::update: the production after the combat update (after the
        // unit AI's rest, when an order on a target stopped it)
        if matches!(outcome, Outcome::Stay) && own_update && after_order.is_none() {
            self.produce(p, i, seconds(dt))?;
        }
        Ok(())
    }

    /// A builder's work found in its Build's update (slot 0x7e8 from case 0xdf, [`Step`]'s
    /// `work_order`), given before the update's unit AI part resumes.
    fn apply_work_order(
        &mut self,
        (p, i): (usize, usize),
        out: &mut ObjectOutputs,
        outcome: &Outcome,
    ) -> Result<(), GenieError> {
        if let Some(work) = out.work_order.take() {
            in_list(
                outcome,
                GenieError::unsupported("a builder's work order of a unit that leaves its list"),
                || self.order_work(p, i, &work),
            )?;
        }
        Ok(())
    }

    /// The unit AI's starts: the waits and repairs, a Gather's build command and reseed, and the
    /// order on a target.
    fn apply_ai_starts(
        &mut self,
        (p, i): (usize, usize),
        out: &mut ObjectOutputs,
        outcome: &Outcome,
    ) -> Result<(), GenieError> {
        // the waits for foundations (slot 0x388) and the repairs (slot 0x378) the unit AI's
        // orders gave
        for start in std::mem::take(&mut out.starts) {
            // a unit that ran its orders stays in its list: the full update runs them
            // only for a unit alive and not inside another, between turns only for a
            // unit of a type below 70 or outside, and nothing after the queue in the
            // unit's update kills, captures or houses it
            debug_assert!(matches!(outcome, Outcome::Stay));
            if !matches!(outcome, Outcome::Stay) {
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
            in_list(outcome, LEAVES_ITS_LIST, || {
                self.build_command(p, i, foundation)
            })?;
        }
        // a Gather's reseed after its walk back (state 0xf2): a build command too
        if let Some(arrived) = out.reseed {
            in_list(outcome, LEAVES_ITS_LIST, || {
                self.reseed_after_walk(p, i, arrived)
            })?;
        }
        if let Some((order, _)) = out.ordered {
            if !matches!(outcome, Outcome::Stay) {
                return Err(GenieError::unsupported(
                    "an order on a target of a unit that leaves its list",
                ));
            }
            self.order_on_target(p, i, order)?;
        }
        Ok(())
    }

    /// What the hits' write-backs left, in the order the hits made it (the blast's objects, the
    /// unit itself for a retaliation, its target): the kills and razings; then a construction
    /// start's clearing and the changed tiles.
    fn apply_hits_and_releases(&mut self, out: &mut ObjectOutputs) -> Result<(), GenieError> {
        for k in std::mem::take(&mut out.kills) {
            self.kill_left(&k)?;
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
