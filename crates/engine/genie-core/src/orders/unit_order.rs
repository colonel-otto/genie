//! An artifact's and a herd's take, an order's cancel and a unit's order.

use crate::gathering::hunt::HUNT;
use crate::math::rng::Stream;
use crate::orders::{BUILDING, HERD_MOST, LIVESTOCK, NO_POINT, Place, TOWN_CENTRE};
use crate::turn::{Turns, Unit};
use crate::unit_ai::GenieError;

impl Turns {
    /// An animal's Artifact takes a move to the object `target` (its slot 8, `FUN_140aaca30`,
    /// [`crate::ownership::artifact::Artifact::take_object`]): its old target is let go (slot 0x180,
    /// [`Turns::release`]), then the target, in a state below 7, taken (slot 0x178,
    /// [`Turns::take`]); livestock (class 58) goes to a building within 1, to the town centre
    /// (master 109) at its point; a `MoveTo` it walked goes, whose teardown stops the unit (slot
    /// 0x5d0).
    pub(crate) fn artifact_take_object(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
    ) -> Result<(), GenieError> {
        let by = self.releaser(player, idx);
        let unit = &self.players[player].objects[idx];
        let livestock = self.master_of(player, unit.life.master_id)?.class == LIVESTOCK;
        let (tp, t) = self.object(target).ok_or(GenieError::unsupported(
            "an Artifact's move to an object the slice does not hold",
        ))?;
        let alive = t.life.state < 7;
        let t_master = self.master_of(tp, t.life.master_id)?;
        let to_building = livestock && t_master.kind == BUILDING;
        let to_town_centre = livestock && t_master.id == TOWN_CENTRE;
        let at = self
            .spatial
            .get(target)
            .map(|p| p.position)
            .ok_or(GenieError::unsupported(
                "an Artifact's move to an object without a place",
            ))?;
        let unit = &mut self.players[player].objects[idx];
        let (targets, gone) = unit
            .artifact
            .as_mut()
            .map(|a| a.take_object((target, alive), at, to_building, to_town_centre))
            .unwrap_or_default();
        if let Some(t) = targets.released {
            self.release(t, by)?;
        }
        if let Some(t) = targets.taken {
            self.take(t, by);
        }
        // set_state(5)'s sub-actions went: the Enter it held lets its building or transport go
        if let Some(t) = gone.released {
            self.release(t, by)?;
        }
        if gone.stops {
            self.players[player].objects[idx]
                .mover
                .as_mut()
                .ok_or(crate::turn::WALKER_UNMOVING)?
                .stop()?;
        }
        Ok(())
    }

    /// A hunt's own livestock `sheep` sent to the drop site `site` (`0x140ac60e0` case 6): the
    /// animal's slot 0x138 to the site ([`Turns::move_to_object`]), then `FUN_140ac4c60` from it
    /// ([`Turns::herd_from`]).
    ///
    /// # Errors
    ///
    /// What the moves do not model.
    pub(crate) fn lead_herd(&mut self, p: usize, sheep: i32, site: i32) -> Result<(), GenieError> {
        self.send_to_site(p, sheep, site)?;
        let mut listed = Vec::new();
        self.herd_from(p, sheep, site, &mut listed)
    }

    /// `FUN_140ac4c60` from the animal `sheep` (`listed` the animals sent so far, a static list of
    /// 16): it joins the list; while it holds fewer than 16, the tiles within 3 of the animal's
    /// (x outer, y inner, held to the map) that player `p` sees (`FUN_141663500` 0xf), each in its
    /// list's order, give the next: livestock (class 58) of the player in state 2 whose top action
    /// has neither a target nor a point ([`Unit::top_action_idle`]) and not listed yet is sent to
    /// the site (slot 0x138) and herds from it in turn; once 16 are listed it ends.
    pub(super) fn herd_from(
        &mut self,
        p: usize,
        sheep: i32,
        site: i32,
        listed: &mut Vec<i32>,
    ) -> Result<(), GenieError> {
        listed.push(sheep);
        if HERD_MOST <= listed.len() {
            return Ok(());
        }
        // the animal's own place (+0x3c8, +0x4c8), truncated; send_to_site found it live
        let at = self.object(sheep).expect("sent to the site").1.position;
        let (x, y) = (
            crate::checkpoint::cvttss2si(at[0]),
            crate::checkpoint::cvttss2si(at[1]),
        );
        let [w, h] = self.spatial.map;
        let (x0, x1) = ((x - 3).max(0), if x + 3 < w { x + 3 } else { w - 1 });
        let (y0, y1) = ((y - 3).max(0), if y + 3 < h { y + 3 } else { h - 1 });
        for tx in x0..=x1 {
            for ty in y0..=y1 {
                if !self.spatial.sees_tile(p, tx, ty) {
                    continue;
                }
                let ids = self.spatial.tiles().list(tx, ty).to_vec();
                for id in ids {
                    let Some(Place::Live(q, i)) = self.place(id) else {
                        continue;
                    };
                    let u = &self.players[q].objects[i];
                    if q != p
                        || u.life.state != 2
                        || self.master_of(q, u.life.master_id)?.class != LIVESTOCK
                        || !u.top_action_idle()?
                        || listed.contains(&id)
                    {
                        continue;
                    }
                    self.send_to_site(p, id, site)?;
                    self.herd_from(p, id, site, listed)?;
                    if HERD_MOST <= listed.len() {
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }

    /// Slot 0x138 of player `p`'s animal `sheep` to the drop site `site` at its place.
    pub(super) fn send_to_site(
        &mut self,
        p: usize,
        sheep: i32,
        site: i32,
    ) -> Result<(), GenieError> {
        let Some(Place::Live(q, idx)) = self.place(sheep) else {
            return Err(GenieError::unsupported(
                "a led animal the slice does not hold live",
            ));
        };
        if q != p {
            return Err(GenieError::unsupported("a led animal of another player"));
        }
        let point = self
            .spatial
            .get(site)
            .map(|s| s.position)
            .ok_or(GenieError::unsupported("a drop site without a place"))?;
        self.move_to_object(p, idx, site, point)
    }

    /// `UNIT_ORDER`'s action 4 (`FUN_14161e370` case 4) to the objects `ids`: each the world
    /// holds (any player's) that is a building in state 2 cancels the `index`-th unit of its
    /// queue ([`Turns::cancel_queued`], `all` the command's `+0x1d`), then game mode 16's tail
    /// ([`Turns::cancel_tail`]).
    pub(crate) fn cancel_order(
        &mut self,
        ids: &[i32],
        index: u32,
        all: bool,
    ) -> Result<(), GenieError> {
        for &id in ids {
            let Some(place) = self.place(id) else {
                continue;
            };
            let Place::Live(p, idx) = place else {
                return Err(GenieError::unsupported("a cancel at a sleeping object"));
            };
            let unit = &self.players[p].objects[idx];
            if unit.life.state != 2 || self.master_of(p, unit.life.master_id)?.kind < 80 {
                continue;
            }
            self.cancel_queued(p, idx, index, all)?;
            self.cancel_tail(p, idx, index)?;
        }
        Ok(())
    }

    /// `UNIT_ORDER`'s order `action` to the units `ids` (`FUN_14161e370`) of `player`'s command,
    /// any player's: the drop-offs 6, 7 and 8 (one case): each unit the world holds in state 2 of
    /// a type above 39 that carries something (`+0x310` above 0). For 7 and 8 a top Hunt (110),
    /// Gather (5) or Build (101) gives it its drop-off (slot 0x78, [`HuntAction::drop_off_asked`],
    /// then `set_state(8)`: [`Unit::drop_off_in`], [`Unit::gather_drop_off_in`],
    /// [`Turns::build_drop_off_now`]); for 8 one without a target or a point records its work for
    /// the town bell first (`FUN_141206a20`) and queues the way back after (`FUN_141205ac0`). For
    /// 6, and a unit without one of these actions: [`Turns::drop_off_unasked`] (the way back for
    /// 8 alone).
    ///
    /// [`HuntAction::drop_off_asked`]: crate::gathering::hunt::HuntAction::drop_off_asked
    pub(crate) fn unit_order(
        &mut self,
        _player: usize,
        ids: &[i32],
        action: u8,
    ) -> Result<(), GenieError> {
        if !matches!(action, 6..=8) {
            return Err(GenieError::unsupported(
                "a UNIT_ORDER other than the garrison (5) and the drop-offs (6 to 8)",
            ));
        }
        // 8 records an unaimed action's work for the town bell and queues the way back after;
        // 6 asks no gather action (the search alone)
        let way_back = action == 8;
        let asks = action != 6;
        for &id in ids {
            let Some(place) = self.place(id) else {
                continue;
            };
            let Place::Live(p, idx) = place else {
                return Err(GenieError::unsupported("a UNIT_ORDER to a sleeping object"));
            };
            // any player's unit the world holds: the command's list (`FUN_140e6b750`) and the
            // order filter none by owner
            let unit = &self.players[p].objects[idx];
            let kind = self.master_of(p, unit.life.master_id)?.kind;
            if unit.life.state != 2 || kind < 40 || unit.life.amount <= 0.0 {
                continue;
            }
            if unit.unmodelled_action.is_some() {
                return Err(GenieError::unsupported(
                    "a UNIT_ORDER drop-off of an action not held",
                ));
            }
            if let Some(gather) = unit.gather.as_ref().filter(|_| {
                asks && unit.top_action_kind() == Some(crate::gathering::gather::GATHER)
            }) {
                // without a target or a point the work goes into the town-bell records first
                // (FUN_141206a20) and the way back after the drop-off (FUN_141205ac0)
                let unaimed = way_back
                    && gather.target == -1
                    && gather.point[0] == NO_POINT
                    && gather.point[1] == NO_POINT;
                let attributes = self.players[p].attributes.len();
                let asked = gather.drop_off_asked(unit.life.attribute_type, attributes);
                if !asked {
                    self.other_load(p, idx);
                }
                if unaimed {
                    self.record_town_bell(p, idx)?;
                }
                if !asked {
                    if unaimed {
                        self.queue_way_back(p, idx)?;
                    }
                    continue;
                }
                let mut unit =
                    std::mem::replace(&mut self.players[p].objects[idx], Unit::taken_out());
                let player = &self.players[p];
                let world = crate::turn::DropWorld {
                    masters: self.masters.get(p),
                    spatial: &self.spatial,
                    zones: &mut self.zones,
                    pathing: &mut self.pathing,
                    object: self.streams.get(Stream::Object),
                    world_time: self.world_time,
                    computer: player.computer,
                    construction_rate: player.construction_rate,
                    groups: &self.groups,
                };
                let result = unit.gather_drop_off_in(world, &player.objects);
                self.players[p].objects[idx] = unit;
                if let Some(site) = result? {
                    self.take(site, self.releaser(p, idx));
                }
                if unaimed {
                    self.queue_way_back(p, idx)?;
                }
                continue;
            }
            if let Some(build) = unit.build.as_ref().filter(|_| {
                asks && unit.top_action_kind() == Some(crate::construction::build::BUILD)
            }) {
                // a Build without a target or a point as the Gather's above
                let unaimed = way_back
                    && build.target == -1
                    && build.point[0] == NO_POINT
                    && build.point[1] == NO_POINT;
                let attributes = self.players[p].attributes.len();
                let asked = crate::gathering::hunt::asked_drop_off(
                    build.task.resource,
                    unit.life.attribute_type,
                    attributes,
                );
                if !asked {
                    self.other_load(p, idx);
                }
                if unaimed {
                    self.record_town_bell(p, idx)?;
                }
                if asked {
                    self.build_drop_off_now(p, idx)?;
                }
                if unaimed {
                    self.queue_way_back(p, idx)?;
                }
                continue;
            }
            let Some(hunt) = unit
                .hunt
                .as_ref()
                .filter(|_| asks && unit.top_action_kind() == Some(HUNT))
            else {
                self.drop_off_unasked(p, idx, way_back)?;
                continue;
            };
            let unaimed = way_back
                && hunt.target == -1
                && hunt.point[0] == NO_POINT
                && hunt.point[1] == NO_POINT;
            let attributes = self.players[p].attributes.len();
            let asked = hunt.drop_off_asked(unit.life.attribute_type, attributes);
            if !asked {
                self.other_load(p, idx);
            }
            if unaimed {
                self.record_town_bell(p, idx)?;
            }
            if !asked {
                if unaimed {
                    self.queue_way_back(p, idx)?;
                }
                continue;
            }
            let mut unit = std::mem::replace(&mut self.players[p].objects[idx], Unit::taken_out());
            let player = &self.players[p];
            let world = crate::turn::DropWorld {
                masters: self.masters.get(p),
                spatial: &self.spatial,
                zones: &mut self.zones,
                pathing: &mut self.pathing,
                object: self.streams.get(Stream::Object),
                world_time: self.world_time,
                computer: player.computer,
                construction_rate: player.construction_rate,
                groups: &self.groups,
            };
            let result = unit.drop_off_in(world, &player.objects);
            self.players[p].objects[idx] = unit;
            if let Some(site) = result? {
                self.take(site, self.releaser(p, idx));
            }
            if unaimed {
                self.queue_way_back(p, idx)?;
            }
        }
        Ok(())
    }
}
