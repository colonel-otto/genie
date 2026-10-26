//! A builder's and a gatherer's drop-off and a gatherer's update and search.

use crate::gathering::hunt::HuntSprites;
use crate::math::rng::Stream;
use crate::movement::{Me, footprint_gap, footprint_gap_squared};
use crate::tasks::Actor;
use crate::turn::unit::{Unit, second_target_checked};
use crate::turn::view::{Context, DropWorld};
use crate::turn::world::state_of;
use crate::unit_ai::{BUILD_ACTION, GenieError};

impl Unit {
    /// The Build's `set_state(8)` from its update (case 7 searching again): the drop-site table's
    /// nearest drop site for what the unit carries ([`crate::order::nearest_table_site`] over
    /// its player's objects; the sleeping ones, static only, are never drop sites) and the task
    /// its slot 0x820 finds on it ([`task_search`] with any action type, no switch: its range,
    /// else the Build task's), unless it carries nothing (COMISS: a NaN load neither); then
    /// [`crate::construction::build::BuildAction::set_drop`], whose second targets the caller releases and
    /// takes and whose notify 0x203 goes to the unit AI. A builder of state 8 counts among no
    /// builders (its `set_state(8)` left them).
    pub(crate) fn build_drop_off(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
        me: Me,
    ) -> Result<(), GenieError> {
        use crate::construction::build::DropSearch;
        let by = self.releaser(c);
        let build = self.build.as_ref().expect("the caller's Build");
        let task_range = build.task.range;
        let search = if 0.0 < self.life.amount {
            let (masters, own) = c
                .masters
                .and_then(|m| Some((m, m.get(self.life.master_id)?)))
                .ok_or(crate::turn::OBJECT_MASTER)?;
            let at = [self.position[0], self.position[1]];
            let look = |id: i32| {
                others
                    .iter()
                    .find(|u| u.id == id)
                    .map(|u| (u.life.state, u.position))
            };
            let found = crate::order::nearest_table_site(
                (c.drop_sites, c.civ, c.tracker),
                (at, self.life.attribute_type, own.class),
                look,
            );
            match found {
                None => DropSearch::Nothing,
                Some(id) => {
                    let (player, foreign) = (c.player, c.foreign);
                    let unit = &*self;
                    let find = |t: i32| {
                        if t == unit.id {
                            return Some((player, unit));
                        }
                        others
                            .iter()
                            .find(|u| u.id == t)
                            .map(|u| (player, u))
                            .or_else(|| foreign.find(t))
                    };
                    let every = c.every_masters;
                    let task = match (
                        crate::turn::world::task_target_with(id, 1, &find, every)?,
                        crate::turn::world::task_target_with(id, 2, &find, every)?,
                    ) {
                        (Some(t), Some(general)) => {
                            use crate::turn::world::{SideWorld, task_search};
                            let actor = Actor {
                                id: self.id,
                                master: own,
                                player,
                                held: (self.life.attribute_type, self.life.amount),
                                attribute_95: c.attributes.get(95).copied().unwrap_or(0.0),
                                civ: c.civ,
                            };
                            let dip = SideWorld {
                                player,
                                relations: c.relations,
                                foreign,
                                drop_sites: c.drop_sites,
                            };
                            task_search(masters, &actor, &t, &general, -1, &dip)?
                        }
                        _ => None,
                    };
                    let range = task.map_or(task_range, |(t, _)| t.work_range);
                    DropSearch::Site { id, range }
                }
            }
        } else {
            DropSearch::Empty
        };
        let mut walk = c.walk(self.mover.as_mut(), me);
        let build = self.build.as_mut().expect("the caller's Build");
        let out = build.set_drop(&mut walk, None, search)?;
        if out.let_go != -1 {
            c.releases.push((out.let_go, by));
        }
        if out.took != -1 {
            c.takes.push((out.took, by));
        }
        if out.notified {
            self.post_own(crate::unit_ai::NO_DROP_SITE, [BUILD_ACTION, 0, 0])?;
        }
        Ok(())
    }

    /// The Gather's update (slot 6, `0x140a8e1d0`). It first checks its targets: a target gone
    /// from the world's table goes (slot 0x68 with none), one past state 6 is let go (slot 0x180)
    /// and goes; a second target gone goes (`FUN_140a7a190` with none), one past state 6 is let
    /// go and goes. Then state 8 walks the
    /// load to its drop site ([`crate::gathering::gather::GatherAction::update_drop`]): the load goes to the
    /// player, the drop site goes (`FUN_140a7a190` with none) and the gather walks back to its
    /// target (`set_state(4)`); without a target the unit AI's search is not modelled. The other
    /// states ([`crate::gathering::gather::GatherAction::update`]) may ask for the drop-off
    /// ([`Unit::gather_drop_off`]).
    pub(crate) fn gather_update(
        &mut self,
        c: &mut Context<'_>,
        others: &mut [Self],
        me: Me,
    ) -> Result<(), GenieError> {
        let by = self.releaser(c);
        let gather = self.gather.as_mut().expect("the caller's gather");
        if gather.target != -1 {
            let state = c
                .victim
                .as_deref()
                .filter(|v| v.id == gather.target)
                .map(|v| v.state);
            let gone = match state {
                Some(s) => 6 < s,
                None if c.spatial.get(gather.target).is_some() => {
                    return Err(GenieError::unsupported(
                        "a gather's target the slice cannot read",
                    ));
                }
                None => true,
            };
            if gone {
                if state.is_some() {
                    c.releases.push((gather.target, by));
                }
                gather.target = -1;
            }
        }
        let site = gather.carry_to;
        let held = c.spatial.get(site).is_some();
        let (kept, let_go) = second_target_checked(site, held, state_of(site, others, c.foreign));
        gather.carry_to = kept;
        if let Some(t) = let_go {
            c.releases.push((t, by));
        }
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(crate::turn::GATHERER_MASTER)?;
        let g = |id: i16| (0 <= id).then(|| c.graphics.get(&id).copied()).flatten();
        let task = gather.task;
        let sprites = HuntSprites {
            moving: g(task.sprites[0]),
            working: g(task.sprites[2]),
            carrying: g(task.sprites[3]),
            standing: g(master.standing_sprite),
        };
        // the gatherer's view: the master's work rate (+0x798) plus the powerup (slot 0x6e8)
        let productivity = usize::try_from(task.productivity)
            .ok()
            .and_then(|i| c.attributes.get(i))
            .copied()
            .filter(|&v| 0.0 < v)
            .unwrap_or(1.0);
        let (work_rate, capacity, class) = (
            self.work_rate_powerup + master.work_rate,
            f32::from(master.capacity),
            master.class,
        );
        let stance = self.ai.as_ref().map_or(0, |ai| ai.stance);
        let (world_time, farm_stops, dt) = (c.world_time, c.farm_stops, c.dt_seconds);
        let stop_productivity =
            crate::gathering::gather::productivity_of(task.productivity, c.attributes);
        let food_sold = c
            .attributes
            .get(crate::gathering::gather::FARM_FOOD_SOLD)
            .copied()
            .unwrap_or(0.0);
        let (player, computer) = (c.player, c.computer);
        if gather.state == crate::gathering::gather::DROP {
            let me_at = c
                .spatial
                .get_known(self.id)?
                .ok_or(GenieError::unsupported("a gatherer without a place"))?;
            let (me_position, me_radius) = (me_at.position, me_at.radius);
            let site = match c.spatial.get_known(gather.carry_to)? {
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
            let (walk, sprite_rng, action_rng) = c.walk_sprites_action(self.mover.as_mut(), me);
            let mut work = crate::gathering::gather::Gatherer {
                held_kind: &mut self.life.attribute_type,
                held: &mut self.life.amount,
                work_rate,
                capacity,
                productivity,
                sprites,
                sprite: &mut self.sprite,
                sprite_rng,
                class,
                master: self.life.master_id,
                stance,
                world_time,
                farm_stops,
                posted: Vec::new(),
                action_rng,
                player,
                computer,
                stop_productivity,
                food_sold,
                sold: 0.0,
                anim_stopped: false,
            };
            let before = [gather.target, gather.carry_to];
            let dropped = gather.update_drop(walk, &mut work, attributes, site)?;
            let (posted, anim_stopped) = (std::mem::take(&mut work.posted), work.anim_stopped);
            for (kind, p) in posted {
                self.post_own(kind, p)?;
            }
            if anim_stopped && let Some(a) = self.anim_speed.as_mut() {
                *a = 0.0;
            }
            let (kind, amount) = match dropped {
                crate::gathering::hunt::Dropped::Going => return Ok(()),
                crate::gathering::hunt::Dropped::Load(kind, amount) => (kind, amount),
                // set_state(8) again: the drop site kept or searched
                crate::gathering::hunt::Dropped::Again => return self.gather_drop_off(c, others),
                // set_state(2): the target, then the drop site go (slot 0x68, FUN_140a7a190)
                crate::gathering::hunt::Dropped::Idle => {
                    for id in before.into_iter().filter(|&id| id != -1) {
                        c.releases.push((id, by));
                    }
                    return Ok(());
                }
            };
            // the player's slot 0xf8 (FUN_141616820) with 1 and 1, as the hunt's drop-off
            give_load(
                (&c.spatial.relations, c.sharing),
                c.player,
                &mut c.attribute_adds,
                kind,
                amount,
            )?;
            let gather = self.gather.as_mut().expect("the caller's gather");
            if gather.target == -1 {
                return self.gather_after_drop(c, others);
            }
            // back to the target: the drop site goes (FUN_140a7a190 with none), set_state(4)
            c.releases.push((gather.carry_to, by));
            gather.carry_to = -1;
            let t = c
                .spatial
                .get_known(gather.target)?
                .ok_or(GenieError::unsupported("a gather's target without a place"))?;
            let gap = footprint_gap(
                self.position,
                [me_radius[0], me_radius[1]],
                t.position,
                t.radius,
            );
            let mover = self
                .mover
                .as_mut()
                .ok_or(GenieError::unsupported("a gatherer without movement state"))?;
            let rng = c.streams.get(Stream::Action);
            return gather.set_walk(t.position, t.radius, gap, mover, rng);
        }
        // the target the caller holds, back in the context once the update is done
        let mut victim = c.victim.take();
        let (old_target, old_second) = (gather.target, gather.carry_to);
        let (next, posted, sold) = {
            let (walk, sprite_rng, action_rng) = c.walk_sprites_action(self.mover.as_mut(), me);
            let mut work = crate::gathering::gather::Gatherer {
                held_kind: &mut self.life.attribute_type,
                held: &mut self.life.amount,
                work_rate,
                capacity,
                productivity,
                sprites,
                sprite: &mut self.sprite,
                sprite_rng,
                class,
                master: self.life.master_id,
                stance,
                world_time,
                farm_stops,
                posted: Vec::new(),
                action_rng,
                player,
                computer,
                stop_productivity,
                food_sold,
                sold: 0.0,
                anim_stopped: false,
            };
            let target = victim.as_deref_mut().filter(|v| v.id == gather.target);
            let next = gather.update(walk, target, &mut work, dt);
            (
                next,
                std::mem::take(&mut work.posted),
                (work.sold, work.anim_stopped),
            )
        };
        let (sold, anim_stopped) = sold;
        c.victim = victim;
        let next = next?;
        for (kind, p) in posted {
            self.post_own(kind, p)?;
        }
        // a random wait stops the unit's animation (FUN_141161850: its +0x7b8 0)
        if anim_stopped && let Some(a) = self.anim_speed.as_mut() {
            *a = 0.0;
        }
        // a farm's food sold at once: the player's slot 0xf8 with 1 (food), as a drop-off's
        if 0.0 < sold {
            give_load(
                (&c.spatial.relations, c.sharing),
                c.player,
                &mut c.attribute_adds,
                0,
                sold,
            )?;
        }
        // the targets an end or an idle let go (slot 0x68, FUN_140a7a190 with none); state 1's
        // update returns 1: the action list removes the Gather, whose teardown lets its targets go
        if next == crate::gathering::gather::Next::Done {
            let g = self.gather.take().expect("the caller's gather");
            for t in [g.target, g.carry_to] {
                if t != -1 {
                    c.releases.push((t, by));
                }
            }
            return Ok(());
        }
        if let Some(g) = self.gather.as_ref() {
            for (old, new) in [(old_target, g.target), (old_second, g.carry_to)] {
                if old != new && old != -1 {
                    c.releases.push((old, by));
                }
            }
        }
        match next {
            crate::gathering::gather::Next::Drop => self.gather_drop_off(c, others)?,
            crate::gathering::gather::Next::Search => self.gather_search(c, others)?,
            crate::gathering::gather::Next::Build => {
                let target = self.gather.as_ref().map_or(-1, |g| g.target);
                if c.build_command.replace(target).is_some() {
                    return Err(GenieError::unsupported("two build commands in one update"));
                }
            }
            // state 0xf2's walk ended: the caller reseeds after the update
            crate::gathering::gather::Next::Reseed { arrived } => c.reseed = Some(arrived),
            crate::gathering::gather::Next::Stay | crate::gathering::gather::Next::Done => {}
        }
        Ok(())
    }

    /// The Gather's search in state 3 (`0x140a8cd40` case 3 and its repeat, `0x140a8e1d0` case
    /// 2): the unit's notify 0x202 to its unit AI (slot 0x328) with the gather action (`0x261`),
    /// the Gather's target type (`+0x68`) and a drop site: the second target's, else for a master
    /// with drop sites (`+0x7a0`) the one `FUN_1411671e0` finds from the unit (inside others too:
    /// [`Unit::drop_site_from`]), 0 when it finds none; −1 for a master without.
    pub(crate) fn gather_search(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
    ) -> Result<(), GenieError> {
        let gather = self.gather.as_ref().expect("the caller's gather");
        let (carry_to, target_type, state) = (gather.carry_to, gather.target_type, gather.state);
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(crate::turn::GATHERER_MASTER)?;
        let site = if carry_to != -1 {
            carry_to
        } else if master.drop_sites.first().is_some_and(|&d| d != -1) {
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
            self.drop_site_from(&mut w, others, state, false)?
                .unwrap_or(0)
        } else {
            -1
        };
        self.post_own(
            crate::unit_ai::REQUIRES_SEARCH,
            [crate::unit_ai::GATHER_ACTION, target_type, site],
        )
    }
}

/// The player's slot 0xf8 (`FUN_141616820`) with 1 and 1 on a unit's load: the resource and its
/// gathered total (166 to 168; gold's 49 and 169), which the caller adds after the update; the
/// statistics (slot 0xb8) and the local player's tracker (`FUN_14097daf0`) are not held. With 1
/// the resources would go to the allies too when the player shares (`sharing`, `+0x1426`,
/// `FUN_14165d040`): a sharing player with allies is not modelled.
pub(crate) fn give_load(
    (relations, sharing): (&[Vec<u8>], bool),
    player: usize,
    adds: &mut Vec<(i16, f32)>,
    kind: i16,
    amount: f32,
) -> Result<(), GenieError> {
    let own = relations.get(player).map_or(&[][..], |r| &r[..]);
    let allied = (1..own.len()).any(|q| q != player && crate::tasks::allied(own, player, q, true));
    if sharing && allied {
        return Err(GenieError::unsupported(
            "a load given to a player with allies (+0x1426)",
        ));
    }
    adds.push((kind, amount));
    match kind {
        0..=2 => adds.push((166 + kind, amount)),
        3 => {
            adds.push((49, amount));
            adds.push((169, amount));
        }
        _ => {}
    }
    Ok(())
}
