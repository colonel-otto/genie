//! The unit AI's state, gates, the auto scout and the STOP, EXPLORE and standing commands.

use crate::GenieError;
use crate::math::rng::Stream;
use crate::orders::{AUTO_SCOUT_CLASS, BUILDING, Place, lock_gate, set_stance};
use crate::world::World;

impl World {
    /// `UNIT_AI_STATE` (`do_command` case 0x12): the check ([`World::command_check`]), then
    /// `FUN_140e6a7c0` over its first 60 ids (500 for a computer player): each object the world
    /// holds (any player's the check let through) with a unit AI takes the stance `state` (slot
    /// 0x1c0, [`set_stance`]), but not stances 1 and 2 for classes 54 and 51 (and, the siege AI's
    /// slot, masters 42 and 331). The UI's flag is not held.
    pub(crate) fn unit_ai_state(
        &mut self,
        player: usize,
        ids: &[i32],
        state: u8,
    ) -> Result<(), GenieError> {
        let Some(ids) = self.command_check(player, ids) else {
            return Ok(());
        };
        let cap = if self.players[player].computer {
            500
        } else {
            60
        };
        for &id in ids.iter().take(cap) {
            let Some(place) = self.place(id) else {
                continue;
            };
            let (p, idx) = match place {
                Place::Live(p, i) => (p, i),
                // only slot 0x180 puts an object to sleep, one of a type below 30, which has no
                // unit AI: the handler passes it over
                Place::Sleeping(p, i) if self.players[p].sleeping[i].ai.is_none() => continue,
                Place::Sleeping(..) => {
                    return Err(GenieError::unsupported(
                        "a UNIT_AI_STATE of a sleeping object with a unit AI",
                    ));
                }
            };
            let unit = &self.players[p].objects[idx];
            if unit.ai.is_none() {
                continue;
            }
            let master = self.master_of(p, unit.life.master_id)?;
            if matches!(state, 1 | 2)
                && (matches!(master.class, 51 | 54) || matches!(master.id, 42 | 331))
            {
                continue;
            }
            let class = master.class;
            let unit = &mut self.players[p].objects[idx];
            let place = ([unit.position[0], unit.position[1]], class);
            let ai = unit.ai.as_mut().expect("checked above");
            let mut fx = Vec::new();
            set_stance(ai, state, place, &mut fx)?;
            for e in fx {
                match e {
                    crate::unit_ai::Effect::Stop => self.command_stop(p, idx)?,
                    _ => {
                        return Err(GenieError::unsupported(
                            "a stance's stop effect not modelled",
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// GATE (`do_command` case 0x72): the command's check refuses an object `id` the world does
    /// not hold, or another player's while players may not order each other's units (the
    /// setting `+0x1074`, [`crate::world::settings::Settings::cheats_enabled`]; the world's `+0xb12` cheat is not held).
    /// The object, live or sleeping (`FUN_1415417a0`), of a master of type 80 toggles its gate
    /// lock: slot 0x670 ([`lock_gate`]) with slot 0x668's value negated. Its sound (slot 0x4c0)
    /// and the UI's flag are not held.
    pub(crate) fn gate(&mut self, player: usize, id: i32) -> Result<(), GenieError> {
        let Some((p, unit)) = self.object(id) else {
            return Ok(());
        };
        if p != player && !self.settings.cheats_enabled {
            return Ok(());
        }
        if self.master_of(p, unit.life.master_id)?.kind != BUILDING {
            return Ok(());
        }
        let now = self.world_time;
        let (_, unit) = self.object_mut(id).expect("found above");
        // slot 0x668: the lock's value (a building without its gate state is lock_gate's)
        let locked = unit.gate.is_some_and(|g| g.locked);
        lock_gate(unit, !locked, now)?;
        // the walks of this update read the last move time the lock set
        if let Some(t) = unit.body.mover().map(|m| m.last_move) {
            self.spatial.set_last_move(id, t);
        }
        Ok(())
    }

    /// `AUTO_SCOUT` (0x26, `FUN_140e666f0`): of the first 60 ids (500 for a computer player,
    /// `FUN_141659cf0`) each object the world holds (any player's: the handler has no owner
    /// test) with a unit AI that may scout (`FUN_140c89760`: its master's object flags `+0x654`
    /// have bit 4; class 58's rule behind a game setting, `FUN_140eca780`, is not modelled)
    /// takes an order at once (slot 0x30 -> slot 0x28 with Immediate,
    /// [`crate::unit_ai::UnitAi::take_order_now`]): the stop order when its action is the auto
    /// scout's already, else the auto scout order, from the object's owner (`+0x718`), without
    /// a target, at the point (−1, −1) at the unit's height, range 1, priority 100, shown at
    /// (−1, −1) and ready. Its sound and the UI's flags are not held.
    pub(crate) fn auto_scout(&mut self, player: usize, ids: &[i32]) -> Result<(), GenieError> {
        use crate::unit_ai::{
            AUTO_SCOUT_ACTION, AUTO_SCOUT_ORDER, Effect, QueuedOrder, STOP_ORDER,
        };
        let cap = if self.players[player].computer {
            500
        } else {
            60
        };
        for &id in ids.iter().take(cap) {
            let Some(place) = self.place(id) else {
                continue;
            };
            let (p, idx) = match place {
                Place::Live(p, i) => (p, i),
                // only slot 0x180 puts an object to sleep, one of a type below 30, which has no
                // unit AI: the handler passes it over
                Place::Sleeping(p, i) if self.players[p].sleeping[i].ai.is_none() => continue,
                Place::Sleeping(..) => {
                    return Err(GenieError::unsupported(
                        "an AUTO_SCOUT of a sleeping object with a unit AI",
                    ));
                }
            };
            let unit = &self.players[p].objects[idx];
            if unit.ai.is_none() {
                continue;
            }
            let master = self.master_of(p, unit.life.master_id)?;
            if master.object_flags & 0x10 == 0 {
                if master.class == AUTO_SCOUT_CLASS {
                    return Err(GenieError::unsupported(
                        "AUTO_SCOUT's class 58 rule (FUN_140eca780)",
                    ));
                }
                continue;
            }
            let class = master.class;
            let unit = &mut self.players[p].objects[idx];
            let (state, position) = (unit.life.state, unit.position);
            let ai = unit.ai.as_mut().expect("checked above");
            let order = QueuedOrder {
                issuer: p as i32,
                order: if ai.current_action == AUTO_SCOUT_ACTION {
                    STOP_ORDER
                } else {
                    AUTO_SCOUT_ORDER
                },
                priority: 100,
                target: -1,
                target_owner: -1,
                point: [-1.0, -1.0, position[2]],
                range: 1.0,
                shown: [-1.0, -1.0],
                ready: true,
                target_type: -1,
            };
            let mut fx = Vec::new();
            ai.take_order_now(
                state,
                order,
                ([position[0], position[1]], class),
                self.streams.get(Stream::Ai),
                &mut fx,
            )?;
            for e in fx {
                match e {
                    Effect::Stop => self.command_stop(p, idx)?,
                    Effect::Explore(point) => self.command_explore(p, idx, point)?,
                    _ => {
                        return Err(GenieError::unsupported(
                            "an order effect of AUTO_SCOUT not modelled",
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// STOP (0x01, `RGE_Command::do_command` case 1) of the player's units `ids`: the command's
    /// check ([`World::command_check`]), the lookup ([`World::lookup`]), then `FUN_14161e200`. First
    /// each unit AI resets as for an order (`FUN_141631040`, the loop of `FUN_140e6cfc0`:
    /// [`crate::unit_ai::UnitAi::reset_for_order`]). Several entries (a gone unit's null among
    /// them) that are exactly one unit group's members (`FUN_14152ec80`, the group's size
    /// `+0xe4`) stop as the group: each unit AI's order queue empties and its idle timer takes its
    /// adjusted timeout, then the group's stop (`FUN_1410110c0`), which is not modelled. Else each
    /// unit leaves the unit group the world holds of its link (`FUN_14102c160` with -1 and 0,
    /// [`World::remove_unit`]), its unit AI's order queue empties (`FUN_140c5e010`) and its idle
    /// timer takes its adjusted timeout (`+0x160` = `+0x164`), its ignore list empties
    /// (`FUN_141197da0`), the object stops (slot 0x148, [`World::object_stop`]) and its movement
    /// stands (slot 0x560, `FUN_1411616f0`: slot 0x5d0, then `FUN_140f98e80` on its walked
    /// level).
    pub(crate) fn stop_command(&mut self, player: usize, ids: &[i32]) -> Result<(), GenieError> {
        let Some(ids) = self.command_check(player, ids) else {
            return Ok(());
        };
        let list: Vec<Option<i32>> = self
            .lookup(&ids)
            .into_iter()
            .zip(&ids)
            .map(|(place, &id)| place.map(|_| id))
            .collect();
        let units: Vec<i32> = list.iter().flatten().copied().collect();
        for &id in &units {
            let unit = match self.place(id) {
                Some(Place::Live(p, i)) => &mut self.players[p].objects[i],
                Some(Place::Sleeping(p, i)) => &mut self.players[p].sleeping[i],
                None => continue,
            };
            if let Some(ai) = unit.ai.as_mut() {
                ai.reset_for_order(self.streams.get(Stream::Ai));
            }
        }
        if 1 < list.len() {
            let entries: Vec<i32> = list.iter().map(|u| u.unwrap_or(-1)).collect();
            let group = crate::groups::common_group(&entries, |id| {
                self.object(id).map(|(_, u)| u.group.id)
            });
            if self
                .groups
                .get(group)
                .is_some_and(|g| g.members.len() == list.len())
            {
                return Err(GenieError::unsupported(
                    "a STOP of a whole unit group (FUN_1410110c0)",
                ));
            }
        }
        for &id in &units {
            let link = self.object(id).map_or(-1, |(_, u)| u.group.id);
            if link != -1 {
                let _ = self.remove_unit(link, id, -1, false)?;
            }
            let Some((_, u)) = self.object_mut(id) else {
                continue;
            };
            if let Some(ai) = u.ai.as_mut() {
                ai.orders.clear();
                ai.idle_timer = ai.adjusted_idle_timeout;
            }
            if let Some(m) = u.body.mover_mut() {
                m.ignore_units.clear();
            }
            // a sleeping object is of a type below 30 (only slot 0x180 puts one to sleep), whose
            // slots 0x148 and 0x560 are empty (0x141072730, 0x141072740)
            let Some(Place::Live(p, idx)) = self.place(id) else {
                continue;
            };
            self.object_stop(p, idx)?;
            if let Some(m) = self.players[p].objects[idx].body.mover_mut() {
                m.stand()?;
            }
        }
        Ok(())
    }

    /// The check of a command on units (STOP's `FUN_140e55a30` and `UNIT_AI_STATE`'s `FUN_140e55d30`,
    /// twins of `FUN_140e558b0`) of the ids of `player`: of the first 60 (500 for a computer,
    /// `FUN_141659cf0`) an id the world does not hold, or another player's without the setting
    /// `+0x1074` ([`crate::world::settings::Settings::cheats_enabled`]), becomes -1; the ids past them stay unchecked. A
    /// human's command whose checked ids all went is refused (None).
    /// The packet's own tests (its size, a player the game lacks) are the decoder's, and the
    /// world's `+0xb12` (the cheat that would let either through) is not held.
    pub(crate) fn command_check(&self, player: usize, ids: &[i32]) -> Option<Vec<i32>> {
        let computer = self.players[player].computer;
        let checked = ids.len().min(if computer { 500 } else { 60 });
        let mut ids = ids.to_vec();
        let mut dropped = 0;
        for id in &mut ids[..checked] {
            let keep = self
                .object(*id)
                .is_some_and(|(p, _)| p == player || self.settings.cheats_enabled);
            if *id == -1 || !keep {
                *id = -1;
                dropped += 1;
            }
        }
        if checked <= dropped && !computer {
            return None;
        }
        Some(ids)
    }

    /// UNLOAD's check (`FUN_140e6f620`) of the ids of `player`: only the first 60 (500 for a
    /// computer, `FUN_140e6b250`) go on. An id the world does not hold (`FUN_140e6bad0`, live or
    /// sleeping; −1 none) becomes −1. Its own player's object stays. Another player's stays when
    /// the two count each other as allies ([`World::mutual_allies`]) and the command ties it to
    /// the issuer: `flag` 1 names the issuer (`named`), flags 3 to 6 an object of the issuer's
    /// (`FUN_1415417a0`). Any other stays only with the setting `+0x1074`
    /// ([`crate::world::settings::Settings::cheats_enabled`]). A human's command whose ids all
    /// went is refused (None). The packet's size tests are the decoder's, and the world's
    /// `+0xb12` is not held.
    pub(crate) fn unload_check(
        &self,
        player: usize,
        ids: &[i32],
        flag: u8,
        named: i32,
    ) -> Option<Vec<i32>> {
        let computer = self.players[player].computer;
        let mut ids = ids[..ids.len().min(if computer { 500 } else { 60 })].to_vec();
        let tied = |owner: usize| {
            self.mutual_allies(owner, player)
                && match flag {
                    1 => usize::try_from(named) == Ok(player),
                    3..=6 => self.place(named).is_some_and(|p| p.player() == player),
                    _ => false,
                }
        };
        let mut dropped = 0;
        for id in &mut ids {
            let keep = (*id != -1)
                && self.place(*id).is_some_and(|p| {
                    p.player() == player || tied(p.player()) || self.settings.cheats_enabled
                });
            if !keep {
                *id = -1;
                dropped += 1;
            }
        }
        if ids.len() <= dropped && !computer {
            return None;
        }
        Some(ids)
    }

    /// `FUN_1416642c0` with the ally flag: players `a` and `b` each count the other as its own
    /// or allied ([`crate::unit_ai::task_finder::allied`] on each one's relations); not for a
    /// player the world lacks.
    pub(crate) fn mutual_allies(&self, a: usize, b: usize) -> bool {
        let allied = |from: usize, to: usize| {
            self.players
                .get(from)
                .is_some_and(|p| crate::unit_ai::task_finder::allied(&p.relations, from, to, true))
        };
        allied(a, b) && allied(b, a)
    }

    /// `FUN_140e6b750`: each id's object (`FUN_1415417a0`, live or sleeping), None where the
    /// world holds none; the list keeps the ids' count.
    pub(crate) fn lookup(&self, ids: &[i32]) -> Vec<Option<Place>> {
        ids.iter().map(|&id| self.place(id)).collect()
    }

    /// stopAction (slot 0x3c0, `FUN_14108c160`) in the command phase: the action list empties
    /// (`FUN_140a98620`, [`World::clear_actions`]) and a unit in state 2 takes its standing
    /// sprite (slot 0x798, then slot 0x68) where the slice holds its sprite.
    pub(crate) fn command_stop(&mut self, p: usize, idx: usize) -> Result<(), GenieError> {
        if self.players[p].objects[idx].unmodelled_action.is_some() {
            return Err(GenieError::unsupported(
                "a command's stop of an action in the command phase",
            ));
        }
        self.clear_actions(p, idx)?;
        if self.players[p].objects[idx].life.state == 2 {
            self.standing_sprite(p, idx)?;
        }
        Ok(())
    }

    /// The unit's explore (slot 0x398) in the command phase: a unit without an Explore takes a
    /// new one ([`crate::movement::explore::ExploreAction::new`]) at its height; its `set_state`'s notify and
    /// standing sprite follow.
    pub(super) fn command_explore(
        &mut self,
        p: usize,
        idx: usize,
        point: [f32; 2],
    ) -> Result<(), GenieError> {
        use crate::movement::explore::{EXPLORE_ACTION, ExploreAction, Owner, WANTS_POINT};
        let unit = &mut self.players[p].objects[idx];
        if unit.explore.is_some() || unit.top_action().is_some() {
            return Err(GenieError::unsupported(
                "a command's explore of a unit with an action",
            ));
        }
        let (e, owner) = ExploreAction::new([point[0], point[1], unit.position[2]]);
        unit.explore = Some(e);
        match owner {
            Owner::Nothing => return Ok(()),
            Owner::WantsPoint => {
                let (id, state) = (unit.id, unit.life.state);
                if let Some(ai) = unit.ai.as_mut() {
                    let n = crate::unit_ai::Notify::own(id, WANTS_POINT, [EXPLORE_ACTION, 0, 0]);
                    ai.post(n, state)?;
                }
            }
            Owner::Standing => {}
            Owner::Stopped => {
                return Err(GenieError::unsupported(
                    "an Explore's set_state 2 (slots 0x68)",
                ));
            }
        }
        {
            self.standing_sprite(p, idx)?;
        }
        Ok(())
    }

    /// Slot 0x68 with the master's standing sprite (`+0x470`): the unit's top speed is the
    /// sprite's ([`crate::movement::Mover::shows_standing`]), and the sprite itself where the
    /// slice holds it.
    pub(crate) fn standing_sprite(&mut self, p: usize, idx: usize) -> Result<(), GenieError> {
        let unit = &mut self.players[p].objects[idx];
        if let Some(m) = unit.body.mover_mut() {
            m.shows_standing();
        }
        if unit.sprite.is_none() {
            return Ok(());
        }
        let master = self.players[p].objects[idx].life.master_id;
        let standing = self.master_of(p, master)?.standing_sprite;
        let g = self
            .graphics
            .get(&standing)
            .copied()
            .ok_or(GenieError::unsupported(
                "a unit without its standing sprite",
            ))?;
        let rng = self.streams.get(Stream::Sprite);
        crate::combat::attack::set_sprite(&mut self.players[p].objects[idx].sprite, g, rng);
        Ok(())
    }
}
