//! A builder's search for more work once its Build ended without queued orders (the Build's
//! case 0xdf): a resource around the building it finished, as the unit AI's
//! `ClosestResourceObject` names it.

use crate::GenieError;
use crate::map::spatial::Spatial;
use crate::objects::unit::Unit;
use crate::world::context::Context;

impl Unit {
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
    pub(crate) fn more_work(
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
        let entry =
            crate::unit_ai::task_finder::drop_site(c.drop_sites, built, master.class, c.civ);
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
                .ok_or(crate::world::OBJECT_MASTER)?;
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
}
