//! A gatherer's walk after a drop and the drop sites it walks to.

use crate::GenieError;
use crate::gathering::drop_sites::{Seeker, drop_site_search};
use crate::map::spatial::Spatial;
use crate::math::rng::Stream;
use crate::movement::{Me, Planning, footprint_gap};
use crate::objects::release::Releaser;
use crate::objects::unit::Unit;
use crate::unit_ai::world_view::DropWorld;
use crate::world::context::Context;
use crate::world::others::{Others, OthersRef};

impl Unit {
    /// The Gather's state 8 after the deposit without a target (`0x140a8e1d0` case 7; its drop
    /// site stays): the unit AI drops the queued orders on targets gone (`FUN_140c5de60`) and,
    /// with orders left (`FUN_140c564a0`), ends its order and action (`FUN_140c64b60`,
    /// `FUN_140c64b50`) and the Gather is done (`set_state(1)`: its update returns 1, the list
    /// removes it). Else the gather group of its target class (the AI's `+0x58`, else the
    /// Gather's `mTargetType`; slot 0x1b0, the civilian's) names the unit AI's
    /// `ClosestResourceObject` (slot 0x80: no drop site, any workers, state 2, no point), and the
    /// Gather takes what it finds (slot 0x48, [`crate::gathering::gather::GatherAction::retarget`]). Without
    /// one a Gather with its point would walk there to search again (`set_state(11)`, not
    /// modelled, or 0xf3 for a farmer); without a point a computer player's Gather is done, a
    /// human's idles (`set_state(2)`).
    pub(crate) fn gather_after_drop(
        &mut self,
        c: &mut Context<'_>,
        others: &mut Others<'_>,
    ) -> Result<(), GenieError> {
        let by = self.releaser(c);
        if self.after_drop_orders(c, others, by)? {
            return Ok(());
        }
        let found = self.after_drop_search(c, others.view())?;
        if found == -1 {
            return self.after_drop_nothing(c, others, by);
        }
        self.after_drop_retarget(c, others, found, by)
    }

    /// The unit AI's queued orders after the drop: the orders on targets gone (a target the world
    /// does not hold, or one its sight sees past state 5) leave (`FUN_140c5de60`); with orders
    /// left the AI ends its order and action and the Gather is done (its drop site let go). True
    /// when it is.
    fn after_drop_orders(
        &mut self,
        c: &mut Context<'_>,
        others: &mut Others<'_>,
        by: Releaser,
    ) -> Result<bool, GenieError> {
        let spatial: &Spatial = c.spatial;
        let gone = |id: i32| match spatial.get(id) {
            None => true,
            Some(_) => spatial
                .sight
                .as_ref()
                .and_then(|v| v.seer(id))
                .is_some_and(|s| 5 < s.state),
        };
        let Some(ai) = self.ai.as_mut() else {
            return Ok(false);
        };
        ai.orders
            .retain(|o| !(matches!(o.order, 0x2c5 | 0x2c9 | 0x2db) && gone(o.target)));
        if ai.orders.is_empty() {
            return Ok(false);
        }
        ai.end_order_and_action()?;
        let mut gather = self.actions.take_gather().expect("the caller's gather");
        gather.tear_down(self.body.mover_mut())?;
        if gather.carry_to != -1 {
            crate::world::lets::release(c, others, gather.carry_to, by)?;
        }
        Ok(true)
    }

    /// The unit AI's `ClosestResourceObject` after the drop (slot 0x80: no drop site, any
    /// workers, state 2, no point) for the gather group (slot 0x1b0, the civilian's) of its target
    /// class (the AI's `+0x58`, else the Gather's `mTargetType`); -1 for none, a class of no
    /// group, or a unit without a unit AI.
    fn after_drop_search(
        &self,
        c: &mut Context<'_>,
        others: OthersRef<'_>,
    ) -> Result<i32, GenieError> {
        let gather = self.actions.gather().expect("the caller's gather");
        let Some(ai) = self.ai.as_ref() else {
            return Ok(-1);
        };
        let class = if ai.target_type == -1 {
            gather.target_type
        } else {
            ai.target_type
        };
        if ai.ai_type != crate::unit_ai::CIVILIAN {
            return Err(GenieError::unsupported(
                "slot 0x1b0 of another unit AI than the civilian's",
            ));
        }
        let kind = crate::unit_ai::civilian_kind(class);
        if kind == -1 {
            return Ok(-1);
        }
        self.resource_search(c, others, kind as u8, -1)
    }

    /// Nothing found after the drop: a Gather with its point walks there to search again
    /// (`set_state(11)` within the task's range; a farmer's 0xf3 is not modelled); without one
    /// (`LAB_140a8ed55`) a computer player's Gather is done, a human's idles (`set_state(2)`: its
    /// targets and point cleared, its sprite [`Unit::idle_sprite`]'s). Its drop site is let go.
    fn after_drop_nothing(
        &mut self,
        c: &mut Context<'_>,
        others: &mut Others<'_>,
        by: Releaser,
    ) -> Result<(), GenieError> {
        let farm_waits = crate::gathering::gather::waits_for_a_farm(
            self.life.master_id,
            self.actions.gather().map_or(-1, |g| g.target_type),
        );
        let gather = self.actions.gather_mut().expect("the caller's gather");
        if gather.point[0] != -1.0 && gather.point[1] != -1.0 {
            if farm_waits {
                return Err(GenieError::unsupported(
                    "a farmer that found nothing after its drop-off (set_state 0xf3)",
                ));
            }
            // set_state(11) without a target: to the point within the task's range
            let mover = self
                .body
                .mover_mut()
                .ok_or(GenieError::unsupported("a gatherer without movement state"))?;
            return gather.walk_back(0.0, mover);
        }
        let carry_to = gather.carry_to;
        if c.computer {
            let mut gather = self.actions.take_gather().expect("the caller's gather");
            gather.tear_down(self.body.mover_mut())?;
        } else {
            let shows = self.idle_sprite(c)?;
            let gather = self.actions.gather_mut().expect("the caller's gather");
            gather.tear_down(self.body.mover_mut())?;
            gather.state = crate::gathering::gather::IDLE;
            gather.timer = 0.0;
            gather.target = -1;
            gather.carry_to = -1;
            gather.point = [-1.0; 3];
            if let Some(g) = shows {
                crate::combat::attack::set_sprite(
                    &mut self.sprite,
                    g,
                    c.streams.get(Stream::Sprite),
                );
            }
        }
        if carry_to != -1 {
            crate::world::lets::release(c, others, carry_to, by)?;
        }
        Ok(())
    }

    /// The sprite an idle gatherer shows: its task's carrying sprite while it holds at least 1 of
    /// the task's resource, else its master's standing sprite (none the data file lacks).
    fn idle_sprite(
        &self,
        c: &Context<'_>,
    ) -> Result<Option<crate::objects::sprite::Graphic>, GenieError> {
        let gather = self.actions.gather().expect("the caller's gather");
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(crate::world::GATHERER_MASTER)?;
        let standing = (0 <= master.standing_sprite)
            .then(|| c.graphics.get(&master.standing_sprite).copied())
            .flatten();
        let carrying = (0 <= gather.task.sprites[3])
            .then(|| c.graphics.get(&gather.task.sprites[3]).copied())
            .flatten();
        Ok(match carrying {
            Some(g)
                if gather.task.resource != -1
                    && self.life.attribute_type == gather.task.resource
                    && 1 <= self.life.amount as i32 =>
            {
                Some(g)
            }
            _ => standing,
        })
    }

    /// The Gather takes what the search found (slot 0x48,
    /// [`crate::gathering::gather::GatherAction::retarget`]) with its stop rate; each target it
    /// changed is let go and the new one taken (one of its own player's directly, never listed).
    fn after_drop_retarget(
        &mut self,
        c: &mut Context<'_>,
        others: &mut Others<'_>,
        found: i32,
        by: Releaser,
    ) -> Result<(), GenieError> {
        let (owner, resource) = self.found_resource(c, others.view(), found)?;
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(crate::world::GATHERER_MASTER)?;
        let (computer, farm_stops, held) = (c.computer, c.farm_stops, self.life.amount);
        let rate = crate::gathering::gather::StopRate {
            work_rate: self.work_rate_powerup + master.work_rate,
            capacity: f32::from(master.capacity),
            productivity: crate::gathering::gather::productivity_of(
                self.actions.gather().map_or(-1, |g| g.task.productivity),
                c.attributes,
            ),
        };
        let (Some(gather), Some(mover)) = (self.actions.gather_mut(), self.body.mover_mut()) else {
            return Err(GenieError::unsupported(
                "a gatherer without its gather or movement state",
            ));
        };
        let old = [gather.target, gather.carry_to];
        let rng = c.streams.get(Stream::Action);
        let _ = gather.retarget(
            &resource,
            c.player,
            computer,
            held,
            (farm_stops, rate),
            mover,
            rng,
        )?;
        let new = [gather.target, gather.carry_to];
        for (old, new) in old.into_iter().zip(new) {
            if old == new {
                continue;
            }
            if old != -1 {
                crate::world::lets::release(c, others, old, by)?;
            }
            if new == -1 {
                continue;
            }
            if owner == c.player && new == found {
                // one of its own player's: allied, never listed
                let u = others.own_mut(found).expect("found above");
                u.taken(false, by.id);
            } else {
                let by = self.releaser(c);
                crate::world::lets::take(c, others, new, by);
            }
        }
        Ok(())
    }

    /// What the Gather's retarget reads of the object `found` (its own player's, or another's
    /// the slice holds): its owner, place, class, master, workers and radii, whether it is one of
    /// the gatherer's drop sites, and the footprint gap to it.
    fn found_resource(
        &self,
        c: &Context<'_>,
        others: OthersRef<'_>,
        found: i32,
    ) -> Result<(usize, crate::gathering::gather::Resource), GenieError> {
        let (owner, t) = others.find(found).ok_or(GenieError::unsupported(
            "a gather's new target the slice does not hold",
        ))?;
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(crate::world::GATHERER_MASTER)?;
        let t_master = c
            .every_masters
            .get(owner)
            .and_then(|m| m.get(t.life.master_id))
            .ok_or(GenieError::unsupported(
                "a gather's target of a master the slice does not have",
            ))?;
        Ok((
            owner,
            crate::gathering::gather::Resource {
                id: found,
                position: t.position,
                class: t_master.class,
                master: t_master.id,
                player: owner,
                workers: t.workers,
                drop_site: master.drop_sites.contains(&t.life.master_id),
                gap: footprint_gap(self.position, master.radius, t.position, t_master.radius),
                radius: t_master.radius,
            },
        ))
    }

    /// The Gather's `set_state(8)` (`0x140a8cd40` case 8): the current stop clears; a gatherer
    /// carrying nothing would search (`set_state` 3, not modelled); else its drop site
    /// ([`Unit::drop_site`]) becomes its second target (`FUN_140a7a190`: slot 0x178) and a quiet
    /// `MoveTo` walks there ([`crate::gathering::gather::GatherAction::set_drop`]). Without one the unit AI's
    /// part (notify 0x203) is not modelled.
    pub(crate) fn gather_drop_off(
        &mut self,
        c: &mut Context<'_>,
        others: &mut Others<'_>,
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
        if let Some(site) = self.gather_drop_off_in(w, others.view())? {
            let by = self.releaser(c);
            crate::world::lets::take(c, others, site, by);
        }
        Ok(())
    }

    /// [`Unit::gather_drop_off`] in `w`, from the update or a command: the drop site it took
    /// when it is a new second target (the caller's to take, slot 0x178).
    pub(crate) fn gather_drop_off_in(
        &mut self,
        mut c: DropWorld<'_>,
        others: OthersRef<'_>,
    ) -> Result<Option<i32>, GenieError> {
        if self.life.amount <= 0.0 {
            return Err(GenieError::unsupported(
                "a gather's drop-off carrying nothing (set_state 3)",
            ));
        }
        let gather = self.actions.gather().expect("the caller's gather");
        let (carry_to, state) = (gather.carry_to, gather.state);
        let (site, gap) = self.drop_site(&mut c, others, carry_to, state)?;
        let (Some(gather), Some(mover)) = (self.actions.gather_mut(), self.body.mover_mut()) else {
            return Err(GenieError::unsupported(
                "a gatherer without its gather or movement state",
            ));
        };
        gather.carry_to = site;
        gather.set_drop(gap, mover)?;
        Ok((site != carry_to && site != -1).then_some(site))
    }

    /// [`Unit::hunt_drop_off`] in `w`, from the update or a command: the drop site it took when
    /// it is a new second target (`FUN_140a7a190`, the caller's to take, slot 0x178).
    pub(crate) fn drop_off_in(
        &mut self,
        mut c: DropWorld<'_>,
        others: OthersRef<'_>,
    ) -> Result<Option<i32>, GenieError> {
        if self.life.amount <= 0.0 {
            return Err(GenieError::unsupported(
                "a hunt's drop-off carrying nothing (set_state 3)",
            ));
        }
        let hunt = self.actions.hunt().expect("the caller's hunt");
        let (carry_to, state) = (hunt.carry_to, hunt.state);
        let (site, gap) = self.drop_site(&mut c, others, carry_to, state)?;
        if site == -1 {
            return Err(GenieError::unsupported(
                "a hunt without a drop site (notify 0x203, set_state 2)",
            ));
        }
        let (Some(hunt), Some(mover)) = (self.actions.hunt_mut(), self.body.mover_mut()) else {
            return Err(GenieError::unsupported(
                "a hunter without its hunt or movement state",
            ));
        };
        hunt.carry_to = site;
        hunt.set_drop(gap, mover)?;
        Ok((site != carry_to).then_some(site))
    }

    /// The drop site a gatherer's `set_state(8)` walks to, its second target `carry_to` or, with
    /// none and a master with drop sites (`+0x7a0`), the closest of them (`FUN_1411671e0`: first
    /// one it reaches, [`Unit::drop_site_from`] outside other objects, then any); −1 for none.
    /// With the footprint gap to it (`FUN_14118f680`).
    pub(crate) fn drop_site(
        &self,
        c: &mut DropWorld<'_>,
        others: OthersRef<'_>,
        carry_to: i32,
        state: u8,
    ) -> Result<(i32, f32), GenieError> {
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(crate::world::GATHERER_MASTER)?;
        let mover = self
            .body
            .mover()
            .ok_or(GenieError::unsupported("a gatherer without movement state"))?;
        let mut site = carry_to;
        if site == -1 && master.drop_sites.first().is_some_and(|&d| d != -1) {
            let masters = c.masters.ok_or(GenieError::unsupported(
                "a drop-off of a player without masters",
            ))?;
            let sites = &master.drop_sites;
            site = match self.drop_site_from(c, others, state, true)? {
                Some(s) => Some(s),
                None => drop_site_search(
                    others.own_iter(),
                    masters,
                    sites,
                    self.position,
                    None,
                    (true, crate::gathering::drop_sites::UNPACK),
                )?,
            }
            .unwrap_or(-1);
        }
        if site == -1 {
            return Ok((-1, 0.0));
        }
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        let s = c
            .spatial
            .get_known(site)?
            .ok_or(GenieError::unsupported("a drop site without a place"))?;
        Ok((
            site,
            footprint_gap(self.position, ground, s.position, s.radius),
        ))
    }

    /// `FUN_1411671e0` from the unit (`state` its action's, which the planner reads) over its
    /// master's drop sites (`+0x7a0`): [`drop_site_search`] with the unit as the seeker, by path
    /// for a human player (its argument 9, `FUN_141659cf0` false), skipping objects inside others
    /// when `outside` (its argument 10) and those at work on `0x7d` or `0x7c` (its argument 8).
    pub(crate) fn drop_site_from(
        &self,
        c: &mut DropWorld<'_>,
        others: OthersRef<'_>,
        state: u8,
        outside: bool,
    ) -> Result<Option<i32>, GenieError> {
        let by_path = !c.computer;
        self.drop_site_with(
            c,
            others,
            state,
            (outside, by_path, crate::gathering::drop_sites::UNPACK),
        )
    }

    /// [`Unit::drop_site_from`] with the search's arguments 10, 9 and 8: `outside`, `by_path`
    /// and the action type `at_work` it passes over (−1 none).
    pub(crate) fn drop_site_with(
        &self,
        c: &mut DropWorld<'_>,
        others: OthersRef<'_>,
        state: u8,
        (outside, by_path, at_work): (bool, bool, i16),
    ) -> Result<Option<i32>, GenieError> {
        let masters = c.masters.ok_or(GenieError::unsupported(
            "a drop-off of a player without masters",
        ))?;
        let master = masters
            .get(self.life.master_id)
            .ok_or(crate::world::GATHERER_MASTER)?;
        let mover = self
            .body
            .mover()
            .ok_or(GenieError::unsupported("a gatherer without movement state"))?;
        let [w, h] = c.spatial.map;
        let terrain = c.spatial.terrain.as_ref().ok_or(GenieError::unsupported(
            "the map's zones without its terrain",
        ))?;
        let values = terrain
            .restrictions
            .get(mover.stats.restriction)
            .ok_or(crate::pathing::zones::UNKNOWN_RESTRICTION)?;
        let zones = c.zones.of(w, h, &terrain.kinds, values);
        let me = Me {
            id: self.id,
            position: self.position,
            group: self.group.id,
            action_state: state,
        };
        let mut plan = Planning {
            pathing: &mut *c.pathing,
            now: c.world_time,
            computer: c.computer,
            object: &mut *c.object,
            construction_rate: c.construction_rate,
            groups: c.groups,
        };
        let seeker = Seeker {
            unit: self,
            me,
            mover,
            plan: &mut plan,
            spatial: c.spatial,
            zones,
            class: master.class,
            by_path,
        };
        drop_site_search(
            others.own_iter(),
            masters,
            &master.drop_sites,
            self.position,
            Some(seeker),
            (outside, at_work),
        )
    }
}
