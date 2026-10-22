//! Garrisoning: `UNIT_ORDER`'s garrison (action 5 on a building, `FUN_14161e370` ->
//! `FUN_141621220`), which sends each unit to a building that takes it in, nearest first, each
//! building up to its capacity, and gives each building's units, through the order splitter,
//! the garrison work on it (slot 0x7e8 with task filter 3); and a building's test whether it takes
//! a unit in (`FUN_14120eb10`).

use crate::checkpoint::cvttss2si;
use crate::fmath::{at_most_or_unordered, maxss};
use crate::masters::{Master, Masters, Task};
use crate::move_to::{Goal, MoveTo, Walk, start_walk, walk_on};
use crate::movement::footprint_gap_squared;
use crate::order::{BUILDING, GARRISON, Work};
use crate::tasks::Diplomacy;
use crate::turn::{Turns, World};
use crate::unit_ai::Unsupported;

/// The squared footprint gap past which a garrison order sends no unit (`FUN_141621220` with 0:
/// no limit).
const FAR: f32 = crate::image::garrison::FAR;
/// A building takes units in only above this share of its master's hit points (`FUN_14120eb10`,
/// in doubles).
const LEAST_HP_SHARE: f64 = crate::image::garrison::LEAST_HP_SHARE;
/// The class of buildings, which a unit's garrison task names (`FUN_1415c9240` with 3, 3).
pub(crate) const BUILDING_CLASS: i16 = 3;
/// A gate's class: its annex is not its main building for a garrison order.
const GATE_CLASS: i16 = 39;
/// The class of relics, which a building's count leaves out (`+0xc2c`).
const RELIC_CLASS: i16 = 1;
/// The ids a command's check reads (`FUN_140e558b0`): a human player's first 60, a computer
/// player's first 500.
const HUMAN_IDS: usize = 60;
const COMPUTER_IDS: usize = 500;
/// `SEEK_SHELTER`'s villagers: class 4 of type 70.
const VILLAGER_CLASS: i16 = 4;
const VILLAGER_KIND: u8 = 70;

/// The unit masters whose own rules `FUN_14120eb10` keeps apart from their class's (the game
/// compares their ids, + 0x609c).
const MASTER_1263: i16 = 1263;
const MASTER_775: i16 = 775;
const MASTER_1715: i16 = 1715;
const MASTER_1808: i16 = 1808;
const MASTER_1811: i16 = 1811;
const MASTER_1822: i16 = 1822;

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
fn has_garrison_task(tasks: &[Task], action: i16, class: i16) -> bool {
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

/// The unit AI's order of entering (`0x2cd`), which the garrison work notifies.
pub const ENTER_ORDER: i32 = crate::ai_kinds::ORDER_ENTER;
/// The unit AI's action value of entering (`0x269`), which the garrison work notifies and the
/// Enter's notifies carry.
pub const ENTER_ACTION: i32 = 0x269;

/// A combat master's attack rate needs a weapon range above this (`FUN_14120dc10`).
const RATE_RANGE: f32 = crate::image::garrison::RATE_RANGE;
/// A unit's firepower counts its volley missiles beyond this many (`FUN_14120e1a0`).
const ONE_MISSILE: f32 = crate::image::garrison::ONE_MISSILE;
/// A reload at most this long gives no firepower (`FUN_14120e1a0`).
const EPSILON: f32 = crate::image::garrison::EPSILON;
/// The armor classes the attack rates sum: pierce (3) and 38 for missiles; melee (4) and 31 too
/// for a master's own weapons.
const PIERCE: i16 = 3;
const MELEE: i16 = 4;
const CLASS_31: i16 = 31;
const CLASS_38: i16 = 38;
/// The class whose garrison share is 1 whatever its firepower (`FUN_141213520`, `-0xe82` raw);
/// UNLOAD's flag 0 leaves it inside (`FUN_14118a610`).
const WHOLE_SHARE_CLASS: i16 = 42;
/// UNLOAD's point without one (`DAT_14330ec20`).
const NO_POINT: f32 = crate::image::garrison::NO_POINT;

/// The amounts of `weapons` (class, amount) whose class `counts`.
fn weapons_sum(weapons: &[(i16, i16)], counts: impl Fn(i16) -> bool) -> f32 {
    weapons
        .iter()
        .filter(|&&(class, _)| counts(class))
        .fold(0.0, |sum, &(_, amount)| sum + f32::from(amount))
}

/// `FUN_14120dc10`: the attack rate of an object of `master` (its reload powerup `+0xa28`
/// taken off its reload): none without a reload or a range above 1 (an unordered one too);
/// else the pierce and class 38 amounts of its volley missile (a master its player has; not for
/// classes 4 and 6), else its own melee, pierce, class 31 and 38 amounts, over the reload.
#[must_use]
pub fn attack_rate(master: &Master, masters: &Masters, reload_powerup: f32) -> f32 {
    // COMISS reload, 0 and COMISS range, 1.0, each with JBE
    if at_most_or_unordered(master.reload, 0.0)
        || at_most_or_unordered(master.weapon_range, RATE_RANGE)
    {
        return 0.0;
    }
    // the game reads the slot of an id below the player's master count without testing it: a
    // null slot there would crash it, so a master the player has is the only case it runs
    let volley = master
        .volley_missile
        .and_then(|id| masters.get(id))
        .filter(|_| !matches!(master.class, 4 | 6));
    let sum = volley.map_or_else(
        || {
            weapons_sum(&master.weapons, |c| {
                matches!(c, MELEE | PIERCE | CLASS_31 | CLASS_38)
            })
        },
        |v| weapons_sum(&v.weapons, |c| matches!(c, PIERCE | CLASS_38)),
    );
    sum / (master.reload - reload_powerup)
}

/// Whether the player's master table has a slot for `id` (`0 <= id < player+0x20`), as far as
/// [`Masters`] knows it: up to its last master. The game's count can run past the last master
/// with null slots, which [`Masters`] does not keep.
fn in_table(masters: &Masters, id: i16) -> bool {
    0 <= id && masters.iter().any(|m| id <= m.id)
}

/// `FUN_14120e1a0`: what a unit of `master` (its reload powerup `reload_powerup`) fires inside a
/// building. A master's garrison firepower at most 0 is its magnitude; else the firepower times
/// its rate: the pierce and class 38 amounts of its volley missile (else its missile; the slot
/// the player's table holds, nothing for a null one) times its volley fire past one missile (in
/// whole missiles, `CVTTSS2SI`, the products wrapping), plus its own melee, pierce, class 31 and
/// 38 amounts, over its reload (none at most 1e-7).
#[must_use]
pub fn firepower(master: &Master, masters: &Masters, reload_powerup: f32) -> f32 {
    let g = master.garrison_firepower;
    // COMISS 0, g; JC: an unordered firepower goes on
    if g <= 0.0 {
        return g.abs();
    }
    let mut sum = 0.0f32;
    // COMISS volley, 1.0; JBE: an unordered volley fire adds no missiles
    if ONE_MISSILE < master.volley_fire {
        // the volley missile's slot when the table has one, else the missile's (an id out of
        // the table gives none); a null slot or one without weapons adds nothing
        let missile = master
            .volley_missile
            .filter(|&id| in_table(masters, id))
            .map_or_else(|| masters.get(master.missile), |id| masters.get(id));
        if let Some(m) = missile {
            let extra = cvttss2si(master.volley_fire - ONE_MISSILE);
            for &(class, amount) in &m.weapons {
                if matches!(class, PIERCE | CLASS_38) {
                    // IMUL EAX, R8D then CVTDQ2PS
                    sum += i32::from(amount).wrapping_mul(extra) as f32;
                }
            }
        }
    }
    // one sum, the missile's amounts first
    for &(class, amount) in &master.weapons {
        if matches!(class, MELEE | PIERCE | CLASS_31 | CLASS_38) {
            sum += f32::from(amount);
        }
    }
    let reload = master.reload - reload_powerup;
    if reload <= EPSILON {
        return 0.0;
    }
    sum / reload * g
}

/// The type of a combat unit: the master a transforming master names (`+0x656`), and a unit a
/// transport takes in (`FUN_14120eb10`).
const COMBAT_KIND: u8 = 70;

/// `FUN_141213520`: the share of a building's volleys a unit of `master` inside it adds, against
/// the building's attack rate `rate` ([`attack_rate`]): none against a rate of ±0 (`_fdclass`);
/// 1 for class 42; else its [`firepower`] over the rate, at least the floor (`MAXSS`): 0, or for
/// a master with object flag 3 the firepower over the rate of the master the flags' upper half
/// names (`+0x656`, the `.dat`'s trait piece) when that one is of type 70.
pub fn share(
    master: &Master,
    masters: &Masters,
    reload_powerup: f32,
    rate: f32,
) -> Result<f32, Unsupported> {
    if rate == 0.0 {
        return Ok(0.0);
    }
    if master.class == WHOLE_SHARE_CLASS {
        return Ok(1.0);
    }
    let mut floor = 0.0;
    if master.object_flags & 8 != 0 {
        // MOVZX of the ushort at +0x656; the game reads the slot without a test, so a null one
        // would crash it: a master the player has is the only case it runs
        let other = u16::try_from(master.object_flags >> 16).unwrap_or(u16::MAX);
        if let Some(m) = i16::try_from(other).ok().and_then(|id| masters.get(id))
            && m.kind == COMBAT_KIND
        {
            floor = firepower(m, masters, reload_powerup) / rate;
        }
    }
    Ok(maxss(
        firepower(master, masters, reload_powerup) / rate,
        floor,
    ))
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
    ) -> Result<bool, Unsupported> {
        let capacity = i32::from(self.master_of(bp, b.life.master_id)?.garrison);
        let with = match unit {
            Some(u) => self.garrisoned(u)? + 1,
            None => 1,
        };
        Ok(self.garrisoned(b.id)? + with <= capacity)
    }

    /// The work command's Enter (its action type 3): the action takes its building (an annex its
    /// main building, slot 0x178), the unit AI's notify (slot 0x828: the enter order and action,
    /// the building with its class and the work's point), the action list's clearing and the
    /// Enter's start ([`EnterAction::start`]) with the building's place as its point, the task's
    /// work range and class.
    pub(crate) fn start_enter(
        &mut self,
        player: usize,
        idx: usize,
        target: i32,
        task: &Task,
        point: [f32; 3],
        clears: bool,
    ) -> Result<(), Unsupported> {
        let target = self
            .object(target)
            .and_then(|(tp, t)| {
                let kind = self.masters.get(tp)?.get(t.life.master_id)?.kind;
                Some(if kind == BUILDING {
                    t.annex_of.unwrap_or(target)
                } else {
                    target
                })
            })
            .ok_or(Unsupported("an Enter whose building is gone"))?;
        let (tp, t) = self.object(target).expect("found above");
        let tm = self.master_of(tp, t.life.master_id)?;
        if tm.kind != BUILDING {
            return Err(Unsupported("an Enter into a unit (a transport)"));
        }
        let class = i32::from(tm.class);
        self.take(target);
        let id = self.players[player].objects[idx].id;
        if let Some(ai) = self.players[player].objects[idx].ai.as_mut() {
            ai.notify_work(
                ENTER_ORDER,
                ENTER_ACTION,
                Some((target, class)),
                point,
                clears,
                self.streams.get(crate::rng::Stream::Ai),
            )?;
        }
        self.clear_actions(player, idx)?;
        let unit = &self.players[player].objects[idx];
        if unit.life.inside.is_some() {
            return Err(Unsupported(
                "an Enter of a unit inside an object (state 13)",
            ));
        }
        let (tp, t) = self.object(target).expect("found above");
        if !self.has_room(tp, t, Some(id))? {
            return Err(Unsupported(
                "an Enter into a building without room (state 11)",
            ));
        }
        let me = self
            .spatial
            .get_known(id)?
            .ok_or(Unsupported("an entering unit without a place"))?;
        let them = self
            .spatial
            .get_known(target)?
            .ok_or(Unsupported("an Enter's building without a place"))?;
        let gap =
            crate::movement::footprint_gap(me.position, me.radius, them.position, them.radius);
        let gap_squared = footprint_gap_squared(me.position, me.radius, them.position, them.radius);
        let speed = unit.mover.as_ref().map_or(0.0, |m| m.stats.speed);
        let in_reach = EnterAction::in_reach(task.work_range, gap_squared, 0.0)?;
        let action = EnterAction::start(
            target,
            them.position,
            task.work_range,
            task.object_group,
            gap,
            speed,
            in_reach,
        )?;
        self.players[player].objects[idx].enter = Some(action);
        if in_reach {
            self.enter_now(player, idx, target)?;
        }
        Ok(())
    }

    /// The enter at the Enter's start ([`crate::turn::Unit::enter_building`], then the start's
    /// own `set_state(1)`); the load it gives goes to the owner's attributes at once.
    fn enter_now(&mut self, player: usize, idx: usize, target: i32) -> Result<(), Unsupported> {
        let inside = self
            .players
            .iter()
            .flat_map(|p| p.objects.iter().chain(&p.sleeping))
            .filter(|u| u.life.inside == Some(target))
            .count();
        let mut unit = std::mem::take(&mut self.players[player].objects[idx]);
        let mut adds = Vec::new();
        let result = (|| {
            let building = self.players[player]
                .objects
                .iter()
                .find(|u| u.id == target)
                .ok_or(Unsupported("an enter into a building of another player"))?;
            let owner = &self.players[player];
            let mut w = crate::turn::EnterWorld {
                spatial: &mut self.spatial,
                pathing: &mut self.pathing,
                masters: self.masters.get(player),
                graphics: &self.graphics,
                sprite_rng: self.streams.get(crate::rng::Stream::Sprite),
                drop_sites: &self.drop_sites,
                attributes: &owner.attributes,
                civ: owner.civ,
                sharing: owner.sharing,
                player,
                attribute_adds: &mut adds,
                firepower: 0.0,
            };
            if !unit.enter_building(building, inside, &mut w)? {
                return Err(Unsupported("an Enter in reach that did not get in"));
            }
            unit.entered(&mut w)?;
            Ok(w.firepower)
        })();
        self.players[player].objects[idx] = unit;
        let firepower = result?;
        let id = self.players[player].objects[idx].id;
        if let Some(b) = self.players[player]
            .objects
            .iter_mut()
            .find(|u| u.id == target)
        {
            b.firepower += firepower;
            b.garrison.push(id);
        }
        for (kind, amount) in adds {
            self.players[player].add_attribute(kind, amount)?;
        }
        Ok(())
    }

    /// UNLOAD (`FUN_14161f410`) of player `player`'s objects `ids` (each alive, at least 1 hit
    /// point): a building lets the units inside it out, from the last to come in, those `flag`
    /// and `unit_type` pick ([`unload_picks`]): each through [`Turns::let_out`], toward the point
    /// (or its unit AI's command queue location, which is not modelled); each one out has its
    /// order queue emptied (`FUN_140c5e010`). A unit inside a building leaves it toward the point
    /// (with its sound, no work), its orders gone and, of type 70 and up, no longer under the
    /// town bell. A unit inside a transport, a unit with units inside it (a transport's unload)
    /// and the other units' steps (a siege unit's, the group leave, the ignore list and the
    /// unit AI's order 0x2d1) are not modelled; the local player's interface flag
    /// (`FUN_141663f50`) is not held. A unit (type 70) out is no longer under the town bell
    /// (`+0x9dc`).
    pub(crate) fn unload(
        &mut self,
        player: usize,
        ids: &[i32],
        point: [f32; 2],
        flag: u8,
        unit_type: i32,
        sound: bool,
    ) -> Result<(), Unsupported> {
        for &id in ids {
            let Some(idx) = self.players[player].objects.iter().position(|u| u.id == id) else {
                continue;
            };
            let unit = &self.players[player].objects[idx];
            if unit.life.hp < 1.0 {
                continue;
            }
            // a unit inside a building leaves it toward the point (`FUN_1411e7220` with no
            // work and its sound); a combat unit is no longer under the town bell (`+0x9dc`),
            // and its unit AI's orders go (`FUN_140c5e010`)
            if let Some(container) = unit.life.inside {
                let building = match self.place(container) {
                    Some(crate::order::Place::Live(cp, ci)) => {
                        let c = &self.players[cp].objects[ci];
                        (BUILDING <= self.master_of(cp, c.life.master_id)?.kind).then_some((cp, ci))
                    }
                    _ => None,
                };
                let Some((cp, ci)) = building else {
                    return Err(Unsupported(
                        "an unload of a unit inside a transport (or a sleeping building)",
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
                continue;
            }
            if self.master_of(player, unit.life.master_id)?.kind < BUILDING {
                if unit.garrison.is_empty() {
                    continue;
                }
                return Err(Unsupported(
                    "an unload of a unit with units inside (a transport)",
                ));
            }
            if unit
                .ai
                .as_ref()
                .is_some_and(|ai| !ai.building_orders.is_empty())
            {
                return Err(Unsupported(
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
        }
        Ok(())
    }

    /// `GO_BACK_TO_WORK` (`do_command` 0x80 with one object, 0x86 with several:
    /// `FUN_141625590`) on the player's objects `ids` (another player's only while players may
    /// order each other's, [`Turns::allied_control`]; the cheat `+0xb12` is not held): a building
    /// lets out, from the last in, each unit inside of type 70 or more that is a villager (class
    /// 4) or of master 1808, back to its work ([`Turns::back_to_work`]).
    pub(crate) fn go_back_to_work(
        &mut self,
        player: usize,
        ids: &[i32],
    ) -> Result<(), Unsupported> {
        for id in self.checked_units(player, ids) {
            let Some((p, _)) = self.object(id) else {
                continue;
            };
            let Some(idx) = self.players[p].objects.iter().position(|u| u.id == id) else {
                return Err(Unsupported("a GO_BACK_TO_WORK of a sleeping object"));
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
                    self.back_to_work(p, idx, inside)?;
                }
            }
        }
        Ok(())
    }

    /// `FUN_1412081e0` for the unit `id` inside the building at `building` of player `p`: the
    /// work to go back to ([`TownBell::back_to_work`]: the town-bell records' object the world
    /// holds); with none but a record of an action and a target class, the unit AI takes the
    /// action (`FUN_140c64b50`) and the unit tells it to search (notify 0x202 with the action,
    /// the class and the old target). Then the unit leaves the building toward the record's point
    /// with the object to work on ([`Turns::let_out`], `FUN_1411e7220` with its sound); out, it
    /// is no longer under the town bell (`+0x9dc`; `+0xa14` takes 1, not held).
    ///
    /// [`TownBell::back_to_work`]: crate::town_bell::TownBell::back_to_work
    fn back_to_work(&mut self, p: usize, building: usize, id: i32) -> Result<(), Unsupported> {
        use crate::town_bell::RecordTarget;
        let (up, ui) = self.live_inside(id)?;
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
        let out = self.let_out(p, building, id, record.point, (found.map(|f| f.id), true))?;
        if out && let Some((_, u)) = self.object_mut(id) {
            u.town_bell.flag = false;
        }
        Ok(())
    }

    /// `FUN_14118a610`: whether UNLOAD's `flag` and `unit_type` (`param`) pick `unit` of player
    /// `player`, by its master's class and id: flag 0 every unit but class 42; 1 the units of
    /// player `param`; 2 a group of classes ([`unload_group_takes`]); 3 the object `param`; with
    /// the object `param` of the world (`FUN_1415417a0`; the unit's player's or none), 4 the units
    /// of its master (a villager: any villager for a villager), 5 every unit but it and class 42,
    /// 6 the units not of its master (no villager for a villager, never class 42). Other flags
    /// pick none.
    fn unload_picks(
        &self,
        player: usize,
        unit: &crate::turn::Unit,
        flag: u8,
        param: i32,
    ) -> Result<bool, Unsupported> {
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

    /// The player and live index of the unit `id` inside a building, whoever's it is.
    fn live_inside(&self, id: i32) -> Result<(usize, usize), Unsupported> {
        match self.place(id) {
            Some(crate::order::Place::Live(p, i)) => Ok((p, i)),
            _ => Err(Unsupported(
                "a unit inside a building outside the live lists",
            )),
        }
    }

    /// `FUN_1411e7220`: the unit `id` (any player's, with its own player's masters and orders)
    /// leaves the building at `building` of player `player` toward `point`, when a spot beside
    /// it is free ([`Turns::spot_beside`], toward the point, or the building's gather point when
    /// not on itself): it takes the spot at the building's
    /// height (`FUN_14119d610`), leaves the building (slot 0x1a8, `FUN_141213120`: the
    /// building's slot 0x1c8, `FUN_1411d6ad0` -> `FUN_141217910`, takes its share off the
    /// volleys, back to its master's volley fire when none is left inside or it fell below it,
    /// and lets it out of its list, `FUN_141197e10`; the building's sight lets the circle of 5
    /// go, slot 0x1d8 with 5) and goes on the map there (slot 0x58: its tile's list, its
    /// obstruction record, the mesh, its sight). With a gather point (`+0xc30` 1) not on the
    /// building and no point, it then goes to work on the gather point's target when the world
    /// holds it (its unit AI forgets its target, `FUN_140c5e1e0`; [`Turns::gather`]); else, or
    /// with a point, it moves there ([`Turns::gather_move`] after the same forgetting). False
    /// when no spot is free. Not modelled: the work given (slot 0x128 with a target), neither a
    /// point nor a gather point (the unit AI's slots 0xb0 and 0x1c0), a gather point of kind 0xff,
    /// class 42's and livestock's counts in a building of a player with `+0x88`, a siege
    /// building's (class 13) slot 0x1b0; not held: the building's villager count (`+0xba8`), its
    /// sound (slot 0x4c0), its garrison sprite, the unit's `+0x9a0`. Its work command (slot
    /// 0x128) has `+0xa14` 0: the unit AI keeps its notifies and its queued orders.
    pub(crate) fn let_out(
        &mut self,
        player: usize,
        building: usize,
        id: i32,
        point: [f32; 2],
        (work_on, _sound): (Option<i32>, bool),
    ) -> Result<bool, Unsupported> {
        let b = &self.players[player].objects[building];
        let bid = b.id;
        let bm = self.master_of(player, b.life.master_id)?.clone();
        let gather = b.gather_point;
        let toward = if point[0] == NO_POINT {
            gather
                .filter(|g| g.target != bid)
                .map(|g| [g.point[0], g.point[1]])
        } else {
            Some(point)
        };
        if point[0] == NO_POINT && gather.is_none() {
            return Err(Unsupported(
                "an unload without a gather point (the unit AI's command queue location)",
            ));
        }
        // the unit is any player's (an ally's too): its own player's masters and orders
        let (up, ui) = self.live_inside(id)?;
        let unit = &self.players[up].objects[ui];
        let made = self.master_of(up, unit.life.master_id)?.clone();
        if bm.class == 13 {
            return Err(Unsupported("an unload of a siege building (class 13)"));
        }
        if made.class == WHOLE_SHARE_CLASS || made.class == 58 {
            return Err(Unsupported(
                "a class 42 or livestock unit leaving a building (its player's counts)",
            ));
        }
        if !unit.garrison.is_empty() {
            return Err(Unsupported("a unit with units inside leaving a building"));
        }
        let Ok(spot) = self.spot_beside(player, building, &made, toward) else {
            return Err(Unsupported(
                "an unload whose unit finds no spot (its later rings)",
            ));
        };
        let z = self.players[player].objects[building].position[2];
        let at = [spot[0], spot[1], z];
        // the building's slot 0x1c8 (`FUN_141217910`): a unit (type 70) takes its share off the
        // volleys (the building's rate by its player's masters, the unit's share by its own)
        let share = if made.kind == COMBAT_KIND {
            let (Some(own), Some(theirs)) = (self.masters.get(player), self.masters.get(up)) else {
                return Err(Unsupported("an unload without the masters"));
            };
            let b = &self.players[player].objects[building];
            let unit = &self.players[up].objects[ui];
            let rate = attack_rate(&bm, own, b.reload_powerup);
            Some(share(&made, theirs, unit.reload_powerup, rate)?)
        } else {
            None
        };
        let b = &mut self.players[player].objects[building];
        if let Some(share) = share {
            b.firepower -= share;
        }
        // the unit out of its list (`FUN_141197e10`)
        if let Some(k) = b.garrison.iter().position(|&g| g == id) {
            let _ = b.garrison.swap_remove(k);
        }
        // COMISS volley, firepower; JBE keeps it: a NaN firepower stays
        if b.garrison.is_empty() || b.firepower < bm.volley_fire {
            b.firepower = bm.volley_fire;
        }
        let (b_at, b_radius) = (b.position, bm.radius);
        if let Some(v) = self.spatial.sight.as_mut() {
            v.sight_off_los(bid, b_at, b_radius, false, 5, false);
        }
        // slot 0x58: on the map at the spot (new_position: its tile's list, its sight)
        let mut placed = self
            .spatial
            .come_out(id)
            .ok_or(Unsupported("a unit inside a building without its record"))?;
        placed.position = at;
        let solid = 0.0 < made.radius[0] && 0.0 < made.radius[1] && made.tall;
        placed.obstruction =
            (made.obstruction_type != 0 && solid).then_some(crate::spatial::Obstruction {
                kind: made.obstruction_type,
                class: made.obstruction_class,
            });
        self.spatial.insert(id, placed);
        self.pathing.moved(id, &placed, false);
        self.pathing.tiles_changed(self.spatial.take_changes());
        let world_time = self.world_time;
        let unit = &mut self.players[up].objects[ui];
        unit.position = at;
        unit.life.inside = None;
        // the position setter's end: a place off where it last moved to is a move
        if let Some(m) = unit.mover.as_mut() {
            m.mark_move(at, world_time);
        }
        let state = unit.life.state;
        if let Some(v) = self.spatial.sight.as_mut()
            && state == 2
        {
            let _ = v.sight_on(id, at, made.radius, false, false);
        }
        if let Some(e) = self.spatial.take_sight_unsupported() {
            return Err(e);
        }
        // an object to work on (GO_BACK_TO_WORK's): slot 0x128 on it at the point, the unit's
        // `+0xa14` 0 (its unit AI keeps its notifies)
        if let Some(target) = work_on {
            let work = Work {
                target,
                point: [point[0], point[1], 0.0],
                carry_test: false,
                filter: -1,
                notified: false,
            };
            self.order_work(up, ui, &work)?;
            return Ok(true);
        }
        // the gather point (`+0xc30` 1, not on the building) gives a unit without a point its
        // target, when the world holds it, else its point
        let to = match gather {
            Some(g) if point[0] == NO_POINT && g.target != bid => {
                if g.target != -1 && self.object(g.target).is_some() {
                    if let Some(ai) = self.players[up].objects[ui].ai.as_mut() {
                        ai.remove_target();
                    }
                    // slot 0x128 with `+0xa14` 0: the unit AI keeps its notifies
                    self.gather(up, ui, g, bid, false, false)?;
                    return Ok(true);
                }
                [g.point[0], g.point[1]]
            }
            _ => point,
        };
        if to[0] == NO_POINT || to[1] == NO_POINT {
            return Err(Unsupported(
                "an unloaded unit without a point (its unit AI's slots 0xb0 and 0x1c0)",
            ));
        }
        // a move to the point: the unit AI forgets its target (`FUN_140c5e1e0`), then slot 0x2a0
        // and slot 0x128 with no target, `+0xa14` 0
        if let Some(ai) = self.players[up].objects[ui].ai.as_mut() {
            ai.remove_target();
        }
        self.gather_move(up, ui, [to[0], to[1], 0.0], false)?;
        Ok(true)
    }

    /// `FUN_14120eb10(building, player, master, 1, 1)` ([`Turns::takes_in_as`]).
    pub(crate) fn takes_in(
        &self,
        building: i32,
        player: usize,
        master: i16,
    ) -> Result<bool, Unsupported> {
        self.takes_in_as(building, player, master, true, true)
    }

    /// `FUN_14120eb10`: whether `building` takes a unit of `player`'s master `master` in. The
    /// player must have the master; another player's building must be allowed (`others`, the
    /// game's `param_4` 1) and allied both ways (`FUN_141663d90`); the building in state 2. A
    /// building whose `object_flags` bit 0 is set (a transport) takes a unit of type 70 (or of a
    /// class of flag 0x10002, `FUN_1411205e0`) with a task of action 3 on its own class
    /// ([`has_garrison_task`]). Any other takes a unit of type 70 or more when it is of type 80
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
    ) -> Result<bool, Unsupported> {
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
        if own.object_flags & 1 != 0 {
            if unit.kind != COMBAT_KIND {
                return Err(Unsupported(
                    "a transport's garrison test of a unit not of type 70 (the class flags \
                     0x10002, FUN_1411205e0)",
                ));
            }
            if !has_garrison_task(&unit.tasks, GARRISON, own.class) {
                return Ok(false);
            }
            return Ok(unchecked || self.has_room(bp, b, None)?);
        }
        if unit.kind < COMBAT_KIND || own.kind != BUILDING {
            return Ok(false);
        }
        let (bp, b) = match b.annex_of {
            Some(main) => self
                .object(main)
                .ok_or(Unsupported("an annex whose main building is gone"))?,
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

    /// A building's slot 0x498 (`FUN_1411d0ce0`): the objects inside it (`+0x2f8`'s count) and
    /// what they hold (`+0x9a8`), less its relics (`+0xc2c`). An object inside that holds others
    /// and a relic inside are not modelled.
    fn garrisoned(&self, building: i32) -> Result<i32, Unsupported> {
        let holds = |id: i32| {
            self.players.iter().any(|q| {
                q.objects
                    .iter()
                    .chain(&q.sleeping)
                    .any(|v| v.life.inside == Some(id))
            })
        };
        let mut count = 0;
        for (p, player) in self.players.iter().enumerate() {
            for u in player.objects.iter().chain(&player.sleeping) {
                if u.life.inside != Some(building) {
                    continue;
                }
                if self.master_of(p, u.life.master_id)?.class == RELIC_CLASS {
                    return Err(Unsupported("a relic inside a building (+0xc2c)"));
                }
                if holds(u.id) {
                    return Err(Unsupported(
                        "an object inside a building that holds others (+0x9a8)",
                    ));
                }
                count += 1;
            }
        }
        Ok(count)
    }

    /// `UNIT_ORDER`'s garrison (action 5) on `target`, which the world holds (`FUN_140e7f200`; the
    /// UI's sound for it, `FUN_1409648c0`, is not held): `FUN_14161e370` lists the order's units
    /// (the ids the world holds of a type 70 or more) and, for a target of type 80, the target
    /// (an annex its main building, but a gate's) with its count ([`Turns::garrisoned`]); then
    /// [`Turns::shelter`]. A target of a lower type (a transport) and the buildings near the
    /// target (`+0x1e`) are not modelled.
    pub(crate) fn garrison_order(
        &mut self,
        player: usize,
        ids: &[i32],
        target: i32,
        through_ai: bool,
        nearby: bool,
    ) -> Result<(), Unsupported> {
        let Some((tp, t)) = self.object(target) else {
            return Ok(());
        };
        if nearby {
            return Err(Unsupported(
                "a garrison into the buildings near its target (+0x1e)",
            ));
        }
        let tm = self.master_of(tp, t.life.master_id)?;
        if tm.kind < BUILDING {
            return Err(Unsupported("a garrison into a unit (a transport)"));
        }
        let building = match t.annex_of {
            Some(main) if tm.class != GATE_CLASS => main,
            _ => target,
        };
        let count = self.garrisoned(building)?;
        let mut units = Vec::with_capacity(ids.len());
        for &id in ids {
            let Some((p, u)) = self.object(id) else {
                continue;
            };
            if 70 <= self.master_of(p, u.life.master_id)?.kind {
                units.push(id);
            }
        }
        self.shelter(player, &units, &[(building, count)], through_ai)
    }

    /// `SEEK_SHELTER` (0x2c, `FUN_140e6a3a0`), after the command's check (`FUN_140e558b0`): of
    /// the first 60 `ids` (500 for a computer player), an id the world does not hold, and another
    /// player's unit unless players may order each other's ([`Turns::allied_control`],
    /// `+0x1074`), are dropped ([`Turns::checked_units`]); with every id dropped the command does
    /// nothing. Of the others, the villagers (class 4 of type 70) and the units of master 1808
    /// seek shelter (a villager of master 1808 is listed twice) in the player's buildings that
    /// take villagers: in the player's object order, those of type 80 in state 2, not inside
    /// another (`+0x2f0`, `FUN_141188da0`), whose master's `garrison_type` has bit 0
    /// (`FUN_1411ccb60` with 1), each with its count ([`Turns::garrisoned`]); then
    /// [`Turns::shelter`] (`FUN_141622450`).
    pub(crate) fn seek_shelter(&mut self, player: usize, ids: &[i32]) -> Result<(), Unsupported> {
        let cap = if self.players[player].computer {
            COMPUTER_IDS
        } else {
            HUMAN_IDS
        };
        let checked = self.checked_units(player, &ids[..ids.len().min(cap)]);
        // a human's command whose ids all dropped is refused; run (the cheat +0xb12) on no
        // units it would only read the buildings' counts
        if checked.iter().all(|&id| id == -1) {
            return Ok(());
        }
        let mut units = Vec::with_capacity(checked.len());
        for id in checked {
            let Some((p, u)) = self.object(id) else {
                continue;
            };
            let m = self.master_of(p, u.life.master_id)?;
            if m.class == VILLAGER_CLASS && m.kind == VILLAGER_KIND {
                units.push(id);
            }
            if m.id == MASTER_1808 {
                units.push(id);
            }
        }
        let mut buildings = Vec::new();
        for b in &self.players[player].objects {
            let m = self.master_of(player, b.life.master_id)?;
            let takes_villagers = m.building.is_some_and(|x| x.garrison_type & 1 != 0);
            if b.life.state == 2 && b.life.inside.is_none() && m.kind == BUILDING && takes_villagers
            {
                buildings.push(b.id);
            }
        }
        let buildings = buildings
            .into_iter()
            .map(|b| Ok((b, self.garrisoned(b)?)))
            .collect::<Result<Vec<_>, Unsupported>>()?;
        self.shelter(player, &units, &buildings, false)
    }

    /// `FUN_141621220` with no transports and no gap limit (its `param_4` empty, `param_6` 0):
    /// each of `units` pairs with each of `buildings` (with its count inside) whose master has a
    /// capacity (`obj_max`) and takes the unit's master's copy in ([`Turns::takes_in`]), within
    /// [`FAR`]; the pairs sorted by their squared footprint gap (`FUN_1415f0830`, MSVC's
    /// `std::sort`) give each unit, in that order, the first building with room, then any unit
    /// left its nearest building. The buildings go in a hash map by id (`FUN_1415f0c40`), whose
    /// order ([`crate::msvc::unordered_map_order`]) the next part follows: each building's units
    /// go through the order splitter, which drops those whose unit AI's test fails
    /// (`FUN_140c40e00`: the building takes the unit's master in), and each loose unit (not in a
    /// group, `+0x6d0`; in state 2 or 6 or less, of a type 70 or more) takes slot 0x7e8 on the
    /// building at its place with task filter 3. The unit AI's queue (`through_ai`, `param_7`)
    /// and the groups the splitter forms are not modelled.
    pub(crate) fn shelter(
        &mut self,
        player: usize,
        units: &[i32],
        buildings: &[(i32, i32)],
        through_ai: bool,
    ) -> Result<(), Unsupported> {
        let ground = |id: i32| {
            self.spatial
                .get_known(id)?
                .map(|p| (p.position, p.radius))
                .ok_or(Unsupported("a garrison's object without a place"))
        };
        // the pairs of a unit and a building: (unit index, building index, squared gap)
        let mut pairs = Vec::new();
        let mut capacities = Vec::with_capacity(buildings.len());
        for &(building, _) in buildings {
            let (bp, b) = self
                .object(building)
                .ok_or(Unsupported("a garrison's building is gone"))?;
            let bm = self.master_of(bp, b.life.master_id)?;
            capacities.push((bm.kind == BUILDING, i32::from(bm.garrison)));
        }
        for (i, &id) in units.iter().enumerate() {
            let (p, u) = self
                .object(id)
                .ok_or(Unsupported("a garrison's unit is gone"))?;
            let copy = self.master_of(p, u.life.master_id)?.copy_id;
            for (j, &(building, _)) in buildings.iter().enumerate() {
                let (is_building, capacity) = capacities[j];
                if is_building && capacity != 0 && self.takes_in(building, player, copy)? {
                    let (at, r) = ground(id)?;
                    let (bat, br) = ground(building)?;
                    let gap = footprint_gap_squared(at, r, bat, br);
                    if gap < FAR {
                        pairs.push((i, j, gap));
                    }
                }
            }
        }
        crate::msvc::sort(
            &mut pairs,
            &|a: (usize, usize, f32), b: (usize, usize, f32)| a.2 < b.2,
        );
        let mut counts: Vec<i32> = buildings.iter().map(|&(_, count)| count).collect();
        let mut taken = vec![false; units.len()];
        // each building's units, by building index, and the order of the buildings' first take
        let mut going: Vec<Vec<i32>> = vec![Vec::new(); buildings.len()];
        let mut first_taken = Vec::new();
        for &(i, j, _) in &pairs {
            if !taken[i] && counts[j] < capacities[j].1 {
                going[j].push(units[i]);
                first_taken.push(buildings[j].0);
                counts[j] += 1;
                taken[i] = true;
            }
        }
        for &(i, j, _) in &pairs {
            if !taken[i] {
                going[j].push(units[i]);
                first_taken.push(buildings[j].0);
                taken[i] = true;
            }
        }
        for building in crate::msvc::unordered_map_order(first_taken) {
            let j = buildings
                .iter()
                .position(|&(b, _)| b == building)
                .expect("a building of the list");
            self.shelter_in(building, &going[j], through_ai)?;
        }
        Ok(())
    }

    /// `FUN_140c40e00`: a unit AI's test of the building `b` of player `bp` for its unit of
    /// player `player` and master `master`: none for an object below type 70, else whether it
    /// takes the unit in ([`Turns::takes_in`], with 1 and 1).
    fn ai_takes(
        &self,
        bp: usize,
        b: &crate::turn::Unit,
        player: usize,
        master: i16,
    ) -> Result<bool, Unsupported> {
        if self.master_of(bp, b.life.master_id)?.kind < COMBAT_KIND {
            return Ok(false);
        }
        self.takes_in(b.id, player, master)
    }

    /// One building's part of [`Turns::shelter`]: the order splitter (`FUN_140fd9dc0` with
    /// command `0x2713`) on its units, which keeps those whose unit AI's test passes
    /// ([`Turns::ai_takes`]), then slot 0x7e8 for each loose unit.
    fn shelter_in(
        &mut self,
        building: i32,
        going: &[i32],
        through_ai: bool,
    ) -> Result<(), Unsupported> {
        let (bp, b) = self
            .object(building)
            .ok_or(Unsupported("a garrison's building is gone"))?;
        // the splitter tests a unit with a unit AI only
        let keeps = |id: i32| -> Result<bool, Unsupported> {
            let (p, u) = self.object(id).expect("a going unit");
            if u.ai.is_none() {
                return Ok(true);
            }
            self.ai_takes(bp, b, p, u.life.master_id)
        };
        let point = b.position;
        let mut kept = Vec::with_capacity(going.len());
        for &id in going {
            if self.object(id).is_some() && keeps(id)? {
                kept.push(id);
            }
        }
        let loose = self.loose_units_kept(going, &|id| kept.contains(&id))?;
        for id in loose {
            let Some(crate::order::Place::Live(p, idx)) = self.place(id) else {
                return Err(Unsupported(
                    "a garrison of a unit the slice does not hold live",
                ));
            };
            let unit = &self.players[p].objects[idx];
            let kind = self.master_of(p, unit.life.master_id)?.kind;
            if (2 < unit.life.state && unit.life.state != 6) || kind < 70 {
                continue;
            }
            if through_ai && unit.ai.is_some() {
                return Err(Unsupported(
                    "a garrison through the unit AI's queue (+0x1d, slot 0x30 with 0x2cd)",
                ));
            }
            let work = Work {
                target: building,
                point,
                carry_test: false,
                filter: GARRISON,
                notified: true,
            };
            self.order_work(p, idx, &work)?;
        }
        Ok(())
    }
}

/// The Enter action's type (`RGE_Action_Enter`, vtable `0x142ed4db8`), which word 9 adds.
pub const ENTER: i16 = 3;

/// The gap below which the Enter's `MoveTo` starts arrived (`0x140aa2100`, `DAT_142eb99cc`).
const ARRIVED: f32 = crate::image::move_to::ARRIVED;

/// The Enter action's states the slice models: 1 done (inside), 2 waiting to try again, 4
/// walking to the target.
pub const ENTERED: u8 = 1;
const WAIT: u8 = 2;
const WALK: u8 = 4;

/// A unit's Enter action (`RGE_Action_Enter`, type 3): it walks to a building and enters it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnterAction {
    /// The building (`+0x20`, its id `+0x30`).
    pub target: i32,
    /// The action state (`+0x18`).
    pub state: u8,
    /// Its `MoveTo` sub-action's state.
    pub move_to: Option<MoveTo>,
    /// `+0x38`..`+0x40`: where it meets the target (`FUN_140a85f30`: a building's place), which
    /// a target that moves leaves.
    pub point: [f32; 3],
    /// The task's work range (`+0x30`): its `MoveTo` stops this near, and the enter needs it.
    pub range: f32,
    /// `+0x44`: state 2's wait, in seconds.
    pub timer: f32,
    /// `+0x68`, 1 at first: a `MoveTo` that fails (code 3) asks once more.
    pub retry: bool,
    /// `+0x6c`: the task's class (its `+0x10`).
    pub task_class: i16,
}

/// What an Enter's update reads of its building.
#[derive(Clone, Copy, Debug)]
pub struct EnterTarget {
    /// Its state (`+0x640`).
    pub state: u8,
    /// The unit's player counts its player as allied (`FUN_141663dc0`), or it is Gaia's.
    pub allied: bool,
    /// It has room for the unit (slot 0x198).
    pub room: bool,
    /// Its place.
    pub position: [f32; 3],
    /// The unit is in reach of it ([`EnterAction::in_reach`]).
    pub in_reach: bool,
}

/// What an Enter's update leaves to its caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EnterStep {
    /// The update returned 1 (state 1): the action list removes it.
    pub done: bool,
    /// The unit reached the building: the caller enters it (`FUN_140a867b0`), then the action's
    /// `set_state(1)` ([`EnterAction::entered`]) or, when it could not, `set_state(2)`
    /// ([`EnterAction::wait`]).
    pub enter: bool,
}

impl EnterAction {
    /// The factory's constructor (`FUN_140a855d0`: the target, an annex's main building, is the
    /// caller's, as its add, slot 0x178; the point `FUN_140a85f30` gives a building, its place)
    /// and the install's start (slot 5, `FUN_140a86f20`) with its `set_state(4)` (`FUN_140a87320`)
    /// on a building with room: a unit in reach (`FUN_140a85c30`) enters at once, which the
    /// caller does (`FUN_140a867b0`, then `set_state(1)`); else a `MoveTo` to it within the
    /// task's range (`FUN_140aa0df0`, quiet, its start `0x140aa2100`: within [`ARRIVED`] of its
    /// footprint already in state 1, else 3). A unit inside an object (`set_state` 13), a
    /// building without room (`FUN_140a864d0`, then `set_state` 11) and a unit without speed (the
    /// `MoveTo`'s state 13) are not modelled.
    pub fn start(
        target: i32,
        point: [f32; 3],
        range: f32,
        task_class: i16,
        gap: f32,
        speed: f32,
        in_reach: bool,
    ) -> Result<Self, Unsupported> {
        let move_to = if in_reach {
            None
        } else if ARRIVED < gap {
            if speed == 0.0 {
                return Err(Unsupported("an Enter's MoveTo of a unit without speed"));
            }
            Some(MoveTo::Start)
        } else {
            Some(MoveTo::Arrived)
        };
        Ok(Self {
            target,
            state: WALK,
            move_to,
            point,
            range,
            timer: 0.0,
            retry: true,
            task_class,
        })
    }

    /// `FUN_140a85c30`: the unit is in reach of the building: their footprints' squared gap
    /// (`FUN_14118f8c0`) at most the range squared. A range of at most 1 to a target with speed
    /// (the tile test around the two) is not modelled.
    pub fn in_reach(range: f32, gap_squared: f32, target_speed: f32) -> Result<bool, Unsupported> {
        if range * range < gap_squared {
            return Ok(false);
        }
        if range <= 1.0 && 0.0 < target_speed {
            return Err(Unsupported("an Enter's reach of a target with speed"));
        }
        Ok(true)
    }

    /// One update (slot 6, `FUN_140a87a80`). It first checks its building: one gone, dead (state
    /// 3 or more) or of an unallied player, or without room for the unit, makes the action let
    /// it go and turn to state 11, which is not modelled. State 1 is done; state 2 waits its timer
    /// (by the world's `dt`), then walks again (`set_state(4)`). State 4: a building that left
    /// the action's point by more than 0.5 makes it walk again (`FUN_140a7bd10`; a building never
    /// moves: not modelled); then the `MoveTo` updates (`FUN_140a992b0`: code 1 once it ended, or
    /// without it): codes 1 and 2 enter in reach ([`EnterStep::enter`]), else wait (2); code 3
    /// (a `MoveTo` that failed) walks again once (`+0x68`), then waits; codes 4 and 5 let the
    /// building go (state 11), which is not modelled.
    pub fn update(
        &mut self,
        walk: Walk<'_>,
        target: Option<EnterTarget>,
    ) -> Result<EnterStep, Unsupported> {
        let mut step = EnterStep::default();
        let Some(t) = target else {
            return Err(Unsupported("an Enter whose building is gone (state 11)"));
        };
        if 3 <= t.state || !t.allied {
            return Err(Unsupported(
                "an Enter whose building died or is not allied (state 11)",
            ));
        }
        if !t.room {
            return Err(Unsupported(
                "an Enter whose building has no room (state 11)",
            ));
        }
        match self.state {
            ENTERED => step.done = true,
            WAIT => {
                self.timer -= walk.dt;
                if self.timer <= 0.0 {
                    return Err(Unsupported("an Enter walking again after its wait"));
                }
            }
            WALK => {
                if t.position != self.point {
                    return Err(Unsupported("an Enter whose building moved"));
                }
                let code = match self.move_to {
                    None | Some(MoveTo::Arrived) => 1,
                    Some(MoveTo::Start) => {
                        self.move_to = Some(MoveTo::Moving);
                        let range = self.range;
                        self.move_to =
                            Some(start_walk(walk, Goal::Target(self.target), range, true)?);
                        0
                    }
                    Some(MoveTo::Moving) => {
                        self.move_to = Some(walk_on(walk)?);
                        0
                    }
                    Some(MoveTo::Failed) => 3,
                    Some(MoveTo::Waiting) => {
                        return Err(Unsupported("an Enter's waiting MoveTo"));
                    }
                };
                match code {
                    1 | 2 if t.in_reach => step.enter = true,
                    1 | 2 => {
                        return Err(Unsupported("an Enter that arrived out of reach (state 2)"));
                    }
                    3 => return Err(Unsupported("an Enter whose MoveTo failed")),
                    _ => {}
                }
            }
            _ => return Err(Unsupported("an Enter state not modelled")),
        }
        Ok(step)
    }
}

/// UNLOAD's flag 2 (`FUN_14118a610`): the classes of group `group` (1 villagers and class 19;
/// 2 classes 6, 45, 46, 50 and 56; 3 classes 12, 23, 36, 47 and 57; 5 classes 0 and 44; 6
/// classes 18, 42 and 43; no other group).
const fn unload_group_takes(group: i32, class: i16) -> bool {
    match group {
        1 => matches!(class, VILLAGER_CLASS | 19),
        2 => matches!(class, 6 | 45 | 46 | 50 | 56),
        3 => matches!(class, 12 | 23 | 36 | 47 | 57),
        5 => matches!(class, 0 | 44),
        6 => matches!(class, 18 | WHOLE_SHARE_CLASS | 43),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::masters::BuildingMaster;
    use crate::turn::{Player, Unit};

    const TC: i16 = 109;
    const VILLAGER: i16 = 83;
    const ARCHER: i16 = 4;
    const SHIP: i16 = 545;
    const GATE: i16 = 88;

    fn task(action_type: i16, object_group: i16) -> Task {
        Task {
            action_type,
            object_group,
            ..Task::default()
        }
    }

    /// A town centre (garrison type 11, room for 15), a villager and an archer that may enter
    /// buildings, a transport ship (object flag 0, room for 5) and a gate.
    fn garrison_masters() -> Masters {
        Masters::new([
            Master {
                id: TC,
                kind: 80,
                class: 52,
                hit_points: 2400,
                garrison: 15,
                building: Some(BuildingMaster {
                    garrison_type: 11,
                    ..BuildingMaster::default()
                }),
                ..Master::default()
            },
            Master {
                id: VILLAGER,
                kind: 70,
                class: 4,
                copy_id: VILLAGER,
                tasks: vec![task(GARRISON, 3), task(GARRISON, 20)],
                ..Master::default()
            },
            Master {
                id: ARCHER,
                kind: 70,
                class: 0,
                tasks: vec![task(7, 3)],
                ..Master::default()
            },
            Master {
                id: SHIP,
                kind: 70,
                class: 20,
                object_flags: 1,
                garrison: 1,
                ..Master::default()
            },
            Master {
                id: GATE,
                kind: 80,
                class: 39,
                hit_points: 2750,
                garrison: 0,
                ..Master::default()
            },
        ])
    }

    fn unit(id: i32, master: i16, hp: f32) -> Unit {
        let mut u = Unit {
            id,
            pasture: -1,
            my_pasture: -1,
            ..Unit::default()
        };
        u.life.master_id = master;
        u.life.hp = hp;
        u.life.state = 2;
        u
    }

    /// Gaia and three players; players 1 and 2 allied both ways, player 3 an enemy. Player 1
    /// holds town centre 100, ship 200 and gate 300 (an annex of the town centre).
    fn garrison_world() -> Turns {
        let mut players: Vec<Player> = (0..4).map(|_| Player::default()).collect();
        players[1].relations = vec![1, 0, 0, 3];
        players[2].relations = vec![1, 0, 0, 3];
        players[3].relations = vec![1, 3, 3, 0];
        let mut t = Turns::new(0, 0, 0, vec![0; 4], players);
        t.masters = (0..4).map(|_| garrison_masters()).collect();
        t.players[1].objects.push(unit(100, TC, 2400.0));
        t.players[1].objects.push(unit(200, SHIP, 100.0));
        let mut gate = unit(300, GATE, 2750.0);
        gate.annex_of = Some(100);
        t.players[1].objects.push(gate);
        t
    }

    /// UNLOAD's class groups (flag 2) as `FUN_14118a610` tests them; group 3 is the bit mask
    /// `0x200801000801000` over classes below 58.
    #[test]
    fn the_unload_groups() {
        let group = |g| {
            (0..64)
                .filter(|&c| unload_group_takes(g, c))
                .collect::<Vec<_>>()
        };
        assert_eq!(group(1), [4, 19]);
        assert_eq!(group(3), [12, 23, 36, 47, 57]);
        assert_eq!(group(6), [18, 42, 43]);
        assert_eq!(group(4), [0i16; 0]);
    }

    #[test]
    fn an_enter_walks_to_its_building_unless_it_is_in_reach() {
        // MarvinBlue/426048731's 9171 garrisons the town centre at (104, 47) from 3.5 tiles off
        let far = EnterAction::start(9149, [104.0, 47.0, 0.0], 1.0, 3, 3.5, 0.8, false).unwrap();
        assert_eq!((far.state, far.move_to), (WALK, Some(MoveTo::Start)));
        // 10049 of update 4409 stands within the task's range: it enters at once
        let near = EnterAction::start(9149, [104.0, 47.0, 0.0], 1.0, 3, 0.6, 0.8, true).unwrap();
        assert_eq!(near.move_to, None);
        // the reach is the footprints' squared gap against the range squared
        assert!(EnterAction::in_reach(1.0, 1.0, 0.0).unwrap());
        assert!(!EnterAction::in_reach(1.0, 1.01, 0.0).unwrap());
        assert!(EnterAction::in_reach(1.0, 0.5, 0.8).is_err());
    }

    #[test]
    fn a_town_center_takes_villagers_infantry_cavalry_and_monks() {
        // a town center's garrison_type 11: bits 0, 1 and 3
        assert!(garrison_type_takes(11, 4, 83, true));
        assert!(garrison_type_takes(11, 6, 74, true));
        assert!(!garrison_type_takes(11, 12, 38, true));
        assert!(garrison_type_takes(11, 18, 125, true));
        // master 1811 is no infantry, but a monk
        assert!(garrison_type_takes(11, 6, MASTER_1811, true));
        assert!(!garrison_type_takes(2, 6, MASTER_1811, true));
        // a tower's 3 takes no monk; the task finder takes no class 42
        assert!(!garrison_type_takes(3, 18, 125, true));
        assert!(garrison_type_takes(8, 42, 1, true));
        assert!(!garrison_type_takes(8, 42, 1, false));
    }

    #[test]
    fn a_villager_inside_a_town_center_adds_one_missile_to_its_volleys() {
        // MarvinBlue/426048731's town center 9149 (the tick oracle, update 4439): pierce 5 by its
        // volley missile 54 every 2 s; its 7 villagers made its firepower 1 + 7
        let arrow = Master {
            id: 54,
            kind: 60,
            weapons: vec![(3, 5)],
            ..Master::default()
        };
        let tc = Master {
            id: 109,
            kind: 80,
            class: 3,
            reload: 2.0,
            weapon_range: 6.0,
            weapons: vec![(3, 5)],
            volley_missile: Some(54),
            volley_fire: 1.0,
            max_volley: 11,
            ..Master::default()
        };
        let villager = Master {
            id: 83,
            kind: 70,
            class: 4,
            reload: 2.0,
            weapons: vec![(11, 3), (4, 3), (13, 6), (24, 0), (17, 0)],
            garrison_firepower: -2.5,
            ..Master::default()
        };
        let masters = Masters::new([arrow, tc.clone(), villager.clone()]);
        let rate = attack_rate(&tc, &masters, 0.0);
        assert_eq!(rate, 2.5);
        // a garrison firepower at most 0 is its magnitude
        assert_eq!(firepower(&villager, &masters, 0.0), 2.5);
        assert_eq!(share(&villager, &masters, 0.0, rate), Ok(1.0));
        assert_eq!(share(&villager, &masters, 0.0, 0.0), Ok(0.0));
        // one above 0 times the melee, pierce, class 31 and 38 amounts over the reload
        let armed = Master {
            garrison_firepower: 0.5,
            ..villager
        };
        assert_eq!(firepower(&armed, &masters, 0.0), 0.75);
    }

    /// A town centre's master as `a_villager_inside_a_town_center_adds_one_missile_to_its_volleys`
    /// has it.
    fn town_center() -> Master {
        Master {
            id: 109,
            kind: 80,
            class: 3,
            reload: 2.0,
            weapon_range: 6.0,
            weapons: vec![(3, 5)],
            volley_missile: Some(54),
            volley_fire: 1.0,
            ..Master::default()
        }
    }

    #[test]
    fn an_attack_rate_needs_a_reload_and_a_range_that_compare_above_their_limits() {
        // FUN_14120dc10: COMISS reload, 0 and COMISS range, 1.0, each with JBE: an unordered
        // value gives no rate, as one at the limit does
        let arrow = Master {
            id: 54,
            kind: 60,
            weapons: vec![(3, 5), (4, 7), (38, 2)],
            ..Master::default()
        };
        let masters = Masters::new([arrow, town_center()]);
        assert_eq!(attack_rate(&town_center(), &masters, 0.0), 3.5);
        for (reload, range) in [(f32::NAN, 6.0), (2.0, f32::NAN), (0.0, 6.0), (2.0, 1.0)] {
            let tc = Master {
                reload,
                weapon_range: range,
                ..town_center()
            };
            assert_eq!(attack_rate(&tc, &masters, 0.0).to_bits(), 0.0f32.to_bits());
        }
        // classes 4 and 6 sum their own melee, pierce, 31 and 38 amounts, not the missile's
        let infantry = Master {
            class: 6,
            weapons: vec![(4, 4), (3, 1), (31, 2), (38, 1), (11, 9)],
            ..town_center()
        };
        assert_eq!(attack_rate(&infantry, &masters, 0.5), 8.0 / 1.5);
    }

    #[test]
    fn a_volley_missile_slot_the_table_holds_is_taken_even_when_it_is_empty() {
        // FUN_14120e1a0: the volley missile when 0 <= id < the player's master count, else the
        // missile; a null slot or one without weapons adds nothing, with no fallback
        let missile = Master {
            id: 50,
            kind: 60,
            weapons: vec![(3, 6)],
            ..Master::default()
        };
        let volley = Master {
            id: 54,
            kind: 60,
            weapons: vec![(3, 2), (38, 1), (4, 9)],
            ..Master::default()
        };
        let unarmed = Master {
            id: 55,
            kind: 60,
            ..Master::default()
        };
        let archer = |volley_missile| Master {
            id: 4,
            kind: 70,
            reload: 2.0,
            weapons: vec![(3, 4)],
            missile: 50,
            garrison_firepower: 1.0,
            volley_fire: 3.0,
            volley_missile,
            ..Master::default()
        };
        let masters = Masters::new([missile, volley, unarmed]);
        // (2 + 1) * 2 extra missiles, plus its own 4, over its reload 2
        assert_eq!(firepower(&archer(Some(54)), &masters, 0.0), 5.0);
        // no volley missile: the missile's (6 * 2 + 4) / 2
        assert_eq!(firepower(&archer(None), &masters, 0.0), 8.0);
        // a volley missile without weapons adds nothing
        assert_eq!(firepower(&archer(Some(55)), &masters, 0.0), 2.0);
        // a null slot of the player's table (id 52 below its last master) adds nothing either
        assert_eq!(firepower(&archer(Some(52)), &masters, 0.0), 2.0);
        // a negative id is no slot: the missile's
        assert_eq!(firepower(&archer(Some(-2)), &masters, 0.0), 8.0);
    }

    #[test]
    fn the_extra_missiles_truncate_and_multiply_as_the_game_does() {
        // FUN_14120e1a0: CVTTSS2SI R8D gives i32::MIN past the i32 range, and IMUL EAX, R8D
        // wraps: amount 1 adds -2^31, amount 2 adds 0
        let volley = |amount| Master {
            id: 54,
            kind: 60,
            weapons: vec![(3, amount)],
            ..Master::default()
        };
        let tower = Master {
            id: 79,
            kind: 70,
            reload: 1.0,
            garrison_firepower: 1.0,
            volley_fire: 1e10,
            volley_missile: Some(54),
            ..Master::default()
        };
        let masters = Masters::new([volley(1)]);
        assert_eq!(firepower(&tower, &masters, 0.0), -2_147_483_648.0);
        let masters = Masters::new([volley(2)]);
        assert_eq!(firepower(&tower, &masters, 0.0), 0.0);
        // the f32 truncation toward zero: 3.9 - 1 gives 2 extra missiles
        let masters = Masters::new([volley(3)]);
        let three = Master {
            volley_fire: 3.9,
            ..tower
        };
        assert_eq!(firepower(&three, &masters, 0.0), 6.0);
    }

    #[test]
    fn a_share_is_at_least_zero_and_a_transforming_master_takes_its_other_form_as_its_floor() {
        // FUN_141213520: MAXSS(own / rate, floor), the floor 0.0 unless object flag 3 names (in
        // its upper half, +0x656) a master of type 70: that master's firepower over the rate
        let foot = Master {
            id: 1120,
            kind: 70,
            class: 0,
            reload: 2.0,
            weapons: vec![(3, 2)],
            garrison_firepower: 1.0,
            object_flags: 8 | (1122 << 16),
            ..Master::default()
        };
        let mounted = Master {
            id: 1122,
            kind: 70,
            class: 36,
            reload: 2.0,
            weapons: vec![(3, 6)],
            garrison_firepower: 1.0,
            ..Master::default()
        };
        let masters = Masters::new([foot.clone(), mounted.clone()]);
        // its own 1.0 against the other form's 3.0, over a rate of 2
        assert_eq!(share(&foot, &masters, 0.0, 2.0), Ok(1.5));
        // the other form's lower share leaves its own
        assert_eq!(share(&mounted, &masters, 0.0, 2.0), Ok(1.5));
        let reversed = Master {
            object_flags: 8 | (1120 << 16),
            ..mounted.clone()
        };
        assert_eq!(share(&reversed, &masters, 0.0, 2.0), Ok(1.5));
        // a named master not of type 70, or none, gives the floor 0.0; flag 3 clear names none
        let building = Master {
            kind: 80,
            ..mounted
        };
        let masters_80 = Masters::new([foot.clone(), building]);
        assert_eq!(share(&foot, &masters_80, 0.0, 2.0), Ok(0.5));
        let unflagged = Master {
            object_flags: 1122 << 16,
            ..foot.clone()
        };
        assert_eq!(share(&unflagged, &masters, 0.0, 2.0), Ok(0.5));
        // a negative quotient and a NaN one give the floor 0.0
        assert_eq!(share(&unflagged, &masters, 0.0, -2.0), Ok(0.0));
        assert_eq!(share(&unflagged, &masters, 0.0, f32::NAN), Ok(0.0));
        // _fdclass: a rate of ±0 gives +0.0; class 42 gives 1.0 whatever the rate
        assert_eq!(share(&foot, &masters, 0.0, -0.0).map(f32::to_bits), Ok(0));
        let relic_like = Master {
            class: WHOLE_SHARE_CLASS,
            ..foot
        };
        assert_eq!(share(&relic_like, &masters, 0.0, f32::NAN), Ok(1.0));
    }

    #[test]
    fn a_garrison_task_compares_the_bytes_the_game_passes_widened() {
        // FUN_1415c9240: the task's ushorts +0xe and +0x10 against the byte arguments
        let tasks = [task(GARRISON, 20), task(GARRISON, -1)];
        // class 276 passes its low byte 20; a task of 276 never matches a byte
        assert!(has_garrison_task(&tasks[..1], GARRISON, 276));
        assert!(has_garrison_task(&tasks[..1], GARRISON, 20));
        assert!(!has_garrison_task(&[task(GARRISON, 276)], GARRISON, 276));
        // a class of -1 passes 0xff, which a task of 0xffff does not match
        assert!(!has_garrison_task(&tasks[1..], GARRISON, -1));
        assert!(has_garrison_task(&[task(GARRISON, 255)], GARRISON, -1));
        assert!(!has_garrison_task(&[task(7, 3)], GARRISON, 3));
    }

    #[test]
    fn a_building_takes_a_unit_in_by_its_player_state_hit_points_room_and_garrison_type() {
        // FUN_14120eb10
        let mut t = garrison_world();
        let takes = |t: &Turns, b, p, m, others, unchecked| {
            t.takes_in_as(b, p, m, others, unchecked).unwrap()
        };
        assert!(takes(&t, 100, 1, VILLAGER, true, true));
        assert!(takes(&t, 100, 1, VILLAGER, true, false));
        // a master the player does not have, a building the world does not hold
        assert!(!takes(&t, 100, 1, 999, true, true));
        assert!(!takes(&t, 101, 1, VILLAGER, true, true));
        // an ally's building only with param_4 1; an enemy's never
        assert!(takes(&t, 100, 2, VILLAGER, true, true));
        assert!(!takes(&t, 100, 2, VILLAGER, false, true));
        assert!(!takes(&t, 100, 3, VILLAGER, true, true));
        // no garrison task on buildings; garrison type 11 has no bit for class 0
        assert!(!takes(&t, 100, 1, ARCHER, true, true));
        // the gate (an annex) asks its main building, but its own master must be of type 80
        assert!(takes(&t, 300, 1, VILLAGER, true, true));
        // a unit takes nothing in through this path: the archer is no transport
        assert!(!takes(&t, 200, 1, ARCHER, true, true));
        // hit points at most a fifth of the master's (COMISD, JBE: NaN too)
        for hp in [480.0, f32::NAN] {
            t.players[1].objects[0].life.hp = hp;
            assert!(!takes(&t, 100, 1, VILLAGER, true, true));
            assert!(!takes(&t, 300, 1, VILLAGER, true, true));
        }
        t.players[1].objects[0].life.hp = 480.5;
        assert!(takes(&t, 100, 1, VILLAGER, true, true));
        // state 2 only
        t.players[1].objects[0].life.state = 3;
        assert!(!takes(&t, 100, 1, VILLAGER, true, true));
        t.players[1].objects[0].life.state = 2;
        // full: only the room test (param_5 0) refuses
        for k in 0..15 {
            let mut v = unit(1000 + k, VILLAGER, 25.0);
            v.life.inside = Some(100);
            t.players[1].objects.push(v);
        }
        assert!(takes(&t, 100, 1, VILLAGER, true, true));
        assert!(!takes(&t, 100, 1, VILLAGER, true, false));
    }

    #[test]
    fn a_transport_takes_a_combat_unit_with_a_task_on_its_class_and_room() {
        // FUN_14120eb10's object-flag-0 branch: FUN_1415c9240 with 3 and the transport's class
        let mut t = garrison_world();
        let takes = |t: &Turns, m, unchecked| t.takes_in_as(200, 1, m, true, unchecked);
        assert_eq!(takes(&t, VILLAGER, false), Ok(true));
        assert_eq!(takes(&t, ARCHER, true), Ok(false));
        let mut v = unit(1000, VILLAGER, 25.0);
        v.life.inside = Some(200);
        t.players[1].objects.push(v);
        assert_eq!(takes(&t, VILLAGER, false), Ok(false));
        assert_eq!(takes(&t, VILLAGER, true), Ok(true));
        // a unit of another type needs the class flags (FUN_1411205e0), not modelled
        assert!(takes(&t, TC, true).is_err());
    }

    #[test]
    fn a_room_test_counts_the_unit_and_what_it_holds_or_one_for_none() {
        // FUN_1412169b0: slot 0x498 of the building plus that of the unit and 1
        let mut t = garrison_world();
        t.players[1].objects.push(unit(1000, VILLAGER, 25.0));
        let room = |t: &Turns, unit| {
            let (bp, b) = t.object(200).unwrap();
            t.has_room(bp, b, unit).unwrap()
        };
        assert!(room(&t, None));
        assert!(room(&t, Some(1000)));
        // a unit holding one more needs room for two
        let mut held = unit(1001, ARCHER, 25.0);
        held.life.inside = Some(1000);
        t.players[1].objects.push(held);
        assert!(!room(&t, Some(1000)));
        assert!(room(&t, None));
    }

    #[test]
    fn unload_picks_the_units_its_flag_names_of_any_player() {
        // FUN_14118a610 on the units inside town centre 100: player 1's villager 1000, archer
        // 1001 and the class-42 1002, and player 2's (an ally's) villager 2000
        let mut t = garrison_world();
        let mut relic = garrison_masters();
        relic = Masters::new(relic.iter().cloned().chain([Master {
            id: 285,
            kind: 70,
            class: WHOLE_SHARE_CLASS,
            ..Master::default()
        }]));
        t.masters = (0..4).map(|_| relic.clone()).collect();
        for (p, id, m) in [
            (1, 1000, VILLAGER),
            (1, 1001, ARCHER),
            (1, 1002, 285),
            (2, 2000, VILLAGER),
            (3, 3000, ARCHER),
        ] {
            let mut u = unit(id, m, 25.0);
            u.life.inside = (p != 3).then_some(100);
            t.players[p].objects.push(u);
        }
        let picks = |t: &Turns, flag, param| {
            [1000, 1001, 1002, 2000]
                .into_iter()
                .filter(|&id| {
                    let (p, u) = t.object(id).unwrap();
                    t.unload_picks(p, u, flag, param).unwrap()
                })
                .collect::<Vec<i32>>()
        };
        assert_eq!(picks(&t, 0, -1), [1000, 1001, 2000]);
        assert_eq!(picks(&t, 1, 2), [2000]);
        assert_eq!(picks(&t, 2, 1), [1000, 2000]);
        assert_eq!(picks(&t, 2, 6), [1002]);
        assert_eq!(picks(&t, 3, 1001), [1001]);
        // flag 4: the named object's master (a villager names every villager of its player)
        assert_eq!(picks(&t, 4, 1000), [1000]);
        assert_eq!(picks(&t, 4, 2000), [2000]);
        assert_eq!(picks(&t, 4, 1001), [1001]);
        assert_eq!(picks(&t, 4, 9999), [0i32; 0]);
        // flag 5: every unit but it and class 42, of its player or with no named object
        assert_eq!(picks(&t, 5, 1001), [1000]);
        assert_eq!(picks(&t, 5, 9999), [1000, 1001, 2000]);
        // flag 6: not of its master, never class 42, a villager not for a villager
        assert_eq!(picks(&t, 6, 1001), [1000]);
        assert_eq!(picks(&t, 6, 1000), [1001]);
        assert_eq!(picks(&t, 6, 3000), [0i32; 0]);
        assert_eq!(picks(&t, 7, 1000), [0i32; 0]);
    }

    /// A unit's movement as the spot test reads it: terrain restriction 0, no speed.
    fn standing() -> crate::movement::MoveStats {
        crate::movement::MoveStats {
            fly_mode: 0,
            speed: 0.0,
            walk_speed: 0.0,
            run_speed: 0.0,
            stand_speed: 0.0,
            turn_rate: 0.0,
            turn_radius: 0.0,
            turn_radius_speed: f32::MAX,
            radius: [0.2, 0.2, 2.0],
            restriction: 0,
            restriction_mask: 255,
            turn_time: 0.0,
            move_algorithm: 0,
            min_collision: 0.0,
            fast_turn: false,
            facets: None,
            weapon_range: 0.0,
        }
    }

    /// [`garrison_world`] on a flat 20×20 map every player sees, the town centre at (10, 10)
    /// with its footprint on the map, the villager master able to stand beside it.
    fn let_out_world() -> Turns {
        let mut t = garrison_world();
        let mut masters = garrison_masters();
        if let Some(tc) = masters.get_mut(TC) {
            tc.radius = [2.0, 2.0];
            tc.tall = true;
            tc.volley_fire = 1.0;
        }
        if let Some(v) = masters.get_mut(VILLAGER) {
            v.radius = [0.2, 0.2];
            v.tall = true;
            v.moving = Some(standing());
        }
        t.masters = (0..4).map(|_| masters.clone()).collect();
        let (w, h) = (20, 20);
        let n = (w * h) as usize;
        t.spatial = crate::spatial::Spatial::new([w, h]);
        let mut terrain = crate::terrain::Terrain::new(
            w,
            h,
            vec![(0, 255, -1); n],
            vec![
                crate::terrain::TerrainKind {
                    shown: true,
                    flags: 0,
                    forest: false,
                };
                1
            ],
            vec![vec![1.0]],
        );
        terrain.set_elevations(vec![0; n]);
        t.spatial.terrain = Some(terrain);
        let seen = (0..4).fold(0u64, |b, p| b | (1 << p) | (1 << (32 + p)));
        t.spatial.sight = Some(crate::visibility::Visibility::new(
            w,
            h,
            (0..4)
                .map(|_| crate::visibility::VisibleMap {
                    counts: vec![0; n],
                    explored: 0,
                })
                .collect(),
            vec![seen; n],
            Vec::new(),
            0,
            vec![false; 4],
        ));
        let tc = &mut t.players[1].objects[0];
        tc.position = [10.0, 10.0, 0.0];
        tc.firepower = 1.0;
        t.spatial.insert(
            100,
            crate::spatial::Placed {
                position: [10.0, 10.0, 0.0],
                radius: [2.0, 2.0],
                class: 52,
                player: 1,
                master: TC,
                obstruction_type: 2,
                buildable_on: 0,
                tall: true,
                speed: 0.0,
                unknown: None,
                obstruction: Some(crate::spatial::Obstruction { kind: 2, class: 2 }),
            },
        );
        t
    }

    /// Puts unit `id` of player `p` and master `m` inside the town centre.
    fn inside_tc(t: &mut Turns, p: usize, id: i32, m: i16) {
        let mut u = unit(id, m, 25.0);
        u.position = [10.0, 10.0, 0.0];
        u.life.inside = Some(100);
        t.players[p].objects.push(u);
        t.spatial.insert(
            id,
            crate::spatial::Placed {
                position: [10.0, 10.0, 0.0],
                radius: [0.2, 0.2],
                class: 4,
                player: p,
                master: m,
                obstruction_type: 0,
                buildable_on: 0,
                tall: true,
                speed: 0.0,
                unknown: None,
                obstruction: None,
            },
        );
        t.spatial.go_inside(id, [10.0, 10.0, 0.0]);
        t.players[1].objects[0].garrison.push(id);
    }

    #[test]
    fn back_to_work_lets_an_allys_villager_out_toward_its_record() {
        // FUN_141625590 -> FUN_1412081e0 -> FUN_1411e7220 on player 2's villager 2000 inside
        // player 1's town centre: its own player's master and orders, the town bell cleared
        let mut t = let_out_world();
        inside_tc(&mut t, 2, 2000, VILLAGER);
        let v = &mut t.players[2].objects[0];
        v.town_bell.flag = true;
        v.town_bell.work = crate::town_bell::BackToWork {
            point: [4.5, 4.5],
            ..crate::town_bell::BackToWork::NONE
        };
        // the building's last unit inside: the volleys go back to the master's volley fire
        t.players[1].objects[0].firepower = 2.0;
        t.go_back_to_work(1, &[100]).unwrap();
        let v = &t.players[2].objects[0];
        assert_eq!(v.life.inside, None);
        assert!(!v.town_bell.flag);
        assert_eq!(t.players[1].objects[0].garrison.len(), 0);
        assert_eq!(t.players[1].objects[0].firepower, 1.0);
        assert!(t.spatial.get_known(2000).unwrap().is_some());
        // FUN_141217910's reset: COMISS volley, firepower; JBE keeps it while units stay inside
        // and the firepower is not below the volley fire (a NaN one too)
        let mut t = let_out_world();
        inside_tc(&mut t, 1, 1000, VILLAGER);
        inside_tc(&mut t, 2, 2000, VILLAGER);
        t.players[2].objects[0].town_bell.work.point = [4.5, 4.5];
        t.players[1].objects[0].firepower = f32::NAN;
        t.back_to_work(1, 0, 2000).unwrap();
        assert!(t.players[1].objects[0].firepower.is_nan());
        assert_eq!(t.players[1].objects[0].garrison, [1000]);
        t.players[1].objects[0].firepower = 0.5;
        t.object_mut(1000).unwrap().1.town_bell.work.point = [4.5, 15.5];
        t.back_to_work(1, 0, 1000).unwrap();
        assert_eq!(t.players[1].objects[0].firepower, 1.0);
        // a unit not a villager stays inside
        let mut t = let_out_world();
        inside_tc(&mut t, 2, 2001, ARCHER);
        t.go_back_to_work(1, &[100]).unwrap();
        assert_eq!(t.players[2].objects[0].life.inside, Some(100));
    }

    #[test]
    fn a_unit_ais_test_takes_a_building_or_transport_that_takes_its_unit_in() {
        // FUN_140c40e00: type below 70 none, else FUN_14120eb10 with 1, 1
        let mut t = garrison_world();
        let mut masters = garrison_masters();
        masters = Masters::new(masters.iter().cloned().chain([Master {
            id: 59,
            kind: 60,
            class: 3,
            garrison: 5,
            ..Master::default()
        }]));
        t.masters = (0..4).map(|_| masters.clone()).collect();
        t.players[1].objects.push(unit(400, 59, 10.0));
        let takes = |t: &Turns, b, p, m| {
            let (bp, bu) = t.object(b).unwrap();
            t.ai_takes(bp, bu, p, m)
        };
        assert_eq!(takes(&t, 100, 1, VILLAGER), Ok(true));
        assert_eq!(takes(&t, 100, 2, VILLAGER), Ok(true));
        assert_eq!(takes(&t, 100, 1, ARCHER), Ok(false));
        // the transport (type 70) by its own branch
        assert_eq!(takes(&t, 200, 1, VILLAGER), Ok(true));
        // an object below type 70
        assert_eq!(takes(&t, 400, 1, VILLAGER), Ok(false));
    }

    /// A villager of player `p` with a unit AI and speed at (`x`, `y`).
    fn walker(t: &mut Turns, p: usize, id: i32, m: i16, at: [f32; 2]) {
        let mut v = unit(id, m, 25.0);
        v.position = [at[0], at[1], 0.0];
        v.ai = Some(crate::unit_ai::UnitAi {
            ai_type: 8,
            current_order: -1,
            current_action: -1,
            current_target: -1,
            ..crate::unit_ai::UnitAi::default()
        });
        v.mover = Some(crate::movement::Mover {
            forward: [1.0, 0.0, 0.0],
            right: [0.0, 0.0, -1.0],
            velocity: None,
            paths: vec![],
            factor: 1.0,
            stats: crate::movement::MoveStats {
                speed: 0.8,
                ..standing()
            },
            waiting: 0,
            wait_delays: 0,
            attack_moving: false,
            last_move: 0,
            moved_at: [0.0; 3],
            substitute_count: 0,
            ignore_units: Vec::new(),
            last_turn: 0.0,
            angle: 0.0,
            move_to_point: None,
            speed_stopped: false,
            pauses: Vec::new(),
            in_the_way: Vec::new(),
            sprite_facets: None,
        });
        t.players[p].objects.push(v);
        t.spatial.insert(
            id,
            crate::spatial::Placed {
                position: [at[0], at[1], 0.0],
                radius: [0.2, 0.2],
                class: 4,
                player: p,
                master: m,
                obstruction_type: 0,
                buildable_on: 0,
                tall: true,
                speed: 0.8,
                unknown: None,
                obstruction: None,
            },
        );
    }

    #[test]
    fn seek_shelter_sends_the_players_villagers_and_checked_units_to_its_buildings() {
        // FUN_140e6a3a0 -> FUN_141622450 -> FUN_141621220 with no transports: the command's
        // check (FUN_140e558b0) drops another player's unit unless players may order each
        // other's (+0x1074); a villager walks to the town centre's Enter
        let mut t = let_out_world();
        walker(&mut t, 1, 1000, VILLAGER, [5.5, 5.5]);
        walker(&mut t, 2, 2000, VILLAGER, [5.5, 6.5]);
        walker(&mut t, 1, 1001, ARCHER, [6.5, 5.5]);
        t.seek_shelter(1, &[1000, 2000, 1001, 4242]).unwrap();
        let enter = |t: &Turns, id| t.object(id).unwrap().1.enter.map(|e| e.target);
        assert_eq!(enter(&t, 1000), Some(100));
        // the ally's villager was dropped by the check; the archer is no villager
        assert_eq!((enter(&t, 2000), enter(&t, 1001)), (None, None));
        // with +0x1074 the ally's villager is the player's to send; its own player's town
        // centres are not asked, only the commanding player's
        let mut t = let_out_world();
        t.allied_control = true;
        walker(&mut t, 2, 2000, VILLAGER, [5.5, 6.5]);
        t.seek_shelter(1, &[2000]).unwrap();
        assert_eq!(enter(&t, 2000), Some(100));
        // a building that takes no villagers (garrison type bit 0) is not asked
        let mut t = let_out_world();
        for m in &mut t.masters {
            if let Some(b) = m.get_mut(TC).and_then(|tc| tc.building.as_mut()) {
                b.garrison_type = 2;
            }
        }
        walker(&mut t, 1, 1000, VILLAGER, [5.5, 5.5]);
        t.seek_shelter(1, &[1000]).unwrap();
        assert_eq!(enter(&t, 1000), None);
    }

    #[test]
    fn unload_lets_a_unit_out_of_the_building_it_is_inside() {
        // FUN_14161f410 on a unit inside a building: FUN_1411e7220 toward the point, +0x9dc
        // cleared, its orders gone; the building's own unload of the others is not run
        let mut t = let_out_world();
        inside_tc(&mut t, 1, 1000, VILLAGER);
        inside_tc(&mut t, 1, 1001, VILLAGER);
        let v = t.object_mut(1000).unwrap().1;
        v.town_bell.flag = true;
        v.ai = Some(crate::unit_ai::UnitAi::default());
        t.unload(1, &[1000], [4.5, 4.5], 0, -1, false).unwrap();
        let v = t.object(1000).unwrap().1;
        assert_eq!(v.life.inside, None);
        assert!(!v.town_bell.flag);
        assert_eq!(t.players[1].objects[0].garrison, [1001]);
        // a unit inside a transport is not modelled
        let mut t = let_out_world();
        let mut u = unit(1000, VILLAGER, 25.0);
        u.life.inside = Some(200);
        t.players[1].objects.push(u);
        assert!(t.unload(1, &[1000], [4.5, 4.5], 0, -1, false).is_err());
    }
}
