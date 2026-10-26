//! The searches for more work and for resources, and the objects its unit AI names.

use crate::map::spatial::Spatial;
use crate::masters::Master;
use crate::math::rng::Stream;
use crate::movement::{Planning, footprint_gap_squared};
use crate::turn::unit::Unit;
use crate::turn::view::{Context, OwnerView};
use crate::turn::world::{Releaser, ResourceWorld, closest_resource_of};
use crate::unit_ai::{GenieError, Notify};

impl Unit {
    /// A notify the unit posts to its own unit AI (slot 0x328).
    /// The Build's case 0xdf once its work ended without queued orders (`0x140ab0b43`): the
    /// builder looks for more work around building `building` of master `built`
    /// ([`crate::gathering::gather_search::more_work`] with the drop site's entry for the building, the
    /// builder's class and civ, the reach the building's line of sight held to 4 to 20, and the
    /// planner of the builder's restriction). Nothing found: None, the caller lets the foundation
    /// go. A farm found is not modelled; else the unit AI's `ClosestResourceObject` for the found
    /// object's gather group and workers cap ([`crate::gathering::gather_search::gather_group`],
    /// [`crate::gathering::gather_search::closest_resource`]) names the target, the found object when it
    /// names none, and slot 0x7e8 works it at its place (the work flag 1 only logged, any task,
    /// no carried-resource test, `+0xa14` as it stands outside slot 0x128: 1; the caller's).
    pub(in crate::turn) fn more_work(
        &self,
        c: &mut Context<'_>,
        others: &[Self],
        building: i32,
        built: i16,
    ) -> Result<Option<crate::orders::Work>, GenieError> {
        use crate::gathering::gather_search::{self, Candidate};
        use crate::math::image::gather_search::{REACH_CAP, REACH_FLOOR};
        /// The reach a line of sight below the floor takes (the Build's immediate 4.0).
        const REACH_LOW: f32 = 4.0;
        let masters = c.masters.ok_or(GenieError::unsupported(
            "a builder without its player's masters",
        ))?;
        let master = masters
            .get(self.life.master_id)
            .ok_or(GenieError::unsupported(
                "a builder of a master the slice does not have",
            ))?;
        let site = masters.get(built).ok_or(GenieError::unsupported(
            "a building of a master the slice does not have",
        ))?;
        let at = *c.spatial.get(building).ok_or(GenieError::unsupported(
            "a builder's building without a place",
        ))?;
        let entry = crate::tasks::drop_site(c.drop_sites, built, master.class, c.civ);
        let los = site.line_of_sight;
        let held = if REACH_FLOOR <= los { los } else { REACH_LOW };
        let reach = if held <= REACH_CAP { held } else { REACH_CAP };
        let builder = gather_search::Builder {
            position: self.position,
            radius: master.radius[0],
            building: at.position,
            range: reach as i32 as i16,
        };
        let mover = self
            .mover
            .as_ref()
            .ok_or(GenieError::unsupported("a builder without movement state"))?;
        let restriction = mover.stats.restriction;
        let (player, me_id, foreign, every) = (c.player, self.id, c.foreign, c.every_masters);
        let candidate = |id: i32, owner: usize, u: &Self, position: [f32; 3]| {
            let m = every
                .get(owner)
                .and_then(|m| m.get(u.life.master_id))
                .ok_or(crate::turn::OBJECT_MASTER)?;
            Ok(Candidate {
                id,
                player: owner,
                class: m.class,
                master: m.id,
                state: u.life.state,
                workers: u.workers,
                position,
                pasture: u.pasture,
                untargetable: u.untargetable,
                attribute: u.life.attribute_type,
                amount: u.life.amount,
                auto_gather: m.auto_gather,
                gather_group: m.gather_group,
            })
        };
        let me = candidate(me_id, player, self, self.position)?;
        let spatial: &Spatial = c.spatial;
        let objects = |x: i32, y: i32| -> Result<Vec<Candidate>, GenieError> {
            spatial
                .objects_on(x, y)
                .map(|(id, p)| {
                    if id == me_id {
                        return Ok(me);
                    }
                    let (owner, u) = others
                        .iter()
                        .find(|u| u.id == id)
                        .map(|u| (player, u))
                        .or_else(|| foreign.find(id))
                        .ok_or(GenieError::unsupported(
                            "an object on a tile the slice does not hold",
                        ))?;
                    candidate(id, owner, u, p.position)
                })
                .collect()
        };
        let found =
            {
                let planner = c.pathing.planner(restriction, spatial.tiles()).ok_or(
                    GenieError::unsupported("a builder's restriction without a planner"),
                )?;
                let width = i64::from(planner.level_sizes().0[0]);
                let cells = planner.level(0);
                let blocked = move |x: i32, y: i32| {
                    cells[(width * i64::from(y) + i64::from(x)) as usize][0] == 0
                };
                gather_search::more_work(entry, &builder, spatial.map, Some(&blocked), &objects)?
            };
        let Some(found) = found else {
            return Ok(None);
        };
        if found.class == gather_search::FARM {
            return Err(GenieError::unsupported(
                "a builder's search for more work finding a farm",
            ));
        }
        let (kind, cap) = gather_search::gather_group(found.attribute);
        let target = self.resource_search(c, others, kind, cap)?;
        let spatial: &Spatial = c.spatial;
        let target = if target != -1 && spatial.get(target).is_some() {
            target
        } else {
            found.id
        };
        let point = spatial
            .get(target)
            .ok_or(GenieError::unsupported(
                "a builder's new work without a place",
            ))?
            .position;
        Ok(Some(crate::orders::Work {
            target,
            point,
            carry_test: false,
            filter: -1,
            notified: true,
        }))
    }

    /// The hunt's search for another target (`FUN_140ac4670`) when the class its unit AI targets
    /// (else the hunt's) is one the AI's slot 0x1b0 gives 0 (the civilian's `FUN_140c1c4f0`:
    /// trees, 15): with `within` (its first argument above 0; 8 when the target went on the walk
    /// to it) only a hunt within it of its point searches (the 3D distance, its components summed
    /// x, height, y); the unit AI's `ClosestResourceObject` with gather group 0, the hunt's second
    /// target as the drop site, at most 2 workers ([`Unit::resource_search_from`]) names the next
    /// tree; with `near` (its second argument above 0; 5 when the Attack's `MoveTo` failed) a tree
    /// farther than it from the unit (the same 3D distance) does not count. The hunt takes it
    /// (slot 0x48: [`crate::gathering::hunt::HuntAction::retarget`], the new target taken). True when it
    /// found one.
    pub(in crate::turn) fn hunt_tree_search(
        &mut self,
        c: &mut Context<'_>,
        others: &[Self],
        within: Option<f32>,
        near: Option<f32>,
    ) -> Result<bool, GenieError> {
        // NaN compares false as the game's `comiss` does
        let beyond = |a: [f32; 3], b: [f32; 3], limit: f32| {
            let (dx, dz, dy) = (a[0] - b[0], a[2] - b[2], a[1] - b[1]);
            let d = crate::math::fmath::sqrt(dx * dx + dz * dz + dy * dy);
            d.partial_cmp(&limit) != Some(std::cmp::Ordering::Less)
        };
        let hunt = self.hunt.as_ref().expect("the caller's");
        if let Some(within) = within
            && beyond(hunt.point, self.position, within)
        {
            return Ok(false);
        }
        let site = hunt.carry_to;
        let found = self.resource_search_from(c, others, 0, 2, site)?;
        if found == -1 {
            return Ok(false);
        }
        let (tp, t) = others
            .iter()
            .find(|u| u.id == found)
            .map(|u| (c.player, u))
            .or_else(|| c.foreign.find(found))
            .ok_or(GenieError::unsupported(
                "a hunt's next tree the slice does not hold",
            ))?;
        if let Some(near) = near {
            let (dx, dz, dy) = (
                self.position[0] - t.position[0],
                self.position[2] - t.position[2],
                self.position[1] - t.position[1],
            );
            if near < crate::math::fmath::sqrt(dx * dx + dz * dz + dy * dy) {
                return Ok(false);
            }
        }
        let t_master = c
            .every_masters
            .get(tp)
            .and_then(|m| m.get(t.life.master_id))
            .ok_or(GenieError::unsupported(
                "a hunt's next tree of a master the slice lacks",
            ))?;
        let own = c
            .spatial
            .relations
            .get(c.player)
            .ok_or(GenieError::unsupported(
                "a hunter's player without relations",
            ))?;
        let quarry = crate::gathering::hunt::Quarry {
            id: found,
            position: t.position,
            class: t_master.class,
            kind: t_master.kind,
            state: t.life.state,
            player: tp,
            amount: t.life.amount,
            allied: crate::tasks::allied(own, c.player, tp, true),
            top_action: t.top_action_kind(),
            drop_site: false,
        };
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(crate::turn::HUNTER_MASTER)?;
        let drops_here = master.drop_sites.contains(&t.life.master_id);
        let at = |id| {
            c.spatial
                .get_known(id)?
                .ok_or(GenieError::unsupported("a hunt's object without a place"))
        };
        let (me_at, them) = (at(self.id)?, at(found)?);
        let speed = self.mover.as_ref().map_or(0.0, |m| m.stats.speed);
        let engagement = crate::combat::attack::engagement(
            self.ai.as_ref(),
            speed,
            master,
            me_at,
            them,
            c.world_time,
        )?;
        let hunt = self.hunt.as_mut().expect("the caller's");
        let old = hunt.target;
        let moving = hunt
            .task
            .walk_sprite_id(self.life.attribute_type, self.life.amount);
        let arms = crate::combat::attack::arms(master, c.graphics, hunt.task.sprites, moving);
        let mover = self.mover.as_mut().ok_or(crate::turn::HUNTER_UNMOVING)?;
        if let Some(arrived) = hunt.retarget(
            &quarry,
            c.player,
            self.life.amount,
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
            // slot 0x68 (FUN_140a7a230): the old target goes, the new one is taken
            self.hunt_release(c, [old]);
            c.takes.push((found, self.releaser(c)));
        }
        Ok(true)
    }

    /// The unit AI's `ClosestResourceObject` (slot 0x80, `FUN_140c49760`) with no drop site, no
    /// point and `param_6` 2 ([`crate::gathering::gather_search::closest_resource`]): the closest object of
    /// gather group `kind` with fewer than `cap` workers (−1 any) the unit reaches (slot 0x3d0
    /// with range 0), from the tiles the player explored that the unit's restriction lets it
    /// stand on; −1 for none.
    pub(crate) fn resource_search(
        &self,
        c: &mut Context<'_>,
        others: &[Self],
        kind: u8,
        cap: i32,
    ) -> Result<i32, GenieError> {
        self.resource_search_from(c, others, kind, cap, -1)
    }

    /// [`Unit::resource_search`] from the drop site `site` (−1 for none;
    /// [`crate::gathering::gather_search::closest_resource_at`]): the site's place, its master's radii and
    /// line of sight, and whether the search starts from its footprint (the site's tile in the
    /// unit's zone of its restriction, or a dock: masters 45 and 2172, `0x140c49d17`).
    pub(in crate::turn) fn resource_search_from(
        &self,
        c: &mut Context<'_>,
        others: &[Self],
        kind: u8,
        cap: i32,
        site: i32,
    ) -> Result<i32, GenieError> {
        let current = self
            .ai
            .as_ref()
            .ok_or(GenieError::unsupported(
                "a resource search without a unit AI",
            ))?
            .current_target;
        let mut w = ResourceWorld {
            player: c.player,
            spatial: c.spatial,
            zones: c.zones,
            foreign: c.foreign,
            every: c.every_masters,
            masters: c.masters,
            plan: Planning {
                pathing: c.pathing,
                now: c.world_time,
                computer: c.computer,
                object: c.streams.get(Stream::Object),
                construction_rate: c.construction_rate,
                groups: c.groups,
            },
        };
        let search = (current, 2, crate::unit_ai::NO_POINT);
        closest_resource_of(self, &mut w, others, (kind, cap, site), search).map(|(id, _)| id)
    }

    /// The unit as the releaser of its actions' targets (slot 0x180's argument).
    pub(crate) fn releaser(&self, c: &Context<'_>) -> Releaser {
        Releaser {
            id: self.id,
            class: c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .map_or(-1, |m| m.class),
            player: c.player,
        }
    }

    /// The objects its unit AI names besides its target, as the world's table holds them
    /// ([`crate::unit_ai::Surroundings::units`]): its attackers and the callers of its damage
    /// notifies, from its own list or another player's, each with its master's class, state, its
    /// unit AI's target and its footprint's squared gap to the unit's (`FUN_14118f8c0`).
    pub(in crate::turn) fn named_units(
        &self,
        c: &Context<'_>,
        view: &OwnerView<'_>,
        master: Option<&Master>,
    ) -> Result<Vec<crate::unit_ai::UnitSeen>, GenieError> {
        let Some(ai) = self.ai.as_ref() else {
            return Ok(Vec::new());
        };
        let damaged = ai
            .notifies
            .iter()
            .filter(|n| n.kind == crate::unit_ai::DAMAGED)
            .map(|n| n.caller);
        let in_the_way = ai
            .notifies
            .iter()
            .filter(|n| n.kind == crate::unit_ai::IN_THE_WAY)
            .map(|n| n.p[0]);
        let mut ids: Vec<i32> = ai
            .attackers
            .iter()
            .copied()
            .chain(damaged)
            .chain(in_the_way)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let mut seen = Vec::new();
        for id in ids {
            let found = view
                .others
                .iter()
                .find(|u| u.id == id)
                .map(|u| (view.player, u))
                .or_else(|| view.foreign.find(id));
            let (Some((owner, u)), Some(p)) = (found, c.spatial.get(id)) else {
                continue;
            };
            let m = c
                .every_masters
                .get(owner)
                .and_then(|ms| ms.get(u.life.master_id))
                .ok_or(GenieError::unsupported(
                    "a named object of a master the slice does not have",
                ))?;
            let me = master.ok_or(crate::turn::UNIT_AI_MASTER)?;
            seen.push(crate::unit_ai::UnitSeen {
                id,
                class: m.class,
                owner,
                combat_level: m.combat_level,
                state: u.life.state,
                target: u.ai.as_ref().map(|a| a.current_target),
                gap_squared: footprint_gap_squared(self.position, me.radius, p.position, p.radius),
                position: p.position,
                hp: u.life.hp,
                master: u.life.master_id,
                kind: m.kind,
                main: u.annex_of,
                allied: crate::tasks::allied(view.relations, view.player, owner, true),
            });
        }
        Ok(seen)
    }

    pub(crate) fn post_own(&mut self, kind: i32, p: [i32; 3]) -> Result<(), GenieError> {
        let (id, state) = (self.id, self.life.state);
        self.ai
            .as_mut()
            .map_or(Ok(()), |ai| ai.post(Notify::own(id, kind, p), state))
    }
}
