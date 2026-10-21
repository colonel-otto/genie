//! The Empire Wars start: in the game's first update every idle villager is put to work
//! (`BaseWorld::update`, `0x14154fc70`, its per-player part before the player's objects update).
//!
//! The pass runs once (the world's `+0x6c6`) when the game mode is 13, Empire Wars (in another
//! mode a lobby setting would ask for it, `FUN_141324b60` bit 0, which no recording seen so far
//! sets): each object of the player's live list, in its order, that is a villager (class 4) or a
//! fishing ship (class 21), may be interrupted with an empty order queue (slot 0x2d0 with 1) and
//! has a unit AI takes work. Under the game's farm setting a farm on its tile comes first (the
//! map tile's objects in their order: the first of class 49; a fishing ship's must be a fish
//! trap, master 199); else the closest resource of the five gather groups by the unit AI's
//! `ClosestResourceObject` (slot 0x80, [`crate::gather_search::closest_resource_at`]: no drop
//! site, any workers, the least distance wins, the first group at a tie). Slot 0x140
//! (`FUN_141070c90`) then gives the unit its work on the target at the target's place
//! ([`Turns::order_work`], slot 0x7e8; `+0x888`, the work's count, is not held).

use crate::gather_search::{self, Candidate};
use crate::movement::{Me, Planning};
use crate::order::Work;
use crate::rng::Stream;
use crate::turn::Turns;
use crate::unit_ai::Unsupported;

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

impl Turns {
    /// The pass for player `p` ([`crate::start_work`]).
    ///
    /// # Errors
    ///
    /// The pass without the game's farm setting (the player's nearest unworked farm), a fishing
    /// ship's own search (its unit AI's slot 0x80), and what the work command does not model.
    pub(crate) fn start_work(&mut self, p: usize) -> Result<(), Unsupported> {
        let mut i = 0;
        while i < self.players[p].objects.len() {
            let unit = &self.players[p].objects[i];
            let class = self.master_of(p, unit.life.master_id)?.class;
            let idle = unit.ai.as_ref().is_some_and(|ai| ai.orders.is_empty());
            if !matches!(class, VILLAGER | FISHING_SHIP) || !idle || !unit.interruptible()? {
                i += 1;
                continue;
            }
            if !self.farm_stops {
                return Err(Unsupported(
                    "the Empire Wars start without the farm setting (the nearest unworked farm)",
                ));
            }
            let target = match self.farm_under(p, i, class)? {
                Some(farm) => Some(farm),
                None if class == FISHING_SHIP => {
                    return Err(Unsupported(
                        "the Empire Wars start of a fishing ship (its unit AI's slot 0x80)",
                    ));
                }
                None => self.closest_of_groups(p, i)?,
            };
            if let Some(target) = target.filter(|&t| t != self.players[p].objects[i].id) {
                let point = self
                    .spatial
                    .get_known(target)?
                    .ok_or(Unsupported(
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
    fn farm_under(&self, p: usize, i: usize, class: i16) -> Result<Option<i32>, Unsupported> {
        let at = self.players[p].objects[i].position;
        let (x, y) = (at[0] as i32, at[1] as i32);
        self.spatial.check_known([at[0], at[1], at[0], at[1]])?;
        for (id, _) in self.spatial.objects_on(x, y) {
            let (owner, o) = self
                .object(id)
                .ok_or(Unsupported("an object on a tile the slice does not hold"))?;
            let m = self.master_of(owner, o.life.master_id)?;
            if m.class == FARM && (class != FISHING_SHIP || m.id == FISH_TRAP) {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// The unit AI's `ClosestResourceObject` for each gather group (no drop site, any workers,
    /// `param_6` 2, no point): the answer at the least distance, the first group at a tie.
    fn closest_of_groups(&mut self, p: usize, i: usize) -> Result<Option<i32>, Unsupported> {
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
    ) -> Result<(i32, Option<i32>), Unsupported> {
        let unit = &self.players[p].objects[i];
        let master = self.master_of(p, unit.life.master_id)?;
        let mover = unit
            .mover
            .as_ref()
            .ok_or(Unsupported("a resource search without movement state"))?;
        let ai = unit
            .ai
            .as_ref()
            .ok_or(Unsupported("a resource search without a unit AI"))?;
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
        let candidate = |id: i32| -> Result<Candidate, Unsupported> {
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
                .ok_or(Unsupported("an object on a tile the slice does not hold"))?;
            let m = masters
                .get(owner)
                .and_then(|m| m.get(u.life.master_id))
                .ok_or(Unsupported("an object of a master the slice does not have"))?;
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
        let objects = |x: i32, y: i32| -> Result<Vec<Candidate>, Unsupported> {
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
        let blocked = u64::from(mask ^ 0xff) | crate::spatial::BLOCKED;
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
