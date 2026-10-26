//! The Empire Wars start: in the game's first update every idle villager is put to work
//! (`BaseWorld::update`, `0x14154fc70`, its per-player part before the player's objects update).
//!
//! The pass runs once (the world's `+0x6c6`) under the Empire Wars rule, the game mode 13 or in
//! another mode the sub-mode's bit 0 (`FUN_141324b60`, [`crate::world::settings::Settings::empire_wars`];
//! 426422314's random map game): each object of the player's live list, in its order, that is a villager (class 4) or a
//! fishing ship (class 21), may be interrupted with an empty order queue (slot 0x2d0 with 1) and
//! has a unit AI takes work. Under the game's farm setting a farm on its tile comes first (the
//! map tile's objects in their order: the first of class 49; a fishing ship's must be a fish
//! trap, master 199); else the closest resource of the five gather groups by the unit AI's
//! `ClosestResourceObject` (slot 0x80, [`crate::gathering::gather_search::closest_resource_at`]: no drop
//! site, any workers, the least distance wins, the first group at a tie). Slot 0x140
//! (`FUN_141070c90`) then gives the unit its work on the target at the target's place
//! ([`Turns::order_work`], slot 0x7e8; `+0x888`, the work's count, is not held). Any other
//! object that is a building (master type 80) whose master has a task of action type 0x99 (a
//! mule cart), may be interrupted as above and has a unit AI runs that AI's slot 0x1e8, the
//! cart's own search for resources ([`Turns::cart_work_search`]).

use crate::GenieError;
use crate::gathering::gather_search::{self, Candidate};
use crate::math::rng::Stream;
use crate::movement::{Me, Planning};
use crate::orders::Work;
use crate::world::Turns;

/// The game mode that puts the villagers to work at the start (`+0x34` 13, Empire Wars).
pub const EMPIRE_WARS: i32 = 13;

/// The classes the pass puts to work: villagers and fishing ships (decoded 4 and 21).
const VILLAGER: i16 = 4;
const FISHING_SHIP: i16 = 21;
/// The farm class a unit's tile is searched for, and the fish trap a fishing ship takes.
const FARM: i16 = 49;
const FISH_TRAP: i16 = 199;
/// The gather groups the resource search tries, in order (0 to 4).
const GROUPS: u8 = 5;
/// The distance a group's answer must be below (the pass's 99).
const NO_DISTANCE: i32 = 99;

/// What the pass gives an object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Start {
    /// A villager's or fishing ship's work on its farm or its closest resource.
    Work,
    /// A mule cart's own search for resources (its unit AI's slot 0x1e8).
    CartSearch,
}

/// The pass's choice for an object of the decoded `class`: a villager or fishing ship works,
/// else a `building` (master type 80) whose master has a task of action type 0x99 (`cart_task`,
/// `FUN_1415c60b0`) searches. The caller then tests that it is ready (slot 0x2d0 with 1: its
/// top action may be interrupted and its unit AI holds no order) and has a unit AI.
const fn start_of(class: i16, building: bool, cart_task: bool) -> Option<Start> {
    if matches!(class, VILLAGER | FISHING_SHIP) {
        return Some(Start::Work);
    }
    if building && cart_task {
        return Some(Start::CartSearch);
    }
    None
}

impl Turns {
    /// The pass for player `p` ([`crate::orders::start_work`]).
    ///
    /// # Errors
    ///
    /// The pass without the game's farm setting (the player's nearest unworked farm), a fishing
    /// ship's own search (its unit AI's slot 0x80), and what the work command does not model.
    pub(crate) fn start_work(&mut self, p: usize) -> Result<(), GenieError> {
        let mut i = 0;
        while i < self.players[p].objects.len() {
            let unit = &self.players[p].objects[i];
            let master = self.master_of(p, unit.life.master_id)?;
            let class = master.class;
            let building = master.kind == crate::objects::masters::master_type::BUILDING;
            let cart_task = building
                && self.has_task(
                    p,
                    unit.life.master_id,
                    crate::construction::build::CART_TASK,
                )?;
            let Some(start) = start_of(class, building, cart_task) else {
                i += 1;
                continue;
            };
            let idle = unit.ai.as_ref().is_some_and(|ai| ai.orders.is_empty());
            if !idle || !unit.interruptible()? {
                i += 1;
                continue;
            }
            if start == Start::CartSearch {
                self.cart_work_search(p, i)?;
                i += 1;
                continue;
            }
            if !self.settings.walkable_farms {
                return Err(GenieError::unsupported(
                    "the Empire Wars start without the farm setting (the nearest unworked farm)",
                ));
            }
            let target = match self.farm_under(p, i, class)? {
                Some(farm) => Some(farm),
                None if class == FISHING_SHIP => {
                    return Err(GenieError::unsupported(
                        "the Empire Wars start of a fishing ship (its unit AI's slot 0x80)",
                    ));
                }
                None => self.closest_of_groups(p, i)?,
            };
            if let Some(target) = target.filter(|&t| t != self.players[p].objects[i].id) {
                let point = self
                    .spatial
                    .get_known(target)?
                    .ok_or(GenieError::unsupported(
                        "the Empire Wars start's target without a place",
                    ))?
                    .position;
                let work = Work {
                    target,
                    point,
                    carry_test: false,
                    filter: -1,
                    notified: true,
                };
                self.order_work(p, i, &work)?;
            }
            i += 1;
        }
        Ok(())
    }

    /// The first farm in the object list of the unit's tile (a fish trap for a fishing ship).
    fn farm_under(&self, p: usize, i: usize, class: i16) -> Result<Option<i32>, GenieError> {
        let at = self.players[p].objects[i].position;
        let (x, y) = (at[0] as i32, at[1] as i32);
        self.spatial.check_known([at[0], at[1], at[0], at[1]])?;
        for (id, _) in self.spatial.objects_on(x, y) {
            let (owner, o) = self.object(id).ok_or(GenieError::unsupported(
                "an object on a tile the slice does not hold",
            ))?;
            let m = self.master_of(owner, o.life.master_id)?;
            if m.class == FARM && (class != FISHING_SHIP || m.id == FISH_TRAP) {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// The unit AI's `ClosestResourceObject` for each gather group (no drop site, any workers,
    /// `param_6` 2, no point): the answer at the least distance, the first group at a tie.
    fn closest_of_groups(&mut self, p: usize, i: usize) -> Result<Option<i32>, GenieError> {
        let mut best = (NO_DISTANCE, None);
        for kind in 0..GROUPS {
            let (id, distance) = self.resource_search_from(p, i, kind)?;
            if let Some(d) = distance.filter(|&d| d < best.0) {
                best = (d, Some(id));
            }
        }
        Ok(best.1)
    }

    /// [`gather_search::closest_resource_at`] for the unit at `i` of player `p` outside its
    /// update: the tiles the player explored that its restriction lets it stand on, the objects
    /// the world holds on them, its reach by the planners (slot 0x3d0 with the range 0).
    fn resource_search_from(
        &mut self,
        p: usize,
        i: usize,
        kind: u8,
    ) -> Result<(i32, Option<i32>), GenieError> {
        let unit = &self.players[p].objects[i];
        let master = self.master_of(p, unit.life.master_id)?;
        let mover = unit.mover.as_ref().ok_or(GenieError::unsupported(
            "a resource search without movement state",
        ))?;
        let ai = unit.ai.as_ref().ok_or(GenieError::unsupported(
            "a resource search without a unit AI",
        ))?;
        let seeker = gather_search::Seeker {
            player: p,
            position: unit.position,
            line_of_sight: master.line_of_sight,
            state: unit.life.state,
            current_target: ai.current_target,
            my_pasture: unit.my_pasture,
            min_state: 2,
        };
        let me_walk = Me {
            id: unit.id,
            position: unit.position,
            group: unit.group.id,
            action_state: unit.top_action().map_or(0, |a| a.state),
        };
        let (mask, master_id) = (mover.stats.restriction_mask, unit.life.master_id);
        let (players, masters, spatial) = (&self.players, &self.masters, &self.spatial);
        let candidate = |id: i32| -> Result<Candidate, GenieError> {
            let (owner, u) = players
                .iter()
                .enumerate()
                .find_map(|(q, player)| {
                    player
                        .objects
                        .iter()
                        .chain(&player.sleeping)
                        .find(|u| u.id == id)
                        .map(|u| (q, u))
                })
                .ok_or(GenieError::unsupported(
                    "an object on a tile the slice does not hold",
                ))?;
            let m = masters
                .get(owner)
                .and_then(|m| m.get(u.life.master_id))
                .ok_or(crate::world::OBJECT_MASTER)?;
            let position = spatial.get(id).map_or(u.position, |o| o.position);
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
        let objects = |x: i32, y: i32| -> Result<Vec<Candidate>, GenieError> {
            spatial
                .objects_on(x, y)
                .map(|(id, _)| candidate(id))
                .collect()
        };
        let mut plan = Planning {
            pathing: &mut self.pathing,
            now: self.world_time,
            computer: self.players[p].computer,
            object: self.streams.get(Stream::Object),
            construction_rate: self.players[p].construction_rate,
            groups: &self.groups,
        };
        let blocked = u64::from(mask ^ 0xff) | crate::map::spatial::BLOCKED;
        gather_search::closest_resource_at(
            &seeker,
            None,
            kind,
            -1,
            spatial.map,
            &|x, y| spatial.explored(p, x, y),
            &|x, y| spatial.tiles().word(x, y).is_some_and(|w| w & blocked == 0),
            &objects,
            &mut |id| mover.reaches_object(&me_walk, spatial, &mut plan, id, 0.0, master_id),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `BaseWorld::update`'s first update per object: a villager (4) or a fishing ship (21)
    /// works; else a building whose master has a task of action type 0x99 (a mule cart,
    /// 431096717's four carts of player 2) searches; anything else takes nothing.
    #[test]
    fn the_start_pass_works_villagers_and_lets_mule_carts_search() {
        assert_eq!(start_of(VILLAGER, false, false), Some(Start::Work));
        assert_eq!(start_of(FISHING_SHIP, false, false), Some(Start::Work));
        assert_eq!(start_of(99, true, true), Some(Start::CartSearch));
        assert_eq!(start_of(99, true, false), None);
        assert_eq!(start_of(99, false, true), None);
        assert_eq!(start_of(99, false, false), None);
    }
}
