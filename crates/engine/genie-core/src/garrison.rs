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
    /// order queue emptied (`FUN_140c5e010`). A unit inside a building that unloads itself and a
    /// unit with units inside it (a transport's unload) are not modelled; the local player's
    /// interface flag (`FUN_141663f50`) is not held. A unit (type 70) out is no longer under the
    /// town bell (`+0x9dc`).
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
            if unit.life.inside.is_some() {
                return Err(Unsupported("an unload of a unit inside a building"));
            }
            if self.master_of(player, unit.life.master_id)?.kind < BUILDING {
                if unit.garrison.is_empty() {
                    continue;
                }
                return Err(Unsupported(
                    "an unload of a unit with units inside (a transport)",
                ));
            }
            if unit.ai.as_ref().is_some_and(|ai| ai.building_orders != 0) {
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
                if !self.unload_picks(player, inside, flag, unit_type)? {
                    continue;
                }
                let _ = self.let_out(player, idx, inside, point, sound)?;
                let ui = self.players[player]
                    .objects
                    .iter()
                    .position(|u| u.id == inside)
                    .ok_or(Unsupported("an unloaded unit outside its player's list"))?;
                let kind = self
                    .master_of(player, self.players[player].objects[ui].life.master_id)?
                    .kind;
                let u = &mut self.players[player].objects[ui];
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

    /// `FUN_14118a610`: whether UNLOAD's `flag` and `unit_type` (`param`) pick the unit `id`, by
    /// its master's class and id: flag 0 every unit but class 42; 1 the units of player `param`;
    /// 2 a group of classes ([`unload_group_takes`]); 3 the object `param`; with the object
    /// `param` of the world (`FUN_1415417a0`; the unit's player's or none), 4 the units of its
    /// master (a villager: any villager for a villager), 5 every unit but it and class 42, 6 the
    /// units not of its master (no villager for a villager, never class 42). Other flags pick none.
    fn unload_picks(
        &self,
        player: usize,
        id: i32,
        flag: u8,
        param: i32,
    ) -> Result<bool, Unsupported> {
        let unit = self.players[player]
            .objects
            .iter()
            .find(|u| u.id == id)
            .ok_or(Unsupported(
                "a unit inside a building outside its player's list",
            ))?;
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

    /// `FUN_1411e7220`: the unit `id` leaves the building at `building` of player `player`
    /// toward `point`, when a spot beside it is free ([`Turns::spot_beside`], toward the point,
    /// or the building's gather point when not on itself): it takes the spot at the building's
    /// height (`FUN_14119d610`), leaves the building (slot 0x1a8, `FUN_141213120`: the
    /// building's slot 0x1c8, `FUN_1411d6ad0` -> `FUN_141217910`, takes its share off the
    /// volleys, back to its master's volley fire when none is left inside or it fell below it,
    /// and lets it out of its list, `FUN_141197e10`; the building's sight lets the circle of 5
    /// go, slot 0x1d8 with 5) and goes on the map there (slot 0x58: its tile's list, its
    /// obstruction record, the mesh, its sight). With a gather point (`+0xc30` 1) not on the
    /// building and no point, it then goes to work on the gather point's target (its unit AI
    /// forgets its target, `FUN_140c5e1e0`; [`Turns::gather`]). False when no spot is free. Not
    /// modelled: the work given (slot 0x128 with a target), a point without a gather point (the
    /// unit AI's stop, or the move there), class 42's and livestock's counts in a building of a
    /// player with `+0x88`, a siege building's (class 13) slot 0x1b0; not held: the building's
    /// villager count (`+0xba8`), its sound (slot 0x4c0), its garrison sprite, the unit's
    /// `+0x9a0`. Its work command (slot 0x128) has `+0xa14` 0: the unit AI keeps its notifies.
    pub(crate) fn let_out(
        &mut self,
        player: usize,
        building: usize,
        id: i32,
        point: [f32; 2],
        _sound: bool,
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
        let ui = self.players[player]
            .objects
            .iter()
            .position(|u| u.id == id)
            .ok_or(Unsupported(
                "a unit inside a building outside its player's list",
            ))?;
        let unit = &self.players[player].objects[ui];
        let made = self.master_of(player, unit.life.master_id)?.clone();
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
        // the building's slot 0x1c8: the share off its volleys, the unit out of its list
        let share = if made.kind == 70 {
            let masters = self
                .masters
                .get(player)
                .ok_or(Unsupported("an unload without the masters"))?;
            let b = &self.players[player].objects[building];
            let unit = &self.players[player].objects[ui];
            let rate = attack_rate(&bm, masters, b.reload_powerup);
            share(&made, masters, unit.reload_powerup, rate)?
        } else {
            0.0
        };
        let b = &mut self.players[player].objects[building];
        b.firepower -= share;
        if let Some(k) = b.garrison.iter().position(|&g| g == id) {
            let _ = b.garrison.swap_remove(k);
        }
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
        let unit = &mut self.players[player].objects[ui];
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
        // the gather point's work
        match gather {
            Some(g) if point[0] == NO_POINT && g.target != bid && g.target != -1 => {
                if let Some(ai) = self.players[player].objects[ui].ai.as_mut() {
                    ai.remove_target();
                }
                // slot 0x128 with `+0xa14` 0: the unit AI keeps its notifies
                self.gather(player, ui, g, bid, false, false)?;
            }
            _ => {
                return Err(Unsupported(
                    "an unloaded unit's order without a gather point's target",
                ));
            }
        }
        Ok(true)
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

    /// `SEEK_SHELTER` (0x2c, `FUN_140e6a3a0`): of `ids`, the villagers (class 4 of type 70) and
    /// the units of master 1808 seek shelter (a villager of master 1808 is listed twice) in the
    /// player's buildings that take villagers: in the player's object order, those of type 80 in
    /// state 2, not inside another (`+0x2f0`, `FUN_141188da0`), whose master's `garrison_type`
    /// has bit 0 (`FUN_1411ccb60` with 1), each with its count ([`Turns::garrisoned`]); then
    /// [`Turns::shelter`] (`FUN_141622450`). The command's check (`FUN_140e558b0`) drops an id
    /// the world does not hold and another player's unit unless the game setting `+0x1074`
    /// allows it, which the slice does not hold: it stops there.
    pub(crate) fn seek_shelter(&mut self, player: usize, ids: &[i32]) -> Result<(), Unsupported> {
        let mut units = Vec::with_capacity(ids.len());
        for &id in ids {
            let Some((p, u)) = self.object(id) else {
                continue;
            };
            if p != player {
                return Err(Unsupported(
                    "a SeekShelter of another player's unit (the setting +0x1074)",
                ));
            }
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

    /// One building's part of [`Turns::shelter`]: the order splitter (`FUN_140fd9dc0` with
    /// command `0x2713`) on its units, then slot 0x7e8 for each loose unit.
    fn shelter_in(
        &mut self,
        building: i32,
        going: &[i32],
        through_ai: bool,
    ) -> Result<(), Unsupported> {
        let (bp, b) = self
            .object(building)
            .ok_or(Unsupported("a garrison's building is gone"))?;
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
        let loose = self.loose_units_kept(going, &keeps)?;
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
}
