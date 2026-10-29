//! A builder's and a gatherer's drop-off and a gatherer's update and search.

use crate::GenieError;
use crate::combat::victims::{readable, victim, write_back};
use crate::gathering::hunt::HuntSprites;
use crate::math::rng::{Stream, Well512};
use crate::movement::{Me, footprint_gap, footprint_gap_squared};
use crate::objects::release::Releaser;
use crate::objects::sprite::sprite_id;
use crate::objects::teardown::HeadCode;
use crate::objects::unit::{Unit, second_target_checked};
use crate::unit_ai::ActionKind;
use crate::unit_ai::task_finder::Actor;
use crate::unit_ai::task_finder::world::state_of;
use crate::unit_ai::world_view::DropWorld;
use crate::world::context::Context;
use crate::world::others::OthersRef;

impl Unit {
    /// The Build's `set_state(8)` from its update (case 7 searching again): the drop-site table's
    /// nearest drop site for what the unit carries ([`crate::orders::nearest_table_site`] over
    /// its player's objects; the sleeping ones, static only, are never drop sites) and the task
    /// its slot 0x820 finds on it ([`task_search`] with any action type, no switch: its range,
    /// else the Build task's), unless it carries nothing (COMISS: a NaN load neither); then
    /// [`crate::construction::build::BuildAction::set_drop`], whose second targets the caller releases and
    /// takes and whose notify 0x203 goes to the unit AI. A builder of state 8 counts among no
    /// builders (its `set_state(8)` left them).
    pub(crate) fn build_drop_off(&mut self, c: &mut Context<'_>, me: Me) -> Result<(), GenieError> {
        use crate::construction::build::DropSearch;
        let by = self.releaser(c);
        let build = self.actions.build().expect("the caller's Build");
        let task_range = build.task.range;
        let view = OthersRef::of(&c.w.players, c.player);
        let search = if 0.0 < self.life.amount {
            let (masters, own) =
                c.w.masters
                    .get(c.player)
                    .and_then(|m| Some((m, m.get(self.life.master_id)?)))
                    .ok_or(crate::world::OBJECT_MASTER)?;
            let at = [self.position()[0], self.position()[1]];
            let look = |id: i32| view.own(id).map(|u| (u.life.state, u.position()));
            let found = crate::orders::nearest_table_site(
                (&c.w.drop_sites, c.civ, &c.w.players[c.player].tracker),
                (at, self.life.attribute_type, own.class),
                look,
            );
            match found {
                None => DropSearch::Nothing,
                Some(id) => {
                    let (player, foreign) = (c.player, view.players());
                    let unit = &*self;
                    let find = |t: i32| {
                        if t == unit.id {
                            return Some((player, unit));
                        }
                        view.find(t)
                    };
                    let every = &c.w.masters;
                    let task = match (
                        crate::unit_ai::task_finder::world::task_target_with(id, 1, &find, every)?,
                        crate::unit_ai::task_finder::world::task_target_with(id, 2, &find, every)?,
                    ) {
                        (Some(t), Some(general)) => {
                            use crate::unit_ai::task_finder::world::SideWorld;
                            use crate::unit_ai::task_finder::world::task_search;
                            let actor = Actor {
                                id: self.id,
                                master: own,
                                player,
                                held: (self.life.attribute_type, self.life.amount),
                                attribute_95: c.w.players[c.player]
                                    .attributes
                                    .get(95)
                                    .copied()
                                    .unwrap_or(0.0),
                                civ: c.civ,
                            };
                            let dip = SideWorld {
                                player,
                                relations: &c.w.players[c.player].relations,
                                foreign,
                                drop_sites: &c.w.drop_sites,
                            };
                            // slot 0x820's point: x, y and z −1, no terrain
                            task_search(masters, &actor, &t, &general, None, -1, &dip)?
                        }
                        _ => None,
                    };
                    let range = task.map_or(task_range, |(t, _)| t.work_range);
                    let sprite = task.and_then(|(t, _)| sprite_id(t.sprites[3]));
                    DropSearch::Site { id, range, sprite }
                }
            }
        } else {
            DropSearch::Empty
        };
        let mut walk = c.walk(&mut self.body, me);
        let build = self.actions.build_mut().expect("the caller's Build");
        let out = build.set_drop(&mut walk, None, search)?;
        if out.let_go != -1 {
            crate::world::lets::release(c, out.let_go, by)?;
        }
        if out.took != -1 {
            crate::world::lets::take(c, out.took, by);
        }
        if out.notified {
            self.post_own(
                crate::unit_ai::NO_DROP_SITE,
                [ActionKind::Build.value(), 0, 0],
            )?;
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
        me: Me,
    ) -> Result<HeadCode, GenieError> {
        let by = self.releaser(c);
        self.gather_check_targets(c, by)?;
        let view = self.gather_view(c)?;
        if self.actions.gather().expect("the caller's gather").state
            == crate::gathering::gather::DROP
        {
            return self.gather_drop_update(c, me, view, by);
        }
        self.gather_work_update(c, me, view, by)
    }

    /// The Gather's update's check of its targets: a target the world does not hold goes, one
    /// past state 6 is let go and goes (one the slice holds but cannot read is not modelled); a
    /// second target ([`second_target_checked`]) alike.
    fn gather_check_targets(
        &mut self,
        c: &mut Context<'_>,
        by: Releaser,
    ) -> Result<(), GenieError> {
        let gather = self.actions.gather_mut().expect("the caller's gather");
        if gather.target != -1 {
            let state = readable(c, gather.target).map(|(_, u, _)| u.life.state);
            let gone = match state {
                Some(s) => 6 < s,
                None if c.w.spatial.get(gather.target).is_some() => {
                    return Err(GenieError::unsupported(
                        "a gather's target the slice cannot read",
                    ));
                }
                None => true,
            };
            if gone {
                if state.is_some() {
                    crate::world::lets::release(c, gather.target, by)?;
                }
                gather.target = -1;
            }
        }
        let site = gather.carry_to;
        let held = c.w.spatial.get(site).is_some();
        let (kept, let_go) = second_target_checked(
            site,
            held,
            state_of(site, OthersRef::of(&c.w.players, c.player)),
        );
        gather.carry_to = kept;
        if let Some(t) = let_go {
            crate::world::lets::release(c, t, by)?;
        }
        Ok(())
    }

    /// What the Gather's update reads of the gatherer and its player ([`GatherView`]): the
    /// task's sprites and the master's standing one, the master's work rate (`+0x798`) plus the
    /// powerup (slot 0x6e8), its capacity and class, the task's productivity (1 for none or not
    /// above 0), the stance, and the player's farm settings.
    fn gather_view(&self, c: &Context<'_>) -> Result<GatherView, GenieError> {
        let gather = self.actions.gather().expect("the caller's gather");
        let master =
            c.w.masters
                .get(c.player)
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(crate::world::GATHERER_MASTER)?;
        let g = |id: i16| (0 <= id).then(|| c.w.graphics.get(&id).copied()).flatten();
        let task = gather.task;
        Ok(GatherView {
            work_rate: self.work_rate_powerup + master.work_rate,
            capacity: f32::from(master.capacity),
            productivity: usize::try_from(task.productivity)
                .ok()
                .and_then(|i| c.w.players[c.player].attributes.get(i))
                .copied()
                .filter(|&v| 0.0 < v)
                .unwrap_or(1.0),
            sprites: HuntSprites {
                moving: g(task.sprites[0]),
                working: g(task.sprites[2]),
                carrying: g(task.sprites[3]),
                standing: g(master.standing_sprite),
            },
            class: master.class,
            stance: self.ai.as_ref().map_or(0, |ai| ai.stance),
            world_time: c.world_time,
            farm_stops: c.farm_stops,
            player: c.player,
            computer: c.computer,
            stop_productivity: crate::gathering::gather::productivity_of(
                task.productivity,
                &c.w.players[c.player].attributes,
            ),
            food_sold: c.w.players[c.player]
                .attributes
                .get(crate::gathering::gather::FARM_FOOD_SOLD)
                .copied()
                .unwrap_or(0.0),
        })
    }

    /// The Gather's state 8 ([`crate::gathering::gather::GatherAction::update_drop`]): the walk
    /// of the load to its drop site (the site's take and the gap to it, [`Unit::gather_site`]).
    /// A load dropped goes to the player ([`Unit::gather_load_dropped`]); a drop site lost sets
    /// the drop-off again ([`Unit::gather_drop_off`]); an idle gather lets its targets go.
    fn gather_drop_update(
        &mut self,
        c: &mut Context<'_>,
        me: Me,
        view: GatherView,
        by: Releaser,
    ) -> Result<HeadCode, GenieError> {
        use crate::gathering::hunt::Dropped;
        let (position, radius) =
            c.w.spatial
                .get_known(self.id)?
                .map(|p| (p.position, p.radius))
                .ok_or(GenieError::unsupported("a gatherer without a place"))?;
        let carry_to = self.actions.gather().expect("the caller's gather").carry_to;
        let site = self.gather_site(c, carry_to, (position, radius))?;
        let attributes = c.w.players[c.player].attributes.len();
        let gather = self.actions.gather_mut().expect("the caller's gather");
        let (walk, sprite_rng, action_rng) = c.walk_sprites_action(&mut self.body, me);
        let mut work = view.gatherer(
            (&mut self.life.attribute_type, &mut self.life.amount),
            &mut self.sprite,
            self.life.master_id,
            (sprite_rng, action_rng),
        );
        let before = [gather.target, gather.carry_to];
        let dropped = gather.update_drop(walk, &mut work, attributes, site)?;
        let (posted, anim_stopped) = (std::mem::take(&mut work.posted), work.anim_stopped);
        for (kind, p) in posted {
            self.post_own(kind, p)?;
        }
        if anim_stopped {
            self.body.set_anim_speed(0.0);
        }
        match dropped {
            Dropped::Going => Ok(HeadCode::Keeps),
            Dropped::Load(kind, amount) => self.gather_load_dropped(c, by, (kind, amount), radius),
            // set_state(8) again: the drop site kept or searched
            Dropped::Again => self.gather_drop_off(c).map(|()| HeadCode::Keeps),
            // set_state(2): the target, then the drop site go (slot 0x68, FUN_140a7a190)
            Dropped::Idle => {
                for id in before.into_iter().filter(|&id| id != -1) {
                    crate::world::lets::release(c, id, by)?;
                }
                Ok(HeadCode::Keeps)
            }
        }
    }

    /// The drop site `site` as the drop-off reads it (none the world does not place): whether it
    /// takes what the gatherer carries (`FUN_140fdd190`) and the squared gap between the two
    /// footprints (`me`: the gatherer's place and radii).
    fn gather_site(
        &self,
        c: &Context<'_>,
        site: i32,
        (position, radius): ([f32; 3], [f32; 2]),
    ) -> Result<Option<crate::gathering::hunt::DropOff>, GenieError> {
        let Some(s) = c.w.spatial.get_known(site)? else {
            return Ok(None);
        };
        let master =
            c.w.masters
                .get(c.player)
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(crate::world::GATHERER_MASTER)?;
        let actor = Actor {
            id: self.id,
            master,
            player: c.player,
            held: (self.life.attribute_type, self.life.amount),
            attribute_95: c.w.players[c.player]
                .attributes
                .get(95)
                .copied()
                .unwrap_or(0.0),
            civ: c.civ,
        };
        Ok(Some(crate::gathering::hunt::DropOff {
            takes: crate::unit_ai::task_finder::drops_at(&c.w.drop_sites, s.master, &actor),
            gap_squared: footprint_gap_squared(position, radius, s.position, s.radius),
        }))
    }

    /// A load dropped at the drop site (`load`: its kind and amount) goes to the player (its slot
    /// 0xf8 with 1 and 1, [`give_load`], as the hunt's drop-off). Without a target the search
    /// follows ([`Unit::gather_after_drop`]); else the drop site goes (`FUN_140a7a190` with none)
    /// and the gather walks back to its target (`set_state(4)`, the gap from the gatherer's
    /// footprint, `radius` its radii).
    fn gather_load_dropped(
        &mut self,
        c: &mut Context<'_>,
        by: Releaser,
        (kind, amount): (i16, f32),
        radius: [f32; 2],
    ) -> Result<HeadCode, GenieError> {
        c.give_load(kind, amount)?;
        let gather = self.actions.gather_mut().expect("the caller's gather");
        if gather.target == -1 {
            return self.gather_after_drop(c);
        }
        crate::world::lets::release(c, gather.carry_to, by)?;
        gather.carry_to = -1;
        let t =
            c.w.spatial
                .get_known(gather.target)?
                .ok_or(GenieError::unsupported("a gather's target without a place"))?;
        let gap = footprint_gap(self.body.position(), radius, t.position, t.radius);
        let mover = self
            .body
            .mover_mut()
            .ok_or(GenieError::unsupported("a gatherer without movement state"))?;
        let rng = c.w.streams.get(Stream::Action);
        gather
            .set_walk(t.position, t.radius, gap, mover, rng)
            .map(|()| HeadCode::Keeps)
    }

    /// The Gather's other states ([`crate::gathering::gather::GatherAction::update`]) on the
    /// target the caller holds (back in the context after): what it posts goes to the unit AI, a
    /// random wait stops the animation (`FUN_141161850`: its `+0x7b8` 0), a farm's food sold
    /// goes to the player at once ([`give_load`], food), then what follows
    /// ([`Unit::gather_next`]).
    fn gather_work_update(
        &mut self,
        c: &mut Context<'_>,
        me: Me,
        view: GatherView,
        by: Releaser,
    ) -> Result<HeadCode, GenieError> {
        let dt = c.dt_seconds;
        let gather = self.actions.gather_mut().expect("the caller's gather");
        let mut victim = victim(c, gather.target);
        let old = (gather.target, gather.carry_to);
        let (next, posted, sold, anim_stopped) = {
            let (walk, sprite_rng, action_rng) = c.walk_sprites_action(&mut self.body, me);
            let mut work = view.gatherer(
                (&mut self.life.attribute_type, &mut self.life.amount),
                &mut self.sprite,
                self.life.master_id,
                (sprite_rng, action_rng),
            );
            let target = victim.as_mut();
            let next = gather.update(walk, target, &mut work, dt);
            (
                next,
                std::mem::take(&mut work.posted),
                work.sold,
                work.anim_stopped,
            )
        };
        let next = next?;
        // what the work changed on its target, where it ran
        if let Some(v) = victim.as_mut()
            && let Some(k) = write_back(c, v)?
        {
            crate::combat::victims::file_kill(c, k)?;
        }
        for (kind, p) in posted {
            self.post_own(kind, p)?;
        }
        if anim_stopped {
            self.body.set_anim_speed(0.0);
        }
        if 0.0 < sold {
            c.give_load(0, sold)?;
        }
        self.gather_next(c, next, old, by)
    }

    /// What follows the Gather's update (`next`; `old`: its targets before): an end is its code
    /// (state 1's update returns 1, whose deletion lets both targets go); else each target it
    /// changed is let go (slot 0x68, `FUN_140a7a190` with none), then the drop-off, the search, a
    /// build command, or the reseed after the update (state 0xf2's walk ended).
    pub(crate) fn gather_next(
        &mut self,
        c: &mut Context<'_>,
        next: crate::gathering::gather::Next,
        (old_target, old_second): (i32, i32),
        by: Releaser,
    ) -> Result<HeadCode, GenieError> {
        use crate::gathering::gather::Next;
        if next == Next::Done {
            return Ok(HeadCode::Deletes);
        }
        if let Some(g) = self.actions.gather() {
            for (old, new) in [(old_target, g.target), (old_second, g.carry_to)] {
                if old != new && old != -1 {
                    crate::world::lets::release(c, old, by)?;
                }
            }
        }
        match next {
            Next::Drop => self.gather_drop_off(c)?,
            Next::Search => self.gather_search(c)?,
            Next::Build => {
                let target = self.actions.gather().map_or(-1, |g| g.target);
                if c.build_command.replace(target).is_some() {
                    return Err(GenieError::unsupported("two build commands in one update"));
                }
            }
            Next::Reseed { arrived } => c.reseed = Some(arrived),
            Next::Stay | Next::Done => {}
        }
        Ok(HeadCode::Keeps)
    }

    /// The Gather's search in state 3 (`0x140a8cd40` case 3 and its repeat, `0x140a8e1d0` case
    /// 2): the unit's notify 0x202 to its unit AI (slot 0x328) with the gather action (`0x261`),
    /// the Gather's target type (`+0x68`) and a drop site: the second target's, else for a master
    /// with drop sites (`+0x7a0`) the one `FUN_1411671e0` finds from the unit (inside others too:
    /// [`Unit::drop_site_from`]), 0 when it finds none; −1 for a master without.
    pub(crate) fn gather_search(&mut self, c: &mut Context<'_>) -> Result<(), GenieError> {
        let others = OthersRef::of(&c.w.players, c.player);
        let gather = self.actions.gather().expect("the caller's gather");
        let (carry_to, target_type, state) = (gather.carry_to, gather.target_type, gather.state);
        let master =
            c.w.masters
                .get(c.player)
                .and_then(|m| m.get(self.life.master_id))
                .ok_or(crate::world::GATHERER_MASTER)?;
        let site = if carry_to != -1 {
            carry_to
        } else if master.drop_sites.first().is_some_and(|&d| d != -1) {
            let mut w = DropWorld {
                masters: c.w.masters.get(c.player),
                spatial: &c.w.spatial,
                zones: &mut c.w.zones,
                pathing: &mut c.w.pathing,
                object: c.w.streams.get(Stream::Object),
                world_time: c.world_time,
                computer: c.computer,
                construction_rate: c.construction_rate,
                groups: &c.w.groups,
            };
            self.drop_site_from(&mut w, others, state, false)?
                .unwrap_or(0)
        } else {
            -1
        };
        self.post_own(
            crate::unit_ai::REQUIRES_SEARCH,
            [ActionKind::Gather.value(), target_type, site],
        )
    }
}

/// The player's slot 0xf8 (`FUN_141616820`) with 1 and 1 on a unit's load: the resource and its
/// gathered total (166 to 168; gold's 49 and 169) in `adds`, which the caller gives the player; the
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
    let allied = (1..own.len())
        .any(|q| q != player && crate::unit_ai::task_finder::allied(own, player, q, true));
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

/// What the Gather's update reads of the gatherer and its player besides its own state
/// ([`crate::gathering::gather::Gatherer`] without its borrows).
#[derive(Clone, Copy)]
struct GatherView {
    work_rate: f32,
    capacity: f32,
    productivity: f32,
    sprites: HuntSprites,
    class: i16,
    stance: u8,
    world_time: u32,
    farm_stops: bool,
    player: usize,
    computer: bool,
    stop_productivity: f32,
    food_sold: f32,
}

impl GatherView {
    /// The gatherer the Gather's update works with: the held resource, the sprite, the master
    /// and the sprite and action streams.
    const fn gatherer<'a>(
        self,
        (held_kind, held): (&'a mut i16, &'a mut f32),
        sprite: &'a mut Option<crate::objects::sprite::ActiveSprite>,
        master: i16,
        (sprite_rng, action_rng): (&'a mut Well512, &'a mut Well512),
    ) -> crate::gathering::gather::Gatherer<'a> {
        crate::gathering::gather::Gatherer {
            held_kind,
            held,
            work_rate: self.work_rate,
            capacity: self.capacity,
            productivity: self.productivity,
            sprites: self.sprites,
            sprite,
            sprite_rng,
            class: self.class,
            master,
            stance: self.stance,
            world_time: self.world_time,
            farm_stops: self.farm_stops,
            posted: Vec::new(),
            action_rng,
            player: self.player,
            computer: self.computer,
            stop_productivity: self.stop_productivity,
            food_sold: self.food_sold,
            sold: 0.0,
            anim_stopped: false,
        }
    }
}
