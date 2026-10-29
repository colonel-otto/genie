//! The builder's part of its update: what the Build action reads as it starts (its
//! foundation, the foundation's annexes and main building, the builder and its unit AI's
//! orders), the action's update on the Build copied out of the action list, and the work's
//! part that runs on the world: the foundation's work, its starts and its completion.

use crate::objects::unit::object_master;
use std::collections::BTreeMap;

use crate::GenieError;
use crate::construction::build::{
    AnnexStart, BuildAction, BuildReads, Built, ClearLook, Completion, Construction, DROP,
    DROPPED_ORDERS, FOREIGN_SITE, MainWork, NO_CONSTRUCTION, Site, WorkChain, Worked,
};
use crate::gathering::hunt::DropOff;
use crate::map::spatial::Spatial;
use crate::math::rng::Stream;
use crate::movement::move_to::MoveDefaults;
use crate::movement::{Me, footprint_gap_squared};
use crate::objects::lifecycle::death_parts::{clear_look_of, farm_bonus};
use crate::objects::object_list::ObjectList;
use crate::objects::teardown::HeadCode;
use crate::objects::unit::{TAKEN_OUT, Unit};
use crate::unit_ai::OrderKind;
use crate::unit_ai::task_finder::Actor;
use crate::unit_ai::task_finder::world::state_of;
use crate::world::others::OthersRef;
use crate::world::step::Step;
use crate::world::{BUILDING_KIND, DROP_SITE_MASTERS, World};

impl Unit {
    /// The Build's update ([`BuildAction::update`]) with what it reads as it starts
    /// ([`Unit::build_reads`]), on the Build copied out of the action list and put back after
    /// it; its code (the end's 1). Nothing without a Build.
    pub(crate) fn build_part(&mut self, c: &mut Step<'_>, me: Me) -> Result<HeadCode, GenieError> {
        let Some(mut build) = self.actions.build().copied() else {
            return Ok(HeadCode::Keeps);
        };
        let reads = self.build_reads(c, build)?;
        let built = build.update(c, self, me, &reads);
        if let Some(b) = self.actions.build_mut() {
            *b = build;
        }
        // its end's 1 deletes it; the destructor (FUN_140aadbf0) finds it out of the builders
        // (set_state left them) and lets go the foundation and the second target it still holds
        built
    }

    /// What the footprint clearing of the Build's `set_state(7)` reads (`FUN_1415c3620`) of the
    /// objects on the tiles about a desolid foundation, as the update starts: none for a builder
    /// in another state or at another foundation.
    fn clearing_looks(
        &self,
        c: &Step<'_>,
        others: OthersRef<'_>,
        action: BuildAction,
    ) -> BTreeMap<i32, ClearLook> {
        let mut looks = BTreeMap::new();
        if let Some(f) = Some(action)
            .filter(|b| matches!(b.state, 6 | 222))
            .and_then(|b| others.own(b.target))
            .filter(|u| u.construction.is_some_and(|k| k.desolid))
        {
            let [fx, fy, _] = f.position();
            let reach = crate::construction::build::CLEARING_REACH + 1.0;
            for (id, _) in
                c.w.spatial
                    .objects_within(fx - reach, fy - reach, fx + reach, fy + reach)
            {
                let (owner, u) = if id == self.id {
                    (c.player, self)
                } else if let Some(found) = others.find(id) {
                    found
                } else {
                    continue;
                };
                if let Some(l) = clear_look_of(u, c.w.masters.get(owner)) {
                    let _ = looks.insert(id, l);
                }
            }
        }
        looks
    }

    /// What the Build's update reads of the world as it starts ([`BuildReads`]).
    pub(super) fn build_reads(
        &self,
        c: &Step<'_>,
        action: BuildAction,
    ) -> Result<BuildReads, GenieError> {
        let others = OthersRef::of(&c.w.players, c.player);
        let target = action.target;
        let foundation = others.own(target);
        let site = foundation
            .filter(|u| u.construction.is_some())
            .map(|u| u.id);
        let built = foundation.map(|u| u.life.master_id);
        let stack_made = foundation.and_then(|u| u.stack_made);
        let walk_target_state = others.get(action.walk_target).map(|u| u.life.state);
        let build_targets: Vec<(i32, Option<u8>)> = self.ai.as_ref().map_or_else(Vec::new, |ai| {
            ai.orders
                .iter()
                .filter(|o| o.order == OrderKind::Build)
                .map(|o| {
                    let state = others.own(o.target).map(|u| u.life.state);
                    (o.target, state)
                })
                .collect()
        });
        let (deposit, gone_orders) = if action.state == DROP {
            (
                self.build_deposit(c, others, action.site)?,
                self.gone_drop_orders(&c.w.spatial),
            )
        } else {
            (None, Vec::new())
        };
        let master = self.master(&c.w.masters, c.player);
        // FUN_140aadc50 drops only a kind the player has (FUN_140efe5b0)
        let attributes = c.w.players[c.player].attributes.len();
        let has_kind = usize::try_from(self.life.attribute_type).is_ok_and(|k| k < attributes);
        Ok(BuildReads {
            site,
            built,
            stack_made,
            walk_target_state,
            build_targets,
            deposit,
            gone_orders,
            main_work: main_work_of(others, target)?,
            looks: self.clearing_looks(c, others, action),
            // the sprites its MoveTos' constructors take for none (FUN_140aa0d30, FUN_140aa0df0)
            defaults: master
                .as_ref()
                .ok()
                .map_or_else(MoveDefaults::default, |m| {
                    MoveDefaults::of(m, self.abilities.look.as_ref())
                }),
            class: master.map_or(-1, |m| m.class),
            drop_site: drop_site_master(c, built) && has_kind,
            annexed: others.own_iter().any(|u| u.annex_of == Some(target)),
            annex_ids: c.w.spatial.annexes_of(target),
            by: self.releaser(c),
        })
    }

    /// State 8: what the drop-off reads of its drop site `site`: its state, whether it takes
    /// what the builder carries and the gap between their footprints; none without the site.
    fn build_deposit(
        &self,
        c: &Step<'_>,
        others: OthersRef<'_>,
        site: i32,
    ) -> Result<Option<(u8, DropOff)>, GenieError> {
        let Some(s) = c.w.spatial.get(site).filter(|_| site != -1) else {
            return Ok(None);
        };
        let me_at =
            c.w.spatial
                .get_known(self.id)?
                .ok_or(GenieError::unsupported("a builder without a place"))?;
        let master = self.master(&c.w.masters, c.player)?;
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
            civ: c.w.players[c.player].civ,
        };
        Ok(state_of(site, others).map(|state| {
            let drop = DropOff {
                takes: crate::unit_ai::task_finder::drops_at(&c.w.drop_sites, s.master, &actor),
                gap_squared: footprint_gap_squared(
                    me_at.position,
                    me_at.radius,
                    s.position,
                    s.radius,
                ),
            };
            (state, drop)
        }))
    }

    /// State 8: the targets of the unit AI's orders that a drop-off drops, gone from the world
    /// or past state 5 as the player sees them.
    fn gone_drop_orders(&self, spatial: &Spatial) -> Vec<i32> {
        let gone = |t: i32| match spatial.get(t) {
            None => true,
            Some(_) => spatial
                .sight
                .as_ref()
                .and_then(|v| v.seer(t))
                .is_some_and(|s| 5 < s.state),
        };
        self.ai.as_ref().map_or_else(Vec::new, |ai| {
            ai.orders
                .iter()
                .filter(|o| DROPPED_ORDERS.contains(&o.order))
                .map(|o| o.target)
                .filter(|&t| gone(t))
                .collect()
        })
    }
}

impl BuildAction {
    /// Runs `f` on the world with the builder lent back to its slot ([`Step::lend`]: the
    /// world's routines that scan the objects meet it, as the game's do), its Build as this
    /// update left it so far; after `f` the Build is the one `f` left.
    ///
    /// # Errors
    ///
    /// `f`'s and the lend's.
    pub(super) fn lend<R>(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        f: impl FnOnce(&mut World) -> Result<R, GenieError>,
    ) -> Result<R, GenieError> {
        if let Some(b) = builder.actions.build_mut() {
            *b = *self;
        }
        let answer = c.lend(builder, f);
        if let Some(b) = builder.actions.build() {
            *self = *b;
        }
        answer
    }

    /// `FUN_1411cc430`'s `rate` seconds of work on the foundation ([`Construction::work`] on
    /// its [`Site`], built here), then at once what the work started and completed: each
    /// construction that started (its annexes' first) its owner watches (`FUN_1411e5670`'s
    /// `FUN_141090430`), the main building of an annex takes the work
    /// ([`World::work_building`] with the builder lent) and the completion's parts follow
    /// ([`BuildAction::completed`]). Returns what the work did.
    ///
    /// # Errors
    ///
    /// Another player's foundation, and what the work, the watches and the completion do not
    /// model.
    pub(super) fn work_on(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        reads: &BuildReads,
        rate: f32,
    ) -> Result<Worked, GenieError> {
        let target = self.target;
        let worked = work_site(c, target, reads, rate)?;
        for id in reads.annex_ids.iter().copied().chain([target]) {
            if let Some(cons) = c.w.players[c.player]
                .objects
                .by_id_mut(id)
                .and_then(|u| u.construction.as_mut())
                .filter(|cons| cons.started)
            {
                cons.started = false;
                c.w.watch_started(c.player, id)?;
            }
        }
        match &worked {
            Worked::Main { amount, .. } => {
                let amount = *amount;
                self.lend(c, builder, |w| w.work_building(target, amount))?;
            }
            Worked::Completed(built) => self.completed(c, builder, built.clone())?,
            Worked::Going | Worked::Done => {}
        }
        Ok(worked)
    }

    /// The completion's parts after [`Construction::complete`]: its `set_state(2)` takes its
    /// annexes along (`FUN_1411dbb70`: their slot 0xa0, [`World::set_state`]), the first
    /// part's tracker part (`FUN_1411c6bd0` -> `FUN_14104eac0`), its standing sprite
    /// ([`crate::construction::build::show_built`]) and what it leaves its owner
    /// ([`World::built`]), at once and in the game's order, the builder lent to the world for
    /// the searches and effects; the start's watch is [`BuildAction::work_on`]'s.
    ///
    /// # Errors
    ///
    /// What an annex's `set_state` and the completion's outputs do not model.
    fn completed(
        &mut self,
        c: &mut Step<'_>,
        builder: &mut Unit,
        built: Built,
    ) -> Result<(), GenieError> {
        let target = self.target;
        for annex in c.w.spatial.annexes_of(target) {
            if let Some((q, _)) = c.w.object(annex) {
                self.lend(c, builder, |w| w.set_state(q, annex, 2))?;
            }
        }
        let p = c.player;
        if let Some(master_id) = c.w.players[p]
            .objects
            .by_id(target)
            .map(|u| u.life.master_id)
        {
            let k = crate::players::tracker::key(c.w.masters.get(p), &c.w.lines, master_id);
            c.w.players[p].tracker.complete(target, k);
            // its set_state(2)'s standing sprite (FUN_1411dbb70)
            let World {
                players,
                masters,
                graphics,
                streams,
                ..
            } = &mut *c.w;
            if let Some(u) = players[p].objects.by_id_mut(target)
                && let Ok(m) = object_master(
                    u.own_master.as_deref(),
                    (u.id, u.life.master_id),
                    masters.get(p),
                )
            {
                crate::construction::build::show_built(
                    &mut u.sprite,
                    u.body.move_stats_mut(),
                    m,
                    u.build_frame,
                    graphics,
                    streams.get(Stream::Sprite),
                );
            }
        }
        let built = Built {
            started: false,
            ..built
        };
        self.lend(c, builder, |w| w.built(p, target, built))
    }
}

/// [`Construction::work`] of `rate` seconds on the foundation `target` of the builder's
/// player, its [`Site`] built from the world as it stands ([`build_site`]).
///
/// # Errors
///
/// A foundation the player does not hold, and what the work does not model.
fn work_site(
    c: &mut Step<'_>,
    target: i32,
    reads: &BuildReads,
    rate: f32,
) -> Result<Worked, GenieError> {
    let p = c.player;
    let (attribute_204, now) = (c.w.attribute_204(), c.w.world_time);
    let World {
        players,
        masters,
        spatial,
        pathing,
        mode_680,
        ..
    } = &mut *c.w;
    let template = Completion {
        master: None,
        annexed: reads.annexed,
        attribute_204,
        mode_680: *mode_680,
        farm_bonus: farm_bonus(&players[p].attributes),
        masters: masters.get(p),
        computer: players[p].computer,
        ai: false,
        main: false,
        now,
        rice: false,
    };
    let own = &mut players[p].objects;
    let mut site = build_site(own, target, &reads.annex_ids, template, reads.main_work)?
        .ok_or(FOREIGN_SITE)?;
    Construction::work(&mut site, rate, spatial, pathing, now)
}

/// `FUN_1411cc430`'s way to the building that takes the work: while the foundation `target` is
/// an annex of a main building in state 0 with at least 1 hit point (COMISS: a NaN too), that
/// main building, the last one found.
fn main_work_of(others: OthersRef<'_>, target: i32) -> Result<Option<MainWork>, GenieError> {
    let mut worked = target;
    let mut main_work = None;
    while let Some(m) = others
        .own(worked)
        .and_then(|u| u.annex_of)
        .and_then(|m| others.own(m))
        .filter(|m| m.life.state == 0 && crate::math::fmath::at_most_or_unordered(1.0, m.life.hp))
    {
        worked = m.id;
        main_work = Some(MainWork {
            construction: m.construction.ok_or(NO_CONSTRUCTION)?,
            hp: m.life.hp,
        });
    }
    Ok(main_work)
}

/// The foundation `target` as the Build's work works it ([`Site`]): its construction and its
/// completion (`template` with the foundation's own master, unit AI, main building and rice),
/// the foundation's annexes of a building type (`annex_ids`), whose work starts first (none when
/// the work goes on to a main building, `main_work`). None when this player does not hold the
/// foundation.
fn build_site<'u>(
    own: &'u mut ObjectList,
    target: i32,
    annex_ids: &[i32],
    template: Completion<'u>,
    main_work: Option<MainWork>,
) -> Result<Option<Site<'u>>, GenieError> {
    let masters = template.masters;
    let mut site = None;
    let mut starts = Vec::new();
    for u in own.iter_mut().filter(|u| u.id != TAKEN_OUT) {
        if u.id == target {
            site = Some(u);
            continue;
        }
        if main_work.is_some() || !annex_ids.contains(&u.id) {
            continue;
        }
        let Some(am) = object_master(u.own_master.as_deref(), (u.id, u.life.master_id), masters)
            .ok()
            .filter(|m| BUILDING_KIND <= m.kind)
        else {
            continue;
        };
        let construction = u.construction.as_mut().ok_or(NO_CONSTRUCTION)?;
        starts.push(AnnexStart {
            id: u.id,
            construction,
            mover: u.body.mover_mut(),
            firepower: &mut u.firepower,
            volley: am.volley_fire,
        });
    }
    // the slot order: the constructor made them, and their ids, in it
    starts.sort_by_key(|a| a.id);
    Ok(site.and_then(|u| {
        let master = object_master(u.own_master.as_deref(), (u.id, u.life.master_id), masters).ok();
        let (ai, main, rice) = (u.ai.is_some(), u.annex_of.is_some(), u.rice);
        Some(Site {
            id: u.id,
            life: &mut u.life,
            construction: u.construction.as_mut()?,
            mover: u.body.mover_mut(),
            firepower: Some(&mut u.firepower),
            roo_called: Some(&mut u.roo_called),
            store: Some(&mut u.store),
            completion: Completion {
                master,
                ai,
                main,
                rice,
                ..template
            },
            chain: WorkChain {
                annexes: starts,
                main: main_work,
            },
        })
    }))
}

/// `FUN_140fde180`: the foundation's master `built` is a drop site (the masters 562, 584, 621,
/// 68, 1808, 50, 1187 and 199, or one the drop-site table has for every civ or the builder's).
fn drop_site_master(c: &Step<'_>, built: Option<i16>) -> bool {
    built.is_some_and(|m| {
        DROP_SITE_MASTERS.contains(&m)
            || c.w.drop_sites.iter().any(|d| {
                d.building == m
                    && (d.civs.is_empty() || d.civs.contains(&c.w.players[c.player].civ))
            })
    })
}
