//! What the unit AI asks of a build while it runs (through its view, [`UnitView`]): the build
//! command at once, the builder's search for another foundation, and a unit's spot off a
//! foundation's footprint.

use crate::GenieError;
use crate::objects::release::Releaser;
use crate::unit_ai::task_finder::Actor;
use crate::unit_ai::task_finder::world::{SideWorld, task_search, task_target_with};
use crate::unit_ai::world_view::UnitView;

impl UnitView<'_> {
    /// The build command at once ([`World::build_command`]'s path without a Build on top, which
    /// a stop before it leaves none): slot 0x820 with any action type and the switch
    /// ([`task_search`] on the foundation and its main building) finds a task; the switch (slot
    /// 0x90 with 0: the actions go, then the master's unit part, [`crate::orders::switch_master`];
    /// its owner's part after the update); a Build on the task (`FUN_140aad4e0`, the footprints'
    /// gap, the task's range and work value, the builder's work rate with the unit's powerup)
    /// takes the foundation (slot 0x178) and replaces the actions (slot 0x808). A Build on top
    /// (its slot 0x48) is not modelled here.
    pub(crate) fn start_build_now(&mut self, foundation: i32) -> Result<(), GenieError> {
        use crate::construction::build::{BUILD, BuildAction, BuildTask};
        if self.unit.top_action_kind() == Some(BUILD) {
            return Err(crate::unit_ai::PENDING_BEFORE_ATTACK);
        }
        let unit = &*self.unit;
        let (player, others, foreign) = (self.player, self.others, self.foreign);
        let find = |id: i32| {
            if id == unit.id {
                return Some((player, unit));
            }
            others
                .iter()
                .find(|u| u.id == id)
                .map(|u| (player, u))
                .or_else(|| foreign.find(id))
        };
        let (fp, f) = find(foundation).ok_or(crate::commands::FOUNDATION_GONE)?;
        let site = self
            .every
            .get(fp)
            .and_then(|m| m.get(f.life.master_id))
            .ok_or(crate::world::OBJECT_MASTER)?;
        let f_at = f.position;
        let (Some(t), Some(general)) = (
            task_target_with(foundation, 1, &find, self.every)?,
            task_target_with(foundation, 2, &find, self.every)?,
        ) else {
            return Err(crate::commands::FOUNDATION_GONE);
        };
        let masters = self.masters.ok_or(crate::world::UNIT_AI_MASTER)?;
        let own = masters
            .get(unit.life.master_id)
            .ok_or(crate::world::UNIT_AI_MASTER)?;
        let actor = Actor {
            id: unit.id,
            master: own,
            player,
            held: (unit.life.attribute_type, unit.life.amount),
            attribute_95: self.attributes.get(95).copied().unwrap_or(0.0),
            civ: self.civ,
        };
        let dip = SideWorld {
            player,
            relations: self.relations,
            foreign,
            drop_sites: self.drop_sites,
        };
        let (task, switch) = task_search(masters, &actor, &t, &general, -1, &dip)?
            .ok_or(crate::commands::NO_BUILD_TASK)?;
        let builder = switch.unwrap_or(own);
        let gap = crate::movement::footprint_gap(unit.position, builder.radius, f_at, site.radius);
        let build = BuildAction::start(
            foundation,
            f_at,
            gap,
            BuildTask {
                range: task.work_range,
                work_value: task.work_val1,
                work_rate: builder.work_rate + unit.work_rate_powerup,
                resource: task.attribute_type,
                id: task.id,
            },
        );
        let id = unit.id;
        let unit = &mut *self.unit;
        if let Some(next) = switch {
            let by = Releaser {
                id,
                class: own.class,
                player,
            };
            unit.tear_down_actions(self.releases, self.builder_leaves, by)?;
            let world = crate::orders::SwitchWorld {
                spatial: self.spatial,
                pathing: self.pathing,
                graphics: self.graphics,
                sprite: self.sprite,
            };
            let switched = crate::orders::switch_master(unit, own, next, world)?;
            self.attribute_adds.extend(switched.leaving);
            if switched.promoted {
                self.promotions.push((id, own.id, next.id));
            }
            self.attribute_adds.extend(switched.coming);
        }
        let by = Releaser {
            id,
            class: builder.class,
            player,
        };
        self.takes.push((foundation, by));
        unit.tear_down_actions(self.releases, self.builder_leaves, by)?;
        unit.build = Some(build);
        Ok(())
    }

    /// `FUN_140c1c7c0`: the flood ([`crate::unit_ai::flood_search`]) from the unit's tile over the
    /// larger of its search radius (slot 0x2c8) and its line of sight, truncated, testing each tile
    /// it reaches (`FUN_140c2b950`: the first object of its tile's list, in order, of type 80 or
    /// more in state 0 of the unit's player); −1 for none.
    pub(crate) fn own_foundation_near(&self) -> Result<i32, GenieError> {
        let unit = &*self.unit;
        let master = self
            .masters
            .and_then(|m| m.get(unit.life.master_id))
            .ok_or(GenieError::unsupported(
                "a builder of a master the slice does not have",
            ))?;
        let r = if master.search_radius <= master.line_of_sight {
            master.line_of_sight
        } else {
            master.search_radius
        } as i32;
        let (ux, uy) = (unit.position[0] as i32, unit.position[1] as i32);
        let (player, others, masters, spatial) =
            (self.player, self.others, self.masters, &*self.spatial);
        let foundation_on = |x: i32, y: i32| -> Option<i32> {
            spatial.objects_on(x, y).find_map(|(id, p)| {
                if p.player != player {
                    return None;
                }
                let u = others.iter().find(|u| u.id == id)?;
                let kind = masters.and_then(|m| m.get(u.life.master_id))?.kind;
                (80 <= kind && u.life.state == 0).then_some(id)
            })
        };
        Ok(crate::unit_ai::flood_search((ux, uy), r, foundation_on).unwrap_or(-1))
    }

    /// The base handler's case 0x216: the unit's closest spot beside the foundation `site`
    /// ([`Spatial::spot_off`] with the notify's radii; none without the foundation), its height
    /// and class, its owner, the foundation and its current action's target as orders name them.
    ///
    /// [`Spatial::spot_off`]: crate::map::spatial::Spatial::spot_off
    pub(crate) fn spot_off_footprint(
        &self,
        site: i32,
        radius: [f32; 2],
    ) -> Result<crate::unit_ai::OffFootprint, GenieError> {
        let unit = &*self.unit;
        let master = self
            .masters
            .and_then(|m| m.get(unit.life.master_id))
            .ok_or(GenieError::unsupported(
                "a unit sent off of a master the slice does not have",
            ))?;
        let (player, others, foreign) = (self.player, self.others, &self.foreign);
        let named = |id: i32| {
            let found = if id == unit.id {
                Some((player, unit))
            } else {
                others
                    .iter()
                    .find(|u| u.id == id)
                    .map(|u| (player, u))
                    .or_else(|| foreign.find(id))
            };
            found.map(|(p, u)| crate::unit_ai::Named {
                id,
                master: i32::from(u.life.master_id),
                owner: p as i32,
                vector: [u.position[0], u.position[2]],
            })
        };
        let spot = match self.spatial.get(site) {
            Some(placed) => {
                let mover = unit.mover.as_ref().ok_or(GenieError::unsupported(
                    "a unit sent off without movement state",
                ))?;
                self.spatial.spot_off(
                    unit.position,
                    (unit.id, mover.stats.radius[0]),
                    mover.stats.restriction_mask,
                    (placed.position, radius),
                    false,
                )?
            }
            None => None,
        };
        let target = unit.top_target().filter(|&t| t != -1);
        Ok(crate::unit_ai::OffFootprint {
            spot,
            z: unit.position[2],
            class: master.class,
            computer: self.computer,
            player: player as i32,
            site: (site != -1).then(|| named(site)).flatten(),
            action_target: target.and_then(named),
        })
    }
}
