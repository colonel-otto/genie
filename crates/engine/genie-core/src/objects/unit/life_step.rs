//! The life cycle's step in a unit's update: the hit points, the states, and what each new
//! state sets off.

use crate::GenieError;
use crate::math::rng::Stream;
use crate::objects::lifecycle::death_parts::forget_resource;
use crate::objects::unit::{Step, Unit};
use crate::world::context::Context;

impl Unit {
    /// The life cycle's part of the update: the storage that a dying object's player loses, the
    /// life cycle's step and what its new state sets off (`FUN_14119d710`): the death, the ghost,
    /// the annexes' states, the sight, the remains and the death spawn. [`Step::Leave`] when the
    /// object leaves the world.
    pub(crate) fn life_step(&mut self, c: &mut Context<'_>) -> Result<Step, GenieError> {
        if self.life.is_alive() && self.life.hp < 1.0 {
            // set_state(3)'s storage (FUN_1411a56c0 with 0): each of kinds 2 and 16 of an
            // attribute leaves the player (its slot 0xf8 with the amount negated)
            if let Some(m) = c.masters.and_then(|m| m.get(self.life.master_id)) {
                for &(attribute, amount, kind) in &m.storage {
                    if matches!(kind, 2 | 16) && 0 <= attribute {
                        c.attribute_adds.push((attribute, -amount));
                    }
                }
            }
        }
        let before = self.life.state;
        if before == 4 && self.sprite.is_none() {
            return Err(GenieError::unsupported(
                "a dying object whose sprite the slice does not hold",
            ));
        }
        // the sprite list's slot 0x20 on the current sprite (FUN_1413b38a0)
        let animating = self
            .sprite
            .is_some_and(|s| s.animate != 0 && s.graphic.flags & 1 != 0);
        let leaves = if self
            .life
            .update(c.dt_seconds, c.streams.get(Stream::Object), animating)?
        {
            Step::Leave
        } else {
            Step::Stay
        };
        // the set_states of the steps (FUN_14119d710): a death goes through state 3
        let after = self.life.state;
        if before <= 2 && 3 <= after {
            self.died(c, before)?;
            // the static set_state's ghosts (FUN_14118e990), the caller's
            if c.masters
                .and_then(|m| m.get(self.life.master_id))
                .is_some_and(|m| m.ghost_on_death == crate::vision::ghost::LEAVES_GHOSTS)
            {
                c.ghost_deaths.push(self.id);
            }
        }
        // a building's set_states after its death's 3 take its annexes along too
        // (FUN_1411dbb70): the animated update's 4 or 5 from 3, the static update's 7 and 8; the
        // dying sprite's end (4 to 5) writes the state without a set_state
        if 3 < after
            && after != before
            && !(before == 4 && after == 5)
            && c.masters
                .and_then(|m| m.get(self.life.master_id))
                .is_some_and(|m| m.kind == 80)
        {
            for annex in c.spatial.annexes_of(self.id) {
                c.annex_states.push((annex, after));
            }
        }
        if after != before {
            if before <= 2 && 3 <= after {
                c.spatial.sight_state(self.id, 3);
            }
            if after != 3 {
                c.spatial.sight_state(self.id, after);
            }
            if before < 4 && 6 < after {
                let master = c.masters.and_then(|m| m.get(self.life.master_id));
                forget_resource(c.spatial, master, self.id, self.position);
            }
        }
        // the dying sprite ended (RGE_Animated_Object::update): the animation stops and an object
        // below type 80 leaves its remains (FUN_1411a4e90)
        if before == 4 && after == 5 {
            self.anim_speed = Some(0.0);
            let kind = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .map(|m| m.kind)
                .ok_or(GenieError::unsupported(
                    "a dead object of a master the slice does not have",
                ))?;
            if kind < 80 && 0 <= self.life.remains {
                // its facet (`+0xe`) as its angle left it ([`crate::objects::facing::facet_of_angle`])
                let facet = self
                    .mover
                    .as_ref()
                    .map(|m| crate::objects::facing::facet_of_angle(m.angle, m.stats.facets).0);
                c.remains = Some((self.life.remains, self.position, facet));
            }
        }
        // a dead object that holds nothing goes to state 7 and its death spawn comes (slot 0x160,
        // FUN_1411a5090: the master's `death_spawn_obj_id` makes an object at its place; a moving
        // object's override gives it the dead object's facet and that facet's angle); the
        // spawn's sprite from slot 0x7a8 (the master's undead sprite) is not held
        if matches!(before, 3 | 5) && after == 7 && self.life.death_spawn {
            let spawn = c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .map(|m| m.death_spawns[0])
                .ok_or(GenieError::unsupported(
                    "a dead object of a master the slice does not have",
                ))?;
            let facet = self
                .mover
                .as_ref()
                .map(|m| crate::objects::facing::facet_of_angle(m.angle, m.stats.facets).0);
            c.remains = Some((spawn, self.position, facet));
        }
        Ok(leaves)
    }
}
