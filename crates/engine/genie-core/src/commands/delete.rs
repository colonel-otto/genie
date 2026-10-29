//! A player's deletion of one of its objects: `CANCEL_BUILD` (0x6a, `TRIBE_Command::do_command`
//! case 0x6a), which the game's delete key sends.
//!
//! The command checks the object's owner and whether it may be deleted (`FUN_1416257c0`), then
//! runs its slot 0x8b8: a unit's (`FUN_14120f620`) takes its hit points (slot 0xd8), a building's
//! (`FUN_1411cd780` -> `FUN_1411ccbb0`) gives back what is left of a foundation's cost and takes
//! its hit points too. The object dies in its next update.

use crate::GenieError;
use crate::construction::build::held;
use crate::objects::ObjectId;
use crate::objects::masters::Master;
use crate::orders::Place;
use crate::players::PlayerId;
use crate::world::World;

/// The classes no delete takes (`DAT_142f925c0`, and `FUN_14120f620`'s own test): livestock,
/// class 42, artifacts and class 60.
const UNDELETABLE: [i16; 4] = [58, 42, 1, 60];

/// The first master type a delete takes (`FUN_1416257c0`: above `0x45`).
const DELETABLE_TYPE: u8 = 70;

/// The buildings' master type, whose slot 0x8b8 is the building's.
const BUILDING: u8 = 80;

/// The first master type whose slot 0xd0 is the action object's (`FUN_14106c420`), and the
/// TRIBE combat objects' type, whose slot 0xa0 is `FUN_14121cc40`.
const ACTION_OBJECT: u8 = 40;
const TRIBE_COMBAT: u8 = 70;

/// A player's game status once it lost (`+0x89`).
const LOST: u8 = 2;

/// A town centre's master whose delete also tells the world's `+0x820` (`FUN_1414615d0`).
const TOWN_CENTRE_621: i16 = 621;

/// A capture flag (`+0x924`) under which no delete takes the object.
const BEING_CAPTURED: u8 = 2;

/// The class a unit's slot 0xd8 (`FUN_141211340`) spares, and the livestock it spares while it
/// holds some.
const SPARED_CLASS: i16 = 42;
pub(crate) const LIVESTOCK: i16 = 58;

/// `CANCEL_BUILD` (0x6a, `do_command` case 0x6a): the player deletes its object `id`
/// ([`World::delete`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeleteCommand {
    /// The player.
    pub player: PlayerId,
    /// The object.
    pub id: ObjectId,
}

impl DeleteCommand {
    /// Its handler ([`World::delete`]).
    pub(crate) fn run(self, w: &mut World) -> Result<(), GenieError> {
        w.delete(self.player, self.id)
    }
}

impl World {
    /// `CANCEL_BUILD` for `player` on object `id`: the object must exist and be the player's,
    /// unless the setting `+0x1074` lets players order each other's ([`crate::world::settings::Settings::cheats_enabled`])
    /// or its owner lost and the two are allies both ways (`FUN_14162ef10`). `FUN_1416257c0`
    /// then wants its deletion enabled (`+0x687`), a master of type 70 or more, a class other
    /// than [`UNDELETABLE`]'s and a capture flag (`+0x924`) other than 2; the object's slot 0x8b8
    /// follows ([`World::delete_unit`], [`World::delete_building`]). The bypass of the
    /// disable-verification cheat (`+0xb12`) is not held.
    ///
    /// # Errors
    ///
    /// What the deletes do not model.
    pub(crate) fn delete(&mut self, player: PlayerId, id: ObjectId) -> Result<(), GenieError> {
        let player = player.index();
        let id = id.get();
        let Some((owner, unit)) = self.object(id) else {
            return Ok(());
        };
        let mutual =
            |a: usize, b: usize| self.spatial.allied_with(a, b) && self.spatial.allied_with(b, a);
        let lost_ally = self.players[owner].game_status == LOST && mutual(player, owner);
        if owner != player && !self.settings.cheats_enabled && !lost_ally {
            return Ok(());
        }
        let master = unit.master(&self.masters, owner)?;
        if unit.undeletable
            || master.kind < DELETABLE_TYPE
            || UNDELETABLE.contains(&master.class)
            || unit.capture_flag == BEING_CAPTURED
        {
            return Ok(());
        }
        if master.kind == BUILDING {
            self.delete_building(owner, id)
        } else {
            self.delete_unit(owner, id)
        }
    }

    /// A construction start's clearing (`FUN_1411cd0d0`) for the foundation `id`: the tiles from 5
    /// before its box to 5 past it (held on the map; rows, then columns), each tile's objects in
    /// its list's order: a desolid foundation (type 80, `+0xc60`) other than it whose owner sees
    /// and explored the tile ([`crate::map::spatial::Spatial::sees_and_explored`]) and whose box
    /// overlaps its own (on each axis the gap below the two radii summed) is removed
    /// ([`World::remove_building`]) and its `mROOAlreadyCalled` (`+0x686`) set.
    pub(crate) fn clear_foundations_under(&mut self, id: i32) -> Result<(), GenieError> {
        const AROUND: i32 = 5;
        let Some(site) = self.spatial.get(id).copied() else {
            return Ok(());
        };
        let [x, y, _] = site.position;
        let [rx, ry] = site.radius;
        let [w, h] = self.spatial.map;
        let (x0, y0) = (
            (((x - rx) as i32) - AROUND).max(0),
            (((y - ry) as i32) - AROUND).max(0),
        );
        let x1 = if ((x + rx) as i32 + AROUND) < w {
            (x + rx) as i32 + AROUND
        } else {
            w - 1
        };
        let y1 = if ((y + ry) as i32 + AROUND) < h {
            (y + ry) as i32 + AROUND
        } else {
            h - 1
        };
        let mut gone = Vec::new();
        for ty in y0..=y1 {
            for tx in x0..=x1 {
                for (oid, o) in self.spatial.objects_on(tx, ty) {
                    if oid == id || gone.iter().any(|&(_, g)| g == oid) {
                        continue;
                    }
                    let Some((owner, u)) = self.object(oid) else {
                        continue;
                    };
                    let kind = u.master(&self.masters, owner)?.kind;
                    let desolid = u.construction.is_some_and(|c| c.desolid);
                    if kind != BUILDING
                        || !desolid
                        || !self.spatial.sees_and_explored(owner, tx, ty)
                    {
                        continue;
                    }
                    let dx = (o.position[0] - x).abs();
                    let dy = (o.position[1] - y).abs();
                    if dx < o.radius[0] + rx && dy < ry + o.radius[1] {
                        gone.push((owner, oid));
                    }
                }
            }
        }
        for (owner, oid) in gone {
            self.remove_building(owner, oid)?;
            // its record went with it (`+0x686` set)
            if let Some((_, u)) = self.object_mut(oid) {
                u.roo_called = true;
            }
        }
        Ok(())
    }

    /// A unit's slot 0x8b8 (`FUN_14120f620`): a unit of a class other than [`UNDELETABLE`]'s is
    /// marked deleted (`+0xa0c`) and its slot 0xd8 (`FUN_141211340`) spares class 42 and
    /// livestock holding some, else `FUN_14118f580`: a sleeping unit wakes to the end of its
    /// player's live list and its hit points go to 0. Not held: the deleted mark, the killer
    /// (`+0x690`, its owner) and the special-object list (`+0x30c` bit 0).
    fn delete_unit(&mut self, owner: usize, id: i32) -> Result<(), GenieError> {
        let (_, unit) = self.object(id).expect("the caller's");
        let class = unit.master(&self.masters, owner)?.class;
        if UNDELETABLE.contains(&class) {
            return Ok(());
        }
        if class == SPARED_CLASS || (class == LIVESTOCK && 0.0 < unit.life.amount) {
            return Ok(());
        }
        let i = self.wake(id);
        self.players[owner].objects[i].life.hp = 0.0;
        Ok(())
    }

    /// A building's slot 0x8b8 (`FUN_1411cd780`, then `FUN_1411ccbb0`, [`World::remove_building`]):
    /// it wakes; a foundation
    /// (below state 2) with at least 1 hit point gives its owner back the share of its paid cost
    /// (`+0x880`: each entry with a resource and the paid flag, slot 0xf8) its build time has
    /// still to go (`(build time - build points) / build time`, at most 1), all of it while no
    /// builder started it (`+0xc60`); then its hit points go to 0, and such a foundation is
    /// destroyed at once ([`World::destroy_foundation`]); master 621 clears its owner's treaty
    /// zones first ([`World::clear_treaty_zones`]). Not held: the deleted marks (`+0xa0c`, `+0xc`, `+0xb9c`), the
    /// semi-asleep list (`+0xc98`), the killer (`+0x690`) and a computer player's AI event (slot
    /// 0x380).
    fn delete_building(&mut self, owner: usize, id: i32) -> Result<(), GenieError> {
        let (_, unit) = self.object(id).expect("the caller's");
        let master = unit.master(&self.masters, owner)?;
        if master.id == TOWN_CENTRE_621 {
            self.clear_treaty_zones()?;
        }
        self.remove_building(owner, id)
    }

    /// `FUN_1411ccbb0`, the core of a building's delete ([`World::delete_building`]) and of a
    /// construction start's clearing ([`World::clear_foundations_under`]): the building wakes,
    /// a foundation's paid cost comes back by the share its build time has still to go, its hit
    /// points go to 0 and a desolid foundation is destroyed at once.
    pub(crate) fn remove_building(&mut self, owner: usize, id: i32) -> Result<(), GenieError> {
        let (_, unit) = self.object(id).ok_or(GenieError::unsupported(
            "a removed building the slice does not hold",
        ))?;
        let master = unit.master(&self.masters, owner)?;
        let cost = master.cost.clone();
        let (state, hp) = (unit.life.state, unit.life.hp);
        let site = unit.construction;
        let desolid = site.is_some_and(|c| c.desolid);
        let share = if state < 2 && 1.0 <= hp {
            let c = site.ok_or(crate::construction::build::NO_CONSTRUCTION)?;
            let build_time = f32::from(c.build_time);
            (build_time - c.progress) / build_time
        } else {
            0.0
        };
        // at most 1 (a NaN keeps the 1)
        let mut share = if share <= 1.0 { share } else { 1.0 };
        if desolid && 0.0 < hp {
            share = 1.0;
        }
        if 0.0 < share {
            for (resource, amount, paid) in cost {
                if 0 <= resource && paid {
                    self.players[owner].add_attribute(resource, f32::from(amount) * share)?;
                }
            }
        }
        let i = self.wake(id);
        self.players[owner].objects[i].life.hp = 0.0;
        if state < 2 && desolid {
            self.destroy_foundation(owner, id)?;
        }
        Ok(())
    }

    /// A foundation's destroy (slot 0xd0, [`World::destroy`]) from its delete.
    fn destroy_foundation(&mut self, owner: usize, id: i32) -> Result<(), GenieError> {
        self.destroy(owner, id)
    }

    /// Slot 0xd0 of object `id` of player `owner`, by its master's type. A building's
    /// (`FUN_1411cfb30`): in state 2 what its storage gave its owner leaves
    /// ([`World::storage_leaves`]); a town-centre foundation (621) while its owner has no town
    /// centre (`FUN_1416597e0`, the player's object tracker for masters 109 and 444; the slice
    /// asks its live lists) clears its owner's treaty zones ([`World::clear_treaty_zones`]);
    /// then the action object's. The action object's (types 40 to
    /// 79 and the buildings, `FUN_14106c420`): its action list goes (`FUN_140a98620`,
    /// [`World::clear_actions`]), then the base destroy ([`World::base_destroy`]). A building's
    /// owner's watched list lets it go last (`FUN_141091310`). Not held: the semi-asleep flag
    /// (`+0xc98`, `FUN_14107db90`), `+0xca4` and the log lines.
    ///
    /// # Errors
    ///
    /// An object the slice does not hold, and what the parts do not model.
    pub(crate) fn destroy(&mut self, owner: usize, id: i32) -> Result<(), GenieError> {
        let (_, u) = self
            .object(id)
            .ok_or(crate::construction::build::NOT_HELD)?;
        let state = u.life.state;
        let master = u.master(&self.masters, owner)?.clone();
        if master.kind == BUILDING {
            if state == 2 {
                self.storage_leaves(owner, id, &master)?;
            }
            if master.id == TOWN_CENTRE_621 && !self.has_town_centre(owner) {
                self.clear_treaty_zones()?;
            }
        }
        if ACTION_OBJECT <= master.kind
            && let Some(Place::Live(p, i)) = self.place(id)
        {
            self.clear_actions(p, i)?;
        }
        self.base_destroy(owner, id)?;
        if master.kind == BUILDING {
            self.unwatch(owner, id);
        }
        Ok(())
    }

    /// `FUN_1414615d0` on the world's treaty monitor (`+0x820`) for a town centre's owner: its
    /// zones (`+0x18 + (owner − 1) × 0x24`: two rectangles and a flag) go to 0, and the game
    /// screen's list of them (`FUN_1414d2060`, `Game/ui/GameScreenPC.cpp`) loses its entries. Only
    /// a treaty fills the zones (`FUN_141461c00`), so without one they are 0 already.
    ///
    /// # Errors
    ///
    /// A game with a treaty ([`crate::world::settings::Settings::treaty_length`]), whose zones the slice
    /// does not hold.
    const fn clear_treaty_zones(&self) -> Result<(), GenieError> {
        if self.settings.treaty_length != 0 {
            return Err(GenieError::unsupported(
                "a deleted town centre 621 during a treaty (its owner's treaty zones)",
            ));
        }
        Ok(())
    }

    /// `FUN_1416597e0(owner, 0)`: the owner's tracker files a town centre the world holds
    /// ([`World::town_centre_at`]).
    fn has_town_centre(&self, owner: usize) -> bool {
        self.town_centre_at(owner, 0).is_some()
    }

    /// The base destroy (`FUN_14118f1e0`) of object `id` of player `owner`: below state 7 its
    /// hit points go to 0, it wakes (`FUN_14119deb0` with 0) and goes to state 7 (slot 0xa0,
    /// [`World::set_state`]). An object inside another would leave it first (slot 0x1a8): not
    /// modelled. Not held: the special-object list (`+0x30c` bit 0, `FUN_141677380`) and the log
    /// line.
    pub(crate) fn base_destroy(&mut self, owner: usize, id: i32) -> Result<(), GenieError> {
        let (_, u) = self
            .object(id)
            .ok_or(crate::construction::build::NOT_HELD)?;
        if 7 <= u.life.state {
            return Ok(());
        }
        if u.life.inside.is_some() {
            return Err(GenieError::unsupported(
                "a destroyed object inside another (slot 0x1a8)",
            ));
        }
        let i = self.wake(id);
        self.players[owner].objects[i].life.hp = 0.0;
        self.set_state(owner, id, 7)
    }

    /// Slot 0xa0 of object `id` of player `owner` to `new`, a state of 3 or more (2 for a
    /// building's completion), outside the
    /// object's own update, by its master's type: a building's (`FUN_1411dbb70`,
    /// [`World::building_set_state`]), a combat object's of type 70 (`FUN_14121cc40`,
    /// [`World::combat_set_state`]), else the base one ([`World::base_set_state`]).
    ///
    /// # Errors
    ///
    /// What the parts do not model.
    pub(crate) fn set_state(&mut self, owner: usize, id: i32, new: u8) -> Result<(), GenieError> {
        let (_, u) = self.object(id).ok_or(GenieError::unsupported(
            "a set_state of an object the slice does not hold",
        ))?;
        let master = u.master(&self.masters, owner)?.clone();
        match master.kind {
            BUILDING => self.building_set_state(owner, id, new, &master),
            TRIBE_COMBAT => self.combat_set_state(owner, id, new, &master),
            _ => self.base_set_state(owner, id, new, &master),
        }
    }

    /// The building's `set_state` (`FUN_1411dbb70`) to a state of 3 or more: its annexes first
    /// (`+0xbc0`..`+0xbd8`, each a building: their own slot 0xa0 with the same state); a
    /// foundation (state 0) then runs the completion's first part ([`World::completion_part`],
    /// `FUN_1411c6bd0`) and, still in state 0 and going to a state from 1 to 6, its
    /// construction's start (`FUN_1411e5670`, [`World::construction_starts`]); a built building
    /// going to state 3 dies ([`World::built_building_dies`], which ends in the combat object's
    /// `set_state`); any other goes to the combat object's ([`World::combat_set_state`]).
    fn building_set_state(
        &mut self,
        owner: usize,
        id: i32,
        new: u8,
        master: &Master,
    ) -> Result<(), GenieError> {
        for annex in self.spatial.annexes_of(id) {
            if let Some((q, a)) = self.object(annex)
                && a.master(&self.masters, q)?.kind == BUILDING
            {
                self.set_state(q, annex, new)?;
            }
        }
        let state = self.object(id).map_or(8, |(_, u)| u.life.state);
        // the completion: its first part, its construction's start and the block for state 2
        if new == 2 && state == 0 {
            return self.complete_building(id);
        }
        if state == 0 {
            self.completion_part(owner, id, master)?;
            let still = self.object(id).is_some_and(|(_, u)| u.life.state == 0);
            if still && (1..=6).contains(&new) {
                self.construction_starts(owner, id)?;
            }
        }
        if new == 3 && state == 2 {
            return self.built_building_dies(owner, id, master);
        }
        self.combat_set_state(owner, id, new, master)
    }

    /// The combat object's `set_state` (`FUN_14121cc40`) to a state of 3 or more: from below 3
    /// its death's part ([`crate::objects::lifecycle::death_parts::combat_dies`]); to state 3 from 2 its owner's attributes
    /// lose its paid cost ([`crate::objects::lifecycle::death_parts::refund_of`]: 98 and 164, a building 98 and 165, each at
    /// least 0); then the base `set_state` ([`World::base_set_state`]).
    fn combat_set_state(
        &mut self,
        owner: usize,
        id: i32,
        new: u8,
        master: &Master,
    ) -> Result<(), GenieError> {
        // an object the slice does not hold fails in the base set_state
        let Some(place) = self.place(id) else {
            return self.base_set_state(owner, id, new, master);
        };
        let u = held(&mut self.players, place);
        let (state, main) = (u.life.state, u.annex_of);
        if 3 <= new && state < 3 {
            crate::objects::lifecycle::death_parts::combat_dies(
                u,
                master,
                &mut self.spatial,
                &mut self.pathing,
            )?;
            self.pathing.tiles_changed(self.spatial.take_changes());
        }
        if new == 3 && state == 2 {
            for (kind, amount) in crate::objects::lifecycle::death_parts::refund_of(master, main) {
                let player = &mut self.players[owner];
                player.add_attribute(kind, -amount)?;
                if player.attributes[kind as usize] < 0.0 {
                    player.attributes[kind as usize] = 0.0;
                }
            }
        }
        self.base_set_state(owner, id, new, master)
    }

    /// The base `set_state` (`FUN_14119d710`) to `new`, a state of 3 or more, outside the
    /// object's update: a lower state than its own changes nothing; from below 3 the storage of
    /// flags 2 and 16 leaves its owner (`FUN_1411a56c0` with 0) and, while the world's game state
    /// is not 2, a master of fog visibility 2 leaves its ghosts (`FUN_14118e990`,
    /// [`World::ghosts_at_death`], before the state changes); from below 4 to above 6 it leaves
    /// the visible-resource lists (`FUN_1411980b0`, [`crate::objects::lifecycle::death_parts::forget_resource`]); its sight
    /// follows ([`crate::map::spatial::Spatial::sight_state`]); from below 7 to 7 the storage of flag
    /// 32 leaves its owner; then the state, and a sleeping object wakes unless the state is 2 or
    /// 6. Not held: the statistics and the scenario's events.
    fn base_set_state(
        &mut self,
        owner: usize,
        id: i32,
        new: u8,
        master: &Master,
    ) -> Result<(), GenieError> {
        let (_, u) = self.object(id).ok_or(GenieError::unsupported(
            "a set_state of an object the slice does not hold",
        ))?;
        let (old, position) = (u.life.state, u.position());
        if old == new || (new < old && new != 0) {
            return Ok(());
        }
        if 2 < new && old < 3 {
            for &(kind, amount, flag) in &master.storage {
                if 0 <= kind && matches!(flag, 2 | 16) {
                    self.players[owner].add_attribute(kind, -amount)?;
                }
            }
            if master.ghost_on_death == crate::vision::ghost::LEAVES_GHOSTS && self.game_state != 2
            {
                self.ghosts_at_death(owner, id)?;
            }
        }
        if old < 4 && 6 < new {
            crate::objects::lifecycle::death_parts::forget_resource(
                &mut self.spatial,
                Some(master),
                id,
                position,
            );
        }
        self.spatial.sight_state(id, new);
        if old < 7 && new == 7 {
            for &(kind, amount, flag) in &master.storage {
                if 0 <= kind && flag == 32 {
                    self.players[owner].add_attribute(kind, -amount)?;
                }
            }
        }
        if let Some((_, u)) = self.object_mut(id) {
            u.life.state = new;
        }
        if !matches!(new, 2 | 6) && matches!(self.place(id), Some(Place::Sleeping(..))) {
            let _ = self.wake(id);
        }
        Ok(())
    }

    /// A built building's death (`FUN_1411dbb70` to state 3 from 2) outside its update: its
    /// part ([`crate::objects::lifecycle::death_parts::built_death`]), then the combat object's `set_state`; the count it
    /// lowers and its technology follow (as its own update's caller does them).
    fn built_building_dies(
        &mut self,
        owner: usize,
        id: i32,
        master: &Master,
    ) -> Result<(), GenieError> {
        let computer = self.players[owner].computer;
        let (_, u) = self
            .object_mut(id)
            .ok_or(crate::construction::build::NOT_HELD)?;
        let death = crate::objects::lifecycle::death_parts::built_death(u, master, computer)?;
        for &(kind, amount) in &death.adds {
            self.players[owner].add_attribute(kind, amount)?;
        }
        self.unwatch(owner, id);
        if let (Some(v), Some(at)) = (self.spatial.sight.as_mut(), death.unlink) {
            v.unlink_building(at, master.radius);
        }
        self.combat_set_state(owner, id, 3, master)?;
        if let Some(kind) = death.building.count {
            let player = &mut self.players[owner];
            player.add_attribute(kind, -1.0)?;
            if player.attributes[kind as usize] < 0.0 {
                player.attributes[kind as usize] = 0.0;
            }
        }
        if let Some((m, tech)) = death.building.tech {
            self.withdraw_tech(owner, m, tech)?;
        }
        Ok(())
    }

    /// `FUN_14119deb0` with 0: an object asleep wakes to the end of its player's live list, a
    /// live one flagged to sleep drops the flag; its index in the live list.
    pub(crate) fn wake(&mut self, id: i32) -> usize {
        match self.place(id).expect("the caller's") {
            Place::Live(p, i) => {
                self.players[p].objects[i].life.goto_sleep = false;
                i
            }
            Place::Sleeping(p, i) => {
                let mut woken = self.players[p].sleeping.swap_remove(i);
                woken.life.goto_sleep = false;
                let (id, master) = (woken.id, woken.life.master_id);
                self.players[p].objects.push(woken);
                // the remove and add (slots 0x2d8, 0x2d0) file it last in the tracker
                self.track_relist(p, id, master);
                self.players[p].objects.len() - 1
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::construction::build::Construction;
    use crate::map::spatial::Obstruction;
    use crate::objects::masters::{Master, Masters};
    use crate::objects::unit::Unit;
    use crate::test_ids::object_id;
    use crate::test_ids::player_id;
    use crate::world::Player;

    /// Gaia and two players: player 1's villager 10, house foundation 11 (build time 25, cost 25
    /// wood paid) and sheep 12, and player 2's villager 20.
    fn game(progress: f32, desolid: bool) -> World {
        let house = Unit {
            id: 11,
            construction: Some(Construction {
                progress,
                desolid,
                builders: 0,
                hit_points: 550,
                build_time: 25,
                record: Obstruction { kind: 2, class: 0 },
                snow: false,
                sprite_override: false,
            }),
            ..Unit::default()
        };
        let mut t = World::new(
            0,
            1,
            0,
            vec![0; 3],
            vec![
                Player::default(),
                Player {
                    objects: vec![
                        Unit {
                            id: 10,
                            ..Unit::default()
                        },
                        house,
                        Unit {
                            id: 12,
                            ..Unit::default()
                        },
                    ]
                    .into(),
                    attributes: vec![0.0; 10],
                    ..Player::default()
                },
                Player {
                    objects: vec![Unit {
                        id: 20,
                        ..Unit::default()
                    }]
                    .into(),
                    ..Player::default()
                },
            ],
        );
        for (p, objects) in [
            (1, [(0, 83), (1, 70), (2, 594)].as_slice()),
            (2, &[(0, 83)]),
        ] {
            for &(i, master) in objects {
                let u = &mut t.players[p].objects[i];
                u.life.master_id = master;
                u.life.hp = 25.0;
                u.pasture = -1;
                u.my_pasture = -1;
            }
        }
        t.players[1].objects[1].life.state = 0;
        t.players[1].objects[2].life.amount = 100.0;
        let masters = Masters::new([
            Master {
                id: 83,
                class: 4,
                kind: 70,
                ..Master::default()
            },
            Master {
                id: 70,
                class: 3,
                kind: 80,
                cost: vec![(1, 25, true), (4, 1, false)],
                ..Master::default()
            },
            Master {
                id: 594,
                class: 58,
                kind: 70,
                ..Master::default()
            },
        ]);
        t.masters = vec![masters.clone(), masters.clone(), masters];
        t
    }

    #[test]
    fn a_delete_takes_a_unit_s_hit_points_but_not_another_player_s_or_livestock() {
        let mut t = game(0.0, true);
        t.delete(player_id(1), object_id(10)).unwrap();
        t.delete(player_id(1), object_id(20)).unwrap();
        t.delete(player_id(1), object_id(12)).unwrap();
        let hp = |t: &World, p: usize, i: usize| t.players[p].objects[i].life.hp;
        assert_eq!(
            (hp(&t, 1, 0), hp(&t, 2, 0), hp(&t, 1, 2)),
            (0.0, 25.0, 25.0)
        );
        // the setting +0x1074 lets a player delete another's
        t.settings.cheats_enabled = true;
        t.delete(player_id(1), object_id(20)).unwrap();
        assert_eq!(hp(&t, 2, 0), 0.0);
        // a unit whose deletion is disabled stays
        let mut t = game(0.0, true);
        t.players[1].objects[0].undeletable = true;
        t.delete(player_id(1), object_id(10)).unwrap();
        assert_eq!(hp(&t, 1, 0), 25.0);
    }

    #[test]
    fn a_deleted_foundation_gives_back_what_its_build_time_has_still_to_go() {
        // no builder started it: all of its paid cost, and it is destroyed at once (state 7)
        let mut t = game(0.0, true);
        t.delete(player_id(1), object_id(11)).unwrap();
        let house = &t.players[1].objects[1].life;
        assert_eq!(
            (t.players[1].resources[1], house.hp, house.state),
            (25.0, 0.0, 7)
        );
        // 10 of its 25 seconds of work done: 15/25 of the wood (in f32: 15.000001); it dies in
        // its own update
        let mut t = game(10.0, false);
        t.delete(player_id(1), object_id(11)).unwrap();
        let house = &t.players[1].objects[1].life;
        let share = (25.0_f32 - 10.0) / 25.0;
        assert_eq!(
            (t.players[1].resources[1], house.hp, house.state),
            (25.0 * share, 0.0, 0)
        );
    }

    /// A town-centre foundation (master 621) deleted, and destroyed at once: both clear its
    /// owner's treaty zones in the world's treaty monitor (`+0x820`, `FUN_1414615d0`, from
    /// `FUN_1411cd780` and `FUN_1411cfb30`), which only a treaty fills (`FUN_141461c00`): without
    /// one the delete is any building's; with one the zones are not held.
    #[test]
    fn a_deleted_town_centre_foundation_clears_no_zones_without_a_treaty() {
        let town_centre = |t: &mut World| {
            for m in &mut t.masters {
                let mut tc = m.get(70).unwrap().clone();
                tc.id = TOWN_CENTRE_621;
                *m = Masters::new(m.iter().cloned().chain([tc]));
            }
            t.players[1].objects[1].life.master_id = TOWN_CENTRE_621;
        };
        let mut t = game(0.0, true);
        town_centre(&mut t);
        t.delete(player_id(1), object_id(11)).unwrap();
        let tc = &t.players[1].objects[1].life;
        assert_eq!((t.players[1].resources[1], tc.hp, tc.state), (25.0, 0.0, 7));
        let mut t = game(0.0, true);
        town_centre(&mut t);
        t.settings.treaty_length = 10;
        assert_eq!(
            t.delete(player_id(1), object_id(11)),
            Err(GenieError::unsupported(
                "a deleted town centre 621 during a treaty (its owner's treaty zones)"
            ))
        );
    }
}

#[cfg(test)]
mod clearing_tests {
    use super::*;
    use crate::map::spatial::{Placed, Spatial, Tiles};
    use crate::objects::lifecycle::{Class, Lifecycle};
    use crate::objects::masters::{Master, Masters};
    use crate::objects::unit::Unit;
    use crate::world::Player;

    /// A live object `id` of `master` at `x`, `y`, outside any pasture.
    fn object(id: i32, master: i16, x: f32, y: f32) -> Unit {
        Unit {
            id,
            body: crate::objects::layers::Body::at([x, y, 0.0]),
            life: Lifecycle {
                master_id: master,
                ..Lifecycle::default()
            },
            pasture: -1,
            my_pasture: -1,
            ..Unit::default()
        }
    }

    /// Its place on the map, from its master.
    fn placed(u: &Unit, m: &Master, player: usize) -> Placed {
        Placed {
            position: u.position(),
            radius: m.radius,
            class: m.class,
            player,
            master: m.id,
            obstruction_type: m.obstruction_type,
            buildable_on: m.buildable_on,
            tall: m.tall,
            speed: 0.0,
            unknown: None,
            obstruction: None,
        }
    }

    /// The masters: a house (70), two decorations Gaia's map scatters (1351, 1358; type 10,
    /// buildable on), a dead villager (type 30, buildable on, its flag-32 storage of attribute
    /// 7), a pile (type 70, buildable on, tall, with a record) and a dead farm (type 80, class
    /// 11, buildable on).
    fn masters() -> Masters {
        let deco = |id| Master {
            id,
            kind: 10,
            class: 5,
            radius: [0.5, 0.5],
            buildable_on: 1,
            ..Master::default()
        };
        Masters::new([
            Master {
                id: 70,
                kind: 80,
                class: 3,
                radius: [1.0, 1.0],
                ..Master::default()
            },
            deco(1351),
            deco(1358),
            Master {
                id: 43,
                kind: 30,
                class: 11,
                radius: [0.2, 0.2],
                buildable_on: 1,
                storage: vec![(7, 1.0, 32)],
                ..Master::default()
            },
            Master {
                id: 254,
                kind: 70,
                class: 40,
                radius: [0.5, 0.5],
                height: 1.0,
                tall: true,
                obstruction_type: 2,
                buildable_on: 1,
                ..Master::default()
            },
            Master {
                id: 357,
                kind: 80,
                class: 11,
                radius: [1.5, 1.5],
                buildable_on: 1,
                building: Some(crate::objects::masters::BuildingMaster {
                    stack_unit: -1,
                    foundation_terrain: -1,
                    tech: -1,
                    annexes: [-1; 4],
                    head_unit: -1,
                    transform_unit: -1,
                    ..crate::objects::masters::BuildingMaster::default()
                }),
                ..Master::default()
            },
        ])
    }

    /// Gaia holding `gaia` (live) and `asleep`, player 1 holding `own`, on a 20 by 20 map.
    fn world(gaia: Vec<Unit>, asleep: Vec<Unit>, own: Vec<Unit>) -> World {
        let ms = masters();
        let mut t = World::new(
            0,
            1,
            0,
            vec![0; 2],
            vec![
                Player {
                    objects: gaia.into(),
                    sleeping: asleep.into(),
                    attributes: vec![0.0; 200],
                    ..Player::default()
                },
                Player {
                    objects: own.into(),
                    attributes: vec![0.0; 200],
                    ..Player::default()
                },
            ],
        );
        t.spatial = Spatial::with_tiles([20, 20], Tiles::new(20, 20, vec![0; 400]));
        for p in 0..2 {
            let all: Vec<Unit> = t.players[p]
                .objects
                .iter()
                .chain(&t.players[p].sleeping)
                .cloned()
                .collect();
            for u in all {
                let m = ms.get(u.life.master_id).expect("a test master");
                let mut at = placed(&u, m, p);
                if m.obstruction_type == 2 {
                    at.obstruction = Some(crate::map::spatial::Obstruction { kind: 2, class: 0 });
                }
                t.spatial.insert(u.id, at);
            }
        }
        t.masters = vec![ms.clone(), ms];
        t
    }

    /// records/429859027's update 19 (and 161 more stops of census v5): a foundation's
    /// constructor clears two of Gaia's sleeping decorations off one tile (`FUN_1415c3620`): each
    /// in its tile's list order is destroyed (`FUN_14118f1e0`): its hit points go, it wakes to
    /// the end of Gaia's live list and goes to state 7.
    #[test]
    fn a_footprint_clearing_wakes_two_sleeping_decorations_in_their_tile_s_order() {
        let mut t = world(
            vec![object(1, 1351, 3.5, 3.5)],
            vec![
                object(6084, 1358, 10.6, 10.6),
                object(6083, 1351, 10.4, 10.4),
            ],
            vec![],
        );
        assert_eq!(t.spatial.tiles().list(10, 10), [6084, 6083]);
        t.clear_footprint(1, 70, [10.5, 10.5], false).unwrap();
        let ids: Vec<i32> = t.players[0].objects.iter().map(|u| u.id).collect();
        assert_eq!(ids, [1, 6084, 6083]);
        assert_eq!(t.players[0].sleeping.len(), 0);
        for u in &t.players[0].objects[1..] {
            assert_eq!((u.life.state, u.life.hp), (7, 0.0));
        }
        assert_eq!(t.players[0].objects[0].life.state, 2);
    }

    /// records/428477858's update 1838: a foundation clears a dead villager (type 30) in state 5
    /// off its footprint: the base destroy takes it to state 7 and its storage of flag 32 leaves
    /// its owner (`FUN_14119d710`).
    #[test]
    fn a_cleared_corpse_goes_to_state_7_and_its_flag_32_storage_leaves() {
        let mut corpse = object(30, 43, 10.5, 10.5);
        corpse.life.state = 5;
        corpse.life.class = Class::Animated;
        let mut t = world(vec![], vec![], vec![corpse]);
        t.players[1].attributes[7] = 3.0;
        t.clear_footprint(1, 70, [10.5, 10.5], false).unwrap();
        let u = &t.players[1].objects[0];
        assert_eq!((u.life.state, u.life.hp), (7, 0.0));
        assert_eq!(t.players[1].attributes[7], 2.0);
    }

    /// A pile (type 70) a new object's clearing (`param_6`) destroys: its action list goes
    /// (`FUN_14106c420`), and the combat object's `set_state(7)` (`FUN_14121cc40`) drops its
    /// record and its height and sets `+0x686`; it gets no refund (only state 3 from 2 does).
    #[test]
    fn a_cleared_pile_loses_its_record_in_its_destroy() {
        let mut t = world(vec![object(40, 254, 10.5, 10.5)], vec![], vec![]);
        t.players[0].attributes[98] = 5.0;
        assert!(t.spatial.get(40).unwrap().obstruction.is_some());
        t.clear_footprint(1, 70, [10.5, 10.5], true).unwrap();
        let u = &t.players[0].objects[0];
        assert_eq!((u.life.state, u.life.hp, u.roo_called), (7, 0.0, true));
        let p = t.spatial.get(40).unwrap();
        assert_eq!((p.obstruction, p.tall), (None, false));
        assert_eq!(t.players[0].attributes[98], 5.0);
    }

    /// A dead farm (`FARM_D`, a building of type 80) a new farm's constructor clears: the
    /// building's destroy (`FUN_1411cfb30`): its storage leaves in state 2, its action list goes,
    /// the base destroy takes it to state 7 through the building's and the combat object's
    /// `set_state`, and its owner's watched list lets it go.
    #[test]
    fn a_cleared_dead_farm_goes_through_the_building_s_destroy() {
        let mut t = world(vec![], vec![], vec![object(50, 357, 10.5, 10.5)]);
        t.clear_footprint(1, 70, [10.5, 10.5], false).unwrap();
        let u = &t.players[1].objects[0];
        assert_eq!((u.life.state, u.life.hp, u.roo_called), (7, 0.0, true));
    }

    /// The base `set_state` (`FUN_14119d710`) of a decoration (type 10): the same or a lower
    /// state changes nothing; from below 3 its storage of flags 2 and 16 leaves its owner (that
    /// of flag 32 only at 7, a kind below 0 never); a sleeping object wakes to the end of the
    /// live list unless the state is 6; an object the world does not hold is not modelled.
    #[test]
    fn the_base_set_state_lets_its_storage_go_and_wakes_a_sleeper() {
        let mut t = world(
            vec![object(1, 1351, 3.5, 3.5)],
            vec![object(2, 1358, 5.5, 5.5), object(3, 1358, 7.5, 7.5)],
            vec![],
        );
        for m in [1351, 1358] {
            t.masters[0].get_mut(m).unwrap().storage =
                vec![(4, 2.0, 2), (5, 3.0, 16), (7, 1.0, 32), (-1, 9.0, 2)];
        }
        let at = |t: &World| [4, 5, 7].map(|i| t.players[0].attributes[i]);
        t.players[0].attributes[4..8].copy_from_slice(&[10.0, 10.0, 10.0, 10.0]);
        t.players[0].objects[0].life.state = 2;
        // the same state, then a lower one: nothing
        t.set_state(0, 1, 2).unwrap();
        t.players[0].objects[0].life.state = 5;
        t.set_state(0, 1, 3).unwrap();
        assert_eq!(t.players[0].objects[0].life.state, 5);
        assert_eq!(at(&t), [10.0; 3]);
        // from 2 to 5: flags 2 and 16 leave, flag 32 stays; then to 7, flag 32 leaves
        t.players[0].objects[0].life.state = 2;
        t.set_state(0, 1, 5).unwrap();
        assert_eq!(at(&t), [8.0, 7.0, 10.0]);
        t.set_state(0, 1, 7).unwrap();
        assert_eq!(at(&t), [8.0, 7.0, 9.0]);
        assert_eq!(t.players[0].objects[0].life.state, 7);
        // a sleeper set to 6 sleeps on, one set to 7 wakes
        t.players[0].sleeping[0].life.state = 5;
        t.set_state(0, 2, 6).unwrap();
        assert_eq!(t.players[0].sleeping.len(), 2);
        t.set_state(0, 3, 7).unwrap();
        let ids: Vec<i32> = t.players[0].objects.iter().map(|u| u.id).collect();
        assert_eq!((ids, t.players[0].sleeping.len()), (vec![1, 3], 1));
        assert!(t.set_state(0, 99, 7).is_err());
    }

    /// `FUN_14121cc40`'s refund at a death from state 2: a unit's paid cost leaves attributes 98
    /// and 164, a building's 98 and 165, the castle's and the wonder's 98 alone, an annex's none.
    #[test]
    fn a_death_refunds_its_paid_cost_by_its_type() {
        let unit = Master {
            kind: 70,
            cost: vec![(0, 50, true), (3, 20, true), (1, 5, false)],
            ..Master::default()
        };
        assert_eq!(
            crate::objects::lifecycle::death_parts::refund_of(&unit, None),
            [(98, 70.0), (164, 70.0)]
        );
        let house = Master {
            id: 70,
            kind: 80,
            ..unit
        };
        assert_eq!(
            crate::objects::lifecycle::death_parts::refund_of(&house, None),
            [(98, 70.0), (165, 70.0)]
        );
        assert_eq!(
            crate::objects::lifecycle::death_parts::refund_of(&house, Some(9)),
            []
        );
        let castle = Master { id: 82, ..house };
        assert_eq!(
            crate::objects::lifecycle::death_parts::refund_of(&castle, None),
            [(98, 70.0)]
        );
    }
}
