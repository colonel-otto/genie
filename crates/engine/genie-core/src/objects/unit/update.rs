//! A unit's update on its owner's turn, action by action.

use crate::GenieError;
use crate::math::rng::Stream;
use crate::movement::{Me, Mover};
use crate::objects::lifecycle::Class;
use crate::objects::unit::{Step, Unit, loses_actions};
use crate::unit_ai::task_finder::world::wait_progress;
use crate::unit_ai::unit::AiResume;
use crate::world::context::Context;

impl Unit {
    pub(crate) fn update(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<Step, GenieError> {
        if let Some(resume) = c.ai_resume.take() {
            return self.ai_part(c, others, resume);
        }
        let Some(leaves) = self.update_to_actions(c, others)? else {
            return Ok(Step::Sleep);
        };
        let captured = self.update_actions(c, others)?;
        let resume = AiResume {
            captured,
            leaves,
            after: None,
        };
        // a builder's work found in its Build's update (slot 0x7e8 from case 0xdf) runs inside the
        // action update; the caller gives it (`World::order_work`) and then runs the rest of this
        // update ([`Unit::ai_part`])
        if c.work_order.is_some() || !c.completions.is_empty() || c.transport_unload {
            c.ai_resume = Some(resume);
            return Ok(Step::Stay);
        }
        self.ai_part(c, others, resume)
    }

    /// `RGE_Static_Object::update` and the combat object's steps before the actions: the death
    /// blast, the sleep ([`Unit::go_to_sleep`], None), the sprite's facets, a gate's and a
    /// building's update, the attack timers, the motion, the life step (what
    /// the unit AI later reads of it), the animation and the actions past the unit's life.
    pub(crate) fn update_to_actions(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<Option<Step>, GenieError> {
        self.death_blast(c)?;
        if self.go_to_sleep()? {
            return Ok(None);
        }
        self.take_sprite_facets();
        self.gate_update(c, others)?;
        self.complete_on_update(c, others)?;
        self.building_seconds(c, others)?;
        self.run_down_attack_timers(c);
        self.motion_step(c)?;
        let leaves = self.life_step(c)?;
        self.animate(c);
        self.actions_past_life(c)?;
        Ok(Some(leaves))
    }

    /// `RGE_Action_Object::update`: the default task's start, then each action held in the
    /// game's order (Artifact, Build, Repair, garrison, the wait for a foundation, a missile's
    /// flight, `MoveTo`, Explore, Fly, Make, Gather, Hunt, Attack) and what follows them; the
    /// Artifact's capture, which the unit AI's part reads.
    pub(crate) fn update_actions(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<Option<crate::ownership::capture::Captured>, GenieError> {
        let started = self.start_default_task(c)?;
        // The default action exists from tick 1 on, but its first update is in tick 2: fitted to
        // 16 checkpoints of idle players with livestock (workspace FD-008).
        let me = Me {
            id: self.id,
            position: self.position,
            group: self.group.id,
            action_state: self.top_action().map_or(0, |a| a.state),
        };
        // RGE_Action_Object::update runs the actions of an object not inside another (`+0x2f0`
        // none as it reaches them), else the inside update, never both
        let inside_at_start = self.life.inside.is_some();
        let captured = self.artifact_part(c, others, me, inside_at_start, started.artifact)?;
        self.build_part(c, others, me)?;
        if self.actions.repair().is_some() {
            self.repair_part(c, others, me)?;
        }
        self.garrison_part(c, others, me, inside_at_start, started.any())?;
        // the wait for a foundation (FUN_140ae8b70) on its build points, none when the world
        // does not hold it; its 1 removes it (FUN_140a98e20: the base teardown lets no target go,
        // and no action follows it, the wait having replaced the list)
        if let Some(w) = self.actions.wait_solidify().copied()
            && w.update(wait_progress(w.target, others, c.foreign))
        {
            self.actions.set_wait_solidify(None);
        }
        if self.actions.missile().is_some() {
            self.fly(c, others)?;
        }
        self.move_to_part(c, me)?;
        self.explore_part(c, me)?;
        self.fly_part(c, me, started.fly)?;
        self.make_part(c, started.make)?;
        if self.actions.gather().is_some() {
            self.gather_update(c, others, me)?;
        }
        self.hunt_targets(c, others)?;
        // the hunter as a combat object, which its hits read and change
        let mut own = self.own_victim(c);
        let tree_lost = self.tree_lost();
        if !self.hunt_part(c, others, me, &mut own, tree_lost)? {
            self.attack_part(c, me, own)?;
        }
        self.after_actions(c)?;
        Ok(captured)
    }

    /// `RGE_Static_Object::update` (`0x14118ad50`): an object flagged to sleep (`+0x643`) drops
    /// the flag, is marked asleep (`+0x641`) and moves to its player's sleeping list (player
    /// slots 0x2d8 and 0x2d0), and its update ends (true); only static objects (type 10) are
    /// flagged, so one with movement state or a unit AI is not modelled.
    pub(crate) fn go_to_sleep(&mut self) -> Result<bool, GenieError> {
        if !self.life.goto_sleep {
            return Ok(false);
        }
        if self.life.class != Class::Static || self.body.mover().is_some() || self.ai.is_some() {
            return Err(GenieError::unsupported(
                "an object above the static type goes to sleep",
            ));
        }
        self.life.goto_sleep = false;
        Ok(true)
    }

    /// The yaw and the facing read the current sprite's facets: the movement state takes them.
    pub(crate) const fn take_sprite_facets(&mut self) {
        if let (Some(m), Some(s)) = (self.body.mover_mut(), self.sprite) {
            m.sprite_facets = Some(crate::objects::facing::SpriteFacets {
                count: s.graphic.facets,
                flag_1: s.graphic.flags & 2 != 0,
            });
        }
    }

    /// The animated object's update (`0x141071880`): its sprite's clock at its animation speed,
    /// then its idle swap, which reads the loop this update's clock made.
    pub(crate) fn animate(&mut self, c: &mut Context<'_>) {
        if let (Some(sprite), Some(speed)) = (self.sprite.as_mut(), self.body.anim_speed()) {
            sprite.update(c.dt_seconds, speed);
            if let Some(master) = c.masters.and_then(|m| m.get(self.life.master_id)) {
                self.idle_swap(master, c.graphics, c.streams.get(Stream::Sprite));
            }
        }
    }

    /// `RGE_Action_Object::update` (`0x14106ac90`) after the moving update: an object past state
    /// 2 (the Wonder past state 5) loses its action list (`FUN_140a98620`). An alive object with
    /// an action the slice does not model fails here.
    pub(crate) fn actions_past_life(&mut self, c: &mut Context<'_>) -> Result<(), GenieError> {
        if let Some(e) = self
            .unmodelled_action
            .clone()
            .filter(|_| self.life.state == 2)
        {
            return Err(e);
        }
        if loses_actions(&self.life) {
            let by = self.releaser(c);
            self.clear_own_actions(&mut c.releases, &mut c.builder_leaves, by)?;
        }
        Ok(())
    }

    /// After the actions: the unit takes its place as the world has it (the motion step may have
    /// put it onto a waypoint it reached) and hands the pauses it gave to the Context. A unit
    /// whose walk met others in its way is not modelled.
    pub(crate) fn after_actions(&mut self, c: &mut Context<'_>) -> Result<(), GenieError> {
        if let Some(p) = c.spatial.get(self.id) {
            self.position = p.position;
        }
        if let Some(m) = self.body.mover_mut() {
            c.pauses.append(&mut m.pauses);
            if !m.in_the_way.is_empty() {
                return Err(crate::world::IN_THE_WAY);
            }
        }
        Ok(())
    }

    /// [`Unit::ai_part`]'s first step: the pathing grid takes a unit of a master of kind 40 or
    /// more where it stands, moving or not.
    pub(crate) fn refresh_pathing(&self, c: &mut Context<'_>) {
        if let Some(m) = c
            .masters
            .and_then(|ms| ms.get(self.life.master_id))
            .filter(|m| 40 <= m.kind)
            && let Some(p) = c.spatial.get(self.id).copied()
        {
            let moving = self.body.mover().is_some_and(Mover::is_moving);
            c.pathing.refresh(self.id, &p, moving, m.outline);
        }
    }
}
