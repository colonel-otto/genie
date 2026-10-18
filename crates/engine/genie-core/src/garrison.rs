//! Garrisoning: `UNIT_ORDER`'s garrison (action 5 on a building, `FUN_14161e370` ->
//! `FUN_141621220`), which sends each unit to a building that takes it in, nearest first, each
//! building up to its capacity, and gives each building's units, through the order splitter,
//! the garrison work on it (slot 0x7e8 with task filter 3); and a building's test whether it takes
//! a unit in (`FUN_14120eb10`).

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
/// The insertion sort `FUN_1415f0830` keeps equal gaps in order below this many pairs; more go
/// through its introsort.
const STABLE_PAIRS: usize = 33;

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

/// `FUN_1415c9240` with 3, 3: the master's task list holds a garrison task (action type 3) on
/// buildings (class 3).
fn has_garrison_task(tasks: &[Task]) -> bool {
    tasks.iter().any(is_garrison_task)
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
/// The class whose garrison share is 1 whatever its firepower (`FUN_141213520`, `-0xe82` raw).
const WHOLE_SHARE_CLASS: i16 = 42;

/// The amounts of `weapons` (class, amount) whose class `counts`.
fn weapons_sum(weapons: &[(i16, i16)], counts: impl Fn(i16) -> bool) -> f32 {
    weapons
        .iter()
        .filter(|&&(class, _)| counts(class))
        .fold(0.0, |sum, &(_, amount)| sum + f32::from(amount))
}

/// `FUN_14120dc10`: the attack rate of an object of `master` (its reload powerup `+0xa28`
/// taken off its reload): none without a reload or a range above 1; else the pierce and class 38
/// amounts of its volley missile (a master its player has; not for classes 4 and 6), else its
/// own melee, pierce, class 31 and 38 amounts, over the reload.
#[must_use]
pub fn attack_rate(master: &Master, masters: &Masters, reload_powerup: f32) -> f32 {
    if master.reload <= 0.0 || master.weapon_range <= RATE_RANGE {
        return 0.0;
    }
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

/// `FUN_14120e1a0`: what a unit of `master` (its reload powerup `reload_powerup`) fires inside a
/// building. A master's garrison firepower at most 0 is its magnitude; else the firepower times
/// its rate: the pierce and class 38 amounts of its volley missile (else its missile; masters its
/// player has) times its volley fire past one missile (in whole missiles), plus its own melee,
/// pierce, class 31 and 38 amounts, over its reload (none at most 1e-7).
#[must_use]
pub fn firepower(master: &Master, masters: &Masters, reload_powerup: f32) -> f32 {
    let g = master.garrison_firepower;
    if g <= 0.0 {
        return g.abs();
    }
    let mut sum = 0.0;
    if ONE_MISSILE < master.volley_fire {
        let missile = master
            .volley_missile
            .and_then(|id| masters.get(id))
            .or_else(|| masters.get(master.missile));
        if let Some(m) = missile {
            let extra = (master.volley_fire - ONE_MISSILE) as i32;
            for &(class, amount) in &m.weapons {
                if matches!(class, PIERCE | CLASS_38) {
                    sum += (i32::from(amount) * extra) as f32;
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

/// `FUN_141213520`: the share of a building's volleys a unit of `master` inside it adds, against
/// the building's attack rate `rate` ([`attack_rate`]): none against a rate of 0; 1 for class 42;
/// else its [`firepower`] over the rate. A master whose object flags have bit 3 (the larger
/// share of another master, `+0x656`) is not modelled.
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
    if master.object_flags & 8 != 0 {
        return Err(Unsupported(
            "a garrison share of a master with object flag 3 (+0x656's share)",
        ));
    }
    Ok(firepower(master, masters, reload_powerup) / rate)
}

impl Turns {
    /// Slot 0x198 of a building (`FUN_1412169b0`) for `unit`: its count ([`Turns::garrisoned`])
    /// plus the unit and what it holds (1 and its own count) within its master's capacity
    /// (`obj_max`).
    pub(crate) fn has_room(&self, building: i32, unit: i32) -> Result<bool, Unsupported> {
        let (bp, b) = self
            .object(building)
            .ok_or(Unsupported("a room test of a building that is gone"))?;
        let capacity = i32::from(self.master_of(bp, b.life.master_id)?.garrison);
        Ok(self.garrisoned(building)? + 1 + self.garrisoned(unit)? <= capacity)
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
        if !self.has_room(target, id)? {
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
        if let Some(b) = self.players[player]
            .objects
            .iter_mut()
            .find(|u| u.id == target)
        {
            b.firepower += firepower;
        }
        for (kind, amount) in adds {
            self.players[player].add_attribute(kind, amount)?;
        }
        Ok(())
    }

    /// `FUN_14120eb10(building, player, master, 1, 1)`: whether `building` takes a unit of
    /// `player`'s master `master` in. The player must have the master; another player's building
    /// must be allied both ways (`FUN_141663d90`); the building in state 2 (an annex asks its main
    /// building, `+0xbb8`), of type 80, above [`LEAST_HP_SHARE`] of its master's hit points, with
    /// a capacity (`obj_max`); the unit's master of type 70 or more with a garrison task on
    /// buildings ([`has_garrison_task`]) and of a kind the building's `garrison_type` takes
    /// ([`garrison_type_takes`]). A building whose `object_flags` bit 0 is set (a transport's
    /// test, for a unit of type 70 or a class of flag 0x10002, `FUN_1411205e0`) is not modelled.
    pub(crate) fn takes_in(
        &self,
        building: i32,
        player: usize,
        master: i16,
    ) -> Result<bool, Unsupported> {
        let Some(unit) = self.masters.get(player).and_then(|m| m.get(master)) else {
            return Ok(false);
        };
        let Some((bp, b)) = self.object(building) else {
            return Ok(false);
        };
        let world = World {
            players: &self.players,
            drop_sites: &self.drop_sites,
        };
        if bp != player && !(world.allied(bp, player, true) && world.allied(player, bp, true)) {
            return Ok(false);
        }
        if b.life.state != 2 {
            return Ok(false);
        }
        if self.master_of(bp, b.life.master_id)?.object_flags & 1 != 0 {
            return Err(Unsupported(
                "a garrison test of a building with object flag 0 (a transport's)",
            ));
        }
        if unit.kind < 70 {
            return Ok(false);
        }
        let (bp, b) = match b.annex_of {
            Some(main) => self
                .object(main)
                .ok_or(Unsupported("an annex whose main building is gone"))?,
            None => (bp, b),
        };
        let bm = self.master_of(bp, b.life.master_id)?;
        if bm.kind != BUILDING {
            return Ok(false);
        }
        let max_hp = f64::from(f32::from(bm.hit_points));
        if f64::from(b.life.hp) <= max_hp * LEAST_HP_SHARE || bm.garrison == 0 {
            return Ok(false);
        }
        if !has_garrison_task(&unit.tasks) {
            return Ok(false);
        }
        let garrison_type = bm.building.map_or(0, |m| m.garrison_type);
        Ok(garrison_type_takes(
            garrison_type,
            unit.class,
            unit.id,
            true,
        ))
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
    /// `FUN_141621220` pairs each unit with each building that takes its master's copy in
    /// ([`Turns::takes_in`]) within [`FAR`], sorts the pairs by their squared footprint gap and
    /// gives each unit, in that order, the first building with room, then any unit left its
    /// nearest building. Each building's units go through the order splitter, which drops those
    /// whose unit AI's test fails (`FUN_140c40e00`: the building takes the unit's master in), and
    /// each loose unit (not in a group, `+0x6d0`; in state 2 or 6 or less, of a type 70 or more)
    /// takes slot 0x7e8 on the building at its place with task filter 3. A target of a lower type
    /// (a transport), the buildings near the target (`+0x1e`), the unit AI's queue (`+0x1d`) and
    /// the groups the splitter forms are not modelled.
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
        let mut count = self.garrisoned(building)?;
        let (bp, b) = self
            .object(building)
            .ok_or(Unsupported("a garrison's building is gone"))?;
        let bm = self.master_of(bp, b.life.master_id)?;
        let capacity = i32::from(bm.garrison);
        // the pairs of a unit and a building: (unit index, building, squared gap)
        let mut pairs = Vec::new();
        let ground = |id: i32| {
            self.spatial
                .get_known(id)?
                .map(|p| (p.position, p.radius))
                .ok_or(Unsupported("a garrison's object without a place"))
        };
        let units: Vec<Option<i32>> = ids
            .iter()
            .map(|&id| {
                let (p, u) = self.object(id)?;
                let m = self.masters.get(p)?.get(u.life.master_id)?;
                (70 <= m.kind).then_some(id)
            })
            .collect();
        for (i, &unit) in units.iter().enumerate() {
            let Some(id) = unit else { continue };
            let (p, u) = self.object(id).expect("listed above");
            let copy = self.master_of(p, u.life.master_id)?.copy_id;
            if bm.kind == BUILDING && bm.garrison != 0 && self.takes_in(building, player, copy)? {
                let (at, r) = ground(id)?;
                let (bat, br) = ground(building)?;
                let gap = footprint_gap_squared(at, r, bat, br);
                if gap < FAR {
                    pairs.push((i, gap));
                }
            }
        }
        if STABLE_PAIRS <= pairs.len() {
            return Err(Unsupported(
                "a garrison of 33 pairs or more (the introsort of FUN_1415f0830)",
            ));
        }
        pairs.sort_by(|a, b| a.1.total_cmp(&b.1));
        let mut taken = vec![false; units.len()];
        let mut going = Vec::new();
        for &(i, _) in &pairs {
            if !taken[i] && count < capacity {
                going.extend(units[i]);
                count += 1;
                taken[i] = true;
            }
        }
        for &(i, _) in &pairs {
            if !taken[i] {
                going.extend(units[i]);
                taken[i] = true;
            }
        }
        if going.is_empty() {
            return Ok(());
        }
        let keeps = |id: i32| -> Result<bool, Unsupported> {
            let (p, u) = self.object(id).expect("a going unit");
            if u.ai.is_none() {
                return Ok(true);
            }
            if self.master_of(bp, b.life.master_id)?.kind < 70 {
                return Ok(false);
            }
            self.takes_in(building, p, u.life.master_id)
        };
        let loose = self.loose_units_kept(&going, &keeps)?;
        let point = b.position;
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
