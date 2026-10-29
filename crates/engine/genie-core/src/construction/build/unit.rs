//! The builder's part of its update: what the Build action reads of its foundation, the
//! foundation's annexes and main building, the builder and its unit AI's orders; the action's
//! update; and what its step changes in the world.

use crate::objects::unit::object_master;
use crate::unit_ai::ActionKind;
use std::collections::BTreeMap;

use crate::GenieError;
use crate::construction::build::{
    AnnexStart, BuildAction, BuildStep, Builder, ClearLook, Completion, DROP, DROPPED_ORDERS,
    Looks, MainWork, NO_CONSTRUCTION, NOT_HELD, Site, WorkChain,
};
use crate::gathering::hunt::DropOff;
use crate::map::spatial::Spatial;
use crate::math::rng::Stream;
use crate::movement::move_to::{MoveDefaults, Sprites};
use crate::movement::{Me, footprint_gap_squared};
use crate::objects::lifecycle::death_parts::{clear_look_of, farm_bonus};
use crate::objects::object_list::ObjectList;
use crate::objects::release::Releaser;
use crate::objects::teardown::HeadCode;
use crate::objects::unit::{TAKEN_OUT, Unit};
use crate::unit_ai::OrderKind;
use crate::unit_ai::task_finder::Actor;
use crate::unit_ai::task_finder::world::state_of;
use crate::world::context::Context;
use crate::world::others::OthersRef;
use crate::world::{BUILDING_KIND, DROP_SITE_MASTERS, SEARCH_NEEDED, World};

/// What the Build's update reads of the world besides its foundation, as the update starts.
struct BuildReads {
    /// The foundation's master, when this player holds the foundation.
    built: Option<i16>,
    /// The object the foundation's stacking made (`Unit::stack_made`).
    stack_made: Option<i32>,
    /// The state of its `MoveTo`'s target, which the `MoveTo`'s update checks first.
    walk_target_state: Option<u8>,
    /// The foundations its unit AI's queued build orders name, as they stand.
    build_targets: Vec<(i32, Option<u8>)>,
    /// State 8: what the drop-off reads of its drop site.
    deposit: Option<(u8, DropOff)>,
    /// State 8: the targets of its unit AI's orders that the drop-off drops, gone.
    gone_orders: Vec<i32>,
    /// The main building that takes the work instead (`FUN_1411cc430`).
    main_work: Option<MainWork>,
}

impl Unit {
    /// The Build's part of the builder's update (`RGE_Action_Object::update` on its Build, the
    /// action's slot 6) and its code: nothing without a Build.
    pub(crate) fn build_part(
        &mut self,
        c: &mut Context<'_>,
        me: Me,
    ) -> Result<HeadCode, GenieError> {
        let looks = self.clearing_looks(c, OthersRef::of(&c.w.players, c.player));
        let by = self.releaser(c);
        let Some(action) = self.actions.build().copied() else {
            return Ok(HeadCode::Keeps);
        };
        let target = action.target;
        let annexed = OthersRef::of(&c.w.players, c.player)
            .own_iter()
            .any(|u| u.annex_of == Some(target));
        let reads = self.build_reads(c, OthersRef::of(&c.w.players, c.player), action)?;
        let annex_ids = c.w.spatial.annexes_of(target);
        let class = self.master(&c.w.masters, c.player).map_or(-1, |m| m.class);
        // FUN_140aadc50 drops only a kind the player has (FUN_140efe5b0)
        let held_kind = self.life.attribute_type;
        let attributes = c.w.players[c.player].attributes.len();
        let has_kind = usize::try_from(held_kind).is_ok_and(|k| k < attributes);
        let drop_site = drop_site_master(c, reads.built) && has_kind;
        let farm_bonus = farm_bonus(&c.w.players[c.player].attributes);
        let (player, pace) = (c.player, c.pace());
        let (attribute_204, mode_680, computer, now) =
            (c.attribute_204, c.mode_680, c.computer, c.world_time);
        // the sprites its MoveTos' constructors take for none (FUN_140aa0d30, FUN_140aa0df0)
        let defaults = self
            .master(&c.w.masters, c.player)
            .ok()
            .map_or_else(MoveDefaults::default, |m| {
                MoveDefaults::of(m, self.abilities.look.as_ref())
            });
        let build = self.actions.build_mut().expect("above");
        let (walk, beside) = c.w.walk_beside(pace, &mut self.body, me);
        let template = Completion {
            master: None,
            annexed,
            attribute_204,
            mode_680,
            farm_bonus,
            masters: beside.masters.get(player),
            computer,
            ai: false,
            main: false,
            now,
            rice: false,
        };
        let own = &mut beside.players[player].objects;
        let site = build_site(own, target, &annex_ids, template, reads.main_work)?;
        let builder = Builder {
            defaults,
            sprites: Sprites {
                sprite: &mut self.sprite,
                rng: beside.sprite_rng,
                graphics: beside.graphics,
            },
            class,
            master: self.life.master_id,
            held: self.life.amount,
            held_kind,
            drop_site,
            ai: self.ai.as_mut(),
            waits: &mut self.footprint_waits,
            build_targets: &reads.build_targets,
            look: Looks(&|id| looks.get(&id).copied()),
            walk_target_state: reads.walk_target_state,
            deposit: reads.deposit,
            attributes,
            gone_orders: &reads.gone_orders,
            stack_made: reads.stack_made,
        };
        let mut step = build.update(walk, site, builder)?;
        record_build_step(c, self, target, &annex_ids, &mut step)?;
        self.after_build_step(c, me, (target, reads.built, by), step)
    }

    /// What the footprint clearing of the Build's `set_state(7)` reads (`FUN_1415c3620`) of the
    /// objects on the tiles about a desolid foundation, as the update starts: none for a builder
    /// in another state or at another foundation.
    fn clearing_looks(&self, c: &Context<'_>, others: OthersRef<'_>) -> BTreeMap<i32, ClearLook> {
        let mut looks = BTreeMap::new();
        if let Some(f) = self
            .actions
            .build()
            .copied()
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

    /// What the Build's update reads of the world besides its foundation ([`BuildReads`]).
    fn build_reads(
        &self,
        c: &Context<'_>,
        others: OthersRef<'_>,
        action: BuildAction,
    ) -> Result<BuildReads, GenieError> {
        let target = action.target;
        let built = others.own(target).map(|u| u.life.master_id);
        let stack_made = others.own(target).and_then(|u| u.stack_made);
        let walk_target = action.walk_target;
        let walk_target_state = others.get(walk_target).map(|u| u.life.state);
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
        Ok(BuildReads {
            built,
            stack_made,
            walk_target_state,
            build_targets,
            deposit,
            gone_orders,
            main_work: main_work_of(others, target)?,
        })
    }

    /// State 8: what the drop-off reads of its drop site `site`: its state, whether it takes
    /// what the builder carries and the gap between their footprints; none without the site.
    fn build_deposit(
        &self,
        c: &Context<'_>,
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
            civ: c.civ,
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

    /// What the Build's step changes of the builder: the load dropped, its animation, the
    /// targets let go, its notifies, its standing sprite, the work it takes next and its
    /// drop-off; its code (the end's 1). `target` is the foundation, `built` its master.
    pub(crate) fn after_build_step(
        &mut self,
        c: &mut Context<'_>,
        me: Me,
        (target, built, by): (i32, Option<i16>, Releaser),
        step: BuildStep,
    ) -> Result<HeadCode, GenieError> {
        // the load dropped at the end of the work (FUN_140aadc50: the player's slot 0xf8 with
        // 1 and 1, then the builder carries nothing, FUN_14119d310)
        if let Some((kind, amount)) = step.dropped {
            c.give_load(kind, amount)?;
            self.life.attribute_type = -1;
            self.life.amount = 0.0;
        }
        // the unit's animation stopped (FUN_141161850), the second target let go
        if step.anim_stopped {
            self.body.set_anim_speed(0.0);
        }
        if let Some(s) = step.site_let_go {
            crate::world::lets::release(c, s, by)?;
        }
        if step.released {
            crate::world::lets::release(c, target, by)?;
        }
        self.post_build_notifies(&step)?;
        if step.stands {
            self.show_standing(
                c.w.masters.get(c.player),
                &c.w.graphics,
                c.w.streams.get(Stream::Sprite),
            )?;
        }
        if step.works_it {
            // slot 0x7e8 on the building it built, at its place (the caller's work command)
            let point =
                c.w.spatial
                    .get(target)
                    .ok_or(GenieError::unsupported(
                        "a builder's building without a place",
                    ))?
                    .position;
            c.work_order = Some(crate::orders::Work {
                target,
                point,
                carry_test: false,
                filter: -1,
                notified: true,
            });
        }
        if step.search {
            self.build_search(c, (target, built, by))?;
        }
        if step.drop_off {
            self.build_drop_off(c, me)?;
        }
        // its end's 1 deletes it; the destructor (FUN_140aadbf0) finds it out of the builders
        // (set_state left them) and lets go the foundation and the second target it still holds
        Ok(HeadCode::ended(step.done))
    }

    /// The notifies a Build's step posts to the builder's unit AI: the search needed, a pasture
    /// already built (`FUN_1411c5f70`'s new id) and the failure.
    fn post_build_notifies(&mut self, step: &BuildStep) -> Result<(), GenieError> {
        if step.notified {
            self.post_own(SEARCH_NEEDED, [ActionKind::Build.value(), 0, 0])?;
        }
        if let Some(made) = step.pasture_made {
            self.post_own(
                crate::unit_ai::PASTURE_BUILT,
                [ActionKind::Build.value(), made, 0],
            )?;
        }
        if step.failed {
            self.post_own(
                crate::unit_ai::ACTION_FAILED,
                [ActionKind::Build.value(), 0, 0],
            )?;
        }
        Ok(())
    }

    /// The Build's case 0xdf after the work: the builder's search for more work around the
    /// building it finished ([`Unit::more_work`]); without one the Build waits (`set_state(3)`
    /// without orders: notify 0x202, then the standing sprite) and lets the building go.
    pub(crate) fn build_search(
        &mut self,
        c: &mut Context<'_>,
        (target, built, by): (i32, Option<i16>, Releaser),
    ) -> Result<(), GenieError> {
        let built = built.ok_or(GenieError::unsupported(
            "a builder's building the slice does not hold",
        ))?;
        if let Some(work) = self.more_work(c, target, built)? {
            c.work_order = Some(work);
        } else {
            if let Some(b) = self.actions.build_mut() {
                b.no_more_work()?;
            }
            crate::world::lets::release(c, target, by)?;
            self.post_own(SEARCH_NEEDED, [ActionKind::Build.value(), 0, 0])?;
            self.show_standing(
                c.w.masters.get(c.player),
                &c.w.graphics,
                c.w.streams.get(Stream::Sprite),
            )?;
        }
        Ok(())
    }
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

/// The foundation `target` as the Build's update works it ([`Site`]): its construction and its
/// completion (`template` with the foundation's own master, unit AI, main building and rice),
/// the foundation's annexes of a building type (`annex_ids`), whose work starts first (none when
/// the work goes on to a main building, `main_work`), and the builders of the main building a
/// building annex counts its builders in (`FUN_140aaddb0`, `+0xbb8`). None when this player does
/// not hold the foundation.
fn build_site<'u>(
    own: &'u mut ObjectList,
    target: i32,
    annex_ids: &[i32],
    template: Completion<'u>,
    main_work: Option<MainWork>,
) -> Result<Option<Site<'u>>, GenieError> {
    let masters = template.masters;
    let counted_main = own
        .by_id(target)
        .filter(|u| u.master_in(masters).is_ok_and(|m| m.kind == BUILDING_KIND))
        .and_then(|u| u.annex_of);
    let mut main_builders = None;
    let mut site = None;
    let mut starts = Vec::new();
    for u in own.iter_mut().filter(|u| u.id != TAKEN_OUT) {
        if u.id == target {
            site = Some(u);
            continue;
        }
        if Some(u.id) == counted_main {
            let k = u.construction.as_mut().ok_or(NO_CONSTRUCTION)?;
            main_builders = Some(&mut k.builders);
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
    if counted_main.is_some() && main_builders.is_none() {
        return Err(NOT_HELD);
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
            main_builders,
        })
    }))
}

/// `FUN_140fde180`: the foundation's master `built` is a drop site (the masters 562, 584, 621,
/// 68, 1808, 50, 1187 and 199, or one the drop-site table has for every civ or the builder's).
fn drop_site_master(c: &Context<'_>, built: Option<i16>) -> bool {
    built.is_some_and(|m| {
        DROP_SITE_MASTERS.contains(&m)
            || c.w
                .drop_sites
                .iter()
                .any(|d| d.building == m && (d.civs.is_empty() || d.civs.contains(&c.civ)))
    })
}

/// What the Build's step changes of the world beside the builder `me`: the cleared objects and
/// the footprint's removals and occupants (at once, through the world), the constructions that
/// started (its annexes' first: their owner watches each at once, `FUN_1411e5670`), the work an annex's
/// main building takes ([`World::work_building`]), and the completion (taken out of the step)
/// with its annexes' state ([`World::set_state`]), its tracker part, its standing sprite and
/// what it leaves its owner ([`World::built`]). The last three run with the builder lent to the
/// world ([`Context::lend`]): their searches and effects meet it, as the game's do.
///
/// # Errors
///
/// What an annex's `set_state`, the main building's work and the completion's outputs do not
/// model.
pub(crate) fn record_build_step(
    c: &mut Context<'_>,
    me: &mut Unit,
    target: i32,
    annex_ids: &[i32],
    step: &mut BuildStep,
) -> Result<(), GenieError> {
    // the footprint clearing's deletes (the Build's set_state(7), FUN_1415c3620), then the
    // foundation another player's marks lie on: a human owner's goes (FUN_1411ccbb0)
    for cl in step.cleared.iter().copied() {
        c.w.clear_object(cl, true)?;
    }
    if let Some(site) = step.removes
        && let Some((q, _)) = c.w.object(site)
        && !c.w.players[q].computer
    {
        c.w.remove_building(q, site)?;
    }
    // the units its footprint's test met are sent off (FUN_14123bb20), the builder not among
    // them
    if !step.occupants.is_empty() {
        c.w.evacuate(target, &step.occupants)?;
    }
    for id in annex_ids.iter().copied().chain([target]) {
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
    // the work the annex leaves its main building (FUN_1411cc430), where the builder's runs
    if let Some(amount) = step.main_work {
        c.lend(me, |w| w.work_building(target, amount))?;
    }
    if let Some(built) = step.completed.take() {
        // its set_state(2) took its annexes along (FUN_1411dbb70: their slot 0xa0), which the
        // game does before the completion the step made; then the completion's first part's
        // tracker part (FUN_1411c6bd0 -> FUN_14104eac0), the foundation the builder's player's
        for annex in c.w.spatial.annexes_of(target) {
            if let Some((q, _)) = c.w.object(annex) {
                c.lend(me, |w| w.set_state(q, annex, 2))?;
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
        // what it leaves its owner, at once and in the game's order, the builder lent to the
        // world for its searches and effects; the start's watch is the started loop's above,
        // which cleared the flag
        let built = crate::construction::build::Built {
            started: false,
            ..built
        };
        c.lend(me, |w| w.built(p, target, built))?;
    }
    Ok(())
}
