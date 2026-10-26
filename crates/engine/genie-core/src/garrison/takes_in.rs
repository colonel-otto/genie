//! A building's test whether it takes a unit in (`FUN_14120eb10`), its room and what it holds.

use crate::GenieError;
use crate::garrison::{BUILDING_CLASS, CLASS_FLAGS, COMBAT_KIND, LEAST_HP_SHARE, TRANSPORTED};
use crate::objects::masters::Task;
use crate::orders::{BUILDING, GARRISON};
use crate::turn::{Turns, World};
use crate::unit_ai::task_finder::Diplomacy;

/// `FUN_1411205e0`: whether a master's class (`+0x410`, the short `MOVSX` widens) has one of
/// `flags` in the class-flags table: a class below 0 or not below the table's count
/// (`CMP`/`JGE`, signed) has none.
#[must_use]
pub fn class_has_flags(class: i16, flags: u32) -> bool {
    usize::try_from(class)
        .ok()
        .and_then(|c| CLASS_FLAGS.get(c))
        .is_some_and(|&f| f & flags != 0)
}

/// The unit masters whose own rules `FUN_14120eb10` keeps apart from their class's (the game
/// compares their ids, + 0x609c).
pub(super) const MASTER_1263: i16 = 1263;
pub(super) const MASTER_775: i16 = 775;
pub(super) const MASTER_1715: i16 = 1715;
pub(super) const MASTER_1808: i16 = 1808;
pub(super) const MASTER_1811: i16 = 1811;
pub(super) const MASTER_1822: i16 = 1822;

/// Whether `garrison_type` (a building master's `+0x9bc`) takes in a unit of `class` and master
/// `id` (the switch of `FUN_14120eb10` on the class): bit 0 villagers (classes 4, 19, 59), bit 1
/// archers and infantry (0, 6, 35, 44, 45, 46, 50, 56; not masters 1263, 1811), bit 2 cavalry
/// (12, 23, 36, 47, 57), bit 3 monks (18, 42, 43; not masters 775, 1715, 1822), bit 4 herdables
/// (58), bit 5 siege (13, 51, 54, 55), bit 6 ships (2, 20, 21, 22, 53); then bit 2 for masters
/// 1263, 775, 1715 and 1822 and bit 3 for masters 1811 and 1808, whatever their class. The task
/// finder's copy of the switch (`FUN_1412784d0`) leaves class 42 out (`with_42` false).
#[must_use]
pub fn garrison_type_takes(garrison_type: u8, class: i16, id: i16, with_42: bool) -> bool {
    let bit = |b: u8| garrison_type & (1 << b) != 0;
    let by_class = match class {
        0 | 44 => bit(1),
        2 | 20 | 21 | 22 | 53 => bit(6),
        4 | 19 | 59 => bit(0),
        6 | 35 | 45 | 46 | 50 | 56 => id != MASTER_1263 && id != MASTER_1811 && bit(1),
        12 | 23 | 36 | 47 | 57 => bit(2),
        13 | 51 | 54 | 55 => bit(5),
        18 | 43 => id != MASTER_775 && id != MASTER_1715 && id != MASTER_1822 && bit(3),
        42 if with_42 => id != MASTER_775 && id != MASTER_1715 && id != MASTER_1822 && bit(3),
        58 => bit(4),
        _ => false,
    };
    let as_cavalry = matches!(id, MASTER_1263 | MASTER_775 | MASTER_1715 | MASTER_1822) && bit(2);
    let as_monk = matches!(id, MASTER_1811 | MASTER_1808) && bit(3);
    by_class || as_cavalry || as_monk
}

/// `FUN_1415c9240`: the master's task list holds a task of action type `action` on objects of
/// class `class`, both bytes the game widens to compare with the task's ushorts (`+0xe`,
/// `+0x10`). A building's test asks for 3 on its own class (a transport) or on buildings (3).
pub(super) fn has_garrison_task(tasks: &[Task], action: i16, class: i16) -> bool {
    let byte = |v: i16| u16::from(v.to_le_bytes()[0]);
    tasks.iter().any(|t| {
        t.action_type.cast_unsigned() == byte(action)
            && t.object_group.cast_unsigned() == byte(class)
    })
}

/// A garrison task (action type 3) on buildings (class 3).
#[must_use]
pub const fn is_garrison_task(task: &Task) -> bool {
    task.action_type == GARRISON && task.object_group == BUILDING_CLASS
}

impl Turns {
    /// Slot 0x198 of a building (`FUN_1412169b0`) `b` of player `bp` for `unit`: its count
    /// ([`Turns::garrisoned`]) plus the unit and what it holds (1 and its own count; 1 for no
    /// unit) within its master's capacity (`obj_max`).
    pub(crate) fn has_room(
        &self,
        bp: usize,
        b: &crate::turn::Unit,
        unit: Option<i32>,
    ) -> Result<bool, GenieError> {
        let capacity = i32::from(self.master_of(bp, b.life.master_id)?.garrison);
        let with = match unit {
            Some(u) => self.garrisoned(u)? + 1,
            None => 1,
        };
        Ok(self.garrisoned(b.id)? + with <= capacity)
    }
}

impl Turns {
    /// `FUN_14120eb10(building, player, master, 1, 1)` ([`Turns::takes_in_as`]).
    pub(crate) fn takes_in(
        &self,
        building: i32,
        player: usize,
        master: i16,
    ) -> Result<bool, GenieError> {
        self.takes_in_as(building, player, master, true, true)
    }

    /// `FUN_14120eb10`: whether `building` takes a unit of `player`'s master `master` in. The
    /// player must have the master; another player's building must be allowed (`others`, the
    /// game's `param_4` 1) and allied both ways (`FUN_141663d90`); the building in state 2. A
    /// building whose `object_flags` bit 0 is set (a transport) takes a unit of type 70 or of a
    /// class with a flag of 0x10002 ([`class_has_flags`]) when the unit has a task of action 3 on
    /// its own class ([`has_garrison_task`]); a unit of neither goes on as for a building. Any
    /// other takes a unit of type 70 or more when it is of type 80
    /// (an annex asks its main building, `+0xbb8`), above [`LEAST_HP_SHARE`] of its master's hit
    /// points, with a capacity (`obj_max`), the unit's master with a garrison task on buildings
    /// and of a kind the building's `garrison_type` takes ([`garrison_type_takes`]). Either then
    /// needs room for one more (slot 0x198 with no unit, [`Turns::has_room`]) unless `unchecked`
    /// (`param_5` not 0).
    pub(crate) fn takes_in_as(
        &self,
        building: i32,
        player: usize,
        master: i16,
        others: bool,
        unchecked: bool,
    ) -> Result<bool, GenieError> {
        let Some(unit) = self.masters.get(player).and_then(|m| m.get(master)) else {
            return Ok(false);
        };
        let Some((bp, b)) = self.object(building) else {
            return Ok(false);
        };
        if bp != player {
            let world = World {
                players: &self.players,
                drop_sites: &self.drop_sites,
            };
            if !others || !world.allied(bp, player, true) || !world.allied(player, bp, true) {
                return Ok(false);
            }
        }
        if b.life.state != 2 {
            return Ok(false);
        }
        let own = self.master_of(bp, b.life.master_id)?;
        if own.object_flags & 1 != 0
            && (unit.kind == COMBAT_KIND || class_has_flags(unit.class, TRANSPORTED))
        {
            if !has_garrison_task(&unit.tasks, GARRISON, own.class) {
                return Ok(false);
            }
            return Ok(unchecked || self.has_room(bp, b, None)?);
        }
        if unit.kind < COMBAT_KIND || own.kind != BUILDING {
            return Ok(false);
        }
        let (bp, b) = match b.annex_of {
            Some(main) => self.object(main).ok_or(GenieError::unsupported(
                "an annex whose main building is gone",
            ))?,
            None => (bp, b),
        };
        let bm = self.master_of(bp, b.life.master_id)?;
        // CVTPS2PD, CVTDQ2PD, MULSD, then COMISD with JBE: an unordered hit point count takes
        // nothing
        let max_hp = f64::from(f32::from(bm.hit_points));
        let above = f64::from(b.life.hp) > max_hp * LEAST_HP_SHARE;
        if !above || bm.garrison == 0 {
            return Ok(false);
        }
        if !has_garrison_task(&unit.tasks, GARRISON, BUILDING_CLASS) {
            return Ok(false);
        }
        let garrison_type = bm.building.map_or(0, |m| m.garrison_type);
        if !garrison_type_takes(garrison_type, unit.class, unit.id, true) {
            return Ok(false);
        }
        Ok(unchecked || self.has_room(bp, b, None)?)
    }

    /// Slot 0x498 of the object `id`: the objects inside it (`+0x2f8`'s count, relics too) and
    /// what its own `+0x9a8` (`inside_garrison_count`, a saved byte) adds (`FUN_141213500`); a
    /// building (`FUN_1411d0ce0`) less its `+0xc2c`, the units of type 70 that came in with a
    /// running `raider_build_countdown` (`+0x9a0`, `FUN_1411cb850`/`FUN_1411d6ad0`). Not
    /// modelled: an object inside that holds others (the slice reads no `+0x9a8`) and, in a
    /// building, a unit with its countdown set (`+0xc2c`, which the slice does not keep).
    pub(crate) fn garrisoned(&self, building: i32) -> Result<i32, GenieError> {
        let holds = |id: i32| {
            self.players.iter().any(|q| {
                q.objects
                    .iter()
                    .chain(&q.sleeping)
                    .any(|v| v.life.inside == Some(id))
            })
        };
        let is_building = self
            .object(building)
            .map(|(bp, b)| {
                self.master_of(bp, b.life.master_id)
                    .map(|m| m.kind == BUILDING)
            })
            .transpose()?
            .unwrap_or(false);
        let mut count = 0;
        for (p, player) in self.players.iter().enumerate() {
            for u in player.objects.iter().chain(&player.sleeping) {
                if u.life.inside != Some(building) {
                    continue;
                }
                let kind = self.master_of(p, u.life.master_id)?.kind;
                if is_building && kind == COMBAT_KIND && u.raider_countdown != 0 {
                    return Err(GenieError::unsupported(
                        "a unit inside a building with its raider_build_countdown (+0xc2c)",
                    ));
                }
                if holds(u.id) {
                    return Err(GenieError::unsupported(
                        "an object inside a building that holds others (+0x9a8)",
                    ));
                }
                count += 1;
            }
        }
        Ok(count)
    }
}
