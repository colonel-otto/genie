//! The hunt's search for another target when its class is one the unit AI does not attack
//! (trees): the closest tree from the hunt's drop site, which the hunt then takes.

use crate::GenieError;
use crate::math::rng::Stream;
use crate::objects::unit::Unit;
use crate::world::context::Context;

impl Unit {
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
    pub(crate) fn hunt_tree_search(
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
        let hunt = self.actions.hunt().expect("the caller's");
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
            allied: crate::unit_ai::task_finder::allied(own, c.player, tp, true),
            top_action: t.top_action_kind(),
            drop_site: false,
        };
        let master = c
            .masters
            .and_then(|m| m.get(self.life.master_id))
            .ok_or(crate::world::HUNTER_MASTER)?;
        let drops_here = master.drop_sites.contains(&t.life.master_id);
        let at = |id| {
            c.spatial
                .get_known(id)?
                .ok_or(GenieError::unsupported("a hunt's object without a place"))
        };
        let (me_at, them) = (at(self.id)?, at(found)?);
        let speed = self.body.mover().map_or(0.0, |m| m.stats.speed);
        let engagement = crate::combat::attack::engagement(
            self.ai.as_ref(),
            speed,
            master,
            me_at,
            them,
            c.world_time,
        )?;
        let hunt = self.actions.hunt_mut().expect("the caller's");
        let old = hunt.target;
        let moving = hunt
            .task
            .walk_sprite_id(self.life.attribute_type, self.life.amount);
        let arms = crate::combat::attack::arms(master, c.graphics, hunt.task.sprites, moving);
        let mover = self.body.mover_mut().ok_or(crate::world::HUNTER_UNMOVING)?;
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
                    &mut self.body,
                    c.streams.get(Stream::Sprite),
                )?;
            }
            // slot 0x68 (FUN_140a7a230): the old target goes, the new one is taken
            self.hunt_release(c, [old]);
            c.takes.push((found, self.releaser(c)));
        }
        Ok(true)
    }
}
