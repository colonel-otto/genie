//! The player's resources and attributes: the modify and multiply slots, the store, the
//! setters and the side effects of allied vision and the farms' food.

use crate::GenieError;
use crate::objects::masters::master_type;
use crate::turn::Turns;

/// The attribute that turns the allied vision on (`FUN_141616820`'s and `FUN_14162d610`'s 0x32):
/// the player's `allied_LOS_Enable` (`+0x2cc`) takes 1, the world's masks are rebuilt (its slot
/// 0x78, [`Turns::rebuild_vision`]) and the player's map files every tile its masks reach as new
/// ([`Turns::refill_tiles`], `FUN_141285cb0`).
pub(super) const ALLIED_VISION: i16 = 0x32;
/// The attribute that shares every player's sight (0xb7): the masks are rebuilt.
pub(super) const SHARED_VISION: i16 = 0xb7;
/// The attribute of the town centres' flare reveal across the players (0xc4, Spies' "Treason"):
/// a player with it above 0 changes nothing; else each other player with it at most 0 takes the
/// amount too.
pub(super) const SPREAD_ATTRIBUTE: i16 = 0xc4;
/// The attribute whose set above 0 calls the scripts' `EffectFunction<n>` (0x21, `FUN_14162d610`).
pub(super) const EFFECT_FUNCTION: i16 = 0x21;
/// The attribute giving the exponent the farms' food grows by (31, `FUN_141615270`).
pub(super) const FARM_EXPONENT: i16 = 31;
/// The masters whose objects' food a farm-food change grows (`DAT_142f925d0`'s keys: 50, 1187,
/// 1897, 1893), in that order.
pub(super) const FARMS: [i16; 4] = [50, 1187, 1897, 1893];

impl Turns {
    /// Slot 0x1b0 (`FUN_141620c70`) of player `p` on resource `attribute` with `amount`: every
    /// live combat unit's and building's volleys lose the shares of the units inside, the
    /// attribute changes (`FUN_141661160`: `mode` 0 sets it, slot 0xf0
    /// [`Turns::set_player_attribute`]; any other adds to it, slot 0xf8 with no statistics,
    /// [`Turns::add_player_attribute`]; the local player's statistics are not simulation), and
    /// the volleys take the shares again. A change of resource 0x20 would not run in the screen's
    /// mode 4 (a playback's is not) or with the game's `+0x668` set (not held: taken unset).
    pub(super) fn modify_resource(
        &mut self,
        p: usize,
        attribute: i16,
        amount: f32,
        mode: u8,
    ) -> Result<(), GenieError> {
        self.all_garrison_volleys(p, false)?;
        if mode == 0 {
            self.set_player_attribute(p, attribute, amount)?;
        } else {
            self.add_player_attribute(p, attribute, amount)?;
        }
        self.all_garrison_volleys(p, true)
    }

    /// Slot 0x1b8 (`FUN_141620e30`) of player `p` on resource `attribute` by `v`: as
    /// [`Turns::modify_resource`], the attribute set to its value times `v` (`FUN_1416614c0`).
    pub(super) fn multiply_resource(
        &mut self,
        p: usize,
        attribute: i16,
        v: f32,
    ) -> Result<(), GenieError> {
        self.all_garrison_volleys(p, false)?;
        let value = self.attribute_of(p, attribute) * v;
        self.set_player_attribute(p, attribute, value)?;
        self.all_garrison_volleys(p, true)
    }

    /// `FUN_141217ac0` (`after` false) or `FUN_14120b910` on each of player `p`'s live combat
    /// units and buildings ([`Turns::garrison_volleys`]).
    pub(super) fn all_garrison_volleys(&mut self, p: usize, after: bool) -> Result<(), GenieError> {
        let ids: Vec<i32> = self.players[p].objects.iter().map(|u| u.id).collect();
        for id in ids {
            let kind = self
                .object(id)
                .map(|(_, u)| u.life.master_id)
                .and_then(|m| self.masters.get(p).and_then(|t| t.get(m)))
                .map(|m| m.kind);
            if matches!(kind, Some(master_type::COMBAT | master_type::BUILDING)) {
                self.garrison_volleys(p, id, after)?;
            }
        }
        Ok(())
    }

    /// `FUN_1416674b0`, the attribute's store: within the player's attributes' count it takes `v`
    /// (the resources are [`crate::turn::Player::resources`]). A computer player's resource
    /// peaks (`+0x6dc`) and attribute 262's fields (`+0x628`..`+0x630`) are not held.
    pub(super) fn store_attribute(&mut self, p: usize, attribute: i16, v: f32) {
        let player = &mut self.players[p];
        let Some(i) = usize::try_from(attribute)
            .ok()
            .filter(|&i| i < player.attributes.len())
        else {
            return;
        };
        if i < 4 {
            player.resources[i] = v;
        } else {
            player.attributes[i] = v;
        }
    }

    /// Slot 0xf0 (`FUN_14162d610`) of player `p`: attribute `attribute` takes `v`
    /// ([`Turns::store_attribute`]), with its side effects: 0x32 turns the allied vision on
    /// first ([`Turns::allied_vision_on`]); 0xc4 changes nothing for a player above 0 and spreads
    /// to the others at most 0 ([`SPREAD_ATTRIBUTE`]); 0xb7 rebuilds the masks after; 0x21 above 0
    /// calls the scripts' `EffectFunction<v>` (the XS runtime is not modelled).
    pub(crate) fn set_player_attribute(
        &mut self,
        p: usize,
        attribute: i16,
        v: f32,
    ) -> Result<(), GenieError> {
        match attribute {
            ALLIED_VISION => self.allied_vision_on(p)?,
            SPREAD_ATTRIBUTE => {
                if 0.0 < self.attribute_of(p, SPREAD_ATTRIBUTE) {
                    return Ok(());
                }
                // the message to the players is the interface's
                for q in 1..self.players.len() {
                    if q != p && self.attribute_of(q, SPREAD_ATTRIBUTE) <= 0.0 {
                        self.store_attribute(q, SPREAD_ATTRIBUTE, v);
                    }
                }
            }
            EFFECT_FUNCTION if 0.0 < v => {
                return Err(GenieError::unsupported(
                    "an effect's script function (attribute 0x21, EffectFunction in Effects.xs)",
                ));
            }
            _ => {}
        }
        self.store_attribute(p, attribute, v);
        if attribute == SHARED_VISION {
            // the world's slot 0x78; the message is the interface's
            self.rebuild_vision()?;
        }
        Ok(())
    }

    /// Slot 0xf8 (`FUN_141616820`) of player `p` with no statistics (`param_4` 0): `v` adds to
    /// attribute `attribute` ([`crate::turn::Player::add_attribute`]), with the side effects of
    /// 0x32 (the allied vision on first), 0xb7 (the masks rebuilt first, with the old value) and
    /// 0xc4 (nothing for a player above 0; the others at most 0 add it too).
    pub(crate) fn add_player_attribute(
        &mut self,
        p: usize,
        attribute: i16,
        v: f32,
    ) -> Result<(), GenieError> {
        match attribute {
            ALLIED_VISION => self.allied_vision_on(p)?,
            SHARED_VISION => self.rebuild_vision()?,
            SPREAD_ATTRIBUTE => {
                if 0.0 < self.attribute_of(p, SPREAD_ATTRIBUTE) {
                    return Ok(());
                }
                for q in 1..self.players.len() {
                    let theirs = self.attribute_of(q, SPREAD_ATTRIBUTE);
                    if q != p && theirs <= 0.0 {
                        self.store_attribute(q, SPREAD_ATTRIBUTE, v + theirs);
                    }
                }
            }
            _ => return self.players[p].add_attribute(attribute, v),
        }
        // FUN_14165d040's store of the sum
        let sum = v + self.attribute_of(p, attribute);
        self.store_attribute(p, attribute, sum);
        Ok(())
    }

    /// Attribute 0x32's side effect: player `p`'s allied vision (`+0x2cc`) on, the masks rebuilt
    /// (the world's slot 0x78, `FUN_1415e92b0`) and its tile list refilled
    /// ([`Turns::refill_tiles`], `FUN_141285cb0`).
    pub(super) fn allied_vision_on(&mut self, p: usize) -> Result<(), GenieError> {
        self.players[p].allied_vision = true;
        self.rebuild_vision()?;
        self.refill_tiles(p);
        Ok(())
    }

    /// `FUN_141615270` after player `p`'s change of the farms' food (from `before`, by `amount`):
    /// with attribute 31 at least 0 (`COMISS`, `JC`: not a NaN), each object the tracker files
    /// under the farm masters ([`FARMS`], their done objects, in that order,
    /// [`Turns::tracked_key`]) from type 40 on with an action (its Make) has the food still to make (`+0x44`) grow: with `held` its food
    /// (`+0x310`) and `f = held + still to make`, it takes `(f / before)^exponent × amount + f −
    /// held` (`powf`, here libm's).
    pub(super) fn farm_food(
        &mut self,
        p: usize,
        before: f32,
        amount: f32,
    ) -> Result<(), GenieError> {
        let exponent = self.attribute_of(p, FARM_EXPONENT);
        if crate::math::fmath::below_or_unordered(exponent, 0.0) {
            return Ok(());
        }
        let mut ids = Vec::new();
        for farm in FARMS {
            ids.extend(self.tracked_key(p, farm));
        }
        for id in ids {
            let Some((q, u)) = self.object(id) else {
                continue;
            };
            if self.master_of(q, u.life.master_id)?.kind < master_type::ACTION {
                continue;
            }
            let (_, u) = self.object_mut(id).expect("found above");
            let held = u.life.amount;
            let Some(make) = u.make.as_mut() else {
                continue;
            };
            let f = held + make.remaining;
            let grown = libm::powf(f / before, exponent);
            make.remaining = (grown * amount + f) - held;
        }
        Ok(())
    }
}
