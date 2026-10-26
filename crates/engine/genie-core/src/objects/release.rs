//! An object's leave of the world and its release by the action that held it (slot 0x180); the
//! update's regeneration and hit point check, and a walk's pause.

use crate::GenieError;
use crate::objects::lifecycle::death_parts::{count_class, forget_resource};
use crate::objects::masters::Masters;
use crate::objects::unit::Unit;
use crate::unit_ai::Notify;
use crate::world::Turns;
use crate::world::context::Context;

/// Slot 0x180 (`Turns::release`) on an object of the unit's own live list, from inside the unit's
/// update ([`released`]; the releaser's own player is allied).
pub(crate) fn release_own(
    others: &mut [Unit],
    masters: Option<&Masters>,
    id: i32,
    by: Releaser,
) -> Result<(), GenieError> {
    let u = others
        .iter_mut()
        .find(|u| u.id == id)
        .ok_or(GenieError::unsupported(
            "a release of an object not in the unit's live list",
        ))?;
    let kind = masters
        .and_then(|m| m.get(u.life.master_id))
        .ok_or(GenieError::unsupported(
            "a released object of a master the slice does not have",
        ))?
        .kind;
    // one of the releaser's own player: allied, never listed
    released(u, kind, false, by)
}

/// The object whose action lets its target go (the target's slot 0x180 takes it): its id, its
/// master's class and its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Releaser {
    pub(crate) id: i32,
    pub(crate) class: i16,
    pub(crate) player: usize,
}

/// The first master type whose slot 0x180 is not the static object's (`FUN_141197d00`, types 10
/// and 15): the animated object's (`FUN_141071ea0`, types 20 to 40) only counts a worker less.
pub(crate) const ANIMATED_TYPE: u8 = 20;

/// Slot 0x180 on object `u` of master type `kind`, let go by `by`: the static object's
/// (`FUN_141197d00`, types 10 and 15) counts a worker less and a static one without workers
/// left, alive (state 2 with hit points, or 6), goes to sleep; the animated object's
/// (`FUN_141071ea0`, types 20 to 40) only counts the worker less. A combat object's (`FUN_141086e60`) counts the worker
/// less (`FUN_141071ea0`) and tells its unit AI that the releaser no longer acts on it (notify
/// 699, the releaser's master class as its first parameter; none from an object of a negative
/// id). A building's (`FUN_1411d6020`) first drops the releaser's first entry from its attacking
/// units (`+0xce0`, the rest keeping their order) when it lists it (`listed`, [`Unit::lists_taker`]).
pub(crate) fn released(
    u: &mut Unit,
    kind: u8,
    listed: bool,
    by: Releaser,
) -> Result<(), GenieError> {
    if listed && let Some(i) = u.attacking_units.iter().position(|&t| t == by.id) {
        let _ = u.attacking_units.remove(i);
    }
    if 0 < u.workers {
        u.workers -= 1;
    }
    // COMISS hp with 0, JNC: a NaN does not count as alive
    let alive = (u.life.state == 2 && 0.0 <= u.life.hp) || u.life.state == 6;
    if kind < ANIMATED_TYPE && u.workers == 0 && alive {
        u.life.goto_sleep = true;
    }
    if !u.combat {
        return Ok(());
    }
    if 0 <= by.id
        && let Some(ai) = u.ai.as_mut()
    {
        let n = Notify {
            caller: by.id,
            recipient: u.id,
            kind: crate::unit_ai::NO_LONGER_ON_ME,
            p: [i32::from(by.class), 0, 0],
        };
        ai.post(n, u.life.state)?;
    }
    Ok(())
}

impl Turns {
    /// The leave of object `gone`, out of player `p`'s live list already (its list's update that
    /// returned 1, `FUN_14116b5e0`): its player's remove (slot 0x2d8) takes it out of the tracker,
    /// it leaves the visible-resource lists ([`forget_resource`]), a combat object's destructor
    /// lowers its player's class counters (`FUN_141200450`), it leaves the world's places and its
    /// owner's watched list (`FUN_141091310`), a recyclable master's missile goes to the pool of
    /// type 60 (the world's `+0x1d8`), and it leaves the mesh. Every step runs; returns the
    /// world's leave of the place that the slice does not model.
    pub(crate) fn leave_world(&mut self, p: usize, gone: &Unit) -> Result<(), GenieError> {
        self.track_remove(p, gone.id, gone.life.master_id);
        let master = self.masters.get(p).and_then(|m| m.get(gone.life.master_id));
        forget_resource(&mut self.spatial, master, gone.id, gone.position);
        if let Some(m) = master.filter(|m| 70 <= m.kind) {
            count_class(&mut self.players[p].attributes, m, -1.0);
        }
        let recyclable = master.is_some_and(|m| m.recyclable);
        let left = self.spatial.leave(gone.id);
        self.unwatch(p, gone.id);
        if let Some(m) = gone.missile.as_ref().filter(|_| recyclable) {
            self.recycling.missiles.push((gone.id, m.flying));
        }
        self.pathing.removed(gone.id);
        self.pathing.tiles_changed(self.spatial.take_changes());
        left
    }

    /// `TRIBE_Combat_Object::update`'s regeneration ([`Unit::regenerate`]) of the object in
    /// `slot` ([`Turns::live_slot`]), a combat object or building (types 70 and 80) by its own
    /// master, else its player's.
    pub(crate) fn regenerate_object(&mut self, slot: (usize, usize, i32), dt_seconds: f32) {
        let Some((p, i)) = self.live_slot(slot) else {
            return;
        };
        let unit = &self.players[p].objects[i];
        let Some(master) = unit
            .own_master
            .as_deref()
            .or_else(|| self.masters.get(p).and_then(|m| m.get(unit.life.master_id)))
            .filter(|m| matches!(m.kind, 70 | 80))
            .cloned()
        else {
            return;
        };
        self.players[p].objects[i].regenerate(&master, dt_seconds);
    }

    /// `TRIBE_Combat_Object::update`'s hit point powerup check ([`Unit::clamp_hp_powerup`]) of the
    /// object in `slot` ([`Turns::live_slot`]), a combat object or building (types 70 and 80).
    pub(crate) fn clamp_object_powerup(&mut self, slot: (usize, usize, i32)) {
        let Some((p, i)) = self.live_slot(slot) else {
            return;
        };
        let unit = &self.players[p].objects[i];
        let combat = unit
            .own_master
            .as_deref()
            .or_else(|| self.masters.get(p).and_then(|m| m.get(unit.life.master_id)))
            .is_some_and(|m| matches!(m.kind, 70 | 80));
        if combat {
            self.players[p].objects[i].clamp_hp_powerup();
        }
    }
}

impl Turns {
    /// Slot 0x5f0 (`FUN_141158b90`): the unit's walked level, if it has one, pauses for `pause`
    /// seconds; the world's view of it follows.
    pub(crate) fn pause(&mut self, id: i32, pause: f32) {
        let Some(u) = self
            .players
            .iter_mut()
            .flat_map(|p| p.objects.iter_mut())
            .find(|u| u.id == id)
        else {
            return;
        };
        if let Some(level) = u.mover.as_mut().and_then(|m| m.paths.first_mut()) {
            level.pause_time = pause;
            let gait = u.gait();
            self.spatial.set_gait(id, gait);
        }
    }
}

impl Unit {
    /// The unit as the releaser of its actions' targets (slot 0x180's argument).
    pub(crate) fn releaser(&self, c: &Context<'_>) -> Releaser {
        Releaser {
            id: self.id,
            class: c
                .masters
                .and_then(|m| m.get(self.life.master_id))
                .map_or(-1, |m| m.class),
            player: c.player,
        }
    }
}
