//! UNLOAD and `GO_BACK_TO_WORK`: the units inside a building or a transport come out.

use crate::GenieError;
use crate::garrison::{
    BUILDING_CLASS, COMBAT_KIND, MASTER_1808, SIEGE_CLASS, SIEGE_UNLOAD, VILLAGER_CLASS,
    WHOLE_SHARE_CLASS,
};
use crate::orders::BUILDING;
use crate::world::Turns;

/// UNLOAD's point without one (`DAT_14330ec20`).
pub(super) const NO_POINT: f32 = crate::math::image::garrison::NO_POINT;
/// UNLOAD's order range (`DAT_14330eb8c`).
pub(super) const UNLOAD_RANGE: f32 = crate::math::image::garrison::UNLOAD_RANGE;

impl Turns {
    /// UNLOAD (`FUN_14161f410`) of player `player`'s objects `ids` (each alive, at least 1 hit
    /// point; `COMISS`/`JC`: a NaN is):
    /// - A unit inside a building leaves it toward the point (with its sound, no work), its
    ///   orders gone and, of type 70 and up, no longer under the town bell.
    /// - A unit (below type 80) inside a transport (type 70 or more) or, with `flag` 3, the unit
    ///   `unit_type` names inside this one, goes ashore ([`Turns::drop_out`]).
    /// - A siege unit (class 13) unloads its own way (`FUN_141225c40`, a siege tower's order 800),
    ///   which is not modelled ([`SIEGE_UNLOAD`]).
    /// - Any other unit takes the unload order at the point ([`Turns::unload_order`]): a transport
    ///   sails there and unloads (its Transport, [`crate::garrison::transport`]), any other walks there.
    /// - A building lets the units inside it out, from the last to come in, those `flag` and
    ///   `unit_type` pick ([`unload_picks`]): each through [`Turns::let_out`], toward the point
    ///   (or its unit AI's command queue location, which is not modelled); each one out has its
    ///   order queue emptied (`FUN_140c5e010`); a unit (type 70) out is no longer under the town
    ///   bell (`+0x9dc`).
    ///
    /// The local player's interface flag (`FUN_141663f50`) is not held.
    pub(crate) fn unload(
        &mut self,
        player: usize,
        ids: &[i32],
        point: [f32; 2],
        flag: u8,
        unit_type: i32,
        sound: bool,
    ) -> Result<(), GenieError> {
        // the drop locations' last place across the command's units (FUN_14161f410's locals)
        let mut last = None;
        for &id in ids {
            let Some(idx) = self.players[player].objects.iter().position(|u| u.id == id) else {
                continue;
            };
            let unit = &self.players[player].objects[idx];
            if unit.life.hp < 1.0 {
                continue;
            }
            // the object it is inside, and that object's type
            let container = match unit.life.inside {
                Some(c) => {
                    let (cp, cu) = self.object(c).ok_or(GenieError::unreachable(
                        "a unit inside an object the world does not hold",
                    ))?;
                    Some((c, self.master_of(cp, cu.life.master_id)?.kind))
                }
                None => None,
            };
            // a unit inside a building leaves it toward the point (`FUN_1411e7220` with no work
            // and its sound); a combat unit is no longer under the town bell (`+0x9dc`), and its
            // unit AI's orders go (`FUN_140c5e010`)
            if let Some((c, kind)) = container
                && BUILDING <= kind
            {
                self.leave_building(c, id, point)?;
                continue;
            }
            let unit = &self.players[player].objects[idx];
            let master = self.master_of(player, unit.life.master_id)?;
            if master.kind < BUILDING {
                let transport = container
                    .filter(|&(_, kind)| COMBAT_KIND <= kind)
                    .map(|(c, _)| c);
                if flag == 3 || transport.is_some() {
                    self.drop_out(player, idx, transport, unit_type, &mut last)?;
                } else if master.class == SIEGE_CLASS {
                    return Err(SIEGE_UNLOAD);
                } else {
                    self.unload_order(player, idx, point)?;
                }
                continue;
            }
            self.unload_building(player, idx, point, (flag, unit_type), sound)?;
        }
        Ok(())
    }

    /// [`Turns::unload`] of unit `id` inside building `c`: it leaves toward `point`
    /// (`FUN_1411e7220` with no work and its sound); a combat unit is no longer under the town
    /// bell (`+0x9dc`), and its unit AI's orders go (`FUN_140c5e010`).
    ///
    /// # Errors
    ///
    /// A building outside the live lists, and what [`Turns::let_out`] does not model.
    pub(super) fn leave_building(
        &mut self,
        c: i32,
        id: i32,
        point: [f32; 2],
    ) -> Result<(), GenieError> {
        let Some(crate::orders::Place::Live(cp, ci)) = self.place(c) else {
            return Err(GenieError::unreachable(
                "a building that holds units outside the live lists",
            ));
        };
        let _ = self.let_out(cp, ci, id, point, (None, true))?;
        let (up, ui) = self.live_inside(id)?;
        let kind = self
            .master_of(up, self.players[up].objects[ui].life.master_id)?
            .kind;
        let u = &mut self.players[up].objects[ui];
        if COMBAT_KIND <= kind {
            u.town_bell.flag = false;
        }
        if let Some(ai) = u.ai.as_mut() {
            ai.orders.clear();
        }
        Ok(())
    }

    /// [`Turns::unload`] of player `player`'s building `idx`: the units inside it come out from
    /// the last to come in, those `flag` and `unit_type` pick ([`unload_picks`]), each through
    /// [`Turns::let_out`] toward `point` (with `sound`), its order queue emptied
    /// (`FUN_140c5e010`) and, a unit (type 70), no longer under the town bell (`+0x9dc`).
    ///
    /// # Errors
    ///
    /// An unload toward the building's command queue location, and what [`Turns::let_out`] does
    /// not model.
    pub(super) fn unload_building(
        &mut self,
        player: usize,
        idx: usize,
        point: [f32; 2],
        (flag, unit_type): (u8, i32),
        sound: bool,
    ) -> Result<(), GenieError> {
        let unit = &self.players[player].objects[idx];
        if unit
            .ai
            .as_ref()
            .is_some_and(|ai| !ai.building_orders.is_empty())
        {
            return Err(GenieError::unsupported(
                "an unload toward a building's command queue location",
            ));
        }
        let mut i = unit.garrison.len();
        while 0 < i {
            i -= 1;
            let Some(&inside) = self.players[player].objects[idx].garrison.get(i) else {
                continue;
            };
            // the game skips a null entry; an id the world no longer holds is one
            let Some((up, u)) = self.object(inside) else {
                continue;
            };
            if !self.unload_picks(up, u, flag, unit_type)? {
                continue;
            }
            let _ = self.let_out(player, idx, inside, point, (None, sound))?;
            let (up, ui) = self.live_inside(inside)?;
            let kind = self
                .master_of(up, self.players[up].objects[ui].life.master_id)?
                .kind;
            let u = &mut self.players[up].objects[ui];
            // a unit (type 70) is no longer under the town bell (`+0x9dc`)
            if 70 <= kind {
                u.town_bell.flag = false;
            }
            if let Some(ai) = u.ai.as_mut() {
                ai.orders.clear();
            }
        }
        Ok(())
    }

    /// UNLOAD's unit at `idx` of player `player` inside the transport `transport` (`FUN_14161f410`
    /// with a container of type 70 or more) or, without one (flag 3), the unit `named` when it
    /// is inside this one (`FUN_1415417a0`; one the world does not hold, or elsewhere, stays):
    /// the unit inside, in state 2, goes ashore where a drop location about the holder is free
    /// ([`Turns::drop_location`] about the holder's place within twice its master's radii, in any
    /// zone; `last` the command's): it takes the holder's place (`FUN_14119d610`), leaves it
    /// (slot 0x1a8) for the place at the holder's height (slot 0x58,
    /// [`Turns::leave_container`]), takes its own stance again at its place (its unit AI's slot
    /// 0x1c0 with `+0x1a9`, [`crate::orders::set_stance`]) and, of a type above 69, is no longer
    /// under the town bell (`+0x9dc`). Without a free place the interface's message 0x85 is all.
    pub(super) fn drop_out(
        &mut self,
        player: usize,
        idx: usize,
        transport: Option<i32>,
        named: i32,
        last: &mut Option<[f32; 2]>,
    ) -> Result<(), GenieError> {
        let own = self.players[player].objects[idx].id;
        let (inner, holder) = match transport {
            Some(t) => (own, t),
            None => match self.object(named) {
                Some((_, o)) if o.life.inside == Some(own) => (named, own),
                _ => return Ok(()),
            },
        };
        let (ip, unit) = self.object(inner).expect("found above");
        if unit.life.state != 2 {
            return Ok(());
        }
        let made = self.master_of(ip, unit.life.master_id)?.clone();
        let Some(crate::orders::Place::Live(hp, hi)) = self.place(holder) else {
            return Err(GenieError::unreachable(
                "a transport that holds units outside the live lists",
            ));
        };
        let h = &self.players[hp].objects[hi];
        let at = h.position;
        let [rx, ry] = self.master_of(hp, h.life.master_id)?.radius;
        let around = crate::garrison::transport::Around {
            center: [at[0], at[1]],
            radius: [rx + rx, ry + ry],
        };
        let Some(spot) = self.drop_location((hp, hi), inner, &made, around, u32::MAX, last)? else {
            return Ok(());
        };
        let (up, ui) = self.live_inside(inner)?;
        self.players[up].objects[ui].position = at;
        self.leave_container((hp, hi), (up, ui), &made, [spot[0], spot[1], at[2]])?;
        let unit = &mut self.players[up].objects[ui];
        let place = ([unit.position[0], unit.position[1]], made.class);
        if let Some(ai) = unit.ai.as_mut() {
            let mut fx = Vec::new();
            let stance = ai.stance;
            crate::orders::set_stance(ai, stance, place, &mut fx)?;
            for _ in fx.iter().filter(|e| **e == crate::unit_ai::Effect::Stop) {
                self.command_stop(up, ui)?;
            }
        }
        if COMBAT_KIND <= made.kind {
            self.players[up].objects[ui].town_bell.flag = false;
        }
        Ok(())
    }

    /// UNLOAD's other unit at `idx` of player `player` (`FUN_14161f410`'s last branch): it leaves
    /// the unit group the world holds of its link (`FUN_14102c160` with −1 and 0), its ignore
    /// list empties (`FUN_141197da0`; an object without movement state holds none the slice
    /// keeps), and its owner gives its unit AI the unload order at once (slot 0x50,
    /// `FUN_141667db0` -> the unit AI's slot 0x30 -> slot 0x28 with Immediate set and inFront
    /// clear, [`crate::unit_ai::UnitAi::take_order_now_with`], run outside the unit's update):
    /// issued by the owner, on the unit itself with its master as the target type and the owner
    /// as the target's, at the point (the unit's own place for (−1, −1) unless its master's class
    /// is 3), height 0, within [`UNLOAD_RANGE`], priority 100. Of the record's last three fields
    /// the call writes only the first's low byte (1, the shown x: its upper bytes are the
    /// stack's, not held); the stack slots it leaves as the other two hold the caller's saved
    /// `RDI` (the shown y, not held) and the return address, whose low byte (`0x85`) makes the
    /// order ready.
    pub(super) fn unload_order(
        &mut self,
        player: usize,
        idx: usize,
        point: [f32; 2],
    ) -> Result<(), GenieError> {
        let id = self.players[player].objects[idx].id;
        self.leave_group_of(id, -1)?;
        let unit = &mut self.players[player].objects[idx];
        if let Some(m) = unit.mover.as_mut() {
            m.ignore_units.clear();
        }
        let unit = &self.players[player].objects[idx];
        let master = self.master_of(player, unit.life.master_id)?;
        // UCOMISS with −1, JP and JNZ: a NaN coordinate is not −1
        let to = if master.class != BUILDING_CLASS && point == [NO_POINT, NO_POINT] {
            [unit.position[0], unit.position[1]]
        } else {
            point
        };
        let order = crate::unit_ai::QueuedOrder {
            issuer: player as i32,
            order: crate::unit_ai::ai_kinds::ORDER_UNLOAD,
            priority: 100,
            target: id,
            target_owner: player as i32,
            point: [to[0], to[1], 0.0],
            range: UNLOAD_RANGE,
            shown: [f32::from_bits(1), 0.0],
            ready: true,
            target_type: i32::from(master.id),
        };
        if unit.ai.is_none() {
            return Ok(());
        }
        self.run_unit_ai((player, idx), |ai, at, side, computer, rng| {
            let mut fx = Vec::new();
            ai.take_order_now_with(order, (at, side, computer), rng, &mut fx)?;
            Ok(fx)
        })
    }

    /// `GO_BACK_TO_WORK` (`do_command` 0x80 with one object, 0x86 with several:
    /// `FUN_141625590`) on the player's objects `ids` (another player's only while players may
    /// order each other's, [`crate::world::settings::Settings::cheats_enabled`]; the cheat `+0xb12` is not held): a building
    /// lets out, from the last in, each unit inside of type 70 or more that is a villager (class
    /// 4) or of master 1808, back to its work ([`Turns::back_to_work`]).
    pub(crate) fn go_back_to_work(&mut self, player: usize, ids: &[i32]) -> Result<(), GenieError> {
        for id in self.checked_units(player, ids) {
            let Some((p, _)) = self.object(id) else {
                continue;
            };
            let Some(idx) = self.players[p].objects.iter().position(|u| u.id == id) else {
                return Err(GenieError::unsupported(
                    "a GO_BACK_TO_WORK of a sleeping object",
                ));
            };
            let b = &self.players[p].objects[idx];
            if self.master_of(p, b.life.master_id)?.kind != BUILDING {
                continue;
            }
            let mut i = b.garrison.len();
            while 0 < i {
                i -= 1;
                let Some(&inside) = self.players[p].objects[idx].garrison.get(i) else {
                    continue;
                };
                // any player's unit (an ally's too); the game skips a null entry, and an id the
                // world no longer holds is one
                let Some((up, u)) = self.object(inside) else {
                    continue;
                };
                let m = self.master_of(up, u.life.master_id)?;
                if 70 <= m.kind && (m.class == VILLAGER_CLASS || m.id == MASTER_1808) {
                    let _ = self.back_to_work(Some((p, idx)), inside)?;
                }
            }
        }
        Ok(())
    }

    /// `FUN_1412081e0` for the unit `id`, `from` the building it is inside (player `p`'s object
    /// at an index) or outside (None): the work to go back to ([`TownBell::back_to_work`]: the
    /// town-bell records' object the world holds); with none but a record of an action and a
    /// target class, the unit AI takes the action (`FUN_140c64b50`) and the unit tells it to
    /// search (notify 0x202 with the action, the class and the old target). From a building the
    /// unit then leaves it toward the record's point with the object to work on
    /// ([`Turns::let_out`], `FUN_1411e7220` with its sound); outside it goes back
    /// ([`Turns::back_outside`]). Once out or back it is no longer under the town bell
    /// (`+0x9dc`; `+0xa14` takes 1, not held). True when it went.
    ///
    /// [`TownBell::back_to_work`]: crate::orders::town_bell::TownBell::back_to_work
    pub(crate) fn back_to_work(
        &mut self,
        from: Option<(usize, usize)>,
        id: i32,
    ) -> Result<bool, GenieError> {
        use crate::orders::town_bell::RecordTarget;
        let (up, ui) = match from {
            Some(_) => self.live_inside(id)?,
            None => match self.place(id) {
                Some(crate::orders::Place::Live(p, i)) => (p, i),
                // the callers pass an object of type 70 or more the world holds, which never
                // sleeps
                _ => return Ok(false),
            },
        };
        let mut bell = self.players[up].objects[ui].town_bell;
        let lookup = |t: i32| -> Option<RecordTarget> {
            let (op, o) = self.object(t)?;
            let m = self.master_of(op, o.life.master_id).ok()?;
            Some(RecordTarget {
                id: t,
                building: m.kind == BUILDING,
                class: m.class,
                built: o
                    .construction
                    .is_some_and(|c| f32::from(m.build_time) <= c.progress),
            })
        };
        let (found, record) = bell.back_to_work(&lookup);
        self.players[up].objects[ui].town_bell = bell;
        if found.is_none() && 0 <= record.target_type && 0 <= record.action {
            let unit = &mut self.players[up].objects[ui];
            if let Some(ai) = unit.ai.as_mut() {
                ai.set_action(record.action);
            }
            unit.post_own(
                crate::unit_ai::REQUIRES_SEARCH,
                [record.action, record.target_type, record.target],
            )?;
        }
        let Some((p, building)) = from else {
            return self.back_outside(id, found.map(|f| f.id), record.point);
        };
        let out = self.let_out(p, building, id, record.point, (found.map(|f| f.id), true))?;
        if out && let Some((_, u)) = self.object_mut(id) {
            u.town_bell.flag = false;
        }
        Ok(out)
    }

    /// `FUN_14118a610`: whether UNLOAD's `flag` and `unit_type` (`param`) pick `unit` of player
    /// `player`, by its master's class and id: flag 0 every unit but class 42; 1 the units of
    /// player `param`; 2 a group of classes ([`unload_group_takes`]); 3 the object `param`; with
    /// the object `param` of the world (`FUN_1415417a0`; the unit's player's or none), 4 the units
    /// of its master (a villager: any villager for a villager), 5 every unit but it and class 42,
    /// 6 the units not of its master (no villager for a villager, never class 42). Other flags
    /// pick none.
    pub(super) fn unload_picks(
        &self,
        player: usize,
        unit: &crate::world::Unit,
        flag: u8,
        param: i32,
    ) -> Result<bool, GenieError> {
        let id = unit.id;
        let master = self.master_of(player, unit.life.master_id)?;
        let class = master.class;
        // the object `param` names, for flags 4 to 6: its master, and whether it is the unit's
        // player's (or the world does not hold it)
        let named = if matches!(flag, 4..=6) {
            self.object(param)
                .map(|(p, o)| self.master_of(p, o.life.master_id).map(|m| (p, m)))
                .transpose()?
        } else {
            None
        };
        let same_player = named.is_none_or(|(p, _)| p == player);
        let named = named.map(|(_, m)| m);
        Ok(match flag {
            0 => class != WHOLE_SHARE_CLASS,
            1 => player as i32 == param,
            2 => unload_group_takes(param, class),
            3 => id == param,
            4 => match named.filter(|_| same_player) {
                Some(m) if m.id == master.id => true,
                Some(m) => class == VILLAGER_CLASS && m.class == VILLAGER_CLASS,
                None => false,
            },
            5 => same_player && id != param && class != WHOLE_SHARE_CLASS,
            6 => match named.filter(|_| same_player) {
                Some(m) if m.id != master.id => {
                    if class == VILLAGER_CLASS {
                        m.class != VILLAGER_CLASS
                    } else {
                        class != WHOLE_SHARE_CLASS
                    }
                }
                _ => false,
            },
            _ => false,
        })
    }
}

/// UNLOAD's flag 2 (`FUN_14118a610`): the classes of group `group` (1 villagers and class 19;
/// 2 classes 6, 45, 46, 50 and 56; 3 classes 12, 23, 36, 47 and 57; 5 classes 0 and 44; 6
/// classes 18, 42 and 43; no other group).
pub(super) const fn unload_group_takes(group: i32, class: i16) -> bool {
    match group {
        1 => matches!(class, VILLAGER_CLASS | 19),
        2 => matches!(class, 6 | 45 | 46 | 50 | 56),
        3 => matches!(class, 12 | 23 | 36 | 47 | 57),
        5 => matches!(class, 0 | 44),
        6 => matches!(class, 18 | WHOLE_SHARE_CLASS | 43),
        _ => false,
    }
}
