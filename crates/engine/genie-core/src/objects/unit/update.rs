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
        self.death_blast(c)?;
        // RGE_Static_Object::update (0x14118ad50): an object flagged to sleep (`+0x643`) drops
        // the flag, is marked asleep (`+0x641`) and moves to its player's sleeping list (player
        // slots 0x2d8 and 0x2d0), and its update ends; only static objects (type 10) are flagged
        if self.life.goto_sleep {
            if self.life.class != Class::Static || self.mover.is_some() || self.ai.is_some() {
                return Err(GenieError::unsupported(
                    "an object above the static type goes to sleep",
                ));
            }
            self.life.goto_sleep = false;
            return Ok(Step::Sleep);
        }
        // the yaw and the facing read the current sprite's facets
        if let (Some(m), Some(s)) = (self.mover.as_mut(), self.sprite) {
            m.sprite_facets = Some(crate::objects::facing::SpriteFacets {
                count: s.graphic.facets,
                flag_1: s.graphic.flags & 2 != 0,
            });
        }
        self.gate_update(c, others)?;
        self.complete_on_update(c, others)?;
        self.building_seconds(c, others)?;
        self.take_stopped_speed();
        self.run_down_attack_timers(c);
        self.motion_step(c);
        let leaves = self.life_step(c)?;
        // the animated object's update (0x141071880): its sprite's clock at its animation speed,
        // then its idle swap
        if let (Some(sprite), Some(speed)) = (self.sprite.as_mut(), self.anim_speed) {
            sprite.update(c.dt_seconds, speed);
            // the swap reads the loop this update's clock made
            if let Some(master) = c.masters.and_then(|m| m.get(self.life.master_id)) {
                self.idle_swap(master, c.graphics, c.streams.get(Stream::Sprite));
            }
        }
        if let Some(e) = self
            .unmodelled_action
            .clone()
            .filter(|_| self.life.state == 2)
        {
            return Err(e);
        }
        // RGE_Action_Object::update (0x14106ac90), after the moving update: an object past
        // state 2 (the Wonder past state 5) loses its action list (FUN_140a98620)
        if loses_actions(&self.life) {
            let by = self.releaser(c);
            self.clear_own_actions(&mut c.releases, &mut c.builder_leaves, by)?;
        }
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
        if self.repair.is_some() {
            self.repair_part(c, others, me)?;
        }
        self.garrison_part(c, others, me, inside_at_start, started.any())?;
        // the wait for a foundation (FUN_140ae8b70) on its build points, none when the world
        // does not hold it; its 1 removes it (FUN_140a98e20: the base teardown lets no target go,
        // and no action follows it, the wait having replaced the list)
        if let Some(w) = self.wait_solidify
            && w.update(wait_progress(w.target, others, c.foreign))
        {
            self.wait_solidify = None;
        }
        if self.missile.as_ref().is_some_and(|m| m.action.is_some()) {
            self.fly(c, others)?;
        }
        self.move_to_part(c, me)?;
        self.explore_part(c, me)?;
        self.fly_part(c, me, started.fly)?;
        self.make_part(c, started.make)?;
        if self.gather.is_some() {
            self.gather_update(c, others, me)?;
        }
        self.hunt_targets(c, others)?;
        // the hunter as a combat object, which its hits read and change
        let mut own = self.own_victim(c);
        let tree_lost = self.tree_lost();
        if !self.hunt_part(c, others, me, &mut own, tree_lost)? {
            self.attack_part(c, me, own)?;
        }
        // the motion step may have put the unit onto a waypoint it reached
        if let Some(p) = c.spatial.get(self.id) {
            self.position = p.position;
        }
        if let Some(m) = self.mover.as_mut() {
            c.pauses.append(&mut m.pauses);
            if !m.in_the_way.is_empty() {
                return Err(crate::world::IN_THE_WAY);
            }
        }
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

    /// [`Unit::ai_part`]'s first step: the pathing grid takes a unit of a master of kind 40 or
    /// more where it stands, moving or not.
    pub(crate) fn refresh_pathing(&self, c: &mut Context<'_>) {
        if let Some(m) = c
            .masters
            .and_then(|ms| ms.get(self.life.master_id))
            .filter(|m| 40 <= m.kind)
            && let Some(p) = c.spatial.get(self.id).copied()
        {
            let moving = self.mover.as_ref().is_some_and(Mover::is_moving);
            c.pathing.refresh(self.id, &p, moving, m.outline);
        }
    }
}
