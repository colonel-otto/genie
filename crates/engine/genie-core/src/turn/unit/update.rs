//! A unit's update on its owner's turn, action by action.

use crate::combat::attack::Strike;
use crate::construction::build::{Builder, Completion, Site};
use crate::gathering::hunt::{Gatherer, HuntSprites};
use crate::lifecycle::Class;
use crate::masters::Master;
use crate::math::rng::Stream;
use crate::move_to::MoveTo;
use crate::movement::{Me, Mover, footprint_gap_squared};
use crate::ownership::artifact::{Artifact, Handover};
use crate::spatial::Spatial;
use crate::tasks::Actor;
use crate::turn::unit::{
    AiResume, Aimed, Captured, Step, Unit, armed, give_load, hunt_walk_sprite, loses_actions,
    second_target_checked,
};
use crate::turn::view::Context;
use crate::turn::world::{
    clear_look_of, farm_bonus, forget_resource, nearest_drop_site, release_own, state_of,
    victim_of, wait_progress,
};
use crate::turn::{
    BUILDING_KIND, DROP_SITE_MASTERS, SEARCH_AGAIN, SEARCH_NEEDED, SPEED_EPSILON, TREE_NEAR,
    TREE_SEARCH_WITHIN,
};
use crate::unit_ai::{
    ACTION_COMPLETED, ACTION_EXECUTED, BUILD_ACTION, BUILD_ORDER, GenieError, TARGET_MOVED, UnitAi,
};

impl Unit {
    pub(in crate::turn) fn update(
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
            m.sprite_facets = Some(crate::facing::SpriteFacets {
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
                // its facet (`+0xe`) as its angle left it ([`crate::facing::facet_of_angle`])
                let facet = self
                    .mover
                    .as_ref()
                    .map(|m| crate::facing::facet_of_angle(m.angle, m.stats.facets).0);
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
                .map(|m| crate::facing::facet_of_angle(m.angle, m.stats.facets).0);
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
        // what the footprint clearing of the Build's set_state(7) reads (FUN_1415c3620) of
        // the objects on the tiles about a desolid foundation, as the update starts
        let mut looks = std::collections::BTreeMap::new();
        if let Some(f) = self
            .build
            .filter(|b| matches!(b.state, 6 | 222))
            .and_then(|b| others.iter().find(|u| u.id == b.target))
            .filter(|u| u.construction.is_some_and(|k| k.desolid))
        {
            let [fx, fy, _] = f.position;
            let reach = crate::construction::build::CLEARING_REACH + 1.0;
            for (id, _) in c
                .spatial
                .objects_within(fx - reach, fy - reach, fx + reach, fy + reach)
            {
                let (owner, u) = if id == self.id {
                    (c.player, &*self)
                } else if let Some(u) = others.iter().find(|u| u.id == id) {
                    (c.player, u)
                } else if let Some((q, u)) = c.foreign.find(id) {
                    (q, u)
                } else {
                    continue;
                };
                if let Some(l) = clear_look_of(u, c.every_masters.get(owner)) {
                    let _ = looks.insert(id, l);
                }
            }
        }
        let by = self.releaser(c);
        if let Some(build) = self.build.as_mut() {
            let target = build.target;
            let annexed = others.iter().any(|u| u.annex_of == Some(target));
            let masters = c.masters;
            let (attribute_204, mode_680, computer) = (c.attribute_204, c.mode_680, c.computer);
            let world_time = c.world_time;
            let farm_bonus = farm_bonus(c.attributes);
            let built = others
                .iter()
                .find(|u| u.id == target)
                .map(|u| u.life.master_id);
            let stack_made = others
                .iter()
                .find(|u| u.id == target)
                .and_then(|u| u.stack_made);
            // the state of its MoveTo's target, which the MoveTo's update checks first
            let walk_target = build.walk_target;
            let walk_target_state = others
                .iter()
                .find(|u| u.id == walk_target)
                .or_else(|| c.foreign.find(walk_target).map(|(_, u)| u))
                .map(|u| u.life.state);
            // the foundations its unit AI's queued build orders name, as they stand
            let build_targets: Vec<(i32, Option<u8>)> =
                self.ai.as_ref().map_or_else(Vec::new, |ai| {
                    ai.orders
                        .iter()
                        .filter(|o| o.order == BUILD_ORDER)
                        .map(|o| {
                            let state = others
                                .iter()
                                .find(|u| u.id == o.target)
                                .map(|u| u.life.state);
                            (o.target, state)
                        })
                        .collect()
                });
            // state 8: what the drop-off reads of its drop site and of its unit AI's orders
            let (deposit, gone_orders) = if build.state == crate::construction::build::DROP {
                let deposit = match c.spatial.get(build.site).filter(|_| build.site != -1) {
                    Some(s) => {
                        let me_at = c
                            .spatial
                            .get_known(self.id)?
                            .ok_or(GenieError::unsupported("a builder without a place"))?;
                        let master = masters
                            .and_then(|m| m.get(self.life.master_id))
                            .ok_or(crate::turn::OBJECT_MASTER)?;
                        let actor = Actor {
                            id: self.id,
                            master,
                            player: c.player,
                            held: (self.life.attribute_type, self.life.amount),
                            attribute_95: c.attributes.get(95).copied().unwrap_or(0.0),
                            civ: c.civ,
                        };
                        state_of(build.site, others, c.foreign).map(|state| {
                            let drop = crate::gathering::hunt::DropOff {
                                takes: crate::tasks::drops_at(c.drop_sites, s.master, &actor),
                                gap_squared: footprint_gap_squared(
                                    me_at.position,
                                    me_at.radius,
                                    s.position,
                                    s.radius,
                                ),
                            };
                            (state, drop)
                        })
                    }
                    None => None,
                };
                let spatial: &Spatial = c.spatial;
                let gone = |t: i32| match spatial.get(t) {
                    None => true,
                    Some(_) => spatial
                        .sight
                        .as_ref()
                        .and_then(|v| v.seer(t))
                        .is_some_and(|s| 5 < s.state),
                };
                let orders: Vec<i32> = self.ai.as_ref().map_or_else(Vec::new, |ai| {
                    ai.orders
                        .iter()
                        .filter(|o| crate::construction::build::DROPPED_ORDERS.contains(&o.order))
                        .map(|o| o.target)
                        .filter(|&t| gone(t))
                        .collect()
                });
                (deposit, orders)
            } else {
                (None, Vec::new())
            };
            // FUN_1411cc430's way to the building that takes the work: while the foundation is an
            // annex of a main building in state 0 with at least 1 hit point (COMISS: a NaN too),
            // that main building; then the foundation's annexes of a building type, which its
            // work starts first (none when the work goes on to a main building: the caller's)
            let mut worked = target;
            let mut main_work = None;
            while let Some(m) = others
                .iter()
                .find(|u| u.id == worked)
                .and_then(|u| u.annex_of)
                .and_then(|m| others.iter().find(|u| u.id == m))
                .filter(|m| {
                    m.life.state == 0 && crate::math::fmath::at_most_or_unordered(1.0, m.life.hp)
                })
            {
                worked = m.id;
                main_work = Some(crate::construction::build::MainWork {
                    construction: m
                        .construction
                        .ok_or(crate::construction::build::NO_CONSTRUCTION)?,
                    hp: m.life.hp,
                });
            }
            let annex_ids = c.spatial.annexes_of(target);
            // FUN_140aaddb0: a building annex (`+0xbb8`) counts its builders in its main
            // building
            let counted_main = others
                .iter()
                .find(|u| u.id == target)
                .filter(|u| {
                    masters
                        .and_then(|m| m.get(u.life.master_id))
                        .is_some_and(|m| m.kind == BUILDING_KIND)
                })
                .and_then(|u| u.annex_of);
            let mut main_builders = None;
            let mut site = None;
            let mut starts = Vec::new();
            for u in others.iter_mut() {
                if u.id == target {
                    site = Some(u);
                    continue;
                }
                if Some(u.id) == counted_main {
                    let k = u
                        .construction
                        .as_mut()
                        .ok_or(crate::construction::build::NO_CONSTRUCTION)?;
                    main_builders = Some(&mut k.builders);
                    continue;
                }
                if main_work.is_some() || !annex_ids.contains(&u.id) {
                    continue;
                }
                let Some(am) = masters
                    .and_then(|m| m.get(u.life.master_id))
                    .filter(|m| BUILDING_KIND <= m.kind)
                else {
                    continue;
                };
                let construction = u
                    .construction
                    .as_mut()
                    .ok_or(crate::construction::build::NO_CONSTRUCTION)?;
                starts.push(crate::construction::build::AnnexStart {
                    id: u.id,
                    construction,
                    mover: u.mover.as_mut(),
                    firepower: &mut u.firepower,
                    volley: am.volley_fire,
                });
            }
            if counted_main.is_some() && main_builders.is_none() {
                return Err(crate::construction::build::NOT_HELD);
            }
            // the slot order: the constructor made them, and their ids, in it
            starts.sort_by_key(|a| a.id);
            let site = site.and_then(|u| {
                let master = masters.and_then(|m| m.get(u.life.master_id));
                let (ai, main, rice) = (u.ai.is_some(), u.annex_of.is_some(), u.rice);
                Some(Site {
                    id: u.id,
                    life: &mut u.life,
                    construction: u.construction.as_mut()?,
                    mover: u.mover.as_mut(),
                    firepower: Some(&mut u.firepower),
                    roo_called: Some(&mut u.roo_called),
                    store: Some(&mut u.store),
                    completion: Completion {
                        master,
                        annexed,
                        attribute_204,
                        mode_680,
                        farm_bonus,
                        masters,
                        computer,
                        ai,
                        main,
                        now: world_time,
                        rice,
                    },
                    chain: crate::construction::build::WorkChain {
                        annexes: starts,
                        main: main_work,
                    },
                    main_builders,
                })
            });
            let class = masters
                .and_then(|m| m.get(self.life.master_id))
                .map_or(-1, |m| m.class);
            // FUN_140fde180: the foundation's master is a drop site (the masters 562, 584, 621,
            // 68, 1808, 50, 1187 and 199, or one the drop-site table has for every civ or the
            // builder's)
            let drop_site = built.is_some_and(|m| {
                DROP_SITE_MASTERS.contains(&m)
                    || c.drop_sites
                        .iter()
                        .any(|d| d.building == m && (d.civs.is_empty() || d.civs.contains(&c.civ)))
            });
            // FUN_140aadc50 drops only a kind the player has (FUN_140efe5b0)
            let held_kind = self.life.attribute_type;
            let has_kind = usize::try_from(held_kind).is_ok_and(|k| k < c.attributes.len());
            let builder = Builder {
                class,
                master: self.life.master_id,
                held: self.life.amount,
                held_kind,
                drop_site: drop_site && has_kind,
                ai: self.ai.as_mut(),
                waits: &mut self.footprint_waits,
                build_targets: &build_targets,
                look: crate::construction::build::Looks(&|id| looks.get(&id).copied()),
                walk_target_state,
                deposit,
                attributes: c.attributes.len(),
                gone_orders: &gone_orders,
                stack_made,
            };
            let step = build.update(c.walk(self.mover.as_mut(), me), site, builder)?;
            c.cleared.extend(step.cleared.iter().copied());
            c.site_removals.extend(step.removes);
            if !step.occupants.is_empty() {
                c.evacuations.push((target, step.occupants.clone()));
            }
            // the constructions that started, its annexes' first: their owner watches each
            // (FUN_1411e5670), the caller's
            for id in annex_ids.iter().copied().chain([target]) {
                if let Some(cons) = others
                    .iter_mut()
                    .find(|u| u.id == id)
                    .and_then(|u| u.construction.as_mut())
                    .filter(|cons| cons.started)
                {
                    cons.started = false;
                    c.started.push(id);
                }
            }
            // the work an annex's main building takes, the caller's
            if let Some(amount) = step.main_work {
                c.main_works.push((target, amount));
            }
            if let Some(built) = step.completed {
                // the completion's first part's tracker part (FUN_1411c6bd0 ->
                // FUN_14104eac0); the foundation is the builder's player's
                if let Some(u) = others.iter_mut().find(|u| u.id == target) {
                    let k = crate::tracker::key(c.masters, c.lines, u.life.master_id);
                    c.tracker.complete(target, k);
                    // its set_state(2)'s standing sprite (FUN_1411dbb70)
                    if let Some(m) = c.masters.and_then(|m| m.get(u.life.master_id)) {
                        crate::construction::build::show_built(
                            &mut u.sprite,
                            m,
                            u.build_frame,
                            c.graphics,
                            c.streams.get(Stream::Sprite),
                        );
                    }
                }
                // its set_state(2) took its annexes along first (FUN_1411dbb70)
                for annex in c.spatial.annexes_of(target) {
                    c.annex_states.push((annex, 2));
                }
                c.history_events.extend(built.history);
                c.attribute_adds.extend(built.attributes);
                c.techs_done.extend(built.tech);
                if built.pasture {
                    c.pastures.push(target);
                }
                if let Some(state) = built.cart {
                    c.carts.push((target, state));
                }
                if built.stack.is_some() || built.vanishes {
                    c.completions.push((target, built.stack, built.vanishes));
                }
            }
            // the load dropped at the end of the work (FUN_140aadc50: the player's slot 0xf8 with
            // 1 and 1, then the builder carries nothing, FUN_14119d310)
            if let Some((kind, amount)) = step.dropped {
                give_load(
                    (&c.spatial.relations, c.sharing),
                    c.player,
                    &mut c.attribute_adds,
                    kind,
                    amount,
                )?;
                self.life.attribute_type = -1;
                self.life.amount = 0.0;
            }
            // the unit's animation stopped (FUN_141161850), the second target let go
            if step.anim_stopped {
                self.anim_speed = Some(0.0);
            }
            if let Some(s) = step.site_let_go {
                c.releases.push((s, by));
            }
            if step.done {
                // the action list removes it; its destructor (FUN_140aadbf0) finds it out of the
                // builders (set_state left them) and releases the foundation and the second
                // target it still holds (slot 0x180, FUN_140a73fc0)
                let (holds, second) = self.build.map_or((false, -1), |b| (b.target != -1, b.site));
                self.build = None;
                if holds {
                    release_own(others, masters, target, by)?;
                }
                if second != -1 {
                    c.releases.push((second, by));
                }
            }
            if step.released {
                release_own(others, masters, target, by)?;
            }
            if step.notified {
                self.post_own(SEARCH_NEEDED, [BUILD_ACTION, 0, 0])?;
            }
            // a pasture already built (FUN_1411c5f70's new id)
            if let Some(made) = step.pasture_made {
                self.post_own(crate::unit_ai::PASTURE_BUILT, [BUILD_ACTION, made, 0])?;
            }
            if step.failed {
                self.post_own(crate::unit_ai::ACTION_FAILED, [BUILD_ACTION, 0, 0])?;
            }
            if step.stands {
                self.show_standing(c.masters, c.graphics, c.streams.get(Stream::Sprite))?;
            }
            if step.works_it {
                // slot 0x7e8 on the building it built, at its place (the caller's work command)
                let point = c
                    .spatial
                    .get(target)
                    .ok_or(GenieError::unsupported(
                        "a builder's building without a place",
                    ))?
                    .position;
                c.work_order = Some(crate::order::Work {
                    target,
                    point,
                    carry_test: false,
                    filter: -1,
                    notified: true,
                });
            }
            if step.search {
                let built = built.ok_or(GenieError::unsupported(
                    "a builder's building the slice does not hold",
                ))?;
                if let Some(work) = self.more_work(c, others, target, built)? {
                    c.work_order = Some(work);
                } else {
                    if let Some(b) = self.build.as_mut() {
                        b.no_more_work()?;
                    }
                    release_own(others, masters, target, by)?;
                    // set_state(3) without orders: notify 0x202, then the standing sprite
                    self.post_own(SEARCH_NEEDED, [BUILD_ACTION, 0, 0])?;
                    self.show_standing(c.masters, c.graphics, c.streams.get(Stream::Sprite))?;
                }
            }
            if step.drop_off {
                self.build_drop_off(c, others, me)?;
            }
        }
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
            None => crate::move_to::MoveStep::default(),
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
                crate::explore::DONE => {
                    return Err(GenieError::unsupported(
                        "a done Explore (the action list after it)",
                    ));
                }
                crate::explore::FAILED => {
                    return Err(GenieError::unsupported(
                        "a failed Explore (notify 0x1f9, set_state 2)",
                    ));
                }
                crate::explore::WALK => {
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
                            let goal = crate::move_to::Goal::Point(explore.point);
                            let walk = c.walk(self.mover.as_mut(), me);
                            explore.move_to = Some(
                                match crate::move_to::start_walk_or_give_up(walk, goal, 0.0, quiet)?
                                {
                                    Some(state) => state,
                                    None => explore.move_wait.give_up(waits, stance),
                                },
                            );
                            0
                        }
                        Some(MoveTo::Moving) => {
                            let walk = c.walk(self.mover.as_mut(), me);
                            explore.move_to = Some(match crate::move_to::walk_or_give_up(walk)? {
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
            let sky = crate::fly::Sky {
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
        // the Hunt's update (0x140ac7a00) first checks its targets: one gone from the world's
        // table is dropped (slot 0x68 with none), one past state 6 is let go (slot 0x180, its
        // `+0x60` being 0) and dropped; then the second target ([`second_target_checked`])
        if let Some(target) = self.hunt.map(|h| h.target).filter(|&t| t != -1) {
            let state = c
                .victim
                .as_deref()
                .filter(|v| v.id == target)
                .map(|v| v.state);
            let gone = match state {
                Some(s) => 6 < s,
                None if c.spatial.get(target).is_some() => {
                    return Err(GenieError::unsupported(
                        "a hunt's target the slice cannot read",
                    ));
                }
                None => true,
            };
            if gone {
                if state.is_some() {
                    c.releases.push((target, self.releaser(c)));
                }
                if let Some(h) = self.hunt.as_mut() {
                    h.target = -1;
                }
            }
        }
        if let Some(site) = self.hunt.map(|h| h.carry_to) {
            let held = c.spatial.get(site).is_some();
            let (kept, let_go) =
                second_target_checked(site, held, state_of(site, others, c.foreign));
            if let Some(h) = self.hunt.as_mut() {
                h.carry_to = kept;
            }
            if let Some(t) = let_go {
                self.hunt_release(c, [t]);
            }
        }
        // the hunter as a combat object, which its hits read and change
        let own = (self.hunt.is_some() || self.attack.is_some())
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
        // a hunt whose target went on the walk to it (state 4, its MoveTo quiet: code 5), or
        // whose MoveTo failed (code 3), and whose class is a tree's searches for the next tree
        // (FUN_140ac4670 with 8 and -1 both times)
        let tree_lost = self.hunt.as_ref().is_some_and(|h| {
            h.state == crate::gathering::hunt::WALK
                && 0.0 <= h.timer
                && ((h.target == -1 && h.move_to.is_some()) || h.move_to == Some(MoveTo::Failed))
        }) && self.ai.as_ref().is_some_and(|ai| {
            let class = if ai.target_type == -1 {
                self.hunt.map_or(-1, |h| i32::from(h.target_class))
            } else {
                ai.target_type
            };
            class == crate::gathering::hunt::TREE_CLASS
        });
        if let Some(hunt) = self
            .hunt
            .as_mut()
            .filter(|h| h.state == crate::gathering::hunt::DROP && h.timer < 0.0)
        {
            // case 7 waiting to try again: the timer runs by dt; at its end the drop site goes
            // (FUN_140a7a190 with none: slot 0x180) and set_state(8) searches again
            hunt.timer += c.dt_seconds;
            if 0.0 <= hunt.timer {
                hunt.timer = 0.0;
                let site = std::mem::replace(&mut hunt.carry_to, -1);
                self.hunt_release(c, [site]);
                self.hunt_drop_off(c, others)?;
            }
        } else if let Some(hunt) = self
            .hunt
            .as_mut()
            .filter(|h| h.state == crate::gathering::hunt::DROP)
        {
            // state 8 has no target: its drop site (the second target) is what it walks to
            let master = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(crate::turn::HUNTER_MASTER)?;
            let me_at = c
                .spatial
                .get_known(self.id)?
                .ok_or(GenieError::unsupported("a hunter without a place"))?;
            let (me_position, me_radius) = (me_at.position, me_at.radius);
            let site = match c.spatial.get_known(hunt.carry_to)? {
                Some(s) => {
                    let actor = Actor {
                        id: self.id,
                        master,
                        player: c.player,
                        held: (self.life.attribute_type, self.life.amount),
                        attribute_95: c.attributes.get(95).copied().unwrap_or(0.0),
                        civ: c.civ,
                    };
                    Some(crate::gathering::hunt::DropOff {
                        takes: crate::tasks::drops_at(c.drop_sites, s.master, &actor),
                        gap_squared: footprint_gap_squared(
                            me_position,
                            me_radius,
                            s.position,
                            s.radius,
                        ),
                    })
                }
                None => None,
            };
            let attributes = c.attributes.len();
            let shows = hunt_walk_sprite(hunt, c.graphics, &self.life);
            let who = crate::gathering::hunt::PointWalker {
                computer: c.computer,
                villager: master.class == 4,
                stance: self.ai.as_ref().map_or(0, |ai| ai.stance),
            };
            let (walk, rng) = c.walk_and_sprites(self.mover.as_mut(), me);
            let shown = crate::gathering::hunt::Shown {
                sprite: &mut self.sprite,
                rng,
                graphic: shows,
            };
            match hunt.update_drop(
                walk,
                &mut self.life.attribute_type,
                &mut self.life.amount,
                attributes,
                site,
                shown,
                who,
                &mut self.anim_speed,
            )? {
                crate::gathering::hunt::Dropped::Going => {}
                crate::gathering::hunt::Dropped::Load(kind, amount) => {
                    // the player's slot 0xf8 (FUN_141616820) with 1 and 1: the resource, and its
                    // gathered total (166 to 168; gold's 49 and 169); the statistics (slot 0xb8)
                    // and the local player's tracker (FUN_14097daf0) are not held; a sharing
                    // player's allies' share (+0x1426) is not modelled
                    give_load(
                        (&c.spatial.relations, c.sharing),
                        c.player,
                        &mut c.attribute_adds,
                        kind,
                        amount,
                    )?;
                    self.hunt_after_drop(c, others)?;
                }
                crate::gathering::hunt::Dropped::Again => {
                    // the drop site goes (FUN_140a7a190 with none), then set_state(8)
                    let hunt = self.hunt.as_mut().expect("above");
                    let site = std::mem::replace(&mut hunt.carry_to, -1);
                    self.hunt_release(c, [site]);
                    self.hunt_drop_off(c, others)?;
                }
                crate::gathering::hunt::Dropped::Idle => {
                    return Err(GenieError::unsupported(
                        "a hunt's drop-off idling (set_state 2)",
                    ));
                }
            }
        } else if let Some(hunt) = self
            .hunt
            .as_mut()
            .filter(|h| h.state == crate::gathering::hunt::FAILED)
        {
            // state 13 (0x140ac7a00 case 0xc): notify 0x1f9 with the hunt's action value, and the
            // update returns 3: the action list removes the hunt (FUN_140a98e20), whose teardown
            // lets its target, then its second target go
            let (target, site) = (hunt.target, hunt.carry_to);
            if hunt.attack.is_some() || hunt.move_to.is_some() {
                return Err(GenieError::unsupported("a failed hunt with a sub-action"));
            }
            self.post_own(
                crate::unit_ai::ACTION_FAILED,
                [crate::unit_ai::HUNT_ACTION, 0, 0],
            )?;
            self.hunt = None;
            if target != -1 {
                if others.iter().any(|u| u.id == target) {
                    let by = self.releaser(c);
                    release_own(others, c.masters, target, by)?;
                } else {
                    // another player's (Gaia's deer, a neighbour's sheep): its slot 0x180 after
                    // this update ([`Turns::release`]), as the other teardowns' releases
                    self.hunt_release(c, [target]);
                }
            }
            self.hunt_release(c, [site]);
        } else if let Some(hunt) = self.hunt.as_mut().filter(|h| {
            matches!(
                h.state,
                crate::gathering::hunt::WALK_POINT | crate::gathering::hunt::WALK_SEARCH
            )
        }) {
            // states 15 and 11: the walk to a point; state 15's end is set_state(2): the unit
            // takes the hunt's sprite to stand with (slot 0x68 with FUN_140ac5210's), the target
            // and the second target go; state 11's is set_state(3), the search
            let master = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(crate::turn::HUNTER_MASTER)?;
            let who = crate::gathering::hunt::PointWalker {
                computer: c.computer,
                villager: master.class == 4,
                stance: self.ai.as_ref().map_or(0, |ai| ai.stance),
            };
            let shows = hunt_walk_sprite(hunt, c.graphics, &self.life);
            let (walk, rng) = c.walk_and_sprites(self.mover.as_mut(), me);
            let shown = crate::gathering::hunt::Shown {
                sprite: &mut self.sprite,
                rng,
                graphic: shows,
            };
            match hunt.update_point_walk(walk, who, shown)? {
                crate::gathering::hunt::PointWalk::Walking => {}
                crate::gathering::hunt::PointWalk::Search => self.hunt_search_state(c, others)?,
                crate::gathering::hunt::PointWalk::Done { target, second } => {
                    let task = hunt.task;
                    self.hunt_stand(c, &task)?;
                    self.hunt_release(c, [target, second]);
                }
            }
        } else if let Some(hunt) = self
            .hunt
            .as_mut()
            .filter(|h| h.state == crate::gathering::hunt::SEARCH)
        {
            // state 3 (case 2): the timer runs by dt; at its end it starts again at −10 s and the
            // unit AI is told again (notify 0x202)
            if hunt.timer < 0.0 {
                hunt.timer += c.dt_seconds;
                if 0.0 <= hunt.timer {
                    hunt.timer = SEARCH_AGAIN;
                    self.hunt_search_notify(c, others)?;
                }
            }
        } else if self
            .hunt
            .as_ref()
            .is_some_and(|h| h.state == crate::gathering::hunt::IDLE)
        {
            // state 2 (case 1): a done hunt waits; one of a tree's target class would search
            // again (set_state 3)
            if self.hunt.as_ref().is_some_and(|h| h.target_class == 15) {
                return Err(GenieError::unsupported(
                    "a done hunt of a tree's class (set_state 3)",
                ));
            }
        } else if let Some(hunt) = self
            .hunt
            .as_mut()
            .filter(|h| h.target == -1 && h.state == crate::gathering::hunt::WALK && h.timer < 0.0)
        {
            // case 3 waiting after a given-up MoveTo, its target gone: the timer runs by dt,
            // then the walk to the point (set_state 11)
            let mover = self.mover.as_mut().ok_or(crate::turn::HUNTER_UNMOVING)?;
            hunt.wait_then_walk(c.dt_seconds, mover)?;
        } else if self
            .hunt
            .as_ref()
            .is_some_and(|h| h.state == crate::gathering::hunt::LEAD && h.target == -1)
        {
            // state 10 without its target (0x140ac7a00 case 9): set_state(3), the search
            self.hunt.as_mut().expect("above").state = crate::gathering::hunt::SEARCH;
            self.hunt_set_search(c, others)?;
        } else if tree_lost && self.hunt_tree_search(c, others, Some(TREE_SEARCH_WITHIN), None)? {
            // the hunt took the next tree (slot 0x48): its MoveTo's code 5 ends the update
        } else if let Some(hunt) = self.hunt.as_mut() {
            let computer = c.computer;
            let master = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(crate::turn::HUNTER_MASTER)?;
            let victim = c.victim.take().ok_or(GenieError::unsupported(
                "an attack's target the slice does not hold live",
            ))?;
            let own = own.ok_or(crate::turn::HUNTER_MASTER)?;
            // the gatherer's view: the master's work rate (+0x798) plus the powerup (slot 0x6e8)
            let productivity = usize::try_from(hunt.task.productivity)
                .ok()
                .and_then(|i| c.attributes.get(i))
                .copied()
                .filter(|&v| 0.0 < v)
                .unwrap_or(1.0);
            let g = |id: i16| (0 <= id).then(|| c.graphics.get(&id).copied()).flatten();
            let sprites = HuntSprites {
                moving: g(hunt.task.sprites[0]),
                working: g(hunt.task.sprites[2]),
                carrying: g(hunt.task.sprites[3]),
                standing: g(master.standing_sprite),
            };
            let mut gather = Gatherer {
                held_kind: &mut self.life.attribute_type,
                held: &mut self.life.amount,
                work_rate: self.work_rate_powerup + master.work_rate,
                capacity: f32::from(master.capacity),
                productivity,
                target_class: victim.class,
                target_storage: victim.storage,
                stops_decay: c.attributes.get(292) == Some(&1.0),
                ai_class: self.ai.as_ref().map(|ai| ai.target_type),
                sprites,
                tree_searched: tree_lost,
                predator_hunts_end: c.predator_hunts_end,
                searches: false,
                leads: None,
                attack_failed: false,
                let_go: None,
            };
            // what set_state(6) on live livestock reads (0x140ac60e0 case 6), and the lead's
            // update (state 10, 0x140ac7a00 case 9: whatever the target, a dead one too)
            let faces =
                hunt.state == crate::gathering::hunt::WALK && hunt.move_to == Some(MoveTo::Arrived);
            let herd = if (faces
                && victim.class == crate::gathering::hunt::LIVESTOCK
                && victim.state <= 2)
                || hunt.state == crate::gathering::hunt::LEAD
            {
                let at = |id| {
                    c.spatial
                        .get_known(id)?
                        .ok_or(GenieError::unsupported("a hunt's object without a place"))
                };
                let (me_at, them) = (at(self.id)?, at(hunt.target)?);
                let speed = self.mover.as_ref().map_or(0.0, |m| m.stats.speed);
                let engagement = crate::combat::attack::engagement(
                    self.ai.as_ref(),
                    speed,
                    master,
                    me_at,
                    them,
                    c.world_time,
                )?;
                let moving = hunt.task.walk_sprite_id(*gather.held_kind, *gather.held);
                let arms =
                    crate::combat::attack::arms(master, c.graphics, hunt.task.sprites, moving);
                let own = them.player == c.player;
                let drop_site = if !own {
                    None
                } else if hunt.drop_site != -1 {
                    let site = c.spatial.get(hunt.drop_site);
                    site.map(|s| {
                        let d =
                            footprint_gap_squared(them.position, them.radius, s.position, s.radius);
                        (hunt.drop_site, d)
                    })
                } else {
                    if computer {
                        return Err(GenieError::unsupported(
                            "a computer player's hunt of its livestock (its drop-site search)",
                        ));
                    }
                    let sites: Vec<i16> = c
                        .drop_sites
                        .iter()
                        .filter(|d| d.livestock)
                        .map(|d| d.building)
                        .collect();
                    nearest_drop_site(others, &sites, them.position)
                        .map(|(id, _)| -> Result<(i32, f32), GenieError> {
                            let s = at(id)?;
                            Ok((
                                id,
                                footprint_gap_squared(
                                    them.position,
                                    them.radius,
                                    s.position,
                                    s.radius,
                                ),
                            ))
                        })
                        .transpose()?
                };
                // the animal's top action without a target or a point (slots 0x190, 0x288)
                let animal = others
                    .iter()
                    .find(|u| u.id == hunt.target)
                    .or_else(|| c.foreign.find(hunt.target).map(|(_, u)| u));
                let idle = match animal {
                    Some(u) => u.top_action_idle()?,
                    None => false,
                };
                Some(crate::gathering::hunt::Herd {
                    own,
                    drop_site,
                    engagement,
                    arms,
                    idle,
                    special_graphic: if master.kind == 70 && master.special_graphic {
                        Some(c.graphics.get(&master.special_sprite).copied().ok_or(
                            GenieError::unsupported(
                                "a special graphic the data file does not have",
                            ),
                        )?)
                    } else {
                        None
                    },
                })
            } else {
                None
            };
            let mut blasted = std::mem::take(&mut c.blasted);
            // the unit AI's canMoveToAttack (FUN_140c48420) and stance
            let can_move = self
                .ai
                .as_ref()
                .is_some_and(|ai| ai.current_order != -1 || ai.stance <= 1);
            let stance = self.ai.as_ref().map_or(0, |ai| ai.stance);
            let world_time = c.world_time;
            let standing = c.graphics.get(&master.standing_sprite).copied();
            let volleys = c
                .masters
                .is_some_and(|m| crate::combat::attack::volleys(master, m));
            let swing_sprites = crate::combat::attack::swing_sprites(
                self.abilities.look.as_ref(),
                master,
                c.graphics,
            );
            let (walk, sprite_rng) = c.walk_and_sprites(self.mover.as_mut(), me);
            let mut strike = Strike {
                sprite: &mut self.sprite,
                anim_speed: &mut self.anim_speed,
                standing,
                volleys,
                attack_timer: &mut self.attack_timer,
                reload_worldtime: &mut self.reload_worldtime,
                reload: self.reload_stinger + (master.reload - self.reload_powerup),
                hit_chance: master.hit_chance,
                victim,
                blasted: &mut blasted,
                sprite_rng,
                own,
                executed: None,
                fire: None,
                can_move,
                stance,
                state_position: self.ai.as_ref().map_or([0.0; 2], |ai| ai.state_position),
                posted: Vec::new(),
                queue_empty: self.ai.as_ref().is_some_and(|ai| ai.orders.is_empty()),
                world_time,
                ai_order: self.ai.as_ref().map(|ai| ai.current_order),
                ai_type: self.ai.as_ref().map(|ai| ai.ai_type),
                computer,
                armed: true,
                volley_under_way: self.next_volley != 0,
                let_go: None,
                forgets: false,
                sends_home: false,
                swing_sprites,
            };
            let full = hunt.update(walk, computer, &mut strike, &mut gather, herd)?;
            let searches = gather.searches;
            let (attack_failed, let_go) = (gather.attack_failed, gather.let_go);
            let posted = std::mem::take(&mut strike.posted);
            let leads = gather.leads.map(|site| (hunt.target, site));
            let fired = strike.fire.map(|target| {
                let v = &*strike.victim;
                Aimed {
                    target,
                    position: v.position,
                    radius: [v.radius[0], v.radius[1], v.height],
                    class: v.class,
                    player: v.player,
                }
            });
            let (executed, own) = (strike.executed, std::mem::take(&mut strike.own));
            c.blasted = blasted;
            if let Some(aimed) = fired {
                self.volley(c, aimed)?;
            }
            if let Some(target) = executed {
                self.post_own(ACTION_EXECUTED, [600, target, 0])?;
            }
            for (kind, p) in posted {
                self.post_own(kind, p)?;
            }
            c.own_killed = self.hit_back(own)?;
            if full {
                self.hunt_drop_off(c, others)?;
            }
            if searches {
                self.hunt_set_search(c, others)?;
            }
            if let Some(target) = let_go {
                self.hunt_release(c, [target]);
            }
            if attack_failed {
                // FUN_140ac4670 with -1 and 5: the class its unit AI targets (else the hunt's)
                // that the civilian's slot 0x1b0 gives 0 (trees) takes the next tree within 5
                // of the unit; else set_state(3) with the target kept
                let tree = self.ai.as_ref().is_some_and(|ai| {
                    let class = if ai.target_type == -1 {
                        self.hunt.map_or(-1, |h| i32::from(h.target_class))
                    } else {
                        ai.target_type
                    };
                    class == crate::gathering::hunt::TREE_CLASS
                });
                if !(tree && self.hunt_tree_search(c, others, None, Some(TREE_NEAR))?) {
                    let hunt = self.hunt.as_mut().expect("above");
                    hunt.tear_down(self.mover.as_mut())?;
                    hunt.state = crate::gathering::hunt::SEARCH;
                    self.hunt_search_state(c, others)?;
                }
            }
            c.leads = leads;
        } else if let Some(attack) = self.attack.as_mut().filter(|a| a.target == -1) {
            // the unit's own Attack after its target went ([`Attack::lost`])
            let master = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(crate::turn::ATTACKER_MASTER)?;
            let armed = armed(master, self.firepower);
            let (code, disarmed) =
                attack.update_targetless(c.dt_seconds, armed, &mut self.anim_speed)?;
            if disarmed {
                let posted = attack.disarmed(
                    &mut self.sprite,
                    &mut self.anim_speed,
                    c.streams.get(Stream::Sprite),
                )?;
                if let Some((kind, p)) = posted {
                    self.post_own(kind, p)?;
                }
            }
            if matches!(code, 1 | 3) {
                self.delete_attack(c)?;
            }
        } else if let Some(attack) = self.attack.as_mut() {
            // the unit's own Attack (slot 6, 0x140a82ae0), its notifies posted to its unit AI
            // (slot 0x328); codes 1 and 3 delete it (RGE_Action_Object::update)
            let master = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(crate::turn::ATTACKER_MASTER)?;
            let armed = armed(master, self.firepower);
            let computer = c.computers.get(c.player).copied().unwrap_or(false);
            let victim = c.victim.take().ok_or(GenieError::unsupported(
                "an attack's target the slice does not hold live",
            ))?;
            let own = own.ok_or(crate::turn::ATTACKER_MASTER)?;
            let mut blasted = std::mem::take(&mut c.blasted);
            let can_move = self
                .ai
                .as_ref()
                .is_some_and(|ai| ai.current_order != -1 || ai.stance <= 1);
            let stance = self.ai.as_ref().map_or(0, |ai| ai.stance);
            let world_time = c.world_time;
            let standing = c.graphics.get(&master.standing_sprite).copied();
            let volleys = c
                .masters
                .is_some_and(|m| crate::combat::attack::volleys(master, m));
            let swing_sprites = crate::combat::attack::swing_sprites(
                self.abilities.look.as_ref(),
                master,
                c.graphics,
            );
            let (walk, sprite_rng) = c.walk_and_sprites(self.mover.as_mut(), me);
            let mut strike = Strike {
                sprite: &mut self.sprite,
                anim_speed: &mut self.anim_speed,
                standing,
                volleys,
                attack_timer: &mut self.attack_timer,
                reload_worldtime: &mut self.reload_worldtime,
                reload: self.reload_stinger + (master.reload - self.reload_powerup),
                hit_chance: master.hit_chance,
                victim,
                blasted: &mut blasted,
                sprite_rng,
                own,
                executed: None,
                fire: None,
                can_move,
                stance,
                state_position: self.ai.as_ref().map_or([0.0; 2], |ai| ai.state_position),
                posted: Vec::new(),
                queue_empty: self.ai.as_ref().is_some_and(|ai| ai.orders.is_empty()),
                world_time,
                ai_order: self.ai.as_ref().map(|ai| ai.current_order),
                ai_type: self.ai.as_ref().map(|ai| ai.ai_type),
                computer,
                armed,
                volley_under_way: self.next_volley != 0,
                let_go: None,
                forgets: false,
                sends_home: false,
                swing_sprites,
            };
            let code = attack.update(walk, &mut strike)?;
            let fired = strike.fire.map(|target| {
                let v = &*strike.victim;
                Aimed {
                    target,
                    position: v.position,
                    radius: [v.radius[0], v.radius[1], v.height],
                    class: v.class,
                    player: v.player,
                }
            });
            let (executed, own) = (strike.executed, std::mem::take(&mut strike.own));
            let posted = std::mem::take(&mut strike.posted);
            let (let_go, forgets) = (strike.let_go, strike.forgets);
            let sends_home = strike.sends_home;
            c.blasted = blasted;
            if let Some(t) = let_go {
                c.releases.push((t, self.releaser(c)));
            }
            if forgets && let Some(ai) = self.ai.as_mut() {
                ai.remove_target();
            }
            if sends_home && let Some(home) = self.ai.as_mut().and_then(|ai| ai.home.as_mut()) {
                home.go_home = true;
            }
            for (kind, p) in posted {
                self.post_own(kind, p)?;
            }
            if let Some(aimed) = fired {
                self.volley(c, aimed)?;
            }
            if let Some(target) = executed {
                self.post_own(ACTION_EXECUTED, [600, target, 0])?;
            }
            c.own_killed = self.hit_back(own)?;
            if matches!(code, 1 | 3) {
                self.delete_attack(c)?;
            }
        }
        // the motion step may have put the unit onto a waypoint it reached
        if let Some(p) = c.spatial.get(self.id) {
            self.position = p.position;
        }
        if let Some(m) = self.mover.as_mut() {
            c.pauses.append(&mut m.pauses);
            if !m.in_the_way.is_empty() {
                return Err(crate::turn::IN_THE_WAY);
            }
        }
        let resume = AiResume {
            captured,
            leaves,
            after: None,
        };
        // a builder's work found in its Build's update (slot 0x7e8 from case 0xdf) runs inside the
        // action update; the caller gives it (`Turns::order_work`) and then runs the rest of this
        // update ([`Unit::ai_part`])
        if c.work_order.is_some() || !c.completions.is_empty() || c.transport_unload {
            c.ai_resume = Some(resume);
            return Ok(Step::Stay);
        }
        self.ai_part(c, others, resume)
    }

    /// [`Unit::ai_part`]'s first step: the pathing grid takes a unit of a master of kind 40 or
    /// more where it stands, moving or not.
    pub(super) fn refresh_pathing(&self, c: &mut Context<'_>) {
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
