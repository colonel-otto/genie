//! A combat object's hit points, a unit's or a building's (`TRIBE_Combat_Object`, types 70 and
//! 80): the heal, the gain and the loss, the regeneration and the powerup's clamp (slots 0x110,
//! 0xb8 and 0x260, and the update's checks). Whatever heals (a monk's heal action, a building
//! that heals around it) calls the healed object's heal here.

use crate::GenieError;
use crate::objects::masters::Master;
use crate::objects::unit::Unit;

/// A hero's regeneration (`DAT_142ed06a8`, a minute's hit points).
pub(crate) const HERO_REGENERATION: f32 = crate::math::image::turn::HERO_REGENERATION;
/// The regeneration's minute (`DAT_14330ebc8`).
pub(crate) const REGENERATION_MINUTE: f32 = crate::math::image::turn::REGENERATION_MINUTE;
/// The size below which a rate or a period is none (`DAT_142f49458`).
pub(crate) const REGENERATION_EPSILON: f32 = crate::math::image::turn::REGENERATION_EPSILON;
/// The hit points below which nothing regenerates (`DAT_14330eb8c`).
pub(crate) const REGENERATION_LEAST_HP: f32 = crate::math::image::turn::REGENERATION_LEAST_HP;
/// A whole hit point, the `fmodf` divisor (`XMM11`, `DAT_14330eb8c`).
pub(crate) const REGENERATION_POINT: f32 = crate::math::image::turn::REGENERATION_POINT;
/// The hit points a powerup's check leaves at least (`MAXSS` with `DAT_14330eb8c`).
pub(crate) const HP_POWERUP_FLOOR: f32 = crate::math::image::turn::HP_POWERUP_FLOOR;

/// Slot 0x260 of a combat object (`FUN_1412033a0`), its regeneration rate in hit points a
/// minute: its stinger (`+0xb3c`, slot 0x750) plus (its powerup (`+0xa34`, slot 0x6f0) plus (a
/// hero's [`HERO_REGENERATION`] plus its master's `regeneration_rate`, `+0x8b4`)), in the
/// assembly's order. A hero is a master whose `hero_flag` (`+0x8d9`) has bit 0 or 2, unless it
/// has both bits 7 and 2.
#[must_use]
pub(crate) fn regeneration_rate(master: &Master, unit: &Unit) -> f32 {
    let flag = master.hero_flag;
    let hero = flag & 5 != 0 && !(flag & 0x80 != 0 && flag & 4 != 0);
    let bonus = if hero { HERO_REGENERATION } else { 0.0 };
    let [powerup, stinger] = unit.regen_powerups;
    stinger + (powerup + (bonus + master.regeneration))
}

impl Unit {
    /// Slot 0x110 of a combat object (`FUN_141213b10`) of its master's `class` and
    /// `hit_points`: the classes 2, 20, 21, 22 and 53 repair at a cost (not modelled); any other
    /// heals (`FUN_141193b30`, [`crate::combat::damage::heal_hp`]).
    pub(crate) fn heal(
        &mut self,
        class: i16,
        hit_points: i16,
        amount: f32,
    ) -> Result<(), GenieError> {
        if matches!(class, 2 | 20 | 21 | 22 | 53) {
            return Err(GenieError::unsupported(
                "a heal at a cost (FUN_141213b10's classes)",
            ));
        }
        let _ = crate::combat::damage::heal_hp(
            &mut self.life.hp,
            &mut self.hp_powerups,
            hit_points,
            amount,
        );
        Ok(())
    }

    /// Slot 0xb8 of a combat object (`FUN_141203be0`): `amount` added up to its hit points with
    /// its powerups' room, an overflow taken off the powerup (`+0xa38`, never below 0; the hit
    /// point transforms of `FUN_141204450`, empty in the recordings, are not held).
    pub(crate) fn gain_hp(&mut self, master: &Master, amount: f32) {
        crate::combat::damage::gain_hp(
            &mut self.life.hp,
            &mut self.hp_powerups,
            master.hit_points,
            amount,
        );
    }

    /// Slot 0xb0 of a combat object (`FUN_141205c80`), the loss of `d` hit points: the hit point
    /// powerups' room (`+0xa38` up to `+0xa3c`) takes it first, the rest comes off the hit points,
    /// which stay above the master's `invulnerability_level` floor (`FUN_141202a80`). Not held
    /// (no update of the recordings fills either list): a dying object's kill credits of its
    /// active stingers (`+0xb58`, `+0x954`) and a surviving one's hit point transforms
    /// (`FUN_141204450`); [`crate::combat::damage::lose`] is the same loss on a hit's victim.
    pub(crate) fn lose_hp(&mut self, master: &Master, d: f32) {
        let mut d = d;
        let [up, room_max] = self.hp_powerups;
        if up < room_max {
            let room = room_max - up;
            if d <= room {
                self.hp_powerups[0] = up + d;
                return;
            }
            self.hp_powerups[0] = up + room;
            d -= room;
        }
        let floor =
            crate::combat::damage::invulnerability_floor(master.invulnerability, master.hit_points);
        let left = self.life.hp - d;
        self.life.hp = if floor == 0.0 || floor <= left {
            left
        } else {
            floor
        };
    }

    /// `TRIBE_Combat_Object::update`'s regeneration (`0x141208440` after
    /// `RGE_Combat_Object::update`): a living object (at least 1 hit point, `COMISS`/`JC`: a NaN
    /// is not; in state 2) whose rate ([`regeneration_rate`], slot 0x260) is above
    /// [`REGENERATION_EPSILON`] in size (a NaN is) earns a whole hit point per 60 / |rate|
    /// seconds: the world's dt plus the time held back (`berserker_timer`, `+0xa10`) over that
    /// period (none for a period of at most [`REGENERATION_EPSILON`]), its fraction (`fmodf`)
    /// held back as time. Whole points above 0 heal (a positive rate: slot 0xb8,
    /// [`Unit::gain_hp`]) at most the room below its most (slot 200: its master's hit points plus
    /// its powerups' room; the hit point stinger `+0xb44`, written only by auras, is not held), or
    /// come off (slot 0xb0, [`Unit::lose_hp`]) at most its hit points with its powerups' room
    /// (slot 0xc0). Then an object at its most (slot 0xc0 at least slot 200) has the armor its
    /// charge's hits took back (`+0x9e4` and `+0x9e6`, one dword, zeroed).
    pub(crate) fn regenerate(&mut self, master: &Master, dt_seconds: f32) {
        use crate::math::fmath::{at_most_or_unordered, below_or_unordered};
        let rate = regeneration_rate(master, self);
        if below_or_unordered(self.life.hp, REGENERATION_LEAST_HP) || self.life.state != 2 {
            return;
        }
        if !below_or_unordered(REGENERATION_EPSILON, rate.abs()) {
            return;
        }
        let held = dt_seconds + self.regen_timer;
        let per = (REGENERATION_MINUTE / rate).abs();
        let points = if below_or_unordered(REGENERATION_EPSILON, per.abs()) {
            held / per
        } else {
            0.0
        };
        let fraction = points % REGENERATION_POINT;
        let whole = points - fraction;
        self.regen_timer = fraction * per;
        if at_most_or_unordered(whole, 0.0) {
            return;
        }
        let [up, room] = self.hp_powerups;
        let most = f32::from(master.hit_points) + room;
        let now = (room - up) + self.life.hp;
        if at_most_or_unordered(rate, 0.0) {
            let amount = if at_most_or_unordered(whole, now) {
                whole
            } else {
                now
            };
            if !at_most_or_unordered(amount, 0.0) {
                self.lose_hp(master, amount);
            }
        } else {
            let left = most - now;
            let amount = if at_most_or_unordered(whole, left) {
                whole
            } else {
                left
            };
            if !at_most_or_unordered(amount, 0.0) {
                self.gain_hp(master, amount);
            }
        }
        // at its most hit points (slot 0xc0 at least slot 200, `COMISS`: a NaN is not) the
        // armor its charge's hits took (`+0x9e4`, `+0x9e6`) comes back
        let [up, room] = self.hp_powerups;
        if most <= (room - up) + self.life.hp {
            self.armor_removed = [0; 2];
        }
    }

    /// `TRIBE_Combat_Object::update`'s hit point powerup check (after its abilities): a powerup
    /// (`+0xa38`) above its room (`+0xa3c`; `COMISS`/`JBE`, a NaN is not) comes off the hit points
    /// (`MAXSS` with 1: not below 1, a NaN becomes 1) and the powerup takes the room.
    pub(crate) fn clamp_hp_powerup(&mut self) {
        let [up, room] = self.hp_powerups;
        if crate::math::fmath::at_most_or_unordered(up, room) {
            return;
        }
        self.life.hp =
            crate::math::fmath::maxss(self.life.hp - (room - up).abs(), HP_POWERUP_FLOOR);
        self.hp_powerups[0] = room;
    }
}
