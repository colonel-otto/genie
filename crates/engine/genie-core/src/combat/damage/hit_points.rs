//! The shield, the invulnerability floor, and the hit points' loss (slot 0xb0) and gain
//! (slot 0xb8).

use crate::combat::damage::{Armored, Victim};
use crate::math::fmath;

/// `FUN_14108d8a0`: a shield blocks the hit: a target whose master's charge type is 5 against a
/// melee hit (4 against a missile) spends a charge of 1 when it has one (`COMISS 1.0, charge`,
/// `JA` past: a NaN charge blocks too). The type-5 shield's local-player statistics
/// (`FUN_140a497f0`) change no simulation state (FD-037-D20). A target without combat values has
/// charge type 0.
pub fn shield(v: &mut Victim, missile: bool) -> bool {
    let kind = if missile { 4 } else { 5 };
    if v.source.charge_type == kind && fmath::at_most_or_unordered(1.0, v.source.charge) {
        v.source.charge -= 1.0;
        return true;
    }
    false
}

/// `FUN_141202a80`: the hit points the master's `invulnerability_level` keeps (a share of its hit
/// points when above 0, the level itself when below), 0 for none.
pub(super) fn hp_floor(a: &Armored, master_hp: i16) -> f32 {
    invulnerability_floor(a.invulnerability, master_hp)
}

/// [`hp_floor`] of a master's `invulnerability_level` and hit points.
#[must_use]
pub(crate) fn invulnerability_floor(level: f32, master_hp: i16) -> f32 {
    if level == 0.0 {
        0.0
    } else if 0.0 < level {
        f32::from(master_hp) * level
    } else {
        level.abs()
    }
}

/// Slot 0xc0: the hit points with the powerups' room.
pub(super) fn effective(v: &Victim) -> f32 {
    let pu = v.armored.as_ref().map_or([0.0; 2], |a| a.hp_powerups);
    (pu[1] - pu[0]) + v.hp
}

/// Slot 0xb0, the loss of `d` hit points: a combat object of type 70 or 80 (`FUN_141205c80`)
/// takes it on its hit point powerups' room first, and its hit points stay above its
/// `invulnerability_level` floor; another object (`FUN_141189160`) just loses it. Not modelled
/// (no update of this recording fills either list): the kill credits of a dying object's active
/// stingers (`+0xb58`) and a surviving object's hit point transforms (`+0xb20`,
/// `FUN_141204450`).
pub fn lose(t: &mut Victim, d: f32) {
    let unit = matches!(t.kind, 70 | 80);
    let Some(a) = t.armored.as_mut().filter(|_| unit) else {
        t.hp -= d;
        return;
    };
    let mut d = d;
    let [up, room_max] = a.hp_powerups;
    if up < room_max {
        let room = room_max - up;
        if d <= room {
            a.hp_powerups[0] = up + d;
            return;
        }
        a.hp_powerups[0] = up + room;
        d -= room;
    }
    let floor = hp_floor(a, t.master_hp);
    let left = t.hp - d;
    t.hp = if floor == 0.0 || floor <= left {
        left
    } else {
        floor
    };
}

/// Slot 0xb8 of a combat object (`FUN_141203be0`) on hit points `hp` with the powerups
/// `hp_powerups` (`+0xa38`, its room `+0xa3c`) of a master of `master_hp`: `amount` added up to
/// its most (the master's plus the room), an overflow taken off the powerup (never below 0); a
/// powerup above its room first takes the room. The hit point transforms of `FUN_141204450`,
/// empty in the recordings, are not held.
pub fn gain_hp(hp: &mut f32, hp_powerups: &mut [f32; 2], master_hp: i16, amount: f32) {
    let [mut up, room] = *hp_powerups;
    let most = f32::from(master_hp) + room;
    if room < up {
        up = room;
    }
    if most <= *hp {
        if 0.0 < up {
            up = (up - amount).max(0.0);
        }
    } else {
        let healed = *hp + amount;
        *hp = healed;
        if most < healed {
            *hp = most;
            if 0.0 < up {
                up = (up - (healed - most)).max(0.0);
            }
        }
    }
    hp_powerups[0] = up;
}

/// `FUN_141193b30`: slot 0xb8 adds `amount` ([`gain_hp`]); a loss (`amount` below 0) leaves at
/// least 1 hit point; healed to its full hit points (slot 0xc0 at least slot 200, `COMISS` with
/// `JC`: a NaN is not), it holds exactly its master's. True then.
pub fn heal_hp(hp: &mut f32, hp_powerups: &mut [f32; 2], master_hp: i16, amount: f32) -> bool {
    gain_hp(hp, hp_powerups, master_hp, amount);
    if amount < 0.0 && *hp < 1.0 {
        *hp = 1.0;
    }
    let [up, room] = *hp_powerups;
    let most = f32::from(master_hp) + room;
    if fmath::below_or_unordered((room - up) + *hp, most) {
        return false;
    }
    *hp = f32::from(master_hp);
    true
}
