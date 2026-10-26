//! What a repair reaches and pays: a building's and a combat unit's slot 0x8c8 by the
//! repairer's player.

use crate::GenieError;
use crate::construction::repair::ONE;

/// The classes whose repair pays its unit share (`FUN_1412143b0`: the ships 20, 21, 22, 2 and
/// 53, the siege 13 and the classes 51, 54 and 55).
pub(super) const COSTED: [i16; 9] = [20, 21, 22, 2, 53, 13, 51, 54, 55];
/// The trebuchets' classes (51, 54), whose building repair is the unit's.
pub(super) const TREBUCHETS: [i16; 2] = [51, 54];
/// The owner's attributes of a unit's and a building's repair cost (270, 271: `player+0x100`
/// at `+0x870` and `+0x878`).
pub const UNIT_REPAIR_COST: usize = 270;
/// The building repair cost attribute (271).
pub const BUILDING_REPAIR_COST: usize = 271;

/// One object a repair reaches (the target, an annex's main building, a main building's
/// annexes) as the repair reads and changes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Repaired {
    /// The object.
    pub id: i32,
    /// Its master's type.
    pub kind: u8,
    /// Its master's class.
    pub class: i16,
    /// Its master.
    pub master: i16,
    /// Its master's hit points (`+0x530`).
    pub master_hp: i16,
    /// Its hit points (`+0x278`).
    pub hp: f32,
    /// Its hit point powerup and its room (`+0xa38`, `+0xa3c`).
    pub hp_powerups: [f32; 2],
    /// An annex's main building (`+0xbb8`).
    pub main: Option<i32>,
    /// A main building's annexes (`+0xbc0`, its four slots in their order).
    pub annexes: Vec<i32>,
    /// Its owner's attributes 270 and 271 ([`UNIT_REPAIR_COST`], [`BUILDING_REPAIR_COST`]).
    pub cost_rates: [f32; 2],
}

/// What a repair pays with: the repairer's player's masters and attributes (`FUN_14162fb60`
/// with the repairer's player).
pub struct Payer<'a> {
    /// The repairer's player's masters.
    pub masters: &'a crate::objects::masters::Masters,
    /// The repairer's player's attributes.
    pub attributes: &'a mut dyn crate::technology::Attributes,
}

impl std::fmt::Debug for Payer<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Payer").finish_non_exhaustive()
    }
}

/// The object of `set` with id `id`.
pub(super) fn find(set: &[Repaired], id: i32) -> Result<usize, GenieError> {
    set.iter()
        .position(|o| o.id == id)
        .ok_or(GenieError::unreachable("a repair's object outside its set"))
}

/// A building's slot 0x8c8 (`FUN_1411d37d0`) by `amount` for the repairer (`payer`): a
/// trebuchet's class repairs as a unit ([`repair_unit`]); one below 1 hit point (`COMISS`: a NaN
/// is) takes nothing (2). From its repairer (`top`) an annex's repair is its main building's.
/// Else each of its annexes (a combat unit's or a building's type) is repaired by the amount
/// first (not `top`); then the amount, at most what its hit points lack of its master's, costs
/// its owner's building repair rate times the amount over its master's hit points of its
/// master's costs ([`crate::production::pay`]); paid, it heals as a unit's does, else 2.
///
/// # Errors
///
/// An object outside `set`, and an attribute the player does not have.
pub fn repair_building(
    set: &mut [Repaired],
    idx: usize,
    amount: f32,
    top: bool,
    payer: &mut Payer<'_>,
) -> Result<u8, GenieError> {
    if TREBUCHETS.contains(&set[idx].class) {
        return repair_unit(set, idx, amount, payer);
    }
    if crate::math::fmath::below_or_unordered(set[idx].hp, ONE) {
        return Ok(2);
    }
    if top && let Some(main) = set[idx].main {
        let m = find(set, main)?;
        return repair_building(set, m, amount, true, payer);
    }
    for annex in set[idx].annexes.clone() {
        let a = find(set, annex)?;
        match set[a].kind {
            70 => {
                let _ = repair_unit(set, a, amount, payer)?;
            }
            80 => {
                let _ = repair_building(set, a, amount, false, payer)?;
            }
            _ => {}
        }
    }
    let o = &set[idx];
    let most = f32::from(o.master_hp);
    let mut amount = amount;
    if most < o.hp + amount {
        amount = most - o.hp;
    }
    let share = o.cost_rates[1] * amount / most;
    if crate::production::pay(payer.masters, o.master, share, &mut *payer.attributes)? {
        return repair_unit(set, idx, amount, payer);
    }
    Ok(2)
}

/// A combat unit's slot 0x8c8 (`FUN_1412143b0`, also a building's last step) by `amount`: a
/// class of [`COSTED`] first takes at most what its hit points lack of its most (slots 0xc0 and
/// 200) and pays its owner's unit repair rate times the amount over its most of its master's
/// costs (unpaid: 2); then it heals (`FUN_141193b30`, [`crate::combat::damage::heal_hp`]): 1 once full,
/// else 0.
///
/// # Errors
///
/// An attribute the player does not have.
pub fn repair_unit(
    set: &mut [Repaired],
    idx: usize,
    amount: f32,
    payer: &mut Payer<'_>,
) -> Result<u8, GenieError> {
    let o = &mut set[idx];
    let mut amount = amount;
    if COSTED.contains(&o.class) {
        let [up, room] = o.hp_powerups;
        let most = f32::from(o.master_hp) + room;
        if most < ((room - up) + o.hp) + amount {
            amount = most - ((room - up) + o.hp);
        }
        let share = o.cost_rates[0] * amount / most;
        if !crate::production::pay(payer.masters, o.master, share, &mut *payer.attributes)? {
            return Ok(2);
        }
    }
    let full = crate::combat::damage::heal_hp(&mut o.hp, &mut o.hp_powerups, o.master_hp, amount);
    Ok(u8::from(full))
}
