//! A new object on the ground, the command check's units and the mule cart's searches.

use crate::math::rng::Stream;
use crate::movement::{Planning, footprint_gap_squared};
use crate::objects::unit::Unit;
use crate::unit_ai::GenieError;
use crate::unit_ai::world_view::Foreign;
use crate::world::Turns;
use crate::world::lookups::{ResourceWorld, closest_resource_of};

impl Turns {
    /// A new object of player `p`'s `master` placed at `at` (the static `new_position`,
    /// `0x1411a58b0`, slot 0x60's `FUN_141193a10`): its height the ground's by the master's fly
    /// mode; without the map's elevations the height stays.
    pub(crate) fn on_ground(&self, p: usize, master: i16, at: [f32; 3]) -> [f32; 3] {
        let fly_mode = self
            .masters
            .get(p)
            .and_then(|m| m.get(master))
            .map_or(0, |m| m.fly_mode);
        self.spatial
            .terrain
            .as_ref()
            .and_then(|t| t.ground(at[0], at[1]).ok())
            .map_or(at, |g| {
                [
                    at[0],
                    at[1],
                    crate::combat::missile::grounded(at[2], g, false, fly_mode),
                ]
            })
    }

    /// The command check's units (`FUN_140e553f0`, `FUN_140e55250`): each unit the world holds
    /// that is the player's own, or any while players may order each other's units
    /// ([`crate::world::settings::Settings::cheats_enabled`]); −1 for the others.
    pub(crate) fn checked_units(&self, player: usize, units: &[i32]) -> Vec<i32> {
        units
            .iter()
            .map(|&id| match self.object(id) {
                Some((p, _)) if p == player || self.settings.cheats_enabled => id,
                _ => -1,
            })
            .collect()
    }

    /// A mule cart's completion (`FUN_1411c6bd0`'s tail, a building whose master has a task of
    /// action type 0x99, [`crate::construction::build::Built::cart`]) with a unit AI: with no current target a
    /// human player's runs its AI's slot 0x1e8 ([`Turns::cart_work_search`]; the game mode's
    /// test, `FUN_140c87f20` not 4, holds in every recording probed), all of it inside the
    /// completion's `set_state(state)`; then a cart with a target moves to it (slot 0x138 at the
    /// target's place).
    pub(crate) fn cart_search(&mut self, p: usize, id: i32, state: u8) -> Result<(), GenieError> {
        let Some(idx) = self.players[p].objects.iter().position(|u| u.id == id) else {
            return Ok(());
        };
        let Some(current) = self.players[p].objects[idx]
            .ai
            .as_ref()
            .map(|ai| ai.current_target)
        else {
            return Ok(());
        };
        let found = if current == -1 && !self.players[p].computer {
            Some(self.cart_find(p, idx)?)
        } else {
            None
        };
        // all of it runs inside the completion's set_state(2), before the cart is in state 2
        let built = std::mem::replace(&mut self.players[p].objects[idx].life.state, state);
        let result = (|| {
            if let Some(found) = found {
                self.cart_work(p, idx, found)?;
            }
            // with a target (the one the work command gave the unit AI) the cart moves to it
            // (slot 0x138 at its place)
            let target = self.players[p].objects[idx]
                .ai
                .as_ref()
                .map_or(-1, |ai| ai.current_target);
            if target != -1
                && let Some(at) = self.spatial.get(target).map(|o| o.position)
            {
                self.move_to_object(p, idx, target, at)?;
            }
            Ok::<_, GenieError>(())
        })();
        self.players[p].objects[idx].life.state = built;
        result
    }

    /// The mule cart's own search for resources, its unit AI's slot 0x1e8 (`FUN_140c16240`) for
    /// the cart at `idx` of player `p`: [`Turns::cart_find`], then [`Turns::cart_work`]. The
    /// completion runs it ([`Turns::cart_search`]), and so does the Empire Wars start
    /// ([`crate::orders::start_work`]).
    pub(crate) fn cart_work_search(&mut self, p: usize, idx: usize) -> Result<(), GenieError> {
        let found = self.cart_find(p, idx)?;
        self.cart_work(p, idx, found)
    }

    /// Slot 0x1e8's search (`FUN_140c16240`, its world test `FUN_141533b60` held): per task of
    /// the cart's master in order, the target class of the last of type 0x99 so far names a
    /// gather group (`FUN_140fdd5c0` with 1: 1 for 7, 3 for 8, 0 for 15, 4 for 32 and 63), each
    /// searched by `ClosestResourceObject` (slot 0x80) from the cart as the drop site, any
    /// workers, from state 0 on; of those found, the nearest by the squared footprint gap
    /// (`FUN_14118f8c0`, below 99999, `DAT_142ef9008`), with its place; none at the cart's own.
    pub(crate) fn cart_find(
        &mut self,
        p: usize,
        idx: usize,
    ) -> Result<(i32, [f32; 3]), GenieError> {
        const CART_REACH: f32 = crate::math::image::turn::CART_REACH;
        let id = self.players[p].objects[idx].id;
        let current = self.players[p].objects[idx]
            .ai
            .as_ref()
            .map_or(-1, |ai| ai.current_target);
        if self.players[p].objects[idx].mover.is_none() {
            return Err(GenieError::unsupported(
                "a mule cart's search without its movement state (a moving building's setup)",
            ));
        }
        let master = self.master_of(p, self.players[p].objects[idx].life.master_id)?;
        let mut kinds = Vec::new();
        let mut class = -1;
        for t in &master.tasks {
            if t.action_type == crate::construction::build::CART_TASK {
                class = t.object_group;
            }
            let kind = match class {
                7 => 1,
                8 => 3,
                15 => 0,
                32 | 63 => 4,
                _ => continue,
            };
            kinds.push(kind);
        }
        let cart = std::mem::replace(&mut self.players[p].objects[idx], Unit::taken_out());
        let result = (|| {
            let (before, rest) = self.players.split_at_mut(p);
            let (player, after) = rest.split_first_mut().expect("p is a player");
            let mut w = ResourceWorld {
                player: p,
                spatial: &self.spatial,
                zones: &mut self.zones,
                foreign: Foreign {
                    before: &*before,
                    after: &*after,
                    left: None,
                },
                every: &self.masters,
                masters: self.masters.get(p),
                plan: Planning {
                    pathing: &mut self.pathing,
                    now: self.world_time,
                    computer: player.computer,
                    object: self.streams.get(Stream::Object),
                    construction_rate: player.construction_rate,
                    groups: &self.groups,
                },
            };
            let mut found = Vec::new();
            for kind in kinds {
                let (f, _) = closest_resource_of(
                    &cart,
                    &mut w,
                    &player.objects,
                    (kind, -1, id),
                    (current, 0, crate::unit_ai::NO_POINT),
                )?;
                if f != -1 {
                    found.push(f);
                }
            }
            Ok::<_, GenieError>(found)
        })();
        self.players[p].objects[idx] = cart;
        let found = result?;
        let cart_master = self.master_of(p, self.players[p].objects[idx].life.master_id)?;
        let at = self.players[p].objects[idx].position;
        let mut best: Option<(i32, [f32; 3])> = None;
        let mut best_d = CART_REACH;
        for f in found {
            let Some(o) = self.spatial.get(f) else {
                continue;
            };
            let d = footprint_gap_squared(at, cart_master.radius, o.position, o.radius);
            if d < best_d {
                best = Some((f, o.position));
                best_d = d;
            }
        }
        Ok(best.unwrap_or((-1, at)))
    }

    /// Slot 0x1e8's end: slot 0x140 (`FUN_141070c90`), the work command toward the found
    /// target at its place, else at the cart's own (slot 0x7e8: no carried-resource test, any
    /// task; `+0x888`'s count is not held).
    pub(crate) fn cart_work(
        &mut self,
        p: usize,
        idx: usize,
        (target, point): (i32, [f32; 3]),
    ) -> Result<(), GenieError> {
        let work = crate::orders::Work {
            target,
            point,
            carry_test: false,
            filter: -1,
            notified: true,
        };
        self.order_work(p, idx, &work)
    }
}
