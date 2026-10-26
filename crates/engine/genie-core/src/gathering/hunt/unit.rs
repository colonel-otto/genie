//! A hunter's search, its walk after a drop and its drop-off.

use crate::math::rng::Stream;
use crate::movement::{Me, Planning};
use crate::spatial::{Placed, Spatial};
use crate::turn::unit::Unit;
use crate::turn::view::{Context, DropWorld};
use crate::turn::world::herded;
use crate::turn::{SEARCH_AGAIN, SEARCH_NEEDED};
use crate::unit_ai::{GenieError, Search};

impl Unit {
    /// The Hunt's `set_state(3)` (`0x140ac60e0` case 3) after its target went empty when it faced
    /// it: the target goes (slot 0x68 with none: its slot 0x180), the unit AI is told
    /// ([`Unit::hunt_search_notify`]), the unit takes the sprite it stands with (slot 0x68 with
    /// `FUN_140ac5210`'s) and the timer starts at −10 s. A hunt of class 10 would end its search
    /// (`set_state` 2), not modelled.
    pub(crate) fn hunt_set_search(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), GenieError> {
        let hunt = self.hunt.as_mut().expect("the caller's hunt");
        if hunt.target_class != 10 {
            let target = std::mem::replace(&mut hunt.target, -1);
            self.hunt_release(c, [target]);
        }
        self.hunt_search_state(c, others)
    }

    /// The Hunt's `set_state(3)` itself (`0x140ac60e0` case 3; its target stays): the unit AI is
    /// told ([`Unit::hunt_search_notify`]), the unit takes the sprite it stands with and the timer
    /// starts at −10 s. A human player's hunt of class 10 is done instead (`set_state` 2: the
    /// target and the second target go), a computer player's would end (1), not modelled.
    pub(crate) fn hunt_search_state(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), GenieError> {
        let hunt = self.hunt.as_mut().expect("the caller's hunt");
        let task = hunt.task;
        if hunt.target_class == 10 {
            // a human player's hunt is done (set_state 2), a computer player's ends (1)
            if c.computer {
                return Err(GenieError::unsupported(
                    "a computer player's hunt of class 10's search (set_state 1)",
                ));
            }
            let released = hunt.set_done();
            self.hunt_stand(c, &task)?;
            self.hunt_release(c, released);
            return Ok(());
        }
        hunt.timer = SEARCH_AGAIN;
        self.hunt_search_notify(c, others)?;
        self.hunt_stand(c, &task)
    }

    /// The hunter takes the sprite its hunt stands with (slot 0x68 with `FUN_140ac5210`'s, by the
    /// task and what the unit carries; [`crate::gathering::hunt::stand_sprite`]).
    pub(crate) fn hunt_stand(
        &mut self,
        c: &mut Context<'_>,
        task: &crate::gathering::hunt::HuntTask,
    ) -> Result<(), GenieError> {
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(crate::turn::HUNTER_MASTER)?;
        let stand = crate::gathering::hunt::stand_sprite(
            task,
            master.standing_sprite,
            self.life.attribute_type,
            self.life.amount,
            c.graphics,
        );
        if let Some(s) = stand {
            crate::combat::attack::set_sprite(&mut self.sprite, s, c.streams.get(Stream::Sprite));
        }
        Ok(())
    }

    /// The objects a hunt let go (−1 for none), released in order after the update.
    pub(crate) fn hunt_release<const N: usize>(&self, c: &mut Context<'_>, ids: [i32; N]) {
        for id in ids.into_iter().filter(|&id| id != -1) {
            c.releases.push((id, self.releaser(c)));
        }
    }

    /// The Hunt in state 3 tells its unit AI to look for the next target (`0x140ac60e0` case 3,
    /// `0x140ac7a00` case 2): notify 0x202 with the hunt action (0x265), its target class (`+0x68`)
    /// and its drop site: the second target; −1 for a master without drop sites (`+0x7a0`); else
    /// the closest (`FUN_1411671e0` as `set_state(8)` searches, [`Unit::drop_site`]), with none
    /// found 0, and the hunt's class goes (−1).
    pub(crate) fn hunt_search_notify(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), GenieError> {
        let hunt = self.hunt.as_ref().expect("the caller's hunt");
        let (carry_to, class) = (hunt.carry_to, i32::from(hunt.target_class));
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(crate::turn::HUNTER_MASTER)?;
        let site = if carry_to != -1 {
            carry_to
        } else if master.drop_sites.first().is_none_or(|&d| d == -1) {
            -1
        } else {
            let mut w = DropWorld {
                masters: c.masters,
                spatial: c.spatial,
                zones: c.zones,
                pathing: c.pathing,
                object: c.streams.get(Stream::Object),
                world_time: c.world_time,
                computer: c.computer,
                construction_rate: c.construction_rate,
                groups: c.groups,
            };
            let (site, _) = self.drop_site(&mut w, others, -1, crate::gathering::hunt::SEARCH)?;
            if site == -1 {
                self.hunt.as_mut().expect("the caller's hunt").target_class = -1;
                0
            } else {
                site
            }
        };
        self.post_own(SEARCH_NEEDED, [crate::unit_ai::HUNT_ACTION, class, site])
    }

    /// The hunt's state 8 after its load went (`0x140ac7a00` case 7, after `FUN_14119d310`): a
    /// hunt with a target lets the drop site go (`FUN_140a7a190` with none: slot 0x180) and walks
    /// back to it (`set_state` 4). One without asks its unit AI: the queued orders (gather, hunt,
    /// on a target) whose target is gone or dead go (`FUN_140c5de60`); orders left end the hunt
    /// (the AI's order and action clear, `set_state` 1, the action list removes it: its teardown
    /// lets the drop site go). The target class (the AI's target's, else the hunt's `+0x68`) the
    /// civilian's slot 0x1b0 knows (`FUN_140c1c4f0`) takes the search by kind (slot 0x80); a hunt
    /// of class 10 searches nothing; a unit in a pasture (`FUN_1411c7670`) and a farm found are
    /// not modelled. Else `ClosestObject` (`FUN_140c488b0`, every relation, below one worker for
    /// class 49) of the class over the AI's search (`FUN_140c63e20`: its sight plus a tile, at
    /// least its search radius, plus the civilian's growth; enemies found are not modelled, else
    /// the alert mode clears). The object found becomes the hunt's target (slot 0x48,
    /// [`HuntAction::retarget`], which takes it: its workers count one more); with none a human
    /// player's hunt walks to its point to search again (`set_state(11)`,
    /// [`HuntAction::search_walk`]). A hunt that fails at once takes the standing sprite (slot
    /// 0x68 with `FUN_140ac5210`'s: the unit carries nothing now).
    ///
    /// [`HuntAction::retarget`]: crate::gathering::hunt::HuntAction::retarget
    /// [`HuntAction::search_walk`]: crate::gathering::hunt::HuntAction::search_walk
    pub(crate) fn hunt_after_drop(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
    ) -> Result<(), GenieError> {
        let hunt = self.hunt.as_ref().expect("the caller's hunt");
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(crate::turn::HUNTER_MASTER)?;
        if hunt.target != -1 {
            // back to the target: the drop site goes (FUN_140a7a190 with none), set_state(4)
            let v = c.victim.as_deref().ok_or(GenieError::unsupported(
                "a hunt's target the slice does not hold live",
            ))?;
            let quarry = crate::gathering::hunt::Quarry {
                id: v.id,
                position: v.position,
                class: v.class,
                kind: v.kind,
                state: v.state,
                player: v.player,
                amount: v.amount,
                allied: v.allied[0],
                top_action: None,
                drop_site: false,
            };
            let (me_at, them) = (
                c.spatial
                    .get_known(self.id)?
                    .ok_or(GenieError::unsupported("a hunter without a place"))?,
                c.spatial
                    .get_known(v.id)?
                    .ok_or(GenieError::unsupported("a hunt's target without a place"))?,
            );
            let speed = self.mover.as_ref().map_or(0.0, |m| m.stats.speed);
            let engagement = crate::combat::attack::engagement(
                self.ai.as_ref(),
                speed,
                master,
                me_at,
                them,
                c.world_time,
            )?;
            let moving = hunt
                .task
                .walk_sprite_id(self.life.attribute_type, self.life.amount);
            let arms = crate::combat::attack::arms(master, c.graphics, hunt.task.sprites, moving);
            let hunt = self.hunt.as_mut().expect("the caller's hunt");
            let site = std::mem::replace(&mut hunt.carry_to, -1);
            let by = self.releaser(c);
            if site != -1 {
                c.releases.push((site, by));
            }
            let (Some(hunt), Some(mover)) = (self.hunt.as_mut(), self.mover.as_mut()) else {
                return Err(GenieError::unsupported(
                    "a hunter without its hunt or movement state",
                ));
            };
            if hunt.chase(&quarry, c.player, &engagement, arms, mover)? {
                return Err(GenieError::unsupported(
                    "a hunt's Attack whose MoveTo starts arrived after its drop-off",
                ));
            }
            if hunt.state == crate::gathering::hunt::FAILED {
                return Err(GenieError::unsupported(
                    "a hunt failing on its target after its drop-off",
                ));
            }
            return Ok(());
        }
        let ai = self.ai.as_mut().ok_or(GenieError::unsupported(
            "a hunt without a unit AI after its drop-off",
        ))?;
        let spatial: &Spatial = c.spatial;
        let gone = |id: i32| match spatial.get(id) {
            None => true,
            Some(_) => spatial
                .sight
                .as_ref()
                .and_then(|v| v.seer(id))
                .is_some_and(|s| 5 < s.state),
        };
        ai.orders
            .retain(|o| !(matches!(o.order, 0x2c5 | 0x2c9 | 0x2db) && gone(o.target)));
        if !ai.orders.is_empty() {
            // the AI's order and action clear (FUN_140c64b60, FUN_140c64b50), the hunt ends
            // (set_state 1; its update returns 1 and the action list removes it): its unit AI
            // runs the queued order
            ai.end_order_and_action()?;
            let mut hunt = self.hunt.take().expect("the caller's hunt");
            hunt.tear_down(self.mover.as_mut())?;
            // the base teardown lets the drop site go (its target is none by now)
            self.hunt_release(c, [hunt.target, hunt.carry_to]);
            return Ok(());
        }
        let class = if ai.target_type == -1 {
            i32::from(hunt.target_class)
        } else {
            ai.target_type
        };
        if ai.ai_type != crate::unit_ai::CIVILIAN {
            return Err(GenieError::unsupported(
                "slot 0x1b0 of another unit AI than the civilian's",
            ));
        }
        let kind = crate::unit_ai::civilian_kind(class);
        let found = if kind == -1 && hunt.target_class == 10 {
            // a hunt of class 10 searches nothing
            -1
        } else if kind == -1 {
            if self.my_pasture != -1 {
                return Err(GenieError::unsupported(
                    "a hunter tending a pasture (FUN_1411c7670)",
                ));
            }
            let mover = self.mover.as_ref().ok_or(crate::turn::HUNTER_UNMOVING)?;
            let radius =
                crate::unit_ai::civilian_search_radius(master.line_of_sight, master.search_radius);
            let search = Search {
                at: [self.position[0], self.position[1]],
                radius,
                classes: &crate::unit_ai::CIVILIAN_CARED,
                size: master.radius,
                pick: crate::unit_ai::Pick::Closest {
                    group: class,
                    relation: -1,
                    state: -1,
                    master: -1,
                    workers: if class == 49 { 1 } else { -1 },
                    spared: -1,
                    reach: true,
                },
                memory: crate::unit_ai::Memory::default(),
            };
            let me = Me {
                id: self.id,
                position: self.position,
                group: self.group.id,
                action_state: hunt.state,
            };
            let mut plan = Planning {
                pathing: c.pathing,
                now: c.world_time,
                computer: c.computer,
                object: c.streams.get(Stream::Object),
                construction_rate: c.construction_rate,
                groups: c.groups,
            };
            let master_id = self.life.master_id;
            let mut reach = |id: i32, range: f32| {
                mover.reaches_object(&me, spatial, &mut plan, id, range, master_id)
            };
            let mut no_attack = crate::unit_ai::NoAttack;
            let foreign = c.foreign;
            let moving = |id: i32| herded(others, foreign, id);
            let shuns = |_: &Placed| -> Result<bool, GenieError> {
                Err(GenieError::unsupported(
                    "a class-10 searcher in a hunt's search",
                ))
            };
            let farm = |id: i32| {
                others
                    .iter()
                    .find(|u| u.id == id)
                    .or_else(|| foreign.find(id).map(|(_, u)| u))
                    .map(|u| (u.workers, u.pasture))
            };
            let result = spatial.search(
                self.id,
                c.player,
                &search,
                &mut no_attack,
                &mut reach,
                &moving,
                &shuns,
                &farm,
            )?;
            // FUN_140c63e20's alert: enemies found set the alert mode and its time again
            ai.alert(result, c.dt_seconds);
            result.id
        } else {
            // the unit AI's ClosestResourceObject (slot 0x80) of the kind, at most 2 workers for
            // wood, no drop site, state 2, no point (the probe of 4533's call)
            let cap = if kind == 0 { 2 } else { -1 };
            self.resource_search(c, others, kind as u8, cap)?
        };
        if found == -1 {
            // LAB_140ac9509 with nothing found: a computer player's hunt of a kind but wood
            // would end (set_state 1); one with its point walks there to search again
            // (set_state 11, its second target kept); without a point it would wait
            // (set_state 2)
            if c.computer && kind != 0 {
                return Err(GenieError::unsupported(
                    "a computer player's hunt that found nothing after its drop-off (set_state 1)",
                ));
            }
            let hunt = self.hunt.as_mut().expect("the caller's hunt");
            if hunt.point[0] == -1.0 || hunt.point[1] == -1.0 {
                return Err(GenieError::unsupported(
                    "a hunt without a point that found nothing after its drop-off (set_state 2)",
                ));
            }
            let mover = self.mover.as_mut().ok_or(crate::turn::HUNTER_UNMOVING)?;
            return hunt.search_walk(mover);
        }
        let foreign = c.foreign;
        let (owner, t) = match others.iter().find(|u| u.id == found) {
            Some(t) => (c.player, t),
            None => foreign.find(found).ok_or(GenieError::unsupported(
                "a hunt's new target the slice does not hold",
            ))?,
        };
        let t_master = c
            .every_masters
            .get(owner)
            .and_then(|m| m.get(t.life.master_id))
            .ok_or(GenieError::unsupported(
                "a hunt's target of a master the slice does not have",
            ))?;
        let relations = c
            .spatial
            .relations
            .get(c.player)
            .map_or(&[][..], |r| &r[..]);
        if matches!(t.life.master_id, 1893 | 1897) {
            return Err(GenieError::unsupported(
                "a hunt finding a farm (FUN_1411c7670)",
            ));
        }
        let quarry = crate::gathering::hunt::Quarry {
            id: found,
            position: t.position,
            class: t_master.class,
            kind: t_master.kind,
            state: t.life.state,
            player: owner,
            amount: t.life.amount,
            allied: crate::tasks::allied(relations, c.player, owner, true),
            top_action: t.top_action_kind(),
            drop_site: false,
        };
        let (me_at, them) = (
            c.spatial
                .get_known(self.id)?
                .ok_or(GenieError::unsupported("a hunter without a place"))?,
            c.spatial
                .get_known(found)?
                .ok_or(GenieError::unsupported("a hunt's target without a place"))?,
        );
        let speed = self.mover.as_ref().map_or(0.0, |m| m.stats.speed);
        let engagement = crate::combat::attack::engagement(
            self.ai.as_ref(),
            speed,
            master,
            me_at,
            them,
            c.world_time,
        )?;
        let (sprites, moving) = self.hunt.as_ref().map_or(([-1; 4], -1), |h| {
            let moving = h
                .task
                .walk_sprite_id(self.life.attribute_type, self.life.amount);
            (h.task.sprites, moving)
        });
        let arms = crate::combat::attack::arms(master, c.graphics, sprites, moving);
        let drops_here = master.drop_sites.contains(&t.life.master_id);
        let held = self.life.amount;
        let by = self.releaser(c);
        let (Some(hunt), Some(mover)) = (self.hunt.as_mut(), self.mover.as_mut()) else {
            return Err(GenieError::unsupported(
                "a hunter without its hunt or movement state",
            ));
        };
        if let Some(arrived) = hunt.retarget(
            &quarry,
            c.player,
            held,
            drops_here,
            &engagement,
            arms,
            mover,
        )? {
            if arrived {
                crate::combat::attack::show_arrived(
                    &arms,
                    &mut self.sprite,
                    &mut self.anim_speed,
                    c.streams.get(Stream::Sprite),
                )?;
            }
            if owner == c.player {
                // one of its own player's: allied, never listed
                let u = others
                    .iter_mut()
                    .find(|u| u.id == found)
                    .expect("found above");
                u.taken(false, by.id);
            } else {
                c.takes.push((found, by));
            }
        }
        if hunt.state == crate::gathering::hunt::FAILED
            && let Some(m) = self.mover.as_mut()
        {
            m.shows_standing();
        }
        if hunt.state == crate::gathering::hunt::FAILED && self.sprite.is_some() {
            let standing =
                c.graphics
                    .get(&master.standing_sprite)
                    .copied()
                    .ok_or(GenieError::unsupported(
                        "a hunter without its standing sprite",
                    ))?;
            crate::combat::attack::set_sprite(
                &mut self.sprite,
                standing,
                c.streams.get(Stream::Sprite),
            );
        }
        Ok(())
    }

    /// The hunt's `set_state(8)` after its gathering (`0x140ac60e0` case 8): a unit that carries
    /// nothing goes back to state 3 (not modelled). Without a drop site, its master's drop sites
    /// (`+0x7a0`; none when the first is −1) are searched among its player's objects, from the
    /// unit by path ([`drop_site_search`] with the unit), else by distance from its place (without
    /// it); none found is not modelled (notify 0x203, `set_state` 2). The drop site found is the
    /// second target (`FUN_140a7a190`, which takes it: slot 0x178 after the update) and a quiet
    /// `MoveTo` to it starts ([`HuntAction::set_drop`]).
    ///
    /// [`HuntAction::set_drop`]: crate::gathering::hunt::HuntAction::set_drop
    /// [`drop_site_search`]: crate::turn::world::drop_site_search
    pub(crate) fn hunt_drop_off(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), GenieError> {
        let w = DropWorld {
            masters: c.masters,
            spatial: c.spatial,
            zones: c.zones,
            pathing: c.pathing,
            object: c.streams.get(Stream::Object),
            world_time: c.world_time,
            computer: c.computer,
            construction_rate: c.construction_rate,
            groups: c.groups,
        };
        if let Some(site) = self.drop_off_in(w, others)? {
            c.takes.push((site, self.releaser(c)));
        }
        Ok(())
    }
}
