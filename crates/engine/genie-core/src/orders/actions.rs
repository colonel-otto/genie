//! Taking and releasing targets, the order record, actions cleared, transformations, gates
//! and master switches.

use crate::GenieError;
use crate::math::rng::Stream;
use crate::objects::masters::Master;
use crate::objects::release::Releaser;
use crate::objects::unit::Unit;
use crate::orders::{LIVESTOCK, Place, TROPHIES, TROPHY, clear_powerups, damage_share};
use crate::unit_ai::task_finder::Diplomacy;
use crate::unit_ai::task_finder::world::World;
use crate::world::Turns;

impl Turns {
    /// Slot 0x178 on the object an action of `by` takes as its target: a building's
    /// (`FUN_1411dba70`) first lists an unallied taker of a foundation with a stack unit
    /// ([`Unit::taken`]); then the base take (`FUN_14119d3e0`): its worker count (`+0x644`)
    /// rises, to at most 550; a sleeping object (static, never a building) wakes: it leaves its
    /// player's sleeping list (slot 0x2d8, the last object taking its place) for the end of the
    /// live list (slot 0x2d0). A live object flagged to sleep only drops the flags.
    pub(crate) fn take(&mut self, id: i32, by: Releaser) {
        match self.place(id) {
            Some(Place::Live(p, i)) => {
                let world = World {
                    players: &self.players,
                    drop_sites: &self.drop_sites,
                };
                let allied = world.allied(by.player, p, true);
                let u = &self.players[p].objects[i];
                let listed = self
                    .master_of(p, u.life.master_id)
                    .is_ok_and(|m| u.lists_taker(m, allied));
                self.players[p].objects[i].taken(listed, by.id);
            }
            Some(Place::Sleeping(p, i)) => {
                let owner = &mut self.players[p];
                let u = &mut owner.sleeping[i];
                if u.workers < 550 {
                    u.workers += 1;
                }
                let woken = owner.sleeping.swap_remove(i);
                let (id, master) = (woken.id, woken.life.master_id);
                owner.objects.push(woken);
                self.track_relist(p, id, master);
            }
            None => {}
        }
    }

    /// Slot 0x180 on the object an action of `by` lets go ([`crate::objects::release::released`]): a
    /// building's (`FUN_1411d6020`) drops an unallied releaser from the attacking units of a
    /// foundation with a stack unit ([`Unit::lists_taker`]); its worker count falls; a static
    /// object (type below 30, `FUN_141197d00`) that no one works on any more and is alive (state
    /// 2 with hit points at least 0) or in state 6 is flagged to sleep (`+0x641`, `+0x643`); a
    /// combat object (`FUN_141086e60`) tells its unit AI (notify 699).
    pub(crate) fn release(&mut self, id: i32, by: Releaser) -> Result<(), GenieError> {
        let (p, i) = match self.place(id) {
            Some(Place::Live(p, i)) => (p, i),
            // a sleeping object (static: only those sleep) found by FUN_1415417a0 counts a worker
            // less (FUN_141197d00); asleep already (`+0x641`), it is not flagged again
            Some(Place::Sleeping(p, i)) => {
                let u = &mut self.players[p].sleeping[i];
                if 0 < u.workers {
                    u.workers -= 1;
                }
                return Ok(());
            }
            None => return Ok(()),
        };
        let world = World {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        let allied = world.allied(by.player, p, true);
        let (kind, listed) = {
            let u = &self.players[p].objects[i];
            let master = self.master_of(p, u.life.master_id)?;
            (master.kind, u.lists_taker(master, allied))
        };
        crate::objects::release::released(&mut self.players[p].objects[i], kind, listed, by)
    }

    /// Object `idx` of player `player` as the releaser of its actions' targets.
    pub(crate) fn releaser(&self, player: usize, idx: usize) -> Releaser {
        let u = &self.players[player].objects[idx];
        Releaser {
            id: u.id,
            class: self
                .master_of(player, u.life.master_id)
                .map_or(-1, |m| m.class),
            player,
        }
    }

    /// `FUN_140c42080` after the notify of a new action (`FUN_14106e4e0`, the combat object's
    /// `FUN_14121c890`, the attack order of `FUN_14161b970`) of object `idx` of player `player`:
    /// with a target the world holds (`FUN_1415417a0`), its unit AI remembers `order` and
    /// `action` on it ([`crate::unit_ai::UnitAi::remember_order`]) at the world's time, with the
    /// unit's place and the target's (slot 0x520) and the target's attack category by its master
    /// ([`crate::unit_ai::best_target::category`]: its own, else its owner's). An order on a point (no
    /// target) records nothing.
    pub(crate) fn remember_order(
        &mut self,
        player: usize,
        idx: usize,
        (order, action): (i32, i32),
    ) -> Result<(), GenieError> {
        let u = &self.players[player].objects[idx];
        let Some(target) = u.ai.as_ref().map(|a| a.current_target).filter(|&t| 0 <= t) else {
            return Ok(());
        };
        let position = u.position;
        let Some((q, t)) = self.object(target) else {
            return Ok(());
        };
        let class = match t.own_master.as_deref() {
            Some(m) => m.class,
            None => self.master_of(q, t.life.master_id)?.class,
        };
        let record = crate::unit_ai::OrderRecord {
            order,
            action,
            time: self.world_time,
            position,
            target,
            category: crate::unit_ai::best_target::category(t.life.master_id, class),
            target_position: t.position,
        };
        if let Some(ai) = self.players[player].objects[idx].ai.as_mut() {
            ai.remember_order(record);
        }
        Ok(())
    }

    /// The action list's clearing (`FUN_140a98620`) of object `idx` of player `player`: each
    /// action's teardown ([`Unit::clear_own_actions`]), then what they leave to the world: a
    /// counted builder leaves its building (`FUN_140aadbf0` before its base teardown,
    /// [`Turns::leave_builders`]), then the targets go (slot 0x180, [`Turns::release`]). A
    /// building's production is a list of its own, which stays.
    pub(crate) fn clear_actions(&mut self, player: usize, idx: usize) -> Result<(), GenieError> {
        let by = self.releaser(player, idx);
        let (mut releases, mut builder_leaves) = (Vec::new(), Vec::new());
        self.players[player].objects[idx].clear_own_actions(
            &mut releases,
            &mut builder_leaves,
            by,
        )?;
        for id in builder_leaves {
            self.leave_builders(id)?;
        }
        for (id, by) in releases {
            self.release(id, by)?;
        }
        Ok(())
    }

    /// `TRANSFORM_OBJECT` (`RGE_Command::do_command` case 0x29): the object `id`, the player's
    /// (another player's is refused, the cheat flag `+0xb12` aside), of the trophy master
    /// (1649) takes one of the trophies 1650 to 1653 the player has (`FUN_141659d20`): slot 0x90
    /// with 0, the building's ([`Turns::transform_building`]).
    pub(crate) fn transform_object(
        &mut self,
        player: usize,
        id: i32,
        master: i16,
    ) -> Result<(), GenieError> {
        let Some(Place::Live(p, idx)) = self.place(id) else {
            return Ok(());
        };
        if p != player || self.master_of(player, master).is_err() {
            return Ok(());
        }
        let unit = &self.players[p].objects[idx];
        if unit.life.master_id != TROPHY || !TROPHIES.contains(&master) {
            return Ok(());
        }
        self.transform_building(p, idx, master)
    }

    /// A building's slot 0x90 (`FUN_1411e6d90`, with 0 from each caller): [`Turns::change_master`],
    /// then in state 2 its build points (`+0xc50`) take the new master's build time (its first
    /// train location's time, 0 without one). The slice holds them in the building's
    /// construction record.
    pub(super) fn transform_building(
        &mut self,
        player: usize,
        idx: usize,
        new: i16,
    ) -> Result<(), GenieError> {
        self.change_master(player, idx, new)?;
        let build_time = self.master_of(player, new)?.build_time;
        let unit = &mut self.players[player].objects[idx];
        if unit.life.state == 2
            && let Some(c) = unit.construction.as_mut()
        {
            c.progress = f32::from(build_time);
        }
        Ok(())
    }

    /// The player's gate toggle (`FUN_14162f8c0`, from its slots 0x340 and 0x338): a gate of its
    /// player's masters, without masters of its own ([`crate::world::Gate::other_masters`]),
    /// changes to its master's `transform_unit` (slot 0x90, [`Turns::transform_building`]). A
    /// gate that shut (the new master's height above 0) kills the units in it (slot 0xd8 on
    /// those its count finds, [`Turns::shut_in_gate`]) and takes the world time plus 1 as its
    /// last move time (`+0x870`).
    pub(crate) fn toggle_gate(&mut self, player: usize, idx: usize) -> Result<(), GenieError> {
        let unit = &self.players[player].objects[idx];
        if unit.gate.is_some_and(|g| g.other_masters) {
            return Err(GenieError::unsupported(
                "a gate's toggle to a master of its own or its original owner's",
            ));
        }
        let new = self
            .master_of(player, unit.life.master_id)?
            .building
            .as_ref()
            .map_or(-1, |b| b.transform_unit);
        if new == -1 {
            return Ok(());
        }
        self.transform_building(player, idx, new)?;
        let height = self.master_of(player, new)?.height;
        let now = self.world_time;
        if 0.0 < height {
            let id = self.players[player].objects[idx].id;
            let annexes = crate::objects::unit::annexes_of(id, &self.players[player].objects);
            let (units, _) = crate::construction::gates::gate_units(&self.spatial, id, &annexes)?;
            for u in units {
                self.shut_in_gate(u)?;
            }
            self.players[player].objects[idx]
                .mover
                .as_mut()
                .ok_or(GenieError::unsupported(
                    "a shut gate without its moving part",
                ))?
                .last_move = now.wrapping_add(1);
            self.spatial.set_last_move(id, now.wrapping_add(1));
        }
        Ok(())
    }

    /// Slot 0xd8 of object `id` in a gate that shut (`FUN_14162f8c0`): from type 70 on
    /// (`FUN_141211340`) class 42 is spared, and livestock (class 58) while it holds some
    /// (`COMISS`, `JA`: a NaN load holds none); else, and below type 70, `FUN_14118f580`: an
    /// object asleep wakes to the end of its player's live list (slots 0x2d8, 0x2d0), one flagged
    /// to sleep drops the flag, and its hit points go to 0. Its place in its player's
    /// special-object list (`+0x30c` bit 0, `FUN_141677380`) is not held.
    pub(super) fn shut_in_gate(&mut self, id: i32) -> Result<(), GenieError> {
        /// The first master type whose slot 0xd8 is `FUN_141211340` (`TRIBE_Combat_Object`).
        const TRIBE_COMBAT: u8 = 70;
        /// The class `FUN_141211340` always spares.
        const SPARED: i16 = 42;
        let Some((p, u)) = self.object(id) else {
            return Ok(());
        };
        let master = self.master_of(p, u.life.master_id)?;
        let spared = TRIBE_COMBAT <= master.kind
            && (master.class == SPARED || (master.class == LIVESTOCK && 0.0 < u.life.amount));
        if spared {
            return Ok(());
        }
        let unit = match self.place(id).expect("found above") {
            Place::Live(p, i) => &mut self.players[p].objects[i],
            Place::Sleeping(p, i) => {
                let woken = self.players[p].sleeping.swap_remove(i);
                let (id, master) = (woken.id, woken.life.master_id);
                self.players[p].objects.push(woken);
                // the wake's remove and add (slots 0x2d8, 0x2d0) file it last in the tracker
                self.track_relist(p, id, master);
                self.players[p].objects.last_mut().expect("pushed")
            }
        };
        unit.life.goto_sleep = false;
        unit.life.hp = 0.0;
        Ok(())
    }

    /// Slot 0x90 (`FUN_141225b70` with 0): the unit changes its master. Its action list is
    /// cleared; in a state below 3 (`FUN_1411a6a70`) the old master's storage leaves its player's
    /// attributes and the new one's comes in (`FUN_1411a56c0`, flag 2), its hit points keep their
    /// share of the new master's (whole, at least 1), its sight goes and the new master's comes
    /// (slots 0x1d8, 0x1d0), its place and obstruction record take the new master's
    /// (`FUN_141055b40`, `FUN_14118c540`) and it moves with the new master's values; its powerups
    /// go ([`clear_powerups`]) and in state 2 its volleys (`+0xa00`) are the new master's volley
    /// fire; it takes the new master's standing sprite (slot 0x68). Its
    /// per-master counts, facet, class registry (`DAT_143f69170`) and particles are not held.
    pub(crate) fn change_master(
        &mut self,
        player: usize,
        idx: usize,
        new: i16,
    ) -> Result<(), GenieError> {
        self.clear_actions(player, idx)?;
        let unit = &self.players[player].objects[idx];
        let old = self.master_of(player, unit.life.master_id)?.clone();
        let next = self.master_of(player, new)?.clone();
        let world = SwitchWorld {
            spatial: &mut self.spatial,
            pathing: &mut self.pathing,
            graphics: &self.graphics,
            sprite: self.streams.get(Stream::Sprite),
        };
        let unit = &mut self.players[player].objects[idx];
        let switched = switch_master(unit, &old, &next, world)?;
        let owner = &mut self.players[player];
        for &(kind, amount) in &switched.leaving {
            owner.add_attribute(kind, amount)?;
        }
        if switched.promoted {
            // the tracker's promotion (FUN_14104fb20): last in its new key master's list and
            // class's
            let id = self.players[player].objects[idx].id;
            self.track_promote(player, id, old.id, new);
        }
        let owner = &mut self.players[player];
        for &(kind, amount) in &switched.coming {
            owner.add_attribute(kind, amount)?;
        }
        Ok(())
    }
}

/// What a master change ([`switch_master`]) changes of the world besides its unit: the place and
/// its records, the pathing, and the sprites with the Sprite stream.
pub(crate) struct SwitchWorld<'a> {
    pub(crate) spatial: &'a mut crate::map::spatial::Spatial,
    pub(crate) pathing: &'a mut crate::pathing::Pathing,
    pub(crate) graphics: &'a std::collections::BTreeMap<i16, crate::objects::sprite::Graphic>,
    pub(crate) sprite: &'a mut crate::math::rng::Well512,
}

/// What [`switch_master`] leaves to its owner, in the game's order: the old master's storage
/// leaving its player's attributes, the tracker's promotion (`FUN_14104fb20`), the new master's
/// storage coming in.
#[derive(Debug, Default)]
pub(crate) struct Switched {
    pub(crate) leaving: Vec<(i16, f32)>,
    pub(crate) promoted: bool,
    pub(crate) coming: Vec<(i16, f32)>,
}

/// Slot 0x90's unit part (`FUN_141225b70` with 0) of a master change from `old` to `next`, its
/// action list cleared by the caller: in a state below 3 (`FUN_1411a6a70`) the old master's
/// storage leaves its player's attributes and the new one's comes in (`FUN_1411a56c0`, flag 2:
/// the owner's, [`Switched`]), its hit points keep their share of the new master's (whole, at
/// least 1), its sight goes and the new master's comes (slots 0x1d8, 0x1d0), its place and
/// obstruction record take the new master's (`FUN_141055b40`, `FUN_14118c540`) and it moves with
/// the new master's values; it takes the new master's standing sprite (slot 0x68). Then its
/// powerups go ([`clear_powerups`]) and in state 2 its volleys (`+0xa00`) are the new master's
/// volley fire. Its per-master counts, facet, class registry (`DAT_143f69170`) and particles are
/// not held.
///
/// # Errors
///
/// A change of its damage sprites, a moving unit's new master that does not move and a standing
/// sprite the data file lacks.
pub(crate) fn switch_master(
    unit: &mut Unit,
    old: &Master,
    next: &Master,
    world: SwitchWorld<'_>,
) -> Result<Switched, GenieError> {
    let mut switched = Switched::default();
    let new = next.id;
    if unit.life.state < 3 {
        for &(kind, amount, flag) in &old.storage {
            if kind >= 0 && matches!(flag, 2 | 16 | 32) {
                switched.leaving.push((kind, -amount));
            }
        }
        let new_hp = f32::from(next.hit_points);
        let share = unit.life.hp / f32::from(old.hit_points) * new_hp;
        let hp = if share > new_hp {
            new_hp
        } else if 1.0 <= share {
            share
        } else {
            1.0
        };
        let old_hp = unit.life.hp;
        unit.life.hp = f32::from(hp as i32 as i16);
        // FUN_1411859a0: the damage share picks the damage sprites; one that stays keeps them
        // (the object's `+0xd`, not held, taken to be the old master's share)
        if (old.damage_sprites || next.damage_sprites)
            && (damage_share(old_hp, old.hit_points, old.class)?
                != damage_share(unit.life.hp, next.hit_points, next.class)?)
        {
            return Err(GenieError::unsupported(
                "a master change that changes the damage sprites (FUN_1411859a0)",
            ));
        }
        unit.life.master_id = new;
        let moving = unit
            .mover
            .as_ref()
            .is_some_and(crate::movement::Mover::is_moving);
        if let Some(m) = unit.mover.as_mut() {
            m.stats = next.moving.ok_or(GenieError::unsupported(
                "a moving unit's new master does not move",
            ))?;
        }
        let id = unit.id;
        if let Some(mut placed) = world.spatial.get(id).copied() {
            placed.radius = next.radius;
            placed.class = next.class;
            placed.master = new;
            placed.obstruction_type = next.obstruction_type;
            placed.buildable_on = next.buildable_on;
            placed.tall = next.tall;
            placed.speed = next.moving.as_ref().map_or(0.0, |m| m.speed);
            if unit.life.inside.is_some() {
                // inside another (`+0x2f0`) the unit has no tile (`+0x2e8` 0: no sight slots) and
                // is in no list nor the obstruction manager: its record takes the new master's
                // values for its way out
                world.spatial.set_inside_record(id, placed);
                if let Some(v) = world.spatial.sight.as_mut() {
                    v.seer_master(id, new, next.line_of_sight);
                }
            } else {
                if let Some(o) = placed.obstruction.as_mut() {
                    o.kind = next.obstruction_type;
                    o.class = next.obstruction_class;
                }
                world.pathing.removed(id);
                world
                    .spatial
                    .sight_master(id, next.radius, next.class, new, next.line_of_sight)?;
                world.spatial.change_master(id, placed);
                world.pathing.moved(id, &placed, moving);
            }
        }
        // slot 0x68 with the new master's standing sprite (`+0x470`, FUN_141217000): a new
        // instance unless the unit shows it already. The slice holds sprites from an attack
        // on; a unit whose sprite it does not hold shows its old master's. Without the
        // graphics (no data file) the Sprite stream is not modelled.
        if !world.graphics.is_empty() && old.standing_sprite != next.standing_sprite {
            let g = (0 <= next.standing_sprite)
                .then(|| world.graphics.get(&next.standing_sprite).copied())
                .flatten()
                .ok_or(GenieError::unsupported(
                    "a new master's standing sprite the data file does not have",
                ))?;
            if unit.sprite.is_some() {
                crate::combat::attack::set_sprite(&mut unit.sprite, g, world.sprite);
            } else {
                let _ = crate::objects::sprite::ActiveSprite::start(g, world.sprite);
            }
        }
        switched.promoted = true;
        for &(kind, amount, flag) in &next.storage {
            if kind >= 0 && flag == 2 {
                switched.coming.push((kind, amount));
            }
        }
    }
    clear_powerups(unit);
    // in state 2 the volleys (`+0xa00`) are the new master's volley fire, the shares of the
    // units inside dropped
    if unit.life.state == 2 {
        unit.firepower = next.volley_fire;
    }
    Ok(switched)
}
