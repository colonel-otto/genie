//! A unit's update on its owner's turn, action by action.

use crate::GenieError;
use crate::combat::victims::victim_of;
use crate::map::spatial::Spatial;
use crate::math::rng::Stream;
use crate::movement::move_to::MoveTo;
use crate::movement::{Me, Mover};
use crate::objects::lifecycle::Class;
use crate::objects::lifecycle::death_parts::forget_resource;
use crate::objects::masters::Master;
use crate::objects::unit::{Step, Unit, loses_actions};
use crate::ownership::artifact::{Artifact, Handover};
use crate::ownership::capture::Captured;
use crate::unit_ai::task_finder::world::wait_progress;
use crate::unit_ai::unit::AiResume;
use crate::unit_ai::{ACTION_COMPLETED, TARGET_MOVED, UnitAi};
use crate::world::SPEED_EPSILON;
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
        // RGE_Combat_Object::update (0x14107dfb0): the attack timer runs down by dt times
        // `+0x87c` (a terrain look's rate, [`crate::combat::abilities`])
        if 0.0 < self.attack_timer {
            self.attack_timer -= c.dt_seconds * self.abilities.attack_rate;
            if self.attack_timer < 0.0 {
                self.attack_timer = 0.0;
            }
        }
        // then a reload world time past ends the volley under way (`+0xa04`) and is none
        if self.reload_worldtime != 0 && self.reload_worldtime < c.world_time {
            self.next_volley = 0;
            self.reload_worldtime = 0;
        }
        // RGE_Moving_Object::update counts the last human order time down before its velocity
        // step
        if self.mover.is_some() {
            self.count_down_human_order(c.dt, c.spatial);
        }
        if let Some(mover) = self.mover.as_mut() {
            let at = mover.step_velocity(
                self.position,
                c.dt_seconds,
                c.spatial.map,
                c.spatial.terrain.as_ref(),
                c.world_time,
            );
            if at != self.position {
                self.position = at;
                c.spatial.set_position(self.id, at);
                if let Some(p) = c.spatial.get(self.id) {
                    c.pathing.moved(self.id, p, mover.is_moving());
                }
            }
            // RGE_Moving_Object::update: a unit that moves animates at its speed (+0x7b8)
            if let Some(v) = mover.velocity.as_ref() {
                let [x, y, z] = v.velocity;
                let speed = crate::math::fmath::sqrt(x * x + y * y + z * z);
                if SPEED_EPSILON < speed {
                    self.anim_speed = Some(speed);
                }
            }
        }
        if self.life.is_alive() && self.life.hp < 1.0 {
            // set_state(3)'s storage (FUN_1411a56c0 with 0): each of kinds 2 and 16 of an
            // attribute leaves the player (its slot 0xf8 with the amount negated)
            if let Some(m) = c.masters.and_then(|m| m.get(self.life.master_id)) {
                for &(attribute, amount, kind) in &m.storage {
                    if matches!(kind, 2 | 16) && 0 <= attribute {
                        c.attribute_adds.push((attribute, -amount));
                    }
                }
            }
        }
        let before = self.life.state;
        if before == 4 && self.sprite.is_none() {
            return Err(GenieError::unsupported(
                "a dying object whose sprite the slice does not hold",
            ));
        }
        // the sprite list's slot 0x20 on the current sprite (FUN_1413b38a0)
        let animating = self
            .sprite
            .is_some_and(|s| s.animate != 0 && s.graphic.flags & 1 != 0);
        let leaves = if self
            .life
            .update(c.dt_seconds, c.streams.get(Stream::Object), animating)?
        {
            Step::Leave
        } else {
            Step::Stay
        };
        // the set_states of the steps (FUN_14119d710): a death goes through state 3
        let after = self.life.state;
        if before <= 2 && 3 <= after {
            self.died(c, before)?;
            // the static set_state's ghosts (FUN_14118e990), the caller's
            if c.masters
                .and_then(|m| m.get(self.life.master_id))
                .is_some_and(|m| m.ghost_on_death == crate::vision::ghost::LEAVES_GHOSTS)
            {
                c.ghost_deaths.push(self.id);
            }
        }
        // a building's set_states after its death's 3 take its annexes along too
        // (FUN_1411dbb70): the animated update's 4 or 5 from 3, the static update's 7 and 8; the
        // dying sprite's end (4 to 5) writes the state without a set_state
        if 3 < after
            && after != before
            && !(before == 4 && after == 5)
            && c.masters
                .and_then(|m| m.get(self.life.master_id))
                .is_some_and(|m| m.kind == 80)
        {
            for annex in c.spatial.annexes_of(self.id) {
                c.annex_states.push((annex, after));
            }
        }
        if after != before {
            if before <= 2 && 3 <= after {
                c.spatial.sight_state(self.id, 3);
            }
            if after != 3 {
                c.spatial.sight_state(self.id, after);
            }
            if before < 4 && 6 < after {
                let master = c.masters.and_then(|m| m.get(self.life.master_id));
                forget_resource(c.spatial, master, self.id, self.position);
            }
        }
        // the dying sprite ended (RGE_Animated_Object::update): the animation stops and an object
        // below type 80 leaves its remains (FUN_1411a4e90)
        if before == 4 && after == 5 {
            self.anim_speed = Some(0.0);
            let kind = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .map(|m| m.kind)
                .ok_or(GenieError::unsupported(
                    "a dead object of a master the slice does not have",
                ))?;
            if kind < 80 && 0 <= self.life.remains {
                // its facet (`+0xe`) as its angle left it ([`crate::objects::facing::facet_of_angle`])
                let facet = self
                    .mover
                    .as_ref()
                    .map(|m| crate::objects::facing::facet_of_angle(m.angle, m.stats.facets).0);
                c.remains = Some((self.life.remains, self.position, facet));
            }
        }
        // a dead object that holds nothing goes to state 7 and its death spawn comes (slot 0x160,
        // FUN_1411a5090: the master's `death_spawn_obj_id` makes an object at its place; a moving
        // object's override gives it the dead object's facet and that facet's angle); the
        // spawn's sprite from slot 0x7a8 (the master's undead sprite) is not held
        if matches!(before, 3 | 5) && after == 7 && self.life.death_spawn {
            let spawn = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .map(|m| m.death_spawns[0])
                .ok_or(GenieError::unsupported(
                    "a dead object of a master the slice does not have",
                ))?;
            let facet = self
                .mover
                .as_ref()
                .map(|m| crate::objects::facing::facet_of_angle(m.angle, m.stats.facets).0);
            c.remains = Some((spawn, self.position, facet));
        }
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
        // RGE_Action_Object::update with no action in state 2 starts the master's default task
        // (slot 0x7f0): a bird's Fly, whose first update comes in the next tick
        let mut flies_from_now = false;
        if self.life.state == 2
            && self.top_action_kind().is_none()
            && self.production.is_none()
            && let Some(task) = self.default_fly
        {
            self.start_fly(c, task)?;
            flies_from_now = true;
        }
        let mut makes_from_now = false;
        if self.life.state == 2
            && self.top_action_kind().is_none()
            && self.production.is_none()
            && std::mem::take(&mut self.default_make)
        {
            let task = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .and_then(Master::default_task)
                .copied()
                .ok_or(GenieError::unsupported(
                    "a Make whose master the slice does not have",
                ))?;
            self.make = Some(crate::gathering::make::MakeAction::start(
                task,
                c.attributes,
                &mut crate::gathering::make::Held {
                    attribute_type: &mut self.life.attribute_type,
                    amount: &mut self.life.amount,
                },
            ));
            makes_from_now = true;
        }
        let mut artifact_from_now = false;
        if self.life.state == 2
            && self.top_action_kind().is_none()
            && self.production.is_none()
            && self.default_artifact
        {
            self.artifact = Some(Artifact::new(c.player == 0));
            artifact_from_now = true;
        }
        if self.life.state == 2
            && self.top_action_kind().is_none()
            && self.production.is_none()
            && let Some(e) = self.default_task.clone()
        {
            return Err(e);
        }
        // The default action exists from tick 1 on, but its first update is in tick 2: fitted to
        // 16 checkpoints of idle players with livestock (workspace FD-008).
        let me = Me {
            id: self.id,
            position: self.position,
            group: self.group.id,
            action_state: self.top_action().map_or(0, |a| a.state),
        };
        let mut captured = None;
        let by = self.releaser(c);
        // RGE_Action_Object::update runs the actions of an object not inside another (`+0x2f0`
        // none as it reaches them), else the inside update, never both
        let inside_at_start = self.life.inside.is_some();
        // the Artifact's Enter sub-action (an enter command's, FUN_1410847e0) updates first
        // (FUN_140a992b0) as the unit's own Enter does (taken as the unit's meanwhile); once it
        // ended (1 to 5: the failed one's 3, the list removing it) the sub-actions go, the point
        // is -1 and the target goes (slot 0x68 with none); an Enter that got the unit in stays,
        // and the inside update of the next update deletes the Artifact
        if c.tick > 1
            && !inside_at_start
            && let Some(e) = self.artifact.as_mut().and_then(|a| a.enter.take())
        {
            let own = self.enter.replace(e);
            let updated = self.enter_update(c, others, me);
            let after = std::mem::replace(&mut self.enter, own);
            updated?;
            if let Some(art) = self.artifact.as_mut() {
                if let Some(e) = after {
                    art.enter = Some(e);
                } else {
                    art.goal = None;
                    if let Some(t) = art.drop_target() {
                        c.releases.push((t, by));
                    }
                }
            }
        }
        if let Some(artifact) = self
            .artifact
            .as_mut()
            .filter(|_| c.tick > 1 && !artifact_from_now && !inside_at_start)
        {
            let dt = c.dt;
            let ended = artifact.walk(Some(c.walk(self.mover.as_mut(), me)))?;
            // an ended MoveTo (codes 1 to 5) goes (FUN_140a98620): its teardown stops the unit,
            // quiet or not; the action lets its target go
            if ended.is_some()
                && let Some(m) = self.mover.as_mut()
            {
                m.stop()?;
            }
            if let Some(t) = ended.and_then(|e| e.released) {
                c.releases.push((t, by));
            }
            // FUN_14107cc90: an object of the combat types whose +0x924 is 0 never changes hands
            let capture = c
                .spatial
                .sight
                .as_ref()
                .and_then(|v| v.seer(self.id))
                .map(|s| s.capture);
            let searches = !(self.combat && capture == Some(0));
            let spatial: &Spatial = c.spatial;
            let (attribute_97, game_status) = (c.attribute_97, c.game_status);
            let id = self.id;
            let new = artifact.step(dt, searches, &mut || {
                spatial.artifact_owner(id, attribute_97, game_status)
            })?;
            // the check's sub-actions went: an Enter's teardown lets its building go, and a
            // walking MoveTo's, the Artifact's own or the Enter's, stops the unit
            if let Some(Handover::Taken { gone, .. }) = new {
                if let Some(t) = gone.released {
                    c.releases.push((t, by));
                }
                if gone.stops
                    && let Some(m) = self.mover.as_mut()
                {
                    m.stop()?;
                }
            }
            if let Some(kind) = ended.and_then(|e| e.notify) {
                self.post_own(kind, [crate::unit_ai::MOVE_ACTION, 0, 0])?;
            }
            match new {
                Some(Handover::Taken { new, .. }) => {
                    let (main, also) = self.capture(c, others, new)?;
                    captured = Some(Captured {
                        new,
                        main,
                        also,
                        building: std::mem::take(&mut c.building_handover),
                    });
                }
                // state 100: the unit's stop (slot 0x148), then slot 0x70 to Gaia
                Some(Handover::ToGaia) => {
                    let class = self.placed_class(c)?;
                    self.capture_stop(c, class)?;
                    let main = self.change_owner(c, 0)?;
                    let building = std::mem::take(&mut c.building_handover);
                    if main || building {
                        captured = Some(Captured {
                            new: 0,
                            main,
                            also: Vec::new(),
                            building,
                        });
                    }
                }
                None => {}
            }
        }
        self.build_part(c, others, me)?;
        if self.repair.is_some() {
            self.repair_part(c, others, me)?;
        }
        // RGE_Action_Object::update: the actions of an object inside another do not update; its
        // inside update (FUN_140a98790) deletes its top action (FUN_140a78480 -> FUN_140a784a0:
        // a dead target let go, the sub-actions' end; then the action's teardown, which lets its
        // targets go, slot 0x180, and a MoveTo's stops the unit, slot 0x5d0); the actions below
        // it (FUN_140a78470) and the next one's slot 0x28 need a second action, which the slice
        // never holds; a unit its actions got in this update waits for the next. An empty list
        // takes the default task instead (its branch of the three): livestock inside makes its
        // Artifact in one update and loses it in the next
        let default_made = flies_from_now || makes_from_now || artifact_from_now;
        if inside_at_start && !default_made {
            if let Some(e) = self.enter.take() {
                if e.move_to.is_some()
                    && let Some(m) = self.mover.as_mut()
                {
                    m.stop()?;
                }
                c.releases.push((e.target, self.releaser(c)));
            } else if self.top_action_kind() == Some(crate::ownership::artifact::ARTIFACT_TYPE) {
                // the Artifact goes (FUN_140a73fc0): its target is let go, then its sub-actions
                // go (an Enter's teardown lets its building go; a walking MoveTo's stops the
                // unit)
                let mut art = self.artifact.take().expect("its top action");
                let by = self.releaser(c);
                let (released, gone) = art.tear_down();
                for t in [released, gone.released].into_iter().flatten() {
                    c.releases.push((t, by));
                }
                if gone.stops
                    && let Some(m) = self.mover.as_mut()
                {
                    m.stop()?;
                }
            } else if self.unmodelled_action.is_some() {
                return Err(GenieError::unsupported(
                    "the inside update of an action the slice does not hold",
                ));
            } else if self.top_action_kind().is_some() {
                // any other action's destructor, as the list's deletion runs it
                let by = self.releaser(c);
                self.clear_own_actions(&mut c.releases, &mut c.builder_leaves, by)?;
            }
        } else if self.enter.is_some() {
            self.enter_update(c, others, me)?;
        } else if self.transport.is_some() {
            self.transport_update(c, me)?;
        }
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
        let step = match self.move_to.as_mut() {
            Some(move_to) => {
                // its state 3 reads the unit's AI for the path it asks (0x140aa2a70), with its
                // point: the goal, or the target's place (a target gone fails the update first)
                let point = match (move_to.target, move_to.goal) {
                    (-1, goal) => goal.map(|p| [p[0], p[1]]),
                    (id, _) => c.spatial.get(id).map(|t| [t.position[0], t.position[1]]),
                };
                let ask = match point {
                    Some(p) if move_to.state == MoveTo::Start => {
                        let at = [me.position[0], me.position[1]];
                        UnitAi::path_ask(self.ai.as_mut(), at, p)
                    }
                    _ => crate::movement::PathAsk::default(),
                };
                move_to.update(c.walk(self.mover.as_mut(), me), ask)?
            }
            None => crate::movement::move_to::MoveStep::default(),
        };
        if step.gave_up {
            let villager = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .is_some_and(|m| m.class == 4);
            let stance = self.ai.as_ref().map_or(0, |ai| ai.stance);
            self.own_move_gave_up(
                villager && !c.computer,
                stance,
                (c.masters, c.graphics, c.streams.get(Stream::Sprite)),
            )?;
        }
        if step.target_moved {
            self.post_own(TARGET_MOVED, [0, 0, 0])?;
        }
        if step.done || step.failed {
            // its update in state 1 posts its completion (0x1fa with the move action value; an
            // own MoveTo is never quiet), in state 13 its failure (0x1f9) and returns 3; the
            // action list removes it (FUN_140a98e20): its teardown (0x140aa1f30) stops the unit
            // (slot 0x5d0)
            let notify = if step.failed {
                crate::unit_ai::ACTION_FAILED
            } else {
                ACTION_COMPLETED
            };
            self.post_own(notify, [crate::unit_ai::MOVE_ACTION, 0, 0])?;
            // ... and the base teardown (FUN_140a73fc0) lets its target go (slot 0x180)
            if let Some(t) = self.move_to.take().map(|m| m.target).filter(|&t| t != -1) {
                c.releases.push((t, self.releaser(c)));
            }
            if let Some(m) = self.mover.as_mut() {
                m.stop()?;
            }
        }
        if let Some(explore) = self.explore.as_mut() {
            // 0x140a880c0: state 1 is done (the action list removes it, the default task's
            // restart is not modelled), 11 walks, 13 fails; the others wait
            match explore.state {
                crate::movement::explore::DONE => {
                    return Err(GenieError::unsupported(
                        "a done Explore (the action list after it)",
                    ));
                }
                crate::movement::explore::FAILED => {
                    return Err(GenieError::unsupported(
                        "a failed Explore (notify 0x1f9, set_state 2)",
                    ));
                }
                crate::movement::explore::WALK => {
                    // the MoveTo's update code (FUN_140a992b0): 1 once it arrived, 1 without it
                    let quiet = self
                        .ai
                        .as_ref()
                        .is_some_and(|ai| !matches!(ai.current_order, 711 | 733));
                    // the motion step giving up: a human player's villager waits once, any
                    // other unit's MoveTo fails (LAB_140aa320c)
                    let villager = c
                        .masters
                        .and_then(|m| m.get(self.life.master_id))
                        .is_some_and(|m| m.class == 4);
                    let waits = villager && !c.computer;
                    let stance = self.ai.as_ref().map_or(0, |ai| ai.stance);
                    let code = match explore.move_to {
                        Some(MoveTo::Start) => {
                            if explore.move_wait.state == 1 {
                                explore.move_wait.state = 2;
                            }
                            explore.move_to = Some(MoveTo::Moving);
                            let goal = crate::movement::move_to::Goal::Point(explore.point);
                            let walk = c.walk(self.mover.as_mut(), me);
                            explore.move_to = Some(
                                match crate::movement::move_to::start_walk_or_give_up(
                                    walk, goal, 0.0, quiet,
                                )? {
                                    Some(state) => state,
                                    None => explore.move_wait.give_up(waits, stance),
                                },
                            );
                            0
                        }
                        Some(MoveTo::Moving) => {
                            let walk = c.walk(self.mover.as_mut(), me);
                            explore.move_to =
                                Some(match crate::movement::move_to::walk_or_give_up(walk)? {
                                    Some(state) => state,
                                    None => explore.move_wait.give_up(waits, stance),
                                });
                            0
                        }
                        // case 2: the wait runs by dt, then the MoveTo asks again (set_state 3)
                        Some(MoveTo::Waiting) => {
                            explore.move_wait.timer += c.dt_seconds;
                            if explore.move_wait.delay <= explore.move_wait.timer {
                                explore.move_wait.timer = 0.0;
                                explore.move_to = Some(MoveTo::Start);
                            }
                            0
                        }
                        // its update in state 13 returns 3 (quiet: no notify)
                        Some(MoveTo::Failed) => 3,
                        Some(MoveTo::Arrived) | None => 1,
                    };
                    let walked = explore.move_to.is_some();
                    let owner = explore.after_walk(code)?;
                    if walked && self.explore.is_some_and(|e| e.move_to.is_none()) {
                        self.stop_mover()?;
                    }
                    self.explore_owner(owner, c)?;
                }
                _ => {}
            }
        }
        if let Some(fly) = self.fly.as_mut().filter(|_| !flies_from_now) {
            let mover = self
                .mover
                .as_mut()
                .ok_or(GenieError::unsupported("a bird without movement state"))?;
            let (speed, radius) = (
                mover.stats.speed,
                [mover.stats.radius[0], mover.stats.radius[1]],
            );
            let map = c.spatial.map;
            let (walk, action) = c.walk_with(Some(&mut *mover), me, Stream::Action);
            let sky = crate::movement::fly::Sky {
                speed,
                position: me.position,
                radius,
                map,
                action,
            };
            let step = fly.update(walk, sky)?;
            if step.stopped {
                mover.stop()?;
            }
            // a failed flight's MoveTo posts its failure (notify 0x1f9, the move action value)
            if step.failed {
                self.post_own(
                    crate::unit_ai::ACTION_FAILED,
                    [crate::unit_ai::MOVE_ACTION, 0, 0],
                )?;
            }
        }
        if let Some(make) = self.make.as_mut().filter(|_| !makes_from_now) {
            let master = c.masters.and_then(|m| m.get(self.life.master_id)).ok_or(
                GenieError::unsupported("a Make of a master the slice does not have"),
            )?;
            let farm = crate::gathering::make::Farm {
                master_id: master.id,
                kind: master.kind,
                class: master.class,
                work_rate: master.work_rate + self.work_rate_powerup,
                capacity: master.capacity,
                dt: c.dt_seconds,
            };
            let step = make.update(
                &mut crate::gathering::make::Held {
                    attribute_type: &mut self.life.attribute_type,
                    amount: &mut self.life.amount,
                },
                farm,
            );
            if let Some(add) = step.to_player {
                c.attribute_adds.push(add);
            }
            // slot 0xd8 (FUN_14118f580): no hit points left; the object is in its list, so not
            // asleep, and the special-object list (+0x30c bit 0) is not held
            if step.exhausted {
                self.life.hp = 0.0;
            }
            // the action list's result 1 removes it (FUN_140a98e20)
            if step.done {
                self.make = None;
            }
        }
        if self.gather.is_some() {
            self.gather_update(c, others, me)?;
        }
        self.hunt_targets(c, others)?;
        // the hunter as a combat object, which its hits read and change
        let mut own = (self.hunt.is_some() || self.attack.is_some())
            .then_some(())
            .and_then(|()| {
                let master = c.masters?.get(self.life.master_id)?;
                let terrain = c.spatial.terrain.as_ref();
                Some(victim_of(
                    self,
                    master,
                    c.attributes,
                    c.player,
                    terrain,
                    c.mode_680,
                ))
            });
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
